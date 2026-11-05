// The test runtime's Android parts: the loader's initialization and the
// instance's Android information.
// Part of the repository's own test code (not a translation); see LICENSE.txt.

//! On Android, the runtime has what an application there must use:
//!
//! * `XR_KHR_loader_init` with `XR_KHR_loader_init_android`:
//!   `xrInitializeLoaderKHR`, which `xrGetInstanceProcAddr(XR_NULL_HANDLE,
//!   ...)` gives (the extension's presence, as the specification signals
//!   it: it isn't listed), and which takes an `XrLoaderInitInfoAndroidKHR`
//!   (the application's `JavaVM` and an `android.content.Context`). In an
//!   app's process it must come first: until it has succeeded the three
//!   global functions answer `XR_ERROR_INITIALIZATION_FAILED`;
//! * `XR_KHR_android_create_instance` (listed): `xrCreateInstance` needs it
//!   enabled and an `XrInstanceCreateInfoAndroidKHR` in the create info's
//!   chain (the `JavaVM` and the `android.app.Activity` that drives the
//!   session), else `XR_ERROR_VALIDATION_FAILURE`, as the specification
//!   asks of an Android runtime.
//!
//! The Java VM and the objects are checked as far as the process allows:
//! the VM must be the process's own (`JNI_GetCreatedJavaVMs`, from
//! `libnativehelper.so`, an NDK library since API 31, loaded at run time),
//! and each object a valid reference (`GetObjectRefType`) to an instance of
//! its class (`IsInstanceOf`) on the calling thread, when that thread is
//! attached to the VM (the application's local references are its
//! thread's). A process without a Java VM (a program the shell runs, as
//! the runtime's own tests on the emulator) has nothing to check them
//! against: there they only must not be null, and nothing needs the
//! loader's initialization first.

use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::{Mutex, OnceLock};

use crate::sys::*;

/// `JNI_VERSION_1_6`.
const JNI_VERSION_1_6: i32 = 0x0001_0006;
/// `JNI_OK`.
const JNI_OK: i32 = 0;
/// `JNIInvalidRefType`, `GetObjectRefType`'s answer for what isn't a
/// reference.
const JNI_INVALID_REF_TYPE: i32 = 0;

/// The `JNIInvokeInterface` slot of `GetEnv`.
const GET_ENV: usize = 6;
/// The `JNINativeInterface` slots of the functions called.
const FIND_CLASS: usize = 6;
const EXCEPTION_CLEAR: usize = 17;
const DELETE_LOCAL_REF: usize = 23;
const IS_INSTANCE_OF: usize = 32;
const EXCEPTION_CHECK: usize = 228;
const GET_OBJECT_REF_TYPE: usize = 232;

/// `RTLD_NOW` (bionic's).
const RTLD_NOW: c_int = 2;

#[link(name = "dl")]
extern "C" {
    fn dlopen(name: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// `JNI_GetCreatedJavaVMs`.
type PfnJniGetCreatedJavaVms =
    unsafe extern "system" fn(vms: *mut *mut c_void, size: i32, count: *mut i32) -> i32;

/// The process's Java VM (its address), if it has one.
fn process_vm() -> Option<usize> {
    static VM: OnceLock<Option<usize>> = OnceLock::new();
    *VM.get_or_init(|| {
        // SAFETY: NUL-terminated names; the library stays loaded.
        let f = unsafe {
            let library = dlopen(c"libnativehelper.so".as_ptr(), RTLD_NOW);
            if library.is_null() {
                return None;
            }
            let f = dlsym(library, c"JNI_GetCreatedJavaVMs".as_ptr());
            if f.is_null() {
                return None;
            }
            std::mem::transmute::<*mut c_void, PfnJniGetCreatedJavaVms>(f)
        };
        let mut vm: *mut c_void = std::ptr::null_mut();
        let mut count = 0;
        // SAFETY: room for one VM, and its count.
        let status = unsafe { f(&mut vm, 1, &mut count) };
        (status == JNI_OK && count > 0 && !vm.is_null()).then_some(vm as usize)
    })
}

/// A function of a JNI table (`JNIInvokeInterface`, `JNINativeInterface`),
/// by slot.
///
/// # Safety
///
/// `object` is a `JavaVM *` or a `JNIEnv *`, whose table has `slot`.
unsafe fn slot(object: *mut c_void, slot: usize) -> *const c_void {
    // SAFETY: the caller's; both point to their table's address.
    unsafe { *(*object.cast::<*const *const c_void>()).add(slot) }
}

/// The calling thread's `JNIEnv` of the VM, if it's attached.
///
/// # Safety
///
/// `vm` is the process's Java VM.
unsafe fn thread_env(vm: *mut c_void) -> Option<*mut c_void> {
    type GetEnv = unsafe extern "system" fn(*mut c_void, *mut *mut c_void, i32) -> i32;
    // SAFETY: the VM's GetEnv.
    let get_env: GetEnv = unsafe { std::mem::transmute(slot(vm, GET_ENV)) };
    let mut env = std::ptr::null_mut();
    // SAFETY: an out-pointer.
    let status = unsafe { get_env(vm, &mut env, JNI_VERSION_1_6) };
    (status == JNI_OK && !env.is_null()).then_some(env)
}

/// Whether `object` is a valid reference to an instance of `class` (a
/// JNI class name) in `env` (any pending exception of the lookup cleared).
///
/// # Safety
///
/// `env` is the calling thread's.
unsafe fn instance_of(env: *mut c_void, object: *mut c_void, class: &CStr) -> bool {
    type GetObjectRefType = unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32;
    type FindClass = unsafe extern "system" fn(*mut c_void, *const c_char) -> *mut c_void;
    type IsInstanceOf = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> u8;
    type ExceptionCheck = unsafe extern "system" fn(*mut c_void) -> u8;
    type EnvFn = unsafe extern "system" fn(*mut c_void);
    type DeleteLocalRef = unsafe extern "system" fn(*mut c_void, *mut c_void);
    // SAFETY: the environment's functions, with their arguments.
    unsafe {
        let ref_type: GetObjectRefType = std::mem::transmute(slot(env, GET_OBJECT_REF_TYPE));
        if ref_type(env, object) == JNI_INVALID_REF_TYPE {
            return false;
        }
        let find_class: FindClass = std::mem::transmute(slot(env, FIND_CLASS));
        let class = find_class(env, class.as_ptr());
        let exception_check: ExceptionCheck = std::mem::transmute(slot(env, EXCEPTION_CHECK));
        if exception_check(env) != 0 {
            let clear: EnvFn = std::mem::transmute(slot(env, EXCEPTION_CLEAR));
            clear(env);
        }
        if class.is_null() {
            return false;
        }
        let is_instance_of: IsInstanceOf = std::mem::transmute(slot(env, IS_INSTANCE_OF));
        let result = is_instance_of(env, object, class) != 0;
        let delete: DeleteLocalRef = std::mem::transmute(slot(env, DELETE_LOCAL_REF));
        delete(env, class);
        result
    }
}

/// Check an application's Java VM and object (of `class`): see the
/// module's documentation.
fn check(vm: *mut c_void, object: *mut c_void, class: &CStr) -> Result<(), XrResult> {
    if vm.is_null() || object.is_null() {
        return Err(XR_ERROR_VALIDATION_FAILURE);
    }
    let Some(process_vm) = process_vm() else {
        return Ok(());
    };
    if vm as usize != process_vm {
        return Err(XR_ERROR_VALIDATION_FAILURE);
    }
    // SAFETY: the process's VM, and this thread's environment of it.
    unsafe {
        match thread_env(vm) {
            Some(env) if !instance_of(env, object, class) => Err(XR_ERROR_VALIDATION_FAILURE),
            _ => Ok(()),
        }
    }
}

/// The Java VM the loader's initialization gave (its address), once it
/// succeeded.
static INITIALIZED: Mutex<Option<usize>> = Mutex::new(None);

/// Whether the global functions may be called: after a successful
/// `xrInitializeLoaderKHR`, or in a process without a Java VM.
pub fn global_functions_allowed() -> bool {
    process_vm().is_none()
        || INITIALIZED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
}

/// `xrInitializeLoaderKHR`.
pub unsafe extern "system" fn initialize_loader(
    loader_init_info: *const XrLoaderInitInfoBaseHeaderKHR,
) -> XrResult {
    if let Some(result) = crate::fail::xr_call("xrInitializeLoaderKHR") {
        return result;
    }
    if loader_init_info.is_null() {
        return XR_ERROR_VALIDATION_FAILURE;
    }
    // SAFETY: the application's structure, whose type comes first.
    if unsafe { (*loader_init_info).ty } != XR_TYPE_LOADER_INIT_INFO_ANDROID_KHR {
        return XR_ERROR_VALIDATION_FAILURE;
    }
    // SAFETY: checked above: an XrLoaderInitInfoAndroidKHR.
    let info = unsafe { &*loader_init_info.cast::<XrLoaderInitInfoAndroidKHR>() };
    if let Err(result) = check(
        info.application_vm,
        info.application_context,
        c"android/content/Context",
    ) {
        return result;
    }
    *INITIALIZED.lock().unwrap_or_else(|e| e.into_inner()) = Some(info.application_vm as usize);
    XR_SUCCESS
}

/// The Android part of `xrCreateInstance`'s checks: the extension enabled
/// (`enabled`), and the information of its structure in the chain from
/// `next`.
///
/// # Safety
///
/// `next` is a valid structure chain.
pub unsafe fn check_instance_create_info(
    next: *const c_void,
    enabled: bool,
) -> Result<(), XrResult> {
    let mut next = next;
    let mut found = None;
    while !next.is_null() {
        // SAFETY: the caller's; each structure starts with a base header.
        let header = unsafe { &*next.cast::<XrBaseHeader>() };
        if header.ty == XR_TYPE_INSTANCE_CREATE_INFO_ANDROID_KHR {
            // SAFETY: of that type.
            found = Some(unsafe { &*next.cast::<XrInstanceCreateInfoAndroidKHR>() });
        }
        next = header.next.cast();
    }
    let (Some(info), true) = (found, enabled) else {
        return Err(XR_ERROR_VALIDATION_FAILURE);
    };
    check(
        info.application_vm,
        info.application_activity,
        c"android/app/Activity",
    )?;
    // (the VM of the loader's initialization, when there was one)
    match *INITIALIZED.lock().unwrap_or_else(|e| e.into_inner()) {
        Some(vm) if vm != info.application_vm as usize => Err(XR_ERROR_VALIDATION_FAILURE),
        _ => Ok(()),
    }
}
