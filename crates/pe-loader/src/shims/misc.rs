//! Process/thread identity, time, fail-fast and exception paths, pointer encoding, registry and debug output.

#![allow(non_snake_case)]

use std::ffi::c_void;

use super::{Bool, FALSE, Handle, TRUE, c_to_string, fatal};
use crate::image;

pub extern "win64" fn GetCurrentProcessId() -> u32 {
    unsafe { libc::getpid() as u32 }
}

pub extern "win64" fn GetCurrentThreadId() -> u32 {
    unsafe { libc::gettid() as u32 }
}

pub extern "win64" fn GetCurrentProcess() -> Handle {
    usize::MAX as Handle
}

fn clock(id: libc::clockid_t) -> (u64, u64) {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(id, &mut ts) };
    (ts.tv_sec as u64, ts.tv_nsec as u64)
}

/// Nanoseconds of a monotonic clock (only used to seed the CRT's security cookie).
pub unsafe extern "win64" fn QueryPerformanceCounter(out: *mut i64) -> Bool {
    let (s, ns) = clock(libc::CLOCK_MONOTONIC);
    unsafe { *out = (s * 1_000_000_000 + ns) as i64 };
    TRUE
}

/// 100 ns intervals since 1601-01-01.
pub unsafe extern "win64" fn GetSystemTimeAsFileTime(out: *mut u64) {
    let (s, ns) = clock(libc::CLOCK_REALTIME);
    unsafe { *out = s * 10_000_000 + ns / 100 + 116_444_736_000_000_000 };
}

pub extern "win64" fn TerminateProcess(_process: Handle, code: u32) -> Bool {
    fatal(&format!("DLL called TerminateProcess({code:#x})"))
}

pub extern "win64" fn ExitProcess(code: u32) -> ! {
    fatal(&format!("DLL called ExitProcess({code:#x})"))
}

/// 0 for every feature: in particular PF_FASTFAIL_AVAILABLE (23) must be absent, because `int 0x29` would crash
/// without a log; the CRT then takes its TerminateProcess path, which logs.
pub extern "win64" fn IsProcessorFeaturePresent(_feature: u32) -> Bool {
    FALSE
}

pub extern "win64" fn IsDebuggerPresent() -> Bool {
    FALSE
}

pub extern "win64" fn UnhandledExceptionFilter(_info: *mut c_void) -> i32 {
    log::error!("DLL reported an unhandled exception");
    0 // EXCEPTION_CONTINUE_SEARCH
}

pub extern "win64" fn SetUnhandledExceptionFilter(_filter: *mut c_void) -> *mut c_void {
    std::ptr::null_mut()
}

const CONTEXT_SIZE: usize = 0x4d0;

pub unsafe extern "win64" fn RtlCaptureContext(ctx: *mut c_void) {
    unsafe { std::ptr::write_bytes(ctx as *mut u8, 0, CONTEXT_SIZE) };
}

/// No unwind information is exposed: exceptions are not supported, so nothing may unwind through DLL frames.
pub unsafe extern "win64" fn RtlLookupFunctionEntry(_pc: u64, base: *mut u64, _history: *mut c_void) -> *mut c_void {
    if !base.is_null() {
        unsafe { *base = 0 };
    }
    std::ptr::null_mut()
}

pub extern "win64" fn RtlVirtualUnwind() -> *mut c_void {
    fatal("DLL tried to unwind the stack (RtlVirtualUnwind)")
}

pub extern "win64" fn RtlUnwindEx() {
    fatal("DLL tried to unwind the stack (RtlUnwindEx)")
}

pub extern "win64" fn RtlUnwind() {
    fatal("DLL tried to unwind the stack (RtlUnwind)")
}

pub unsafe extern "win64" fn RtlPcToFileHeader(pc: *mut c_void, base: *mut *mut c_void) -> *mut c_void {
    let b = image::module_containing(pc as usize).map_or(0, |m| m.base()) as *mut c_void;
    unsafe { *base = b };
    b
}

const MSVC_CXX_EXCEPTION: u32 = 0xe06d_7363;

/// Name of the C++ type thrown by `_CxxThrowException`, from its ThrowInfo (all fields image-relative).
unsafe fn cxx_type_name(args: &[usize]) -> Option<String> {
    let (throw_info, base) = (*args.get(2)?, *args.get(3)?);
    if throw_info == 0 || base == 0 {
        return None;
    }
    unsafe {
        let rd = |p: usize| *(p as *const u32) as usize;
        let catchable_array = base + rd(throw_info + 0x0c);
        let first = base + rd(catchable_array + 4);
        let type_desc = base + rd(first + 4);
        c_to_string((type_desc + 0x10) as *const u8)
    }
}

/// C++ exceptions and SEH are only raised on error paths of the supported DLLs; there is no unwinder, so log as
/// much as possible and abort.
pub unsafe extern "win64" fn RaiseException(code: u32, flags: u32, nargs: u32, args: *const usize) -> ! {
    let args = if args.is_null() { &[][..] } else { unsafe { std::slice::from_raw_parts(args, nargs as usize) } };
    let what = if code == MSVC_CXX_EXCEPTION {
        format!("C++ exception {}", unsafe { cxx_type_name(args) }.unwrap_or_else(|| "(unknown type)".into()))
    } else {
        format!("exception {code:#x} (flags {flags:#x}, args {args:x?})")
    };
    fatal(&format!("DLL raised {what}"))
}

pub extern "win64" fn EncodePointer(p: *mut c_void) -> *mut c_void {
    p
}

pub extern "win64" fn DecodePointer(p: *mut c_void) -> *mut c_void {
    p
}

const ERROR_FILE_NOT_FOUND: i32 = 2;

/// There is no registry: every key is absent.
pub unsafe extern "win64" fn RegOpenKeyExW(
    _key: Handle,
    _sub: *const u16,
    _opts: u32,
    _sam: u32,
    out: *mut Handle,
) -> i32 {
    if !out.is_null() {
        unsafe { *out = std::ptr::null_mut() };
    }
    ERROR_FILE_NOT_FOUND
}

pub extern "win64" fn RegQueryValueExW(
    _key: Handle,
    _name: *const u16,
    _reserved: *mut u32,
    _ty: *mut u32,
    _data: *mut u8,
    _len: *mut u32,
) -> i32 {
    ERROR_FILE_NOT_FOUND
}

pub extern "win64" fn RegCloseKey(_key: Handle) -> i32 {
    0
}

pub unsafe extern "win64" fn OutputDebugStringA(s: *const u8) {
    if let Some(s) = unsafe { c_to_string(s) } {
        log::debug!("DLL: {}", s.trim_end());
    }
}

const PAGE_READONLY: u32 = 0x02;

/// Newer MSVC runtimes make their read-only `.fptable` section writable for a moment to fill it in.
// shortcut: the old protection is always reported as read-only (right for data sections); track real
// protections if a DLL ever re-protects code with it.
pub unsafe extern "win64" fn VirtualProtect(addr: *mut c_void, size: usize, new: u32, old: *mut u32) -> Bool {
    let prot = match new & 0xff {
        0x01 => libc::PROT_NONE,
        0x02 => libc::PROT_READ,
        0x04 | 0x08 => libc::PROT_READ | libc::PROT_WRITE,
        0x10 | 0x20 => libc::PROT_READ | libc::PROT_EXEC,
        0x40 | 0x80 => libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
        _ => return FALSE,
    };
    log::debug!("VirtualProtect({addr:?}, {size:#x}, {new:#x})");
    let start = addr as usize & !0xfff;
    let end = (addr as usize + size.max(1) + 0xfff) & !0xfff;
    // SAFETY: the DLL asks for its own pages, as on Windows.
    if unsafe { libc::mprotect(start as *mut c_void, end - start, prot) } != 0 {
        return FALSE;
    }
    if !old.is_null() {
        unsafe { *old = PAGE_READONLY };
    }
    TRUE
}
