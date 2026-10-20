//! The processing node: a PipeWire filter with one group of input ports per app stream and a stereo output.
//! Its process callback runs on PipeWire's real-time thread and drives the [`audio_graph::Chain`].
//!
//! pipewire-rs has no filter wrapper, so this uses the C API directly.

use std::ffi::{CString, c_void};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use audio_graph::{Chain, MAX_FRAMES, Prepared, StreamInput};
use pipewire::properties::{PropertiesBox, properties};
use pipewire_sys as pw;

/// Streams handled at once (each gets its own port group); further streams share group 0.
pub const MAX_GROUPS: usize = 16;
const MAX_CH: usize = 8;

/// An input-port group: one app stream's channels, in DTS (WAVEFORMATEXTENSIBLE) order.
pub struct Group {
    pub index: usize,
    /// Stable stream id given to the chain (the app stream's serial; 1 for group 0).
    pub id: u32,
    pub ports: Vec<Port>,
}

pub struct Port(*mut c_void);
// SAFETY: port handles are only dereferenced by PipeWire; they stay valid until removed under the groups lock.
unsafe impl Send for Port {}

/// State shared between the main loop and the real-time callback. The callback only ever `try_lock`s.
#[derive(Default)]
pub struct Shared {
    pub groups: Mutex<Vec<Group>>,
    /// A mode to swap in at the next cycle.
    pub pending: Mutex<Option<Prepared>>,
    pub bypass: AtomicBool,
    pub match_loudness: AtomicBool,
    /// Sample rate of the last cycle (0 before the first).
    pub rate: AtomicU32,
    /// Rate of the mode in use (0 = none).
    pub mode_rate: AtomicU32,
    /// What the chain let go of, freed by the main loop ([`Shared::empty_trash`]).
    pub trash: Mutex<Vec<audio_graph::Garbage>>,
}

impl Shared {
    /// Free what the audio thread set aside (main loop), and leave room for next time.
    pub fn empty_trash(&self) {
        let mut t = self.trash.lock().unwrap();
        t.clear();
        t.reserve(audio_graph::TRASH);
    }
}

struct Rt {
    chain: Chain,
    shared: Arc<Shared>,
    out: [*mut c_void; 2],
}

pub struct Filter {
    raw: *mut pw::pw_filter,
    _events: Box<pw::pw_filter_events>,
    rt: *mut Rt,
}

fn port_props(name: &str, channel: &str) -> PropertiesBox {
    properties! {
        "format.dsp" => "32 bit float mono audio",
        "port.name" => name,
        "audio.channel" => channel,
    }
}

impl Filter {
    pub fn new(loop_: &pipewire::loop_::Loop, lib: &'static dts_eagle::Library, shared: Arc<Shared>) -> Filter {
        let props = properties! {
            "media.type" => "Audio",
            "media.category" => "Filter",
            "media.role" => "DSP",
            "node.autoconnect" => "false",
            "node.description" => "AcerUltraX processing",
        };
        let mut events: Box<pw::pw_filter_events> = Box::new(unsafe { std::mem::zeroed() });
        events.version = pw::PW_VERSION_FILTER_EVENTS;
        events.process = Some(on_process);
        let rt = Box::into_raw(Box::new(Rt { chain: Chain::new(lib), shared, out: [std::ptr::null_mut(); 2] }));
        let name = CString::new(crate::router::FILTER_NAME).unwrap();
        // SAFETY: the loop outlives the filter; props ownership passes to PipeWire.
        let raw = unsafe {
            pw::pw_filter_new_simple(loop_.as_raw_ptr(), name.as_ptr(), props.into_raw(), &*events, rt.cast())
        };
        assert!(!raw.is_null(), "cannot create the PipeWire filter");
        let mut f = Filter { raw, _events: events, rt };
        for (i, ch) in ["FL", "FR"].into_iter().enumerate() {
            let p = f.add_port(libspa_sys::SPA_DIRECTION_OUTPUT, &format!("output_{ch}"), ch);
            // SAFETY: the filter is not connected yet, so the callback cannot run concurrently.
            unsafe { (*f.rt).out[i] = p.0 };
        }
        f
    }

    fn add_port(&mut self, dir: libspa_sys::spa_direction, name: &str, channel: &str) -> Port {
        // SAFETY: valid filter; props ownership passes to PipeWire.
        let p = unsafe {
            pw::pw_filter_add_port(
                self.raw,
                dir,
                pw::pw_filter_port_flags_PW_FILTER_PORT_FLAG_MAP_BUFFERS,
                0,
                port_props(name, channel).into_raw(),
                std::ptr::null_mut(),
                0,
            )
        };
        assert!(!p.is_null(), "cannot add filter port {name}");
        Port(p)
    }

    /// Input ports for a group, named `g<index>_<channel>`.
    pub fn add_group(&mut self, index: usize, id: u32, channels: &[&str]) -> Group {
        let ports = channels
            .iter()
            .map(|ch| self.add_port(libspa_sys::SPA_DIRECTION_INPUT, &format!("g{index}_{ch}"), ch))
            .collect();
        Group { index, id, ports }
    }

    pub fn remove_group(&mut self, g: Group) {
        for p in g.ports {
            // SAFETY: the port belongs to this filter and is no longer reachable from the callback.
            unsafe { pw::pw_filter_remove_port(p.0) };
        }
    }

    pub fn connect(&mut self) {
        // SAFETY: valid filter.
        let r = unsafe {
            pw::pw_filter_connect(self.raw, pw::pw_filter_flags_PW_FILTER_FLAG_RT_PROCESS, std::ptr::null_mut(), 0)
        };
        assert!(r >= 0, "cannot connect the PipeWire filter ({r})");
    }
}

impl Drop for Filter {
    fn drop(&mut self) {
        // SAFETY: destroying the filter stops the callback before the state is freed.
        unsafe {
            pw::pw_filter_destroy(self.raw);
            drop(Box::from_raw(self.rt));
        }
    }
}

unsafe fn dsp_buffer<'a>(port: *mut c_void, n: u32) -> Option<&'a mut [f32]> {
    let p = unsafe { pw::pw_filter_get_dsp_buffer(port, n) } as *mut f32;
    (!p.is_null()).then(|| unsafe { std::slice::from_raw_parts_mut(p, n as usize) })
}

unsafe extern "C" fn on_process(data: *mut c_void, position: *mut libspa_sys::spa_io_position) {
    // SAFETY: `data` is the Rt this filter was created with; only this callback touches it while connected.
    let rt = unsafe { &mut *(data as *mut Rt) };
    let clock = unsafe { &(*position).clock };
    let n = (clock.duration as usize).min(MAX_FRAMES) as u32;
    let rate = if clock.rate.denom != 0 { clock.rate.denom } else { 48000 };
    let (Some(l), Some(r)) = (unsafe { dsp_buffer(rt.out[0], n) }, unsafe { dsp_buffer(rt.out[1], n) }) else {
        return;
    };
    let shared = &rt.shared;
    shared.rate.store(rate, Ordering::Relaxed);
    if let Ok(mut p) = shared.pending.try_lock() {
        if let Some(mode) = p.take() {
            shared.mode_rate.store(mode.rate(), Ordering::Relaxed);
            rt.chain.set_mode(mode);
        }
    }
    rt.chain.bypass = shared.bypass.load(Ordering::Relaxed);
    rt.chain.match_loudness = shared.match_loudness.load(Ordering::Relaxed);
    let Ok(groups) = shared.groups.try_lock() else {
        // The main loop is adding or removing a stream right now: one quiet cycle.
        l.fill(0.0);
        r.fill(0.0);
        return;
    };
    let mut bufs: [[&[f32]; MAX_CH]; MAX_GROUPS] = [[&[]; MAX_CH]; MAX_GROUPS];
    let mut ids = [0u32; MAX_GROUPS];
    let mut widths = [0usize; MAX_GROUPS];
    let mut k = 0;
    for g in groups.iter().take(MAX_GROUPS) {
        let mut ok = !g.ports.is_empty();
        for (c, p) in g.ports.iter().enumerate().take(MAX_CH) {
            match unsafe { dsp_buffer(p.0, n) } {
                Some(b) => bufs[k][c] = b,
                None => ok = false,
            }
        }
        if ok {
            ids[k] = g.id;
            widths[k] = g.ports.len().min(MAX_CH);
            k += 1;
        }
    }
    let inputs: [StreamInput; MAX_GROUPS] =
        std::array::from_fn(|i| StreamInput { id: ids[i], channels: &bufs[i][..widths[i]] });
    rt.chain.process(rate, &inputs[..k], [l, r]);
    drop(groups);
    if let Ok(mut bin) = shared.trash.try_lock() {
        rt.chain.take_trash(&mut bin);
    }
}
