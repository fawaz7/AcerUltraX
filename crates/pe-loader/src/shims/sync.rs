//! Critical sections, condition variables, events, SLists, TLS/FLS slots and the last-error value.

#![allow(non_snake_case)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use super::{Bool, ERROR_INVALID_HANDLE, ERROR_TIMEOUT, FALSE, Handle, TRUE, file, set_last_error};
use crate::thread;

const INFINITE: u32 = u32::MAX;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 0x102;
const WAIT_FAILED: u32 = u32::MAX;
const TLS_OUT_OF_INDEXES: u32 = u32::MAX;

// A CRITICAL_SECTION is 40 bytes owned by the caller; its first field holds a pointer to a heap-allocated
// recursive pthread mutex.

unsafe fn cs_mutex(cs: *mut c_void) -> *mut libc::pthread_mutex_t {
    unsafe { *(cs as *const *mut libc::pthread_mutex_t) }
}

unsafe fn init_cs(cs: *mut c_void) {
    // SAFETY: the mutex is boxed, initialised as recursive and never moved.
    unsafe {
        let m: *mut libc::pthread_mutex_t = Box::into_raw(Box::new(std::mem::zeroed()));
        let mut attr: libc::pthread_mutexattr_t = std::mem::zeroed();
        libc::pthread_mutexattr_init(&mut attr);
        libc::pthread_mutexattr_settype(&mut attr, libc::PTHREAD_MUTEX_RECURSIVE);
        libc::pthread_mutex_init(m, &attr);
        libc::pthread_mutexattr_destroy(&mut attr);
        std::ptr::write_bytes(cs as *mut u8, 0, 40);
        *(cs as *mut *mut libc::pthread_mutex_t) = m;
    }
}

pub unsafe extern "win64" fn InitializeCriticalSectionEx(cs: *mut c_void, _spin: u32, _flags: u32) -> Bool {
    unsafe { init_cs(cs) };
    TRUE
}

pub unsafe extern "win64" fn InitializeCriticalSectionAndSpinCount(cs: *mut c_void, _spin: u32) -> Bool {
    unsafe { init_cs(cs) };
    TRUE
}

pub unsafe extern "win64" fn EnterCriticalSection(cs: *mut c_void) {
    unsafe { libc::pthread_mutex_lock(cs_mutex(cs)) };
}

pub unsafe extern "win64" fn LeaveCriticalSection(cs: *mut c_void) {
    unsafe { libc::pthread_mutex_unlock(cs_mutex(cs)) };
}

pub unsafe extern "win64" fn DeleteCriticalSection(cs: *mut c_void) {
    unsafe {
        let m = cs_mutex(cs);
        if !m.is_null() {
            libc::pthread_mutex_destroy(m);
            drop(Box::from_raw(m));
            *(cs as *mut *mut libc::pthread_mutex_t) = std::ptr::null_mut();
        }
    }
}

// A CONDITION_VARIABLE is one pointer, zero-initialised by the caller; the pthread condvar is created on first use.

unsafe fn condvar(cv: *mut c_void) -> *mut libc::pthread_cond_t {
    let slot = unsafe { AtomicPtr::<libc::pthread_cond_t>::from_ptr(cv as *mut *mut libc::pthread_cond_t) };
    let cur = slot.load(Ordering::Acquire);
    if !cur.is_null() {
        return cur;
    }
    // SAFETY: a fresh boxed condvar; the loser of a race frees its copy.
    unsafe {
        let new: *mut libc::pthread_cond_t = Box::into_raw(Box::new(std::mem::zeroed()));
        libc::pthread_cond_init(new, std::ptr::null());
        match slot.compare_exchange(std::ptr::null_mut(), new, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => new,
            Err(existing) => {
                libc::pthread_cond_destroy(new);
                drop(Box::from_raw(new));
                existing
            }
        }
    }
}

pub unsafe extern "win64" fn SleepConditionVariableCS(cv: *mut c_void, cs: *mut c_void, ms: u32) -> Bool {
    // SAFETY: the caller holds `cs` (once), as Win32 requires.
    unsafe {
        let c = condvar(cv);
        let m = cs_mutex(cs);
        if ms == INFINITE {
            libc::pthread_cond_wait(c, m);
            return TRUE;
        }
        let mut ts: libc::timespec = std::mem::zeroed();
        libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts);
        let ns = ts.tv_nsec as u64 + (ms as u64 % 1000) * 1_000_000;
        ts.tv_sec += (ms / 1000) as libc::time_t + (ns / 1_000_000_000) as libc::time_t;
        ts.tv_nsec = (ns % 1_000_000_000) as _;
        if libc::pthread_cond_timedwait(c, m, &ts) == libc::ETIMEDOUT {
            set_last_error(ERROR_TIMEOUT);
            return FALSE;
        }
        TRUE
    }
}

pub unsafe extern "win64" fn WakeAllConditionVariable(cv: *mut c_void) {
    unsafe { libc::pthread_cond_broadcast(condvar(cv)) };
}

pub struct Event {
    manual_reset: bool,
    signalled: Mutex<bool>,
    cond: Condvar,
}

impl Event {
    fn wait(&self, ms: u32) -> bool {
        let mut s = self.signalled.lock().unwrap();
        if ms == INFINITE {
            while !*s {
                s = self.cond.wait(s).unwrap();
            }
        } else {
            let (g, _) = self.cond.wait_timeout_while(s, Duration::from_millis(ms as u64), |s| !*s).unwrap();
            s = g;
            if !*s {
                return false;
            }
        }
        if !self.manual_reset {
            *s = false;
        }
        true
    }
}

pub unsafe extern "win64" fn CreateEventW(
    _attr: *mut c_void,
    manual: Bool,
    initial: Bool,
    _name: *const u16,
) -> Handle {
    let ev = Event { manual_reset: manual != 0, signalled: Mutex::new(initial != 0), cond: Condvar::new() };
    file::insert(file::Object::Event(Arc::new(ev)))
}

fn with_event(h: Handle, f: impl FnOnce(&Event)) -> Bool {
    match file::get(h) {
        Some(file::Object::Event(e)) => {
            f(&e);
            TRUE
        }
        _ => {
            set_last_error(ERROR_INVALID_HANDLE);
            FALSE
        }
    }
}

pub extern "win64" fn SetEvent(h: Handle) -> Bool {
    with_event(h, |e| {
        *e.signalled.lock().unwrap() = true;
        e.cond.notify_all();
    })
}

pub extern "win64" fn ResetEvent(h: Handle) -> Bool {
    with_event(h, |e| *e.signalled.lock().unwrap() = false)
}

pub extern "win64" fn WaitForSingleObjectEx(h: Handle, ms: u32, _alertable: Bool) -> u32 {
    match file::get(h) {
        Some(file::Object::Event(e)) => {
            if e.wait(ms) {
                WAIT_OBJECT_0
            } else {
                WAIT_TIMEOUT
            }
        }
        _ => {
            set_last_error(ERROR_INVALID_HANDLE);
            WAIT_FAILED
        }
    }
}

pub extern "win64" fn WaitForSingleObject(h: Handle, ms: u32) -> u32 {
    WaitForSingleObjectEx(h, ms, FALSE)
}

// SLIST_HEADER: 16 bytes. Nothing in the supported DLLs pushes entries, so a flush always finds an empty list.

pub unsafe extern "win64" fn InitializeSListHead(head: *mut c_void) {
    unsafe { std::ptr::write_bytes(head as *mut u8, 0, 16) };
}

pub unsafe extern "win64" fn InterlockedFlushSList(head: *mut c_void) -> *mut c_void {
    unsafe { std::ptr::write_bytes(head as *mut u8, 0, 16) };
    std::ptr::null_mut()
}

fn slot_get(fls: bool, i: u32) -> usize {
    thread::with(|t| {
        let slots = if fls { &t.fls_slots } else { &t.tls_slots };
        match slots.get(i as usize) {
            Some(s) => {
                t.last_error.set(0);
                s.get()
            }
            None => {
                t.last_error.set(super::ERROR_INVALID_PARAMETER);
                0
            }
        }
    })
}

fn slot_set(fls: bool, i: u32, v: usize) -> Bool {
    thread::with(|t| {
        let slots = if fls { &t.fls_slots } else { &t.tls_slots };
        match slots.get(i as usize) {
            Some(s) => {
                s.set(v);
                TRUE
            }
            None => {
                t.last_error.set(super::ERROR_INVALID_PARAMETER);
                FALSE
            }
        }
    })
}

pub extern "win64" fn TlsAlloc() -> u32 {
    thread::alloc_slot(false).unwrap_or(TLS_OUT_OF_INDEXES)
}

pub extern "win64" fn TlsGetValue(i: u32) -> usize {
    slot_get(false, i)
}

pub extern "win64" fn TlsSetValue(i: u32, v: usize) -> Bool {
    slot_set(false, i, v)
}

pub extern "win64" fn TlsFree(_i: u32) -> Bool {
    TRUE
}

/// The destructor callback is not called at thread exit; the CRT's per-thread data is small and threads that
/// call DSP code are long-lived.
pub extern "win64" fn FlsAlloc(_callback: *mut c_void) -> u32 {
    thread::alloc_slot(true).unwrap_or(TLS_OUT_OF_INDEXES)
}

pub extern "win64" fn FlsGetValue(i: u32) -> usize {
    slot_get(true, i)
}

pub extern "win64" fn FlsSetValue(i: u32, v: usize) -> Bool {
    slot_set(true, i, v)
}

pub extern "win64" fn FlsFree(_i: u32) -> Bool {
    TRUE
}

pub extern "win64" fn GetLastError() -> u32 {
    thread::with(|t| t.last_error.get())
}

pub extern "win64" fn SetLastError(e: u32) {
    set_last_error(e);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recursive_critical_section() {
        let mut cs = [0u8; 40];
        let p = cs.as_mut_ptr() as *mut c_void;
        unsafe {
            InitializeCriticalSectionEx(p, 4000, 0);
            EnterCriticalSection(p);
            EnterCriticalSection(p);
            LeaveCriticalSection(p);
            LeaveCriticalSection(p);
            DeleteCriticalSection(p);
        }
    }

    #[test]
    fn events() {
        unsafe {
            let auto = CreateEventW(std::ptr::null_mut(), FALSE, TRUE, std::ptr::null());
            assert_eq!(WaitForSingleObjectEx(auto, 0, FALSE), WAIT_OBJECT_0);
            assert_eq!(WaitForSingleObjectEx(auto, 10, FALSE), WAIT_TIMEOUT);
            let manual = CreateEventW(std::ptr::null_mut(), TRUE, FALSE, std::ptr::null());
            SetEvent(manual);
            assert_eq!(WaitForSingleObject(manual, 0), WAIT_OBJECT_0);
            assert_eq!(WaitForSingleObject(manual, 0), WAIT_OBJECT_0);
            ResetEvent(manual);
            assert_eq!(WaitForSingleObject(manual, 0), WAIT_TIMEOUT);
            file::CloseHandle(auto);
            file::CloseHandle(manual);
        }
    }

    #[test]
    fn slots_are_per_thread() {
        let i = FlsAlloc(std::ptr::null_mut());
        assert!(FlsSetValue(i, 42) != 0);
        assert_eq!(FlsGetValue(i), 42);
        std::thread::spawn(move || assert_eq!(FlsGetValue(i), 0)).join().unwrap();
    }
}
