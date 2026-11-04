// diag only (claude/diag-openxr-probe, never merged): the Khronos OpenXR
// loader for Android (the AAR's libopenxr_loader.so, put in the APK by the
// diag runner's SDL3_ANDROID_EXTRA_LIBS) in an app, as upstream's
// OPENXR_InitializeAndroidLoader() calls it, with no runtime installed.

#[cfg(not(target_os = "android"))]
fn main() {}

#[cfg(target_os = "android")]
#[used]
#[link_section = ".sdl3_runner"]
static RUNNER: [u8; 4] = *b"app\0";

#[cfg(target_os = "android")]
mod android_support;

#[cfg(target_os = "android")]
fn main() -> std::process::ExitCode {
    use std::ffi::{c_char, c_void, CStr};
    android_support::start();
    let status = android_support::checked(|| unsafe {
        type Gipa = unsafe extern "system" fn(u64, *const c_char, *mut *const c_void) -> i32;
        type Init = unsafe extern "system" fn(*const c_void) -> i32;
        type Exts = unsafe extern "system" fn(*const c_char, u32, *mut u32, *mut c_void) -> i32;
        type Layers = unsafe extern "system" fn(u32, *mut u32, *mut c_void) -> i32;
        let lib = sdl3::loadso::SharedObject::load("libopenxr_loader.so");
        println!("xrprobe: load: {:?}", lib.as_ref().map(|l| l.name().to_owned()).map_err(|e| e.to_string()));
        let lib = lib.expect("loader");
        for name in ["xrGetInstanceProcAddr", "xrEnumerateApiLayerProperties", "xrCreateInstance", "xrEnumerateInstanceExtensionProperties", "xrInitializeLoaderKHR"] {
            println!("xrprobe: export {name}: {}", lib.symbol(name).is_ok());
        }
        let gipa: Gipa = std::mem::transmute(lib.symbol("xrGetInstanceProcAddr").unwrap().as_ptr());
        let mut init: *const c_void = std::ptr::null();
        let r = gipa(0, c"xrInitializeLoaderKHR".as_ptr(), &mut init);
        println!("xrprobe: xrGetInstanceProcAddr(NULL, xrInitializeLoaderKHR) {r} {}", !init.is_null());
        let env = sdl3::android::jni_env().expect("env").as_ptr();
        let vm = android_support::jni::java_vm(env).0;
        let activity = sdl3::android::activity().expect("activity").as_ptr();
        #[repr(C)]
        struct LoaderInitInfoAndroid {
            ty: i32,
            next: *const c_void,
            vm: *mut c_void,
            context: *mut c_void,
        }
        let info = LoaderInitInfoAndroid { ty: 1000089000, next: std::ptr::null(), vm, context: activity };
        let init: Init = std::mem::transmute(init);
        let r = init(&info as *const _ as *const c_void);
        println!("xrprobe: xrInitializeLoaderKHR {r}");
        let r2 = init(&info as *const _ as *const c_void);
        println!("xrprobe: xrInitializeLoaderKHR again {r2}");
        for name in ["xrEnumerateApiLayerProperties", "xrCreateInstance", "xrEnumerateInstanceExtensionProperties"] {
            let mut f: *const c_void = std::ptr::null();
            let cname = std::ffi::CString::new(name).unwrap();
            let r = gipa(0, cname.as_ptr(), &mut f);
            println!("xrprobe: xrGetInstanceProcAddr(NULL, {name}) {r} {}", !f.is_null());
        }
        let layers: Layers = std::mem::transmute(lib.symbol("xrEnumerateApiLayerProperties").unwrap().as_ptr());
        let mut n = 0u32;
        let r = layers(0, &mut n, std::ptr::null_mut());
        println!("xrprobe: xrEnumerateApiLayerProperties {r} ({n})");
        let exts: Exts = std::mem::transmute(lib.symbol("xrEnumerateInstanceExtensionProperties").unwrap().as_ptr());
        let mut n = 0u32;
        let r = exts(std::ptr::null(), 0, &mut n, std::ptr::null_mut());
        println!("xrprobe: xrEnumerateInstanceExtensionProperties {r} ({n})");
        if r == 0 && n > 0 {
            let mut props = vec![[0u8; 152]; n as usize];
            for p in props.iter_mut() {
                p[0] = 2;
            }
            let r = exts(std::ptr::null(), n, &mut n, props.as_mut_ptr() as *mut c_void);
            println!("xrprobe: (2) {r}");
            for p in &props[..n as usize] {
                let name = CStr::from_bytes_until_nul(&p[16..144]).unwrap();
                println!("xrprobe: ext {name:?}");
            }
        }
        for line in android_support::request("logcat") {
            let l = line.to_lowercase();
            if l.contains("openxr") || l.contains("xr") && l.contains("loader") || l.contains("runtime") {
                println!("xrprobe: log: {line}");
            }
        }
    });
    android_support::finish(status)
}
