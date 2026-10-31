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
//!   ([`Code::SuperConstructor`]); the rest of upstream's constructor runs
//!   in Rust after the object is made (`SDLSurface`'s:
//!   `super::java::surface::create_sdl_surface()`);
//! * `SDLActivity`'s static initializer loads the app's library
//!   ([`Code::LoadLibraries`]). Note: upstream's `onCreate()` loads the
//!   libraries of `getLibraries()` (`SDL3` and `main`) itself, before it
//!   calls into them; here `onCreate()` is native, so the library is loaded
//!   when the class is initialized, before the framework calls it. SDL is
//!   linked into the app's `libmain.so`, so there is no `libSDL3.so` to
//!   load.
//!
//! Each class's entry lists the members translated so far, in upstream's
//! order; the classes and members still to come (the controller and HID
//! managers' listeners and receivers, ...) are added with their
//! translations. The
//! anonymous classes upstream creates (`new Runnable() {...}`, dialog
//! listeners) can't be declared with upstream's names (`SDLActivity$1`),
//! which are the compiler's: they are two classes of their own,
//! `SDLNativeRunnable` and `SDLNativeListener` (`super::java::anonymous`).

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
use super::java::{activity, anonymous, controller_manager, dummy_edit, input_connection, surface};

/// `org.libsdl.app.SDLActivity`.
pub(crate) const SDL_ACTIVITY: &str = "org/libsdl/app/SDLActivity";
/// `org.libsdl.app.SDLActivity.SDLCommandHandler`.
pub(crate) const SDL_COMMAND_HANDLER: &str = "org/libsdl/app/SDLActivity$SDLCommandHandler";
/// `org.libsdl.app.SDLActivity.ShowTextInputTask`.
pub(crate) const SDL_SHOW_TEXT_INPUT_TASK: &str = "org/libsdl/app/SDLActivity$ShowTextInputTask";
/// `org.libsdl.app.SDLMain`.
pub(crate) const SDL_MAIN: &str = "org/libsdl/app/SDLMain";
/// `org.libsdl.app.SDLSurface`.
pub(crate) const SDL_SURFACE: &str = "org/libsdl/app/SDLSurface";
/// `org.libsdl.app.SDLDummyEdit`.
pub(crate) const SDL_DUMMY_EDIT: &str = "org/libsdl/app/SDLDummyEdit";
/// `org.libsdl.app.SDLInputConnection`.
pub(crate) const SDL_INPUT_CONNECTION: &str = "org/libsdl/app/SDLInputConnection";
/// `org.libsdl.app.SDLNativeRunnable`, upstream's anonymous `Runnable`s.
pub(crate) const SDL_NATIVE_RUNNABLE: &str = "org/libsdl/app/SDLNativeRunnable";
/// `org.libsdl.app.SDLNativeListener`, the invocation handler of the
/// proxies that are upstream's other anonymous classes.
pub(crate) const SDL_NATIVE_LISTENER: &str = "org/libsdl/app/SDLNativeListener";

/// The names of SDLControllerManager.java's generic motion listeners.
mod controller_manager_names {
    /// `org.libsdl.app.SDLGenericMotionListener_API14`.
    pub(crate) const API14: &str = "org/libsdl/app/SDLGenericMotionListener_API14";
    /// `org.libsdl.app.SDLGenericMotionListener_API24`.
    pub(crate) const API24: &str = "org/libsdl/app/SDLGenericMotionListener_API24";
    /// `org.libsdl.app.SDLGenericMotionListener_API26`.
    pub(crate) const API26: &str = "org/libsdl/app/SDLGenericMotionListener_API26";
    /// `org.libsdl.app.SDLGenericMotionListener_API29`.
    pub(crate) const API29: &str = "org/libsdl/app/SDLGenericMotionListener_API29";
}
#[cfg(target_os = "android")]
pub(crate) use controller_manager_names::{
    API14 as SDL_GENERIC_MOTION_LISTENER_API14, API24 as SDL_GENERIC_MOTION_LISTENER_API24,
    API26 as SDL_GENERIC_MOTION_LISTENER_API26, API29 as SDL_GENERIC_MOTION_LISTENER_API29,
};

/// The classes, each after the classes of the file it extends or
/// implements.
pub(crate) static CLASSES: &[Class] = &[
    // SDLActivity.java.
    Class {
        name: SDL_ACTIVITY,
        access: PUBLIC,
        superclass: "android/app/Activity",
        interfaces: &["android/view/View$OnSystemUiVisibilityChangeListener"],
        fields: &[
            // Handler for the messages
            Field {
                name: "commandHandler",
                descriptor: "Landroid/os/Handler;",
                access: 0,
            },
            // (private final: set from native code)
            Field {
                name: "rehideSystemUi",
                descriptor: "Ljava/lang/Runnable;",
                access: PRIVATE,
            },
        ],
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
            native!(PUBLIC, "onBackPressed", "()V", activity::on_back_pressed[]),
            native!(
                PUBLIC,
                "dispatchKeyEvent",
                "(Landroid/view/KeyEvent;)Z",
                activity::dispatch_key_event[_]
            ),
            // C functions we call
            native!(PUBLIC | STATIC, "nativeGetVersion", "()Ljava/lang/String;", super::native_get_version[]),
            native!(PUBLIC | STATIC, "nativeSetupJNI", "()V", super::native_setup_jni[]),
            native!(
                PUBLIC | STATIC,
                "nativeGetCompiledSubsystems",
                "()I",
                super::native_get_compiled_subsystems[]
            ),
            native!(PUBLIC | STATIC, "nativeIsHIDAPIEnabled", "()Z", super::native_is_hidapi_enabled[]),
            native!(PUBLIC | STATIC, "nativeInitMainThread", "()V", super::native_init_main_thread[]),
            native!(PUBLIC | STATIC, "nativeCleanupMainThread", "()V", super::native_cleanup_main_thread[]),
            native!(
                PUBLIC | STATIC,
                "nativeRunMain",
                "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/Object;)I",
                super::native_run_main[_, _, _]
            ),
            native!(PUBLIC | STATIC, "nativeLowMemory", "()V", super::native_low_memory[]),
            native!(PUBLIC | STATIC, "nativeSendQuit", "()V", super::native_send_quit[]),
            native!(PUBLIC | STATIC, "nativeQuit", "()V", super::native_quit[]),
            native!(PUBLIC | STATIC, "nativePause", "()V", super::native_pause[]),
            native!(PUBLIC | STATIC, "nativeResume", "()V", super::native_resume[]),
            native!(
                PUBLIC | STATIC,
                "nativeFocusChanged",
                "(Z)V",
                super::native_focus_changed[_]
            ),
            native!(
                PUBLIC | STATIC,
                "onNativeDropFile",
                "(Ljava/lang/String;)V",
                super::on_native_drop_file[_]
            ),
            native!(
                PUBLIC | STATIC,
                "nativeSetScreenResolution",
                "(IIIIFF)V",
                super::native_set_screen_resolution[_, _, _, _, _, _]
            ),
            native!(PUBLIC | STATIC, "onNativeResize", "()V", super::on_native_resize[]),
            native!(
                PUBLIC | STATIC,
                "onNativeKeyDown",
                "(I)V",
                super::on_native_key_down[_]
            ),
            native!(
                PUBLIC | STATIC,
                "onNativeKeyUp",
                "(I)V",
                super::on_native_key_up[_]
            ),
            native!(
                PUBLIC | STATIC,
                "onNativeSoftReturnKey",
                "()Z",
                super::on_native_soft_return_key[]
            ),
            native!(
                PUBLIC | STATIC,
                "onNativeKeyboardFocusLost",
                "()V",
                super::on_native_keyboard_focus_lost[]
            ),
            native!(
                PUBLIC | STATIC,
                "onNativeMouse",
                "(IIFFZ)V",
                super::on_native_mouse[_, _, _, _, _]
            ),
            native!(
                PUBLIC | STATIC,
                "onNativeTouch",
                "(IIIFFF)V",
                super::on_native_touch[_, _, _, _, _, _]
            ),
            native!(
                PUBLIC | STATIC,
                "onNativePen",
                "(IIIIFFF)V",
                super::on_native_pen[_, _, _, _, _, _, _]
            ),
            native!(
                PUBLIC | STATIC,
                "onNativeSurfaceCreated",
                "()V",
                super::on_native_surface_created[]
            ),
            native!(
                PUBLIC | STATIC,
                "onNativeSurfaceChanged",
                "()V",
                super::on_native_surface_changed[]
            ),
            native!(
                PUBLIC | STATIC,
                "onNativeSurfaceDestroyed",
                "()V",
                super::on_native_surface_destroyed[]
            ),
            native!(
                PUBLIC | STATIC,
                "onNativeScreenKeyboardShown",
                "()V",
                super::on_native_screen_keyboard_shown[]
            ),
            native!(
                PUBLIC | STATIC,
                "onNativeScreenKeyboardHidden",
                "()V",
                super::on_native_screen_keyboard_hidden[]
            ),
            native!(
                PUBLIC | STATIC,
                "nativeGetHint",
                "(Ljava/lang/String;)Ljava/lang/String;",
                super::native_get_hint[_]
            ),
            native!(
                PUBLIC | STATIC,
                "nativeGetHintBoolean",
                "(Ljava/lang/String;Z)Z",
                super::native_get_hint_boolean[_, _]
            ),
            native!(
                PUBLIC | STATIC,
                "nativeSetenv",
                "(Ljava/lang/String;Ljava/lang/String;)V",
                super::native_setenv[_, _]
            ),
            native!(
                PUBLIC | STATIC,
                "nativeSetNaturalOrientation",
                "(I)V",
                super::native_set_natural_orientation[_]
            ),
            native!(
                PUBLIC | STATIC,
                "onNativeRotationChanged",
                "(I)V",
                super::on_native_rotation_changed[_]
            ),
            native!(
                PUBLIC | STATIC,
                "onNativeInsetsChanged",
                "(IIII)V",
                super::on_native_insets_changed[_, _, _, _]
            ),
            native!(
                PUBLIC | STATIC,
                "nativeAddTouch",
                "(ILjava/lang/String;)V",
                super::native_add_touch[_, _]
            ),
            native!(
                PUBLIC | STATIC,
                "nativePermissionResult",
                "(IZ)V",
                super::native_permission_result[_, _]
            ),
            native!(PUBLIC | STATIC, "onNativeLocaleChanged", "()V", super::on_native_locale_changed[]),
            native!(
                PUBLIC | STATIC,
                "onNativeDarkModeChanged",
                "(Z)V",
                super::on_native_dark_mode_changed[_]
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
            native!(
                PUBLIC | STATIC,
                "onNativePinchStart",
                "(FFFF)V",
                super::on_native_pinch_start[_, _, _, _]
            ),
            native!(
                PUBLIC | STATIC,
                "onNativePinchUpdate",
                "(FFFFF)V",
                super::on_native_pinch_update[_, _, _, _, _]
            ),
            native!(PUBLIC | STATIC, "onNativePinchEnd", "()V", super::on_native_pinch_end[]),
            // Permissions
            native!(
                PUBLIC,
                "onRequestPermissionsResult",
                "(I[Ljava/lang/String;[I)V",
                activity::on_request_permissions_result[_, _, _]
            ),
            native!(
                PUBLIC,
                "onSystemUiVisibilityChange",
                "(I)V",
                activity::on_system_ui_visibility_change[_]
            ),
        ],
    },
    // SDLActivity.java's `protected static class SDLCommandHandler extends
    // Handler`, the handler of the messages from SDL's thread.
    Class {
        name: SDL_COMMAND_HANDLER,
        access: PUBLIC,
        superclass: "android/os/Handler",
        interfaces: &[],
        fields: &[],
        methods: &[
            constructor(PUBLIC, "()V"),
            native!(
                PUBLIC,
                "handleMessage",
                "(Landroid/os/Message;)V",
                activity::command_handler_handle_message[_]
            ),
        ],
    },
    // SDLActivity.java's `static class ShowTextInputTask implements
    // Runnable`, the text edit shown on the main thread. (Its constructor
    // is `Object`'s: upstream's `(IIIII)V` sets the fields, which
    // `super::java::activity` does.)
    Class {
        name: SDL_SHOW_TEXT_INPUT_TASK,
        access: 0,
        superclass: "java/lang/Object",
        interfaces: &["java/lang/Runnable"],
        fields: &[
            Field {
                name: "input_type",
                descriptor: "I",
                access: PUBLIC,
            },
            Field {
                name: "x",
                descriptor: "I",
                access: PUBLIC,
            },
            Field {
                name: "y",
                descriptor: "I",
                access: PUBLIC,
            },
            Field {
                name: "w",
                descriptor: "I",
                access: PUBLIC,
            },
            Field {
                name: "h",
                descriptor: "I",
                access: PUBLIC,
            },
        ],
        methods: &[
            constructor(PUBLIC, "()V"),
            native!(PUBLIC, "run", "()V", activity::show_text_input_task_run[]),
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
    // SDLSurface.java: the activity's view, what SDL draws on.
    Class {
        name: SDL_SURFACE,
        access: PUBLIC,
        superclass: "android/view/SurfaceView",
        interfaces: &[
            "android/view/SurfaceHolder$Callback",
            "android/view/View$OnApplyWindowInsetsListener",
            "android/view/View$OnKeyListener",
            "android/view/View$OnTouchListener",
            "android/hardware/SensorEventListener",
            "android/view/ScaleGestureDetector$OnScaleGestureListener",
        ],
        fields: &[
            // Sensors
            Field {
                name: "mSensorManager",
                descriptor: "Landroid/hardware/SensorManager;",
                access: PROTECTED,
            },
            Field {
                name: "mDisplay",
                descriptor: "Landroid/view/Display;",
                access: PROTECTED,
            },
            // Keep track of the surface size to normalize touch events
            Field {
                name: "mWidth",
                descriptor: "F",
                access: PROTECTED,
            },
            Field {
                name: "mHeight",
                descriptor: "F",
                access: PROTECTED,
            },
            // Is SurfaceView ready for rendering
            Field {
                name: "mIsSurfaceReady",
                descriptor: "Z",
                access: PROTECTED,
            },
            // Is on-screen keyboard visible
            Field {
                name: "mKeyboardVisible",
                descriptor: "Z",
                access: PROTECTED,
            },
            // Pinch events (private final: set from native code)
            Field {
                name: "scaleGestureDetector",
                descriptor: "Landroid/view/ScaleGestureDetector;",
                access: PRIVATE,
            },
        ],
        methods: &[
            // Startup
            constructor(PROTECTED, "(Landroid/content/Context;)V"),
            // Called when we have a valid drawing surface
            native!(
                PUBLIC,
                "surfaceCreated",
                "(Landroid/view/SurfaceHolder;)V",
                surface::surface_created[_]
            ),
            // Called when we lose the surface
            native!(
                PUBLIC,
                "surfaceDestroyed",
                "(Landroid/view/SurfaceHolder;)V",
                surface::surface_destroyed[_]
            ),
            // Called when the surface is resized
            native!(
                PUBLIC,
                "surfaceChanged",
                "(Landroid/view/SurfaceHolder;III)V",
                surface::surface_changed[_, _, _, _]
            ),
            // Window inset
            native!(
                PUBLIC,
                "onApplyWindowInsets",
                "(Landroid/view/View;Landroid/view/WindowInsets;)Landroid/view/WindowInsets;",
                surface::on_apply_window_insets[_, _]
            ),
            // Key events
            native!(
                PUBLIC,
                "onKey",
                "(Landroid/view/View;ILandroid/view/KeyEvent;)Z",
                surface::on_key[_, _, _]
            ),
            // Touch events
            native!(
                PUBLIC,
                "onTouch",
                "(Landroid/view/View;Landroid/view/MotionEvent;)Z",
                surface::on_touch[_, _]
            ),
            // Sensor events
            native!(
                PUBLIC,
                "onAccuracyChanged",
                "(Landroid/hardware/Sensor;I)V",
                surface::on_accuracy_changed[_, _]
            ),
            native!(
                PUBLIC,
                "onSensorChanged",
                "(Landroid/hardware/SensorEvent;)V",
                surface::on_sensor_changed[_]
            ),
            // Prevent android internal NullPointerException (https://github.com/libsdl-org/SDL/issues/13306)
            native!(
                PUBLIC,
                "onResolvePointerIcon",
                "(Landroid/view/MotionEvent;I)Landroid/view/PointerIcon;",
                surface::on_resolve_pointer_icon[_, _]
            ),
            // Captured pointer events for API 26.
            native!(
                PUBLIC,
                "onCapturedPointerEvent",
                "(Landroid/view/MotionEvent;)Z",
                surface::on_captured_pointer_event[_]
            ),
            native!(
                PUBLIC,
                "onScale",
                "(Landroid/view/ScaleGestureDetector;)Z",
                surface::on_scale[_]
            ),
            native!(
                PUBLIC,
                "onScaleBegin",
                "(Landroid/view/ScaleGestureDetector;)Z",
                surface::on_scale_begin[_]
            ),
            native!(
                PUBLIC,
                "onScaleEnd",
                "(Landroid/view/ScaleGestureDetector;)V",
                surface::on_scale_end[_]
            ),
        ],
    },
    // SDLControllerManager.java's generic motion listeners
    // (super::java::controller_manager), each extending the previous API
    // level's: the surface's generic motion listener.
    Class {
        name: controller_manager_names::API14,
        access: 0,
        superclass: "java/lang/Object",
        interfaces: &["android/view/View$OnGenericMotionListener"],
        fields: &[],
        methods: &[
            constructor(0, "()V"),
            // Generic Motion (mouse hover, joystick...) events go here
            native!(
                PUBLIC,
                "onGenericMotion",
                "(Landroid/view/View;Landroid/view/MotionEvent;)Z",
                controller_manager::on_generic_motion[_, _]
            ),
        ],
    },
    Class {
        name: controller_manager_names::API24,
        access: 0,
        superclass: controller_manager_names::API14,
        interfaces: &[],
        fields: &[Field {
            name: "mRelativeModeEnabled",
            descriptor: "Z",
            access: PRIVATE,
        }],
        methods: &[constructor(0, "()V")],
    },
    Class {
        name: controller_manager_names::API26,
        access: 0,
        superclass: controller_manager_names::API24,
        interfaces: &[],
        fields: &[Field {
            name: "mRelativeModeEnabled",
            descriptor: "Z",
            access: PRIVATE,
        }],
        methods: &[constructor(0, "()V")],
    },
    Class {
        name: controller_manager_names::API29,
        access: 0,
        superclass: controller_manager_names::API26,
        interfaces: &[],
        fields: &[],
        methods: &[constructor(0, "()V")],
    },
    // SDLDummyEdit.java: the text edit, a fake invisible editor view that
    // receives the input.
    Class {
        name: SDL_DUMMY_EDIT,
        access: PUBLIC,
        superclass: "android/view/View",
        interfaces: &["android/view/View$OnKeyListener"],
        fields: &[
            Field {
                name: "ic",
                descriptor: "Landroid/view/inputmethod/InputConnection;",
                access: 0,
            },
            Field {
                name: "input_type",
                descriptor: "I",
                access: 0,
            },
        ],
        methods: &[
            constructor(0, "(Landroid/content/Context;)V"),
            native!(
                PUBLIC,
                "onCheckIsTextEditor",
                "()Z",
                dummy_edit::on_check_is_text_editor[]
            ),
            native!(
                PUBLIC,
                "onKey",
                "(Landroid/view/View;ILandroid/view/KeyEvent;)Z",
                dummy_edit::on_key[_, _, _]
            ),
            native!(
                PUBLIC,
                "onKeyPreIme",
                "(ILandroid/view/KeyEvent;)Z",
                dummy_edit::on_key_pre_ime[_, _]
            ),
            native!(
                PUBLIC,
                "onCreateInputConnection",
                "(Landroid/view/inputmethod/EditorInfo;)Landroid/view/inputmethod/InputConnection;",
                dummy_edit::on_create_input_connection[_]
            ),
        ],
    },
    // SDLInputConnection.java: the text edit's input connection.
    Class {
        name: SDL_INPUT_CONNECTION,
        access: 0,
        superclass: "android/view/inputmethod/BaseInputConnection",
        interfaces: &[],
        fields: &[
            Field {
                name: "mEditText",
                descriptor: "Landroid/widget/EditText;",
                access: PROTECTED,
            },
            Field {
                name: "mCommittedText",
                descriptor: "Ljava/lang/String;",
                access: PROTECTED,
            },
        ],
        methods: &[
            constructor(0, "(Landroid/view/View;Z)V"),
            native!(
                PUBLIC,
                "getEditable",
                "()Landroid/text/Editable;",
                input_connection::get_editable[]
            ),
            native!(
                PUBLIC,
                "sendKeyEvent",
                "(Landroid/view/KeyEvent;)Z",
                input_connection::send_key_event[_]
            ),
            native!(
                PUBLIC,
                "commitText",
                "(Ljava/lang/CharSequence;I)Z",
                input_connection::commit_text[_, _]
            ),
            native!(
                PUBLIC,
                "setComposingText",
                "(Ljava/lang/CharSequence;I)Z",
                input_connection::set_composing_text[_, _]
            ),
            native!(
                PUBLIC,
                "deleteSurroundingText",
                "(II)Z",
                input_connection::delete_surrounding_text[_, _]
            ),
            native!(
                PUBLIC | STATIC,
                "nativeCommitText",
                "(Ljava/lang/String;I)V",
                super::native_commit_text[_, _]
            ),
            native!(
                PUBLIC | STATIC,
                "nativeGenerateScancodeForUnichar",
                "(C)V",
                super::native_generate_scancode_for_unichar[_]
            ),
        ],
    },
    // Upstream's anonymous `Runnable`s (`new Runnable() {...}`): a Rust
    // closure in `ptr`, run once (super::java::anonymous).
    Class {
        name: SDL_NATIVE_RUNNABLE,
        access: FINAL,
        superclass: "java/lang/Object",
        interfaces: &["java/lang/Runnable"],
        fields: &[Field {
            name: "ptr",
            descriptor: "J",
            access: PRIVATE,
        }],
        methods: &[
            constructor(0, "()V"),
            native!(PUBLIC, "run", "()V", anonymous::runnable_run[]),
            native!(PROTECTED, "finalize", "()V", anonymous::runnable_finalize[]),
        ],
    },
    // Upstream's other anonymous classes (listeners, callbacks): proxies of
    // their interfaces, whose invocation handler calls a Rust closure in
    // `ptr` (super::java::anonymous).
    Class {
        name: SDL_NATIVE_LISTENER,
        access: FINAL,
        superclass: "java/lang/Object",
        interfaces: &["java/lang/reflect/InvocationHandler"],
        fields: &[Field {
            name: "ptr",
            descriptor: "J",
            access: PRIVATE,
        }],
        methods: &[
            constructor(0, "()V"),
            native!(
                PUBLIC,
                "invoke",
                "(Ljava/lang/Object;Ljava/lang/reflect/Method;[Ljava/lang/Object;)Ljava/lang/Object;",
                anonymous::listener_invoke[_, _, _]
            ),
            native!(PROTECTED, "finalize", "()V", anonymous::listener_finalize[]),
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
