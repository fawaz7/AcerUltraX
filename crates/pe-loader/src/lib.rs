//! A small in-process loader for 64-bit Windows DLLs on Linux x86-64.
//!
//! It maps a PE32+ DLL, applies relocations, binds its imports to built-in Rust implementations of the Win32
//! functions it uses (see [`shims`]), sets up implicit TLS and a per-thread TEB, and runs `DllMain`. It is meant
//! for self-contained DSP libraries with a statically linked C runtime, such as the DTS audio engine; it is not a
//! general Windows compatibility layer.
//!
//! Every call into a loaded DLL must happen inside [`Module::enter`], which prepares the calling thread.

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
compile_error!("pe-loader only supports Linux on x86-64");

mod image;
mod pe;
pub mod shims;
mod thread;

pub use image::{CallGuard, Module};

use std::path::PathBuf;
use std::sync::RwLock;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot read {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("malformed PE file: {0}")]
    Malformed(&'static str),
    #[error("unsupported PE file: {0}")]
    Unsupported(&'static str),
    #[error("cannot map the image: {0}")]
    Map(std::io::Error),
    #[error("the DLL imports functions this loader does not provide: {}", .0.join(", "))]
    MissingImports(Vec<String>),
    #[error("too many modules loaded")]
    TooManyModules,
    #[error("DllMain failed")]
    InitFailed,
}

static FILE_ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Directory that relative file names opened by DLL code are resolved against. Without one, DLL code cannot
/// open any file.
pub fn set_file_root(dir: Option<PathBuf>) {
    *FILE_ROOT.write().unwrap() = dir;
}

pub(crate) fn file_root() -> Option<PathBuf> {
    FILE_ROOT.read().unwrap().clone()
}
