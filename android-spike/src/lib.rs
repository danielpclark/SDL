//! Android spike (plan PR 0): probes shared by the shell program (`probe`)
//! and the app (`spike_app`, which is `libmain.so`). No third-party crates;
//! the platform libraries are dlopened, as SDL does for most of them.

#![allow(clippy::missing_safety_doc, non_snake_case, non_camel_case_types, unused_unsafe)]

pub mod audio;
pub mod camera;
pub mod dl;
pub mod gfx;
pub mod jni;
pub mod sensors;

use std::ffi::{c_char, c_int, CString};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

/// Mirror every line to logcat (tag `spike`) as well as stdout.
pub static TO_LOGCAT: AtomicBool = AtomicBool::new(false);

pub fn out(s: &str) {
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "{s}");
    let _ = o.flush();
    drop(o);
    if TO_LOGCAT.load(Ordering::Relaxed) {
        logcat(s);
    }
}

pub fn logcat(s: &str) {
    let tag = c"spike";
    let text = CString::new(s.replace('\0', "\\0")).unwrap();
    unsafe { __android_log_write(4, tag.as_ptr(), text.as_ptr()) };
}

#[macro_export]
macro_rules! say {
    ($($t:tt)*) => { $crate::out(&format!($($t)*)) };
}

pub fn now_s() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// Ask the host runner to run a command (`adb ...`) and wait for it.
pub fn request(cmd: &str) {
    say!("spike-request: {cmd}");
}

/// The same, without waiting (the host starts it in the background).
pub fn request_bg(cmd: &str) {
    say!("spike-request-bg: {cmd}");
}

pub fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        return "(null)".into();
    }
    unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

pub fn fixed_cstr(b: &[u8]) -> String {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..n]).into_owned()
}
