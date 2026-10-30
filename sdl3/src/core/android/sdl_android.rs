// Rust translation of src/core/android/SDL_android.c from Simple
// DirectMedia Layer (part: the library's loading, the registration of the
// native methods, SDLActivity's natives that start the SDL thread, set the
// environment and pass the activity's lifecycle on, the lifecycle event
// queue, the activity mutex, the manifest's environment variables, the
// entry points of SDL_system.h, the storage paths, the package's name,
// toasts, permissions, the locale, URLs, the content resolver's files and
// the battery).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `JNI_OnLoad()`, which registers the native methods of the classes of
//! [`super::classes`] (upstream's `register_methods()` of its tables:
//! `SDLActivity_tab` so far) and caches them, with the app's class loader,
//! for the threads that aren't the VM's; `SDLActivity`'s natives that run
//! the app's `SDL_main` (`nativeSetupJNI()`, `nativeInitMainThread()`,
//! `nativeRunMain()`, ...), set its environment (`nativeSetenv()`,
//! `Android_JNI_GetManifestEnvironmentVariables()`) and pass the
//! activity's lifecycle to SDL's thread (`nativePause()`, `nativeResume()`,
//! `nativeLowMemory()`, `nativeSendQuit()`, `nativeQuit()`, ...), through
//! the lifecycle event queue (`Android_SendLifecycleEvent()`,
//! `Android_WaitLifecycleEvent()`), which the event pump reads
//! (`video::drivers::android::events`); the activity mutex; and
//! SDL_system.h's `SDL_GetAndroidJNIEnv()`, `SDL_GetAndroidActivity()`,
//! `SDL_GetAndroidSDKVersion()`, `SDL_IsAndroidTV()`, `SDL_IsChromebook()`,
//! `SDL_IsDeXMode()`, `SDL_SendAndroidBackButton()` and
//! `SDL_SendAndroidMessage()` (the `crate::android` module), with
//! `SDL_IsAndroidTablet()` and `SDL_GetAndroidDeviceFormFactor()` for
//! `SDL_GetDeviceFormFactor()`; and what the small backends use: the
//! storage paths (`SDL_GetAndroidInternalStoragePath()`, `...External...`,
//! `SDL_GetAndroidCachePath()`), the package's name, toasts
//! (`SDL_ShowAndroidToast()`), permissions (`SDL_RequestAndroidPermission()`
//! and `nativePermissionResult()`), the locale, URLs, the content
//! resolver's files (`content://`) and the battery. The assets are
//! [`super::assets`]. The rest of SDL_android.c (the video, audio and
//! controller natives, ...) comes with the backends that use it.
//!
//! This library is exported as libSDL3.so's is: a program that links it
//! has its `JNI_OnLoad()` (as an app with upstream's static library has),
//! which Android calls when the app's Java loads `libmain`.

use super::classes::{self, Class, Code, CLASSES, SDL_ACTIVITY};
use super::env::{self, Arg, Env, Global, Obj, Value};
use super::jni::*;
use super::ndk::LogPriority;
use crate::thread::{RawMutex, Semaphore};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

/// Main activity: `mActivityClass` (`SDLActivity`, a global reference that
/// is never deleted, as upstream's).
static ACTIVITY_CLASS: AtomicPtr<_jobject> = AtomicPtr::new(ptr::null_mut());

/// `mActivityClass`, once `nativeSetupJNI()` has run.
pub(crate) fn activity_class() -> Option<Obj<'static>> {
    let raw = ACTIVITY_CLASS.load(Ordering::Acquire);
    // SAFETY: a global reference, never deleted.
    (!raw.is_null()).then(|| unsafe { Obj::from_raw(raw) })
}

/// `bHasEnvironmentVariables`.
static HAS_ENVIRONMENT_VARIABLES: AtomicBool = AtomicBool::new(false);

/// `run_count`.
static RUN_COUNT: AtomicI32 = AtomicI32::new(0);
/// `allow_recreate_activity`.
static ALLOW_RECREATE_ACTIVITY: AtomicBool = AtomicBool::new(false);
/// `allow_recreate_activity_set`.
static ALLOW_RECREATE_ACTIVITY_SET: AtomicBool = AtomicBool::new(false);

fn log(priority: LogPriority, message: &str) {
    env::log(priority, message);
}

/// Register the native methods of `class` (`register_methods()`), found as
/// `FindClass()` finds it, else through `loader` (a class loader of the
/// classes' DEX file): the class, for the cache.
fn register_methods<'a>(
    env: Env<'a>,
    class: &Class,
    loader: Option<Obj<'_>>,
) -> Option<env::Local<'a>> {
    let name = CString::new(class.name).expect("a class name");
    let found = match env.find_class(&name) {
        Ok(found) => Some(found),
        Err(thrown) => {
            thrown.clear();
            loader.and_then(|loader| load_class(env, loader, class.name))
        }
    };
    let natives: Vec<(CString, CString, *mut c_void)> = class
        .methods
        .iter()
        .filter_map(|m| match &m.code {
            Code::Native(f) => Some((
                CString::new(m.name).expect("a method name"),
                CString::new(m.descriptor).expect("a descriptor"),
                f.0,
            )),
            _ => None,
        })
        .collect();
    let table: Vec<JNINativeMethod> = natives
        .iter()
        .map(|(name, signature, f)| JNINativeMethod {
            name: name.as_ptr(),
            signature: signature.as_ptr(),
            fnPtr: *f,
        })
        .collect();
    let registered = found.as_ref().is_some_and(|found| {
        table.is_empty()
            || match env.register_natives(found.obj(), &table) {
                Ok(status) => status >= 0,
                Err(thrown) => {
                    thrown.describe();
                    false
                }
            }
    });
    if !registered {
        log(
            LogPriority::Error,
            &format!("Failed to register methods of {}", class.name),
        );
        return None;
    }
    found
}

/// `loader.loadClass(name)` (the binary name, with dots).
fn load_class<'a>(env: Env<'a>, loader: Obj<'_>, name: &str) -> Option<env::Local<'a>> {
    let binary_name = env.new_string(&name.replace('/', ".")).ok()?;
    match env.call_method(
        loader,
        c"loadClass",
        c"(Ljava/lang/String;)Ljava/lang/Class;",
        &[Arg::Object(binary_name.obj())],
    ) {
        Ok(class) => class.object(),
        Err(thrown) => {
            thrown.clear();
            None
        }
    }
}

/// A class loader of the classes' DEX file ([`classes::SDL_CLASSES_DEX`],
/// the bytes of this library's), with the boot class loader for parent:
/// `new dalvik.system.InMemoryDexClassLoader(ByteBuffer.wrap(dex), null)`
/// (API 26 and later). Its classes are other classes than the app's of the
/// same names.
pub(crate) fn in_memory_class_loader(env: Env<'_>) -> env::JniResult<'_, env::Local<'_>> {
    let bytes = env.new_byte_array(&classes::SDL_CLASSES_DEX)?;
    let byte_buffer = env.find_class(c"java/nio/ByteBuffer")?;
    let buffer = env
        .call_static_method(
            byte_buffer.obj(),
            c"wrap",
            c"([B)Ljava/nio/ByteBuffer;",
            &[Arg::Object(bytes.obj())],
        )?
        .object()
        .expect("ByteBuffer.wrap() returns a buffer");
    let loader_class = env.find_class(c"dalvik/system/InMemoryDexClassLoader")?;
    env.new_object(
        loader_class.obj(),
        c"(Ljava/nio/ByteBuffer;Ljava/lang/ClassLoader;)V",
        &[Arg::Object(buffer.obj()), Arg::Object(Obj::NULL)],
    )
}

/// Library init. Translation of `JNI_OnLoad()`, which also caches the
/// classes it registers (global references) and their class loader, for
/// [`env::app_class`] and [`env::find_app_class`]. When the app has no
/// `SDLActivity` (a library loaded by an app's own activity, which
/// upstream doesn't support: its `FindClass()` fails), they are loaded
/// from this library's DEX file through an `InMemoryDexClassLoader` (API
/// 26 and later), and registered there. (That `SDLActivity` can't be
/// initialized, as its static initializer loads `libmain` through its
/// class loader, which the app's has loaded: its other classes serve.)
///
/// # Safety
///
/// Called by the Java VM, with itself.
#[no_mangle]
pub unsafe extern "system" fn JNI_OnLoad(vm: *mut JavaVM, _reserved: *mut c_void) -> jint {
    env::set_java_vm(vm);

    let Some(env) = NonNull::new(vm).and_then(env::vm_env) else {
        log(LogPriority::Error, "Failed to get JNI Env");
        return JNI_VERSION_1_4;
    };

    // (the class loader: SDLActivity's, the app's; else the DEX file's,
    // loaded from this library)
    let activity_name = CString::new(SDL_ACTIVITY).unwrap();
    let mut in_memory = None;
    let loader = match env.find_class(&activity_name) {
        Ok(activity) => env
            .call_method(
                activity.obj(),
                c"getClassLoader",
                c"()Ljava/lang/ClassLoader;",
                &[],
            )
            .ok()
            .and_then(Value::object),
        Err(thrown) => {
            thrown.clear();
            if sdk_int(env) >= 26 {
                match in_memory_class_loader(env) {
                    Ok(loader) => {
                        log(
                            LogPriority::Info,
                            &format!("No {SDL_ACTIVITY} in the app: its classes come from this library's DEX file"),
                        );
                        in_memory = Some(loader);
                        in_memory.as_ref().and_then(|l| env.new_local_ref(l.obj()))
                    }
                    Err(thrown) => {
                        thrown.describe();
                        None
                    }
                }
            } else {
                None
            }
        }
    };

    let mut cached = Vec::new();
    for class in CLASSES {
        let loader_for_lookup = in_memory.as_ref().map(|l| l.obj());
        if let Some(found) = register_methods(env, class, loader_for_lookup) {
            if let Some(global) = Global::new(env, found.obj()) {
                cached.push((class.name, global));
            }
        }
    }
    env::set_app_classes(env::AppClasses {
        loader: loader.and_then(|l| Global::new(env, l.obj())),
        classes: cached,
    });

    JNI_VERSION_1_4
}

/// `Build.VERSION.SDK_INT`, read once.
pub(crate) fn sdk_int(env: Env<'_>) -> jint {
    static SDK_INT: AtomicI32 = AtomicI32::new(0);
    let cached = SDK_INT.load(Ordering::Relaxed);
    if cached != 0 {
        return cached;
    }
    let value = env
        .find_class(c"android/os/Build$VERSION")
        .and_then(|version| env.get_static_field(version.obj(), c"SDK_INT", c"I"))
        .map(Value::int)
        .unwrap_or_else(|thrown| {
            thrown.clear();
            0
        });
    SDK_INT.store(value, Ordering::Relaxed);
    value
}

/// Translation of `checkJNIReady()`: SDL's main is ready once the
/// activity's JNI is set up. (Upstream also waits for the audio and
/// controller managers' `nativeSetupJNI()`, which come with those
/// classes.)
fn check_jni_ready() {
    if activity_class().is_none() {
        // We aren't fully initialized, let's just return.
        return;
    }

    crate::init::set_main_ready();
}

/// Get SDL version -- called before SDL_main() to verify JNI bindings.
/// Translation of `nativeGetVersion()`.
pub(crate) fn get_version(env: Env<'_>) -> Option<env::Local<'_>> {
    let v = crate::version::Version::CURRENT;
    let version = CString::new(format!("{}.{}.{}", v.major, v.minor, v.micro)).unwrap();
    env.new_string_utf(&version).ok()
}

pub(crate) extern "system" fn native_get_version(env: *mut JNIEnv, _cls: jclass) -> jstring {
    // SAFETY: a native method's environment.
    let env = unsafe { Env::from_raw(env) };
    get_version(env).map_or(ptr::null_mut(), env::Local::into_raw)
}

/// Activity initialization -- called before SDL_main() to initialize JNI
/// bindings. Translation of `nativeSetupJNI()`.
pub(crate) fn setup_jni(env: Env<'_>, cls: Obj<'_>) {
    log(LogPriority::Verbose, "nativeSetupJNI()");

    // Start with a clean slate
    // (SDL_ClearError(): errors are values here)

    /*
     * Create mThreadKey so we can keep track of the JNIEnv assigned to each thread
     * Refer to http://developer.android.com/guide/practices/design/jni.html for the rationale behind this
     */
    env::create_key_once();

    // Save JNIEnv of SDLActivity
    env::store_env(Some(env));

    if env::java_vm().is_none() {
        log(LogPriority::Error, "failed to found a JavaVM");
    }

    /* Use a mutex to prevent concurrency issues between Java Activity and Native thread code, when using 'Android_Window'.
     * (Eg. Java sending Touch events, while native code is destroying the main SDL_Window. )
     */
    // (Android_ActivityMutex: a static here, which can't fail to be made;
    // see lock_activity_mutex())

    // (Android_LifecycleMutex and Android_LifecycleEventSem: made anew, a
    // semaphore of 0; see Lifecycle)
    {
        let mut lifecycle = lifecycle();
        lifecycle.created = true;
        while LIFECYCLE_EVENT_SEM.try_wait() {}
    }

    if let Some(class) = Global::new(env, cls) {
        let raw = class.obj().raw();
        std::mem::forget(class);
        ACTIVITY_CLASS.store(raw, Ordering::Release);
    }

    // (the static methods' IDs: they are Rust functions here, super::java)

    check_jni_ready();
}

pub(crate) extern "system" fn native_setup_jni(env: *mut JNIEnv, cls: jclass) {
    // SAFETY: a native method's environment and class.
    let (env, cls) = unsafe { (Env::from_raw(env), Obj::from_raw(cls)) };
    setup_jni(env, cls);
}

/// The subsystems SDL is built with. Translation of
/// `nativeGetCompiledSubsystems()` (no `SDL_*_DISABLED` here).
pub(crate) fn get_compiled_subsystems() -> jint {
    use crate::init::InitFlags;
    let subsystems = InitFlags::AUDIO
        | InitFlags::VIDEO
        | InitFlags::JOYSTICK
        | InitFlags::HAPTIC
        | InitFlags::GAMEPAD
        | InitFlags::SENSOR
        | InitFlags::CAMERA;
    subsystems.0 as jint
}

pub(crate) extern "system" fn native_get_compiled_subsystems(
    _env: *mut JNIEnv,
    _cls: jclass,
) -> jint {
    get_compiled_subsystems()
}

/// Whether SDL is built with HIDAPI (it is). Translation of
/// `nativeIsHIDAPIEnabled()`.
pub(crate) fn is_hidapi_enabled() -> bool {
    true
}

pub(crate) extern "system" fn native_is_hidapi_enabled(
    _env: *mut JNIEnv,
    _cls: jclass,
) -> jboolean {
    is_hidapi_enabled() as jboolean
}

/// Translation of `nativeCheckSDLThreadCounter()`: the count of the
/// SDL threads started so far, then one more.
pub(crate) fn check_sdl_thread_counter() -> jint {
    RUN_COUNT.fetch_add(1, Ordering::AcqRel)
}

pub(crate) extern "system" fn native_check_sdl_thread_counter(
    _env: *mut JNIEnv,
    _cls: jclass,
) -> jint {
    check_sdl_thread_counter()
}

/// Translation of `nativeAllowRecreateActivity()`.
pub(crate) fn allow_recreate_activity() -> bool {
    ALLOW_RECREATE_ACTIVITY.load(Ordering::Acquire)
}

pub(crate) extern "system" fn native_allow_recreate_activity(
    _env: *mut JNIEnv,
    _cls: jclass,
) -> jboolean {
    allow_recreate_activity() as jboolean
}

/// Translation of `Android_SetAllowRecreateActivity()`.
pub(crate) fn set_allow_recreate_activity(enabled: bool) {
    ALLOW_RECREATE_ACTIVITY.store(enabled, Ordering::Release);
    ALLOW_RECREATE_ACTIVITY_SET.store(true, Ordering::Release);
}

/// Translation of `nativeInitMainThread()`.
pub(crate) fn init_main_thread(env: Env<'_>) {
    log(
        LogPriority::Verbose,
        &format!(
            "nativeInitSDLThread() {} time",
            RUN_COUNT.load(Ordering::Acquire)
        ),
    );
    RUN_COUNT.fetch_add(1, Ordering::AcqRel);

    // Save JNIEnv of SDLThread
    env::store_env(Some(env));
}

pub(crate) extern "system" fn native_init_main_thread(env: *mut JNIEnv, _cls: jclass) {
    // SAFETY: a native method's environment.
    init_main_thread(unsafe { Env::from_raw(env) });
}

/// Translation of `nativeCleanupMainThread()`.
pub(crate) fn cleanup_main_thread() {
    /* This is a Java thread, it doesn't need to be Detached from the JVM.
     * Set to mThreadKey value to NULL not to call pthread_create destructor 'Android_JNI_ThreadDestroyed' */
    env::store_env(None);
}

pub(crate) extern "system" fn native_cleanup_main_thread(_env: *mut JNIEnv, _cls: jclass) {
    cleanup_main_thread();
}

/// `SDL_main_func`.
type MainFunction = unsafe extern "C" fn(argc: c_int, argv: *mut *mut c_char) -> c_int;

/// Start up the SDL app: `dlopen()` the library (by its path, else its
/// name), `dlsym()` the function, and call it through SDL's
/// `SDL_RunApp()`, with the arguments `app_process` and those of `array`
/// (Java strings, as UTF-8: Note: upstream's `GetStringUTFChars()` gives
/// them in Modified UTF-8). Translation of `nativeRunMain()`.
pub(crate) fn run_main(env: Env<'_>, library: Obj<'_>, function: Obj<'_>, array: Obj<'_>) -> jint {
    let mut status = -1;

    let library_file = env.get_string(library).unwrap_or_else(|thrown| {
        thrown.clear();
        String::new()
    });
    let path = CString::new(library_file.as_str()).unwrap_or_default();
    // SAFETY: a C string.
    let mut library_handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_GLOBAL) };

    if library_handle.is_null() {
        /* When deploying android app bundle format uncompressed native libs may not extract from apk to filesystem.
        In this case we should use lib name without path. https://bugzilla.libsdl.org/show_bug.cgi?id=4739 */
        if let Some((_, library_name)) = library_file.rsplit_once('/') {
            if !library_name.is_empty() {
                let name = CString::new(library_name).unwrap_or_default();
                // SAFETY: a C string.
                library_handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_GLOBAL) };
            }
        }
    }

    if !library_handle.is_null() {
        let function_name = env.get_string(function).unwrap_or_else(|thrown| {
            thrown.clear();
            String::new()
        });
        let symbol = CString::new(function_name.as_str()).unwrap_or_default();
        // SAFETY: a library handle and a C string.
        let sdl_main = unsafe { libc::dlsym(library_handle, symbol.as_ptr()) };
        if !sdl_main.is_null() {
            // SAFETY: the app's SDL_main (int SDL_main(int, char *[])).
            let sdl_main: MainFunction =
                unsafe { std::mem::transmute::<*mut c_void, MainFunction>(sdl_main) };
            // Use the name "app_process" for argv[0] so PHYSFS_platformCalcBaseDir() works.
            //   https://github.com/love2d/love-android/issues/24
            // (note that PhysicsFS hasn't used argv on Android in a long time, but we'll keep this for compat at least for SDL3's lifetime.  --ryan.)
            let argv0 = "app_process";
            let len = if array.is_null() {
                0
            } else {
                env.get_array_length(array)
            }; // argv elements, not counting argv[0].

            let mut args = vec![argv0.to_string()];
            for i in 0..len {
                let utf = match env.get_object_array_element(array, i) {
                    Ok(Some(string)) => env.get_string(string.obj()).unwrap_or_else(|thrown| {
                        thrown.clear();
                        String::new()
                    }),
                    Ok(None) => String::new(),
                    Err(thrown) => {
                        thrown.clear();
                        String::new()
                    }
                };
                args.push(utf);
            }

            // Run the application.
            status = crate::app::run_app_with_args(args, |args| {
                let strings: Vec<CString> = args
                    .iter()
                    .map(|a| CString::new(a.split('\0').next().unwrap_or("")).unwrap_or_default())
                    .collect();
                let mut argv: Vec<*mut c_char> =
                    strings.iter().map(|s| s.as_ptr() as *mut c_char).collect();
                argv.push(ptr::null_mut());
                // SAFETY: the app's main function, with its arguments (NULL-terminated).
                unsafe { sdl_main(strings.len() as c_int, argv.as_mut_ptr()) }
            });
        } else {
            log(
                LogPriority::Error,
                &format!("nativeRunMain(): Couldn't find function {function_name} in library {library_file}"),
            );
        }

        // SAFETY: the handle dlopen() gave.
        unsafe { libc::dlclose(library_handle) };
    } else {
        log(
            LogPriority::Error,
            &format!("nativeRunMain(): Couldn't load library {library_file}"),
        );
    }

    // Do not issue an exit or the whole application will terminate instead of just the SDL thread
    // exit(status);

    status
}

pub(crate) extern "system" fn native_run_main(
    env: *mut JNIEnv,
    _cls: jclass,
    library: jstring,
    function: jstring,
    array: jobject,
) -> jint {
    // SAFETY: a native method's environment and arguments.
    let (env, library, function, array) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(library),
            Obj::from_raw(function),
            Obj::from_raw(array),
        )
    };
    run_main(env, library, function, array)
}

// ---------------------------------------------------------------------------
// The lifecycle event queue, and the natives that fill it

/// What the activity tells SDL's thread. Translation of
/// `SDL_AndroidLifecycleEvent`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LifecycleEvent {
    Wake,
    Pause,
    Resume,
    LowMemory,
    Destroy,
}

/// `SDL_NUM_ANDROID_LIFECYCLE_EVENTS`: the room upstream's queue has.
const NUM_LIFECYCLE_EVENTS: usize = 5;

/// The queue (`Android_LifecycleEvents` and `Android_NumLifecycleEvents`),
/// guarded by `Android_LifecycleMutex`, and whether that mutex and the
/// semaphore exist: upstream's `nativeSetupJNI()` makes them,
/// `nativeQuit()` destroys them.
struct Lifecycle {
    created: bool,
    events: Vec<LifecycleEvent>,
}

static LIFECYCLE: Mutex<Lifecycle> = Mutex::new(Lifecycle {
    created: false,
    events: Vec::new(),
});

/// `Android_LifecycleEventSem` (a static, emptied where upstream makes a
/// new one).
static LIFECYCLE_EVENT_SEM: Semaphore = Semaphore::new(0);

fn lifecycle() -> MutexGuard<'static, Lifecycle> {
    LIFECYCLE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `FindLifecycleEvent()`.
fn find_lifecycle_event(events: &[LifecycleEvent], event: LifecycleEvent) -> Option<usize> {
    events.iter().position(|&e| e == event)
}

/// Translation of `RemoveLifecycleEvent()`.
fn remove_lifecycle_event(events: &mut Vec<LifecycleEvent>, index: usize) {
    events.remove(index);
}

/// Drop file (the activity's "Open with"). Translation of
/// `onNativeDropFile()`.
pub(crate) fn on_drop_file(filename: &str) {
    crate::events::window::send_drop_file(None, None, filename);
    crate::events::window::send_drop_complete(None);
}

pub(crate) extern "system" fn on_native_drop_file(
    env: *mut JNIEnv,
    _cls: jclass,
    filename: jstring,
) {
    // SAFETY: a native method's environment and argument.
    let (env, filename) = unsafe { (Env::from_raw(env), Obj::from_raw(filename)) };
    match env.get_string(filename) {
        Ok(path) => on_drop_file(&path),
        Err(thrown) => drop(thrown),
    }
}

/// Low memory. Translation of `nativeLowMemory()`.
pub(crate) fn low_memory() {
    send_lifecycle_event(LifecycleEvent::LowMemory);
}

pub(crate) extern "system" fn native_low_memory(_env: *mut JNIEnv, _cls: jclass) {
    low_memory();
}

/// Locale: requires `android:configChanges="layoutDirection|locale"` in
/// AndroidManifest.xml. Translation of `onNativeLocaleChanged()`.
pub(crate) fn on_locale_changed() {
    crate::events::queue::send_app_event(crate::events::EventType::LOCALE_CHANGED);
}

pub(crate) extern "system" fn on_native_locale_changed(_env: *mut JNIEnv, _cls: jclass) {
    on_locale_changed();
}

/// Dark mode. Translation of `onNativeDarkModeChanged()`.
pub(crate) fn on_dark_mode_changed(enabled: bool) {
    crate::video::drivers::android::set_dark_mode(enabled);
}

pub(crate) extern "system" fn on_native_dark_mode_changed(
    _env: *mut JNIEnv,
    _cls: jclass,
    enabled: jboolean,
) {
    on_dark_mode_changed(enabled != 0);
}

/// Send Quit event to "SDLThread" thread. Translation of
/// `nativeSendQuit()`.
pub(crate) fn send_quit() {
    send_lifecycle_event(LifecycleEvent::Destroy);
}

pub(crate) extern "system" fn native_send_quit(_env: *mut JNIEnv, _cls: jclass) {
    send_quit();
}

/// Activity ends. Translation of `nativeQuit()`. Note: the activity mutex
/// stays (upstream destroys it, after which locking it does nothing; see
/// [`lock_activity_mutex`]), and the message is the one without an error
/// (errors are values here: there's no `SDL_GetError()`).
pub(crate) fn quit() {
    {
        let mut lifecycle = lifecycle();
        lifecycle.created = false;
        // Android_NumLifecycleEvents = 0;
        lifecycle.events.clear();
        while LIFECYCLE_EVENT_SEM.try_wait() {}
    }

    super::assets::destroy_asset_manager();

    log(LogPriority::Verbose, "SDLActivity thread ends");
}

pub(crate) extern "system" fn native_quit(_env: *mut JNIEnv, _cls: jclass) {
    quit();
}

/// Pause. Translation of `nativePause()`.
pub(crate) fn pause() {
    log(LogPriority::Verbose, "nativePause()");

    send_lifecycle_event(LifecycleEvent::Pause);
}

pub(crate) extern "system" fn native_pause(_env: *mut JNIEnv, _cls: jclass) {
    pause();
}

/// Resume. Translation of `nativeResume()`.
pub(crate) fn resume() {
    log(LogPriority::Verbose, "nativeResume()");

    send_lifecycle_event(LifecycleEvent::Resume);
}

pub(crate) extern "system" fn native_resume(_env: *mut JNIEnv, _cls: jclass) {
    resume();
}

/// Translation of `nativeFocusChanged()`.
pub(crate) fn focus_changed(has_focus: bool) {
    lock_activity_mutex();

    if let Some(window) = crate::video::drivers::android::window() {
        log(LogPriority::Verbose, "nativeFocusChanged()");
        let event = if has_focus {
            crate::events::EventType::WINDOW_FOCUS_GAINED
        } else {
            crate::events::EventType::WINDOW_FOCUS_LOST
        };
        crate::events::window::send_window_event(window, event, 0, 0);
    }

    unlock_activity_mutex();
}

pub(crate) extern "system" fn native_focus_changed(
    _env: *mut JNIEnv,
    _cls: jclass,
    has_focus: jboolean,
) {
    focus_changed(has_focus != 0);
}

/// Keydown. Translation of `onNativeKeyDown()`.
pub(crate) fn on_key_down(keycode: jint) {
    lock_activity_mutex();

    if crate::video::drivers::android::window().is_some() {
        crate::video::drivers::android::on_key_down(keycode);
    }

    unlock_activity_mutex();
}

pub(crate) extern "system" fn on_native_key_down(_env: *mut JNIEnv, _cls: jclass, keycode: jint) {
    on_key_down(keycode);
}

/// Keyup. Translation of `onNativeKeyUp()`.
pub(crate) fn on_key_up(keycode: jint) {
    lock_activity_mutex();

    if crate::video::drivers::android::window().is_some() {
        crate::video::drivers::android::on_key_up(keycode);
    }

    unlock_activity_mutex();
}

pub(crate) extern "system" fn on_native_key_up(_env: *mut JNIEnv, _cls: jclass, keycode: jint) {
    on_key_up(keycode);
}

// ---------------------------------------------------------------------------
// Touch, pinch, mouse and pen: the natives, for the video driver

/// A touch device. Translation of `nativeAddTouch()`.
pub(crate) fn add_touch(touch_id: jint, name: &str) {
    crate::events::touch::add_touch(
        crate::video::drivers::android::touch::convert_java_touch_id(touch_id),
        crate::events::touch::TouchDeviceType::Direct,
        name,
    );
}

pub(crate) extern "system" fn native_add_touch(
    env: *mut JNIEnv,
    _cls: jclass,
    touch_id: jint,
    name: jstring,
) {
    // SAFETY: a native method's environment and arguments.
    let (env, name) = unsafe { (Env::from_raw(env), Obj::from_raw(name)) };
    let Ok(utfname) = env.get_string(name) else {
        // (pending, for the caller)
        return;
    };

    add_touch(touch_id, &utfname);
}

/// Touch. Translation of `onNativeTouch()`.
pub(crate) fn on_touch(
    touch_device_id_in: jint,
    pointer_finger_id_in: jint,
    action: jint,
    x: jfloat,
    y: jfloat,
    p: jfloat,
) {
    lock_activity_mutex();

    crate::video::drivers::android::touch::on_touch(
        crate::video::drivers::android::window(),
        touch_device_id_in,
        pointer_finger_id_in,
        action,
        x,
        y,
        p,
    );

    unlock_activity_mutex();
}

#[allow(clippy::too_many_arguments)] // (the Java method's)
pub(crate) extern "system" fn on_native_touch(
    _env: *mut JNIEnv,
    _cls: jclass,
    touch_device_id_in: jint,
    pointer_finger_id_in: jint,
    action: jint,
    x: jfloat,
    y: jfloat,
    p: jfloat,
) {
    on_touch(touch_device_id_in, pointer_finger_id_in, action, x, y, p);
}

/// Pinch. Translation of `onNativePinchStart()`.
pub(crate) fn on_pinch_start(span_x: jfloat, span_y: jfloat, focus_x: jfloat, focus_y: jfloat) {
    lock_activity_mutex();

    if let Some(window) = crate::video::drivers::android::window() {
        crate::events::touch::send_pinch(
            crate::events::EventType::PINCH_BEGIN,
            Duration::ZERO,
            window,
            0.0,
            span_x,
            span_y,
            focus_x,
            focus_y,
        );
    }

    unlock_activity_mutex();
}

pub(crate) extern "system" fn on_native_pinch_start(
    _env: *mut JNIEnv,
    _cls: jclass,
    span_x: jfloat,
    span_y: jfloat,
    focus_x: jfloat,
    focus_y: jfloat,
) {
    on_pinch_start(span_x, span_y, focus_x, focus_y);
}

/// Translation of `onNativePinchUpdate()`.
pub(crate) fn on_pinch_update(
    scale: jfloat,
    span_x: jfloat,
    span_y: jfloat,
    focus_x: jfloat,
    focus_y: jfloat,
) {
    lock_activity_mutex();

    if let Some(window) = crate::video::drivers::android::window() {
        crate::events::touch::send_pinch(
            crate::events::EventType::PINCH_UPDATE,
            Duration::ZERO,
            window,
            scale,
            span_x,
            span_y,
            focus_x,
            focus_y,
        );
    }

    unlock_activity_mutex();
}

pub(crate) extern "system" fn on_native_pinch_update(
    _env: *mut JNIEnv,
    _cls: jclass,
    scale: jfloat,
    span_x: jfloat,
    span_y: jfloat,
    focus_x: jfloat,
    focus_y: jfloat,
) {
    on_pinch_update(scale, span_x, span_y, focus_x, focus_y);
}

/// Translation of `onNativePinchEnd()`.
pub(crate) fn on_pinch_end() {
    lock_activity_mutex();

    if let Some(window) = crate::video::drivers::android::window() {
        crate::events::touch::send_pinch(
            crate::events::EventType::PINCH_END,
            Duration::ZERO,
            window,
            0.0,
            -1.0,
            -1.0,
            -1.0,
            -1.0,
        );
    }

    unlock_activity_mutex();
}

pub(crate) extern "system" fn on_native_pinch_end(_env: *mut JNIEnv, _cls: jclass) {
    on_pinch_end();
}

/// Mouse. Translation of `onNativeMouse()`.
pub(crate) fn on_mouse(button: jint, action: jint, x: jfloat, y: jfloat, relative: bool) {
    lock_activity_mutex();

    crate::video::drivers::android::mouse::on_mouse(
        crate::video::drivers::android::window(),
        button,
        action,
        x,
        y,
        relative,
    );

    unlock_activity_mutex();
}

pub(crate) extern "system" fn on_native_mouse(
    _env: *mut JNIEnv,
    _cls: jclass,
    button: jint,
    action: jint,
    x: jfloat,
    y: jfloat,
    relative: jboolean,
) {
    on_mouse(button, action, x, y, relative != 0);
}

/// Pen. Translation of `onNativePen()`.
pub(crate) fn on_pen(
    pen_id_in: jint,
    device_type: jint,
    button: jint,
    action: jint,
    x: jfloat,
    y: jfloat,
    p: jfloat,
) {
    lock_activity_mutex();

    crate::video::drivers::android::pen::on_pen(
        crate::video::drivers::android::window(),
        pen_id_in,
        device_type,
        button,
        action,
        x,
        y,
        p,
    );

    unlock_activity_mutex();
}

#[allow(clippy::too_many_arguments)] // (the Java method's)
pub(crate) extern "system" fn on_native_pen(
    _env: *mut JNIEnv,
    _cls: jclass,
    pen_id_in: jint,
    device_type: jint,
    button: jint,
    action: jint,
    x: jfloat,
    y: jfloat,
    p: jfloat,
) {
    on_pen(pen_id_in, device_type, button, action, x, y, p);
}

// ---------------------------------------------------------------------------
// The activity's surface: its natives, for the video driver

/// `displayNaturalOrientation` (an `SDL_DisplayOrientation`).
static DISPLAY_NATURAL_ORIENTATION: AtomicI32 = AtomicI32::new(0);
/// `displayCurrentOrientation`.
static DISPLAY_CURRENT_ORIENTATION: AtomicI32 = AtomicI32::new(0);

/// Set screen resolution. Translation of `nativeSetScreenResolution()`.
pub(crate) fn set_screen_resolution(
    surface_width: jint,
    surface_height: jint,
    device_width: jint,
    device_height: jint,
    density: jfloat,
    rate: jfloat,
) {
    lock_activity_mutex();

    crate::video::drivers::android::set_screen_resolution(
        surface_width,
        surface_height,
        device_width,
        device_height,
        density,
        rate,
    );

    unlock_activity_mutex();
}

#[allow(clippy::too_many_arguments)] // (the Java method's)
pub(crate) extern "system" fn native_set_screen_resolution(
    _env: *mut JNIEnv,
    _cls: jclass,
    surface_width: jint,
    surface_height: jint,
    device_width: jint,
    device_height: jint,
    density: jfloat,
    rate: jfloat,
) {
    set_screen_resolution(
        surface_width,
        surface_height,
        device_width,
        device_height,
        density,
        rate,
    );
}

/// Resize. Translation of `onNativeResize()`.
pub(crate) fn on_resize() {
    lock_activity_mutex();

    if let Some(window) = crate::video::drivers::android::window() {
        crate::video::drivers::android::send_resize(Some(window));
    }

    unlock_activity_mutex();
}

pub(crate) extern "system" fn on_native_resize(_env: *mut JNIEnv, _cls: jclass) {
    on_resize();
}

/// Translation of `nativeSetNaturalOrientation()`.
pub(crate) fn set_natural_orientation(orientation: jint) {
    DISPLAY_NATURAL_ORIENTATION.store(orientation, Ordering::Release);
}

pub(crate) extern "system" fn native_set_natural_orientation(
    _env: *mut JNIEnv,
    _cls: jclass,
    orientation: jint,
) {
    set_natural_orientation(orientation);
}

/// The display's rotation (0, 90, 180 or 270 degrees from its natural
/// orientation) as its orientation, to the video driver. Translation of
/// `onNativeRotationChanged()`.
pub(crate) fn on_rotation_changed(rotation: jint) {
    use crate::video::DisplayOrientation;

    lock_activity_mutex();

    let mut rotation = rotation;
    if DISPLAY_NATURAL_ORIENTATION.load(Ordering::Acquire) == DisplayOrientation::Landscape as i32 {
        rotation += 90;
    }

    let orientation = match rotation % 360 {
        0 => DisplayOrientation::Portrait,
        90 => DisplayOrientation::Landscape,
        180 => DisplayOrientation::PortraitFlipped,
        270 => DisplayOrientation::LandscapeFlipped,
        _ => DisplayOrientation::Unknown,
    };
    DISPLAY_CURRENT_ORIENTATION.store(orientation as i32, Ordering::Release);

    crate::video::drivers::android::set_orientation(orientation);

    unlock_activity_mutex();
}

pub(crate) extern "system" fn on_native_rotation_changed(
    _env: *mut JNIEnv,
    _cls: jclass,
    rotation: jint,
) {
    on_rotation_changed(rotation);
}

/// Translation of `onNativeInsetsChanged()`.
pub(crate) fn on_insets_changed(left: jint, right: jint, top: jint, bottom: jint) {
    lock_activity_mutex();

    crate::video::drivers::android::set_window_safe_area_insets(left, right, top, bottom);

    unlock_activity_mutex();
}

pub(crate) extern "system" fn on_native_insets_changed(
    _env: *mut JNIEnv,
    _cls: jclass,
    left: jint,
    right: jint,
    top: jint,
    bottom: jint,
) {
    on_insets_changed(left, right, top, bottom);
}

/// Called from `surfaceCreated()`: the window's native window, the
/// surface's. Translation of `onNativeSurfaceCreated()`.
pub(crate) fn on_surface_created() {
    use crate::video::drivers::android::window as android_window;

    lock_activity_mutex();

    if let Some(window) = android_window::android_window() {
        let native_window = get_native_window();
        android_window::with_window_data(|_, data| data.native_window = native_window);
        android_window::set_pointer_property(
            window,
            crate::video::window::PROP_WINDOW_ANDROID_WINDOW_POINTER,
            native_window.cast(),
        );
        if native_window.is_null() {
            // (SDL_SetError("Could not fetch native window from UI
            // thread"): the activity thread's error, which nothing reads;
            // errors are values here)
        }
    }

    unlock_activity_mutex();
}

pub(crate) extern "system" fn on_native_surface_created(_env: *mut JNIEnv, _cls: jclass) {
    on_surface_created();
}

/// Called from `surfaceChanged()`: if the surface has been previously
/// destroyed by `onNativeSurfaceDestroyed()`, the OpenGL window's EGL
/// surface made again. Translation of `onNativeSurfaceChanged()`.
pub(crate) fn on_surface_changed() {
    use crate::video::drivers::android::window as android_window;
    use crate::video::egl::EGL_NO_SURFACE;

    lock_activity_mutex();

    if let Some(window) = android_window::android_window() {
        let opengl = crate::video::core::with_window(window, |w| {
            w.flags()
                .contains(crate::events::window::WindowFlags::OPENGL)
        })
        .unwrap_or(false);
        if opengl {
            let (egl_surface, native_window) =
                android_window::with_window_data(|_, data| (data.egl_surface, data.native_window))
                    .unwrap_or((EGL_NO_SURFACE, ptr::null_mut()));

            // If the surface has been previously destroyed by onNativeSurfaceDestroyed, recreate it here
            if egl_surface == EGL_NO_SURFACE {
                let egl_surface =
                    crate::video::egl::create_surface(Some(window), native_window.cast())
                        .unwrap_or(EGL_NO_SURFACE);
                android_window::with_window_data(|_, data| data.egl_surface = egl_surface);
                android_window::set_pointer_property(
                    window,
                    crate::video::window::PROP_WINDOW_ANDROID_SURFACE_POINTER,
                    egl_surface,
                );
            }

            // GL Context handling is done in the event loop because this function is run from the Java thread
        }

        // (Android_RestoreScreenKeyboard(): the screen keyboard comes with
        // the keyboard's translation)
    }

    unlock_activity_mutex();
}

pub(crate) extern "system" fn on_native_surface_changed(_env: *mut JNIEnv, _cls: jclass) {
    on_surface_changed();
}

/// Called from `surfaceDestroyed()`: once SDL's thread has released the
/// OpenGL window's context (the pause's backup: waited for up to 50 times
/// 10 ms), its EGL surface and native window freed. Translation of
/// `onNativeSurfaceDestroyed()`.
pub(crate) fn on_surface_destroyed() {
    use crate::video::drivers::android::window as android_window;
    use crate::video::egl::EGL_NO_SURFACE;

    let mut nb_attempt = 50;

    loop {
        lock_activity_mutex();

        let Some(window) = android_window::android_window() else {
            unlock_activity_mutex();
            return;
        };

        // Wait for Main thread being paused and context un-activated to release 'egl_surface'
        let opengl = crate::video::core::with_window(window, |w| {
            w.flags()
                .contains(crate::events::window::WindowFlags::OPENGL)
        })
        .unwrap_or(false);
        let backup_done =
            android_window::with_window_data(|_, data| data.backup_done).unwrap_or(true);
        if opengl && !backup_done {
            nb_attempt -= 1;
            if nb_attempt == 0 {
                // (SDL_SetError("Try to release egl_surface with context
                // probably still active"): the activity thread's error,
                // which nothing reads; errors are values here)
            } else {
                unlock_activity_mutex();
                crate::timer::delay(Duration::from_millis(10));
                continue;
            }
        }

        let (egl_surface, native_window) = android_window::with_window_data(|_, data| {
            let surfaces = (data.egl_surface, data.native_window);
            data.egl_surface = EGL_NO_SURFACE;
            data.native_window = ptr::null_mut();
            surfaces
        })
        .unwrap_or((EGL_NO_SURFACE, ptr::null_mut()));

        if egl_surface != EGL_NO_SURFACE {
            crate::video::egl::destroy_surface(egl_surface);
        }

        if !native_window.is_null() {
            android_window::set_pointer_property(
                window,
                crate::video::window::PROP_WINDOW_ANDROID_WINDOW_POINTER,
                ptr::null_mut(),
            );
            // SAFETY: the window's reference.
            unsafe { super::ndk::ANativeWindow_release(native_window) };
        }

        // GL Context handling is done in the event loop because this function is run from the Java thread

        unlock_activity_mutex();
        return;
    }
}

pub(crate) extern "system" fn on_native_surface_destroyed(_env: *mut JNIEnv, _cls: jclass) {
    on_surface_destroyed();
}

/// Translation of `onNativeScreenKeyboardShown()`.
pub(crate) fn on_screen_keyboard_shown() {
    crate::video::textinput::send_screen_keyboard_shown();
}

pub(crate) extern "system" fn on_native_screen_keyboard_shown(_env: *mut JNIEnv, _cls: jclass) {
    on_screen_keyboard_shown();
}

/// Translation of `onNativeScreenKeyboardHidden()`.
pub(crate) fn on_screen_keyboard_hidden() {
    crate::video::textinput::send_screen_keyboard_hidden();
}

pub(crate) extern "system" fn on_native_screen_keyboard_hidden(_env: *mut JNIEnv, _cls: jclass) {
    on_screen_keyboard_hidden();
}

/// A hint's value. Translation of `nativeGetHint()`.
pub(crate) fn get_hint(name: &str) -> Option<String> {
    crate::hints::get(name)
}

pub(crate) extern "system" fn native_get_hint(
    env: *mut JNIEnv,
    _cls: jclass,
    name: jstring,
) -> jstring {
    // SAFETY: a native method's environment and argument.
    let (env, name) = unsafe { (Env::from_raw(env), Obj::from_raw(name)) };
    let Ok(utfname) = env.get_string(name) else {
        return ptr::null_mut();
    };
    // (NewStringUTF(NULL) is null)
    match get_hint(&utfname) {
        Some(hint) => env
            .new_string(&hint)
            .map_or(ptr::null_mut(), env::Local::into_raw),
        None => ptr::null_mut(),
    }
}

/// A hint's value as a boolean. Translation of `nativeGetHintBoolean()`.
pub(crate) fn get_hint_boolean(name: &str, default_value: bool) -> bool {
    crate::hints::get_bool(name, default_value)
}

pub(crate) extern "system" fn native_get_hint_boolean(
    env: *mut JNIEnv,
    _cls: jclass,
    name: jstring,
    default_value: jboolean,
) -> jboolean {
    // SAFETY: a native method's environment and argument.
    let (env, name) = unsafe { (Env::from_raw(env), Obj::from_raw(name)) };
    let Ok(utfname) = env.get_string(name) else {
        return default_value;
    };
    get_hint_boolean(&utfname, default_value != 0) as jboolean
}

/// Queue a lifecycle event for SDL's thread, merged with those queued:
/// one wake and one low memory at most, a pause and a resume cancel each
/// other, and a destroy replaces them all. Translation of
/// `Android_SendLifecycleEvent()`. (Upstream's queue has room for
/// [`NUM_LIFECYCLE_EVENTS`], which it asserts; this one grows.)
pub(crate) fn send_lifecycle_event(event: LifecycleEvent) {
    let mut lifecycle = lifecycle();
    let created = lifecycle.created;
    let events = &mut lifecycle.events;
    let mut add_event = true;

    match event {
        LifecycleEvent::Wake => {
            // We don't need more than one wake queued
            if find_lifecycle_event(events, LifecycleEvent::Wake).is_some() {
                add_event = false;
            }
        }
        LifecycleEvent::Pause => {
            // If we have a resume queued, just stay in the paused state
            if let Some(index) = find_lifecycle_event(events, LifecycleEvent::Resume) {
                remove_lifecycle_event(events, index);
                add_event = false;
            }
        }
        LifecycleEvent::Resume => {
            // If we have a pause queued, just stay in the resumed state
            if let Some(index) = find_lifecycle_event(events, LifecycleEvent::Pause) {
                remove_lifecycle_event(events, index);
                add_event = false;
            }
        }
        LifecycleEvent::LowMemory => {
            // We don't need more than one low memory event queued
            if find_lifecycle_event(events, LifecycleEvent::LowMemory).is_some() {
                add_event = false;
            }
        }
        LifecycleEvent::Destroy => {
            // Remove all other events, we're done!
            while !events.is_empty() {
                remove_lifecycle_event(events, 0);
            }
        }
    }

    if add_event {
        debug_assert!(events.len() < NUM_LIFECYCLE_EVENTS);
        events.push(event);
        // (SDL_SignalSemaphore() of a semaphore that doesn't exist does
        // nothing)
        if created {
            LIFECYCLE_EVENT_SEM.signal();
        }
    }
}

/// Wait up to `timeout` (`None`: as long as it takes) for the next
/// lifecycle event, with the activity mutex released meanwhile when this
/// thread holds it (so that the activity can send the event). Translation
/// of `Android_WaitLifecycleEvent()`. FIXME (upstream): without the
/// semaphore (before `nativeSetupJNI()`, as in a program that isn't an
/// app, or after `nativeQuit()`), `SDL_WaitSemaphoreTimeoutNS(NULL, ...)`
/// succeeds at once, so upstream's loop never ends (an event pump that
/// never returns); here no event comes.
pub(crate) fn wait_lifecycle_event(timeout: Option<Duration>) -> Option<LifecycleEvent> {
    let mut got_event = None;

    let mut relock_count = 0;
    lock_activity_mutex();
    while ACTIVITY_MUTEX_COUNT.load(Ordering::Acquire) > 1 {
        // We came into this function with the activity lock held, we need to unlock so lifecycle events can be dispatched
        relock_count += 1;
        unlock_activity_mutex();
    }
    unlock_activity_mutex();

    if lifecycle().created {
        while got_event.is_none() && LIFECYCLE_EVENT_SEM.wait_timeout(timeout) {
            let mut lifecycle = lifecycle();
            if !lifecycle.events.is_empty() {
                got_event = Some(lifecycle.events[0]);
                remove_lifecycle_event(&mut lifecycle.events, 0);
            }
        }
    }

    while relock_count > 0 {
        lock_activity_mutex();
        relock_count -= 1;
    }
    got_event
}

/// `Android_ActivityMutex`, which upstream's `nativeSetupJNI()` makes and
/// `nativeQuit()` destroys (locking it before or after does nothing).
/// Note: here it's a static, there from the start, never destroyed.
static ACTIVITY_MUTEX: RawMutex = RawMutex::new();

/// `Android_ActivityMutexCount`: how many times the thread that holds the
/// activity mutex has locked it.
static ACTIVITY_MUTEX_COUNT: AtomicI32 = AtomicI32::new(0);

/// Lock the activity mutex (recursive). Translation of
/// `Android_LockActivityMutex()`.
pub(crate) fn lock_activity_mutex() {
    ACTIVITY_MUTEX.lock();
    ACTIVITY_MUTEX_COUNT.fetch_add(1, Ordering::AcqRel);
}

/// Translation of `Android_UnlockActivityMutex()`.
pub(crate) fn unlock_activity_mutex() {
    ACTIVITY_MUTEX_COUNT.fetch_sub(1, Ordering::AcqRel);
    ACTIVITY_MUTEX.unlock();
}

// ---------------------------------------------------------------------------
// Functions exposed to SDL applications in SDL_system.h

/// Run `f` with the current thread's environment: `None` without one (a
/// program that isn't an app has no Java VM). Note (upstream): upstream
/// calls its Java statics with the environment `Android_JNI_GetEnv()`
/// gives, a null one there (which crashes), and leaves an exception the
/// Java method throws pending (where the next JNI call aborts, as CheckJNI
/// does in a debuggable app); here the exception is described in the log
/// and cleared, and the result is `None`.
fn with_env<T>(f: impl for<'a> FnOnce(Env<'a>) -> env::JniResult<'a, T>) -> Option<T> {
    let env = env::get_env()?;
    match f(env) {
        Ok(value) => Some(value),
        Err(thrown) => {
            thrown.describe();
            None
        }
    }
}

/// Trigger the Android system back button behavior. Translation of
/// `SDL_SendAndroidBackButton()` (`SDLActivity.manualBackButton()`).
pub(crate) fn send_android_back_button() {
    with_env(super::java::activity::manual_back_button);
}

/// Send a user command (0x8000 or more) to `SDLActivity` (to its command
/// handler, on the main thread, which gives it to `onUnhandledMessage()`).
/// Translation of `SDL_SendAndroidMessage()`. Note: upstream gives false
/// without an error when the message wasn't sent (no activity, or a
/// looper that is quitting); an error here.
pub(crate) fn send_android_message(command: u32, param: i32) -> crate::error::Result<()> {
    if command < 0x8000 {
        return Err(crate::error::Error::invalid_param("command"));
    }
    if send_message(command as i32, param) {
        Ok(())
    } else {
        Err(crate::error::Error::new(
            "SDLActivity's handler didn't take the message",
        ))
    }
}

/// The current thread's JNI environment (`JNIEnv *`), attaching the thread
/// to the Java VM if it isn't. Translation of `SDL_GetAndroidJNIEnv()`.
pub(crate) fn get_android_jni_env() -> Option<NonNull<c_void>> {
    env::get_env().and_then(|env| NonNull::new(env.raw().cast()))
}

/// The activity (`SDLActivity.getContext()`), as a local reference of the
/// current thread's environment, which the caller deletes. Translation of
/// `SDL_GetAndroidActivity()`.
pub(crate) fn get_android_activity() -> Option<NonNull<c_void>> {
    // See SDL_system.h for caveats on using this function.

    let env = env::get_env()?;

    // return SDLActivity.getContext();
    let context = super::java::activity::get_context(env)?;
    NonNull::new(context.into_raw().cast())
}

/// Translation of `SDL_IsChromebook()`.
pub(crate) fn is_chromebook() -> bool {
    with_env(super::java::activity::is_chromebook).unwrap_or(false)
}

/// Translation of `SDL_IsDeXMode()`.
pub(crate) fn is_dex_mode() -> bool {
    with_env(super::java::activity::is_dex_mode).unwrap_or(false)
}

/// Translation of `Android_JNI_SetupThread()` (in [`env`]): an SDL
/// thread's start attaches it to the Java VM.
pub(crate) use super::env::setup_thread;

/// The device's API level (its `ro.build.version.sdk`), read once; 0 when
/// it can't be read. Translation of `SDL_GetAndroidSDKVersion()`.
pub(crate) fn get_android_sdk_version() -> i32 {
    static SDK_VERSION: AtomicI32 = AtomicI32::new(0);
    let mut sdk_version = SDK_VERSION.load(Ordering::Relaxed);
    if sdk_version == 0 {
        let mut sdk = [0 as c_char; libc::PROP_VALUE_MAX as usize];
        // SAFETY: a C string, and room for a property's value.
        let length = unsafe {
            libc::__system_property_get(c"ro.build.version.sdk".as_ptr(), sdk.as_mut_ptr())
        };
        if length != 0 {
            // SAFETY: the property's value, NUL-terminated.
            let value = unsafe { std::ffi::CStr::from_ptr(sdk.as_ptr()) };
            sdk_version = crate::stdlib::atoi(&value.to_string_lossy());
            SDK_VERSION.store(sdk_version, Ordering::Relaxed);
        }
    }
    sdk_version
}

/// Translation of `SDL_IsAndroidTablet()` (in SDL_android.h, not
/// SDL_system.h, and unused upstream: `SDL_IsTablet()` is the form
/// factor's).
#[allow(dead_code)]
pub(crate) fn is_android_tablet() -> bool {
    with_env(super::java::activity::is_tablet).unwrap_or(false)
}

/// Translation of `SDL_IsAndroidTV()` (in SDL_android.h, not
/// SDL_system.h, and unused upstream: `SDL_IsTV()` is the form factor's).
#[allow(dead_code)]
pub(crate) fn is_android_tv() -> bool {
    with_env(super::java::activity::is_android_tv).unwrap_or(false)
}

/// The form factor `SDLActivity.getDeviceFormFactor()` names. Translation
/// of `SDL_GetAndroidDeviceFormFactor()`.
pub(crate) fn get_android_device_form_factor() -> crate::init::FormFactor {
    use crate::init::FormFactor;
    let mut form_factor = FormFactor::Unknown;

    if let Some(string) = with_env(super::java::activity::get_device_form_factor) {
        form_factor = match string.as_str() {
            "tv" => FormFactor::Tv,
            "tablet" => FormFactor::Tablet,
            "phone" => FormFactor::Phone,
            "car" => FormFactor::Car,
            "headset" => FormFactor::Headset,
            "watch" => FormFactor::Watch,
            _ => FormFactor::Unknown,
        };
    }

    form_factor
}

// Interface from the SDL library into the Android Java activity (the
// video driver's)

/// Translation of `Android_JNI_SetActivityTitle()`.
pub(crate) fn set_activity_title(title: &str) {
    with_env(|env| super::java::activity::set_activity_title(env, title));
}

/// Translation of `Android_JNI_SetWindowStyle()`.
pub(crate) fn set_window_style(fullscreen: bool) {
    with_env(|env| super::java::activity::set_window_style(env, fullscreen));
}

/// The orientations the activity may take, from the window's size, whether
/// it's resizable and `hint` (`SDL_HINT_ORIENTATIONS`). Translation of
/// `Android_JNI_SetOrientation()`.
pub(crate) fn set_orientation(w: i32, h: i32, resizable: bool, hint: Option<&str>) {
    with_env(|env| {
        super::java::activity::set_orientation(env, w, h, resizable, hint.unwrap_or(""))
    });
}

// Touch support

/// The touch devices (`SDLActivity.initTouch()`). Translation of
/// `Android_JNI_InitTouch()`.
pub(crate) fn init_touch() {
    with_env(super::java::activity::init_touch);
}

// Cursor support

/// A custom cursor of an ARGB8888 surface's pixels (`w` by `h`, packed
/// rows), its ID (0 if the activity couldn't make it). Translation of
/// `Android_JNI_CreateCustomCursor()`.
pub(crate) fn create_custom_cursor(pixels: &[i32], w: i32, h: i32, hot_x: i32, hot_y: i32) -> i32 {
    with_env(|env| super::java::activity::create_custom_cursor(env, pixels, w, h, hot_x, hot_y))
        .unwrap_or(0)
}

/// Translation of `Android_JNI_DestroyCustomCursor()`.
pub(crate) fn destroy_custom_cursor(cursor_id: i32) {
    with_env(|env| {
        super::java::activity::destroy_custom_cursor(env, cursor_id);
        Ok(())
    });
}

/// Translation of `Android_JNI_SetCustomCursor()`.
pub(crate) fn set_custom_cursor(cursor_id: i32) -> bool {
    with_env(|env| super::java::activity::set_custom_cursor(env, cursor_id)).unwrap_or(false)
}

/// Translation of `Android_JNI_SetSystemCursor()`.
pub(crate) fn set_system_cursor(cursor_id: i32) -> bool {
    with_env(|env| super::java::activity::set_system_cursor(env, cursor_id)).unwrap_or(false)
}

// Relative mouse support

/// Translation of `Android_JNI_SupportsRelativeMouse()`.
pub(crate) fn supports_relative_mouse() -> bool {
    with_env(super::java::activity::supports_relative_mouse).unwrap_or(false)
}

/// Translation of `Android_JNI_SetRelativeMouseEnabled()`.
pub(crate) fn set_relative_mouse_enabled(enabled: bool) -> bool {
    with_env(|env| super::java::activity::set_relative_mouse_enabled(env, enabled)).unwrap_or(false)
}

/// Translation of `Android_JNI_MinimizeWindow()`.
pub(crate) fn minimize_window() {
    with_env(super::java::activity::minimize_window);
}

/// Translation of `Android_JNI_ShouldMinimizeOnFocusLoss()`.
pub(crate) fn should_minimize_on_focus_loss() -> bool {
    with_env(|env| Ok(super::java::activity::should_minimize_on_focus_loss(env))).unwrap_or(false)
}

/// Translation of `Android_JNI_SuspendScreenSaver()`.
pub(crate) fn suspend_screen_saver(suspend: bool) -> bool {
    send_message(COMMAND_SET_KEEP_SCREEN_ON, if suspend { 1 } else { 0 })
}

/// `COMMAND_SET_KEEP_SCREEN_ON` (it has to match the activity's).
const COMMAND_SET_KEEP_SCREEN_ON: i32 = 5;

/// The activity's surface's `ANativeWindow`, with a reference the caller
/// releases (null without a surface). Translation of
/// `Android_JNI_GetNativeWindow()`.
pub(crate) fn get_native_window() -> *mut super::ndk::ANativeWindow {
    with_env(|env| {
        let mut anw = ptr::null_mut();
        if let Some(s) = super::java::activity::get_native_surface(env)? {
            // SAFETY: an environment and a Surface.
            anw = unsafe { super::ndk::ANativeWindow_fromSurface(env.raw(), s.obj().raw()) };
        }
        Ok(anw)
    })
    .unwrap_or(ptr::null_mut())
}

/// Translation of `Android_JNI_GetDisplayNaturalOrientation()`.
pub(crate) fn display_natural_orientation() -> crate::video::DisplayOrientation {
    crate::video::DisplayOrientation::from_i32(DISPLAY_NATURAL_ORIENTATION.load(Ordering::Acquire))
}

/// Translation of `Android_JNI_GetDisplayCurrentOrientation()`.
pub(crate) fn display_current_orientation() -> crate::video::DisplayOrientation {
    crate::video::DisplayOrientation::from_i32(DISPLAY_CURRENT_ORIENTATION.load(Ordering::Acquire))
}

/// Translation of `Android_JNI_SendMessage()`.
pub(crate) fn send_message(command: i32, param: i32) -> bool {
    with_env(|env| super::java::activity::send_message(env, command, param)).unwrap_or(false)
}

/// Turn the back button's trap on or off (from API 33, a callback of the
/// activity's back dispatcher, which makes the back button a key).
/// Translation of `Android_JNI_SetBackButtonTrapActive()`, which the
/// android video driver calls when `SDL_HINT_ANDROID_TRAP_BACK_BUTTON`
/// changes.
pub(crate) fn set_back_button_trap_active(enabled: bool) {
    // (midSetBackButtonTrapEnabled: there is one)
    with_env(|env| super::java::activity::set_back_button_trap_enabled(env, enabled));
}

/// Set an environment variable at startup (the manifest's): the process's
/// own, not SDL's environment (`setenv()`, which ignores a name that is
/// empty or has a `=` in it). Translation of `nativeSetenv()`.
pub(crate) fn setenv(env: Env<'_>, name: Obj<'_>, value: Obj<'_>) {
    let get = |string: Obj<'_>| {
        env.get_string(string).unwrap_or_else(|thrown| {
            thrown.clear();
            String::new()
        })
    };
    let (utfname, utfvalue) = (get(name), get(value));

    // This is only called at startup, to initialize the environment
    // Note that we call setenv() directly to avoid affecting SDL environments
    // (std::env::set_var(), setenv() with Rust's lock of the environment)
    if !utfname.is_empty() && !utfname.contains(['=', '\0']) && !utfvalue.contains('\0') {
        std::env::set_var(&utfname, &utfvalue); // This should NOT be SDL_setenv()
    }

    if utfname == crate::hints::ANDROID_ALLOW_RECREATE_ACTIVITY {
        // Special handling for this hint, which needs to persist outside the normal application flow
        // Only set this the first time we run, in case it's been set by the application via SDL_SetHint()
        if !ALLOW_RECREATE_ACTIVITY_SET.load(Ordering::Acquire) {
            set_allow_recreate_activity(crate::hints::string_to_bool(Some(&utfvalue), false));
        }
    }
}

pub(crate) extern "system" fn native_setenv(
    env: *mut JNIEnv,
    _cls: jclass,
    name: jstring,
    value: jstring,
) {
    // SAFETY: a native method's environment and arguments.
    let (env, name, value) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(name),
            Obj::from_raw(value),
        )
    };
    setenv(env, name, value);
}

/// Environment support: the manifest's `SDL_ENV.NAME` meta-data, set as
/// environment variables (once). Translation of
/// `Android_JNI_GetManifestEnvironmentVariables()`, which `SDL_getenv()`
/// and SDL's environment call first.
pub(crate) fn get_manifest_environment_variables() {
    if activity_class().is_none() {
        log(
            LogPriority::Warn,
            "Request to get environment variables before JNI is ready",
        );
        return;
    }

    if !HAS_ENVIRONMENT_VARIABLES.load(Ordering::Acquire) {
        let Some(env) = env::get_env() else {
            return;
        };
        let ret = super::java::activity::get_manifest_environment_variables(env);
        if ret {
            HAS_ENVIRONMENT_VARIABLES.store(true, Ordering::Release);
        }
    }
}

// ---------------------------------------------------------------------------
// The paths, the package, toasts, permissions, the locale, URLs, files of
// the content resolver and the battery

/// The current thread's environment, or the error of having none (a
/// program that isn't an app: upstream dereferences the null environment
/// `Android_JNI_GetEnv()` gives there, after logging this).
fn env_or_error() -> crate::error::Result<Env<'static>> {
    env::get_env().ok_or_else(|| crate::error::Error::new("Failed, there is no JavaVM"))
}

/// A path the context gives, as a `File` (`context.METHOD(args)`), by
/// `path_method` (`getCanonicalPath()` or `getAbsolutePath()`), or the
/// error `none` when it's null. (Upstream's three storage paths, whose
/// duplication its own FIXME notes.)
fn context_path(
    func: &'static str,
    method: &CStr,
    descriptor: &CStr,
    args: &[Arg<'_>],
    none: &'static str,
    path_method: &CStr,
) -> crate::error::Result<String> {
    use crate::error::Error;
    let env = env_or_error()?;
    let mut refs = env::LocalReferenceHolder::setup(func);
    let env = refs.init(env)?;

    // context = SDLActivity.getContext();
    let Some(context) = super::java::activity::get_context(env) else {
        // (Note (upstream): only the internal storage path checks; the
        // others call a method of null, which ART aborts on)
        return Err(Error::new("Couldn't get Android context!"));
    };

    // fileObj = context.METHOD();
    let file_object = match env.call_method(context.obj(), method, descriptor, args) {
        Ok(file) => file.object(),
        Err(thrown) => {
            drop(thrown);
            return Err(env::exception_occurred(env, false).unwrap_or_else(|| Error::new(none)));
        }
    };
    let Some(file_object) = file_object else {
        return Err(Error::new(none));
    };

    // path = fileObject.PATH_METHOD();
    let path_string =
        match env.call_method(file_object.obj(), path_method, c"()Ljava/lang/String;", &[]) {
            Ok(path) => path.object(),
            Err(thrown) => {
                drop(thrown);
                // (Note (upstream): only the internal storage's
                // getCanonicalPath() is checked for an exception; the others'
                // getAbsolutePath() throws none)
                return Err(env::exception_occurred(env, false).unwrap_or_else(|| Error::new(none)));
            }
        };
    let Some(path_string) = path_string else {
        return Err(Error::new(none));
    };
    env.get_string(path_string.obj()).map_err(|thrown| {
        drop(thrown);
        env::exception_occurred(env, false).unwrap_or_else(|| Error::new(none))
    })
}

/// A path computed once (upstream's `static char *`, kept once it is had).
fn cached_path(
    cache: &'static Mutex<Option<String>>,
    get: impl FnOnce() -> crate::error::Result<String>,
) -> crate::error::Result<String> {
    let mut cached = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(path) = &*cached {
        return Ok(path.clone());
    }
    let path = get()?;
    *cached = Some(path.clone());
    Ok(path)
}

/// The app's internal storage (`context.getFilesDir().getCanonicalPath()`:
/// `/data/data/PACKAGE/files`), computed once. Translation of
/// `SDL_GetAndroidInternalStoragePath()`.
pub(crate) fn get_android_internal_storage_path() -> crate::error::Result<String> {
    static S_ANDROID_INTERNAL_FILES_PATH: Mutex<Option<String>> = Mutex::new(None);
    cached_path(&S_ANDROID_INTERNAL_FILES_PATH, || {
        context_path(
            "SDL_GetAndroidInternalStoragePath",
            c"getFilesDir",
            c"()Ljava/io/File;",
            &[],
            "Couldn't get internal directory",
            c"getCanonicalPath",
        )
    })
}

/// `SDL_ANDROID_EXTERNAL_STORAGE_READ`.
pub(crate) const ANDROID_EXTERNAL_STORAGE_READ: u32 = 0x01;
/// `SDL_ANDROID_EXTERNAL_STORAGE_WRITE`.
pub(crate) const ANDROID_EXTERNAL_STORAGE_WRITE: u32 = 0x02;

/// The external storage's state (`Environment.getExternalStorageState()`,
/// logged): read and write when it's `mounted`, read when `mounted_ro`,
/// else 0. Translation of `SDL_GetAndroidExternalStorageState()`.
pub(crate) fn get_android_external_storage_state() -> u32 {
    let Some(env) = env::get_env() else {
        return 0;
    };
    let mut refs = env::LocalReferenceHolder::setup("SDL_GetAndroidExternalStorageState");
    let Ok(env) = refs.init(env) else {
        return 0;
    };

    let state = env
        .find_class(c"android/os/Environment")
        .and_then(|cls| {
            env.call_static_method(
                cls.obj(),
                c"getExternalStorageState",
                c"()Ljava/lang/String;",
                &[],
            )
        })
        .map(Value::object);
    let state_string = match state {
        Ok(Some(state)) => env.get_string(state.obj()).unwrap_or_else(|thrown| {
            thrown.clear();
            String::new()
        }),
        Ok(None) => String::new(),
        Err(thrown) => {
            // (Note (upstream): left pending, for the next JNI call)
            thrown.describe();
            String::new()
        }
    };

    // Print an info message so people debugging know the storage state
    log(
        LogPriority::Info,
        &format!("external storage state: {state_string}"),
    );

    if state_string == "mounted" {
        ANDROID_EXTERNAL_STORAGE_READ | ANDROID_EXTERNAL_STORAGE_WRITE
    } else if state_string == "mounted_ro" {
        ANDROID_EXTERNAL_STORAGE_READ
    } else {
        0
    }
}

/// The app's external storage (`context.getExternalFilesDir(null)
/// .getAbsolutePath()`: `/storage/emulated/0/Android/data/PACKAGE/files`),
/// computed once. Translation of `SDL_GetAndroidExternalStoragePath()`.
pub(crate) fn get_android_external_storage_path() -> crate::error::Result<String> {
    static S_ANDROID_EXTERNAL_FILES_PATH: Mutex<Option<String>> = Mutex::new(None);
    cached_path(&S_ANDROID_EXTERNAL_FILES_PATH, || {
        context_path(
            "SDL_GetAndroidExternalStoragePath",
            c"getExternalFilesDir",
            c"(Ljava/lang/String;)Ljava/io/File;",
            &[Arg::Object(Obj::NULL)],
            "Couldn't get external directory",
            c"getAbsolutePath",
        )
    })
}

/// The app's cache (`context.getCacheDir().getAbsolutePath()`:
/// `/data/user/0/PACKAGE/cache`), computed once. Translation of
/// `SDL_GetAndroidCachePath()`.
pub(crate) fn get_android_cache_path() -> crate::error::Result<String> {
    // !!! FIXME: lots of duplication with SDL_GetAndroidExternalStoragePath and SDL_GetAndroidInternalStoragePath; consolidate these functions!
    static S_ANDROID_CACHE_PATH: Mutex<Option<String>> = Mutex::new(None);
    cached_path(&S_ANDROID_CACHE_PATH, || {
        context_path(
            "SDL_GetAndroidCachePath",
            c"getCacheDir",
            c"()Ljava/io/File;",
            &[],
            "Couldn't get cache directory",
            c"getAbsolutePath",
        )
    })
}

/// The app's package name (`context.getPackageName()`), not cached (the
/// executable's name, which `SDL_GetExeName()` caches). Translation of
/// `SDL_GetAndroidPackageName()`.
pub(crate) fn get_android_package_name() -> crate::error::Result<String> {
    use crate::error::Error;
    // this doesn't currently cache this, because it's only used by SDL_GetExeName, which _does_ cache it.
    let env = env_or_error()?;
    let mut refs = env::LocalReferenceHolder::setup("SDL_GetAndroidPackageName");
    let env = refs.init(env)?;

    // context = SDLActivity.getContext();
    let Some(context) = super::java::activity::get_context(env) else {
        return Err(Error::new("Couldn't get Android context!"));
    };

    let name = match env.call_method(
        context.obj(),
        c"getPackageName",
        c"()Ljava/lang/String;",
        &[],
    ) {
        Ok(name) => name.object(),
        Err(thrown) => {
            drop(thrown);
            return Err(env::exception_occurred(env, false).unwrap_or_else(|| Error::new("")));
        }
    };
    let Some(name) = name else {
        // (GetStringUTFChars() of null: Note (upstream), which ART aborts on)
        return Err(Error::new(""));
    };
    env.get_string(name.obj()).map_err(|thrown| {
        drop(thrown);
        env::exception_occurred(env, false).unwrap_or_else(|| Error::new(""))
    })
}

/// Show a toast (on the main thread). Translation of
/// `SDL_ShowAndroidToast()` and `Android_JNI_ShowToast()`. Note
/// (upstream): false without an error when there's no activity (or
/// showing it threw); here the error's message is empty.
pub(crate) fn show_android_toast(
    message: &str,
    duration: i32,
    gravity: i32,
    x_offset: i32,
    y_offset: i32,
) -> crate::error::Result<()> {
    let env = env_or_error()?;
    let shown = with_env_of(env, |env| {
        super::java::activity::show_toast(env, message, duration, gravity, x_offset, y_offset)
    });
    if shown == Some(true) {
        Ok(())
    } else {
        Err(crate::error::Error::new(""))
    }
}

/// `f` with `env`, whose exception (pending) is described and cleared (see
/// [`with_env`]).
fn with_env_of<'a, T>(env: Env<'a>, f: impl FnOnce(Env<'a>) -> env::JniResult<'a, T>) -> Option<T> {
    match f(env) {
        Ok(value) => Some(value),
        Err(thrown) => {
            thrown.describe();
            None
        }
    }
}

/// The callback of a permission's request: the permission, and whether it
/// is granted.
pub(crate) type PermissionCallback = Box<dyn FnOnce(&str, bool) + Send>;

/// A request waiting for its answer. Translation of
/// `NativePermissionRequestInfo`.
struct NativePermissionRequestInfo {
    request_code: i32,
    permission: String,
    callback: PermissionCallback,
}

/// `pending_permissions`, the requests waiting for their answers (the
/// newest first). Note: upstream guards them with `SDL_event_lock`; here
/// a mutex of their own.
static PENDING_PERMISSIONS: Mutex<Vec<NativePermissionRequestInfo>> = Mutex::new(Vec::new());

/// A request's answer, to its callback (called on this thread, without the
/// lock held), once. Translation of `nativePermissionResult()`.
pub(crate) fn permission_result(request_code: i32, result: bool) {
    let info = {
        let mut pending = PENDING_PERMISSIONS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        pending
            .iter()
            .position(|info| info.request_code == request_code)
            .map(|i| pending.remove(i))
    };
    if let Some(info) = info {
        (info.callback)(&info.permission, result);
    }
}

pub(crate) extern "system" fn native_permission_result(
    _env: *mut JNIEnv,
    _cls: jclass,
    request_code: jint,
    result: jboolean,
) {
    permission_result(request_code, result != 0);
}

/// Request `permission` (an Android permission's name), whose answer goes
/// to `callback`: at once (on this thread, before this returns) when it's
/// granted already, else on the main thread once the user has answered.
/// Translation of `SDL_RequestAndroidPermission()`. Note (upstream): the
/// request is submitted even when the activity's method throws (as
/// without one), and its callback then never called; without a Java VM
/// (no app), here it fails.
pub(crate) fn request_android_permission(
    permission: &str,
    callback: PermissionCallback,
) -> crate::error::Result<()> {
    static NEXT_REQUEST_CODE: AtomicI32 = AtomicI32::new(0);
    let env = env_or_error()?;

    let request_code = NEXT_REQUEST_CODE.fetch_add(1, Ordering::AcqRel);
    PENDING_PERMISSIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(
            0,
            NativePermissionRequestInfo {
                request_code,
                permission: permission.to_owned(),
                callback,
            },
        );

    with_env_of(env, |env| {
        super::java::activity::request_permission(env, permission, request_code)
    });

    Ok(())
}

/// The user's locales (`ll_CC,...`), or `None` when they can't be had.
/// Translation of `Android_JNI_GetLocale()`.
pub(crate) fn get_locale() -> Option<String> {
    with_env(super::java::activity::get_preferred_locales)
}

/// Open a URL (the activity that views it), whether one does. Translation
/// of `Android_JNI_OpenURL()`.
pub(crate) fn open_url(url: &str) -> bool {
    with_env(|env| super::java::activity::open_url(env, url)).unwrap_or(false)
}

/// A file descriptor of the content resolver's file of `uri` (a
/// `content://` URI), opened in the `fopen()`-style `mode`, which the
/// caller owns. Translation of `Android_JNI_OpenFileDescriptor()`. Note
/// (upstream): an exception other than `FileNotFoundException` (a URI of
/// an unknown provider: `SecurityException`, `IllegalArgumentException`)
/// is left pending there, and its 0 taken for a descriptor (stdin's);
/// here it's described in the log, and it's -1.
pub(crate) fn open_file_descriptor(uri: &str, mode: &str) -> crate::error::Result<c_int> {
    // Get fopen-style modes
    let (mut moderead, mut modewrite, mut modeappend, mut modeupdate) =
        (false, false, false, false);

    for cmode in mode.chars() {
        match cmode {
            'a' => modeappend = true,
            'r' => moderead = true,
            'w' => modewrite = true,
            '+' => modeupdate = true,
            _ => {}
        }
    }

    // Translate fopen-style modes to ContentResolver modes.
    // Android only allows "r", "w", "wt", "wa", "rw" or "rwt".
    let mut content_resolver_mode = "r";

    if moderead {
        if modewrite {
            content_resolver_mode = "rwt";
        } else {
            content_resolver_mode = if modeupdate { "rw" } else { "r" };
        }
    } else if modewrite {
        content_resolver_mode = if modeupdate { "rwt" } else { "wt" };
    } else if modeappend {
        content_resolver_mode = if modeupdate { "rw" } else { "wa" };
    }

    let env = env_or_error()?;
    let fd = with_env_of(env, |env| {
        super::java::activity::open_file_descriptor(env, uri, content_resolver_mode)
    })
    .unwrap_or(-1);

    if fd == -1 {
        return Err(crate::error::Error::new("Unspecified error in JNI"));
    }

    Ok(fd)
}

/// What `Android_JNI_GetPowerInfo()` gives: whether the device is plugged
/// in, whether it's charged, whether it has a battery, the seconds left (-1:
/// never known) and the battery's percent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PowerInfo {
    pub(crate) plugged: bool,
    pub(crate) charged: bool,
    pub(crate) battery: bool,
    pub(crate) seconds: i32,
    pub(crate) percent: i32,
}

/// The battery's state, from the sticky `ACTION_BATTERY_CHANGED` intent
/// (`registerReceiver(null, filter)`), or `None` (-1: an error) when an
/// extra is missing. Translation of `Android_JNI_GetPowerInfo()`, asked
/// for everything (`SDL_GetPowerInfo_Android()` is its only caller).
pub(crate) fn get_power_info() -> Option<PowerInfo> {
    let env = env::get_env()?;
    let mut refs = env::LocalReferenceHolder::setup("Android_JNI_GetPowerInfo");
    let env = refs.init(env).ok()?;
    match power_info(env) {
        Ok(info) => info,
        Err(thrown) => {
            // (Note (upstream): left pending, for the next JNI call)
            thrown.describe();
            None
        }
    }
}

fn power_info(env: Env<'_>) -> env::JniResult<'_, Option<PowerInfo>> {
    // context = SDLActivity.getContext();
    let Some(context) = super::java::activity::get_context(env) else {
        // (Note (upstream): a method called on null, which ART aborts on)
        return Ok(None);
    };

    let action = env.new_string("android.intent.action.BATTERY_CHANGED")?;

    let cls = env.find_class(c"android/content/IntentFilter")?;

    let filter = env.new_object(
        cls.obj(),
        c"(Ljava/lang/String;)V",
        &[Arg::Object(action.obj())],
    )?;

    drop(action);

    // (the activity class's registerReceiver(), on the context)
    let intent = match activity_class() {
        Some(activity) => {
            let mid = env.get_method_id(
                activity,
                c"registerReceiver",
                c"(Landroid/content/BroadcastReceiver;Landroid/content/IntentFilter;)Landroid/content/Intent;",
            )?;
            // SAFETY: the activity class's method, called on the context
            // (the activity: the class's instance).
            unsafe {
                env.call_method_id(
                    context.obj(),
                    mid,
                    c"(Landroid/content/BroadcastReceiver;Landroid/content/IntentFilter;)Landroid/content/Intent;",
                    &[Arg::Object(Obj::NULL), Arg::Object(filter.obj())],
                )?
            }
            .object()
        }
        None => None,
    };

    drop(filter);

    let Some(intent) = intent else {
        // (GetObjectClass() of null: Note (upstream), which ART aborts on)
        return Ok(None);
    };

    let plug = int_extra(env, intent.obj(), "plugged")?; // == BatteryManager.EXTRA_PLUGGED (API 5)
    if plug == -1 {
        return Ok(None);
    }
    // 1 == BatteryManager.BATTERY_PLUGGED_AC
    // 2 == BatteryManager.BATTERY_PLUGGED_USB
    let plugged = 0 < plug;

    let status = int_extra(env, intent.obj(), "status")?; // == BatteryManager.EXTRA_STATUS (API 5)
    if status == -1 {
        return Ok(None);
    }
    // 5 == BatteryManager.BATTERY_STATUS_FULL
    let charged = status == 5;

    let present = bool_extra(env, intent.obj(), "present")?; // == BatteryManager.EXTRA_PRESENT (API 5)
    let battery = present;

    let seconds = -1; // not possible

    let level = int_extra(env, intent.obj(), "level")?; // == BatteryManager.EXTRA_LEVEL (API 5)
    let scale = int_extra(env, intent.obj(), "scale")?; // == BatteryManager.EXTRA_SCALE (API 5)

    if level == -1 || scale == -1 {
        return Ok(None);
    }
    // (Note (upstream): a scale of 0 divides by zero; here the percent is 0)
    let percent = (level * 100).checked_div(scale).unwrap_or(0);

    Ok(Some(PowerInfo {
        plugged,
        charged,
        battery,
        seconds,
        percent,
    }))
}

/// `intent.getIntExtra(key, -1)`.
fn int_extra<'a>(env: Env<'a>, intent: Obj<'_>, key: &str) -> env::JniResult<'a, jint> {
    let iname = env.new_string(key)?;
    Ok(env
        .call_method(
            intent,
            c"getIntExtra",
            c"(Ljava/lang/String;I)I",
            &[Arg::Object(iname.obj()), Arg::Int(-1)],
        )?
        .int())
}

/// `intent.getBooleanExtra(key, false)`.
fn bool_extra<'a>(env: Env<'a>, intent: Obj<'_>, key: &str) -> env::JniResult<'a, bool> {
    let bname = env.new_string(key)?;
    Ok(env
        .call_method(
            intent,
            c"getBooleanExtra",
            c"(Ljava/lang/String;Z)Z",
            &[Arg::Object(bname.obj()), Arg::Boolean(false)],
        )?
        .boolean())
}
