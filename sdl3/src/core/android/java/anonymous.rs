// Upstream's anonymous Java classes (android-project's
// `new Runnable() {...}`, `new DialogInterface.OnClickListener() {...}`,
// `new OnBackInvokedCallback() {...}`), as two classes of the DEX file
// that run a Rust closure.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The anonymous classes upstream's Java creates are its compiler's
//! (`SDLActivity$1`, ...), with code in their methods, which a DEX file of
//! `native` methods can't have. Here they are two classes of the DEX file
//! (`super::super::classes`) that carry a Rust closure in their `ptr`
//! field: `org.libsdl.app.SDLNativeRunnable implements Runnable` (a
//! closure run once, by `run()`: [`runnable`]), and
//! `org.libsdl.app.SDLNativeListener implements InvocationHandler`, the
//! handler of a `java.lang.reflect.Proxy` implementing an interface (a
//! closure called for each of the interface's methods: [`listener`]).
//! Note: their names and classes differ from upstream's anonymous ones.
//! A closure is dropped when its object is finalized (or, a runnable's,
//! when it has run).

use super::super::classes::{SDL_NATIVE_LISTENER, SDL_NATIVE_RUNNABLE};
use super::super::env::{self, Arg, Env, JniResult, Local, Obj};
use super::super::jni::*;
use std::ffi::CStr;

/// A runnable's closure.
type RunFn = Box<dyn FnOnce(Env<'_>) + Send>;

/// A listener's closure: the method's name and its arguments (an
/// `Object[]`, null for none), its result (null for `void`).
type InvokeFn =
    Box<dyn for<'a> Fn(Env<'a>, &str, Obj<'_>) -> JniResult<'a, Option<Local<'a>>> + Send + Sync>;

/// The object's `ptr` field.
fn ptr(env: Env<'_>, this: Obj<'_>) -> jlong {
    env.get_field(this, c"ptr", c"J")
        .map(env::Value::long)
        .unwrap_or_else(|thrown| {
            thrown.clear();
            0
        })
}

fn set_ptr(env: Env<'_>, this: Obj<'_>, value: jlong) {
    if let Err(thrown) = env.set_field(this, c"ptr", c"J", Arg::Long(value)) {
        thrown.clear();
    }
}

/// A new instance of the DEX file's class `name`, its `ptr` the boxed
/// closure.
fn new_carrier<'a, T>(env: Env<'a>, name: &str, closure: T) -> JniResult<'a, Local<'a>> {
    let class = env::app_class(name).expect("JNI_OnLoad() found the class");
    let object = env.new_object(class.obj(), c"()V", &[])?;
    let raw = Box::into_raw(Box::new(closure));
    set_ptr(env, object.obj(), raw as jlong);
    Ok(object)
}

/// `new Runnable() { public void run() { f(); } }`: an
/// `SDLNativeRunnable` that runs `f` once (on the thread that calls
/// `run()`).
pub(crate) fn runnable<'a>(
    env: Env<'a>,
    f: impl FnOnce(Env<'_>) + Send + 'static,
) -> JniResult<'a, Local<'a>> {
    let f: RunFn = Box::new(f);
    new_carrier(env, SDL_NATIVE_RUNNABLE, f)
}

/// `SDLNativeRunnable.run()`.
pub(crate) extern "system" fn runnable_run(env: *mut JNIEnv, this: jobject) {
    // SAFETY: a native method's environment and object.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    let raw = ptr(env, this);
    if raw == 0 {
        return;
    }
    set_ptr(env, this, 0);
    // SAFETY: runnable()'s box, taken once (the field is cleared).
    let f = unsafe { Box::from_raw(raw as *mut RunFn) };
    f(env);
}

/// `SDLNativeRunnable.finalize()`: the closure, if it never ran, dropped.
pub(crate) extern "system" fn runnable_finalize(env: *mut JNIEnv, this: jobject) {
    // SAFETY: a native method's environment and object.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    let raw = ptr(env, this);
    if raw != 0 {
        set_ptr(env, this, 0);
        // SAFETY: runnable()'s box, which nothing else holds now.
        drop(unsafe { Box::from_raw(raw as *mut RunFn) });
    }
}

/// `new INTERFACE() { ...methods: f(name, args)... }`: a proxy class's
/// instance implementing `interface` (`android/content/DialogInterface$OnClickListener`,
/// whose methods `f` gets by name; `Object`'s `hashCode()`, `equals()` and
/// `toString()` are the proxy's identity's), through an
/// `SDLNativeListener`.
pub(crate) fn listener<'a>(
    env: Env<'a>,
    interface: &CStr,
    f: impl for<'b> Fn(Env<'b>, &str, Obj<'_>) -> JniResult<'b, Option<Local<'b>>>
        + Send
        + Sync
        + 'static,
) -> JniResult<'a, Local<'a>> {
    let f: InvokeFn = Box::new(f);
    let handler = new_carrier(env, SDL_NATIVE_LISTENER, f)?;
    let interface = env.find_class(interface)?;
    let class_class = env.find_class(c"java/lang/Class")?;
    let interfaces = env.new_object_array(1, class_class.obj(), interface.obj())?;
    let loader = env
        .call_method(
            interface.obj(),
            c"getClassLoader",
            c"()Ljava/lang/ClassLoader;",
            &[],
        )?
        .object();
    let proxy = env.find_class(c"java/lang/reflect/Proxy")?;
    let instance = env.call_static_method(
        proxy.obj(),
        c"newProxyInstance",
        c"(Ljava/lang/ClassLoader;[Ljava/lang/Class;Ljava/lang/reflect/InvocationHandler;)Ljava/lang/Object;",
        &[
            Arg::Object(loader.as_ref().map_or(Obj::NULL, Local::obj)),
            Arg::Object(interfaces.obj()),
            Arg::Object(handler.obj()),
        ],
    )?;
    Ok(instance
        .object()
        .expect("Proxy.newProxyInstance() made one"))
}

/// `SDLNativeListener.invoke(proxy, method, args)`.
pub(crate) extern "system" fn listener_invoke(
    env: *mut JNIEnv,
    this: jobject,
    proxy: jobject,
    method: jobject,
    args: jobjectArray,
) -> jobject {
    // SAFETY: a native method's environment and arguments.
    let (env, this, proxy, method, args) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(this),
            Obj::from_raw(proxy),
            Obj::from_raw(method),
            Obj::from_raw(args),
        )
    };
    match invoke(env, this, proxy, method, args) {
        Ok(result) => result.map_or(std::ptr::null_mut(), Local::into_raw),
        // (thrown to the proxy's caller)
        Err(thrown) => {
            drop(thrown);
            std::ptr::null_mut()
        }
    }
}

fn invoke<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    proxy: Obj<'_>,
    method: Obj<'_>,
    args: Obj<'_>,
) -> JniResult<'a, Option<Local<'a>>> {
    let name = env
        .call_method(method, c"getName", c"()Ljava/lang/String;", &[])?
        .object()
        .expect("a method's name");
    let name = env.get_string(name.obj())?;
    let system = env.find_class(c"java/lang/System")?;
    match name.as_str() {
        "hashCode" => {
            let hash = env
                .call_static_method(
                    system.obj(),
                    c"identityHashCode",
                    c"(Ljava/lang/Object;)I",
                    &[Arg::Object(proxy)],
                )?
                .int();
            let integer = env.find_class(c"java/lang/Integer")?;
            env.call_static_method(
                integer.obj(),
                c"valueOf",
                c"(I)Ljava/lang/Integer;",
                &[Arg::Int(hash)],
            )
            .map(env::Value::object)
        }
        "equals" => {
            let other = env.get_object_array_element(args, 0)?;
            let same = env.is_same_object(proxy, other.as_ref().map_or(Obj::NULL, Local::obj));
            let boolean = env.find_class(c"java/lang/Boolean")?;
            env.call_static_method(
                boolean.obj(),
                c"valueOf",
                c"(Z)Ljava/lang/Boolean;",
                &[Arg::Boolean(same)],
            )
            .map(env::Value::object)
        }
        "toString" => {
            let hash = env
                .call_static_method(
                    system.obj(),
                    c"identityHashCode",
                    c"(Ljava/lang/Object;)I",
                    &[Arg::Object(proxy)],
                )?
                .int();
            env.new_string(&format!("SDLNativeListener@{hash:x}"))
                .map(Some)
        }
        _ => {
            let raw = ptr(env, this);
            if raw == 0 {
                return Ok(None);
            }
            // SAFETY: listener()'s box, alive until the object is
            // finalized (which can't be while it's called).
            let f = unsafe { &*(raw as *const InvokeFn) };
            f(env, &name, args)
        }
    }
}

/// `SDLNativeListener.finalize()`: the closure dropped.
pub(crate) extern "system" fn listener_finalize(env: *mut JNIEnv, this: jobject) {
    // SAFETY: a native method's environment and object.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    let raw = ptr(env, this);
    if raw != 0 {
        set_ptr(env, this, 0);
        // SAFETY: listener()'s box, which nothing holds now.
        drop(unsafe { Box::from_raw(raw as *mut InvokeFn) });
    }
}
