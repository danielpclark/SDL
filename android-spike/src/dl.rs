//! dlopen/dlsym.

use std::ffi::{c_char, c_int, c_void, CString};

#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(file: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

pub const RTLD_NOW: c_int = 2;

pub struct Lib(pub *mut c_void);
unsafe impl Send for Lib {}

impl Lib {
    pub fn open(name: &str) -> Result<Lib, String> {
        let c = CString::new(name).unwrap();
        let h = unsafe { dlopen(c.as_ptr(), RTLD_NOW) };
        if h.is_null() {
            Err(format!("dlopen({name}) failed: {}", crate::cstr(unsafe { dlerror() })))
        } else {
            Ok(Lib(h))
        }
    }

    pub fn raw(&self, name: &str) -> *mut c_void {
        let c = CString::new(name).unwrap();
        unsafe { dlsym(self.0, c.as_ptr()) }
    }

    /// A typed symbol (a function pointer type `T`).
    pub unsafe fn sym<T: Copy>(&self, name: &str) -> Result<T, String> {
        assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<*mut c_void>());
        let p = self.raw(name);
        if p.is_null() {
            Err(format!("dlsym({name}) failed: {}", crate::cstr(unsafe { dlerror() })))
        } else {
            Ok(unsafe { std::mem::transmute_copy(&p) })
        }
    }
}

pub fn last_error() -> String {
    crate::cstr(unsafe { dlerror() })
}

/// `f!(lib, "name", fn(args) -> ret)`: a typed `unsafe extern "C"` symbol, or
/// return the error.
#[macro_export]
macro_rules! f {
    ($lib:expr, $name:literal, fn($($a:ty),*) $(-> $r:ty)?) => {
        match unsafe { $lib.sym::<unsafe extern "C" fn($($a),*) $(-> $r)?>($name) } {
            Ok(f) => f,
            Err(e) => return Err(e),
        }
    };
}
