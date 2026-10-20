//! Mapping, relocating, binding and initialising a DLL image.

use std::collections::HashMap;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::path::Path;
use std::sync::Mutex;

use crate::pe::{self, PeFile, rd_u16, rd_u32, rd_u64};
use crate::{Error, shims, thread};

type DllEntry = unsafe extern "win64" fn(*mut c_void, u32, *mut c_void) -> i32;

const DLL_PROCESS_ATTACH: u32 = 1;
const DLL_THREAD_ATTACH: u32 = 2;

/// MXCSR as Windows threads start with it: all exceptions masked, round to nearest, no FTZ/DAZ.
const WINDOWS_MXCSR: u32 = 0x1f80;

/// Implicit TLS of a module: the per-thread template (including zero fill) and the TLS callbacks.
type Tls = (Vec<u8>, Vec<usize>);

pub struct Module {
    name: String,
    base: usize,
    size: usize,
    id: usize,
    entry: Option<DllEntry>,
    exports: HashMap<String, u32>,
    tls: Option<Tls>,
}

/// Serialises DllMain calls, as the Windows loader lock does.
static LOADER_LOCK: Mutex<()> = Mutex::new(());
/// Every loaded module (leaked; DLLs are never unloaded).
static MODULES: Mutex<Vec<&'static Module>> = Mutex::new(Vec::new());

pub(crate) fn module_containing(addr: usize) -> Option<&'static Module> {
    MODULES.lock().unwrap().iter().copied().find(|m| addr >= m.base && addr < m.base + m.size)
}

pub(crate) fn module_at(base: usize) -> Option<&'static Module> {
    MODULES.lock().unwrap().iter().copied().find(|m| m.base == base)
}

impl Module {
    /// Load a DLL, bind it to the built-in Win32 functions and run its `DllMain(DLL_PROCESS_ATTACH)` on the
    /// calling thread. The module stays loaded for the life of the process.
    pub fn load(path: &Path) -> Result<&'static Module, Error> {
        let data = std::fs::read(path).map_err(|source| Error::Io { path: path.into(), source })?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let file = PeFile::parse(&data)?;

        let id = MODULES.lock().unwrap().len();
        if id >= thread::MAX_MODULES {
            return Err(Error::TooManyModules);
        }
        let size = file.size_of_image as usize;
        let base = map_image(file.image_base as usize, size)?;
        // From here on the mapping is leaked on error; loading failures are fatal to the caller anyway.
        let img = |rva: u32| (base + rva as usize) as *mut u8;
        // SAFETY: the mapping is `size` bytes, writable, and every copy is bounds-checked against both the file
        // and the image size.
        unsafe {
            let hdr = (file.size_of_headers as usize).min(data.len()).min(size);
            std::ptr::copy_nonoverlapping(data.as_ptr(), img(0), hdr);
            for s in &file.sections {
                // Raw data is file-aligned and may be longer than the section; the rest stays zero (BSS).
                let n = if s.virtual_size == 0 { s.raw_size } else { s.raw_size.min(s.virtual_size) } as usize;
                let src = data
                    .get(s.raw_offset as usize..s.raw_offset as usize + n)
                    .ok_or(Error::Malformed("section data outside the file"))?;
                if s.virtual_address as usize + n > size {
                    return Err(Error::Malformed("section outside the image"));
                }
                std::ptr::copy_nonoverlapping(src.as_ptr(), img(s.virtual_address), n);
            }
        }
        let view = unsafe { std::slice::from_raw_parts_mut(base as *mut u8, size) };

        let delta = base.wrapping_sub(file.image_base as usize);
        if delta != 0 {
            relocate(view, file.dir(pe::DIR_BASERELOC), delta as u64)?;
        }
        bind_imports(view, file.dir(pe::DIR_IMPORT))?;
        let exports = read_exports(view, file.dir(pe::DIR_EXPORT))?;
        let tls = setup_tls(view, base, file.dir(pe::DIR_TLS), id)?;
        protect(base, &file)?;

        let entry = (file.entry_point != 0)
            // SAFETY: the entry point lies in the mapped, executable image.
            .then(|| unsafe { std::mem::transmute::<usize, DllEntry>(base + file.entry_point as usize) });
        let module: &'static Module = Box::leak(Box::new(Module { name, base, size, id, entry, exports, tls }));
        MODULES.lock().unwrap().push(module);
        log::debug!("loaded {} at {:#x} ({} bytes)", module.name, base, size);

        let ok = {
            let _lock = LOADER_LOCK.lock().unwrap();
            thread::with(|t| {
                module.install_thread_tls(t);
                t.set_attached(module.id);
            });
            let _mx = MxcsrGuard::new();
            module.notify(DLL_PROCESS_ATTACH)
        };
        if !ok {
            return Err(Error::InitFailed);
        }
        Ok(module)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn base(&self) -> usize {
        self.base
    }

    /// Address of an exported function or variable.
    pub fn export(&self, name: &str) -> Option<*const c_void> {
        self.exports.get(name).map(|&rva| (self.base + rva as usize) as *const c_void)
    }

    /// Prepare the calling thread for calls into this module: install the TEB and the module's TLS block, send
    /// `DLL_THREAD_ATTACH` the first time, and set MXCSR as on Windows until the guard is dropped.
    ///
    /// The thread needs a stack of at least 1 MiB for DTS code.
    pub fn enter(&self) -> CallGuard {
        let first = thread::with(|t| {
            let first = !t.is_attached(self.id);
            if first {
                self.install_thread_tls(t);
                t.set_attached(self.id);
            }
            first
        });
        let mx = MxcsrGuard::new();
        if first {
            let _lock = LOADER_LOCK.lock().unwrap();
            self.notify(DLL_THREAD_ATTACH);
        }
        CallGuard { _mx: mx, _not_send: PhantomData }
    }

    fn install_thread_tls(&self, t: &thread::ThreadState) {
        if let Some((template, _)) = &self.tls {
            t.install_tls(self.id, template);
        }
    }

    fn notify(&self, reason: u32) -> bool {
        let hinst = self.base as *mut c_void;
        if let Some((_, callbacks)) = &self.tls {
            for &cb in callbacks {
                // SAFETY: TLS callbacks have the DllMain signature.
                unsafe { std::mem::transmute::<usize, DllEntry>(cb)(hinst, reason, std::ptr::null_mut()) };
            }
        }
        match self.entry {
            // SAFETY: the TEB and TLS are installed on this thread and the image is fully bound.
            Some(entry) => unsafe { entry(hinst, reason, std::ptr::null_mut()) != 0 },
            None => true,
        }
    }
}

/// Keeps the thread prepared for DLL calls; restores the caller's MXCSR when dropped. Not `Send`.
pub struct CallGuard {
    _mx: MxcsrGuard,
    _not_send: PhantomData<*mut ()>,
}

struct MxcsrGuard(u32);

impl MxcsrGuard {
    fn new() -> Self {
        let old = get_mxcsr();
        set_mxcsr(WINDOWS_MXCSR);
        MxcsrGuard(old)
    }
}

impl Drop for MxcsrGuard {
    fn drop(&mut self) {
        set_mxcsr(self.0);
    }
}

fn get_mxcsr() -> u32 {
    let mut v: u32 = 0;
    // SAFETY: stmxcsr stores the 32-bit MXCSR to the given address.
    unsafe { std::arch::asm!("stmxcsr [{}]", in(reg) &mut v, options(nostack)) };
    v
}

fn set_mxcsr(v: u32) {
    // SAFETY: loads a valid MXCSR value (reserved bits are never set by callers).
    unsafe { std::arch::asm!("ldmxcsr [{}]", in(reg) &v, options(nostack, readonly)) };
}

fn map_image(preferred: usize, size: usize) -> Result<usize, Error> {
    let prot = libc::PROT_READ | libc::PROT_WRITE;
    let flags = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS;
    // SAFETY: anonymous mappings; MAP_FIXED_NOREPLACE never replaces an existing mapping.
    unsafe {
        let p = libc::mmap(preferred as *mut c_void, size, prot, flags | libc::MAP_FIXED_NOREPLACE, -1, 0);
        if p != libc::MAP_FAILED {
            if p as usize == preferred {
                return Ok(p as usize);
            }
            // Old kernels treat the flag as a hint only.
            libc::munmap(p, size);
        }
        let p = libc::mmap(std::ptr::null_mut(), size, prot, flags, -1, 0);
        if p == libc::MAP_FAILED {
            return Err(Error::Map(std::io::Error::last_os_error()));
        }
        Ok(p as usize)
    }
}

fn relocate(img: &mut [u8], dir: pe::DataDir, delta: u64) -> Result<(), Error> {
    if dir.rva == 0 {
        return Err(Error::Unsupported("image cannot be moved (no relocations)"));
    }
    let mut off = dir.rva as usize;
    let end = off + dir.size as usize;
    while off + 8 <= end {
        let page = rd_u32(img, off)? as usize;
        let block = rd_u32(img, off + 4)? as usize;
        if block < 8 {
            break;
        }
        for i in 0..(block - 8) / 2 {
            let e = rd_u16(img, off + 8 + 2 * i)?;
            match e >> 12 {
                0 => {}
                10 => {
                    let at = page + (e & 0xfff) as usize;
                    let v = rd_u64(img, at)?.wrapping_add(delta);
                    img[at..at + 8].copy_from_slice(&v.to_le_bytes());
                }
                _ => return Err(Error::Unsupported("relocation type other than DIR64")),
            }
        }
        off += block;
    }
    Ok(())
}

fn c_str(img: &[u8], rva: usize) -> Result<String, Error> {
    let s = img.get(rva..).ok_or(Error::Malformed("string outside the image"))?;
    let n = s.iter().position(|&b| b == 0).ok_or(Error::Malformed("unterminated string"))?;
    Ok(String::from_utf8_lossy(&s[..n]).into_owned())
}

fn bind_imports(img: &mut [u8], dir: pe::DataDir) -> Result<(), Error> {
    let mut missing = Vec::new();
    let mut d = dir.rva as usize;
    if d == 0 {
        return Ok(());
    }
    loop {
        let lookup = rd_u32(img, d)? as usize;
        let dll_name = rd_u32(img, d + 12)? as usize;
        let iat = rd_u32(img, d + 16)? as usize;
        if dll_name == 0 && iat == 0 {
            break;
        }
        let dll = c_str(img, dll_name)?;
        let lookup = if lookup != 0 { lookup } else { iat };
        for i in 0.. {
            let thunk = rd_u64(img, lookup + 8 * i)?;
            if thunk == 0 {
                break;
            }
            let addr = if thunk & (1 << 63) != 0 {
                missing.push(format!("{dll}!#{}", thunk & 0xffff));
                0
            } else {
                let name = c_str(img, thunk as usize + 2)?;
                match shims::lookup(&name) {
                    Some(f) => f as u64,
                    None => {
                        missing.push(format!("{dll}!{name}"));
                        0
                    }
                }
            };
            let at = iat + 8 * i;
            img.get_mut(at..at + 8)
                .ok_or(Error::Malformed("IAT outside the image"))?
                .copy_from_slice(&addr.to_le_bytes());
        }
        d += 20;
    }
    if missing.is_empty() { Ok(()) } else { Err(Error::MissingImports(missing)) }
}

fn read_exports(img: &[u8], dir: pe::DataDir) -> Result<HashMap<String, u32>, Error> {
    let mut out = HashMap::new();
    if dir.rva == 0 {
        return Ok(out);
    }
    let d = dir.rva as usize;
    let nnames = rd_u32(img, d + 0x18)? as usize;
    let funcs = rd_u32(img, d + 0x1c)? as usize;
    let names = rd_u32(img, d + 0x20)? as usize;
    let ords = rd_u32(img, d + 0x24)? as usize;
    for i in 0..nnames {
        let name = c_str(img, rd_u32(img, names + 4 * i)? as usize)?;
        let ord = rd_u16(img, ords + 2 * i)? as usize;
        out.insert(name, rd_u32(img, funcs + 4 * ord)?);
    }
    Ok(out)
}

/// Assign the module's implicit-TLS index and capture its template. Values in the TLS directory are already
/// relocated virtual addresses.
fn setup_tls(img: &mut [u8], base: usize, dir: pe::DataDir, index: usize) -> Result<Option<Tls>, Error> {
    if dir.rva == 0 {
        return Ok(None);
    }
    let d = dir.rva as usize;
    let rva = |va: u64| (va as usize).wrapping_sub(base);
    let start = rva(rd_u64(img, d)?);
    let end = rva(rd_u64(img, d + 8)?);
    let index_at = rva(rd_u64(img, d + 16)?);
    let callbacks_va = rd_u64(img, d + 24)?;
    let zero_fill = rd_u32(img, d + 32)? as usize;
    let mut template = img.get(start..end).ok_or(Error::Malformed("TLS template outside the image"))?.to_vec();
    template.resize(template.len() + zero_fill, 0);
    img.get_mut(index_at..index_at + 4)
        .ok_or(Error::Malformed("TLS index outside the image"))?
        .copy_from_slice(&(index as u32).to_le_bytes());
    let mut callbacks = Vec::new();
    if callbacks_va != 0 {
        let mut at = rva(callbacks_va);
        loop {
            let cb = rd_u64(img, at)?;
            if cb == 0 {
                break;
            }
            callbacks.push(cb as usize);
            at += 8;
        }
    }
    Ok(Some((template, callbacks)))
}

fn protect(base: usize, file: &PeFile) -> Result<(), Error> {
    let page = |n: usize| (n + 0xfff) & !0xfff;
    let set = |rva: usize, len: usize, prot: i32| -> Result<(), Error> {
        // SAFETY: the range lies inside the image mapping and is page aligned.
        if unsafe { libc::mprotect((base + rva) as *mut c_void, page(len), prot) } != 0 {
            return Err(Error::Map(std::io::Error::last_os_error()));
        }
        Ok(())
    };
    set(0, file.size_of_headers as usize, libc::PROT_READ)?;
    for s in &file.sections {
        let len = s.virtual_size.max(s.raw_size) as usize;
        if len == 0 {
            continue;
        }
        let prot = if s.executable() {
            libc::PROT_READ | libc::PROT_EXEC
        } else if s.writable() {
            libc::PROT_READ | libc::PROT_WRITE
        } else {
            libc::PROT_READ
        };
        set(s.virtual_address as usize, len, prot)?;
    }
    Ok(())
}
