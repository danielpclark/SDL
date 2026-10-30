// The JNI calls the Android test programs make themselves.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A few calls of a `JNIEnv *`'s function table (`sdl3::android::jni_env()`'s),
//! by their index in jni.h, as an app's C makes them (`(*env)->FindClass(env,
//! ...)`): those upstream's C reference programs make, so the Rust ones make
//! the same. An exception a call leaves pending is described in the log,
//! cleared, and fails the program.

use std::ffi::{c_char, c_void, CStr, CString};

/// `JNIEnv *`.
pub type JNIEnv = *mut c_void;
/// `jobject`.
pub type Jobject = *mut c_void;
type MethodId = *mut c_void;

/// A `jvalue`.
#[repr(C)]
#[derive(Clone, Copy)]
pub union Jvalue {
    pub z: u8,
    pub i: i32,
    pub j: i64,
    pub f: f32,
    pub l: Jobject,
}

/// The function `index` of a table of functions (jni.h's order).
///
/// # Safety
///
/// `table` points to a JNI function table (a `JNIEnv`'s or a `JavaVM`'s),
/// and `F` is the function's type.
unsafe fn function<F: Copy>(table: *mut c_void, index: usize) -> F {
    // SAFETY: the caller's.
    unsafe {
        let functions = *(table as *const *const *const c_void);
        std::mem::transmute_copy(&*functions.add(index))
    }
}

/// `ExceptionCheck()`, then (when one is pending) `ExceptionDescribe()`
/// and `ExceptionClear()`: whether one was.
pub fn exception(env: JNIEnv) -> bool {
    // SAFETY: jni.h's functions 228, 16 and 17.
    unsafe {
        let check: unsafe extern "system" fn(JNIEnv) -> u8 = function(env, 228);
        if check(env) == 0 {
            return false;
        }
        let describe: unsafe extern "system" fn(JNIEnv) = function(env, 16);
        describe(env);
        let clear: unsafe extern "system" fn(JNIEnv) = function(env, 17);
        clear(env);
    }
    true
}

/// `FindClass()`.
pub fn find_class(env: JNIEnv, name: &str) -> Jobject {
    let cname = CString::new(name).unwrap();
    // SAFETY: jni.h's function 6.
    let class = unsafe {
        let f: unsafe extern "system" fn(JNIEnv, *const c_char) -> Jobject = function(env, 6);
        f(env, cname.as_ptr())
    };
    assert!(!exception(env) && !class.is_null(), "FindClass({name:?})");
    class
}

/// `NewGlobalRef()`.
pub fn new_global_ref(env: JNIEnv, obj: Jobject) -> Jobject {
    // SAFETY: jni.h's function 21.
    unsafe {
        let f: unsafe extern "system" fn(JNIEnv, Jobject) -> Jobject = function(env, 21);
        f(env, obj)
    }
}

/// `DeleteLocalRef()`.
pub fn delete_local_ref(env: JNIEnv, obj: Jobject) {
    // SAFETY: jni.h's function 23.
    unsafe {
        let f: unsafe extern "system" fn(JNIEnv, Jobject) = function(env, 23);
        f(env, obj)
    }
}

/// `CallStaticVoidMethodA()` of `class`'s `name` (`GetStaticMethodID()`).
pub fn call_static_void(env: JNIEnv, class: Jobject, name: &str, sig: &str, args: &[Jvalue]) {
    let (cname, csig) = (CString::new(name).unwrap(), CString::new(sig).unwrap());
    // SAFETY: jni.h's functions 113 and 143, with the method's arguments.
    unsafe {
        let get: unsafe extern "system" fn(
            JNIEnv,
            Jobject,
            *const c_char,
            *const c_char,
        ) -> MethodId = function(env, 113);
        let method = get(env, class, cname.as_ptr(), csig.as_ptr());
        assert!(
            !exception(env) && !method.is_null(),
            "GetStaticMethodID({name}{sig})"
        );
        let call: unsafe extern "system" fn(JNIEnv, Jobject, MethodId, *const Jvalue) =
            function(env, 143);
        call(env, class, method, args.as_ptr());
    }
    assert!(!exception(env), "{name}{sig} threw");
}

/// `GetMethodID()` of `obj`'s class (`GetObjectClass()`).
fn method(env: JNIEnv, obj: Jobject, name: &str, sig: &str) -> MethodId {
    let (cname, csig) = (CString::new(name).unwrap(), CString::new(sig).unwrap());
    // SAFETY: jni.h's functions 31 and 33.
    unsafe {
        let class_of: unsafe extern "system" fn(JNIEnv, Jobject) -> Jobject = function(env, 31);
        let class = class_of(env, obj);
        let get: unsafe extern "system" fn(
            JNIEnv,
            Jobject,
            *const c_char,
            *const c_char,
        ) -> MethodId = function(env, 33);
        let method = get(env, class, cname.as_ptr(), csig.as_ptr());
        delete_local_ref(env, class);
        assert!(
            !exception(env) && !method.is_null(),
            "GetMethodID({name}{sig})"
        );
        method
    }
}

/// `CallVoidMethodA()` of `obj`'s `name`.
pub fn call_void(env: JNIEnv, obj: Jobject, name: &str, sig: &str, args: &[Jvalue]) {
    let method = method(env, obj, name, sig);
    // SAFETY: jni.h's function 63, with the method's arguments.
    unsafe {
        let call: unsafe extern "system" fn(JNIEnv, Jobject, MethodId, *const Jvalue) =
            function(env, 63);
        call(env, obj, method, args.as_ptr());
    }
    assert!(!exception(env), "{name}{sig} threw");
}

/// `CallObjectMethodA()` of `obj`'s `name`: a local reference.
pub fn call_object(env: JNIEnv, obj: Jobject, name: &str, sig: &str, args: &[Jvalue]) -> Jobject {
    let method = method(env, obj, name, sig);
    // SAFETY: jni.h's function 36, with the method's arguments.
    let result = unsafe {
        let call: unsafe extern "system" fn(JNIEnv, Jobject, MethodId, *const Jvalue) -> Jobject =
            function(env, 36);
        call(env, obj, method, args.as_ptr())
    };
    assert!(!exception(env), "{name}{sig} threw");
    result
}

/// A Java string's characters (`GetStringUTFChars()`).
pub fn string(env: JNIEnv, string: Jobject) -> String {
    // SAFETY: jni.h's functions 169 and 170, of a string.
    unsafe {
        let get: unsafe extern "system" fn(JNIEnv, Jobject, *mut u8) -> *const c_char =
            function(env, 169);
        let chars = get(env, string, std::ptr::null_mut());
        let text = CStr::from_ptr(chars).to_string_lossy().into_owned();
        let release: unsafe extern "system" fn(JNIEnv, Jobject, *const c_char) = function(env, 170);
        release(env, string, chars);
        text
    }
}

/// `obj.getClass().getName()`.
pub fn class_name(env: JNIEnv, obj: Jobject) -> String {
    let class = call_object(env, obj, "getClass", "()Ljava/lang/Class;", &[]);
    let name = call_object(env, class, "getName", "()Ljava/lang/String;", &[]);
    let text = string(env, name);
    delete_local_ref(env, name);
    delete_local_ref(env, class);
    text
}

/// `JavaVM *`.
#[derive(Clone, Copy)]
pub struct JavaVm(pub *mut c_void);

// SAFETY: a process's Java VM, which any thread may use.
unsafe impl Send for JavaVm {}
unsafe impl Sync for JavaVm {}

/// `GetJavaVM()`.
pub fn java_vm(env: JNIEnv) -> JavaVm {
    // SAFETY: jni.h's function 219.
    unsafe {
        let get_vm: unsafe extern "system" fn(JNIEnv, *mut *mut c_void) -> i32 = function(env, 219);
        let mut vm: *mut c_void = std::ptr::null_mut();
        assert_eq!(get_vm(env, &mut vm), 0, "GetJavaVM()");
        JavaVm(vm)
    }
}

/// Whether the current thread is attached to the Java VM (the VM's
/// `GetEnv()` gives `JNI_OK`).
pub fn is_attached(vm: JavaVm) -> bool {
    // SAFETY: the invocation table's function 6.
    unsafe {
        let get_env: unsafe extern "system" fn(*mut c_void, *mut *mut c_void, i32) -> i32 =
            function(vm.0, 6);
        let mut out: *mut c_void = std::ptr::null_mut();
        get_env(vm.0, &mut out, 0x0001_0004) == 0 // JNI_VERSION_1_4: JNI_OK
    }
}

/// `NewStringUTF()`: a local reference.
pub fn new_string(env: JNIEnv, s: &str) -> Jobject {
    let cs = CString::new(s).unwrap();
    // SAFETY: jni.h's function 167, of a C string.
    let string = unsafe {
        let f: unsafe extern "system" fn(JNIEnv, *const c_char) -> Jobject = function(env, 167);
        f(env, cs.as_ptr())
    };
    assert!(!exception(env) && !string.is_null(), "NewStringUTF({s:?})");
    string
}

/// `NewObjectA()` of `class`'s constructor `sig`: a local reference.
pub fn new_object(env: JNIEnv, class: Jobject, sig: &str, args: &[Jvalue]) -> Jobject {
    let (cname, csig) = (CString::new("<init>").unwrap(), CString::new(sig).unwrap());
    // SAFETY: jni.h's functions 33 and 30, with the constructor's arguments.
    let object = unsafe {
        let get: unsafe extern "system" fn(
            JNIEnv,
            Jobject,
            *const c_char,
            *const c_char,
        ) -> MethodId = function(env, 33);
        let method = get(env, class, cname.as_ptr(), csig.as_ptr());
        assert!(
            !exception(env) && !method.is_null(),
            "GetMethodID(<init>{sig})"
        );
        let new: unsafe extern "system" fn(JNIEnv, Jobject, MethodId, *const Jvalue) -> Jobject =
            function(env, 30);
        new(env, class, method, args.as_ptr())
    };
    assert!(!exception(env) && !object.is_null(), "new {sig}");
    object
}

/// `GetStaticObjectField()` of `class`'s `name`: a local reference.
pub fn get_static_object_field(env: JNIEnv, class: Jobject, name: &str, sig: &str) -> Jobject {
    let (cname, csig) = (CString::new(name).unwrap(), CString::new(sig).unwrap());
    // SAFETY: jni.h's functions 144 and 145.
    let value = unsafe {
        let get_id: unsafe extern "system" fn(
            JNIEnv,
            Jobject,
            *const c_char,
            *const c_char,
        ) -> *mut c_void = function(env, 144);
        let field = get_id(env, class, cname.as_ptr(), csig.as_ptr());
        assert!(
            !exception(env) && !field.is_null(),
            "GetStaticFieldID({name})"
        );
        let get: unsafe extern "system" fn(JNIEnv, Jobject, *mut c_void) -> Jobject =
            function(env, 145);
        get(env, class, field)
    };
    assert!(!exception(env), "the field {name}");
    value
}

/// `CallIntMethodA()` of `obj`'s `name`.
pub fn call_int(env: JNIEnv, obj: Jobject, name: &str, sig: &str, args: &[Jvalue]) -> i32 {
    let method = method(env, obj, name, sig);
    // SAFETY: jni.h's function 51, with the method's arguments.
    let result = unsafe {
        let call: unsafe extern "system" fn(JNIEnv, Jobject, MethodId, *const Jvalue) -> i32 =
            function(env, 51);
        call(env, obj, method, args.as_ptr())
    };
    assert!(!exception(env), "{name}{sig} threw");
    result
}

/// `CallBooleanMethodA()` of `obj`'s `name`.
pub fn call_boolean(env: JNIEnv, obj: Jobject, name: &str, sig: &str, args: &[Jvalue]) -> bool {
    let method = method(env, obj, name, sig);
    // SAFETY: jni.h's function 39, with the method's arguments.
    let result = unsafe {
        let call: unsafe extern "system" fn(JNIEnv, Jobject, MethodId, *const Jvalue) -> u8 =
            function(env, 39);
        call(env, obj, method, args.as_ptr())
    };
    assert!(!exception(env), "{name}{sig} threw");
    result != 0
}

/// `GetStaticMethodID()` of `class`'s `name`.
fn static_method(env: JNIEnv, class: Jobject, name: &str, sig: &str) -> MethodId {
    let (cname, csig) = (CString::new(name).unwrap(), CString::new(sig).unwrap());
    // SAFETY: jni.h's function 113.
    let method = unsafe {
        let get: unsafe extern "system" fn(
            JNIEnv,
            Jobject,
            *const c_char,
            *const c_char,
        ) -> MethodId = function(env, 113);
        get(env, class, cname.as_ptr(), csig.as_ptr())
    };
    assert!(
        !exception(env) && !method.is_null(),
        "GetStaticMethodID({name}{sig})"
    );
    method
}

/// `CallStaticObjectMethodA()` of `class`'s `name`: a local reference.
pub fn call_static_object(
    env: JNIEnv,
    class: Jobject,
    name: &str,
    sig: &str,
    args: &[Jvalue],
) -> Jobject {
    let method = static_method(env, class, name, sig);
    // SAFETY: jni.h's function 116, with the method's arguments.
    let result = unsafe {
        let call: unsafe extern "system" fn(JNIEnv, Jobject, MethodId, *const Jvalue) -> Jobject =
            function(env, 116);
        call(env, class, method, args.as_ptr())
    };
    assert!(!exception(env), "{name}{sig} threw");
    result
}

/// `CallStaticLongMethodA()` of `class`'s `name`.
pub fn call_static_long(
    env: JNIEnv,
    class: Jobject,
    name: &str,
    sig: &str,
    args: &[Jvalue],
) -> i64 {
    let method = static_method(env, class, name, sig);
    // SAFETY: jni.h's function 134, with the method's arguments.
    let result = unsafe {
        let call: unsafe extern "system" fn(JNIEnv, Jobject, MethodId, *const Jvalue) -> i64 =
            function(env, 134);
        call(env, class, method, args.as_ptr())
    };
    assert!(!exception(env), "{name}{sig} threw");
    result
}

/// `CallBooleanMethodA()` of `obj`'s `name`: its result, or the exception
/// it throws (a local reference, cleared).
pub fn call_boolean_catching(
    env: JNIEnv,
    obj: Jobject,
    name: &str,
    sig: &str,
    args: &[Jvalue],
) -> Result<bool, Jobject> {
    let method = method(env, obj, name, sig);
    // SAFETY: jni.h's functions 39, 228, 15 and 17.
    unsafe {
        let call: unsafe extern "system" fn(JNIEnv, Jobject, MethodId, *const Jvalue) -> u8 =
            function(env, 39);
        let result = call(env, obj, method, args.as_ptr());
        let check: unsafe extern "system" fn(JNIEnv) -> u8 = function(env, 228);
        if check(env) == 0 {
            return Ok(result != 0);
        }
        let occurred: unsafe extern "system" fn(JNIEnv) -> Jobject = function(env, 15);
        let thrown = occurred(env);
        let clear: unsafe extern "system" fn(JNIEnv) = function(env, 17);
        clear(env);
        Err(thrown)
    }
}

/// `GetFieldID()` of `class`'s `name`.
fn field_id(env: JNIEnv, class: Jobject, name: &str, sig: &str) -> *mut c_void {
    let (cname, csig) = (CString::new(name).unwrap(), CString::new(sig).unwrap());
    // SAFETY: jni.h's function 94.
    let field = unsafe {
        let get: unsafe extern "system" fn(
            JNIEnv,
            Jobject,
            *const c_char,
            *const c_char,
        ) -> *mut c_void = function(env, 94);
        get(env, class, cname.as_ptr(), csig.as_ptr())
    };
    assert!(!exception(env) && !field.is_null(), "GetFieldID({name})");
    field
}

/// `SetIntField()` of `obj`'s (a `class`) `name`.
pub fn set_int_field(env: JNIEnv, class: Jobject, obj: Jobject, name: &str, value: i32) {
    let field = field_id(env, class, name, "I");
    // SAFETY: jni.h's function 109, of an int field.
    unsafe {
        let set: unsafe extern "system" fn(JNIEnv, Jobject, *mut c_void, i32) = function(env, 109);
        set(env, obj, field, value);
    }
}

/// `SetFloatField()` of `obj`'s (a `class`) `name`.
pub fn set_float_field(env: JNIEnv, class: Jobject, obj: Jobject, name: &str, value: f32) {
    let field = field_id(env, class, name, "F");
    // SAFETY: jni.h's function 111, of a float field.
    unsafe {
        let set: unsafe extern "system" fn(JNIEnv, Jobject, *mut c_void, f32) = function(env, 111);
        set(env, obj, field, value);
    }
}

/// `NewObjectArray()` of `length` nulls of `class`: a local reference.
pub fn new_object_array(env: JNIEnv, length: i32, class: Jobject) -> Jobject {
    // SAFETY: jni.h's function 172.
    let array = unsafe {
        let f: unsafe extern "system" fn(JNIEnv, i32, Jobject, Jobject) -> Jobject =
            function(env, 172);
        f(env, length, class, std::ptr::null_mut())
    };
    assert!(!exception(env) && !array.is_null(), "NewObjectArray()");
    array
}

/// `SetObjectArrayElement()`.
pub fn set_object_array_element(env: JNIEnv, array: Jobject, index: i32, value: Jobject) {
    // SAFETY: jni.h's function 174.
    unsafe {
        let f: unsafe extern "system" fn(JNIEnv, Jobject, i32, Jobject) = function(env, 174);
        f(env, array, index, value);
    }
    assert!(!exception(env), "SetObjectArrayElement()");
}
