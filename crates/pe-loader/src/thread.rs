//! Per-thread Windows state: a minimal TEB installed as the GS base, the TLS array, FLS/TLS slots and the last
//! error value.
//!
//! Windows x64 code reads its thread environment block through `gs:`. On Linux, glibc and Rust's runtime use FS
//! for their own thread pointer and leave GS free, so each thread that calls into a loaded DLL gets its own TEB
//! installed with `arch_prctl(ARCH_SET_GS)`.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU32, Ordering};

const TEB_SIZE: usize = 0x2000;
const TEB_STACK_BASE: usize = 0x08;
const TEB_STACK_LIMIT: usize = 0x10;
const TEB_SELF: usize = 0x30;
const TEB_PID: usize = 0x40;
const TEB_TID: usize = 0x48;
const TEB_TLS_POINTER: usize = 0x58;
const TEB_PEB: usize = 0x60;

const ARCH_SET_GS: libc::c_int = 0x1001;

/// Slots for implicit TLS (`_tls_index`), one per loaded module.
pub const MAX_MODULES: usize = 16;
/// Slots for `TlsAlloc` and `FlsAlloc` indices.
pub const MAX_SLOTS: usize = 128;

static NEXT_TLS_SLOT: AtomicU32 = AtomicU32::new(0);
static NEXT_FLS_SLOT: AtomicU32 = AtomicU32::new(0);

/// Allocate a `TlsAlloc` (`fls == false`) or `FlsAlloc` index valid in every thread.
pub fn alloc_slot(fls: bool) -> Option<u32> {
    let counter = if fls { &NEXT_FLS_SLOT } else { &NEXT_TLS_SLOT };
    let i = counter.fetch_add(1, Ordering::Relaxed);
    if (i as usize) < MAX_SLOTS { Some(i) } else { None }
}

/// The process environment block and its process parameters: zeroed pages that the CRT reads only on exit paths.
fn peb() -> usize {
    static PEB: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *PEB.get_or_init(|| {
        let peb = zeroed_pages(0x1000);
        let params = zeroed_pages(0x1000);
        // SAFETY: both blocks are freshly allocated, zeroed and at least 0x1000 bytes long.
        unsafe { *((peb + 0x20) as *mut usize) = params };
        peb
    })
}

pub(crate) fn zeroed_pages(len: usize) -> usize {
    // SAFETY: anonymous private mapping; checked for failure.
    let p = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    assert!(p != libc::MAP_FAILED, "out of memory mapping {len} bytes");
    p as usize
}

pub struct ThreadState {
    teb: usize,
    /// The array `gs:[0x58]` points at: one TLS block pointer per module TLS index.
    tls_array: Box<[Cell<usize>; MAX_MODULES]>,
    /// Bit i set: module i has received `DLL_PROCESS_ATTACH`/`DLL_THREAD_ATTACH` on this thread.
    attached: Cell<u32>,
    pub last_error: Cell<u32>,
    pub tls_slots: [Cell<usize>; MAX_SLOTS],
    pub fls_slots: [Cell<usize>; MAX_SLOTS],
    /// TLS blocks owned by this thread (freed on exit).
    tls_blocks: RefCell<Vec<Box<[u8]>>>,
}

impl ThreadState {
    fn new() -> Self {
        let teb = zeroed_pages(TEB_SIZE);
        let tls_array: Box<[Cell<usize>; MAX_MODULES]> = Box::new(std::array::from_fn(|_| Cell::new(0)));
        let (stack_base, stack_size) = current_stack();
        if stack_size != 0 && stack_size < 1 << 20 {
            log::warn!("thread stack is only {stack_size} bytes; DLL code may need up to 1 MiB");
        }
        // SAFETY: `teb` is a zeroed, writable block of TEB_SIZE bytes; all offsets are within it.
        unsafe {
            let w = |off: usize, v: usize| *((teb + off) as *mut usize) = v;
            w(TEB_STACK_BASE, stack_base);
            // A stack limit of 0 turns `__chkstk` probing into a no-op; Linux grows and guards the stack itself.
            w(TEB_STACK_LIMIT, 0);
            w(TEB_SELF, teb);
            w(TEB_PID, libc::getpid() as usize);
            w(TEB_TID, libc::gettid() as usize);
            w(TEB_TLS_POINTER, tls_array.as_ptr() as usize);
            w(TEB_PEB, peb());
            let r = libc::syscall(libc::SYS_arch_prctl, ARCH_SET_GS, teb);
            assert!(r == 0, "arch_prctl(ARCH_SET_GS) failed: {}", std::io::Error::last_os_error());
        }
        ThreadState {
            teb,
            tls_array,
            attached: Cell::new(0),
            last_error: Cell::new(0),
            tls_slots: std::array::from_fn(|_| Cell::new(0)),
            fls_slots: std::array::from_fn(|_| Cell::new(0)),
            tls_blocks: RefCell::new(Vec::new()),
        }
    }

    pub fn is_attached(&self, module: usize) -> bool {
        self.attached.get() & (1 << module) != 0
    }

    pub fn set_attached(&self, module: usize) {
        self.attached.set(self.attached.get() | (1 << module));
    }

    /// Give this thread its own copy of a module's TLS template at the module's TLS index.
    pub fn install_tls(&self, index: usize, template: &[u8]) {
        // 16 bytes of slack so the block can be 16-byte aligned regardless of the allocator.
        let mut block = vec![0u8; template.len() + 16].into_boxed_slice();
        let start = (16 - (block.as_ptr() as usize & 15)) & 15;
        block[start..start + template.len()].copy_from_slice(template);
        self.tls_array[index].set(block.as_ptr() as usize + start);
        self.tls_blocks.borrow_mut().push(block);
    }
}

impl Drop for ThreadState {
    fn drop(&mut self) {
        // SAFETY: GS is reset before the TEB is unmapped so no stale pointer remains installed.
        unsafe {
            libc::syscall(libc::SYS_arch_prctl, ARCH_SET_GS, 0usize);
            libc::munmap(self.teb as *mut libc::c_void, TEB_SIZE);
        }
    }
}

/// (top of stack, stack size) of the calling thread, or (0, 0) if unknown.
fn current_stack() -> (usize, usize) {
    // SAFETY: pthread_getattr_np fills `attr`, which is destroyed afterwards.
    unsafe {
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) != 0 {
            return (0, 0);
        }
        let mut addr = std::ptr::null_mut();
        let mut size = 0usize;
        let ok = libc::pthread_attr_getstack(&attr, &mut addr, &mut size) == 0;
        libc::pthread_attr_destroy(&mut attr);
        if ok { (addr as usize + size, size) } else { (0, 0) }
    }
}

thread_local! {
    static STATE: ThreadState = ThreadState::new();
}

/// Run `f` with the calling thread's Windows state, creating it (and installing the TEB) on first use.
pub fn with<R>(f: impl FnOnce(&ThreadState) -> R) -> R {
    STATE.with(f)
}

pub fn set_last_error(e: u32) {
    with(|t| t.last_error.set(e));
}
