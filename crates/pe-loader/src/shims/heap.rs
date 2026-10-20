//! Process heap on top of the C allocator. Each block carries a 16-byte header with the requested size, which
//! `HeapSize` must return exactly (the CRT's `_recalloc` relies on it), and stays 16-byte aligned.

#![allow(non_snake_case)]

use std::ffi::c_void;

use super::{Bool, Handle, TRUE};

const HEAP_ZERO_MEMORY: u32 = 0x08;
const HEAP_REALLOC_IN_PLACE_ONLY: u32 = 0x10;
const HEADER: usize = 16;
const PROCESS_HEAP: usize = 0x0001_0000;

unsafe fn size_of_block(p: *mut c_void) -> usize {
    unsafe { *((p as *mut u8).sub(HEADER) as *const usize) }
}

pub extern "win64" fn GetProcessHeap() -> Handle {
    PROCESS_HEAP as Handle
}

pub unsafe extern "win64" fn HeapAlloc(_heap: Handle, flags: u32, size: usize) -> *mut c_void {
    let total = size.saturating_add(HEADER);
    // SAFETY: glibc malloc/calloc return 16-byte aligned blocks of at least `total` bytes or null.
    unsafe {
        let raw = if flags & HEAP_ZERO_MEMORY != 0 { libc::calloc(1, total) } else { libc::malloc(total) };
        if raw.is_null() {
            return std::ptr::null_mut();
        }
        *(raw as *mut usize) = size;
        (raw as *mut u8).add(HEADER).cast()
    }
}

pub unsafe extern "win64" fn HeapFree(_heap: Handle, _flags: u32, p: *mut c_void) -> Bool {
    if !p.is_null() {
        // SAFETY: `p` came from HeapAlloc/HeapReAlloc.
        unsafe { libc::free((p as *mut u8).sub(HEADER).cast()) };
    }
    TRUE
}

pub unsafe extern "win64" fn HeapReAlloc(heap: Handle, flags: u32, p: *mut c_void, size: usize) -> *mut c_void {
    if p.is_null() {
        return unsafe { HeapAlloc(heap, flags, size) };
    }
    if flags & HEAP_REALLOC_IN_PLACE_ONLY != 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: `p` came from this heap; the header is rewritten for the new size.
    unsafe {
        let old = size_of_block(p);
        let raw = libc::realloc((p as *mut u8).sub(HEADER).cast(), size.saturating_add(HEADER));
        if raw.is_null() {
            return std::ptr::null_mut();
        }
        *(raw as *mut usize) = size;
        let q = (raw as *mut u8).add(HEADER);
        if flags & HEAP_ZERO_MEMORY != 0 && size > old {
            std::ptr::write_bytes(q.add(old), 0, size - old);
        }
        q.cast()
    }
}

pub unsafe extern "win64" fn HeapSize(_heap: Handle, _flags: u32, p: *const c_void) -> usize {
    if p.is_null() {
        return usize::MAX;
    }
    // SAFETY: `p` came from this heap.
    unsafe { size_of_block(p as *mut c_void) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shims::FALSE;

    #[test]
    fn sizes_alignment_and_zeroing() {
        unsafe {
            let p = HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, 40);
            assert_eq!(p as usize % 16, 0);
            assert_eq!(HeapSize(GetProcessHeap(), 0, p), 40);
            assert!(std::slice::from_raw_parts(p as *const u8, 40).iter().all(|&b| b == 0));
            std::ptr::write_bytes(p as *mut u8, 0xab, 40);
            let q = HeapReAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, p, 100);
            assert_eq!(HeapSize(GetProcessHeap(), 0, q), 100);
            let s = std::slice::from_raw_parts(q as *const u8, 100);
            assert!(s[..40].iter().all(|&b| b == 0xab) && s[40..].iter().all(|&b| b == 0));
            assert!(HeapReAlloc(GetProcessHeap(), HEAP_REALLOC_IN_PLACE_ONLY, q, 200).is_null());
            assert_eq!(HeapFree(GetProcessHeap(), 0, q), TRUE);
            assert_ne!(HeapFree(GetProcessHeap(), 0, std::ptr::null_mut()), FALSE);
        }
    }
}
