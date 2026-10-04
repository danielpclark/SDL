// Rust translation of src/loadso/dlopen/SDL_sysloadso.c and
// src/loadso/windows/SDL_sysloadso.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Loading shared objects at run time (`SDL_loadso.h`).
//!
//! SDL loads most platform libraries (X11, Wayland, ALSA, PulseAudio,
//! XInput, ...) this way, so that a program built with every backend still
//! starts on a system missing some of them. [`SharedObject`] is
//! `SDL_SharedObject`: [`SharedObject::load`] is `SDL_LoadObject()`, dropping
//! it is `SDL_UnloadObject()`, and [`SharedObject::function`] is
//! `SDL_LoadFunction()`.

use crate::error::{Error, Result};
use std::ffi::{c_void, CString};
use std::fmt;
use std::ptr::NonNull;

/// A loaded shared object (`SDL_SharedObject`). Unloaded on drop.
pub struct SharedObject {
    handle: NonNull<c_void>,
    name: String,
}

// SAFETY: the handle is an opaque token the loader hands out; dlsym/dlclose
// and GetProcAddress/FreeLibrary may be called on it from any thread.
unsafe impl Send for SharedObject {}
// SAFETY: as above; nothing mutates through a shared reference.
unsafe impl Sync for SharedObject {}

impl fmt::Debug for SharedObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedObject")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl SharedObject {
    /// Load a shared object by file name or path. Translation of
    /// `SDL_LoadObject()`.
    pub fn load(sofile: &str) -> Result<SharedObject> {
        sys::load(sofile).map(|handle| SharedObject {
            handle,
            name: sofile.to_owned(),
        })
    }

    /// Load a shared object with its symbols made available to the ones
    /// loaded after it (`dlopen(sofile, RTLD_NOW | RTLD_GLOBAL)`), as the
    /// X11 driver loads libGL (its `GL_LoadObject()`).
    #[cfg(all(unix, not(target_vendor = "apple")))]
    pub(crate) fn load_global(sofile: &str) -> Result<SharedObject> {
        sys::load_with_flags(sofile, libc::RTLD_NOW | libc::RTLD_GLOBAL).map(|handle| {
            SharedObject {
                handle,
                name: sofile.to_owned(),
            }
        })
    }

    /// The name the object was loaded by.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The address of an exported symbol. Translation of
    /// `SDL_LoadFunction()` without the cast to a function type.
    pub fn symbol(&self, name: &str) -> Result<NonNull<c_void>> {
        let Ok(cname) = CString::new(name) else {
            return Err(Error::invalid_param("name"));
        };
        sys::symbol(self.handle, name, &cname)
    }

    /// An exported function. Translation of `SDL_LoadFunction()`.
    ///
    /// # Safety
    ///
    /// `F` must be a function pointer type (`unsafe extern "C" fn(...)` or
    /// `extern "system"`) matching the symbol's real signature and calling
    /// convention, and the function must not be called after this object is
    /// dropped.
    pub unsafe fn function<F: Copy>(&self, name: &str) -> Result<F> {
        const {
            assert!(size_of::<F>() == size_of::<*mut c_void>());
        }
        let p = self.symbol(name)?;
        // SAFETY: F is a function pointer type of the symbol's signature
        // (the caller's contract) and has the size of a pointer.
        Ok(unsafe { std::mem::transmute_copy::<*mut c_void, F>(&p.as_ptr()) })
    }

    /// A function exported by ordinal only (`GetProcAddress(handle,
    /// (LPCSTR)ordinal)`), which XInput needs for its undocumented exports.
    ///
    /// # Safety
    ///
    /// As for [`SharedObject::function`].
    #[cfg(windows)]
    pub(crate) unsafe fn function_by_ordinal<F: Copy>(&self, ordinal: u16) -> Result<F> {
        const {
            assert!(size_of::<F>() == size_of::<*mut c_void>());
        }
        let p = sys::ordinal(self.handle, ordinal)?;
        // SAFETY: F is a function pointer type of the symbol's signature
        // (the caller's contract) and has the size of a pointer.
        Ok(unsafe { std::mem::transmute_copy::<*mut c_void, F>(&p.as_ptr()) })
    }
}

impl Drop for SharedObject {
    /// Translation of `SDL_UnloadObject()`.
    fn drop(&mut self) {
        sys::unload(self.handle);
    }
}

#[cfg(unix)]
mod sys {
    use super::*;
    use std::ffi::CStr;

    fn dlerror_text() -> String {
        // SAFETY: dlerror() returns NULL or a NUL-terminated string that stays
        // valid until the next dl* call on this thread.
        unsafe {
            let e = libc::dlerror();
            if e.is_null() {
                String::new()
            } else {
                CStr::from_ptr(e).to_string_lossy().into_owned()
            }
        }
    }

    pub(super) fn load(sofile: &str) -> Result<NonNull<c_void>> {
        load_with_flags(sofile, libc::RTLD_NOW | libc::RTLD_LOCAL)
    }

    pub(super) fn load_with_flags(sofile: &str, flags: libc::c_int) -> Result<NonNull<c_void>> {
        let Ok(c) = CString::new(sofile) else {
            return Err(Error::invalid_param("sofile"));
        };
        // SAFETY: c is NUL-terminated; dlopen has no other preconditions.
        let handle = unsafe { libc::dlopen(c.as_ptr(), flags) };
        let loaderror = dlerror_text();
        NonNull::new(handle)
            .ok_or_else(|| Error::new(format!("Failed loading {sofile}: {loaderror}")))
    }

    pub(super) fn symbol(
        handle: NonNull<c_void>,
        name: &str,
        cname: &CStr,
    ) -> Result<NonNull<c_void>> {
        // SAFETY: handle came from dlopen and is still open; cname is
        // NUL-terminated.
        let symbol = unsafe { libc::dlsym(handle.as_ptr(), cname.as_ptr()) };
        if let Some(s) = NonNull::new(symbol) {
            return Ok(s);
        }
        // prepend an underscore for platforms that need that.
        let underscored = CString::new(format!("_{name}")).expect("no NUL in name");
        // SAFETY: as above.
        let symbol = unsafe { libc::dlsym(handle.as_ptr(), underscored.as_ptr()) };
        NonNull::new(symbol)
            .ok_or_else(|| Error::new(format!("Failed loading {name}: {}", dlerror_text())))
    }

    pub(super) fn unload(handle: NonNull<c_void>) {
        // SAFETY: handle came from dlopen and is closed once, here.
        unsafe {
            libc::dlclose(handle.as_ptr());
        }
    }
}

#[cfg(windows)]
mod sys {
    use super::*;
    use crate::core::windows::{set_error, utf8_to_wide};
    use std::ffi::CStr;
    use windows_sys::Win32::Foundation::FreeLibrary;
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

    pub(super) fn load(sofile: &str) -> Result<NonNull<c_void>> {
        let wstr = utf8_to_wide(sofile);
        // SAFETY: wstr is NUL-terminated UTF-16.
        let handle = unsafe { LoadLibraryW(wstr.as_ptr()) };
        // Generate an error message if all loads failed
        NonNull::new(handle).ok_or_else(|| set_error(&format!("Failed loading {sofile}")))
    }

    pub(super) fn symbol(
        handle: NonNull<c_void>,
        name: &str,
        cname: &CStr,
    ) -> Result<NonNull<c_void>> {
        // SAFETY: handle came from LoadLibraryW and is still loaded; cname is
        // NUL-terminated.
        let symbol = unsafe { GetProcAddress(handle.as_ptr(), cname.as_ptr().cast()) };
        match symbol {
            Some(f) => {
                Ok(NonNull::new(f as *mut c_void).expect("GetProcAddress returned a function"))
            }
            None => Err(set_error(&format!("Failed loading {name}"))),
        }
    }

    pub(super) fn ordinal(handle: NonNull<c_void>, ordinal: u16) -> Result<NonNull<c_void>> {
        // SAFETY: handle came from LoadLibraryW and is still loaded; an
        // ordinal is passed in the low word of the name pointer.
        let symbol = unsafe { GetProcAddress(handle.as_ptr(), ordinal as usize as *const u8) };
        match symbol {
            Some(f) => {
                Ok(NonNull::new(f as *mut c_void).expect("GetProcAddress returned a function"))
            }
            None => Err(set_error(&format!("Failed loading ordinal {ordinal}"))),
        }
    }

    pub(super) fn unload(handle: NonNull<c_void>) {
        // SAFETY: handle came from LoadLibraryW and is freed once, here.
        unsafe {
            FreeLibrary(handle.as_ptr());
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod sys {
    // Translation of src/loadso/dummy/SDL_sysloadso.c.
    use super::*;
    use std::ffi::CStr;

    pub(super) fn load(_sofile: &str) -> Result<NonNull<c_void>> {
        Err(Error::new("SDL_LoadObject() not implemented"))
    }

    pub(super) fn symbol(
        _handle: NonNull<c_void>,
        _name: &str,
        _cname: &CStr,
    ) -> Result<NonNull<c_void>> {
        Err(Error::new("SDL_LoadFunction() not implemented"))
    }

    pub(super) fn unload(_handle: NonNull<c_void>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    const LIBC: &str = "libc.so.6";
    #[cfg(windows)]
    const LIBC: &str = "kernel32.dll";

    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn load_and_call() {
        let so = SharedObject::load(LIBC).unwrap();
        assert_eq!(so.name(), LIBC);
        #[cfg(target_os = "linux")]
        {
            // SAFETY: getpid() takes no arguments and returns pid_t.
            let getpid: unsafe extern "C" fn() -> i32 = unsafe { so.function("getpid") }.unwrap();
            // SAFETY: getpid() has no preconditions.
            assert_eq!(unsafe { getpid() } as u32, std::process::id());
        }
        #[cfg(windows)]
        {
            // SAFETY: GetCurrentProcessId() takes no arguments and returns a DWORD.
            let getpid: unsafe extern "system" fn() -> u32 =
                unsafe { so.function("GetCurrentProcessId") }.unwrap();
            // SAFETY: GetCurrentProcessId() has no preconditions.
            assert_eq!(unsafe { getpid() }, std::process::id());
        }
        let err = so.symbol("SDL_no_such_symbol").unwrap_err();
        assert!(
            err.message()
                .starts_with("Failed loading SDL_no_such_symbol"),
            "{}",
            err.message()
        );
    }

    #[test]
    fn missing_object() {
        let err = SharedObject::load("libSDL-no-such-library.so.0").unwrap_err();
        assert!(
            err.message()
                .starts_with("Failed loading libSDL-no-such-library.so.0"),
            "{}",
            err.message()
        );
        assert!(SharedObject::load("a\0b").is_err());
    }
}
