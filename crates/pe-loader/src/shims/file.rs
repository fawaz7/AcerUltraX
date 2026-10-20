//! Handles, read-only file access, standard handles, console output and path helpers.
//!
//! DLL code may only open existing files for reading. Paths are mapped to Linux: the fake module directory and
//! relative paths resolve against the file root set with [`crate::set_file_root`], backslashes become slashes,
//! and the last component is matched case-insensitively as on Windows.

#![allow(non_snake_case)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::module::FAKE_DIR;
use super::sync::Event;
use super::{Bool, ERROR_ACCESS_DENIED, ERROR_CALL_NOT_IMPLEMENTED, ERROR_FILE_NOT_FOUND, ERROR_INVALID_HANDLE};
use super::{FALSE, Handle, INVALID_HANDLE_VALUE, TRUE, set_last_error, wide_to_string};

#[derive(Clone)]
pub(crate) enum Object {
    File(Arc<Mutex<File>>),
    Event(Arc<Event>),
    /// Standard input/output/error (fd 0/1/2).
    Std(i32),
}

static HANDLES: Mutex<Option<HashMap<usize, Object>>> = Mutex::new(None);
static NEXT_HANDLE: AtomicUsize = AtomicUsize::new(0x1000);

/// Fixed pseudo handles for the standard streams; never closed.
const STD_HANDLES: [usize; 3] = [0x10, 0x14, 0x18];

pub(crate) fn insert(o: Object) -> Handle {
    let h = NEXT_HANDLE.fetch_add(4, Ordering::Relaxed);
    HANDLES.lock().unwrap().get_or_insert_with(HashMap::new).insert(h, o);
    h as Handle
}

pub(crate) fn get(h: Handle) -> Option<Object> {
    if let Some(fd) = STD_HANDLES.iter().position(|&s| s == h as usize) {
        return Some(Object::Std(fd as i32));
    }
    HANDLES.lock().unwrap().as_ref()?.get(&(h as usize)).cloned()
}

fn bad_handle() -> Bool {
    set_last_error(ERROR_INVALID_HANDLE);
    FALSE
}

/// Map a Windows path from DLL code to a Linux path under the file root.
pub(crate) fn map_path(win: &str, root: Option<&Path>) -> Option<PathBuf> {
    let mut p = win.strip_prefix("\\\\?\\").unwrap_or(win);
    if p.len() >= FAKE_DIR.len() && p[..FAKE_DIR.len()].eq_ignore_ascii_case(FAKE_DIR) {
        p = &p[FAKE_DIR.len()..];
    }
    let rel = p.replace('\\', "/");
    let is_absolute = rel.starts_with('/') || rel.as_bytes().get(1) == Some(&b':');
    if is_absolute || rel.split('/').any(|c| c == "..") {
        return None;
    }
    let path = root?.join(&rel);
    if path.exists() {
        return Some(path);
    }
    // Case-insensitive match of the last component.
    let (dir, name) = (path.parent()?, path.file_name()?.to_str()?);
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|e| e.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case(name)))
}

const GENERIC_WRITE: u32 = 0x4000_0000;
const OPEN_EXISTING: u32 = 3;

pub unsafe extern "win64" fn CreateFileW(
    name: *const u16,
    access: u32,
    _share: u32,
    _security: *mut c_void,
    disposition: u32,
    _flags: u32,
    _template: Handle,
) -> Handle {
    let Some(name) = (unsafe { wide_to_string(name) }) else {
        set_last_error(ERROR_FILE_NOT_FOUND);
        return INVALID_HANDLE_VALUE;
    };
    if name.eq_ignore_ascii_case("CONOUT$") {
        return STD_HANDLES[2] as Handle;
    }
    if access & GENERIC_WRITE != 0 || disposition != OPEN_EXISTING {
        log::warn!("DLL tried to open {name:?} for writing; refused");
        set_last_error(ERROR_ACCESS_DENIED);
        return INVALID_HANDLE_VALUE;
    }
    let root = crate::file_root();
    let opened = map_path(&name, root.as_deref()).and_then(|p| File::open(&p).ok().map(|f| (p, f)));
    match opened {
        Some((path, f)) => {
            log::debug!("DLL opened {name:?} -> {}", path.display());
            insert(Object::File(Arc::new(Mutex::new(f))))
        }
        None => {
            log::debug!("DLL could not open {name:?}");
            set_last_error(ERROR_FILE_NOT_FOUND);
            INVALID_HANDLE_VALUE
        }
    }
}

pub unsafe extern "win64" fn ReadFile(h: Handle, buf: *mut u8, len: u32, read: *mut u32, _ov: *mut c_void) -> Bool {
    let Some(Object::File(f)) = get(h) else { return bad_handle() };
    let dst = unsafe { std::slice::from_raw_parts_mut(buf, len as usize) };
    let mut f = f.lock().unwrap();
    let mut total = 0;
    while total < dst.len() {
        match f.read(&mut dst[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return FALSE,
        }
    }
    if !read.is_null() {
        unsafe { *read = total as u32 };
    }
    TRUE
}

pub unsafe extern "win64" fn WriteFile(
    h: Handle,
    buf: *const u8,
    len: u32,
    written: *mut u32,
    _ov: *mut c_void,
) -> Bool {
    let Some(Object::Std(fd)) = get(h) else { return bad_handle() };
    let src = unsafe { std::slice::from_raw_parts(buf, len as usize) };
    // DLL console output goes to our stderr, never to stdout (which may carry data).
    let _ = fd;
    let _ = std::io::stderr().write_all(src);
    if !written.is_null() {
        unsafe { *written = len };
    }
    TRUE
}

pub unsafe extern "win64" fn SetFilePointerEx(h: Handle, dist: i64, new_pos: *mut i64, method: u32) -> Bool {
    let Some(Object::File(f)) = get(h) else { return bad_handle() };
    let pos = match method {
        0 => SeekFrom::Start(dist as u64),
        1 => SeekFrom::Current(dist),
        2 => SeekFrom::End(dist),
        _ => return bad_handle(),
    };
    match f.lock().unwrap().seek(pos) {
        Ok(p) => {
            if !new_pos.is_null() {
                unsafe { *new_pos = p as i64 };
            }
            TRUE
        }
        Err(_) => FALSE,
    }
}

pub unsafe extern "win64" fn GetFileSizeEx(h: Handle, size: *mut i64) -> Bool {
    let Some(Object::File(f)) = get(h) else { return bad_handle() };
    match f.lock().unwrap().metadata() {
        Ok(m) => {
            unsafe { *size = m.len() as i64 };
            TRUE
        }
        Err(_) => FALSE,
    }
}

const FILE_TYPE_UNKNOWN: u32 = 0;
const FILE_TYPE_DISK: u32 = 1;
const FILE_TYPE_CHAR: u32 = 2;

pub extern "win64" fn GetFileType(h: Handle) -> u32 {
    match get(h) {
        Some(Object::File(_)) => FILE_TYPE_DISK,
        Some(Object::Std(_)) => FILE_TYPE_CHAR,
        _ => {
            set_last_error(ERROR_INVALID_HANDLE);
            FILE_TYPE_UNKNOWN
        }
    }
}

pub extern "win64" fn CloseHandle(h: Handle) -> Bool {
    if STD_HANDLES.contains(&(h as usize)) {
        return TRUE;
    }
    match HANDLES.lock().unwrap().as_mut().and_then(|m| m.remove(&(h as usize))) {
        Some(_) => TRUE,
        None => bad_handle(),
    }
}

pub extern "win64" fn FlushFileBuffers(_h: Handle) -> Bool {
    TRUE
}

pub extern "win64" fn SetEndOfFile(_h: Handle) -> Bool {
    set_last_error(ERROR_ACCESS_DENIED);
    FALSE
}

pub extern "win64" fn GetStdHandle(which: u32) -> Handle {
    // STD_INPUT_HANDLE = -10, STD_OUTPUT_HANDLE = -11, STD_ERROR_HANDLE = -12
    match which as i32 {
        -10 => STD_HANDLES[0] as Handle,
        -11 => STD_HANDLES[1] as Handle,
        -12 => STD_HANDLES[2] as Handle,
        _ => INVALID_HANDLE_VALUE,
    }
}

pub extern "win64" fn SetStdHandle(_which: u32, _h: Handle) -> Bool {
    TRUE
}

const STARTUPINFOW_SIZE: usize = 104;

pub unsafe extern "win64" fn GetStartupInfoW(info: *mut u8) {
    unsafe {
        std::ptr::write_bytes(info, 0, STARTUPINFOW_SIZE);
        *(info as *mut u32) = STARTUPINFOW_SIZE as u32;
    }
}

pub extern "win64" fn GetConsoleMode(_h: Handle, _mode: *mut u32) -> Bool {
    FALSE
}

pub extern "win64" fn GetConsoleOutputCP() -> u32 {
    65001
}

pub unsafe extern "win64" fn WriteConsoleW(
    _h: Handle,
    buf: *const u16,
    n: u32,
    written: *mut u32,
    _r: *mut c_void,
) -> Bool {
    let s = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(buf, n as usize) });
    let _ = std::io::stderr().write_all(s.as_bytes());
    if !written.is_null() {
        unsafe { *written = n };
    }
    TRUE
}

pub extern "win64" fn ReadConsoleW(_h: Handle, _buf: *mut u16, _n: u32, _read: *mut u32, _c: *mut c_void) -> Bool {
    set_last_error(ERROR_CALL_NOT_IMPLEMENTED);
    FALSE
}

pub extern "win64" fn FindFirstFileExW(
    _name: *const u16,
    _level: u32,
    _data: *mut c_void,
    _op: u32,
    _filter: *mut c_void,
    _flags: u32,
) -> Handle {
    set_last_error(ERROR_FILE_NOT_FOUND);
    INVALID_HANDLE_VALUE
}

pub extern "win64" fn FindNextFileW(_h: Handle, _data: *mut c_void) -> Bool {
    set_last_error(ERROR_FILE_NOT_FOUND);
    FALSE
}

pub extern "win64" fn FindClose(_h: Handle) -> Bool {
    TRUE
}

const S_OK: i32 = 0;
const S_FALSE: i32 = 1;
const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;
const E_INSUFFICIENT_BUFFER: i32 = 0x8007_007a_u32 as i32;

/// Remove the last path component: `C:\a\b.dll` -> `C:\a`, `C:\a` -> `C:\`; S_FALSE if nothing was removed.
pub unsafe extern "win64" fn PathCchRemoveFileSpec(path: *mut u16, cap: usize) -> i32 {
    if path.is_null() || cap == 0 {
        return E_INVALIDARG;
    }
    let len = unsafe { super::wcslen(path) }.min(cap);
    let s = unsafe { std::slice::from_raw_parts_mut(path, len) };
    let is_sep = |c: u16| c == b'\\' as u16 || c == b'/' as u16;
    // Keep a drive root such as `C:\`.
    let root = if len >= 3 && s[1] == b':' as u16 && is_sep(s[2]) { 3 } else { 0 };
    let Some(i) = s.iter().rposition(|&c| is_sep(c)) else { return S_FALSE };
    let cut = if i < root { root } else { i };
    if cut >= len {
        return S_FALSE;
    }
    unsafe { *path.add(cut) = 0 };
    S_OK
}

/// Append a backslash unless the path is empty or already ends with one (S_FALSE then).
pub unsafe extern "win64" fn PathCchAddBackslash(path: *mut u16, cap: usize) -> i32 {
    if path.is_null() {
        return E_INVALIDARG;
    }
    let len = unsafe { super::wcslen(path) };
    if len == 0 || unsafe { *path.add(len - 1) } == b'\\' as u16 {
        return S_FALSE;
    }
    if len + 2 > cap {
        return E_INSUFFICIENT_BUFFER;
    }
    unsafe {
        *path.add(len) = b'\\' as u16;
        *path.add(len + 1) = 0;
    }
    S_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(s: &str, cap: usize) -> Vec<u16> {
        let mut v: Vec<u16> = s.encode_utf16().collect();
        v.resize(cap, 0);
        v
    }

    fn text(v: &[u16]) -> String {
        String::from_utf16_lossy(&v[..v.iter().position(|&c| c == 0).unwrap()])
    }

    #[test]
    fn remove_file_spec() {
        for (input, out, hr) in [
            ("C:\\acerultrax\\x.dll", "C:\\acerultrax", S_OK),
            ("C:\\acerultrax", "C:\\", S_OK),
            ("C:\\", "C:\\", S_FALSE),
            ("", "", S_FALSE),
        ] {
            let mut v = wide(input, 64);
            assert_eq!(unsafe { PathCchRemoveFileSpec(v.as_mut_ptr(), 64) }, hr, "{input}");
            assert_eq!(text(&v), out, "{input}");
        }
    }

    #[test]
    fn add_backslash() {
        let mut v = wide("C:\\acerultrax", 64);
        assert_eq!(unsafe { PathCchAddBackslash(v.as_mut_ptr(), 64) }, S_OK);
        assert_eq!(text(&v), "C:\\acerultrax\\");
        assert_eq!(unsafe { PathCchAddBackslash(v.as_mut_ptr(), 64) }, S_FALSE);
        let mut e = wide("", 4);
        assert_eq!(unsafe { PathCchAddBackslash(e.as_mut_ptr(), 4) }, S_FALSE);
    }

    #[test]
    fn path_mapping() {
        let dir = std::env::temp_dir().join(format!("pe-loader-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Tuning_A.bin"), b"x").unwrap();
        let root = Some(dir.as_path());
        assert_eq!(map_path("C:\\acerultrax\\Tuning_A.bin", root), Some(dir.join("Tuning_A.bin")));
        assert_eq!(map_path("tuning_a.BIN", root), Some(dir.join("Tuning_A.bin")));
        assert_eq!(map_path("..\\etc\\passwd", root), None);
        assert_eq!(map_path("D:\\other\\Tuning_A.bin", root), None);
        assert_eq!(map_path("Tuning_A.bin", None), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
