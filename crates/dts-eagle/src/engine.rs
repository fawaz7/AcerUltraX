//! The DTS Eagle engine: loading `dtstech64.dll` and driving engine instances through their C++ interface.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use pe_loader::Module;

use crate::Error;

/// Size of an engine tuning blob (the complete engine state Windows stores per mode).
pub const BLOB_SIZE: usize = 0x2d20;
/// Largest block `process` accepts in one engine call.
pub const MAX_BLOCK: usize = 1024;
/// Channel counts an engine can be initialised with.
pub const MAX_CHANNELS: usize = 8;
/// Sample rates the engine accepts.
pub const RATES: [u32; 6] = [44100, 48000, 88200, 96000, 176400, 192000];

const ENGINE_NAME: &CStr = c"DTS Eagle";

/// Vtable slots of the engine object (DTS Eagle SDK 2.0.19).
mod slot {
    pub const MAX_BLOCK: usize = 4;
    pub const CONTROLS: usize = 6;
    pub const SET_INT: usize = 8;
    pub const SET_STR: usize = 9;
    pub const GET_FLOAT: usize = 11;
    pub const INIT: usize = 19;
    pub const PROCESS: usize = 20;
    pub const GET_BLOB: usize = 21;
    pub const SET_BLOB: usize = 22;
}

/// One channel buffer in the linked list the engine reads and writes.
#[repr(C)]
struct Chan {
    buf: *mut f32,
    ch: i32,
    _pad: i32,
    next: *mut Chan,
}

/// Layout of MSVC's `std::vector<unsigned char>`.
#[repr(C)]
struct MsvcVec {
    first: *mut u8,
    last: *mut u8,
    end: *mut u8,
}

type CreateFn = unsafe extern "win64" fn(*const c_char, *mut *mut c_void, *const c_char) -> u8;
type DestroyFn = unsafe extern "win64" fn(*mut c_void);
type MaxBlockFn = unsafe extern "win64" fn(*mut c_void) -> u32;
type ControlsFn = unsafe extern "win64" fn(*mut c_void, *mut *const u8) -> i64;
type SetIntFn = unsafe extern "win64" fn(*mut c_void, *const c_char, i32) -> u8;
type SetStrFn = unsafe extern "win64" fn(*mut c_void, *const c_char, *const c_char) -> u8;
type GetFloatFn = unsafe extern "win64" fn(*mut c_void, *const c_char, *mut f32) -> u8;
type InitFn = unsafe extern "win64" fn(*mut c_void, i32, i32) -> u8;
type ProcessFn = unsafe extern "win64" fn(*mut c_void, *mut Chan, *mut Chan, u32) -> u8;
type BlobFn = unsafe extern "win64" fn(*mut c_void, *mut MsvcVec) -> u8;

/// The loaded engine library. There is one per process.
pub struct Library {
    module: &'static Module,
    path: PathBuf,
    create: CreateFn,
    destroy: DestroyFn,
}

static LIBRARY: OnceLock<Result<Library, String>> = OnceLock::new();

impl Library {
    /// Load `dtstech64.dll` (once per process). `tuning_dir` is where the engine finds tuning files named by
    /// file controls such as `Eagle-CTC Blob`; engines configured only from tuning blobs need none.
    pub fn load(dll: &Path, tuning_dir: Option<&Path>) -> Result<&'static Library, Error> {
        let lib = LIBRARY.get_or_init(|| {
            pe_loader::set_file_root(tuning_dir.map(Path::to_path_buf));
            Self::load_once(dll).map_err(|e| e.to_string())
        });
        let lib = lib.as_ref().map_err(|e| Error::Load(e.clone()))?;
        if lib.path != dll {
            return Err(Error::Load(format!(
                "{} is already loaded; cannot also load {}",
                lib.path.display(),
                dll.display()
            )));
        }
        if let Some(dir) = tuning_dir {
            pe_loader::set_file_root(Some(dir.to_path_buf()));
        }
        Ok(lib)
    }

    fn load_once(dll: &Path) -> Result<Library, Error> {
        let module = Module::load(dll).map_err(|e| Error::Load(e.to_string()))?;
        let export =
            |name: &str| module.export(name).ok_or_else(|| Error::Load(format!("{} has no {name}", dll.display())));
        // SAFETY: the exports have these signatures (DTS Eagle SDK 2.0.19).
        let (create, destroy) = unsafe {
            (
                std::mem::transmute::<*const c_void, CreateFn>(export("DTSTechCreateProcess")?),
                std::mem::transmute::<*const c_void, DestroyFn>(export("DTSTechDestroyProcess")?),
            )
        };
        Ok(Library { module, path: dll.to_path_buf(), create, destroy })
    }
}

/// Where an engine instance sits in the Windows audio chain: per stream (SFX) or on the mix (MFX).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Location {
    Sfx,
    Mfx,
}

impl Location {
    pub fn as_str(self) -> &'static str {
        match self {
            Location::Sfx => "SFX",
            Location::Mfx => "MFX",
        }
    }
}

/// How a control's value is passed to the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Int,
    Text,
}

/// One entry of the engine's control table.
#[derive(Debug, Clone)]
pub struct ControlInfo {
    pub name: String,
    pub min: i32,
    pub max: i32,
    pub default: i32,
    /// Allowed values of an enumerated control.
    pub choices: Vec<String>,
    pub kind: ValueKind,
}

/// A tuning blob: the complete engine state for one mode.
#[derive(Clone, PartialEq, Eq)]
pub struct Blob(pub Box<[u8; BLOB_SIZE]>);

impl Blob {
    pub fn from_bytes(b: &[u8]) -> Result<Blob, Error> {
        let arr: [u8; BLOB_SIZE] = b.try_into().map_err(|_| Error::BlobSize(b.len()))?;
        Ok(Blob(Box::new(arr)))
    }
}

impl std::fmt::Debug for Blob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Blob({} bytes)", BLOB_SIZE)
    }
}

/// One engine instance. All methods may be called from any thread (one at a time); the calling thread needs a
/// stack of at least 1 MiB.
pub struct Engine {
    lib: &'static Library,
    obj: *mut c_void,
    location: Location,
    channels: usize,
    rate: u32,
    controls: OnceLock<Vec<ControlInfo>>,
}

// SAFETY: an instance has no thread affinity; `&mut self` serialises all calls that touch its state.
unsafe impl Send for Engine {}

impl Engine {
    pub fn new(lib: &'static Library, location: Location) -> Result<Engine, Error> {
        let loc = CString::new(location.as_str()).unwrap();
        let mut obj = std::ptr::null_mut();
        let _g = lib.module.enter();
        // SAFETY: valid strings and out pointer; the thread is prepared by `enter`.
        let ok = unsafe { (lib.create)(ENGINE_NAME.as_ptr(), &mut obj, loc.as_ptr()) } != 0;
        if !ok || obj.is_null() {
            return Err(Error::Engine("create failed".into()));
        }
        Ok(Engine { lib, obj, location, channels: 0, rate: 0, controls: OnceLock::new() })
    }

    unsafe fn vfn<T: Copy>(&self, i: usize) -> T {
        // SAFETY: `obj` is a live engine object whose first field is its vtable.
        unsafe {
            let vt = *(self.obj as *const *const usize);
            std::mem::transmute_copy(&*vt.add(i))
        }
    }

    pub fn location(&self) -> Location {
        self.location
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn max_block(&self) -> usize {
        let _g = self.lib.module.enter();
        unsafe { self.vfn::<MaxBlockFn>(slot::MAX_BLOCK)(self.obj) as usize }
    }

    /// Prepare for `channels` input channels (2..=8) at `rate` Hz.
    pub fn init(&mut self, channels: usize, rate: u32) -> Result<(), Error> {
        if !(2..=MAX_CHANNELS).contains(&channels) || !RATES.contains(&rate) {
            return Err(Error::Engine(format!("unsupported format: {channels} channels at {rate} Hz")));
        }
        let _g = self.lib.module.enter();
        if unsafe { self.vfn::<InitFn>(slot::INIT)(self.obj, channels as i32, rate as i32) } == 0 {
            return Err(Error::Engine(format!("init({channels}, {rate}) failed")));
        }
        self.channels = channels;
        self.rate = rate;
        Ok(())
    }

    /// The engine's control table.
    pub fn controls(&self) -> &[ControlInfo] {
        self.controls.get_or_init(|| {
            let _g = self.lib.module.enter();
            let mut list: *const u8 = std::ptr::null();
            // SAFETY: the engine returns its static table of 0x30-byte entries.
            unsafe {
                let n = self.vfn::<ControlsFn>(slot::CONTROLS)(self.obj, &mut list);
                (0..n.max(0) as usize).map(|i| read_control(list.add(i * 0x30))).collect()
            }
        })
    }

    pub fn control(&self, name: &str) -> Option<&ControlInfo> {
        self.controls().iter().find(|c| c.name == name)
    }

    pub fn set_int(&mut self, name: &str, value: i32) -> Result<(), Error> {
        let n = cstring(name)?;
        let _g = self.lib.module.enter();
        let ok = unsafe { self.vfn::<SetIntFn>(slot::SET_INT)(self.obj, n.as_ptr(), value) } != 0;
        ok.then_some(()).ok_or_else(|| Error::Control(name.into(), value.to_string()))
    }

    pub fn set_text(&mut self, name: &str, value: &str) -> Result<(), Error> {
        let (n, v) = (cstring(name)?, cstring(value)?);
        let _g = self.lib.module.enter();
        let ok = unsafe { self.vfn::<SetStrFn>(slot::SET_STR)(self.obj, n.as_ptr(), v.as_ptr()) } != 0;
        ok.then_some(()).ok_or_else(|| Error::Control(name.into(), value.into()))
    }

    /// Set a control from its text form, passing it as a number or as text according to the control table.
    pub fn set(&mut self, name: &str, value: &str) -> Result<(), Error> {
        let kind = self.control(name).ok_or_else(|| Error::UnknownControl(name.into()))?.kind;
        match kind {
            ValueKind::Text => self.set_text(name, value),
            ValueKind::Int => {
                let v = value.trim().parse::<i32>().map_err(|_| Error::Control(name.into(), value.into()))?;
                self.set_int(name, v)
            }
        }
    }

    pub fn get_float(&self, name: &str) -> Option<f32> {
        let n = cstring(name).ok()?;
        let mut v = 0f32;
        let _g = self.lib.module.enter();
        let ok = unsafe { self.vfn::<GetFloatFn>(slot::GET_FLOAT)(self.obj, n.as_ptr(), &mut v) } != 0;
        ok.then_some(v)
    }

    pub fn tuning_blob(&mut self) -> Blob {
        let mut blob = Blob(Box::new([0u8; BLOB_SIZE]));
        let p = blob.0.as_mut_ptr();
        // The engine reuses a caller buffer with enough capacity, so it never allocates or frees ours.
        let mut v = MsvcVec { first: p, last: p, end: unsafe { p.add(BLOB_SIZE) } };
        let _g = self.lib.module.enter();
        unsafe { self.vfn::<BlobFn>(slot::GET_BLOB)(self.obj, &mut v) };
        debug_assert_eq!(v.first, p);
        blob
    }

    pub fn set_tuning_blob(&mut self, blob: &Blob) -> Result<(), Error> {
        let p = blob.0.as_ptr() as *mut u8;
        let end = unsafe { p.add(BLOB_SIZE) };
        let mut v = MsvcVec { first: p, last: end, end };
        let _g = self.lib.module.enter();
        let ok = unsafe { self.vfn::<BlobFn>(slot::SET_BLOB)(self.obj, &mut v) } != 0;
        ok.then_some(()).ok_or_else(|| Error::Engine("set tuning blob failed".into()))
    }

    /// Process planar audio: `input` has one slice per initialised channel, `output` at least two (only the
    /// first two carry the result). All slices must have the same length; any length is accepted and processed
    /// in blocks of at most [`MAX_BLOCK`] frames.
    pub fn process(&mut self, input: &[&[f32]], output: &mut [&mut [f32]]) -> Result<(), Error> {
        let ch = self.channels;
        if ch == 0 {
            return Err(Error::Engine("process before init".into()));
        }
        let frames = input.first().map_or(0, |s| s.len());
        if input.len() != ch || input.iter().any(|s| s.len() != frames) {
            return Err(Error::Engine(format!("expected {ch} input channels of equal length")));
        }
        if output.len() < 2 || output.iter().any(|s| s.len() != frames) {
            return Err(Error::Engine("expected at least 2 output channels of the input length".into()));
        }
        // Inputs are copied into aligned scratch (the engine never sees caller memory it might write to); the
        // engine writes every channel of its output list, so unwanted output channels go to scratch too.
        let mut inb = Block([[0.0; MAX_BLOCK]; MAX_CHANNELS]);
        let mut extra = Block([[0.0; MAX_BLOCK]; MAX_CHANNELS]);
        let mut ins: [Chan; MAX_CHANNELS] = std::array::from_fn(|_| null_chan());
        let mut outs: [Chan; MAX_CHANNELS] = std::array::from_fn(|_| null_chan());
        let process = unsafe { self.vfn::<ProcessFn>(slot::PROCESS) };
        let _g = self.lib.module.enter();
        let mut off = 0;
        while off < frames {
            let n = (frames - off).min(MAX_BLOCK);
            for c in 0..ch {
                inb.0[c][..n].copy_from_slice(&input[c][off..off + n]);
                ins[c] = Chan { buf: inb.0[c].as_mut_ptr(), ch: c as i32, _pad: 0, next: std::ptr::null_mut() };
                let buf = match output.get_mut(c) {
                    Some(o) => o[off..].as_mut_ptr(),
                    None => extra.0[c].as_mut_ptr(),
                };
                outs[c] = Chan { buf, ch: c as i32, _pad: 0, next: std::ptr::null_mut() };
            }
            for c in 0..ch - 1 {
                ins[c].next = &mut ins[c + 1];
                outs[c].next = &mut outs[c + 1];
            }
            // SAFETY: both lists have `ch` valid buffers of at least `n` frames.
            if unsafe { process(self.obj, ins.as_mut_ptr(), outs.as_mut_ptr(), n as u32) } == 0 {
                return Err(Error::Engine("process failed".into()));
            }
            off += n;
        }
        Ok(())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _g = self.lib.module.enter();
        unsafe { (self.lib.destroy)(self.obj) };
    }
}

#[repr(C, align(64))]
struct Block([[f32; MAX_BLOCK]; MAX_CHANNELS]);

fn null_chan() -> Chan {
    Chan { buf: std::ptr::null_mut(), ch: 0, _pad: 0, next: std::ptr::null_mut() }
}

fn cstring(s: &str) -> Result<CString, Error> {
    CString::new(s).map_err(|_| Error::Control(s.into(), String::new()))
}

/// Decode one control table entry: +0 name, +8 min, +0xc max, +0x10 default, +0x18 choice count,
/// +0x20 choice strings.
unsafe fn read_control(e: *const u8) -> ControlInfo {
    unsafe {
        let rd_i32 = |off: usize| *(e.add(off) as *const i32);
        let text = |p: *const c_char| {
            if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
        };
        let name = text(*(e as *const *const c_char));
        let count = rd_i32(0x18).max(0) as usize;
        let table = *(e.add(0x20) as *const *const *const c_char);
        let choices = if table.is_null() { Vec::new() } else { (0..count).map(|i| text(*table.add(i))).collect() };
        let kind = value_kind(&name, count > 0);
        ControlInfo { name, min: rd_i32(0x08), max: rd_i32(0x0c), default: rd_i32(0x10), choices, kind }
    }
}

/// Enumerated controls and those naming files, blobs or strings take text; everything else a number.
fn value_kind(name: &str, enumerated: bool) -> ValueKind {
    if enumerated {
        return ValueKind::Text;
    }
    if name.ends_with("Enable") {
        return ValueKind::Int;
    }
    let textual = ["Blob", "File", "Name", "string", "Hearing Test"];
    if textual.iter().any(|t| name.contains(t)) { ValueKind::Text } else { ValueKind::Int }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_value_kinds() {
        assert_eq!(value_kind("Eagle-CTC Blob", false), ValueKind::Text);
        assert_eq!(value_kind("Eagle-Export Blob File Enable", false), ValueKind::Int);
        assert_eq!(value_kind("Eagle-Apo Mode Speaker string", false), ValueKind::Text);
        assert_eq!(value_kind("Eagle-TBHDX Bass Level", false), ValueKind::Int);
        assert_eq!(value_kind("Eagle-LC Preset", true), ValueKind::Text);
    }
}
