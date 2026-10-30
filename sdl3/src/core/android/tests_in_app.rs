// The JNI layer's checks, run in an app.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The checks of [`super::env`] and of the registration, which need a
//! Java VM: a program spawned from the shell has none, so they run in an
//! app, sdl3/tests/android_jni.rs (through `sdl3::__android_jni_checks()`),
//! on its SDL thread (a VM's thread, the activity's SDLThread) and on
//! threads of its own (attached here). In a spawned program the first
//! check finds no VM, and the others don't run.

use super::classes::{CLASSES, SDL_ACTIVITY, SDL_MAIN};
use super::env::{self, Arg, Env, Global, LocalReferenceHolder, Obj};
use super::jni::*;
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};

type Check = Result<(), String>;

/// A check, run in a local frame of its own.
type CheckFn = fn(Env<'_>) -> Check;

macro_rules! ensure {
    ($cond:expr, $($fmt:tt)*) => {
        if !$cond {
            return Err(format!($($fmt)*));
        }
    };
}

/// A call that must not throw: its exception described (in the log) and
/// cleared, as the error.
fn ok<'a, T>(what: &str, result: env::JniResult<'a, T>) -> Result<T, String> {
    result.map_err(|thrown| {
        thrown.describe();
        format!("{what} threw")
    })
}

/// The checks, by name, with their outcome.
pub(crate) fn run() -> Vec<(&'static str, Check)> {
    let Some(env) = env::java_vm().and_then(|_| env::get_env()) else {
        return vec![("java_vm", Err("no Java VM (not an app)".to_string()))];
    };
    let checks: [(&'static str, CheckFn); 12] = [
        ("java_vm", java_vm),
        ("registered", registered),
        ("strings", strings),
        ("exceptions", exceptions),
        ("exception_occurred", exception_occurred),
        ("local_frames", local_frames),
        ("references", references),
        ("fields_and_arrays", fields_and_arrays),
        ("nonvirtual", nonvirtual),
        ("native_threads", native_threads),
        ("in_memory_dex", in_memory_dex),
        ("manifest_environment", manifest_environment),
    ];
    checks
        .iter()
        .map(|(name, check)| {
            let result = env.with_local_frame(64, check).unwrap_or_else(|thrown| {
                thrown.describe();
                Err("PushLocalFrame() threw".to_string())
            });
            if env.exception_check() {
                env.exception_describe();
                env.exception_clear();
                return (
                    *name,
                    Err(format!("left an exception pending ({result:?})")),
                );
            }
            (*name, result)
        })
        .collect()
}

fn java_vm(env: Env<'_>) -> Check {
    ensure!(
        env.get_version() >= JNI_VERSION_1_6,
        "JNI version {:#x}",
        env.get_version()
    );
    let vm = env.get_java_vm().ok_or("no GetJavaVM()")?;
    ensure!(
        Some(vm) == env::java_vm(),
        "GetJavaVM() isn't JNI_OnLoad()'s VM"
    );
    // (this thread's env is the one kept for it: the SDL thread's)
    let again = env::get_env().ok_or("no env")?;
    ensure!(
        again.raw() == env.raw(),
        "another JNIEnv on the same thread"
    );
    ensure!(super::sdk_int(env) >= 21, "SDK_INT {}", super::sdk_int(env));
    Ok(())
}

/// Every class of the table is cached, and its natives answer through JNI
/// (registered: calling an unregistered native throws
/// UnsatisfiedLinkError).
fn registered(env: Env<'_>) -> Check {
    for class in CLASSES {
        ensure!(
            env::app_class(class.name).is_some(),
            "{} isn't cached",
            class.name
        );
    }
    let activity = env::app_class(SDL_ACTIVITY).unwrap();
    let version = ok(
        "nativeGetVersion()",
        env.call_static_method(
            activity.obj(),
            c"nativeGetVersion",
            c"()Ljava/lang/String;",
            &[],
        ),
    )?
    .object()
    .ok_or("nativeGetVersion() returned null")?;
    let version = ok("GetStringRegion()", env.get_string(version.obj()))?;
    ensure!(version == "3.5.0", "nativeGetVersion() = {version:?}");
    let subsystems = ok(
        "nativeGetCompiledSubsystems()",
        env.call_static_method(activity.obj(), c"nativeGetCompiledSubsystems", c"()I", &[]),
    )?
    .int();
    ensure!(
        subsystems == 0x1b230,
        "nativeGetCompiledSubsystems() = {subsystems:#x}"
    );
    // (the class extends Activity, and SDLMain implements Runnable)
    let activity_base = ok("FindClass()", env.find_class(c"android/app/Activity"))?;
    let superclass = env.get_superclass(activity.obj()).ok_or("no superclass")?;
    ensure!(
        env.is_same_object(superclass.obj(), activity_base.obj()),
        "SDLActivity's superclass"
    );
    let runnable = ok("FindClass()", env.find_class(c"java/lang/Runnable"))?;
    let main = env::app_class(SDL_MAIN).unwrap();
    let instance = ok("new SDLMain()", env.new_object(main.obj(), c"()V", &[]))?;
    ensure!(
        env.is_instance_of(instance.obj(), runnable.obj()),
        "SDLMain isn't a Runnable"
    );
    Ok(())
}

fn strings(env: Env<'_>) -> Check {
    for s in [
        "",
        "main",
        "é€",
        "\u{1F600} and \u{10FFFF}",
        "a\0b",
        "日本語",
    ] {
        let java = ok("NewString()", env.new_string(s))?;
        let length = ok(
            "length()",
            env.call_method(java.obj(), c"length", c"()I", &[]),
        )?
        .int();
        ensure!(
            length as usize == s.encode_utf16().count(),
            "{s:?}.length() = {length}"
        );
        let back = ok("GetStringRegion()", env.get_string(java.obj()))?;
        ensure!(back == s, "{s:?} came back as {back:?}");
    }
    // Modified UTF-8, as upstream's GetStringUTFChars() gives it: U+0000
    // as two bytes; and on ART a character past U+FFFF as its UTF-8 (not
    // as its surrogates, which JNI's specification says)
    let java = ok("NewString()", env.new_string("a\0b"))?;
    let mutf8 = ok("GetStringUTFChars()", env.get_string_utf_chars(java.obj()))?;
    ensure!(
        mutf8.as_bytes() == [b'a', 0xc0, 0x80, b'b'],
        "Modified UTF-8 of a, U+0000, b: {:x?}",
        mutf8.as_bytes()
    );
    let java = ok("NewString()", env.new_string("\u{1F600}"))?;
    let mutf8 = ok("GetStringUTFChars()", env.get_string_utf_chars(java.obj()))?;
    ensure!(
        mutf8.as_bytes() == "\u{1F600}".as_bytes(),
        "Modified UTF-8 of U+1F600: {:x?}",
        mutf8.as_bytes()
    );
    let java = ok("NewStringUTF()", env.new_string_utf(c"ascii"))?;
    ensure!(
        ok("GetStringRegion()", env.get_string(java.obj()))? == "ascii",
        "NewStringUTF()"
    );
    Ok(())
}

fn exceptions(env: Env<'_>) -> Check {
    let integer = ok("FindClass()", env.find_class(c"java/lang/Integer"))?;
    let x = ok("NewString()", env.new_string("x"))?;
    // Integer.parseInt("x") throws NumberFormatException: caught
    let thrown = match env.call_static_method(
        integer.obj(),
        c"parseInt",
        c"(Ljava/lang/String;)I",
        &[Arg::Object(x.obj())],
    ) {
        Ok(v) => return Err(format!("parseInt(\"x\") = {v:?}")),
        Err(thrown) => thrown,
    };
    ensure!(env.exception_check(), "no exception pending after a throw");
    let e = thrown.catch_exception().map_err(|t| {
        t.clear();
        "NumberFormatException isn't an Exception".to_string()
    })?;
    ensure!(!env.exception_check(), "an exception pending after catch");
    let nfe = ok(
        "FindClass()",
        env.find_class(c"java/lang/NumberFormatException"),
    )?;
    ensure!(
        env.is_instance_of(e.obj(), nfe.obj()),
        "not a NumberFormatException"
    );
    // An Error isn't an Exception: catch_exception() throws it again
    let error = ok("FindClass()", env.find_class(c"java/lang/AssertionError"))?;
    ensure!(env.throw_new(error.obj(), c"an error"), "ThrowNew()");
    let thrown = env
        .pending()
        .err()
        .ok_or("ThrowNew() left nothing pending")?;
    match thrown.catch_exception() {
        Ok(_) => return Err("an AssertionError was caught as an Exception".to_string()),
        Err(thrown) => {
            ensure!(env.exception_check(), "the Error isn't pending again");
            thrown.clear();
        }
    }
    // A method that doesn't exist: NoSuchMethodError
    let missing = env.get_static_method_id(integer.obj(), c"noSuchMethod", c"()V");
    match missing {
        Ok(_) => return Err("GetStaticMethodID() of no method".to_string()),
        Err(thrown) => thrown.clear(),
    }
    // FindClass() of no class: ClassNotFoundException (as
    // NoClassDefFoundError)
    match env.find_class(c"org/libsdl/app/NoSuchClass") {
        Ok(_) => Err("FindClass() of no class".to_string()),
        Err(thrown) => {
            thrown.clear();
            Ok(())
        }
    }
}

/// Upstream's `Android_JNI_ExceptionOccurred()` in a
/// `LocalReferenceHolder`.
fn exception_occurred(env: Env<'_>) -> Check {
    let mut refs = LocalReferenceHolder::setup("exception_occurred");
    let env = refs.init(env).map_err(|e| e.to_string())?;
    ensure!(
        env::exception_occurred(env, false).is_none(),
        "an exception without a throw"
    );
    let integer = ok("FindClass()", env.find_class(c"java/lang/Integer"))?;
    let x = ok("NewString()", env.new_string("x"))?;
    let result = env.call_static_method(
        integer.obj(),
        c"parseInt",
        c"(Ljava/lang/String;)I",
        &[Arg::Object(x.obj())],
    );
    std::mem::forget(result); // (pending, as upstream's unchecked call leaves it)
    let error = env::exception_occurred(env, false).ok_or("no exception")?;
    ensure!(
        error.to_string() == "java.lang.NumberFormatException: For input string: \"x\"",
        "the error: {error}"
    );
    ensure!(!env.exception_check(), "still pending");
    let result = env.call_static_method(
        integer.obj(),
        c"parseInt",
        c"(Ljava/lang/String;)I",
        &[Arg::Object(x.obj())],
    );
    std::mem::forget(result);
    ensure!(
        env::exception_occurred(env, true).is_some(),
        "no exception (silent)"
    );
    ensure!(!env.exception_check(), "still pending (silent)");
    Ok(())
}

/// Local references: deleted when dropped (thousands in a loop), and a
/// frame's freed when it pops.
fn local_frames(env: Env<'_>) -> Check {
    // (more than any table of local references holds: each is deleted)
    for i in 0..10_000 {
        let s = ok("NewString()", env.new_string(&i.to_string()))?;
        ensure!(
            env.get_object_ref_type(s.obj()) == JNILocalRefType,
            "not a local reference"
        );
    }
    // In frames: references kept until the frame pops
    for _ in 0..20 {
        let kept = ok(
            "with_local_frame()",
            env.with_local_frame(600, |env| -> Result<usize, String> {
                let mut kept = Vec::new();
                for i in 0..500 {
                    kept.push(ok("NewString()", env.new_string(&i.to_string()))?);
                }
                // (the references dropped here are deleted before the pop)
                Ok(kept.len())
            }),
        )??;
        ensure!(kept == 500, "{kept}");
    }
    ok("EnsureLocalCapacity()", env.ensure_local_capacity(256))?;
    Ok(())
}

/// Global references, usable on another thread.
fn references(env: Env<'_>) -> Check {
    let s = ok("NewString()", env.new_string("global"))?;
    let global = Global::new(env, s.obj()).ok_or("NewGlobalRef()")?;
    ensure!(
        env.get_object_ref_type(global.obj()) == JNIGlobalRefType,
        "not a global reference"
    );
    ensure!(
        env.is_same_object(global.obj(), s.obj()),
        "not the same object"
    );
    let local = env.new_local_ref(global.obj()).ok_or("NewLocalRef()")?;
    ensure!(env.is_same_object(local.obj(), s.obj()), "NewLocalRef()");
    ensure!(
        Global::new(env, Obj::NULL).is_none(),
        "a global reference to null"
    );
    let on_thread = std::thread::spawn(move || -> Check {
        let env = env::get_env().ok_or("no env on a native thread")?;
        let text = ok("GetStringRegion()", env.get_string(global.obj()))?;
        ensure!(text == "global", "{text:?}");
        drop(global); // (DeleteGlobalRef() on this thread)
        Ok(())
    })
    .join()
    .map_err(|_| "the thread panicked".to_string())?;
    on_thread
}

fn fields_and_arrays(env: Env<'_>) -> Check {
    let version = ok("FindClass()", env.find_class(c"android/os/Build$VERSION"))?;
    let sdk = ok(
        "SDK_INT",
        env.get_static_field(version.obj(), c"SDK_INT", c"I"),
    )?
    .int();
    ensure!(sdk == super::sdk_int(env), "SDK_INT {sdk}");
    let point = ok("FindClass()", env.find_class(c"android/graphics/Point"))?;
    let p = ok(
        "new Point(1, 2)",
        env.new_object(point.obj(), c"(II)V", &[Arg::Int(1), Arg::Int(2)]),
    )?;
    ensure!(
        ok("x", env.get_field(p.obj(), c"x", c"I"))?.int() == 1,
        "Point.x"
    );
    ok("x = 7", env.set_field(p.obj(), c"x", c"I", Arg::Int(7)))?;
    ensure!(
        ok("x", env.get_field(p.obj(), c"x", c"I"))?.int() == 7,
        "Point.x = 7"
    );
    let text = ok(
        "toString()",
        env.call_method(p.obj(), c"toString", c"()Ljava/lang/String;", &[]),
    )?
    .object()
    .ok_or("null")?;
    let text = ok("GetStringRegion()", env.get_string(text.obj()))?;
    ensure!(text == "Point(7, 2)", "{text:?}");
    // arrays
    let string = ok("FindClass()", env.find_class(c"java/lang/String"))?;
    let array = ok(
        "NewObjectArray()",
        env.new_object_array(3, string.obj(), Obj::NULL),
    )?;
    ensure!(env.get_array_length(array.obj()) == 3, "length");
    let b = ok("NewString()", env.new_string("b"))?;
    ok(
        "SetObjectArrayElement()",
        env.set_object_array_element(array.obj(), 1, b.obj()),
    )?;
    ensure!(
        ok(
            "GetObjectArrayElement()",
            env.get_object_array_element(array.obj(), 0)
        )?
        .is_none(),
        "element 0 isn't null"
    );
    let one = ok(
        "GetObjectArrayElement()",
        env.get_object_array_element(array.obj(), 1),
    )?
    .ok_or("null")?;
    ensure!(
        ok("GetStringRegion()", env.get_string(one.obj()))? == "b",
        "element 1"
    );
    match env.get_object_array_element(array.obj(), 3) {
        Ok(_) => return Err("element 3 of 3".to_string()),
        Err(thrown) => thrown.clear(),
    }
    let bytes = ok("NewByteArray()", env.new_byte_array(&[1, 2, 255]))?;
    ensure!(env.get_array_length(bytes.obj()) == 3, "byte[] length");
    Ok(())
}

/// `CallNonvirtual*Method()`: the superclass's method, not the override
/// (`super.hashCode()` of a string is the identity hash code).
fn nonvirtual(env: Env<'_>) -> Check {
    let s = ok("NewString()", env.new_string("nonvirtual"))?;
    let virtual_hash = ok(
        "hashCode()",
        env.call_method(s.obj(), c"hashCode", c"()I", &[]),
    )?
    .int();
    let expected: i32 = "nonvirtual"
        .encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32));
    ensure!(
        virtual_hash == expected,
        "String.hashCode() {virtual_hash} (want {expected})"
    );
    let object = ok("FindClass()", env.find_class(c"java/lang/Object"))?;
    let object_hash = ok(
        "Object.hashCode()",
        env.call_nonvirtual_method(s.obj(), object.obj(), c"hashCode", c"()I", &[]),
    )?
    .int();
    let system = ok("FindClass()", env.find_class(c"java/lang/System"))?;
    let identity = ok(
        "identityHashCode()",
        env.call_static_method(
            system.obj(),
            c"identityHashCode",
            c"(Ljava/lang/Object;)I",
            &[Arg::Object(s.obj())],
        ),
    )?
    .int();
    ensure!(
        object_hash == identity,
        "Object.hashCode() {object_hash}, identityHashCode() {identity}"
    );
    Ok(())
}

/// On a thread of its own (a native one, attached by `get_env()` and
/// detached when it ends): the framework's classes are found, the app's
/// aren't (the system class loader's), but they are cached and found
/// through the app's class loader.
fn native_threads(_env: Env<'_>) -> Check {
    let results = (0..4)
        .map(|_| {
            std::thread::spawn(|| -> Check {
                let env = env::get_env().ok_or("no env on a native thread")?;
                ensure!(
                    env::get_env().map(Env::raw) == Some(env.raw()),
                    "a second env"
                );
                let r = env.with_local_frame(16, |env| -> Check {
                    ok(
                        "FindClass(android/os/Build)",
                        env.find_class(c"android/os/Build"),
                    )?;
                    let name = CString::new(SDL_ACTIVITY).unwrap();
                    match env.find_class(&name) {
                        Ok(_) => {
                            return Err(
                                "FindClass() of the app's class on a native thread".to_string()
                            )
                        }
                        Err(thrown) => thrown.clear(),
                    }
                    let cached = env::app_class(SDL_ACTIVITY).ok_or("not cached")?;
                    let found = ok("loadClass()", env::find_app_class(env, SDL_ACTIVITY))?
                        .ok_or("no class loader")?;
                    ensure!(
                        env.is_same_object(found.obj(), cached.obj()),
                        "another SDLActivity"
                    );
                    Ok(())
                });
                ok("with_local_frame()", r)?
            })
        })
        .collect::<Vec<_>>();
    for thread in results {
        thread
            .join()
            .map_err(|_| "a thread panicked".to_string())??;
    }
    // and with Android_JNI_SetupThread()
    std::thread::spawn(|| -> Check {
        ensure!(env::setup_thread(), "Android_JNI_SetupThread()");
        let env = env::get_env().ok_or("no env after SetupThread")?;
        ok("FindClass()", env.find_class(c"java/lang/String"))?;
        Ok(())
    })
    .join()
    .map_err(|_| "the thread panicked".to_string())?
}

/// Whether [`in_memory_run`] ran.
static IN_MEMORY_RAN: AtomicBool = AtomicBool::new(false);

/// The in-memory `SDLMain.run()`'s Rust function, for [`in_memory_dex`].
extern "system" fn in_memory_run(_env: *mut JNIEnv, _this: jobject) {
    IN_MEMORY_RAN.store(true, Ordering::Release);
}

/// This library's DEX file through an `InMemoryDexClassLoader`: its own
/// classes (not the app's), whose natives register and answer: its
/// `SDLMain`, run by a Java thread. (Its `SDLActivity` can't be
/// initialized: its static initializer loads `libmain`, which the app's
/// class loader has.)
fn in_memory_dex(env: Env<'_>) -> Check {
    if super::sdk_int(env) < 26 {
        println!("android_jni: InMemoryDexClassLoader is API 26 and later");
        return Ok(());
    }
    let loader = ok(
        "new InMemoryDexClassLoader()",
        super::in_memory_class_loader(env),
    )?;
    let load = |name: &str| -> Result<env::Local<'_>, String> {
        let name = ok("NewString()", env.new_string(name))?;
        ok(
            "loadClass()",
            env.call_method(
                loader.obj(),
                c"loadClass",
                c"(Ljava/lang/String;)Ljava/lang/Class;",
                &[Arg::Object(name.obj())],
            ),
        )?
        .object()
        .ok_or_else(|| "loadClass() returned null".to_string())
    };
    for (name, app) in [
        ("org.libsdl.app.SDLActivity", SDL_ACTIVITY),
        ("org.libsdl.app.SDLMain", SDL_MAIN),
    ] {
        let class = load(name)?;
        let app = env::app_class(app).unwrap();
        ensure!(
            !env.is_same_object(class.obj(), app.obj()),
            "the app's {name}, not the DEX file's"
        );
    }
    let main = load("org.libsdl.app.SDLMain")?;
    let table = [JNINativeMethod {
        name: c"run".as_ptr(),
        signature: c"()V".as_ptr(),
        fnPtr: in_memory_run as *mut std::ffi::c_void,
    }];
    let status = ok(
        "RegisterNatives()",
        env.register_natives(main.obj(), &table),
    )?;
    ensure!(status == JNI_OK, "RegisterNatives() = {status}");
    let runnable = ok("new SDLMain()", env.new_object(main.obj(), c"()V", &[]))?;
    let thread_class = ok("FindClass()", env.find_class(c"java/lang/Thread"))?;
    let thread = ok(
        "new Thread()",
        env.new_object(
            thread_class.obj(),
            c"(Ljava/lang/Runnable;)V",
            &[Arg::Object(runnable.obj())],
        ),
    )?;
    IN_MEMORY_RAN.store(false, Ordering::Release);
    ok(
        "start()",
        env.call_method(thread.obj(), c"start", c"()V", &[]),
    )?;
    ok(
        "join()",
        env.call_method(thread.obj(), c"join", c"()V", &[]),
    )?;
    ensure!(
        IN_MEMORY_RAN.load(Ordering::Acquire),
        "the in-memory SDLMain.run() didn't run"
    );
    Ok(())
}

/// The manifest's `SDL_ENV.*` meta-data were set as environment
/// variables (tools/android_run.py passes the variables that way).
fn manifest_environment(_env: Env<'_>) -> Check {
    super::get_manifest_environment_variables();
    let assert =
        std::env::var("SDL_ASSERT").map_err(|_| "no SDL_ASSERT from the manifest".to_string())?;
    ensure!(!assert.is_empty(), "SDL_ASSERT is empty");
    Ok(())
}
