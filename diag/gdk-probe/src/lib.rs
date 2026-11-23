//! Shared helpers of the two probes.
#![cfg(windows)]

use windows_sys::Win32::Foundation::{GetLastError, HMODULE};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, GetProcAddress, LoadLibraryW};

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// LoadLibraryW(name): the module, or GetLastError().
pub fn load(name: &str) -> Result<HMODULE, u32> {
    let w = wide(name);
    // SAFETY: a NUL-terminated name.
    let m = unsafe { LoadLibraryW(w.as_ptr()) };
    if m.is_null() {
        // SAFETY: no preconditions.
        Err(unsafe { GetLastError() })
    } else {
        Ok(m)
    }
}

/// GetProcAddress for one documented name: found, or GetLastError().
pub fn proc(m: HMODULE, name: &str) -> Result<usize, u32> {
    let c = std::ffi::CString::new(name).unwrap();
    // SAFETY: a loaded module and a NUL-terminated name.
    match unsafe { GetProcAddress(m, c.as_ptr().cast()) } {
        Some(f) => Ok(f as usize),
        // SAFETY: no preconditions.
        None => Err(unsafe { GetLastError() }),
    }
}

/// The loaded module's path.
pub fn module_path(m: HMODULE) -> String {
    let mut buf = [0u16; 1024];
    // SAFETY: the buffer's length is passed.
    let n = unsafe { GetModuleFileNameW(m, buf.as_mut_ptr(), buf.len() as u32) };
    String::from_utf16_lossy(&buf[..n as usize])
}

/// The file version of a file (VS_FIXEDFILEINFO), as a.b.c.d.
pub fn file_version(path: &str) -> Option<String> {
    let w = wide(path);
    // SAFETY: a NUL-terminated path.
    let size = unsafe { GetFileVersionInfoSizeW(w.as_ptr(), std::ptr::null_mut()) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    // SAFETY: the buffer holds size bytes.
    if unsafe { GetFileVersionInfoW(w.as_ptr(), 0, size, data.as_mut_ptr().cast()) } == 0 {
        return None;
    }
    let mut info: *mut VS_FIXEDFILEINFO = std::ptr::null_mut();
    let mut len = 0u32;
    let root = wide("\\");
    // SAFETY: the version block read above.
    if unsafe {
        VerQueryValueW(
            data.as_ptr().cast(),
            root.as_ptr(),
            (&mut info as *mut *mut VS_FIXEDFILEINFO).cast(),
            &mut len,
        )
    } == 0
        || info.is_null()
    {
        return None;
    }
    // SAFETY: VerQueryValueW pointed into the block.
    let i = unsafe { &*info };
    Some(format!(
        "{}.{}.{}.{}",
        i.dwFileVersionMS >> 16,
        i.dwFileVersionMS & 0xffff,
        i.dwFileVersionLS >> 16,
        i.dwFileVersionLS & 0xffff
    ))
}
