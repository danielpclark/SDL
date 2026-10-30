// The Java classes of upstream's android-project
// (android-project/app/src/main/java/org/libsdl/app/*.java) that the
// framework must find in the app: their declarations, for the DEX file.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Upstream's Java runs on the Java side of an Android app; here it is Rust
//! (`super::java`), but the framework instantiates the manifest's
//! activity, its views and listeners from the app's DEX file, and ART can't
//! define a class from native code (no JNI `DefineClass()`). So the classes
//! of upstream's that extend a framework class or implement a framework
//! interface are declared here, with upstream's names, superclasses and
//! interfaces, and written as DEX by `super::dex` (a host test, which
//! checks that `classes_dex.rs` is current). Their methods have no
//! bytecode:
//!
//! * every method the framework calls (an override, an interface's) and
//!   every one of upstream's `native` methods is `native`, implemented by
//!   the Rust function [`native!`] names, which `JNI_OnLoad()` registers
//!   (`RegisterNatives()`, as upstream's `register_methods()` does, with
//!   upstream's names and signatures); an override calls the superclass's
//!   method with `CallNonvirtual*Method()` where upstream calls `super`;
//! * a constructor calls the superclass's with the same parameters
//!   ([`Code::SuperConstructor`]);
//! * `SDLActivity`'s static initializer loads the app's library
//!   ([`Code::LoadLibraries`]). Note: upstream's `onCreate()` loads the
//!   libraries of `getLibraries()` (`SDL3` and `main`) itself, before it
//!   calls into them; here `onCreate()` is native, so the library is loaded
//!   when the class is initialized, before the framework calls it. SDL is
//!   linked into the app's `libmain.so`, so there is no `libSDL3.so` to
//!   load.
//!
//! Each class's entry lists the members translated so far, in upstream's
//! order; the classes and members still to come (`SDLSurface`,
//! `SDLInputConnection`, `SDLDummyEdit`, the controller and HID managers'
//! listeners and receivers, ...) are added with their translations. The
//! anonymous classes upstream creates (`new Runnable() {...}`, dialog
//! listeners) can't be declared with upstream's names (`SDLActivity$1`),
//! which are the compiler's.

#[cfg(target_os = "android")]
use std::ffi::c_void;

/// `ACC_PUBLIC`.
pub(crate) const PUBLIC: u32 = 0x1;
/// `ACC_PRIVATE`.
pub(crate) const PRIVATE: u32 = 0x2;
/// `ACC_PROTECTED`.
pub(crate) const PROTECTED: u32 = 0x4;
/// `ACC_STATIC`.
pub(crate) const STATIC: u32 = 0x8;
/// `ACC_FINAL`.
pub(crate) const FINAL: u32 = 0x10;
/// `ACC_NATIVE`.
pub(crate) const NATIVE: u32 = 0x100;
/// `ACC_INTERFACE`.
pub(crate) const INTERFACE: u32 = 0x200;
/// `ACC_ABSTRACT`.
pub(crate) const ABSTRACT: u32 = 0x400;
/// `ACC_CONSTRUCTOR` (a DEX file's flag for `<init>` and `<clinit>`).
pub(crate) const CONSTRUCTOR: u32 = 0x10000;

/// A class: its name (a binary name with slashes, as JNI's `FindClass()`
/// takes it: `org/libsdl/app/SDLActivity`), its access flags, its
/// superclass and interfaces (the same way), and its members.
#[derive(Debug)]
pub(crate) struct Class {
    pub(crate) name: &'static str,
    pub(crate) access: u32,
    pub(crate) superclass: &'static str,
    pub(crate) interfaces: &'static [&'static str],
    pub(crate) fields: &'static [Field],
    pub(crate) methods: &'static [Method],
}

/// A field: its name, its type (a descriptor: `J`, `Ljava/lang/String;`)
/// and its access flags.
#[derive(Debug)]
pub(crate) struct Field {
    pub(crate) name: &'static str,
    pub(crate) descriptor: &'static str,
    pub(crate) access: u32,
}

/// A method: its name, its descriptor (`(Landroid/os/Bundle;)V`), its access
/// flags and its body.
#[derive(Debug)]
pub(crate) struct Method {
    pub(crate) name: &'static str,
    pub(crate) descriptor: &'static str,
    pub(crate) access: u32,
    pub(crate) code: Code,
}

/// A method's body.
#[derive(Debug)]
pub(crate) enum Code {
    /// None: the method is `native`, implemented by a Rust function.
    Native(Native),
    /// A constructor's: `super(...)`, the superclass's constructor of the
    /// same parameters, then `return`.
    SuperConstructor,
    /// A static initializer's: `System.loadLibrary(name)` for each name,
    /// then `return`.
    LoadLibraries(&'static [&'static str]),
}

/// A native method's Rust function, which `JNI_OnLoad()` registers.
#[cfg(target_os = "android")]
#[derive(Debug)]
pub(crate) struct Native(pub(crate) *mut c_void);

// SAFETY: a function's address, never written.
#[cfg(target_os = "android")]
unsafe impl Sync for Native {}

/// A native method's Rust function: only Android builds have the
/// functions (the DEX writer needs none).
#[cfg(not(target_os = "android"))]
#[derive(Debug)]
pub(crate) struct Native;

/// A [`Method`] that is `native` (with `access`, plus [`NATIVE`]), the
/// Rust function `$f` implementing it, whose parameters after the
/// `JNIEnv *` and the object (or the class, for a static method) are
/// written as `_`s, one for each of the Java method's: `[_, _]`. It's
/// checked at compile time that they are the types the descriptor says
/// (`jint` for `I`, `jobject` for a class or an array, ...). Only Android
/// builds name the function.
#[cfg(target_os = "android")]
macro_rules! native {
    ($access:expr, $name:literal, $descriptor:literal, $f:path [$($p:tt),*]) => {{
        const _: () = assert!(
            super::jni::native_shorty(
                $descriptor,
                super::jni::native_shorty_of(&($f as extern "system" fn(_, _, $($p),*) -> _)),
            ),
            concat!("the Rust function of ", $name, $descriptor, " doesn't take its parameters")
        );
        Method {
            name: $name,
            descriptor: $descriptor,
            access: $access | NATIVE,
            code: Code::Native(Native($f as *mut c_void)),
        }
    }};
}

#[cfg(not(target_os = "android"))]
macro_rules! native {
    ($access:expr, $name:literal, $descriptor:literal, $f:path [$($p:tt),*]) => {
        Method {
            name: $name,
            descriptor: $descriptor,
            access: $access | NATIVE,
            code: Code::Native(Native),
        }
    };
}

/// The superclass's constructor, of the same parameters.
const fn constructor(access: u32, descriptor: &'static str) -> Method {
    Method {
        name: "<init>",
        descriptor,
        access: access | CONSTRUCTOR,
        code: Code::SuperConstructor,
    }
}

#[cfg(target_os = "android")]
use super::java::activity;

/// `org.libsdl.app.SDLActivity`.
pub(crate) const SDL_ACTIVITY: &str = "org/libsdl/app/SDLActivity";
/// `org.libsdl.app.SDLMain`.
pub(crate) const SDL_MAIN: &str = "org/libsdl/app/SDLMain";

/// The classes, each after the classes of the file it extends or
/// implements.
pub(crate) static CLASSES: &[Class] = &[
    // SDLActivity.java. (Its OnSystemUiVisibilityChangeListener comes with
    // the video driver's SDLSurface, as do its methods for the view.)
    Class {
        name: SDL_ACTIVITY,
        access: PUBLIC,
        superclass: "android/app/Activity",
        interfaces: &[],
        fields: &[],
        methods: &[
            Method {
                name: "<clinit>",
                descriptor: "()V",
                access: STATIC | CONSTRUCTOR,
                code: Code::LoadLibraries(&["main"]),
            },
            constructor(PUBLIC, "()V"),
            // Setup
            native!(
                PROTECTED,
                "onCreate",
                "(Landroid/os/Bundle;)V",
                activity::on_create[_]
            ),
            // Events
            native!(PROTECTED, "onPause", "()V", activity::on_pause[]),
            native!(PROTECTED, "onResume", "()V", activity::on_resume[]),
            native!(PROTECTED, "onStop", "()V", activity::on_stop[]),
            native!(PROTECTED, "onStart", "()V", activity::on_start[]),
            native!(
                PUBLIC,
                "onWindowFocusChanged",
                "(Z)V",
                activity::on_window_focus_changed[_]
            ),
            native!(PUBLIC, "onTrimMemory", "(I)V", activity::on_trim_memory[_]),
            native!(
                PUBLIC,
                "onConfigurationChanged",
                "(Landroid/content/res/Configuration;)V",
                activity::on_configuration_changed[_]
            ),
            native!(PROTECTED, "onDestroy", "()V", activity::on_destroy[]),
            // C functions we call
            native!(PUBLIC | STATIC, "nativeGetVersion", "()Ljava/lang/String;", super::native_get_version[]),
            native!(PUBLIC | STATIC, "nativeSetupJNI", "()V", super::native_setup_jni[]),
            native!(
                PUBLIC | STATIC,
                "nativeGetCompiledSubsystems",
                "()I",
                super::native_get_compiled_subsystems[]
            ),
            native!(PUBLIC | STATIC, "nativeInitMainThread", "()V", super::native_init_main_thread[]),
            native!(PUBLIC | STATIC, "nativeCleanupMainThread", "()V", super::native_cleanup_main_thread[]),
            native!(
                PUBLIC | STATIC,
                "nativeRunMain",
                "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/Object;)I",
                super::native_run_main[_, _, _]
            ),
            native!(
                PUBLIC | STATIC,
                "nativeSetenv",
                "(Ljava/lang/String;Ljava/lang/String;)V",
                super::native_setenv[_, _]
            ),
            native!(
                PUBLIC | STATIC,
                "nativeAllowRecreateActivity",
                "()Z",
                super::native_allow_recreate_activity[]
            ),
            native!(
                PUBLIC | STATIC,
                "nativeCheckSDLThreadCounter",
                "()I",
                super::native_check_sdl_thread_counter[]
            ),
        ],
    },
    // SDLActivity.java's `class SDLMain implements Runnable`, the SDL
    // thread's code.
    Class {
        name: SDL_MAIN,
        access: 0,
        superclass: "java/lang/Object",
        interfaces: &["java/lang/Runnable"],
        fields: &[],
        methods: &[
            constructor(0, "()V"),
            native!(PUBLIC, "run", "()V", activity::sdl_main_run[]),
        ],
    },
];

/// The class of `name` in [`CLASSES`].
pub(crate) fn class(name: &str) -> Option<&'static Class> {
    CLASSES.iter().find(|c| c.name == name)
}

/// The DEX file of [`CLASSES`] (`classes.dex`), generated: in a section of
/// its own in Android builds (`.sdl3_android_dex`), from where
/// `tools/android_run.py` puts it in the app's package.
pub(crate) use super::classes_dex::SDL_CLASSES_DEX;
