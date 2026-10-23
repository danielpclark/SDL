// A minimal UIKit app in Rust through the Objective-C runtime (objc.rs, as
// in sdl3/src/core/apple/objc.rs): an app delegate declared at run time,
// UIApplicationMain, a UIWindow with a root view controller, a CADisplayLink
// calling back N times, and probes of Metal, EAGL, GameController and
// UIScreen. Ends by printing `sdl3-ios-runner: exit N` and calling exit(N).
//
// Arguments: --ticks N (default 120), --exit N (default: the tick count),
// --panic-in-tick, --wait-touch SECONDS (stay up until a touch or timeout),
// --read PATH (repeatable).

#![allow(dead_code, unused_imports, unused_macros, clippy::all)]

#[path = "../objc.rs"]
mod objc;

use objc::*;
use std::ffi::{c_char, c_void, CStr, CString};
use std::io::Write;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

// The runner's marker section (plan §3): does it survive linking?
#[used]
#[link_section = "__DATA,__sdl3_runner"]
static RUNNER: [u8; 4] = *b"app\0";

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CGSize {
    w: f64,
    h: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct UIEdgeInsets {
    top: f64,
    left: f64,
    bottom: f64,
    right: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct MTLClearColor {
    r: f64,
    g: f64,
    b: f64,
    a: f64,
}

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    fn UIApplicationMain(argc: i32, argv: *mut *mut c_char, principal: Id, delegate: Id) -> i32;
}
#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {
    static NSDefaultRunLoopMode: Id;
    static NSRunLoopCommonModes: Id;
    fn NSHomeDirectory() -> Id;
    fn NSTemporaryDirectory() -> Id;
}
#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {}
#[link(name = "Metal", kind = "framework")]
unsafe extern "C" {
    fn MTLCreateSystemDefaultDevice() -> Id;
}
#[link(name = "OpenGLES", kind = "framework")]
unsafe extern "C" {
    fn glGetString(name: u32) -> *const c_char;
    fn glGetError() -> u32;
}
#[link(name = "GameController", kind = "framework")]
unsafe extern "C" {
    static GCInputButtonA: Id;
    static GCInputButtonB: Id;
    static GCInputLeftThumbstick: Id;
    static GCInputRightThumbstick: Id;
    static GCControllerDidConnectNotification: Id;
}
unsafe extern "C" {
    fn exit(code: i32) -> !;
}

struct Opts {
    ticks: usize,
    exit: i32,
    panic_in_tick: bool,
    wait_touch: Option<f64>,
    read: Vec<String>,
}

static OPTS: OnceLock<Opts> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();
static TICKS: AtomicUsize = AtomicUsize::new(0);
static TOUCHED: AtomicBool = AtomicBool::new(false);
static WINDOW: AtomicPtr<Object> = AtomicPtr::new(ptr::null_mut());
static VIEWCTL: AtomicPtr<Object> = AtomicPtr::new(ptr::null_mut());
static VIRTUAL: AtomicPtr<Object> = AtomicPtr::new(ptr::null_mut());
static LINK: AtomicPtr<Object> = AtomicPtr::new(ptr::null_mut());

fn opts() -> &'static Opts {
    OPTS.get().unwrap()
}

fn t() -> f64 {
    START.get().map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0)
}

macro_rules! say {
    ($($a:tt)*) => {{
        println!("[app {:7.3}] {}", t(), format!($($a)*));
    }};
}

fn alloc(cls: Class) -> Id {
    unsafe { msg_send![cls, alloc] }
}

fn main_screen() -> Id {
    unsafe { msg_send![class!(UIScreen), mainScreen] }
}

fn ns_string(s: &str) -> Id {
    let c = CString::new(s).unwrap();
    unsafe { msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()] }
}

fn to_string(s: Id) -> String {
    if s.is_null() {
        return "(nil)".into();
    }
    unsafe {
        let p: *const c_char = msg_send![s, UTF8String];
        if p.is_null() {
            "(null UTF8String)".into()
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

fn desc(o: Id) -> String {
    if o.is_null() {
        return "(nil)".into();
    }
    unsafe { to_string(msg_send![o, description]) }
}

fn finish(code: i32, why: &str) -> ! {
    say!("finishing: {why}");
    println!("sdl3-ios-runner: exit {code}");
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    unsafe { exit(code) }
}

// ---- probes

fn probe_device_and_screen() {
    unsafe {
        let dev: Id = msg_send![class!(UIDevice), currentDevice];
        let idiom: NSInteger = msg_send![dev, userInterfaceIdiom];
        say!(
            "UIDevice: name={} model={} system={} {} idiom={idiom}",
            to_string(msg_send![dev, name]),
            to_string(msg_send![dev, model]),
            to_string(msg_send![dev, systemName]),
            to_string(msg_send![dev, systemVersion]),
        );
        let screen: Id = msg_send![class!(UIScreen), mainScreen];
        let bounds: CGRect = msg_send![screen, bounds];
        let native: CGRect = msg_send![screen, nativeBounds];
        let scale: f64 = msg_send![screen, scale];
        let nscale: f64 = msg_send![screen, nativeScale];
        let fps: NSInteger = msg_send![screen, maximumFramesPerSecond];
        let screens: Id = msg_send![class!(UIScreen), screens];
        let nscreens: NSUInteger = msg_send![screens, count];
        say!("UIScreen: screens={nscreens} bounds={bounds:?} nativeBounds={native:?} scale={scale} nativeScale={nscale} maximumFramesPerSecond={fps}");
        let mode: Id = msg_send![screen, currentMode];
        let msize: CGSize = msg_send![mode, size];
        let modes: Id = msg_send![screen, availableModes];
        let nmodes: NSUInteger = msg_send![modes, count];
        say!("UIScreen: currentMode size={msize:?}; availableModes count={nmodes}");
        for i in 0..nmodes {
            let m: Id = msg_send![modes, objectAtIndex: i];
            let s: CGSize = msg_send![m, size];
            let ar: f64 = msg_send![m, pixelAspectRatio];
            say!("  mode {i}: {s:?} pixelAspectRatio={ar}");
        }
        say!(
            "NSHomeDirectory={} NSTemporaryDirectory={}",
            to_string(NSHomeDirectory()),
            to_string(NSTemporaryDirectory())
        );
        let bundle: Id = msg_send![class!(NSBundle), mainBundle];
        say!(
            "mainBundle path={} id={}",
            to_string(msg_send![bundle, bundlePath]),
            to_string(msg_send![bundle, bundleIdentifier])
        );
    }
    say!("cwd={:?} exe={:?}", std::env::current_dir(), std::env::current_exe());
    say!("args={:?}", std::env::args().collect::<Vec<_>>());
    let mut vars: Vec<_> = std::env::vars().collect();
    vars.sort();
    for (k, v) in vars {
        if k.starts_with("SPIKE") || k.starts_with("SDL") || k.starts_with("RUST") || k == "HOME" || k == "TMPDIR" || k == "PATH" || k.starts_with("SIMULATOR_") || k.starts_with("DYLD") || k.starts_with("CFFIXED") {
            say!("env {k}={v}");
        }
    }
    for p in &opts().read {
        match std::fs::read(p) {
            Ok(b) => say!("read {p}: {} bytes OK", b.len()),
            Err(e) => match std::fs::read_dir(p) {
                Ok(rd) => say!("read {p}: directory with {} entries OK", rd.count()),
                Err(e2) => say!("read {p}: failed: {e} / {e2}"),
            },
        }
    }
}

fn probe_metal() {
    unsafe {
        let dev = MTLCreateSystemDefaultDevice();
        say!("MTLCreateSystemDefaultDevice() = {}", if dev.is_null() { "nil".to_string() } else { to_string(msg_send![dev, name]) });
        if dev.is_null() {
            return;
        }
        let fams: &[(&str, NSInteger)] = &[
            ("Apple1", 1001), ("Apple2", 1002), ("Apple3", 1003), ("Apple4", 1004),
            ("Apple5", 1005), ("Apple6", 1006), ("Apple7", 1007), ("Apple8", 1008),
            ("Apple9", 1009), ("Apple10", 1010),
            ("Mac1", 2001), ("Mac2", 2002),
            ("Common1", 3001), ("Common2", 3002), ("Common3", 3003),
            ("MacCatalyst1", 4001), ("MacCatalyst2", 4002),
            ("Metal3", 5001), ("Metal4", 5002),
        ];
        let mut line = String::new();
        if responds_to(dev, sel!(supportsFamily:)) {
            for (n, v) in fams {
                let b: Bool = msg_send![dev, supportsFamily: *v];
                line += &format!(" {n}={:?}", b);
            }
        } else {
            line = " (no supportsFamily:)".into();
        }
        say!("MTLDevice supportsFamily:{line}");
        let unified: Bool = msg_send![dev, hasUnifiedMemory];
        let maxbuf: NSUInteger = msg_send![dev, maxBufferLength];
        let argbuf: NSUInteger = msg_send![dev, argumentBuffersSupport];
        let rw: NSUInteger = msg_send![dev, readWriteTextureSupport];
        let lin: NSUInteger = msg_send![dev, minimumLinearTextureAlignmentForPixelFormat: 80usize];
        let mut samples = String::new();
        for c in [1usize, 2, 4, 8] {
            let b: Bool = msg_send![dev, supportsTextureSampleCount: c];
            samples += &format!(" {c}={b:?}");
        }
        say!("MTLDevice hasUnifiedMemory={unified:?} maxBufferLength={maxbuf} argumentBuffersSupport={argbuf} readWriteTextureSupport={rw} minLinearTexAlign(BGRA8)={lin} sampleCounts:{samples}");
        if responds_to(dev, sel!(supportsBCTextureCompression)) {
            let bc: Bool = msg_send![dev, supportsBCTextureCompression];
            say!("MTLDevice supportsBCTextureCompression={bc:?}");
        }
        // keep it for the drawable probe
        DEVICE.store(dev, Ordering::Relaxed);
    }
}
static DEVICE: AtomicPtr<Object> = AtomicPtr::new(ptr::null_mut());

fn probe_metal_layer(view: Id) {
    unsafe {
        let dev = DEVICE.load(Ordering::Relaxed);
        if dev.is_null() {
            return;
        }
        let layer: Id = msg_send![class!(CAMetalLayer), layer];
        let _: () = msg_send![layer, setDevice: dev];
        let _: () = msg_send![layer, setPixelFormat: 80usize]; // BGRA8Unorm
        let vb: CGRect = msg_send![view, bounds];
        let _: () = msg_send![layer, setFrame: vb];
        let scale: f64 = msg_send![main_screen(), scale];
        let _: () = msg_send![layer, setDrawableSize: CGSize { w: vb.size.w * scale, h: vb.size.h * scale }];
        let vl: Id = msg_send![view, layer];
        let _: () = msg_send![vl, addSublayer: layer];
        let drawable: Id = msg_send![layer, nextDrawable];
        say!("CAMetalLayer nextDrawable = {}", desc(drawable));
        if drawable.is_null() {
            return;
        }
        let tex: Id = msg_send![drawable, texture];
        let w: NSUInteger = msg_send![tex, width];
        let h: NSUInteger = msg_send![tex, height];
        let queue: Id = msg_send![dev, newCommandQueue];
        let rpd: Id = msg_send![class!(MTLRenderPassDescriptor), renderPassDescriptor];
        let atts: Id = msg_send![rpd, colorAttachments];
        let a0: Id = msg_send![atts, objectAtIndexedSubscript: 0usize];
        let _: () = msg_send![a0, setTexture: tex];
        let _: () = msg_send![a0, setLoadAction: 2usize];
        let _: () = msg_send![a0, setStoreAction: 1usize];
        let _: () = msg_send![a0, setClearColor: MTLClearColor { r: 0.2, g: 0.4, b: 0.8, a: 1.0 }];
        let cb: Id = msg_send![queue, commandBuffer];
        let enc: Id = msg_send![cb, renderCommandEncoderWithDescriptor: rpd];
        let _: () = msg_send![enc, endEncoding];
        let _: () = msg_send![cb, presentDrawable: drawable];
        let _: () = msg_send![cb, commit];
        let _: () = msg_send![cb, waitUntilCompleted];
        let status: NSUInteger = msg_send![cb, status];
        let err: Id = msg_send![cb, error];
        say!("Metal clear+present on {w}x{h} drawable: status={status} (4=completed) error={}", desc(err));
    }
}

fn probe_eagl() {
    unsafe {
        let cls = class!(EAGLContext);
        say!("EAGLContext class = {:?}", cls);
        if cls.is_null() {
            return;
        }
        for api in [1usize, 2, 3] {
            let a: Id = msg_send![cls, alloc];
            let ctx: Id = msg_send![a, initWithAPI: api];
            if ctx.is_null() {
                say!("EAGL initWithAPI:{api} = nil");
                continue;
            }
            let ok: Bool = msg_send![cls, setCurrentContext: ctx];
            let s = |n: u32| {
                let p = glGetString(n);
                if p.is_null() { "(null)".to_string() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
            };
            say!(
                "EAGL initWithAPI:{api} = ok, setCurrentContext={ok:?}; GL_VERSION={:?} GL_RENDERER={:?} GL_VENDOR={:?} GLSL={:?} err={:#x}",
                s(0x1F02), s(0x1F01), s(0x1F00), if api >= 2 { s(0x8B8C) } else { String::new() }, glGetError()
            );
            let _: Bool = msg_send![cls, setCurrentContext: NIL];
            release(ctx);
        }
    }
}

fn probe_virtual_controller_start() {
    unsafe {
        let gcv = class!(GCVirtualController);
        let gcc = class!(GCController);
        let ctls: Id = msg_send![gcc, controllers];
        let n: NSUInteger = msg_send![ctls, count];
        say!("GCVirtualController class = {:?}; GCController.controllers before = {n}", gcv);
        if gcv.is_null() {
            return;
        }
        let cfg: Id = msg_send![alloc(class!(GCVirtualControllerConfiguration)), init];
        let elems = [GCInputButtonA, GCInputButtonB, GCInputLeftThumbstick, GCInputRightThumbstick];
        let arr: Id = msg_send![class!(NSArray), arrayWithObjects: elems.as_ptr() count: elems.len()];
        let set: Id = msg_send![class!(NSSet), setWithArray: arr];
        let _: () = msg_send![cfg, setElements: set];
        let vc: Id = msg_send![alloc(gcv), initWithConfiguration: cfg];
        say!("GCVirtualController initWithConfiguration = {}", desc(vc));
        if vc.is_null() {
            return;
        }
        VIRTUAL.store(vc, Ordering::Relaxed);
        let nil: Id = NIL;
        let _: () = msg_send![vc, connectWithReplyHandler: nil];
        say!("GCVirtualController connectWithReplyHandler: sent");
    }
}

fn probe_virtual_controller_poll(tag: &str) {
    unsafe {
        let ctls: Id = msg_send![class!(GCController), controllers];
        let n: NSUInteger = msg_send![ctls, count];
        let vc = VIRTUAL.load(Ordering::Relaxed);
        let c: Id = if vc.is_null() { NIL } else { msg_send![vc, controller] };
        let mut extra = String::new();
        if !c.is_null() {
            let eg: Id = msg_send![c, extendedGamepad];
            extra = format!(
                " vendorName={} productCategory={} extendedGamepad={}",
                to_string(msg_send![c, vendorName]),
                to_string(msg_send![c, productCategory]),
                if eg.is_null() { "nil" } else { "present" }
            );
        }
        say!("GameController [{tag}]: controllers.count={n} virtual.controller={}{extra}", if c.is_null() { "nil" } else { "present" });
    }
}

fn probe_safe_area(tag: &str) {
    unsafe {
        let w = WINDOW.load(Ordering::Relaxed);
        let vc = VIEWCTL.load(Ordering::Relaxed);
        let wi: UIEdgeInsets = msg_send![w, safeAreaInsets];
        let wf: CGRect = msg_send![w, frame];
        let view: Id = msg_send![vc, view];
        let vi: UIEdgeInsets = msg_send![view, safeAreaInsets];
        let vf: CGRect = msg_send![view, frame];
        let scene: Id = msg_send![w, windowScene];
        let orient: NSInteger = if scene.is_null() { -1 } else { msg_send![scene, interfaceOrientation] };
        let key: Bool = msg_send![w, isKeyWindow];
        say!("[{tag}] window frame={wf:?} safeAreaInsets={wi:?} key={key:?}; view frame={vf:?} safeAreaInsets={vi:?}; windowScene={} interfaceOrientation={orient}", desc(scene));
    }
}

// ---- the delegate and view

extern "C" fn did_finish_launching(this: Id, _sel: Sel, app: Id, _opts: Id) -> Bool {
    say!("application:didFinishLaunchingWithOptions: (delegate {})", desc(this));
    probe_device_and_screen();
    unsafe {
        let screen: Id = msg_send![class!(UIScreen), mainScreen];
        let bounds: CGRect = msg_send![screen, bounds];
        let w: Id = msg_send![alloc(class!(UIWindow)), initWithFrame: bounds];
        let vc: Id = msg_send![alloc(class!(UIViewController)), init];
        let view: Id = msg_send![alloc(view_class()), initWithFrame: bounds];
        let bg: Id = msg_send![class!(UIColor), systemTealColor];
        let _: () = msg_send![view, setBackgroundColor: bg];
        let _: () = msg_send![vc, setView: view];
        let _: () = msg_send![w, setRootViewController: vc];
        let _: () = msg_send![w, makeKeyAndVisible];
        WINDOW.store(w, Ordering::Relaxed);
        VIEWCTL.store(vc, Ordering::Relaxed);
        let state: NSInteger = msg_send![app, applicationState];
        say!("window made key and visible; applicationState={state}");
        probe_safe_area("didFinishLaunching");
        probe_metal();
        probe_eagl();
        probe_virtual_controller_start();
        let link: Id = msg_send![class!(CADisplayLink), displayLinkWithTarget: this selector: sel!(tick:)];
        let rl: Id = msg_send![class!(NSRunLoop), mainRunLoop];
        let _: () = msg_send![link, addToRunLoop: rl forMode: NSRunLoopCommonModes];
        LINK.store(link, Ordering::Relaxed);
        say!("CADisplayLink added: {}", desc(link));
    }
    YES
}

extern "C" fn tick(_this: Id, _sel: Sel, link: Id) {
    let n = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    let o = opts();
    if n == 1 || n == 2 || n % 30 == 0 || n == o.ticks {
        unsafe {
            let ts: f64 = msg_send![link, timestamp];
            let dur: f64 = msg_send![link, duration];
            let tt: f64 = msg_send![link, targetTimestamp];
            let pfps: NSInteger = msg_send![link, preferredFramesPerSecond];
            say!("tick {n}: timestamp={ts:.4} duration={dur:.5} targetTimestamp={tt:.4} preferredFramesPerSecond={pfps}");
        }
    }
    if n == 2 {
        probe_safe_area("tick 2");
        unsafe {
            let vc = VIEWCTL.load(Ordering::Relaxed);
            probe_metal_layer(msg_send![vc, view]);
        }
    }
    if n == 30 || n == 90 || n == o.ticks {
        probe_virtual_controller_poll(&format!("tick {n}"));
    }
    if o.panic_in_tick && n == 3 {
        panic!("panicking inside the CADisplayLink callback on purpose");
    }
    if let Some(secs) = o.wait_touch {
        if TOUCHED.load(Ordering::Relaxed) {
            finish(o.exit, "touched");
        }
        if t() > secs {
            finish(o.exit + 100, "wait-touch timed out without a touch");
        }
        return;
    }
    if n >= o.ticks {
        unsafe {
            let _: () = msg_send![link, invalidate];
        }
        finish(o.exit, &format!("{n} ticks"));
    }
}

extern "C" fn lifecycle(_this: Id, sel: Sel, _app: Id) {
    say!("lifecycle: {}", sel.name());
}

extern "C" fn touches_began(this: Id, _sel: Sel, touches: Id, _event: Id) {
    unsafe {
        let touch: Id = msg_send![touches, anyObject];
        let p: CGPoint = msg_send![touch, locationInView: this];
        let n: NSUInteger = msg_send![touches, count];
        say!("touchesBegan: count={n} location={p:?}");
    }
    TOUCHED.store(true, Ordering::Relaxed);
}

extern "C" fn touches_ended(this: Id, _sel: Sel, touches: Id, _event: Id) {
    unsafe {
        let touch: Id = msg_send![touches, anyObject];
        let p: CGPoint = msg_send![touch, locationInView: this];
        say!("touchesEnded: location={p:?}");
    }
}

extern "C" fn presses_began(_this: Id, _sel: Sel, presses: Id, _event: Id) {
    unsafe {
        let press: Id = msg_send![presses, anyObject];
        let key: Id = msg_send![press, key];
        let code: NSInteger = if key.is_null() { -1 } else { msg_send![key, keyCode] };
        say!("pressesBegan: keyCode={code}");
    }
    TOUCHED.store(true, Ordering::Relaxed);
}

static VIEW_CLASS: DeclaredClass = DeclaredClass::new();
fn view_class() -> Class {
    VIEW_CLASS.get(|| {
        let mut b = ClassBuilder::new(c"SpikeView", class!(UIView)).unwrap();
        unsafe {
            b.add_method(sel!(touchesBegan:withEvent:), imp!(touches_began as extern "C" fn(Id, Sel, Id, Id)), c"v@:@@");
            b.add_method(sel!(touchesEnded:withEvent:), imp!(touches_ended as extern "C" fn(Id, Sel, Id, Id)), c"v@:@@");
            b.add_method(sel!(pressesBegan:withEvent:), imp!(presses_began as extern "C" fn(Id, Sel, Id, Id)), c"v@:@@");
        }
        b.register()
    })
}

fn declare_delegate() -> Class {
    let mut b = ClassBuilder::new(c"SpikeAppDelegate", class!(UIResponder)).unwrap();
    b.add_protocol(c"UIApplicationDelegate");
    unsafe {
        b.add_method(
            sel!(application:didFinishLaunchingWithOptions:),
            imp!(did_finish_launching as extern "C" fn(Id, Sel, Id, Id) -> Bool),
            c"B@:@@",
        );
        b.add_method(sel!(tick:), imp!(tick as extern "C" fn(Id, Sel, Id)), c"v@:@");
        for s in [
            sel!(applicationDidBecomeActive:),
            sel!(applicationWillResignActive:),
            sel!(applicationDidEnterBackground:),
            sel!(applicationWillEnterForeground:),
            sel!(applicationWillTerminate:),
        ] {
            b.add_method(s, imp!(lifecycle as extern "C" fn(Id, Sel, Id)), c"v@:@");
        }
    }
    b.register()
}

fn main() {
    START.set(Instant::now()).ok();
    let argv_s: Vec<String> = std::env::args().collect();
    let mut o = Opts { ticks: 120, exit: -1, panic_in_tick: false, wait_touch: None, read: vec![] };
    let mut i = 1;
    while i < argv_s.len() {
        match argv_s[i].as_str() {
            "--ticks" => { i += 1; o.ticks = argv_s[i].parse().unwrap(); }
            "--exit" => { i += 1; o.exit = argv_s[i].parse().unwrap(); }
            "--panic-in-tick" => o.panic_in_tick = true,
            "--wait-touch" => { i += 1; o.wait_touch = Some(argv_s[i].parse().unwrap()); }
            "--read" => { i += 1; o.read.push(argv_s[i].clone()); }
            _ => {}
        }
        i += 1;
    }
    if o.exit < 0 {
        o.exit = o.ticks as i32;
    }
    OPTS.set(o).ok();
    say!("main: before UIApplicationMain (stdout)");
    eprintln!("[app] main: a line on stderr");
    let cargs: Vec<CString> = argv_s.iter().map(|a| CString::new(a.as_str()).unwrap()).collect();
    let mut argv: Vec<*mut c_char> = cargs.iter().map(|a| a.as_ptr() as *mut c_char).collect();
    argv.push(ptr::null_mut());
    let cls = declare_delegate();
    say!("declared {}", class_name(cls));
    let r = autoreleasepool(|| unsafe {
        UIApplicationMain(cargs.len() as i32, argv.as_mut_ptr(), NIL, ns_string("SpikeAppDelegate"))
    });
    say!("UIApplicationMain returned {r} (unexpected)");
    finish(99, "UIApplicationMain returned");
}
