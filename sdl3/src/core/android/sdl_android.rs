// Rust translation of src/core/android/SDL_android.c from Simple
// DirectMedia Layer (part: the library's loading, the registration of the
// native methods, SDLActivity's natives that start the SDL thread and set
// the environment, and the manifest's environment variables).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `JNI_OnLoad()`, which registers the native methods of the classes of
//! [`super::classes`] (upstream's `register_methods()` of its tables:
//! `SDLActivity_tab` so far) and caches them, with the app's class loader,
//! for the threads that aren't the VM's; and `SDLActivity`'s natives that
//! run the app's `SDL_main` (`nativeSetupJNI()`, `nativeInitMainThread()`,
//! `nativeRunMain()`, ...) and set its environment (`nativeSetenv()`,
//! `Android_JNI_GetManifestEnvironmentVariables()`). The rest of
//! SDL_android.c (the lifecycle events, the video, audio and controller
//! natives, the assets, paths, permissions, ...) comes with the backends
//! that use it.
//!
//! This library is exported as libSDL3.so's is: a program that links it
//! has its `JNI_OnLoad()` (as an app with upstream's static library has),
//! which Android calls when the app's Java loads `libmain`.

use super::classes::{self, Class, Code, CLASSES, SDL_ACTIVITY};
use super::env::{self, Arg, Env, Global, Obj, Value};
use super::jni::*;
use super::ndk::LogPriority;
use std::ffi::{c_char, c_int, c_void, CString};
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering};

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
/// 26 and later), and registered there.
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

    // (the activity mutex, and the lifecycle mutex and semaphore, come
    // with the lifecycle events)

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
