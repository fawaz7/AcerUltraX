//! Module handles and names.
//!
//! No other DLLs exist: `kernel32.dll` (and the API set it forwards for) gets a fake handle whose only use is
//! `GetProcAddress` of a few optional functions; everything else is "not found". Module file names live under
//! the fake directory [`FAKE_DIR`], which the file functions map to the configured file root, so DLL code that
//! builds paths from its own location finds its data files.

#![allow(non_snake_case)]

use std::ffi::c_void;
use std::sync::OnceLock;

use super::{Bool, ERROR_MOD_NOT_FOUND, ERROR_PROC_NOT_FOUND, FALSE, Handle, TRUE, c_to_string, set_last_error};
use super::{wide_to_string, write_wide};
use crate::{image, thread};

/// Windows directory the host executable and loaded DLLs appear to live in.
pub(crate) const FAKE_DIR: &str = "C:\\acerultrax\\";
const FAKE_EXE_NAME: &str = "acerultrax.exe";

/// Readable zeroed pages standing in for the host executable and kernel32, in case code peeks at their headers.
fn fake_handles() -> (usize, usize) {
    static H: OnceLock<(usize, usize)> = OnceLock::new();
    *H.get_or_init(|| {
        let p = thread::zeroed_pages(0x2000);
        (p, p + 0x1000)
    })
}

fn exe_handle() -> Handle {
    fake_handles().0 as Handle
}

fn kernel32_handle() -> Handle {
    fake_handles().1 as Handle
}

const KERNEL32_NAMES: &[&str] = &["kernel32.dll", "kernel32", "api-ms-win-core-synch-l1-2-0.dll"];

pub unsafe extern "win64" fn GetModuleHandleW(name: *const u16) -> Handle {
    let Some(name) = (unsafe { wide_to_string(name) }) else {
        return exe_handle();
    };
    if KERNEL32_NAMES.iter().any(|k| k.eq_ignore_ascii_case(&name)) {
        return kernel32_handle();
    }
    set_last_error(ERROR_MOD_NOT_FOUND);
    std::ptr::null_mut()
}

pub unsafe extern "win64" fn GetModuleHandleExW(_flags: u32, _name: *const u16, out: *mut Handle) -> Bool {
    if !out.is_null() {
        unsafe { *out = std::ptr::null_mut() };
    }
    set_last_error(ERROR_MOD_NOT_FOUND);
    FALSE
}

pub unsafe extern "win64" fn GetModuleFileNameW(h: Handle, buf: *mut u16, cap: u32) -> u32 {
    let name = if h.is_null() || h == exe_handle() {
        FAKE_EXE_NAME.to_string()
    } else if let Some(m) = image::module_at(h as usize) {
        m.name().to_string()
    } else {
        set_last_error(ERROR_MOD_NOT_FOUND);
        return 0;
    };
    unsafe { write_wide(&format!("{FAKE_DIR}{name}"), buf, cap) }
}

pub extern "win64" fn LoadLibraryExW(_name: *const u16, _file: Handle, _flags: u32) -> Handle {
    set_last_error(ERROR_MOD_NOT_FOUND);
    std::ptr::null_mut()
}

pub extern "win64" fn FreeLibrary(_h: Handle) -> Bool {
    TRUE
}

/// Functions the C runtime looks up dynamically and may use when present.
const DYNAMIC: &[&str] = &["SleepConditionVariableCS", "WakeAllConditionVariable"];

pub unsafe extern "win64" fn GetProcAddress(h: Handle, name: *const u8) -> *mut c_void {
    // Names below 0x10000 are ordinals.
    if h == kernel32_handle() && name as usize >= 0x10000 {
        if let Some(name) = unsafe { c_to_string(name) } {
            if DYNAMIC.contains(&name.as_str()) {
                if let Some(f) = super::lookup(&name) {
                    return f as *mut c_void;
                }
            }
        }
    }
    set_last_error(ERROR_PROC_NOT_FOUND);
    std::ptr::null_mut()
}
