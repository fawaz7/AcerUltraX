//! Windows' "Spatial sound: DTS:X Ultra" (speakers) and "DTS Headphone:X" (headphones): the spatial encoder
//! `DtsHrtfEnc.dll` from DTS's app, loaded natively. It renders a 5.1 or 7.1 channel bed onto virtual speakers
//! around the listener and down to stereo. (Plain stereo beds are passed through by the encoder itself, so only
//! multichannel sound goes through it.)
//!
//! The encoder is a WinRT class used through Microsoft's undocumented `IAsarEncoder2` interface:
//! `Initialize(frames, rate, bed mask, output channels, output mask, -, -, settings)`, then per block
//! `MixChannelBed(samples, frames, channels, gain, mute)` and `Process(bytes, out, written)`. Its presets come as
//! two blobs in a WinRT property set, which is provided here as a minimal COM object.

mod com;

use std::collections::VecDeque;
use std::ffi::c_void;
use std::path::Path;
use std::sync::OnceLock;

use pe_loader::Module;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot load the spatial encoder: {0}")]
    Load(String),
    #[error("spatial encoder: {what} failed ({hr:#010x})")]
    Call { what: &'static str, hr: u32 },
    #[error("spatial sound needs 6 or 8 channels, not {0}")]
    Channels(usize),
}

/// Which renderer: DTS:X Ultra for speakers, or DTS Headphone:X.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Render {
    Speakers,
    Headphones,
}

impl Render {
    fn class(self) -> &'static str {
        match self {
            Render::Speakers => "DTS.DtsSpeakersEncoder",
            Render::Headphones => "DTS.DtsHeadphonesEncoder",
        }
    }
}

/// The encoder only runs at 48 kHz (other rates would need Windows' resampler).
pub const RATE: u32 = 48000;
/// Frames per encoder call; small so it fits any PipeWire quantum with little delay.
const BLOCK: usize = 128;
/// {DEFF1192-F581-4D77-9C1B-3E596B0CA989}
const IID_IASARENCODER2: com::Guid =
    com::Guid(0xDEFF1192, 0xF581, 0x4D77, [0x9C, 0x1B, 0x3E, 0x59, 0x6B, 0x0C, 0xA9, 0x89]);

type GetFactoryFn = unsafe extern "win64" fn(*mut c_void, *mut *mut c_void) -> i32;
type ActivateFn = unsafe extern "win64" fn(*mut c_void, *mut *mut c_void) -> i32;
type QueryFn = unsafe extern "win64" fn(*mut c_void, *const com::Guid, *mut *mut c_void) -> i32;
type ReleaseFn = unsafe extern "win64" fn(*mut c_void) -> u32;
#[allow(clippy::type_complexity)]
type InitializeFn =
    unsafe extern "win64" fn(*mut c_void, usize, usize, usize, usize, usize, usize, usize, *mut c_void) -> i32;
type MixBedFn = unsafe extern "win64" fn(*mut c_void, *const f32, u32, u32, f32, i32) -> i32;
type ProcessFn = unsafe extern "win64" fn(*mut c_void, u32, *mut f32, *mut u32) -> i32;

static MODULE: OnceLock<Result<&'static Module, String>> = OnceLock::new();

/// Load `DtsHrtfEnc.dll` (once per process).
pub fn load(dll: &Path) -> Result<&'static Module, Error> {
    MODULE.get_or_init(|| Module::load(dll).map_err(|e| e.to_string())).clone().map_err(Error::Load)
}

unsafe fn vfn<T: Copy>(obj: *mut c_void, slot: usize) -> T {
    unsafe {
        let vt = *(obj as *const *const usize);
        std::mem::transmute_copy(&*vt.add(slot))
    }
}

fn check(what: &'static str, hr: i32) -> Result<(), Error> {
    if hr < 0 { Err(Error::Call { what, hr: hr as u32 }) } else { Ok(()) }
}

/// One spatial encoder instance for a multichannel stream, with the buffering that lets it run in fixed blocks.
pub struct Encoder {
    module: &'static Module,
    enc: *mut c_void,
    channels: usize,
    /// Interleaved input not yet encoded, and stereo output not yet handed out.
    pending: Vec<f32>,
    ready: VecDeque<[f32; 2]>,
}

// SAFETY: an instance has no thread affinity; `&mut self` serialises all calls.
unsafe impl Send for Encoder {}

impl Encoder {
    /// `sad`: the spatial preset (`*.SPAC`); `device`: the device profile (`*.rdev`). `channels`: 6 (5.1) or 8
    /// (7.1), in WAVEFORMATEXTENSIBLE order (FL FR FC LFE BL BR SL SR).
    pub fn new(
        module: &'static Module,
        render: Render,
        sad: &[u8],
        device: &[u8],
        channels: usize,
    ) -> Result<Encoder, Error> {
        // AudioObjectType bits: FL 0x2, FR 0x4, FC 0x8, LFE 0x10, SL 0x20, SR 0x40, BL 0x80, BR 0x100.
        let mask = match channels {
            6 => 0x7e,
            8 => 0x1fe,
            n => return Err(Error::Channels(n)),
        };
        let _g = module.enter();
        let get: GetFactoryFn = unsafe {
            std::mem::transmute(
                module.export("DllGetActivationFactory").ok_or(Error::Load("no DllGetActivationFactory".into()))?,
            )
        };
        let class = pe_loader::shims::winrt::hstring(render.class());
        let mut factory = std::ptr::null_mut();
        let hr = unsafe { get(class, &mut factory) };
        unsafe { pe_loader::shims::winrt::delete_hstring(class) };
        check("activation factory", hr)?;
        let mut inst = std::ptr::null_mut();
        // IActivationFactory::ActivateInstance is slot 6 (after IInspectable's six).
        check("activate", unsafe { vfn::<ActivateFn>(factory, 6)(factory, &mut inst) })?;
        unsafe { vfn::<ReleaseFn>(factory, 2)(factory) };
        let mut enc = std::ptr::null_mut();
        let hr = unsafe { vfn::<QueryFn>(inst, 0)(inst, &IID_IASARENCODER2, &mut enc) };
        unsafe { vfn::<ReleaseFn>(inst, 2)(inst) };
        check("IAsarEncoder2", hr)?;
        let props = com::PropertySet::new(vec![
            ("RuntimeSettingsBLOB".into(), sad.to_vec()),
            ("DeviceProfileSettingsBLOB".into(), device.to_vec()),
        ]);
        // Stereo out: 2 channels, WAVE mask FL|FR.
        let hr = unsafe { vfn::<InitializeFn>(enc, 3)(enc, BLOCK, RATE as usize, mask, 2, 0x3, 0, 0, props.as_ptr()) };
        drop(props);
        let me = Encoder { module, enc, channels, pending: Vec::new(), ready: VecDeque::new() };
        check("initialize", hr)?;
        Ok(me)
    }

    /// Encode planar input (one slice per channel, equal lengths) into stereo `out` of the same length. The
    /// output trails the input by [`BLOCK`] frames (the encoder works in fixed blocks).
    pub fn process(&mut self, input: &[&[f32]], out: [&mut [f32]; 2]) -> Result<(), Error> {
        let n = out[0].len().min(out[1].len());
        let ch = self.channels;
        for i in 0..n {
            for c in 0..ch {
                self.pending.push(input.get(c).and_then(|s| s.get(i)).copied().unwrap_or(0.0));
            }
        }
        let _g = self.module.enter();
        let mut stereo = [0f32; BLOCK * 2];
        while self.pending.len() >= BLOCK * ch {
            let mut written = 0u32;
            unsafe {
                check(
                    "MixChannelBed",
                    vfn::<MixBedFn>(self.enc, 6)(self.enc, self.pending.as_ptr(), BLOCK as u32, ch as u32, 1.0, 0),
                )?;
                check(
                    "Process",
                    vfn::<ProcessFn>(self.enc, 7)(self.enc, (BLOCK * 2 * 4) as u32, stereo.as_mut_ptr(), &mut written),
                )?;
            }
            self.pending.drain(..BLOCK * ch);
            let frames = (written as usize / 8).min(BLOCK);
            self.ready
                .extend((0..BLOCK).map(|i| if i < frames { [stereo[2 * i], stereo[2 * i + 1]] } else { [0.0; 2] }));
        }
        let [l, r] = out;
        // Before the first block is out, the delay is silence.
        let missing = n.saturating_sub(self.ready.len());
        for i in 0..n {
            let [a, b] = if i < missing { [0.0; 2] } else { self.ready.pop_front().unwrap_or([0.0; 2]) };
            l[i] = a;
            r[i] = b;
        }
        Ok(())
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        let _g = self.module.enter();
        unsafe { vfn::<ReleaseFn>(self.enc, 2)(self.enc) };
    }
}
