//! The JNI ABI by hand: the function table by index (from jni.h), with
//! only the calls the spike needs.

use std::ffi::{c_char, c_void, CString};

pub type jobject = *mut c_void;
pub type JavaVM = *mut *const *const c_void;
pub type jvalue = u64;

pub const JNI_VERSION_1_6: i32 = 0x0001_0006;

pub fn jl(o: jobject) -> jvalue {
    o as usize as u64
}
pub fn ji(i: i32) -> jvalue {
    i as u32 as u64
}
pub fn jj(i: i64) -> jvalue {
    i as u64
}
pub fn jz(b: bool) -> jvalue {
    b as u64
}

#[derive(Clone, Copy)]
pub struct Env(pub *mut c_void);

#[repr(C)]
pub struct NativeMethod {
    pub name: *const c_char,
    pub sig: *const c_char,
    pub f: *mut c_void,
}

macro_rules! jf {
    ($self:ident, $idx:expr, fn($($a:ty),*) -> $r:ty) => {
        unsafe {
            let table = *($self.0 as *const *const *const c_void);
            std::mem::transmute::<*const c_void, unsafe extern "system" fn(*mut c_void, $($a),*) -> $r>(*table.add($idx))
        }
    };
}

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

impl Env {
    pub fn find_class(self, name: &str) -> jobject {
        let n = c(name);
        unsafe { jf!(self, 6, fn(*const c_char) -> jobject)(self.0, n.as_ptr()) }
    }
    pub fn exception_check(self) -> bool {
        unsafe { jf!(self, 228, fn() -> u8)(self.0) != 0 }
    }
    /// Describe (to logcat) and clear a pending exception; true if there was one.
    pub fn exception_clear(self) -> bool {
        if self.exception_check() {
            unsafe {
                jf!(self, 16, fn() -> ())(self.0);
                jf!(self, 17, fn() -> ())(self.0);
            }
            true
        } else {
            false
        }
    }
    pub fn new_global_ref(self, o: jobject) -> jobject {
        unsafe { jf!(self, 21, fn(jobject) -> jobject)(self.0, o) }
    }
    pub fn delete_local_ref(self, o: jobject) {
        unsafe { jf!(self, 23, fn(jobject) -> ())(self.0, o) }
    }
    pub fn is_same_object(self, a: jobject, b: jobject) -> bool {
        unsafe { jf!(self, 24, fn(jobject, jobject) -> u8)(self.0, a, b) != 0 }
    }
    pub fn get_object_class(self, o: jobject) -> jobject {
        unsafe { jf!(self, 31, fn(jobject) -> jobject)(self.0, o) }
    }
    pub fn method(self, cls: jobject, name: &str, sig: &str) -> *mut c_void {
        let (n, s) = (c(name), c(sig));
        let m = unsafe { jf!(self, 33, fn(jobject, *const c_char, *const c_char) -> *mut c_void)(self.0, cls, n.as_ptr(), s.as_ptr()) };
        if m.is_null() {
            self.exception_clear();
            crate::say!("JNI: no method {name}{sig}");
        }
        m
    }
    pub fn static_method(self, cls: jobject, name: &str, sig: &str) -> *mut c_void {
        let (n, s) = (c(name), c(sig));
        let m = unsafe { jf!(self, 113, fn(jobject, *const c_char, *const c_char) -> *mut c_void)(self.0, cls, n.as_ptr(), s.as_ptr()) };
        if m.is_null() {
            self.exception_clear();
            crate::say!("JNI: no static method {name}{sig}");
        }
        m
    }
    pub fn field(self, cls: jobject, name: &str, sig: &str) -> *mut c_void {
        let (n, s) = (c(name), c(sig));
        unsafe { jf!(self, 94, fn(jobject, *const c_char, *const c_char) -> *mut c_void)(self.0, cls, n.as_ptr(), s.as_ptr()) }
    }
    pub fn static_field(self, cls: jobject, name: &str, sig: &str) -> *mut c_void {
        let (n, s) = (c(name), c(sig));
        unsafe { jf!(self, 144, fn(jobject, *const c_char, *const c_char) -> *mut c_void)(self.0, cls, n.as_ptr(), s.as_ptr()) }
    }
    pub fn new_object(self, cls: jobject, ctor: *mut c_void, args: &[jvalue]) -> jobject {
        unsafe { jf!(self, 30, fn(jobject, *mut c_void, *const jvalue) -> jobject)(self.0, cls, ctor, args.as_ptr()) }
    }
    pub fn call_object(self, o: jobject, m: *mut c_void, args: &[jvalue]) -> jobject {
        unsafe { jf!(self, 36, fn(jobject, *mut c_void, *const jvalue) -> jobject)(self.0, o, m, args.as_ptr()) }
    }
    pub fn call_bool(self, o: jobject, m: *mut c_void, args: &[jvalue]) -> bool {
        unsafe { jf!(self, 39, fn(jobject, *mut c_void, *const jvalue) -> u8)(self.0, o, m, args.as_ptr()) != 0 }
    }
    pub fn call_int(self, o: jobject, m: *mut c_void, args: &[jvalue]) -> i32 {
        unsafe { jf!(self, 51, fn(jobject, *mut c_void, *const jvalue) -> i32)(self.0, o, m, args.as_ptr()) }
    }
    pub fn call_float(self, o: jobject, m: *mut c_void, args: &[jvalue]) -> f32 {
        unsafe { jf!(self, 57, fn(jobject, *mut c_void, *const jvalue) -> f32)(self.0, o, m, args.as_ptr()) }
    }
    pub fn call_void(self, o: jobject, m: *mut c_void, args: &[jvalue]) {
        unsafe { jf!(self, 63, fn(jobject, *mut c_void, *const jvalue) -> ())(self.0, o, m, args.as_ptr()) }
    }
    pub fn call_nonvirtual_bool(self, o: jobject, cls: jobject, m: *mut c_void, args: &[jvalue]) -> bool {
        unsafe { jf!(self, 69, fn(jobject, jobject, *mut c_void, *const jvalue) -> u8)(self.0, o, cls, m, args.as_ptr()) != 0 }
    }
    pub fn call_nonvirtual_void(self, o: jobject, cls: jobject, m: *mut c_void, args: &[jvalue]) {
        unsafe { jf!(self, 93, fn(jobject, jobject, *mut c_void, *const jvalue) -> ())(self.0, o, cls, m, args.as_ptr()) }
    }
    pub fn get_object_field(self, o: jobject, f: *mut c_void) -> jobject {
        unsafe { jf!(self, 95, fn(jobject, *mut c_void) -> jobject)(self.0, o, f) }
    }
    pub fn get_long_field(self, o: jobject, f: *mut c_void) -> i64 {
        unsafe { jf!(self, 101, fn(jobject, *mut c_void) -> i64)(self.0, o, f) }
    }
    pub fn set_long_field(self, o: jobject, f: *mut c_void, v: i64) {
        unsafe { jf!(self, 110, fn(jobject, *mut c_void, i64) -> ())(self.0, o, f, v) }
    }
    pub fn call_static_object(self, cls: jobject, m: *mut c_void, args: &[jvalue]) -> jobject {
        unsafe { jf!(self, 116, fn(jobject, *mut c_void, *const jvalue) -> jobject)(self.0, cls, m, args.as_ptr()) }
    }
    pub fn get_static_object_field(self, cls: jobject, f: *mut c_void) -> jobject {
        unsafe { jf!(self, 145, fn(jobject, *mut c_void) -> jobject)(self.0, cls, f) }
    }
    pub fn get_static_int_field(self, cls: jobject, f: *mut c_void) -> i32 {
        unsafe { jf!(self, 150, fn(jobject, *mut c_void) -> i32)(self.0, cls, f) }
    }
    pub fn new_string(self, s: &str) -> jobject {
        let n = c(s);
        unsafe { jf!(self, 167, fn(*const c_char) -> jobject)(self.0, n.as_ptr()) }
    }
    pub fn string(self, s: jobject) -> String {
        if s.is_null() {
            return "(null)".into();
        }
        unsafe {
            let p = jf!(self, 169, fn(jobject, *mut u8) -> *const c_char)(self.0, s, std::ptr::null_mut());
            let r = crate::cstr(p);
            jf!(self, 170, fn(jobject, *const c_char) -> ())(self.0, s, p);
            r
        }
    }
    pub fn array_len(self, a: jobject) -> i32 {
        unsafe { jf!(self, 171, fn(jobject) -> i32)(self.0, a) }
    }
    pub fn object_array_get(self, a: jobject, i: i32) -> jobject {
        unsafe { jf!(self, 173, fn(jobject, i32) -> jobject)(self.0, a, i) }
    }
    pub fn int_array(self, a: jobject) -> Vec<i32> {
        let n = self.array_len(a);
        let mut v = vec![0i32; n.max(0) as usize];
        unsafe { jf!(self, 203, fn(jobject, i32, i32, *mut i32) -> ())(self.0, a, 0, n, v.as_mut_ptr()) };
        v
    }
    pub fn register_natives(self, cls: jobject, m: &[NativeMethod]) -> i32 {
        unsafe { jf!(self, 215, fn(jobject, *const NativeMethod, i32) -> i32)(self.0, cls, m.as_ptr(), m.len() as i32) }
    }
    pub fn get_java_vm(self) -> JavaVM {
        let mut vm: JavaVM = std::ptr::null_mut();
        unsafe { jf!(self, 219, fn(*mut JavaVM) -> i32)(self.0, &mut vm) };
        vm
    }
    pub fn new_direct_byte_buffer(self, p: *mut c_void, len: i64) -> jobject {
        unsafe { jf!(self, 229, fn(*mut c_void, i64) -> jobject)(self.0, p, len) }
    }

    // ---- conveniences
    /// `obj.name()` returning an object, by signature.
    pub fn obj(self, o: jobject, name: &str, sig: &str, args: &[jvalue]) -> jobject {
        let cls = self.get_object_class(o);
        let m = self.method(cls, name, sig);
        if m.is_null() {
            return std::ptr::null_mut();
        }
        let r = self.call_object(o, m, args);
        self.exception_clear();
        r
    }
    pub fn int(self, o: jobject, name: &str, sig: &str, args: &[jvalue]) -> i32 {
        let cls = self.get_object_class(o);
        let m = self.method(cls, name, sig);
        if m.is_null() {
            return i32::MIN;
        }
        let r = self.call_int(o, m, args);
        self.exception_clear();
        r
    }
    pub fn boolean(self, o: jobject, name: &str, sig: &str, args: &[jvalue]) -> bool {
        let cls = self.get_object_class(o);
        let m = self.method(cls, name, sig);
        if m.is_null() {
            return false;
        }
        let r = self.call_bool(o, m, args);
        self.exception_clear();
        r
    }
    pub fn str_of(self, o: jobject, name: &str, args: &[jvalue], sig: &str) -> String {
        let s = self.obj(o, name, sig, args);
        self.string(s)
    }
    pub fn to_string(self, o: jobject) -> String {
        if o.is_null() {
            return "(null)".into();
        }
        self.str_of(o, "toString", &[], "()Ljava/lang/String;")
    }
}

/// `vm->GetEnv`, or attach the thread.
pub fn env_for(vm: JavaVM) -> Env {
    unsafe {
        let table = *(vm as *const *const *const c_void);
        let get_env: unsafe extern "system" fn(JavaVM, *mut *mut c_void, i32) -> i32 = std::mem::transmute(*table.add(6));
        let attach: unsafe extern "system" fn(JavaVM, *mut *mut c_void, *mut c_void) -> i32 = std::mem::transmute(*table.add(4));
        let mut env = std::ptr::null_mut();
        if get_env(vm, &mut env, JNI_VERSION_1_6) != 0 {
            attach(vm, &mut env, std::ptr::null_mut());
        }
        Env(env)
    }
}
