//! Built-in implementations of the Win32 functions loaded DLLs import.
//!
//! Each function uses the Microsoft x64 calling convention (`extern "win64"`) and is bound by name, regardless of
//! the DLL it is imported from (`KERNEL32.dll`, `api-ms-win-*`, ...). Functions only reached on fatal error paths
//! log what happened and abort the process; ones only needed by features the supported DLLs never use return a
//! failure value.

mod crt;
mod file;
mod heap;
mod misc;
mod module;
mod string;
mod sync;
pub mod winrt;

use std::ffi::c_void;

pub(crate) type Handle = *mut c_void;
pub(crate) type Bool = i32;

pub(crate) const TRUE: Bool = 1;
pub(crate) const FALSE: Bool = 0;
pub(crate) const INVALID_HANDLE_VALUE: Handle = usize::MAX as Handle;

pub(crate) const ERROR_FILE_NOT_FOUND: u32 = 2;
pub(crate) const ERROR_ACCESS_DENIED: u32 = 5;
pub(crate) const ERROR_INVALID_HANDLE: u32 = 6;
pub(crate) const ERROR_INVALID_PARAMETER: u32 = 87;
pub(crate) const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
pub(crate) const ERROR_MOD_NOT_FOUND: u32 = 126;
pub(crate) const ERROR_PROC_NOT_FOUND: u32 = 127;
pub(crate) const ERROR_NO_UNICODE_TRANSLATION: u32 = 1113;
pub(crate) const ERROR_TIMEOUT: u32 = 1460;
pub(crate) const ERROR_CALL_NOT_IMPLEMENTED: u32 = 120;

pub(crate) fn set_last_error(e: u32) {
    crate::thread::set_last_error(e);
}

/// Log and abort: the DLL reached a path that only runs when something has gone badly wrong.
pub(crate) fn fatal(what: &str) -> ! {
    log::error!("{what}");
    eprintln!("pe-loader: fatal: {what}");
    std::process::abort();
}

/// Length of a NUL-terminated UTF-16 string.
///
/// # Safety
/// `p` must point at a NUL-terminated UTF-16 string.
pub(crate) unsafe fn wcslen(p: *const u16) -> usize {
    let mut n = 0;
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    n
}

/// # Safety
/// `p` must be null or point at a NUL-terminated UTF-16 string.
pub(crate) unsafe fn wide_to_string(p: *const u16) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let s = unsafe { std::slice::from_raw_parts(p, wcslen(p)) };
    Some(String::from_utf16_lossy(s))
}

/// # Safety
/// `p` must be null or point at a NUL-terminated byte string.
pub(crate) unsafe fn c_to_string(p: *const u8) -> Option<String> {
    if p.is_null() {
        return None;
    }
    Some(unsafe { std::ffi::CStr::from_ptr(p.cast()) }.to_string_lossy().into_owned())
}

/// Copy `s` plus a NUL into a caller buffer of `cap` UTF-16 units, Win32 style: returns the length without the
/// NUL, or `cap` (truncated, NUL-terminated) with ERROR_INSUFFICIENT_BUFFER when it does not fit.
///
/// # Safety
/// `buf` must be valid for `cap` writes.
pub(crate) unsafe fn write_wide(s: &str, buf: *mut u16, cap: u32) -> u32 {
    let w: Vec<u16> = s.encode_utf16().collect();
    let cap = cap as usize;
    if cap == 0 {
        set_last_error(ERROR_INSUFFICIENT_BUFFER);
        return 0;
    }
    let n = w.len().min(cap - 1);
    unsafe {
        std::ptr::copy_nonoverlapping(w.as_ptr(), buf, n);
        *buf.add(n) = 0;
    }
    if n < w.len() {
        set_last_error(ERROR_INSUFFICIENT_BUFFER);
        cap as u32
    } else {
        n as u32
    }
}

macro_rules! table {
    ($($name:ident => $f:path),* $(,)?) => {
        vec![$((stringify!($name), $f as *const () as usize)),*]
    };
}

fn all() -> &'static [(&'static str, usize)] {
    static ALL: std::sync::OnceLock<Vec<(&'static str, usize)>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        table! {
            // heap.rs
            GetProcessHeap => heap::GetProcessHeap,
            HeapAlloc => heap::HeapAlloc,
            HeapFree => heap::HeapFree,
            HeapReAlloc => heap::HeapReAlloc,
            HeapSize => heap::HeapSize,
            // sync.rs
            InitializeCriticalSectionEx => sync::InitializeCriticalSectionEx,
            InitializeCriticalSectionAndSpinCount => sync::InitializeCriticalSectionAndSpinCount,
            EnterCriticalSection => sync::EnterCriticalSection,
            LeaveCriticalSection => sync::LeaveCriticalSection,
            DeleteCriticalSection => sync::DeleteCriticalSection,
            SleepConditionVariableCS => sync::SleepConditionVariableCS,
            WakeAllConditionVariable => sync::WakeAllConditionVariable,
            CreateEventW => sync::CreateEventW,
            SetEvent => sync::SetEvent,
            ResetEvent => sync::ResetEvent,
            WaitForSingleObjectEx => sync::WaitForSingleObjectEx,
            WaitForSingleObject => sync::WaitForSingleObject,
            InitializeSListHead => sync::InitializeSListHead,
            InterlockedFlushSList => sync::InterlockedFlushSList,
            TlsAlloc => sync::TlsAlloc,
            TlsGetValue => sync::TlsGetValue,
            TlsSetValue => sync::TlsSetValue,
            TlsFree => sync::TlsFree,
            FlsAlloc => sync::FlsAlloc,
            FlsGetValue => sync::FlsGetValue,
            FlsSetValue => sync::FlsSetValue,
            FlsFree => sync::FlsFree,
            GetLastError => sync::GetLastError,
            SetLastError => sync::SetLastError,
            // misc.rs
            GetCurrentProcessId => misc::GetCurrentProcessId,
            GetCurrentThreadId => misc::GetCurrentThreadId,
            GetCurrentProcess => misc::GetCurrentProcess,
            QueryPerformanceCounter => misc::QueryPerformanceCounter,
            GetSystemTimeAsFileTime => misc::GetSystemTimeAsFileTime,
            TerminateProcess => misc::TerminateProcess,
            ExitProcess => misc::ExitProcess,
            IsProcessorFeaturePresent => misc::IsProcessorFeaturePresent,
            IsDebuggerPresent => misc::IsDebuggerPresent,
            UnhandledExceptionFilter => misc::UnhandledExceptionFilter,
            SetUnhandledExceptionFilter => misc::SetUnhandledExceptionFilter,
            RtlCaptureContext => misc::RtlCaptureContext,
            RtlLookupFunctionEntry => misc::RtlLookupFunctionEntry,
            RtlVirtualUnwind => misc::RtlVirtualUnwind,
            RtlUnwindEx => misc::RtlUnwindEx,
            RtlUnwind => misc::RtlUnwind,
            RtlPcToFileHeader => misc::RtlPcToFileHeader,
            RaiseException => misc::RaiseException,
            EncodePointer => misc::EncodePointer,
            DecodePointer => misc::DecodePointer,
            RegOpenKeyExW => misc::RegOpenKeyExW,
            RegQueryValueExW => misc::RegQueryValueExW,
            RegCloseKey => misc::RegCloseKey,
            OutputDebugStringA => misc::OutputDebugStringA,
            // module.rs
            GetModuleHandleW => module::GetModuleHandleW,
            GetModuleHandleExW => module::GetModuleHandleExW,
            GetModuleFileNameW => module::GetModuleFileNameW,
            LoadLibraryExW => module::LoadLibraryExW,
            FreeLibrary => module::FreeLibrary,
            GetProcAddress => module::GetProcAddress,
            // file.rs
            CreateFileW => file::CreateFileW,
            ReadFile => file::ReadFile,
            WriteFile => file::WriteFile,
            SetFilePointerEx => file::SetFilePointerEx,
            GetFileSizeEx => file::GetFileSizeEx,
            GetFileType => file::GetFileType,
            CloseHandle => file::CloseHandle,
            FlushFileBuffers => file::FlushFileBuffers,
            SetEndOfFile => file::SetEndOfFile,
            GetStdHandle => file::GetStdHandle,
            SetStdHandle => file::SetStdHandle,
            GetStartupInfoW => file::GetStartupInfoW,
            GetConsoleMode => file::GetConsoleMode,
            GetConsoleOutputCP => file::GetConsoleOutputCP,
            WriteConsoleW => file::WriteConsoleW,
            ReadConsoleW => file::ReadConsoleW,
            FindFirstFileExW => file::FindFirstFileExW,
            FindNextFileW => file::FindNextFileW,
            FindClose => file::FindClose,
            PathCchRemoveFileSpec => file::PathCchRemoveFileSpec,
            PathCchAddBackslash => file::PathCchAddBackslash,
            // string.rs
            GetCommandLineA => string::GetCommandLineA,
            GetCommandLineW => string::GetCommandLineW,
            GetEnvironmentStringsW => string::GetEnvironmentStringsW,
            FreeEnvironmentStringsW => string::FreeEnvironmentStringsW,
            MultiByteToWideChar => string::MultiByteToWideChar,
            WideCharToMultiByte => string::WideCharToMultiByte,
            GetACP => string::GetACP,
            GetOEMCP => string::GetOEMCP,
            IsValidCodePage => string::IsValidCodePage,
            GetCPInfo => string::GetCPInfo,
            LCMapStringW => string::LCMapStringW,
            LCMapStringEx => string::LCMapStringEx,
            GetStringTypeW => string::GetStringTypeW,
            GetLocaleInfoW => string::GetLocaleInfoW,
            IsValidLocale => string::IsValidLocale,
            GetUserDefaultLCID => string::GetUserDefaultLCID,
            EnumSystemLocalesW => string::EnumSystemLocalesW,
        // crt.rs
        malloc => crt::malloc,
        calloc => crt::calloc,
        free => crt::free,
        _aligned_malloc => crt::_aligned_malloc,
        _aligned_free => crt::_aligned_free,
        _callnewh => crt::_callnewh,
        terminate => crt::terminate,
        abort => crt::abort,
        _configure_narrow_argv => crt::_configure_narrow_argv,
        _initialize_narrow_environment => crt::_initialize_narrow_environment,
        _initialize_onexit_table => crt::_initialize_onexit_table,
        _register_onexit_function => crt::_register_onexit_function,
        _execute_onexit_table => crt::_execute_onexit_table,
        _crt_atexit => crt::_crt_atexit,
        _cexit => crt::_cexit,
        _initterm => crt::_initterm,
        _initterm_e => crt::_initterm_e,
        _seh_filter_dll => crt::_seh_filter_dll,
        strcpy_s => crt::strcpy_s,
        sin => crt::sin,
        exp => crt::exp,
        powf => crt::powf,
        tanf => crt::tanf,
        cosf => crt::cosf,
        atan2f => crt::atan2f,
        lround => crt::lround,
        // winrt.rs
        WindowsCreateString => winrt::WindowsCreateString,
        WindowsCreateStringReference => winrt::WindowsCreateStringReference,
        WindowsGetStringRawBuffer => winrt::WindowsGetStringRawBuffer,
        WindowsIsStringEmpty => winrt::WindowsIsStringEmpty,
        WindowsStringHasEmbeddedNull => winrt::WindowsStringHasEmbeddedNull,
        InitializeSRWLock => winrt::InitializeSRWLock,
        AcquireSRWLockExclusive => winrt::AcquireSRWLockExclusive,
        ReleaseSRWLockExclusive => winrt::ReleaseSRWLockExclusive,
        AcquireSRWLockShared => winrt::AcquireSRWLockShared,
        ReleaseSRWLockShared => winrt::ReleaseSRWLockShared,
        InitOnceExecuteOnce => winrt::InitOnceExecuteOnce,
        CoCreateFreeThreadedMarshaler => winrt::CoCreateFreeThreadedMarshaler,
        CoCreateInstanceFromApp => winrt::CoCreateInstanceFromApp,
        PropVariantClear => winrt::PropVariantClear,
        CoTaskMemAlloc => winrt::CoTaskMemAlloc,
        CoTaskMemFree => winrt::CoTaskMemFree,
        RoOriginateError => winrt::RoOriginateError,
        RoOriginateErrorW => winrt::RoOriginateErrorW,
        EventRegister => winrt::EventRegister,
        EventUnregister => winrt::EventUnregister,
        EventSetInformation => winrt::EventSetInformation,
        EventWriteTransfer => winrt::EventWriteTransfer,
        CompareStringOrdinal => winrt::CompareStringOrdinal,
        }
    })
}

/// Address of the built-in implementation of a Win32 function.
pub fn lookup(name: &str) -> Option<usize> {
    all().iter().find(|(n, _)| *n == name).map(|&(_, f)| f)
}

/// Names of every Win32 function the loader provides.
pub fn provided() -> impl Iterator<Item = &'static str> {
    all().iter().map(|(n, _)| *n)
}
