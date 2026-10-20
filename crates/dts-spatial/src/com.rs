//! Just enough WinRT for the encoder's settings: an `IPropertySet` (used as `IMap<String, Object>`) whose values
//! are `IPropertyValue` byte arrays. Objects are owned by Rust; their reference counts are not tracked (the
//! encoder only borrows them during `Initialize`).

use std::ffi::c_void;
use std::sync::OnceLock;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Guid(pub u32, pub u16, pub u16, pub [u8; 8]);

const S_OK: i32 = 0;
const E_NOTIMPL: i32 = 0x8000_4001_u32 as i32;
const E_BOUNDS: i32 = 0x8000_000b_u32 as i32;
const E_OUTOFMEMORY: i32 = 0x8007_000e_u32 as i32;
/// PropertyType_UInt8Array
const UINT8_ARRAY: i32 = 1025;

/// A COM object: vtable pointer first, then our data.
#[repr(C)]
struct Object<T> {
    vtbl: *const usize,
    data: T,
}

unsafe extern "win64" fn query(this: *mut c_void, _iid: *const Guid, out: *mut *mut c_void) -> i32 {
    // Every interface asked for is the one the object is (the encoder asks a property set for IMap).
    unsafe { *out = this };
    S_OK
}

unsafe extern "win64" fn refcount(_this: *mut c_void) -> u32 {
    1
}

unsafe extern "win64" fn not_impl(_this: *mut c_void) -> i32 {
    E_NOTIMPL
}

/// A vtable of `n` slots: IUnknown/IInspectable basics, the rest "not implemented" unless set.
fn vtable(n: usize, set: &[(usize, usize)]) -> &'static [usize] {
    let mut v = vec![not_impl as *const () as usize; n];
    v[0] = query as *const () as usize;
    v[1] = refcount as *const () as usize;
    v[2] = refcount as *const () as usize;
    for &(i, f) in set {
        v[i] = f;
    }
    Box::leak(v.into_boxed_slice())
}

// IPropertyValue: IInspectable (0-5), get_Type 6, ... GetUInt8Array 26.

unsafe extern "win64" fn value_type(_this: *mut c_void, out: *mut i32) -> i32 {
    unsafe { *out = UINT8_ARRAY };
    S_OK
}

unsafe extern "win64" fn get_u8_array(this: *mut c_void, len: *mut u32, data: *mut *mut u8) -> i32 {
    let bytes = unsafe { &(*(this as *const Object<Vec<u8>>)).data };
    // The caller frees the copy with CoTaskMemFree (the loader's is libc free).
    let p = unsafe { libc::malloc(bytes.len().max(1)) } as *mut u8;
    if p.is_null() {
        return E_OUTOFMEMORY;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        *len = bytes.len() as u32;
        *data = p;
    }
    S_OK
}

fn value_vtable() -> *const usize {
    static VT: OnceLock<usize> = OnceLock::new();
    *VT.get_or_init(|| {
        vtable(27, &[(6, value_type as *const () as usize), (26, get_u8_array as *const () as usize)]).as_ptr() as usize
    }) as *const usize
}

// IMap<HSTRING, IInspectable>: IInspectable (0-5), Lookup 6, get_Size 7, HasKey 8, GetView 9, Insert 10,
// Remove 11, Clear 12.

struct Map {
    entries: Vec<(String, Box<Object<Vec<u8>>>)>,
}

fn key(h: *mut c_void) -> String {
    let mut n = 0;
    let p = unsafe { pe_loader::shims::winrt::WindowsGetStringRawBuffer(h, &mut n) };
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, n as usize) })
}

unsafe extern "win64" fn lookup(this: *mut c_void, k: *mut c_void, out: *mut *mut c_void) -> i32 {
    let map = unsafe { &(*(this as *const Object<Map>)).data };
    match map.entries.iter().find(|(n, _)| *n == key(k)) {
        Some((_, v)) => {
            unsafe { *out = &**v as *const Object<Vec<u8>> as *mut c_void };
            S_OK
        }
        None => {
            unsafe { *out = std::ptr::null_mut() };
            E_BOUNDS
        }
    }
}

unsafe extern "win64" fn size(this: *mut c_void, out: *mut u32) -> i32 {
    unsafe { *out = (*(this as *const Object<Map>)).data.entries.len() as u32 };
    S_OK
}

unsafe extern "win64" fn has_key(this: *mut c_void, k: *mut c_void, out: *mut u8) -> i32 {
    let map = unsafe { &(*(this as *const Object<Map>)).data };
    unsafe { *out = map.entries.iter().any(|(n, _)| *n == key(k)) as u8 };
    S_OK
}

fn map_vtable() -> *const usize {
    static VT: OnceLock<usize> = OnceLock::new();
    *VT.get_or_init(|| {
        vtable(
            13,
            &[(6, lookup as *const () as usize), (7, size as *const () as usize), (8, has_key as *const () as usize)],
        )
        .as_ptr() as usize
    }) as *const usize
}

/// A read-only property set of named byte arrays.
pub struct PropertySet(Box<Object<Map>>);

impl PropertySet {
    pub fn new(blobs: Vec<(String, Vec<u8>)>) -> PropertySet {
        let entries = blobs.into_iter().map(|(n, b)| (n, Box::new(Object { vtbl: value_vtable(), data: b }))).collect();
        PropertySet(Box::new(Object { vtbl: map_vtable(), data: Map { entries } }))
    }

    pub fn as_ptr(&self) -> *mut c_void {
        &*self.0 as *const Object<Map> as *mut c_void
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_and_bytes() {
        let set = PropertySet::new(vec![("RuntimeSettingsBLOB".into(), vec![1, 2, 3])]);
        let p = set.as_ptr();
        unsafe {
            let vt = *(p as *const *const usize);
            let lookup: unsafe extern "win64" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> i32 =
                std::mem::transmute(*vt.add(6));
            let k = pe_loader::shims::winrt::hstring("RuntimeSettingsBLOB");
            let mut v = std::ptr::null_mut();
            assert_eq!(lookup(p, k, &mut v), S_OK);
            let vvt = *(v as *const *const usize);
            let get: unsafe extern "win64" fn(*mut c_void, *mut u32, *mut *mut u8) -> i32 =
                std::mem::transmute(*vvt.add(26));
            let (mut n, mut d) = (0u32, std::ptr::null_mut());
            assert_eq!(get(v, &mut n, &mut d), S_OK);
            assert_eq!(std::slice::from_raw_parts(d, n as usize), [1, 2, 3]);
            libc::free(d.cast());
            let missing = pe_loader::shims::winrt::hstring("Nope");
            assert_eq!(lookup(p, missing, &mut v), E_BOUNDS);
        }
    }
}
