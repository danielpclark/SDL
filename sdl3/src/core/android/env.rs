// Rust translation of the JNI helpers of src/core/android/SDL_android.c
// from Simple DirectMedia Layer (the JNIEnv of each thread, the local
// reference frames, the Java exceptions), with the wrappers of the JNI
// calls the Android layer makes.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The JNI layer: the process's Java VM (`mJavaVM`, which `JNI_OnLoad()`
//! gets) and each thread's environment ([`get_env`], `Android_JNI_GetEnv()`:
//! kept in a pthread key, attaching the threads that aren't the VM's and
//! detaching them when they end, as upstream does), and [`Env`], a thread's
//! `JNIEnv *` with the calls as methods:
//!
//! * references: [`Obj`] is a reference that is valid for a lifetime (a
//!   native method's arguments, for its call), [`Local`] a local reference
//!   the caller owns (deleted when dropped, which matters in loops: a
//!   native method has 16 at least, 512 on old Android versions), and
//!   [`Global`] a global one (any thread; deleted when dropped). A local
//!   reference can't outlive its environment, nor a local frame it's made
//!   in ([`Env::with_local_frame`], and upstream's [`LocalReferenceHolder`]);
//! * Java exceptions: every call that can throw checks for one after it,
//!   giving `Err(`[`Thrown`]`)` with the exception still pending, as JNI
//!   leaves it (the next JNI calls are undefined until it's cleared):
//!   [`Thrown::catch`] takes it as Java's `catch` would, and
//!   [`exception_occurred`] is upstream's `Android_JNI_ExceptionOccurred()`;
//! * method and field lookups by name and descriptor, the calls checked
//!   against the descriptor (the arguments' types and the return type),
//!   and `CallNonvirtual*Method()` for what upstream's Java does with
//!   `super`;
//! * strings converted through UTF-16 (`NewString()`, `GetStringRegion()`),
//!   exactly. Note: upstream converts with `GetStringUTFChars()` and
//!   `NewStringUTF()`, Modified UTF-8, which differs from UTF-8 for U+0000
//!   (two bytes, C0 80), and in JNI's specification for the characters
//!   beyond U+FFFF too (as their surrogates, three bytes each; ART gives
//!   and takes their UTF-8); the translations take or give UTF-8 where
//!   upstream does that;
//! * class lookup: `FindClass()` finds the framework's classes on any
//!   thread, but on a thread attached here (not one of the VM's) it uses
//!   the system class loader, which doesn't know the app's classes, so
//!   those are looked up through the app's class loader ([`app_class`],
//!   cached by `JNI_OnLoad()` with the classes of `super::classes`),
//!   as upstream looks its classes up only on the VM's threads.
//!
//! Nothing here can be used without a Java VM: a program spawned from the
//! shell (no app) has none, and [`get_env`] gives `None` there.

#![allow(dead_code)] // (the calls the translations to come make)

use super::jni::*;
use crate::error::Error;
use std::ffi::{c_void, CStr, CString};
use std::fmt;
use std::marker::PhantomData;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicI32, AtomicPtr, Ordering};
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// The Java VM and each thread's environment

/// `mJavaVM`.
static JAVA_VM: AtomicPtr<JavaVM> = AtomicPtr::new(ptr::null_mut());

/// Keep the process's Java VM (`JNI_OnLoad()`'s `mJavaVM = vm`).
pub(crate) fn set_java_vm(vm: *mut JavaVM) {
    JAVA_VM.store(vm, Ordering::Release);
}

/// The process's Java VM, once `JNI_OnLoad()` has run.
pub(crate) fn java_vm() -> Option<NonNull<JavaVM>> {
    NonNull::new(JAVA_VM.load(Ordering::Acquire))
}

/// The pthread key of each thread's `JNIEnv` (`mThreadKey`), created once
/// (`key_once`).
static THREAD_KEY: OnceLock<Option<libc::pthread_key_t>> = OnceLock::new();

/// Log to Android's log, as upstream's `__android_log_print()`s do.
pub(crate) fn log(priority: super::ndk::LogPriority, message: &str) {
    super::ndk::log_write(priority, "SDL", message);
}

/// Set local storage value. Translation of `Android_JNI_SetEnv()`.
fn set_env(env: *mut JNIEnv) -> bool {
    let Some(key) = thread_key() else {
        return false;
    };
    // SAFETY: the key is created; the value is a pointer stored as it is.
    let status = unsafe { libc::pthread_setspecific(key, env as *const c_void) };
    // (Note (upstream): `status < 0`, but the pthread functions give a
    // positive error number when they fail, so upstream never sees one)
    if status != 0 {
        log(
            super::ndk::LogPriority::Error,
            &format!("Failed pthread_setspecific() in Android_JNI_SetEnv() (err={status})"),
        );
        return false;
    }
    true
}

/// Destructor called for each thread where mThreadKey is not NULL.
/// Translation of `Android_JNI_ThreadDestroyed()`.
unsafe extern "C" fn thread_destroyed(value: *mut c_void) {
    // The thread is being destroyed, detach it from the Java VM and set the mThreadKey value to NULL as required
    let env = value as *mut JNIEnv;
    if !env.is_null() {
        if let Some(vm) = java_vm() {
            // SAFETY: a thread this layer attached, ending.
            unsafe { ((**vm.as_ptr()).DetachCurrentThread)(vm.as_ptr()) };
        }
        set_env(ptr::null_mut());
    }
}

/// Creation of local storage mThreadKey (`Android_JNI_CreateKey()`, once:
/// `Android_JNI_CreateKey_once()`). Note (upstream): `Android_JNI_GetEnv()`
/// reads the key even before `nativeSetupJNI()` creates it (a key that
/// isn't, then: 0, which another library may own); here it's created on
/// first use.
fn thread_key() -> Option<libc::pthread_key_t> {
    *THREAD_KEY.get_or_init(|| {
        let mut key: libc::pthread_key_t = 0;
        // SAFETY: an out-pointer and a destructor of the right type.
        let status = unsafe { libc::pthread_key_create(&mut key, Some(thread_destroyed)) };
        // (as in set_env(), a failure is a positive error number)
        if status != 0 {
            log(
                super::ndk::LogPriority::Error,
                &format!("Error initializing mThreadKey with pthread_key_create() (err={status})"),
            );
            return None;
        }
        Some(key)
    })
}

/// Translation of `Android_JNI_CreateKey_once()`.
pub(crate) fn create_key_once() {
    thread_key();
}

/// Save a VM thread's `JNIEnv` (`Android_JNI_SetEnv(env)` in
/// `nativeSetupJNI()` and `nativeInitMainThread()`), or forget it (`NULL`:
/// `nativeCleanupMainThread()`, for a thread the VM detaches itself).
pub(crate) fn store_env(env: Option<Env<'_>>) -> bool {
    set_env(env.map_or(ptr::null_mut(), Env::raw))
}

/// The current thread's `JNIEnv`: the one kept for it, else it's attached
/// to the Java VM (and detached when it ends), or `None` without a VM (a
/// program spawned from the shell) or when attaching fails. Translation of
/// `Android_JNI_GetEnv()`.
pub(crate) fn get_env() -> Option<Env<'static>> {
    // Get JNIEnv from the Thread local storage
    let key = thread_key()?;
    // SAFETY: the key is created.
    let mut env = unsafe { libc::pthread_getspecific(key) } as *mut JNIEnv;
    if env.is_null() {
        // If it fails, try to attach ! (e.g the thread isn't created with SDL_CreateThread()

        // There should be a JVM
        let Some(vm) = java_vm() else {
            log(super::ndk::LogPriority::Error, "Failed, there is no JavaVM");
            return None;
        };

        /* Attach the current thread to the JVM and get a JNIEnv.
         * It will be detached by pthread_create destructor 'Android_JNI_ThreadDestroyed' */
        // SAFETY: the VM's invocation table; an out-pointer.
        let status = unsafe {
            ((**vm.as_ptr()).AttachCurrentThread)(vm.as_ptr(), &mut env, ptr::null_mut())
        };
        if status < 0 {
            log(
                super::ndk::LogPriority::Error,
                &format!("Failed to attach current thread (err={status})"),
            );
            return None;
        }

        // Save JNIEnv into the Thread local storage
        if !set_env(env) {
            return None;
        }
    }
    // SAFETY: the thread's JNIEnv, valid until it ends (Env isn't Send).
    Some(unsafe { Env::from_raw(env) })
}

/// Set up an external thread for using JNI with [`get_env`]. Translation
/// of `Android_JNI_SetupThread()`.
pub(crate) fn setup_thread() -> bool {
    // There should be a JVM
    let Some(vm) = java_vm() else {
        log(super::ndk::LogPriority::Error, "Failed, there is no JavaVM");
        return false;
    };

    /* Attach the current thread to the JVM and get a JNIEnv.
     * It will be detached by pthread_create destructor 'Android_JNI_ThreadDestroyed' */
    let mut env: *mut JNIEnv = ptr::null_mut();
    // SAFETY: the VM's invocation table; an out-pointer.
    let status =
        unsafe { ((**vm.as_ptr()).AttachCurrentThread)(vm.as_ptr(), &mut env, ptr::null_mut()) };
    if status < 0 {
        log(
            super::ndk::LogPriority::Error,
            &format!("Failed to attach current thread (err={status})"),
        );
        return false;
    }

    // Save JNIEnv into the Thread local storage
    set_env(env)
}

/// `vm->GetEnv(JNI_VERSION_1_4)`: the environment of a thread the VM knows
/// (attached), without attaching it.
pub(crate) fn vm_env(vm: NonNull<JavaVM>) -> Option<Env<'static>> {
    let mut env: *mut c_void = ptr::null_mut();
    // SAFETY: the VM's invocation table; an out-pointer.
    let status = unsafe { ((**vm.as_ptr()).GetEnv)(vm.as_ptr(), &mut env, JNI_VERSION_1_4) };
    if status != JNI_OK || env.is_null() {
        return None;
    }
    // SAFETY: the thread's JNIEnv.
    Some(unsafe { Env::from_raw(env.cast()) })
}

// ---------------------------------------------------------------------------
// References

/// A reference (local or global) that is valid for `'a`, or null:
/// a native method's arguments (for its call), or one borrowed from a
/// [`Local`] or a [`Global`].
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub(crate) struct Obj<'a> {
    raw: jobject,
    _marker: PhantomData<&'a ()>,
}

impl<'a> Obj<'a> {
    /// `NULL`.
    pub(crate) const NULL: Obj<'static> = Obj {
        raw: ptr::null_mut(),
        _marker: PhantomData,
    };

    /// A reference JNI gave, valid for `'a`.
    ///
    /// # Safety
    ///
    /// `raw` is null, or a reference valid for `'a` on this thread (a
    /// native method's argument, for the call; a global reference, while
    /// it isn't deleted).
    pub(crate) unsafe fn from_raw(raw: jobject) -> Obj<'a> {
        Obj {
            raw,
            _marker: PhantomData,
        }
    }

    pub(crate) fn raw(self) -> jobject {
        self.raw
    }

    pub(crate) fn is_null(self) -> bool {
        self.raw.is_null()
    }
}

impl fmt::Debug for Obj<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Obj({:p})", self.raw)
    }
}

/// A local reference (not null), deleted when dropped.
pub(crate) struct Local<'a> {
    raw: NonNull<_jobject>,
    env: Env<'a>,
}

impl<'a> Local<'a> {
    /// A local reference JNI returned (null: `None`).
    ///
    /// # Safety
    ///
    /// `raw` is null, or a local reference of `env` that nothing else
    /// deletes.
    pub(crate) unsafe fn from_raw(env: Env<'a>, raw: jobject) -> Option<Local<'a>> {
        NonNull::new(raw).map(|raw| Local { raw, env })
    }

    /// The reference, for calls.
    pub(crate) fn obj(&self) -> Obj<'_> {
        // SAFETY: valid while self is.
        unsafe { Obj::from_raw(self.raw.as_ptr()) }
    }

    /// The reference, not deleted: a native method's result.
    pub(crate) fn into_raw(self) -> jobject {
        let raw = self.raw.as_ptr();
        std::mem::forget(self);
        raw
    }
}

impl Drop for Local<'_> {
    fn drop(&mut self) {
        // SAFETY: a local reference of the environment, deleted once.
        unsafe { (self.env.table().DeleteLocalRef)(self.env.raw(), self.raw.as_ptr()) };
    }
}

impl fmt::Debug for Local<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Local({:p})", self.raw)
    }
}

/// A global reference (not null), for any thread, deleted when dropped
/// (on a thread with a JNI environment: [`get_env`]).
pub(crate) struct Global {
    raw: NonNull<_jobject>,
}

// SAFETY: a global reference is valid on every thread until it's deleted.
unsafe impl Send for Global {}
// SAFETY: as above; nothing in it changes.
unsafe impl Sync for Global {}

impl Global {
    /// `NewGlobalRef()` (`None` for null, or when the VM is out of memory).
    pub(crate) fn new(env: Env<'_>, obj: Obj<'_>) -> Option<Global> {
        if obj.is_null() {
            return None;
        }
        // SAFETY: a valid reference.
        let raw = unsafe { (env.table().NewGlobalRef)(env.raw(), obj.raw()) };
        NonNull::new(raw).map(|raw| Global { raw })
    }

    /// The reference, for calls.
    pub(crate) fn obj(&self) -> Obj<'_> {
        // SAFETY: valid while self is.
        unsafe { Obj::from_raw(self.raw.as_ptr()) }
    }
}

impl Drop for Global {
    fn drop(&mut self) {
        if let Some(env) = get_env() {
            // SAFETY: a global reference, deleted once.
            unsafe { (env.table().DeleteGlobalRef)(env.raw(), self.raw.as_ptr()) };
        }
    }
}

impl fmt::Debug for Global {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Global({:p})", self.raw)
    }
}

/// A method ID (valid while its class is loaded, on any thread).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MethodId(NonNull<_jmethodID>);

// SAFETY: method IDs are the VM's, the same on every thread.
unsafe impl Send for MethodId {}
// SAFETY: as above.
unsafe impl Sync for MethodId {}

impl MethodId {
    pub(crate) fn raw(self) -> jmethodID {
        self.0.as_ptr()
    }
}

/// A field ID (valid while its class is loaded, on any thread).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FieldId(NonNull<_jfieldID>);

// SAFETY: as MethodId.
unsafe impl Send for FieldId {}
// SAFETY: as above.
unsafe impl Sync for FieldId {}

impl FieldId {
    pub(crate) fn raw(self) -> jfieldID {
        self.0.as_ptr()
    }
}

// ---------------------------------------------------------------------------
// Exceptions

/// A Java exception a call threw, still pending (JNI's state, in which the
/// next JNI calls but the exception functions are undefined): [`catch`]
/// takes it (Java's `catch`), [`clear`] and [`describe`] end it, and
/// dropping it leaves it pending (for the Java code a native method
/// returns to, or for [`exception_occurred`]).
///
/// [`catch`]: Thrown::catch
/// [`clear`]: Thrown::clear
/// [`describe`]: Thrown::describe
#[must_use = "the exception is still pending"]
pub(crate) struct Thrown<'a> {
    env: Env<'a>,
}

impl<'a> Thrown<'a> {
    /// The exception (`ExceptionOccurred()`), cleared (`ExceptionClear()`).
    pub(crate) fn catch(self) -> Local<'a> {
        let env = self.env;
        // SAFETY: an exception is pending.
        let throwable = unsafe { (env.table().ExceptionOccurred)(env.raw()) };
        // SAFETY: as above.
        unsafe { (env.table().ExceptionClear)(env.raw()) };
        // SAFETY: a new local reference, not null while one is pending.
        unsafe { Local::from_raw(env, throwable) }.expect("a pending exception")
    }

    /// Java's `catch (Exception e)`: the exception, cleared, when it's an
    /// `Exception`; else (an `Error`, which that doesn't catch) it's thrown
    /// again, pending.
    pub(crate) fn catch_exception(self) -> Result<Local<'a>, Thrown<'a>> {
        let env = self.env;
        let throwable = self.catch();
        let is_exception = env
            .find_class(c"java/lang/Exception")
            .map(|class| env.is_instance_of(throwable.obj(), class.obj()))
            .unwrap_or_else(|thrown| {
                thrown.clear();
                true
            });
        if is_exception {
            Ok(throwable)
        } else {
            env.throw(throwable.obj());
            Err(Thrown { env })
        }
    }

    /// Java's `catch (A | B e)` of the classes (`java/lang/InterruptedException`):
    /// the exception, cleared, when it's an instance of one of them; else
    /// it's thrown again, pending.
    pub(crate) fn catch_any(self, classes: &[&CStr]) -> Result<Local<'a>, Thrown<'a>> {
        let env = self.env;
        let throwable = self.catch();
        let caught = classes.iter().any(|name| {
            env.find_class(name)
                .map(|class| env.is_instance_of(throwable.obj(), class.obj()))
                .unwrap_or_else(|thrown| {
                    thrown.clear();
                    false
                })
        });
        if caught {
            Ok(throwable)
        } else {
            env.throw(throwable.obj());
            Err(Thrown { env })
        }
    }

    /// `ExceptionClear()`.
    pub(crate) fn clear(self) {
        self.env.exception_clear();
    }

    /// `ExceptionDescribe()`: its stack trace in the log, then
    /// `ExceptionClear()` (see [`Env::exception_describe`]).
    pub(crate) fn describe(self) {
        self.env.exception_describe();
        self.env.exception_clear();
    }
}

impl fmt::Debug for Thrown<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Thrown")
    }
}

/// A JNI call's result: `Err` when it threw (the exception pending).
pub(crate) type JniResult<'a, T> = Result<T, Thrown<'a>>;

/// The count of [`LocalReferenceHolder`]s initialized and not cleaned up
/// (`s_active`).
static S_ACTIVE: AtomicI32 = AtomicI32::new(0);

/// Upstream's guard of the local references a function makes: a local
/// frame (16 references), popped when it's dropped. Translation of `struct
/// LocalReferenceHolder` with `LocalReferenceHolder_Setup()`, `_Init()` and
/// `_Cleanup()` (`Drop`).
pub(crate) struct LocalReferenceHolder<'a> {
    env: Option<Env<'a>>,
    func: &'static str,
}

impl<'a> LocalReferenceHolder<'a> {
    /// Translation of `LocalReferenceHolder_Setup()`.
    pub(crate) fn setup(func: &'static str) -> LocalReferenceHolder<'a> {
        // (DEBUG_JNI: SDL_Log("Entering function %s", func))
        LocalReferenceHolder { env: None, func }
    }

    /// Push the frame (`LocalReferenceHolder_Init()`): the environment to
    /// make the function's references with, which they can't outlive.
    pub(crate) fn init<'h>(&'h mut self, env: Env<'a>) -> Result<Env<'h>, Error> {
        const CAPACITY: jint = 16;
        // SAFETY: a valid environment.
        if unsafe { (env.table().PushLocalFrame)(env.raw(), CAPACITY) } < 0 {
            return Err(Error::new("Failed to allocate enough JVM local references"));
        }
        S_ACTIVE.fetch_add(1, Ordering::AcqRel);
        self.env = Some(env);
        // SAFETY: the same environment, for a shorter lifetime.
        Ok(unsafe { Env::from_raw(env.raw()) })
    }
}

impl Drop for LocalReferenceHolder<'_> {
    /// Translation of `LocalReferenceHolder_Cleanup()`.
    fn drop(&mut self) {
        // (DEBUG_JNI: SDL_Log("Leaving function %s", refholder->m_func))
        let _ = self.func;
        if let Some(env) = self.env {
            // SAFETY: the frame init() pushed.
            unsafe { (env.table().PopLocalFrame)(env.raw(), ptr::null_mut()) };
            S_ACTIVE.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl fmt::Debug for LocalReferenceHolder<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalReferenceHolder")
            .field("func", &self.func)
            .finish()
    }
}

/// Test for an exception and, unless `silent`, describe it as an error
/// (its class's name, and its message after a colon when it has one, as
/// upstream's `SDL_SetError()` does): `Some` when one occurred (it's
/// cleared). With `silent` upstream sets no error, and the error here says
/// only that an exception was thrown. Translation of
/// `Android_JNI_ExceptionOccurred()`, which a [`LocalReferenceHolder`]
/// must be around.
pub(crate) fn exception_occurred(env: Env<'_>, silent: bool) -> Option<Error> {
    // Detect mismatch LocalReferenceHolder_Init/Cleanup
    crate::sdl_assert!(S_ACTIVE.load(Ordering::Acquire) > 0);

    let exception = env.exception_occurred()?;
    // Until this happens most JNI operations have undefined behaviour
    env.exception_clear();

    if silent {
        return Some(Error::new("a Java exception was thrown"));
    }
    let name = env
        .get_object_class(exception.obj())
        .and_then(|class| {
            env.call_method(class.obj(), c"getName", c"()Ljava/lang/String;", &[])
                .ok()
        })
        .and_then(Value::object)
        .and_then(|name| env.get_string(name.obj()).ok())
        .unwrap_or_default();
    let message = env
        .call_method(exception.obj(), c"getMessage", c"()Ljava/lang/String;", &[])
        .ok()
        .and_then(Value::object);
    Some(match message.and_then(|m| env.get_string(m.obj()).ok()) {
        Some(message) => Error::new(format!("{name}: {message}")),
        None => Error::new(name),
    })
}

// ---------------------------------------------------------------------------
// Arguments and results

/// An argument of a method or constructor, of the Java type the descriptor
/// says.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Arg<'a> {
    Boolean(bool),
    Byte(jbyte),
    Char(jchar),
    Short(jshort),
    Int(jint),
    Long(jlong),
    Float(jfloat),
    Double(jdouble),
    Object(Obj<'a>),
}

impl Arg<'_> {
    fn shorty(self) -> u8 {
        match self {
            Arg::Boolean(_) => b'Z',
            Arg::Byte(_) => b'B',
            Arg::Char(_) => b'C',
            Arg::Short(_) => b'S',
            Arg::Int(_) => b'I',
            Arg::Long(_) => b'J',
            Arg::Float(_) => b'F',
            Arg::Double(_) => b'D',
            Arg::Object(_) => b'L',
        }
    }

    fn value(self) -> jvalue {
        match self {
            Arg::Boolean(z) => jvalue { z: z as jboolean },
            Arg::Byte(b) => jvalue { b },
            Arg::Char(c) => jvalue { c },
            Arg::Short(s) => jvalue { s },
            Arg::Int(i) => jvalue { i },
            Arg::Long(j) => jvalue { j },
            Arg::Float(f) => jvalue { f },
            Arg::Double(d) => jvalue { d },
            Arg::Object(o) => jvalue { l: o.raw() },
        }
    }
}

impl From<bool> for Arg<'_> {
    fn from(v: bool) -> Self {
        Arg::Boolean(v)
    }
}
impl From<jint> for Arg<'_> {
    fn from(v: jint) -> Self {
        Arg::Int(v)
    }
}
impl From<jlong> for Arg<'_> {
    fn from(v: jlong) -> Self {
        Arg::Long(v)
    }
}
impl From<jfloat> for Arg<'_> {
    fn from(v: jfloat) -> Self {
        Arg::Float(v)
    }
}
impl From<jdouble> for Arg<'_> {
    fn from(v: jdouble) -> Self {
        Arg::Double(v)
    }
}
impl<'a> From<Obj<'a>> for Arg<'a> {
    fn from(v: Obj<'a>) -> Self {
        Arg::Object(v)
    }
}
impl<'a> From<&'a Local<'_>> for Arg<'a> {
    fn from(v: &'a Local<'_>) -> Self {
        Arg::Object(v.obj())
    }
}
impl<'a> From<&'a Global> for Arg<'a> {
    fn from(v: &'a Global) -> Self {
        Arg::Object(v.obj())
    }
}

/// A method's (or field's) value, of the Java type the descriptor says.
#[derive(Debug)]
pub(crate) enum Value<'a> {
    Void,
    Boolean(bool),
    Byte(jbyte),
    Char(jchar),
    Short(jshort),
    Int(jint),
    Long(jlong),
    Float(jfloat),
    Double(jdouble),
    /// A reference, `None` for `null`.
    Object(Option<Local<'a>>),
}

impl<'a> Value<'a> {
    /// A reference result (panics for another type: the descriptor is the
    /// caller's).
    pub(crate) fn object(self) -> Option<Local<'a>> {
        match self {
            Value::Object(o) => o,
            other => panic!("not an object: {other:?}"),
        }
    }

    pub(crate) fn boolean(self) -> bool {
        match self {
            Value::Boolean(z) => z,
            other => panic!("not a boolean: {other:?}"),
        }
    }

    pub(crate) fn int(self) -> jint {
        match self {
            Value::Int(i) => i,
            other => panic!("not an int: {other:?}"),
        }
    }

    pub(crate) fn long(self) -> jlong {
        match self {
            Value::Long(j) => j,
            other => panic!("not a long: {other:?}"),
        }
    }

    pub(crate) fn float(self) -> jfloat {
        match self {
            Value::Float(f) => f,
            other => panic!("not a float: {other:?}"),
        }
    }

    pub(crate) fn double(self) -> jdouble {
        match self {
            Value::Double(d) => d,
            other => panic!("not a double: {other:?}"),
        }
    }
}

/// The shorty of a method descriptor (return type first), checked.
fn method_shorty(descriptor: &CStr) -> Vec<u8> {
    let descriptor = descriptor.to_str().expect("an ASCII descriptor");
    let mut buf = [0u8; 32];
    let n = descriptor_shorty(descriptor, &mut buf);
    buf[..n].to_vec()
}

/// The shorty character of a field descriptor.
fn field_shorty(descriptor: &CStr) -> u8 {
    match descriptor.to_bytes().first() {
        Some(b'L' | b'[') => b'L',
        Some(&c @ (b'Z' | b'B' | b'C' | b'S' | b'I' | b'J' | b'F' | b'D')) => c,
        _ => panic!("not a field descriptor: {descriptor:?}"),
    }
}

/// The arguments as `jvalue`s, checked against the shorty's parameters.
fn values(shorty: &[u8], args: &[Arg<'_>], what: &CStr) -> Vec<jvalue> {
    assert!(
        shorty.len() == args.len() + 1
            && args.iter().zip(&shorty[1..]).all(|(a, &s)| a.shorty() == s),
        "the arguments {args:?} aren't {what:?}'s"
    );
    args.iter().map(|a| a.value()).collect()
}

// ---------------------------------------------------------------------------
// The environment

/// A thread's JNI environment (`JNIEnv *`), valid for `'a` on the thread it
/// belongs to (it isn't `Send`).
#[derive(Clone, Copy)]
pub(crate) struct Env<'a> {
    raw: NonNull<JNIEnv>,
    _marker: PhantomData<(&'a (), *const ())>,
}

impl fmt::Debug for Env<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Env({:p})", self.raw)
    }
}

/// The `Call<Type>MethodA()` of a descriptor's return type (its shorty
/// character), of a family: `instance` (`Call<Type>MethodA()`),
/// `nonvirtual` or `static_`, with its arguments after the environment's.
macro_rules! call_typed {
    ($env:expr, $shorty:expr, $family:ident, $($a:expr),*) => {{
        let env: Env<'_> = $env;
        let t = env.table();
        let e = env.raw();
        // SAFETY: the arguments are checked against the descriptor, and
        // the references are valid.
        unsafe { call_typed!(@ $family, t, e, env, $shorty, $($a),*) }
    }};
    (@ instance, $t:ident, $e:ident, $env:ident, $shorty:expr, $($a:expr),*) => {
        match $shorty {
            b'V' => { ($t.CallVoidMethodA)($e, $($a),*); Value::Void }
            b'Z' => Value::Boolean(($t.CallBooleanMethodA)($e, $($a),*) != 0),
            b'B' => Value::Byte(($t.CallByteMethodA)($e, $($a),*)),
            b'C' => Value::Char(($t.CallCharMethodA)($e, $($a),*)),
            b'S' => Value::Short(($t.CallShortMethodA)($e, $($a),*)),
            b'I' => Value::Int(($t.CallIntMethodA)($e, $($a),*)),
            b'J' => Value::Long(($t.CallLongMethodA)($e, $($a),*)),
            b'F' => Value::Float(($t.CallFloatMethodA)($e, $($a),*)),
            b'D' => Value::Double(($t.CallDoubleMethodA)($e, $($a),*)),
            _ => Value::Object(Local::from_raw($env, ($t.CallObjectMethodA)($e, $($a),*))),
        }
    };
    (@ nonvirtual, $t:ident, $e:ident, $env:ident, $shorty:expr, $($a:expr),*) => {
        match $shorty {
            b'V' => { ($t.CallNonvirtualVoidMethodA)($e, $($a),*); Value::Void }
            b'Z' => Value::Boolean(($t.CallNonvirtualBooleanMethodA)($e, $($a),*) != 0),
            b'B' => Value::Byte(($t.CallNonvirtualByteMethodA)($e, $($a),*)),
            b'C' => Value::Char(($t.CallNonvirtualCharMethodA)($e, $($a),*)),
            b'S' => Value::Short(($t.CallNonvirtualShortMethodA)($e, $($a),*)),
            b'I' => Value::Int(($t.CallNonvirtualIntMethodA)($e, $($a),*)),
            b'J' => Value::Long(($t.CallNonvirtualLongMethodA)($e, $($a),*)),
            b'F' => Value::Float(($t.CallNonvirtualFloatMethodA)($e, $($a),*)),
            b'D' => Value::Double(($t.CallNonvirtualDoubleMethodA)($e, $($a),*)),
            _ => Value::Object(Local::from_raw($env, ($t.CallNonvirtualObjectMethodA)($e, $($a),*))),
        }
    };
    (@ static_, $t:ident, $e:ident, $env:ident, $shorty:expr, $($a:expr),*) => {
        match $shorty {
            b'V' => { ($t.CallStaticVoidMethodA)($e, $($a),*); Value::Void }
            b'Z' => Value::Boolean(($t.CallStaticBooleanMethodA)($e, $($a),*) != 0),
            b'B' => Value::Byte(($t.CallStaticByteMethodA)($e, $($a),*)),
            b'C' => Value::Char(($t.CallStaticCharMethodA)($e, $($a),*)),
            b'S' => Value::Short(($t.CallStaticShortMethodA)($e, $($a),*)),
            b'I' => Value::Int(($t.CallStaticIntMethodA)($e, $($a),*)),
            b'J' => Value::Long(($t.CallStaticLongMethodA)($e, $($a),*)),
            b'F' => Value::Float(($t.CallStaticFloatMethodA)($e, $($a),*)),
            b'D' => Value::Double(($t.CallStaticDoubleMethodA)($e, $($a),*)),
            _ => Value::Object(Local::from_raw($env, ($t.CallStaticObjectMethodA)($e, $($a),*))),
        }
    };
}

impl<'a> Env<'a> {
    /// A `JNIEnv *` JNI gave (a native method's), or [`get_env`]'s.
    ///
    /// # Safety
    ///
    /// `raw` is the current thread's environment, valid for `'a`.
    pub(crate) unsafe fn from_raw(raw: *mut JNIEnv) -> Env<'a> {
        Env {
            raw: NonNull::new(raw).expect("a JNIEnv"),
            _marker: PhantomData,
        }
    }

    pub(crate) fn raw(self) -> *mut JNIEnv {
        self.raw.as_ptr()
    }

    fn table(self) -> &'a JNINativeInterface {
        // SAFETY: a JNIEnv points to its function table, which the VM
        // keeps.
        unsafe { &**self.raw.as_ptr() }
    }

    /// `Err` when the last call threw.
    fn check(self) -> JniResult<'a, ()> {
        if self.exception_check() {
            Err(Thrown { env: self })
        } else {
            Ok(())
        }
    }

    /// `Err` when an exception is pending (one this thread threw itself,
    /// with [`Env::throw`]).
    pub(crate) fn pending(self) -> JniResult<'a, ()> {
        self.check()
    }

    /// `GetVersion()`.
    pub(crate) fn get_version(self) -> jint {
        // SAFETY: no arguments.
        unsafe { (self.table().GetVersion)(self.raw()) }
    }

    /// `GetJavaVM()`.
    pub(crate) fn get_java_vm(self) -> Option<NonNull<JavaVM>> {
        let mut vm = ptr::null_mut();
        // SAFETY: an out-pointer.
        let status = unsafe { (self.table().GetJavaVM)(self.raw(), &mut vm) };
        if status == JNI_OK {
            NonNull::new(vm)
        } else {
            None
        }
    }

    // Exceptions

    /// `ExceptionCheck()`.
    pub(crate) fn exception_check(self) -> bool {
        // SAFETY: allowed with an exception pending.
        unsafe { (self.table().ExceptionCheck)(self.raw()) != 0 }
    }

    /// `ExceptionOccurred()`: the pending exception, still pending.
    pub(crate) fn exception_occurred(self) -> Option<Local<'a>> {
        // SAFETY: allowed with an exception pending.
        let raw = unsafe { (self.table().ExceptionOccurred)(self.raw()) };
        // SAFETY: a new local reference.
        unsafe { Local::from_raw(self, raw) }
    }

    /// `ExceptionClear()`.
    pub(crate) fn exception_clear(self) {
        // SAFETY: allowed with an exception pending.
        unsafe { (self.table().ExceptionClear)(self.raw()) };
    }

    /// `ExceptionDescribe()`: the pending exception's stack trace in the
    /// log. (The JNI specification says it clears the exception; ART's
    /// leaves it pending.)
    pub(crate) fn exception_describe(self) {
        // SAFETY: allowed with an exception pending.
        unsafe { (self.table().ExceptionDescribe)(self.raw()) };
    }

    /// `Throw()`: `throwable` becomes the pending exception (Java's `throw`,
    /// or a caught one thrown again).
    pub(crate) fn throw(self, throwable: Obj<'_>) -> bool {
        // SAFETY: a valid reference.
        unsafe { (self.table().Throw)(self.raw(), throwable.raw()) == JNI_OK }
    }

    /// `ThrowNew()`.
    pub(crate) fn throw_new(self, class: Obj<'_>, message: &CStr) -> bool {
        // SAFETY: a valid class and message.
        unsafe { (self.table().ThrowNew)(self.raw(), class.raw(), message.as_ptr()) == JNI_OK }
    }

    // References

    /// `NewLocalRef()`.
    pub(crate) fn new_local_ref(self, obj: Obj<'_>) -> Option<Local<'a>> {
        // SAFETY: a valid reference.
        let raw = unsafe { (self.table().NewLocalRef)(self.raw(), obj.raw()) };
        // SAFETY: a new local reference.
        unsafe { Local::from_raw(self, raw) }
    }

    /// `IsSameObject()`.
    pub(crate) fn is_same_object(self, a: Obj<'_>, b: Obj<'_>) -> bool {
        // SAFETY: valid references.
        unsafe { (self.table().IsSameObject)(self.raw(), a.raw(), b.raw()) != 0 }
    }

    /// `GetObjectRefType()`.
    pub(crate) fn get_object_ref_type(self, obj: Obj<'_>) -> jobjectRefType {
        // SAFETY: a valid reference.
        unsafe { (self.table().GetObjectRefType)(self.raw(), obj.raw()) }
    }

    /// `EnsureLocalCapacity()`.
    pub(crate) fn ensure_local_capacity(self, capacity: jint) -> JniResult<'a, ()> {
        // SAFETY: no references.
        let status = unsafe { (self.table().EnsureLocalCapacity)(self.raw(), capacity) };
        self.check()?;
        assert_eq!(status, JNI_OK);
        Ok(())
    }

    /// Run `f` in a local frame of `capacity` references
    /// (`PushLocalFrame()`, then `PopLocalFrame()`): the references it
    /// makes are deleted when it returns, and can't outlive it.
    pub(crate) fn with_local_frame<R>(
        self,
        capacity: jint,
        f: impl for<'f> FnOnce(Env<'f>) -> R,
    ) -> JniResult<'a, R> {
        // SAFETY: no references.
        if unsafe { (self.table().PushLocalFrame)(self.raw(), capacity) } < 0 {
            return Err(Thrown { env: self });
        }
        struct Pop<'p>(Env<'p>);
        impl Drop for Pop<'_> {
            fn drop(&mut self) {
                // SAFETY: the frame pushed above.
                unsafe { (self.0.table().PopLocalFrame)(self.0.raw(), ptr::null_mut()) };
            }
        }
        let _pop = Pop(self);
        Ok(f(self))
    }

    // Classes

    /// `FindClass()` of a class name with slashes (`android/os/Build`): a
    /// framework's class, on any thread; an app's class only on the VM's
    /// threads (see [`app_class`]).
    pub(crate) fn find_class(self, name: &CStr) -> JniResult<'a, Local<'a>> {
        // SAFETY: a NUL-terminated name.
        let raw = unsafe { (self.table().FindClass)(self.raw(), name.as_ptr()) };
        self.check()?;
        // SAFETY: a new local reference.
        Ok(unsafe { Local::from_raw(self, raw) }.expect("FindClass() threw none"))
    }

    /// `GetObjectClass()`.
    pub(crate) fn get_object_class(self, obj: Obj<'_>) -> Option<Local<'a>> {
        // SAFETY: a valid reference.
        let raw = unsafe { (self.table().GetObjectClass)(self.raw(), obj.raw()) };
        // SAFETY: a new local reference.
        unsafe { Local::from_raw(self, raw) }
    }

    /// `GetSuperclass()`.
    pub(crate) fn get_superclass(self, class: Obj<'_>) -> Option<Local<'a>> {
        // SAFETY: a valid class.
        let raw = unsafe { (self.table().GetSuperclass)(self.raw(), class.raw()) };
        // SAFETY: a new local reference.
        unsafe { Local::from_raw(self, raw) }
    }

    /// `IsInstanceOf()`.
    pub(crate) fn is_instance_of(self, obj: Obj<'_>, class: Obj<'_>) -> bool {
        // SAFETY: valid references.
        unsafe { (self.table().IsInstanceOf)(self.raw(), obj.raw(), class.raw()) != 0 }
    }

    // Monitors

    /// Run `f` holding `obj`'s monitor (`MonitorEnter()`, then
    /// `MonitorExit()`): Java's `synchronized (obj) { ... }`.
    pub(crate) fn synchronized<R>(self, obj: Obj<'_>, f: impl FnOnce() -> R) -> JniResult<'a, R> {
        // SAFETY: a valid reference.
        let status = unsafe { (self.table().MonitorEnter)(self.raw(), obj.raw()) };
        self.check()?;
        assert_eq!(status, JNI_OK, "MonitorEnter()");
        struct Exit<'e, 'o>(Env<'e>, Obj<'o>);
        impl Drop for Exit<'_, '_> {
            fn drop(&mut self) {
                // SAFETY: the monitor entered above, on this thread.
                unsafe { (self.0.table().MonitorExit)(self.0.raw(), self.1.raw()) };
            }
        }
        let _exit = Exit(self, obj);
        Ok(f())
    }

    /// `RegisterNatives()`.
    pub(crate) fn register_natives(
        self,
        class: Obj<'_>,
        methods: &[JNINativeMethod],
    ) -> JniResult<'a, jint> {
        // SAFETY: the methods' names and signatures are C strings, their
        // functions take what the signatures say (super::classes checks).
        let status = unsafe {
            (self.table().RegisterNatives)(
                self.raw(),
                class.raw(),
                methods.as_ptr(),
                methods.len() as jint,
            )
        };
        self.check()?;
        Ok(status)
    }

    // Method and field IDs

    /// `GetMethodID()`.
    pub(crate) fn get_method_id(
        self,
        class: Obj<'_>,
        name: &CStr,
        descriptor: &CStr,
    ) -> JniResult<'a, MethodId> {
        // SAFETY: a valid class; C strings.
        let raw = unsafe {
            (self.table().GetMethodID)(self.raw(), class.raw(), name.as_ptr(), descriptor.as_ptr())
        };
        self.check()?;
        Ok(MethodId(
            NonNull::new(raw).expect("GetMethodID() threw none"),
        ))
    }

    /// `GetStaticMethodID()`.
    pub(crate) fn get_static_method_id(
        self,
        class: Obj<'_>,
        name: &CStr,
        descriptor: &CStr,
    ) -> JniResult<'a, MethodId> {
        // SAFETY: a valid class; C strings.
        let raw = unsafe {
            (self.table().GetStaticMethodID)(
                self.raw(),
                class.raw(),
                name.as_ptr(),
                descriptor.as_ptr(),
            )
        };
        self.check()?;
        Ok(MethodId(
            NonNull::new(raw).expect("GetStaticMethodID() threw none"),
        ))
    }

    /// `GetFieldID()`.
    pub(crate) fn get_field_id(
        self,
        class: Obj<'_>,
        name: &CStr,
        descriptor: &CStr,
    ) -> JniResult<'a, FieldId> {
        // SAFETY: a valid class; C strings.
        let raw = unsafe {
            (self.table().GetFieldID)(self.raw(), class.raw(), name.as_ptr(), descriptor.as_ptr())
        };
        self.check()?;
        Ok(FieldId(NonNull::new(raw).expect("GetFieldID() threw none")))
    }

    /// `GetStaticFieldID()`.
    pub(crate) fn get_static_field_id(
        self,
        class: Obj<'_>,
        name: &CStr,
        descriptor: &CStr,
    ) -> JniResult<'a, FieldId> {
        // SAFETY: a valid class; C strings.
        let raw = unsafe {
            (self.table().GetStaticFieldID)(
                self.raw(),
                class.raw(),
                name.as_ptr(),
                descriptor.as_ptr(),
            )
        };
        self.check()?;
        Ok(FieldId(
            NonNull::new(raw).expect("GetStaticFieldID() threw none"),
        ))
    }

    // Calls

    /// `obj.name(args)`: `GetMethodID()` of `obj`'s class, then the
    /// `Call<Type>MethodA()` of the descriptor's return type, with the
    /// arguments checked against its parameters (a mismatch panics).
    pub(crate) fn call_method(
        self,
        obj: Obj<'_>,
        name: &CStr,
        descriptor: &CStr,
        args: &[Arg<'_>],
    ) -> JniResult<'a, Value<'a>> {
        let shorty = method_shorty(descriptor);
        let args = values(&shorty, args, descriptor);
        let class = self.get_object_class(obj).expect("a non-null object");
        let method = self.get_method_id(class.obj(), name, descriptor)?;
        let value = call_typed!(
            self,
            shorty[0],
            instance,
            obj.raw(),
            method.raw(),
            args.as_ptr()
        );
        self.check()?;
        Ok(value)
    }

    /// The method `method` of `obj`, by its ID (a cached one).
    ///
    /// # Safety
    ///
    /// `method` is a method of `obj`'s class whose descriptor is
    /// `descriptor`.
    pub(crate) unsafe fn call_method_id(
        self,
        obj: Obj<'_>,
        method: MethodId,
        descriptor: &CStr,
        args: &[Arg<'_>],
    ) -> JniResult<'a, Value<'a>> {
        let shorty = method_shorty(descriptor);
        let args = values(&shorty, args, descriptor);
        let value = call_typed!(
            self,
            shorty[0],
            instance,
            obj.raw(),
            method.raw(),
            args.as_ptr()
        );
        self.check()?;
        Ok(value)
    }

    /// `class`'s method `name`, called on `obj` without virtual dispatch
    /// (`CallNonvirtual<Type>MethodA()`): Java's `super.name(args)`, with
    /// `class` the superclass.
    pub(crate) fn call_nonvirtual_method(
        self,
        obj: Obj<'_>,
        class: Obj<'_>,
        name: &CStr,
        descriptor: &CStr,
        args: &[Arg<'_>],
    ) -> JniResult<'a, Value<'a>> {
        let shorty = method_shorty(descriptor);
        let args = values(&shorty, args, descriptor);
        let method = self.get_method_id(class, name, descriptor)?;
        let value = call_typed!(
            self,
            shorty[0],
            nonvirtual,
            obj.raw(),
            class.raw(),
            method.raw(),
            args.as_ptr()
        );
        self.check()?;
        Ok(value)
    }

    /// The static method `class.name(args)`.
    pub(crate) fn call_static_method(
        self,
        class: Obj<'_>,
        name: &CStr,
        descriptor: &CStr,
        args: &[Arg<'_>],
    ) -> JniResult<'a, Value<'a>> {
        let shorty = method_shorty(descriptor);
        let args = values(&shorty, args, descriptor);
        let method = self.get_static_method_id(class, name, descriptor)?;
        let value = call_typed!(
            self,
            shorty[0],
            static_,
            class.raw(),
            method.raw(),
            args.as_ptr()
        );
        self.check()?;
        Ok(value)
    }

    /// `new Class(args)` (`NewObjectA()` of the constructor of `descriptor`).
    pub(crate) fn new_object(
        self,
        class: Obj<'_>,
        descriptor: &CStr,
        args: &[Arg<'_>],
    ) -> JniResult<'a, Local<'a>> {
        let shorty = method_shorty(descriptor);
        assert_eq!(shorty[0], b'V', "a constructor returns void");
        let args = values(&shorty, args, descriptor);
        let constructor = self.get_method_id(class, c"<init>", descriptor)?;
        // SAFETY: the arguments are checked against the descriptor.
        let raw = unsafe {
            (self.table().NewObjectA)(self.raw(), class.raw(), constructor.raw(), args.as_ptr())
        };
        self.check()?;
        // SAFETY: a new local reference.
        Ok(unsafe { Local::from_raw(self, raw) }.expect("NewObjectA() threw none"))
    }

    // Fields

    /// `obj.name` (`Get<Type>Field()` of the descriptor's type).
    pub(crate) fn get_field(
        self,
        obj: Obj<'_>,
        name: &CStr,
        descriptor: &CStr,
    ) -> JniResult<'a, Value<'a>> {
        let class = self.get_object_class(obj).expect("a non-null object");
        let field = self.get_field_id(class.obj(), name, descriptor)?;
        let (t, e, o, f) = (self.table(), self.raw(), obj.raw(), field.raw());
        // SAFETY: the field's type is the descriptor's.
        let value = unsafe {
            match field_shorty(descriptor) {
                b'Z' => Value::Boolean((t.GetBooleanField)(e, o, f) != 0),
                b'B' => Value::Byte((t.GetByteField)(e, o, f)),
                b'C' => Value::Char((t.GetCharField)(e, o, f)),
                b'S' => Value::Short((t.GetShortField)(e, o, f)),
                b'I' => Value::Int((t.GetIntField)(e, o, f)),
                b'J' => Value::Long((t.GetLongField)(e, o, f)),
                b'F' => Value::Float((t.GetFloatField)(e, o, f)),
                b'D' => Value::Double((t.GetDoubleField)(e, o, f)),
                _ => Value::Object(Local::from_raw(self, (t.GetObjectField)(e, o, f))),
            }
        };
        Ok(value)
    }

    /// `obj.name = value` (`Set<Type>Field()`).
    pub(crate) fn set_field(
        self,
        obj: Obj<'_>,
        name: &CStr,
        descriptor: &CStr,
        value: Arg<'_>,
    ) -> JniResult<'a, ()> {
        assert_eq!(
            value.shorty(),
            field_shorty(descriptor),
            "{value:?} isn't a {descriptor:?}"
        );
        let class = self.get_object_class(obj).expect("a non-null object");
        let field = self.get_field_id(class.obj(), name, descriptor)?;
        let (t, e, o, f) = (self.table(), self.raw(), obj.raw(), field.raw());
        // SAFETY: the value's type is the field's.
        unsafe {
            match value {
                Arg::Boolean(z) => (t.SetBooleanField)(e, o, f, z as jboolean),
                Arg::Byte(b) => (t.SetByteField)(e, o, f, b),
                Arg::Char(c) => (t.SetCharField)(e, o, f, c),
                Arg::Short(s) => (t.SetShortField)(e, o, f, s),
                Arg::Int(i) => (t.SetIntField)(e, o, f, i),
                Arg::Long(j) => (t.SetLongField)(e, o, f, j),
                Arg::Float(v) => (t.SetFloatField)(e, o, f, v),
                Arg::Double(d) => (t.SetDoubleField)(e, o, f, d),
                Arg::Object(l) => (t.SetObjectField)(e, o, f, l.raw()),
            }
        }
        Ok(())
    }

    /// `Class.name`, a static field (`GetStatic<Type>Field()`).
    pub(crate) fn get_static_field(
        self,
        class: Obj<'_>,
        name: &CStr,
        descriptor: &CStr,
    ) -> JniResult<'a, Value<'a>> {
        let field = self.get_static_field_id(class, name, descriptor)?;
        let (t, e, c, f) = (self.table(), self.raw(), class.raw(), field.raw());
        // SAFETY: the field's type is the descriptor's.
        let value = unsafe {
            match field_shorty(descriptor) {
                b'Z' => Value::Boolean((t.GetStaticBooleanField)(e, c, f) != 0),
                b'B' => Value::Byte((t.GetStaticByteField)(e, c, f)),
                b'C' => Value::Char((t.GetStaticCharField)(e, c, f)),
                b'S' => Value::Short((t.GetStaticShortField)(e, c, f)),
                b'I' => Value::Int((t.GetStaticIntField)(e, c, f)),
                b'J' => Value::Long((t.GetStaticLongField)(e, c, f)),
                b'F' => Value::Float((t.GetStaticFloatField)(e, c, f)),
                b'D' => Value::Double((t.GetStaticDoubleField)(e, c, f)),
                _ => Value::Object(Local::from_raw(self, (t.GetStaticObjectField)(e, c, f))),
            }
        };
        Ok(value)
    }

    /// `Class.name = value`, a static field (`SetStatic<Type>Field()`).
    pub(crate) fn set_static_field(
        self,
        class: Obj<'_>,
        name: &CStr,
        descriptor: &CStr,
        value: Arg<'_>,
    ) -> JniResult<'a, ()> {
        assert_eq!(
            value.shorty(),
            field_shorty(descriptor),
            "{value:?} isn't a {descriptor:?}"
        );
        let field = self.get_static_field_id(class, name, descriptor)?;
        let (t, e, c, f) = (self.table(), self.raw(), class.raw(), field.raw());
        // SAFETY: the value's type is the field's.
        unsafe {
            match value {
                Arg::Boolean(z) => (t.SetStaticBooleanField)(e, c, f, z as jboolean),
                Arg::Byte(b) => (t.SetStaticByteField)(e, c, f, b),
                Arg::Char(ch) => (t.SetStaticCharField)(e, c, f, ch),
                Arg::Short(s) => (t.SetStaticShortField)(e, c, f, s),
                Arg::Int(i) => (t.SetStaticIntField)(e, c, f, i),
                Arg::Long(j) => (t.SetStaticLongField)(e, c, f, j),
                Arg::Float(v) => (t.SetStaticFloatField)(e, c, f, v),
                Arg::Double(d) => (t.SetStaticDoubleField)(e, c, f, d),
                Arg::Object(l) => (t.SetStaticObjectField)(e, c, f, l.raw()),
            }
        }
        Ok(())
    }

    // Strings

    /// `NewString()` of the string's UTF-16.
    pub(crate) fn new_string(self, s: &str) -> JniResult<'a, Local<'a>> {
        let units: Vec<jchar> = s.encode_utf16().collect();
        // SAFETY: the units and their count.
        let raw =
            unsafe { (self.table().NewString)(self.raw(), units.as_ptr(), units.len() as jsize) };
        self.check()?;
        // SAFETY: a new local reference.
        Ok(unsafe { Local::from_raw(self, raw) }.expect("NewString() threw none"))
    }

    /// `NewString()` of UTF-16 units (Java's `String` of a `char[]`).
    pub(crate) fn new_string_utf16(self, units: &[jchar]) -> JniResult<'a, Local<'a>> {
        // SAFETY: the units and their count.
        let raw =
            unsafe { (self.table().NewString)(self.raw(), units.as_ptr(), units.len() as jsize) };
        self.check()?;
        // SAFETY: a new local reference.
        Ok(unsafe { Local::from_raw(self, raw) }.expect("NewString() threw none"))
    }

    /// A Java string's characters (`GetStringLength()` and
    /// `GetStringRegion()`), its UTF-16 units.
    pub(crate) fn get_string_utf16(self, string: Obj<'_>) -> JniResult<'a, Vec<jchar>> {
        // SAFETY: a valid string.
        let length = unsafe { (self.table().GetStringLength)(self.raw(), string.raw()) };
        self.check()?;
        let mut units = vec![0 as jchar; length.max(0) as usize];
        // SAFETY: room for `length` units.
        unsafe {
            (self.table().GetStringRegion)(self.raw(), string.raw(), 0, length, units.as_mut_ptr())
        };
        self.check()?;
        Ok(units)
    }

    /// A Java string's characters (its UTF-16, [`Env::get_string_utf16`]),
    /// as UTF-8 (an unpaired surrogate, which UTF-8 can't have, becomes
    /// U+FFFD).
    pub(crate) fn get_string(self, string: Obj<'_>) -> JniResult<'a, String> {
        Ok(String::from_utf16_lossy(&self.get_string_utf16(string)?))
    }

    /// `NewStringUTF()` of a Modified UTF-8 string (one without U+0000 or
    /// characters past U+FFFF is its UTF-8).
    pub(crate) fn new_string_utf(self, s: &CStr) -> JniResult<'a, Local<'a>> {
        // SAFETY: a C string.
        let raw = unsafe { (self.table().NewStringUTF)(self.raw(), s.as_ptr()) };
        self.check()?;
        // SAFETY: a new local reference.
        Ok(unsafe { Local::from_raw(self, raw) }.expect("NewStringUTF() threw none"))
    }

    /// `GetStringUTFChars()`: a Java string's Modified UTF-8, as upstream
    /// reads strings (released at once: `ReleaseStringUTFChars()`).
    pub(crate) fn get_string_utf_chars(self, string: Obj<'_>) -> JniResult<'a, CString> {
        // SAFETY: a valid string.
        let chars =
            unsafe { (self.table().GetStringUTFChars)(self.raw(), string.raw(), ptr::null_mut()) };
        self.check()?;
        // SAFETY: a C string the VM gave, released once.
        let copy = unsafe { CStr::from_ptr(chars) }.to_owned();
        // SAFETY: as above.
        unsafe { (self.table().ReleaseStringUTFChars)(self.raw(), string.raw(), chars) };
        Ok(copy)
    }

    // Arrays

    /// `GetArrayLength()`.
    pub(crate) fn get_array_length(self, array: Obj<'_>) -> jsize {
        // SAFETY: a valid array.
        unsafe { (self.table().GetArrayLength)(self.raw(), array.raw()) }
    }

    /// `NewObjectArray()`.
    pub(crate) fn new_object_array(
        self,
        length: jsize,
        class: Obj<'_>,
        initial: Obj<'_>,
    ) -> JniResult<'a, Local<'a>> {
        // SAFETY: a valid class and initial element.
        let raw = unsafe {
            (self.table().NewObjectArray)(self.raw(), length, class.raw(), initial.raw())
        };
        self.check()?;
        // SAFETY: a new local reference.
        Ok(unsafe { Local::from_raw(self, raw) }.expect("NewObjectArray() threw none"))
    }

    /// `GetObjectArrayElement()`.
    pub(crate) fn get_object_array_element(
        self,
        array: Obj<'_>,
        index: jsize,
    ) -> JniResult<'a, Option<Local<'a>>> {
        // SAFETY: a valid array.
        let raw = unsafe { (self.table().GetObjectArrayElement)(self.raw(), array.raw(), index) };
        self.check()?;
        // SAFETY: a new local reference.
        Ok(unsafe { Local::from_raw(self, raw) })
    }

    /// `SetObjectArrayElement()`.
    pub(crate) fn set_object_array_element(
        self,
        array: Obj<'_>,
        index: jsize,
        value: Obj<'_>,
    ) -> JniResult<'a, ()> {
        // SAFETY: valid references.
        unsafe {
            (self.table().SetObjectArrayElement)(self.raw(), array.raw(), index, value.raw())
        };
        self.check()
    }

    /// `GetIntArrayRegion()` of one element: `array[index]` of an `int[]`.
    pub(crate) fn get_int_array_element(self, array: Obj<'_>, index: jsize) -> JniResult<'a, jint> {
        let mut value: jint = 0;
        // SAFETY: a valid array, and room for one element.
        unsafe { (self.table().GetIntArrayRegion)(self.raw(), array.raw(), index, 1, &mut value) };
        self.check()?;
        Ok(value)
    }

    /// `GetIntArrayRegion()` of every element: an `int[]`'s values.
    pub(crate) fn get_int_array(self, array: Obj<'_>) -> JniResult<'a, Vec<jint>> {
        let mut values = vec![0; self.get_array_length(array).max(0) as usize];
        // SAFETY: a valid array, and room for its elements.
        unsafe {
            (self.table().GetIntArrayRegion)(
                self.raw(),
                array.raw(),
                0,
                values.len() as jsize,
                values.as_mut_ptr(),
            )
        };
        self.check()?;
        Ok(values)
    }

    /// `GetBooleanArrayRegion()` of every element: a `boolean[]`'s values.
    pub(crate) fn get_boolean_array(self, array: Obj<'_>) -> JniResult<'a, Vec<bool>> {
        let mut values: Vec<jboolean> = vec![0; self.get_array_length(array).max(0) as usize];
        // SAFETY: a valid array, and room for its elements.
        unsafe {
            (self.table().GetBooleanArrayRegion)(
                self.raw(),
                array.raw(),
                0,
                values.len() as jsize,
                values.as_mut_ptr(),
            )
        };
        self.check()?;
        Ok(values.into_iter().map(|b| b != JNI_FALSE).collect())
    }

    /// `GetFloatArrayRegion()`: `N` elements of a `float[]` from `start`
    /// (an `ArrayIndexOutOfBoundsException` where it has fewer).
    pub(crate) fn get_float_array_region<const N: usize>(
        self,
        array: Obj<'_>,
        start: jsize,
    ) -> JniResult<'a, [jfloat; N]> {
        let mut values = [0.0; N];
        // SAFETY: a valid array, and room for `N` elements.
        unsafe {
            (self.table().GetFloatArrayRegion)(
                self.raw(),
                array.raw(),
                start,
                N as jsize,
                values.as_mut_ptr(),
            )
        };
        self.check()?;
        Ok(values)
    }

    /// `NewByteArray()` and `SetByteArrayRegion()`: a Java `byte[]` of the
    /// bytes.
    pub(crate) fn new_byte_array(self, bytes: &[u8]) -> JniResult<'a, Local<'a>> {
        let length = bytes.len() as jsize;
        // SAFETY: a length.
        let raw = unsafe { (self.table().NewByteArray)(self.raw(), length) };
        self.check()?;
        // SAFETY: a new local reference.
        let array = unsafe { Local::from_raw(self, raw) }.expect("NewByteArray() threw none");
        // SAFETY: `length` bytes.
        unsafe {
            (self.table().SetByteArrayRegion)(self.raw(), raw, 0, length, bytes.as_ptr().cast())
        };
        self.check()?;
        Ok(array)
    }

    /// `GetByteArrayRegion()` of every element: a `byte[]`'s bytes.
    pub(crate) fn get_byte_array(self, array: Obj<'_>) -> JniResult<'a, Vec<u8>> {
        let mut bytes = vec![0u8; self.get_array_length(array).max(0) as usize];
        // SAFETY: a valid array, and room for its elements.
        unsafe {
            (self.table().GetByteArrayRegion)(
                self.raw(),
                array.raw(),
                0,
                bytes.len() as jsize,
                bytes.as_mut_ptr().cast(),
            )
        };
        self.check()?;
        Ok(bytes)
    }

    /// `NewIntArray()` and `SetIntArrayRegion()`: a Java `int[]` of the
    /// values.
    pub(crate) fn new_int_array(self, values: &[jint]) -> JniResult<'a, Local<'a>> {
        let length = values.len() as jsize;
        // SAFETY: a length.
        let raw = unsafe { (self.table().NewIntArray)(self.raw(), length) };
        self.check()?;
        // SAFETY: a new local reference.
        let array = unsafe { Local::from_raw(self, raw) }.expect("NewIntArray() threw none");
        // SAFETY: `length` values.
        unsafe { (self.table().SetIntArrayRegion)(self.raw(), raw, 0, length, values.as_ptr()) };
        self.check()?;
        Ok(array)
    }

    // Direct buffers

    /// `NewDirectByteBuffer()` over `len` bytes at `address`.
    ///
    /// # Safety
    ///
    /// The bytes stay valid (and, if Java writes them, writable) while the
    /// buffer is used.
    pub(crate) unsafe fn new_direct_byte_buffer(
        self,
        address: *mut c_void,
        len: jlong,
    ) -> JniResult<'a, Local<'a>> {
        // SAFETY: the caller's.
        let raw = unsafe { (self.table().NewDirectByteBuffer)(self.raw(), address, len) };
        self.check()?;
        // SAFETY: a new local reference.
        Ok(unsafe { Local::from_raw(self, raw) }.expect("NewDirectByteBuffer() threw none"))
    }

    /// `GetDirectBufferCapacity()` (-1 for another object).
    pub(crate) fn get_direct_buffer_capacity(self, buffer: Obj<'_>) -> jlong {
        // SAFETY: a valid reference.
        unsafe { (self.table().GetDirectBufferCapacity)(self.raw(), buffer.raw()) }
    }
}

// ---------------------------------------------------------------------------
// The app's classes

/// The app's class loader (`SDLActivity`'s), and global references to the
/// classes of `super::classes`, which `JNI_OnLoad()` caches (on the VM's
/// thread that loads the library) for the threads whose `FindClass()`
/// can't find them.
pub(crate) struct AppClasses {
    pub(crate) loader: Option<Global>,
    pub(crate) classes: Vec<(&'static str, Global)>,
}

static APP_CLASSES: OnceLock<AppClasses> = OnceLock::new();

/// Cache the app's classes (once: `JNI_OnLoad()`'s).
pub(crate) fn set_app_classes(classes: AppClasses) {
    let _ = APP_CLASSES.set(classes);
}

/// The app's class `name` among those of `super::classes`
/// (`org/libsdl/app/SDLActivity`), cached by `JNI_OnLoad()`.
pub(crate) fn app_class(name: &str) -> Option<&'static Global> {
    APP_CLASSES
        .get()?
        .classes
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, c)| c)
}

/// Any class of the app's (`com/example/Thing`), on any thread, through
/// the app's class loader: `loader.loadClass("com.example.Thing")`.
pub(crate) fn find_app_class<'a>(env: Env<'a>, name: &str) -> JniResult<'a, Option<Local<'a>>> {
    let Some(loader) = APP_CLASSES.get().and_then(|c| c.loader.as_ref()) else {
        return Ok(None);
    };
    let binary_name = env.new_string(&name.replace('/', "."))?;
    env.call_method(
        loader.obj(),
        c"loadClass",
        c"(Ljava/lang/String;)Ljava/lang/Class;",
        &[Arg::Object(binary_name.obj())],
    )
    .map(Value::object)
}
