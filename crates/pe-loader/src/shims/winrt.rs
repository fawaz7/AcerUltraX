//! What WinRT/COM components need: HSTRINGs, slim reader-writer locks, one-time initialisation, a few COM
//! helpers, error origination and event tracing (no-ops: nothing listens).

#![allow(non_snake_case)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{Bool, FALSE, TRUE};

const S_OK: i32 = 0;
const E_NOINTERFACE: i32 = 0x8000_4002_u32 as i32;
const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;
const E_OUTOFMEMORY: i32 = 0x8007_000e_u32 as i32;
const REGDB_E_CLASSNOTREG: i32 = 0x8004_0154_u32 as i32;

// ------------------------------------------------------------------------------------------------ HSTRING
//
// An HSTRING is an opaque pointer. Ours points at this header: either inside a caller-provided HSTRING_HEADER
// (string references, 24 bytes, not owned) or in a heap allocation followed by the characters.

#[repr(C)]
struct HString {
    reference: u32,
    len: u32,
    chars: *const u16,
}

const _: () = assert!(std::mem::size_of::<HString>() <= 24);

static EMPTY: [u16; 1] = [0];

unsafe fn hstr<'a>(h: *const c_void) -> Option<&'a HString> {
    unsafe { (h as *const HString).as_ref() }
}

pub unsafe extern "win64" fn WindowsCreateString(src: *const u16, len: u32, out: *mut *mut c_void) -> i32 {
    if out.is_null() || (src.is_null() && len > 0) {
        return E_INVALIDARG;
    }
    if len == 0 {
        unsafe { *out = std::ptr::null_mut() };
        return S_OK;
    }
    let size = std::mem::size_of::<HString>() + (len as usize + 1) * 2;
    let p = unsafe { libc::malloc(size) } as *mut HString;
    if p.is_null() {
        return E_OUTOFMEMORY;
    }
    unsafe {
        let chars = (p as *mut u8).add(std::mem::size_of::<HString>()) as *mut u16;
        std::ptr::copy_nonoverlapping(src, chars, len as usize);
        *chars.add(len as usize) = 0;
        p.write(HString { reference: 0, len, chars });
        *out = p.cast();
    }
    S_OK
}

/// A string reference lives in the caller's header and points at the caller's (NUL-terminated) characters.
pub unsafe extern "win64" fn WindowsCreateStringReference(
    src: *const u16,
    len: u32,
    header: *mut c_void,
    out: *mut *mut c_void,
) -> i32 {
    if out.is_null() || header.is_null() || (src.is_null() && len > 0) {
        return E_INVALIDARG;
    }
    if len == 0 {
        unsafe { *out = std::ptr::null_mut() };
        return S_OK;
    }
    unsafe {
        (header as *mut HString).write(HString { reference: 1, len, chars: src });
        *out = header;
    }
    S_OK
}

pub unsafe extern "win64" fn WindowsGetStringRawBuffer(h: *const c_void, len: *mut u32) -> *const u16 {
    let s = unsafe { hstr(h) };
    if !len.is_null() {
        unsafe { *len = s.map_or(0, |s| s.len) };
    }
    s.map_or(EMPTY.as_ptr(), |s| s.chars)
}

pub unsafe extern "win64" fn WindowsIsStringEmpty(h: *const c_void) -> Bool {
    (unsafe { hstr(h) }.is_none_or(|s| s.len == 0)) as Bool
}

pub unsafe extern "win64" fn WindowsStringHasEmbeddedNull(h: *const c_void, out: *mut Bool) -> i32 {
    let s = unsafe { hstr(h) };
    let has = s.is_some_and(|s| unsafe { std::slice::from_raw_parts(s.chars, s.len as usize) }.contains(&0));
    if !out.is_null() {
        unsafe { *out = has as Bool };
    }
    S_OK
}

/// Make an HSTRING for a Rust string (for calling WinRT entry points); free it with [`delete_hstring`].
pub fn hstring(s: &str) -> *mut c_void {
    let w: Vec<u16> = s.encode_utf16().collect();
    let mut out = std::ptr::null_mut();
    unsafe { WindowsCreateString(w.as_ptr(), w.len() as u32, &mut out) };
    out
}

pub fn delete_hstring(h: *mut c_void) {
    if let Some(s) = unsafe { hstr(h) } {
        if s.reference == 0 {
            unsafe { libc::free(h) };
        }
    }
}

// ------------------------------------------------------------------------------------------------ SRW locks
//
// SRWLOCK is one pointer, zero-initialised: bit 0 = held exclusively, higher bits = number of shared holders.
// Waiters spin and yield; these locks guard short sections that are rarely contended.

unsafe fn lock_word<'a>(l: *mut c_void) -> &'a AtomicUsize {
    unsafe { AtomicUsize::from_ptr(l as *mut usize) }
}

pub extern "win64" fn InitializeSRWLock(l: *mut c_void) {
    unsafe { lock_word(l) }.store(0, Ordering::Release);
}

pub extern "win64" fn AcquireSRWLockExclusive(l: *mut c_void) {
    let w = unsafe { lock_word(l) };
    while w.compare_exchange_weak(0, 1, Ordering::Acquire, Ordering::Relaxed).is_err() {
        std::thread::yield_now();
    }
}

pub extern "win64" fn ReleaseSRWLockExclusive(l: *mut c_void) {
    unsafe { lock_word(l) }.store(0, Ordering::Release);
}

pub extern "win64" fn AcquireSRWLockShared(l: *mut c_void) {
    let w = unsafe { lock_word(l) };
    loop {
        let v = w.load(Ordering::Relaxed);
        if v & 1 == 0 && w.compare_exchange_weak(v, v + 2, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            return;
        }
        std::thread::yield_now();
    }
}

pub extern "win64" fn ReleaseSRWLockShared(l: *mut c_void) {
    unsafe { lock_word(l) }.fetch_sub(2, Ordering::Release);
}

// ------------------------------------------------------------------------------------------------ InitOnce

type InitFn = unsafe extern "win64" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> Bool;

/// INIT_ONCE is one pointer: 0 = not run, 1 = running, 2 = done (the context value is not kept).
pub unsafe extern "win64" fn InitOnceExecuteOnce(
    once: *mut c_void,
    f: InitFn,
    param: *mut c_void,
    ctx: *mut *mut c_void,
) -> Bool {
    let w = unsafe { lock_word(once) };
    loop {
        match w.compare_exchange(0, 1, Ordering::Acquire, Ordering::Acquire) {
            Ok(_) => {
                let ok = unsafe { f(once, param, ctx) } != 0;
                w.store(if ok { 2 } else { 0 }, Ordering::Release);
                return ok as Bool;
            }
            Err(2) => return TRUE,
            Err(_) => std::thread::yield_now(),
        }
    }
}

// ------------------------------------------------------------------------------------------------ COM

/// A minimal free-threaded marshaler: an IUnknown that answers no interfaces. Components only aggregate it.
#[repr(C)]
struct Unknown {
    vtbl: *const [usize; 3],
}

unsafe extern "win64" fn unk_qi(_this: *mut c_void, _iid: *const c_void, out: *mut *mut c_void) -> i32 {
    unsafe { *out = std::ptr::null_mut() };
    E_NOINTERFACE
}

unsafe extern "win64" fn unk_ref(_this: *mut c_void) -> u32 {
    1
}

static UNKNOWN_VTBL: [usize; 3] = [0; 3];
static FTM: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

pub unsafe extern "win64" fn CoCreateFreeThreadedMarshaler(_outer: *mut c_void, out: *mut *mut c_void) -> i32 {
    let obj = *FTM.get_or_init(|| {
        // Function pointers are not const-evaluable, so the vtable is filled at first use.
        let vt: &'static mut [usize; 3] = Box::leak(Box::new(UNKNOWN_VTBL));
        vt[0] = unk_qi as *const () as usize;
        vt[1] = unk_ref as *const () as usize;
        vt[2] = unk_ref as *const () as usize;
        Box::leak(Box::new(Unknown { vtbl: vt })) as *mut Unknown as usize
    });
    unsafe { *out = obj as *mut c_void };
    S_OK
}

/// No COM classes are registered (the encoder only asks for resamplers, which it skips at 48 kHz).
pub unsafe extern "win64" fn CoCreateInstanceFromApp(
    _clsid: *const c_void,
    _outer: *mut c_void,
    _ctx: u32,
    _reserved: *mut c_void,
    _count: u32,
    _results: *mut c_void,
) -> i32 {
    log::warn!("DLL asked for a COM class; none are available");
    REGDB_E_CLASSNOTREG
}

/// PROPVARIANT is 24 bytes; only the blobs the host passes in are inside, and those are owned by the host.
pub unsafe extern "win64" fn PropVariantClear(pv: *mut c_void) -> i32 {
    if !pv.is_null() {
        unsafe { std::ptr::write_bytes(pv as *mut u8, 0, 24) };
    }
    S_OK
}

pub unsafe extern "win64" fn CoTaskMemAlloc(n: usize) -> *mut c_void {
    unsafe { libc::malloc(n.max(1)) }
}

pub unsafe extern "win64" fn CoTaskMemFree(p: *mut c_void) {
    unsafe { libc::free(p) }
}

// ------------------------------------------------------------------------------------------------ errors, tracing

pub unsafe extern "win64" fn RoOriginateError(hr: i32, msg: *const c_void) -> Bool {
    let mut n = 0;
    let p = unsafe { WindowsGetStringRawBuffer(msg, &mut n) };
    let text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, n as usize) });
    log::warn!("DLL error {hr:#010x}: {text}");
    TRUE
}

pub unsafe extern "win64" fn RoOriginateErrorW(hr: i32, len: u32, msg: *const u16) -> Bool {
    let text = if msg.is_null() {
        String::new()
    } else if len == 0 {
        unsafe { super::wide_to_string(msg) }.unwrap_or_default()
    } else {
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(msg, len as usize) })
    };
    log::warn!("DLL error {hr:#010x}: {text}");
    TRUE
}

pub unsafe extern "win64" fn EventRegister(
    _guid: *const c_void,
    _cb: *mut c_void,
    _ctx: *mut c_void,
    handle: *mut u64,
) -> u32 {
    if !handle.is_null() {
        unsafe { *handle = 1 };
    }
    0
}

pub extern "win64" fn EventUnregister(_handle: u64) -> u32 {
    0
}

pub extern "win64" fn EventSetInformation(_h: u64, _class: u32, _info: *mut c_void, _len: u32) -> u32 {
    0
}

pub extern "win64" fn EventWriteTransfer(
    _h: u64,
    _desc: *const c_void,
    _activity: *const c_void,
    _related: *const c_void,
    _count: u32,
    _data: *mut c_void,
) -> u32 {
    0
}

const CSTR_LESS_THAN: i32 = 1;
const CSTR_EQUAL: i32 = 2;
const CSTR_GREATER_THAN: i32 = 3;

pub unsafe extern "win64" fn CompareStringOrdinal(
    a: *const u16,
    na: i32,
    b: *const u16,
    nb: i32,
    ignore_case: Bool,
) -> i32 {
    let s = |p: *const u16, n: i32| -> Vec<u16> {
        let n = if n < 0 { unsafe { super::wcslen(p) } } else { n as usize };
        let v = unsafe { std::slice::from_raw_parts(p, n) }.to_vec();
        if ignore_case != FALSE {
            // Ordinal case-insensitive comparison upper-cases (simple mapping).
            String::from_utf16_lossy(&v).to_uppercase().encode_utf16().collect()
        } else {
            v
        }
    };
    match s(a, na).cmp(&s(b, nb)) {
        std::cmp::Ordering::Less => CSTR_LESS_THAN,
        std::cmp::Ordering::Equal => CSTR_EQUAL,
        std::cmp::Ordering::Greater => CSTR_GREATER_THAN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hstrings() {
        let h = hstring("DTS.DtsSpeakersEncoder");
        let mut n = 0;
        let p = unsafe { WindowsGetStringRawBuffer(h, &mut n) };
        assert_eq!(
            String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, n as usize) }),
            "DTS.DtsSpeakersEncoder"
        );
        assert_eq!(unsafe { WindowsIsStringEmpty(h) }, FALSE);
        assert_ne!(unsafe { WindowsIsStringEmpty(std::ptr::null()) }, FALSE);
        delete_hstring(h);
        let w: Vec<u16> = "ab\0".encode_utf16().collect();
        let mut header = [0u64; 3];
        let mut r = std::ptr::null_mut();
        unsafe { WindowsCreateStringReference(w.as_ptr(), 2, header.as_mut_ptr().cast(), &mut r) };
        assert_eq!(r, header.as_mut_ptr().cast());
        let mut has = TRUE;
        unsafe { WindowsStringHasEmbeddedNull(r, &mut has) };
        assert_eq!(has, FALSE);
        let cmp =
            unsafe { CompareStringOrdinal(w.as_ptr(), 2, "AB".encode_utf16().collect::<Vec<_>>().as_ptr(), 2, TRUE) };
        assert_eq!(cmp, CSTR_EQUAL);
    }

    #[test]
    fn srw_and_once() {
        let mut l = 0usize;
        let p = &mut l as *mut usize as *mut c_void;
        AcquireSRWLockShared(p);
        AcquireSRWLockShared(p);
        ReleaseSRWLockShared(p);
        ReleaseSRWLockShared(p);
        AcquireSRWLockExclusive(p);
        ReleaseSRWLockExclusive(p);
        assert_eq!(l, 0);
        unsafe extern "win64" fn init(_o: *mut c_void, p: *mut c_void, _c: *mut *mut c_void) -> Bool {
            unsafe { *(p as *mut u32) += 1 };
            TRUE
        }
        let (mut once, mut count) = (0usize, 0u32);
        for _ in 0..3 {
            unsafe {
                InitOnceExecuteOnce(
                    &mut once as *mut usize as *mut c_void,
                    init,
                    &mut count as *mut u32 as *mut c_void,
                    std::ptr::null_mut(),
                )
            };
        }
        assert_eq!(count, 1);
    }
}
