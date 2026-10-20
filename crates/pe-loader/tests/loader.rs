//! Loads a small DLL built from `testdll/testdll.c` and checks relocations, imports, implicit TLS, DllMain
//! notifications and the win64 calling convention.
//!
//! Needs clang, lld-link and llvm-dlltool (the LLVM toolchain); skipped when they are missing.

use std::ffi::{CStr, c_char, c_void};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use pe_loader::Module;

fn tool(name: &str) -> bool {
    Command::new(name).arg("--version").output().is_ok_and(|o| o.status.success())
}

fn run(cmd: &mut Command) {
    let out = cmd.output().unwrap_or_else(|e| panic!("{cmd:?}: {e}"));
    assert!(out.status.success(), "{cmd:?} failed:\n{}", String::from_utf8_lossy(&out.stderr));
}

/// Build the test DLL once per test run; None when the toolchain is missing.
fn test_dll() -> Option<&'static Path> {
    static DLL: OnceLock<Option<PathBuf>> = OnceLock::new();
    DLL.get_or_init(|| {
        if !["clang", "lld-link", "llvm-dlltool"].into_iter().all(tool) {
            eprintln!("skipped: clang, lld-link and llvm-dlltool are needed to build the test DLL");
            return None;
        }
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/testdll");
        let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("testdll");
        std::fs::create_dir_all(&out).unwrap();
        run(Command::new("llvm-dlltool")
            .args(["-m", "i386:x86-64", "-d"])
            .arg(src.join("kernel32.def"))
            .arg("-l")
            .arg(out.join("kernel32.lib")));
        run(Command::new("clang")
            .args(["--target=x86_64-pc-windows-msvc", "-O1", "-ffreestanding", "-fno-stack-protector", "-c"])
            .arg(src.join("testdll.c"))
            .arg("-o")
            .arg(out.join("testdll.obj")));
        let dll = out.join("testdll.dll");
        run(Command::new("lld-link")
            .args(["/dll", "/nodefaultlib", "/entry:DllMain"])
            .arg(format!("/out:{}", dll.display()))
            .arg(out.join("testdll.obj"))
            .arg(out.join("kernel32.lib")));
        Some(dll)
    })
    .as_deref()
}

/// The DLL, loaded once (at its preferred base) for the whole test binary.
fn module() -> Option<&'static Module> {
    static M: OnceLock<Option<&'static Module>> = OnceLock::new();
    *M.get_or_init(|| test_dll().map(|p| Module::load(p).expect("load test DLL")))
}

unsafe fn func<T: Copy>(m: &Module, name: &str) -> T {
    let p = m.export(name).unwrap_or_else(|| panic!("no export {name}"));
    unsafe { std::mem::transmute_copy::<*const c_void, T>(&p) }
}

#[test]
fn calls_exports_with_win64_abi() {
    let Some(m) = module() else { return };
    let _g = m.enter();
    unsafe {
        let mul_add: extern "win64" fn(f64, f64, f32, i32) -> f64 = func(m, "mul_add");
        assert_eq!(mul_add(1.5, 4.0, 0.25, 10), 16.25);
        let heap: extern "win64" fn(i32) -> i32 = func(m, "heap_round_trip");
        assert_eq!(heap(1000), 1);
        let err: extern "win64" fn(u32) -> u32 = func(m, "last_error_round_trip");
        assert_eq!(err(0xdead), 0xdead);
        let fls: extern "win64" fn(usize) -> usize = func(m, "fls_round_trip");
        assert_eq!(fls(77), 77);
    }
}

#[test]
fn relocated_copy_works() {
    let Some(m) = module() else { return };
    // The first copy holds the preferred base, so a second load must be relocated.
    let copy = Module::load(test_dll().unwrap()).unwrap();
    assert_ne!(copy.base(), m.base());
    for module in [m, copy] {
        let _g = module.enter();
        let name_at: extern "win64" fn(i32) -> *const c_char = unsafe { func(module, "name_at") };
        let s = unsafe { CStr::from_ptr(name_at(1)) };
        assert_eq!(s.to_str().unwrap(), "beta");
        assert!((name_at(0) as usize) >= module.base());
    }
}

#[test]
fn thread_local_storage_and_thread_attach() {
    let Some(m) = module() else { return };
    let counts: extern "win64" fn() -> i32 = unsafe { func(m, "attach_counts") };
    let get: extern "win64" fn() -> i32 = unsafe { func(m, "tls_get") };
    let set: extern "win64" fn(i32) = unsafe { func(m, "tls_set") };
    let before = {
        let _g = m.enter();
        set(1);
        counts()
    };
    assert_eq!(before / 1000, 1, "DLL_PROCESS_ATTACH exactly once");
    std::thread::spawn(move || {
        let _g = m.enter();
        assert_eq!(get(), 1234, "a new thread starts from the TLS template");
        set(2);
        assert_eq!(get(), 2);
    })
    .join()
    .unwrap();
    let _g = m.enter();
    assert_eq!(get(), 1, "other threads do not see this thread's TLS");
    assert!(counts() % 1000 > before % 1000, "DLL_THREAD_ATTACH on the new thread");
}

#[test]
fn provides_engine_imports() {
    // Every function the DTS engine imports must be provided.
    let needed = [
        "HeapAlloc",
        "FlsGetValue",
        "CreateFileW",
        "PathCchRemoveFileSpec",
        "RegOpenKeyExW",
        "MultiByteToWideChar",
        "InitializeCriticalSectionEx",
        "RaiseException",
        "GetModuleFileNameW",
    ];
    for n in needed {
        assert!(pe_loader::shims::lookup(n).is_some(), "{n}");
    }
    assert!(pe_loader::shims::lookup("CreateThread").is_none());
}
