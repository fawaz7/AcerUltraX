//! The parts of the Universal C Runtime (`api-ms-win-crt-*`) DLLs built against the shared CRT import: heap,
//! start-up and exit tables, a string function and maths. All Microsoft x64 ABI on top of libc / Rust's libm.

#![allow(non_snake_case)]

use std::ffi::c_void;

use super::fatal;

pub unsafe extern "win64" fn malloc(n: usize) -> *mut c_void {
    unsafe { libc::malloc(n.max(1)) }
}

pub unsafe extern "win64" fn calloc(n: usize, size: usize) -> *mut c_void {
    unsafe { libc::calloc(n.max(1), size.max(1)) }
}

pub unsafe extern "win64" fn free(p: *mut c_void) {
    unsafe { libc::free(p) }
}

pub unsafe extern "win64" fn _aligned_malloc(n: usize, align: usize) -> *mut c_void {
    let mut p = std::ptr::null_mut();
    let align = align.max(std::mem::size_of::<usize>()).next_power_of_two();
    if unsafe { libc::posix_memalign(&mut p, align, n.max(1)) } != 0 {
        return std::ptr::null_mut();
    }
    p
}

pub unsafe extern "win64" fn _aligned_free(p: *mut c_void) {
    unsafe { libc::free(p) }
}

/// No new-handler: `operator new` reports out of memory.
pub extern "win64" fn _callnewh(_size: usize) -> i32 {
    0
}

pub extern "win64" fn terminate() -> ! {
    fatal("DLL called terminate()")
}

pub extern "win64" fn abort() -> ! {
    fatal("DLL called abort()")
}

pub extern "win64" fn _configure_narrow_argv(_mode: i32) -> i32 {
    0
}

pub extern "win64" fn _initialize_narrow_environment() -> i32 {
    0
}

/// An onexit table is three pointers (first, last, end). Exit callbacks are never run: modules stay loaded.
pub unsafe extern "win64" fn _initialize_onexit_table(table: *mut [usize; 3]) -> i32 {
    unsafe { *table = [0; 3] };
    0
}

pub extern "win64" fn _register_onexit_function(_table: *mut c_void, _f: *mut c_void) -> i32 {
    0
}

pub extern "win64" fn _execute_onexit_table(_table: *mut c_void) -> i32 {
    0
}

pub extern "win64" fn _crt_atexit(_f: *mut c_void) -> i32 {
    0
}

pub extern "win64" fn _cexit() {}

/// Run the C++ static initialisers between `begin` and `end`.
pub unsafe extern "win64" fn _initterm(begin: *const usize, end: *const usize) {
    let mut p = begin;
    while p < end {
        let f = unsafe { *p };
        if f != 0 {
            // SAFETY: the table holds `void (*)(void)` pointers into the module.
            unsafe { std::mem::transmute::<usize, unsafe extern "win64" fn()>(f)() };
        }
        p = unsafe { p.add(1) };
    }
}

/// Like `_initterm` for `int (*)(void)` initialisers; stops at the first failure and returns it.
pub unsafe extern "win64" fn _initterm_e(begin: *const usize, end: *const usize) -> i32 {
    let mut p = begin;
    while p < end {
        let f = unsafe { *p };
        if f != 0 {
            let r = unsafe { std::mem::transmute::<usize, unsafe extern "win64" fn() -> i32>(f)() };
            if r != 0 {
                return r;
            }
        }
        p = unsafe { p.add(1) };
    }
    0
}

pub extern "win64" fn _seh_filter_dll(_code: u32, _info: *mut c_void) -> i32 {
    0 // EXCEPTION_CONTINUE_SEARCH
}

const EINVAL: i32 = 22;
const ERANGE: i32 = 34;

pub unsafe extern "win64" fn strcpy_s(dst: *mut u8, size: usize, src: *const u8) -> i32 {
    if dst.is_null() || size == 0 {
        return EINVAL;
    }
    if src.is_null() {
        unsafe { *dst = 0 };
        return EINVAL;
    }
    let n = unsafe { libc::strlen(src.cast()) };
    if n >= size {
        unsafe { *dst = 0 };
        return ERANGE;
    }
    unsafe { std::ptr::copy_nonoverlapping(src, dst, n + 1) };
    0
}

pub extern "win64" fn sin(x: f64) -> f64 {
    x.sin()
}

pub extern "win64" fn exp(x: f64) -> f64 {
    x.exp()
}

pub extern "win64" fn powf(x: f32, y: f32) -> f32 {
    x.powf(y)
}

pub extern "win64" fn tanf(x: f32) -> f32 {
    x.tan()
}

pub extern "win64" fn cosf(x: f32) -> f32 {
    x.cos()
}

pub extern "win64" fn atan2f(y: f32, x: f32) -> f32 {
    y.atan2(x)
}

/// `long` is 32 bits on Windows.
pub extern "win64" fn lround(x: f64) -> i32 {
    x.round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strcpy_s_bounds() {
        let mut buf = [0xffu8; 4];
        assert_eq!(unsafe { strcpy_s(buf.as_mut_ptr(), 4, c"abc".as_ptr().cast()) }, 0);
        assert_eq!(&buf, b"abc\0");
        assert_eq!(unsafe { strcpy_s(buf.as_mut_ptr(), 4, c"abcd".as_ptr().cast()) }, ERANGE);
        assert_eq!(buf[0], 0);
        assert_eq!(lround(2.5), 3);
        assert_eq!(lround(-2.5), -3);
    }
}
