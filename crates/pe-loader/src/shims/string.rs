//! Command line, environment and code pages. The process code page is UTF-8 (65001), which keeps the C runtime
//! on its simple UTF-8 paths and away from the locale tables.

#![allow(non_snake_case)]

use std::ffi::c_void;
use std::sync::OnceLock;

use super::{Bool, ERROR_CALL_NOT_IMPLEMENTED, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_PARAMETER};
use super::{ERROR_NO_UNICODE_TRANSLATION, FALSE, TRUE, set_last_error, wcslen};

const CP_UTF8: u32 = 65001;
const MB_ERR_INVALID_CHARS: u32 = 0x08;

pub extern "win64" fn GetCommandLineA() -> *const u8 {
    c"acerultrax".as_ptr().cast()
}

pub extern "win64" fn GetCommandLineW() -> *const u16 {
    static W: OnceLock<Vec<u16>> = OnceLock::new();
    W.get_or_init(|| "acerultrax\0".encode_utf16().collect()).as_ptr()
}

/// An empty environment: the host's environment is not exposed to DLL code.
pub extern "win64" fn GetEnvironmentStringsW() -> *const u16 {
    static EMPTY: [u16; 2] = [0, 0];
    EMPTY.as_ptr()
}

pub extern "win64" fn FreeEnvironmentStringsW(_p: *const u16) -> Bool {
    TRUE
}

fn utf8_code_page(cp: u32) -> bool {
    // CP_ACP, CP_OEMCP, CP_MACCP, CP_THREAD_ACP all mean the process code page, which is UTF-8.
    matches!(cp, 0 | 1 | 2 | 3 | CP_UTF8)
}

/// Store `out` in the caller's buffer, or report the needed size when `cap` is 0.
unsafe fn store<T: Copy>(out: &[T], dst: *mut T, cap: i32) -> i32 {
    if cap == 0 {
        return out.len() as i32;
    }
    if (cap as usize) < out.len() {
        set_last_error(ERROR_INSUFFICIENT_BUFFER);
        return 0;
    }
    unsafe { std::ptr::copy_nonoverlapping(out.as_ptr(), dst, out.len()) };
    out.len() as i32
}

pub unsafe extern "win64" fn MultiByteToWideChar(
    cp: u32,
    flags: u32,
    src: *const u8,
    len: i32,
    dst: *mut u16,
    cap: i32,
) -> i32 {
    if !utf8_code_page(cp) || src.is_null() || len == 0 || cap < 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // A length of -1 means NUL-terminated, and the NUL is converted too.
    let n = if len < 0 { (unsafe { libc::strlen(src.cast()) }) + 1 } else { len as usize };
    let bytes = unsafe { std::slice::from_raw_parts(src, n) };
    let text = match std::str::from_utf8(bytes) {
        Ok(s) => std::borrow::Cow::Borrowed(s),
        Err(_) if flags & MB_ERR_INVALID_CHARS != 0 => {
            set_last_error(ERROR_NO_UNICODE_TRANSLATION);
            return 0;
        }
        Err(_) => String::from_utf8_lossy(bytes),
    };
    let wide: Vec<u16> = text.encode_utf16().collect();
    unsafe { store(&wide, dst, cap) }
}

#[allow(clippy::too_many_arguments)]
pub unsafe extern "win64" fn WideCharToMultiByte(
    cp: u32,
    _flags: u32,
    src: *const u16,
    len: i32,
    dst: *mut u8,
    cap: i32,
    _default_char: *const u8,
    used_default: *mut Bool,
) -> i32 {
    if !utf8_code_page(cp) || src.is_null() || len == 0 || cap < 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let n = if len < 0 { (unsafe { wcslen(src) }) + 1 } else { len as usize };
    let units = unsafe { std::slice::from_raw_parts(src, n) };
    let text = String::from_utf16_lossy(units);
    if !used_default.is_null() {
        unsafe { *used_default = FALSE };
    }
    unsafe { store(text.as_bytes(), dst, cap) }
}

pub extern "win64" fn GetACP() -> u32 {
    CP_UTF8
}

pub extern "win64" fn GetOEMCP() -> u32 {
    CP_UTF8
}

pub extern "win64" fn IsValidCodePage(cp: u32) -> Bool {
    (cp == CP_UTF8) as Bool
}

/// Only needed for single-byte code pages, which are never in use.
pub extern "win64" fn GetCPInfo(cp: u32, _info: *mut c_void) -> Bool {
    log::debug!("GetCPInfo({cp}) not supported");
    set_last_error(ERROR_INVALID_PARAMETER);
    FALSE
}

// Locale functions: only reached by `setlocale` with a named locale, which the supported DLLs never do.

fn no_locale(what: &str) -> i32 {
    log::warn!("DLL called {what}, which is not supported");
    set_last_error(ERROR_CALL_NOT_IMPLEMENTED);
    0
}

pub extern "win64" fn LCMapStringW(_lcid: u32, _f: u32, _s: *const u16, _n: i32, _d: *mut u16, _c: i32) -> i32 {
    no_locale("LCMapStringW")
}

#[allow(clippy::too_many_arguments)]
pub extern "win64" fn LCMapStringEx(
    _name: *const u16,
    _f: u32,
    _s: *const u16,
    _n: i32,
    _d: *mut u16,
    _c: i32,
    _v: *mut c_void,
    _r: *mut c_void,
    _p: isize,
) -> i32 {
    no_locale("LCMapStringEx")
}

pub extern "win64" fn GetStringTypeW(_t: u32, _s: *const u16, _n: i32, _out: *mut u16) -> Bool {
    no_locale("GetStringTypeW")
}

pub extern "win64" fn GetLocaleInfoW(_lcid: u32, _t: u32, _d: *mut u16, _c: i32) -> i32 {
    no_locale("GetLocaleInfoW")
}

pub extern "win64" fn IsValidLocale(_lcid: u32, _f: u32) -> Bool {
    FALSE
}

pub extern "win64" fn GetUserDefaultLCID() -> u32 {
    0x0409
}

pub extern "win64" fn EnumSystemLocalesW(_f: *mut c_void, _flags: u32) -> Bool {
    no_locale("EnumSystemLocalesW")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_round_trip() {
        let src = "Ä tuning\0";
        unsafe {
            let need = MultiByteToWideChar(CP_UTF8, 0, src.as_ptr(), -1, std::ptr::null_mut(), 0);
            assert_eq!(need, 9);
            let mut w = vec![0u16; need as usize];
            assert_eq!(MultiByteToWideChar(0, 0, src.as_ptr(), -1, w.as_mut_ptr(), need), 9);
            assert_eq!(w.last(), Some(&0));
            let mut b = vec![0u8; 16];
            let n = WideCharToMultiByte(
                CP_UTF8,
                0,
                w.as_ptr(),
                -1,
                b.as_mut_ptr(),
                16,
                std::ptr::null(),
                std::ptr::null_mut(),
            );
            assert_eq!(&b[..n as usize], src.as_bytes());
            assert_eq!(MultiByteToWideChar(CP_UTF8, 0, src.as_ptr(), -1, w.as_mut_ptr(), 2), 0);
            let bad = [0xffu8, 0];
            assert_eq!(MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bad.as_ptr(), -1, w.as_mut_ptr(), 9), 0);
        }
    }
}
