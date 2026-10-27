// What the UIKit test programs share.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The UIKit test programs run as apps, through `tools/ios_run.py`'s app
//! mode (each marks itself with a `__DATA,__sdl3_runner` section holding
//! `app`). simctl doesn't give an app's exit status, so a program ends
//! with [`exit`], which prints the line the runner takes it from, flushes
//! and exits. [`request`] asks the runner for something done from outside
//! the app (`background`: another app comes to the foreground;
//! `foreground`: this one comes back; `tap X Y` and `swipe X1 Y1 X2 Y2
//! SECONDS`: a finger touches the screen there; `key CODE` and `text
//! TEXT`: the hardware keyboard presses a key or types; through idb, when
//! the runner drives it, see [`idb`]).
//!
//! A panic in a callback from Objective-C (the main function, which
//! `run_app()` calls from the app delegate, or a main callback) aborts the
//! process, so the programs run their checks in [`checked`] and end with
//! the status it gives. The few Objective-C messages the programs send
//! themselves (posting notifications, asking UIKit how the app is), the
//! classes they declare (stand-ins for UIKit's and GameController's
//! objects) and the blocks they call go through [`objc`].

#![allow(dead_code)] // (each program uses what it needs)

use std::io::Write;

/// The runner's exit line: `sdl3-ios-runner: exit STATUS`, flushed, then
/// `exit(STATUS)`.
pub fn exit(status: i32) -> ! {
    print_exit_line(status);
    std::process::exit(status)
}

/// Print the runner's exit line (and flush what was printed), for a
/// program that is exiting already (from an `atexit()` handler).
pub fn print_exit_line(status: i32) {
    println!("sdl3-ios-runner: exit {status}");
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
}

/// Ask the runner for something (`sdl3-ios-runner: request WHAT`).
pub fn request(what: &str) {
    println!("sdl3-ios-runner: request {what}");
    let _ = std::io::stdout().flush();
}

/// Whether the runner drives idb (`Ok`), so that `tap` and `swipe`
/// requests touch the screen (and `key` and `text` ones type), or why
/// not: the runner tells the app in
/// `SDL3_IOS_RUNNER_IDB` (`1`, or the reason). When `SDL3_TEST_REQUIRE`
/// names `idb` (or `all`), its absence is a failure.
pub fn idb() -> Result<(), String> {
    let why = match std::env::var("SDL3_IOS_RUNNER_IDB") {
        Ok(v) if v == "1" => return Ok(()),
        Ok(v) => v,
        Err(_) => "the runner doesn't drive idb".to_string(),
    };
    let required = std::env::var("SDL3_TEST_REQUIRE").unwrap_or_default();
    if required
        .split(',')
        .map(str::trim)
        .any(|c| c == "idb" || c == "all")
    {
        panic!("required capability idb unavailable: {why}");
    }
    Err(why)
}

/// Run `checks`, catching a panic (printed by the panic hook): the exit
/// status for the runner, 0 if they passed, 101 (as libtest's) if not.
pub fn checked(checks: impl FnOnce()) -> i32 {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(checks)) {
        Ok(()) => 0,
        Err(_) => 101,
    }
}

/// An event type as the traces name it: its constant's name without
/// `SDL_EVENT_`.
pub fn event_name(ty: sdl3::events::EventType) -> String {
    format!("{ty:?}")
        .trim_start_matches("SDL_EVENT_")
        .to_string()
}

/// Seconds since the program started (for the progress lines, which
/// aren't in the traces).
pub fn elapsed() -> f64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f64()
}

/// Compare a trace with upstream's (both printed, with the first line
/// that differs).
pub fn check_trace(trace: &[String], expected: &str) {
    let expected: Vec<&str> = expected.lines().collect();
    println!("--- trace ({} lines)", trace.len());
    for line in trace {
        println!("  {line}");
    }
    let first_difference = (0..trace.len().max(expected.len()))
        .find(|&i| trace.get(i).map(String::as_str) != expected.get(i).copied());
    if let Some(i) = first_difference {
        panic!(
            "the trace differs from upstream's at line {}: {:?}, expected {:?}",
            i + 1,
            trace.get(i),
            expected.get(i)
        );
    }
    println!("--- the trace matches upstream's");
}

/// The bits of the Objective-C runtime, Foundation, UIKit and GCD the
/// programs use.
pub mod objc {
    use std::ffi::{c_char, c_void, CStr, CString};

    pub type Id = *mut c_void;
    pub type Sel = *mut c_void;

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
        fn object_getClassName(obj: Id) -> *const c_char;
    }

    #[link(name = "UIKit", kind = "framework")]
    unsafe extern "C" {
        pub static UIApplicationDidBecomeActiveNotification: Id;
        pub static UIApplicationWillResignActiveNotification: Id;
        pub static UIApplicationDidEnterBackgroundNotification: Id;
        pub static UIApplicationWillEnterForegroundNotification: Id;
        pub static UIApplicationWillTerminateNotification: Id;
        pub static UIApplicationDidReceiveMemoryWarningNotification: Id;
        pub static UIApplicationDidChangeStatusBarOrientationNotification: Id;
        pub static UIScreenDidConnectNotification: Id;
        pub static UIScreenDidDisconnectNotification: Id;
    }

    unsafe extern "C" {
        static _dispatch_main_q: c_void;
        fn dispatch_time(when: u64, delta: i64) -> u64;
        fn dispatch_after_f(
            when: u64,
            queue: *const c_void,
            context: *mut c_void,
            work: unsafe extern "C" fn(*mut c_void),
        );
        fn dispatch_async_f(
            queue: *const c_void,
            context: *mut c_void,
            work: unsafe extern "C" fn(*mut c_void),
        );
    }

    /// A class by name.
    pub fn class(name: &str) -> Id {
        let name = CString::new(name).unwrap();
        // SAFETY: a NUL-terminated name.
        unsafe { objc_getClass(name.as_ptr()) }
    }

    /// A selector by name.
    pub fn sel(name: &str) -> Sel {
        let name = CString::new(name).unwrap();
        // SAFETY: as above.
        unsafe { sel_registerName(name.as_ptr()) }
    }

    /// `[obj selector]`.
    ///
    /// # Safety
    ///
    /// `R` must be the method's return type.
    pub unsafe fn send0<R>(obj: Id, selector: &str) -> R {
        // SAFETY: objc_msgSend with the method's signature.
        unsafe {
            let f: unsafe extern "C" fn(Id, Sel) -> R =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            f(obj, sel(selector))
        }
    }

    /// `[obj selector:a]`.
    ///
    /// # Safety
    ///
    /// `A` and `R` must be the method's argument and return types.
    pub unsafe fn send1<A, R>(obj: Id, selector: &str, a: A) -> R {
        // SAFETY: as above.
        unsafe {
            let f: unsafe extern "C" fn(Id, Sel, A) -> R =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            f(obj, sel(selector), a)
        }
    }

    /// `[obj selector:a b:b]`.
    ///
    /// # Safety
    ///
    /// As [`send1`].
    pub unsafe fn send2<A, B, R>(obj: Id, selector: &str, a: A, b: B) -> R {
        // SAFETY: as above.
        unsafe {
            let f: unsafe extern "C" fn(Id, Sel, A, B) -> R =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            f(obj, sel(selector), a, b)
        }
    }

    /// `[obj selector:a b:b c:c]`.
    ///
    /// # Safety
    ///
    /// As [`send1`].
    pub unsafe fn send3<A, B, C, R>(obj: Id, selector: &str, a: A, b: B, c: C) -> R {
        // SAFETY: as above.
        unsafe {
            let f: unsafe extern "C" fn(Id, Sel, A, B, C) -> R =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            f(obj, sel(selector), a, b, c)
        }
    }

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_allocateClassPair(superclass: Id, name: *const c_char, extra: usize) -> Id;
        fn objc_registerClassPair(cls: Id);
        fn class_addMethod(cls: Id, name: Sel, imp: *const c_void, types: *const c_char) -> bool;
        fn objc_getProtocol(name: *const c_char) -> Id;
        fn class_conformsToProtocol(cls: Id, protocol: Id) -> bool;
        fn object_getClass(obj: Id) -> Id;
    }

    unsafe extern "C" {
        fn _Block_copy(block: *const c_void) -> *mut c_void;
        fn _Block_release(block: *const c_void);
    }

    /// Declare a class (a subclass of `superclass`) with these methods
    /// (selector, implementation, type encoding).
    ///
    /// # Safety
    ///
    /// Each implementation must be an `extern "C"` function of its
    /// encoding's signature.
    pub unsafe fn declare_class(
        name: &str,
        superclass: &str,
        methods: &[(&str, *const c_void, &str)],
    ) -> Id {
        let cname = CString::new(name).unwrap();
        // SAFETY: the caller's contract; a class pair registered once.
        unsafe {
            let cls = objc_allocateClassPair(class(superclass), cname.as_ptr(), 0);
            assert!(!cls.is_null(), "the class {name} exists already");
            for &(selector, imp, types) in methods {
                let types = CString::new(types).unwrap();
                assert!(class_addMethod(cls, sel(selector), imp, types.as_ptr()));
            }
            objc_registerClassPair(cls);
            cls
        }
    }

    /// Whether an object's class conforms to a protocol (false if the
    /// runtime doesn't know the protocol).
    pub fn conforms_to(obj: Id, protocol: &str) -> bool {
        let name = CString::new(protocol).unwrap();
        // SAFETY: a valid object and a NUL-terminated name.
        unsafe {
            let protocol = objc_getProtocol(name.as_ptr());
            !protocol.is_null() && class_conformsToProtocol(object_getClass(obj), protocol)
        }
    }

    /// A copy of a block (`[block copy]`), nil for nil.
    pub fn block_copy(block: Id) -> Id {
        if block.is_null() {
            return block;
        }
        // SAFETY: a block.
        unsafe { _Block_copy(block) }
    }

    /// Release a block copied with [`block_copy`] (nothing for nil).
    pub fn block_release(block: Id) {
        if !block.is_null() {
            // SAFETY: a block copied with block_copy().
            unsafe { _Block_release(block) }
        }
    }

    /// The start of a block (its invoke function follows `isa`, the flags
    /// and a reserved word).
    #[repr(C)]
    struct BlockLiteral {
        isa: *const c_void,
        flags: i32,
        reserved: i32,
        invoke: *const c_void,
    }

    /// Call a block of three arguments, `^(A, B, C)`.
    ///
    /// # Safety
    ///
    /// `block` must be a block of that signature, returning nothing.
    pub unsafe fn call_block3<A, B, C>(block: Id, a: A, b: B, c: C) {
        // SAFETY: the caller's contract; a block's invoke function takes
        // the block, then the arguments.
        unsafe {
            let invoke = (*(block as *const BlockLiteral)).invoke;
            let f: unsafe extern "C" fn(Id, A, B, C) = std::mem::transmute(invoke);
            f(block, a, b, c)
        }
    }

    /// Call a block of four arguments, `^(A, B, C, D)`.
    ///
    /// # Safety
    ///
    /// As [`call_block3`].
    pub unsafe fn call_block4<A, B, C, D>(block: Id, a: A, b: B, c: C, d: D) {
        // SAFETY: as in call_block3().
        unsafe {
            let invoke = (*(block as *const BlockLiteral)).invoke;
            let f: unsafe extern "C" fn(Id, A, B, C, D) = std::mem::transmute(invoke);
            f(block, a, b, c, d)
        }
    }

    /// The name of a selector.
    pub fn sel_name(selector: Sel) -> String {
        #[link(name = "objc")]
        unsafe extern "C" {
            fn sel_getName(sel: Sel) -> *const c_char;
        }
        // SAFETY: a selector's name, a C string the runtime keeps.
        unsafe { CStr::from_ptr(sel_getName(selector)) }
            .to_string_lossy()
            .into_owned()
    }

    /// The name of the superclass of an object's class.
    pub fn superclass_name(obj: Id) -> String {
        #[link(name = "objc")]
        unsafe extern "C" {
            fn class_getSuperclass(cls: Id) -> Id;
            fn class_getName(cls: Id) -> *const c_char;
        }
        if obj.is_null() {
            return "nil".into();
        }
        // SAFETY: a valid object; class names are C strings the runtime
        // keeps.
        unsafe {
            let cls = class_getSuperclass(object_getClass(obj));
            CStr::from_ptr(class_getName(cls))
                .to_string_lossy()
                .into_owned()
        }
    }

    /// An `NSString` (autoreleased; the programs run in the app's pools).
    pub fn ns_string(s: &str) -> Id {
        let s = CString::new(s).unwrap();
        // SAFETY: NSString's class method taking a C string.
        unsafe { send1(class("NSString"), "stringWithUTF8String:", s.as_ptr()) }
    }

    /// An `NSString`'s contents (`None` for nil).
    pub fn string(s: Id) -> Option<String> {
        if s.is_null() {
            return None;
        }
        // SAFETY: an NSString's UTF-8 buffer, copied at once.
        unsafe {
            let p: *const c_char = send0(s, "UTF8String");
            Some(CStr::from_ptr(p).to_string_lossy().into_owned())
        }
    }

    /// The name of an object's class.
    pub fn class_name(obj: Id) -> String {
        if obj.is_null() {
            return "nil".into();
        }
        // SAFETY: a valid object.
        unsafe { CStr::from_ptr(object_getClassName(obj)) }
            .to_string_lossy()
            .into_owned()
    }

    /// `[UIApplication sharedApplication]`.
    pub fn shared_application() -> Id {
        // SAFETY: UIApplication's class method.
        unsafe { send0(class("UIApplication"), "sharedApplication") }
    }

    /// `[[NSNotificationCenter defaultCenter] postNotificationName:name
    /// object:[UIApplication sharedApplication]]`, as UIKit posts its own.
    pub fn post_notification(name: Id) {
        post_notification_for(name, shared_application());
    }

    /// `[[NSNotificationCenter defaultCenter] postNotificationName:name
    /// object:object]`.
    pub fn post_notification_for(name: Id, object: Id) {
        // SAFETY: NSNotificationCenter's methods.
        unsafe {
            let center: Id = send0(class("NSNotificationCenter"), "defaultCenter");
            let () = send2(center, "postNotificationName:object:", name, object);
        }
    }

    unsafe extern "C" fn run_boxed(context: *mut c_void) {
        // SAFETY: the box made by after() or on_main_queue().
        let f = unsafe { Box::from_raw(context as *mut Box<dyn FnOnce()>) };
        f();
    }

    /// Run `f` on the main queue (from the main run loop) after `seconds`.
    pub fn after(seconds: f64, f: impl FnOnce() + 'static) {
        let f: Box<Box<dyn FnOnce()>> = Box::new(Box::new(f));
        // SAFETY: the main queue; the box is taken back by run_boxed.
        unsafe {
            let when = dispatch_time(0, (seconds * 1e9) as i64);
            dispatch_after_f(
                when,
                &_dispatch_main_q,
                Box::into_raw(f) as *mut c_void,
                run_boxed,
            );
        }
    }

    /// Run `f` on the main queue, the next time the main run loop runs.
    pub fn on_main_queue(f: impl FnOnce() + 'static) {
        let f: Box<Box<dyn FnOnce()>> = Box::new(Box::new(f));
        // SAFETY: as above.
        unsafe {
            dispatch_async_f(
                &_dispatch_main_q,
                Box::into_raw(f) as *mut c_void,
                run_boxed,
            );
        }
    }
}
