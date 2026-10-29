// The spike app: this program, linked with -shared by link.sh (and
// --defsym=SDL_main=main), is the APK's lib/<abi>/libmain.so. dexgen.py's
// SDLActivity.<clinit> loads it; JNI_OnLoad registers the native methods
// of the DEX classes; the native onCreate calls super.onCreate through
// CallNonvirtualVoidMethod, connects stdout/stderr to the host's
// `adb reverse` sockets, and runs the probe the intent's `mode` names on a
// worker thread, ending with the runner's sentinel and exit().

use android_spike::jni::*;
use android_spike::{audio, camera, dl, gfx, logcat, request, request_bg, say, sensors, TO_LOGCAT};
use std::ffi::{c_char, c_int, c_void};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering::SeqCst};
use std::time::{Duration, Instant};

#[used]
#[link_section = ".sdl3_runner"]
static RUNNER_WORDS: [u8; 4] = *b"app\0";

const DEX_LEN: usize = include_bytes!("../../gen/classes.dex").len();
#[used]
#[link_section = ".sdl3_android_dex"]
static CLASSES_DEX: [u8; DEX_LEN] = *include_bytes!("../../gen/classes.dex");
static INMEM_DEX: &[u8] = include_bytes!("../../gen/inmem.dex");

static VM: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static ACTIVITY: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static RUNNABLE_CLS: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static WINDOW: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static FOCUSED: AtomicBool = AtomicBool::new(false);
static RUNNABLE_RAN: AtomicBool = AtomicBool::new(false);
static INMEM_RAN: AtomicBool = AtomicBool::new(false);
static TOUCHES: AtomicI32 = AtomicI32::new(0);
static KEYS: AtomicI32 = AtomicI32::new(0);
static MOTIONS: AtomicI32 = AtomicI32::new(0);
static RESUMES: AtomicI32 = AtomicI32::new(0);
static PAUSES: AtomicI32 = AtomicI32::new(0);

#[link(name = "android")]
unsafe extern "C" {
    fn ANativeWindow_fromSurface(env: *mut c_void, surface: jobject) -> *mut c_void;
    fn ANativeWindow_getWidth(w: *mut c_void) -> i32;
    fn ANativeWindow_getHeight(w: *mut c_void) -> i32;
}

unsafe extern "C" {
    fn dup2(a: c_int, b: c_int) -> c_int;
    fn write(fd: c_int, b: *const c_void, n: usize) -> isize;
    fn _exit(code: c_int) -> !;
    fn exit(code: c_int) -> !;
    fn gettid() -> c_int;
    fn getpid() -> c_int;
    fn getuid() -> u32;
}

fn thread_desc() -> String {
    let tid = unsafe { gettid() };
    let comm = std::fs::read_to_string(format!("/proc/self/task/{tid}/comm")).unwrap_or_default();
    format!("tid={tid} ({}) pid={}", comm.trim(), unsafe { getpid() })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    say!("spike_app main() reached (via SDL_main when in libmain.so): args={args:?} thread {}", thread_desc());
}

fn vm() -> JavaVM {
    VM.load(SeqCst) as JavaVM
}
fn activity() -> jobject {
    ACTIVITY.load(SeqCst)
}

macro_rules! native {
    ($name:literal, $sig:literal, $f:expr) => {
        NativeMethod { name: concat!($name, "\0").as_ptr().cast(), sig: concat!($sig, "\0").as_ptr().cast(), f: $f as *mut c_void }
    };
}

#[no_mangle]
pub extern "system" fn JNI_OnLoad(vm: JavaVM, _reserved: *mut c_void) -> i32 {
    TO_LOGCAT.store(true, SeqCst);
    VM.store(vm as *mut c_void, SeqCst);
    let e = env_for(vm);
    // (the DEX bytes are referenced here, as the plan's fallback would)
    let dex = std::hint::black_box(CLASSES_DEX.as_ptr());
    logcat(&format!("JNI_OnLoad: vm={vm:?} thread {} embedded classes.dex {} bytes at {dex:?}", thread_desc(), CLASSES_DEX.len()));
    let act = e.find_class("org/libsdl/app/SDLActivity");
    let r1 = e.register_natives(act, &[
        native!("onCreate", "(Landroid/os/Bundle;)V", on_create as extern "system" fn(_, _, _)),
        native!("onResume", "()V", on_resume as extern "system" fn(_, _)),
        native!("onPause", "()V", on_pause as extern "system" fn(_, _)),
        native!("onDestroy", "()V", on_destroy as extern "system" fn(_, _)),
        native!("onWindowFocusChanged", "(Z)V", on_focus as extern "system" fn(_, _, _)),
        native!("dispatchTouchEvent", "(Landroid/view/MotionEvent;)Z", dispatch_touch as extern "system" fn(_, _, _) -> _),
        native!("dispatchKeyEvent", "(Landroid/view/KeyEvent;)Z", dispatch_key as extern "system" fn(_, _, _) -> _),
        native!("dispatchGenericMotionEvent", "(Landroid/view/MotionEvent;)Z", dispatch_motion as extern "system" fn(_, _, _) -> _),
    ]);
    let surf = e.find_class("org/libsdl/app/SDLSurface");
    let r2 = e.register_natives(surf, &[
        native!("surfaceCreated", "(Landroid/view/SurfaceHolder;)V", surface_created as extern "system" fn(_, _, _)),
        native!("surfaceChanged", "(Landroid/view/SurfaceHolder;III)V", surface_changed as extern "system" fn(_, _, _, _, _, _)),
        native!("surfaceDestroyed", "(Landroid/view/SurfaceHolder;)V", surface_destroyed as extern "system" fn(_, _, _)),
    ]);
    let run = e.find_class("org/libsdl/app/SDLNativeRunnable");
    let r3 = e.register_natives(run, &[native!("run", "()V", runnable_run as extern "system" fn(_, _))]);
    RUNNABLE_CLS.store(e.new_global_ref(run), SeqCst);
    let exc = e.exception_clear();
    logcat(&format!("JNI_OnLoad: RegisterNatives SDLActivity={r1} SDLSurface={r2} SDLNativeRunnable={r3} exception={exc}"));
    JNI_VERSION_1_6
}

fn super_void(e: Env, this: jobject, name: &str, sig: &str, args: &[jvalue]) -> bool {
    let cls = e.find_class("android/app/Activity");
    let m = e.method(cls, name, sig);
    e.call_nonvirtual_void(this, cls, m, args);
    let exc = e.exception_clear();
    e.delete_local_ref(cls);
    exc
}

fn super_bool(e: Env, this: jobject, name: &str, sig: &str, arg: jobject) -> u8 {
    let cls = e.find_class("android/app/Activity");
    let m = e.method(cls, name, sig);
    let r = e.call_nonvirtual_bool(this, cls, m, &[jl(arg)]);
    e.exception_clear();
    e.delete_local_ref(cls);
    r as u8
}

fn connect_stdio(port: i32) -> String {
    let mut res = vec![];
    for (fd, p) in [(1, port), (2, port + 1)] {
        match std::net::TcpStream::connect(("127.0.0.1", p as u16)) {
            Ok(s) => {
                let r = unsafe { dup2(s.as_raw_fd(), fd) };
                res.push(format!("fd{fd}<-127.0.0.1:{p} dup2={r}"));
                std::mem::forget(s);
            }
            Err(e) => res.push(format!("fd{fd}: connect 127.0.0.1:{p} failed: {e}")),
        }
    }
    res.join(", ")
}

extern "system" fn on_create(env: *mut c_void, this: jobject, bundle: jobject) {
    let e = Env(env);
    let t0 = Instant::now();
    let exc = super_void(e, this, "onCreate", "(Landroid/os/Bundle;)V", &[jl(bundle)]);
    ACTIVITY.store(e.new_global_ref(this), SeqCst);
    let intent = e.obj(this, "getIntent", "()Landroid/content/Intent;", &[]);
    let mode = e.str_of(intent, "getStringExtra", &[jl(e.new_string("mode"))], "(Ljava/lang/String;)Ljava/lang/String;");
    let port = e.int(intent, "getIntExtra", "(Ljava/lang/String;I)I", &[jl(e.new_string("port")), ji(0)]);
    let stdio = if port > 0 { connect_stdio(port) } else { "no port".into() };
    say!("onCreate (native): super.onCreate via CallNonvirtualVoidMethod done in {:?}, exception={exc}; thread {}; mode={mode}; stdio {stdio}",
        t0.elapsed(), thread_desc());
    match mode.as_str() {
        "exit-now" => {
            say!("exit-now: a line on stdout");
            eprintln!("exit-now: a line on stderr");
            print!("exit-now: partial line without a newline, then exit(7) from onCreate");
            say!("");
            say!("sdl3-android-runner: exit 7");
            unsafe { exit(7) };
        }
        "exit-now-raw" => {
            let m = b"exit-now-raw: written with write(2), then _exit(9) at once\nsdl3-android-runner: exit 9\n";
            unsafe {
                write(1, m.as_ptr().cast(), m.len());
                write(2, b"exit-now-raw: stderr\n".as_ptr().cast(), 21);
                _exit(9);
            }
        }
        "panic" => {
            say!("panic: panicking in onCreate (an extern \"system\" fn)");
            panic!("spike_app panics on purpose in onCreate");
        }
        _ => {}
    }
    // The content view: an SDLSurface (a DEX class, SurfaceView subclass)
    let surf_cls = e.find_class("org/libsdl/app/SDLSurface");
    let ctor = e.method(surf_cls, "<init>", "(Landroid/content/Context;)V");
    let surface = e.new_object(surf_cls, ctor, &[jl(this)]);
    let holder = e.obj(surface, "getHolder", "()Landroid/view/SurfaceHolder;", &[]);
    let hcls = e.get_object_class(holder);
    let add = e.method(hcls, "addCallback", "(Landroid/view/SurfaceHolder$Callback;)V");
    e.call_void(holder, add, &[jl(surface)]);
    let acls = e.get_object_class(this);
    let scv = e.method(acls, "setContentView", "(Landroid/view/View;)V");
    e.call_void(this, scv, &[jl(surface)]);
    let exc2 = e.exception_clear();
    say!("onCreate: SDLSurface created and set as the content view (exception={exc2})");
    let mode2 = mode.clone();
    std::thread::Builder::new()
        .name("SDLThread".into())
        .spawn(move || {
            let e = env_for(vm());
            say!("worker: attached, thread {}", thread_desc());
            let code = std::panic::catch_unwind(|| run_mode(e, &mode2)).unwrap_or_else(|_| {
                say!("worker: the probe panicked");
                101
            });
            say!("sdl3-android-runner: exit {code}");
            std::process::exit(code);
        })
        .unwrap();
}

extern "system" fn on_resume(env: *mut c_void, this: jobject) {
    let n = RESUMES.fetch_add(1, SeqCst) + 1;
    let exc = super_void(Env(env), this, "onResume", "()V", &[]);
    say!("onResume (native) #{n}: super done, exception={exc}");
}
extern "system" fn on_pause(env: *mut c_void, this: jobject) {
    let n = PAUSES.fetch_add(1, SeqCst) + 1;
    let exc = super_void(Env(env), this, "onPause", "()V", &[]);
    say!("onPause (native) #{n}: super done, exception={exc}");
}
extern "system" fn on_destroy(env: *mut c_void, this: jobject) {
    let exc = super_void(Env(env), this, "onDestroy", "()V", &[]);
    say!("onDestroy (native): super done, exception={exc}");
}
extern "system" fn on_focus(env: *mut c_void, this: jobject, f: u8) {
    let exc = super_void(Env(env), this, "onWindowFocusChanged", "(Z)V", &[jz(f != 0)]);
    FOCUSED.store(f != 0, SeqCst);
    say!("onWindowFocusChanged (native) {}: exception={exc}", f != 0);
}

extern "system" fn dispatch_touch(env: *mut c_void, this: jobject, ev: jobject) -> u8 {
    let e = Env(env);
    let n = TOUCHES.fetch_add(1, SeqCst) + 1;
    let cls = e.get_object_class(ev);
    let action = e.int(ev, "getActionMasked", "()I", &[]);
    let x = e.call_float(ev, e.method(cls, "getX", "()F"), &[]);
    let y = e.call_float(ev, e.method(cls, "getY", "()F"), &[]);
    let src = e.int(ev, "getSource", "()I", &[]);
    let tool = e.int(ev, "getToolType", "(I)I", &[ji(0)]);
    if n <= 40 || action != 2 {
        say!("dispatchTouchEvent #{n}: action={action} x={x:.1} y={y:.1} source=0x{src:x} toolType={tool}");
    }
    super_bool(e, this, "dispatchTouchEvent", "(Landroid/view/MotionEvent;)Z", ev)
}

extern "system" fn dispatch_key(env: *mut c_void, this: jobject, ev: jobject) -> u8 {
    let e = Env(env);
    let n = KEYS.fetch_add(1, SeqCst) + 1;
    let code = e.int(ev, "getKeyCode", "()I", &[]);
    let action = e.int(ev, "getAction", "()I", &[]);
    let uni = e.int(ev, "getUnicodeChar", "()I", &[]);
    let src = e.int(ev, "getSource", "()I", &[]);
    let dev = e.int(ev, "getDeviceId", "()I", &[]);
    say!("dispatchKeyEvent #{n}: keyCode={code} action={action} unicode={uni} ({:?}) source=0x{src:x} device={dev}",
        char::from_u32(uni as u32).unwrap_or('?'));
    super_bool(e, this, "dispatchKeyEvent", "(Landroid/view/KeyEvent;)Z", ev)
}

extern "system" fn dispatch_motion(env: *mut c_void, this: jobject, ev: jobject) -> u8 {
    let e = Env(env);
    let n = MOTIONS.fetch_add(1, SeqCst) + 1;
    if n <= 30 {
        let cls = e.get_object_class(ev);
        let action = e.int(ev, "getActionMasked", "()I", &[]);
        let src = e.int(ev, "getSource", "()I", &[]);
        let ax = e.call_float(ev, e.method(cls, "getAxisValue", "(I)F"), &[ji(0)]);
        let dev = e.int(ev, "getDeviceId", "()I", &[]);
        say!("dispatchGenericMotionEvent #{n}: action={action} source=0x{src:x} AXIS_X={ax:.3} device={dev}");
    }
    super_bool(e, this, "dispatchGenericMotionEvent", "(Landroid/view/MotionEvent;)Z", ev)
}

extern "system" fn surface_created(env: *mut c_void, _this: jobject, holder: jobject) {
    let e = Env(env);
    let s = e.obj(holder, "getSurface", "()Landroid/view/Surface;", &[]);
    let w = unsafe { ANativeWindow_fromSurface(env, s) };
    WINDOW.store(w, SeqCst);
    say!("SDLSurface.surfaceCreated (native, framework-called) thread {}: ANativeWindow={w:?} {}x{}",
        thread_desc(), unsafe { ANativeWindow_getWidth(w) }, unsafe { ANativeWindow_getHeight(w) });
}
extern "system" fn surface_changed(_env: *mut c_void, _this: jobject, _h: jobject, fmt: i32, w: i32, h: i32) {
    say!("SDLSurface.surfaceChanged (native): format={fmt} {w}x{h}");
}
extern "system" fn surface_destroyed(_env: *mut c_void, _this: jobject, _h: jobject) {
    say!("SDLSurface.surfaceDestroyed (native)");
}
extern "system" fn runnable_run(env: *mut c_void, this: jobject) {
    let e = Env(env);
    let cls = e.get_object_class(this);
    let f = e.field(cls, "ptr", "J");
    let p = e.get_long_field(this, f);
    say!("SDLNativeRunnable.run (native): ptr=0x{p:x} thread {}", thread_desc());
    if p == 2 {
        call_sdl_main();
    }
    RUNNABLE_RAN.store(true, SeqCst);
}
extern "system" fn inmem_run(_env: *mut c_void, _this: jobject) {
    say!("SDLInMemoryThread.run (native, called by Thread.start) thread {}", thread_desc());
    INMEM_RAN.store(true, SeqCst);
}

fn call_sdl_main() {
    match dl::Lib::open("libmain.so") {
        Ok(l) => match unsafe { l.sym::<unsafe extern "C" fn(c_int, *const *const c_char) -> c_int>("SDL_main") } {
            Ok(f) => {
                let a = [c"libmain".as_ptr(), c"--from-SDL_main".as_ptr(), std::ptr::null()];
                say!("SDL_main at {:?} (main at {:?}); calling it on thread {}", f as *const c_void, l.raw("main"), thread_desc());
                let r = unsafe { f(2, a.as_ptr()) };
                say!("SDL_main returned {r}");
            }
            Err(err) => say!("SDL_main: {err}"),
        },
        Err(err) => say!("SDL_main: {err}"),
    }
}

fn wait(flag: &dyn Fn() -> bool, secs: u64) -> Option<Duration> {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(secs) {
        if flag() {
            return Some(t.elapsed());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

fn service(e: Env, name: &str) -> jobject {
    e.obj(activity(), "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[jl(e.new_string(name))])
}

fn run_mode(e: Env, mode: &str) -> i32 {
    match mode {
        "basic" => basic(e),
        "dlopen" => dlopen_tests(e),
        "gfx" => {
            let t = wait(&|| !WINDOW.load(SeqCst).is_null(), 10);
            say!("gfx: surface after {t:?}");
            if let Err(err) = gfx::egl("app", WINDOW.load(SeqCst)) {
                say!("[app] EGL error: {err}");
            }
            if let Err(err) = gfx::vulkan("app") {
                say!("[app] Vulkan error: {err}");
            }
            0
        }
        "audio" => {
            for input in [false, true] {
                if let Err(err) = audio::aaudio("app", input) {
                    say!("[app] AAudio error: {err}");
                }
            }
            if let Err(err) = audio::opensles("app") {
                say!("[app] OpenSL ES error: {err}");
            }
            0
        }
        "sensors" => {
            if let Err(err) = sensors::sensors("app", true) {
                say!("[app] sensors error: {err}");
            }
            0
        }
        "camera" => {
            let pm_ok = e.int(activity(), "checkSelfPermission", "(Ljava/lang/String;)I", &[jl(e.new_string("android.permission.CAMERA"))]);
            say!("[app] checkSelfPermission(CAMERA) = {pm_ok} (0 granted, -1 denied)");
            if let Err(err) = camera::camera("app", true) {
                say!("[app] camera error: {err}");
            }
            0
        }
        "devices" => {
            devices(e);
            0
        }
        "input" => {
            let f = wait(&|| FOCUSED.load(SeqCst), 10);
            say!("input: window focused after {f:?}");
            std::thread::sleep(Duration::from_millis(500));
            for r in ["adb shell input tap 300 600", "adb shell input swipe 100 800 600 800 300", "adb shell input keyevent KEYCODE_A",
                "adb shell input text hello", "adb shell input keyevent KEYCODE_ENTER", "adb shell input mouse tap 400 400",
                "adb shell input stylus tap 500 500", "adb shell input keyboard keyevent KEYCODE_B"] {
                request(r);
            }
            std::thread::sleep(Duration::from_secs(12));
            say!("input: totals touch={} key={} generic-motion={}", TOUCHES.load(SeqCst), KEYS.load(SeqCst), MOTIONS.load(SeqCst));
            0
        }
        "gamepad" => {
            request_bg("adb shell /data/local/tmp/spike/uinput_pad 9");
            std::thread::sleep(Duration::from_millis(3000));
            input_devices(e);
            std::thread::sleep(Duration::from_secs(5));
            say!("gamepad: totals key={} generic-motion={}", KEYS.load(SeqCst), MOTIONS.load(SeqCst));
            0
        }
        "lifecycle" => {
            wait(&|| FOCUSED.load(SeqCst), 10);
            request("adb shell input keyevent KEYCODE_HOME");
            std::thread::sleep(Duration::from_secs(3));
            say!("lifecycle: after HOME pauses={} resumes={} focused={}", PAUSES.load(SeqCst), RESUMES.load(SeqCst), FOCUSED.load(SeqCst));
            request("adb shell am start -n org.libsdl.sdl3_rust.spike/org.libsdl.app.SDLActivity");
            std::thread::sleep(Duration::from_secs(3));
            say!("lifecycle: after am start pauses={} resumes={} focused={} (same pid {})", PAUSES.load(SeqCst), RESUMES.load(SeqCst), FOCUSED.load(SeqCst), unsafe { getpid() });
            0
        }
        "sdl-main" => {
            // The plan's nativeRunMain: on a Java thread (as upstream's
            // SDLThread), dlopen ourselves, dlsym SDL_main and call it.
            let rc = RUNNABLE_CLS.load(SeqCst);
            let r = e.new_object(rc, e.method(rc, "<init>", "()V"), &[]);
            e.set_long_field(r, e.field(rc, "ptr", "J"), 2);
            let tc = e.find_class("java/lang/Thread");
            let th = e.new_object(tc, e.method(tc, "<init>", "(Ljava/lang/Runnable;Ljava/lang/String;)V"), &[jl(r), jl(e.new_string("SDLThread-java"))]);
            e.call_void(th, e.method(tc, "start", "()V"), &[]);
            let exc = e.exception_clear();
            say!("sdl-main: java Thread started (exception={exc}); SDL_main done after {:?}", wait(&|| RUNNABLE_RAN.load(SeqCst), 10));
            0
        }
        "sdl-main-rust-thread" => {
            say!("sdl-main-rust-thread: calling SDL_main from a Rust-spawned thread");
            call_sdl_main();
            0
        }
        m => {
            say!("unknown mode {m}");
            64
        }
    }
}

fn basic(e: Env) -> i32 {
    let ver = e.find_class("android/os/Build$VERSION");
    let sdk = e.get_static_int_field(ver, e.static_field(ver, "SDK_INT", "I"));
    let build = e.find_class("android/os/Build");
    let model = e.get_static_object_field(build, e.static_field(build, "MODEL", "Ljava/lang/String;"));
    let abis = e.get_static_object_field(build, e.static_field(build, "SUPPORTED_ABIS", "[Ljava/lang/String;"));
    let abis: Vec<String> = (0..e.array_len(abis)).map(|i| e.string(e.object_array_get(abis, i))).collect();
    say!("basic: SDK_INT={sdk} MODEL={} SUPPORTED_ABIS={abis:?} uid={}", e.string(model), unsafe { getuid() });
    say!("basic: cwd={:?} HOME={:?} TMPDIR={:?} PATH={:?}", std::env::current_dir(), std::env::var("HOME"), std::env::var("TMPDIR"), std::env::var("PATH"));
    let a = activity();
    let files = e.obj(a, "getFilesDir", "()Ljava/io/File;", &[]);
    let cache = e.obj(a, "getCacheDir", "()Ljava/io/File;", &[]);
    let ai = e.obj(a, "getApplicationInfo", "()Landroid/content/pm/ApplicationInfo;", &[]);
    let aicls = e.get_object_class(ai);
    let nld = e.string(e.get_object_field(ai, e.field(aicls, "nativeLibraryDir", "Ljava/lang/String;")));
    let src = e.string(e.get_object_field(ai, e.field(aicls, "sourceDir", "Ljava/lang/String;")));
    say!("basic: filesDir={} cacheDir={} nativeLibraryDir={nld} sourceDir={src}", e.to_string(files), e.to_string(cache));
    let ls = std::fs::read_dir(&nld).map(|d| d.filter_map(|x| x.ok()).map(|x| x.file_name().to_string_lossy().into_owned()).collect::<Vec<_>>());
    say!("basic: nativeLibraryDir entries {ls:?}");

    // FindClass from a native-attached thread uses the system class loader
    let c = e.find_class("org/libsdl/app/SDLNativeRunnable");
    let exc = e.exception_clear();
    say!("basic: FindClass(SDLNativeRunnable) on the worker thread = {c:?} (exception={exc}); the class cached in JNI_OnLoad = {:?}", RUNNABLE_CLS.load(SeqCst));

    // SDLNativeRunnable posted to the UI thread
    let rc = RUNNABLE_CLS.load(SeqCst);
    let r = e.new_object(rc, e.method(rc, "<init>", "()V"), &[]);
    e.set_long_field(r, e.field(rc, "ptr", "J"), 0x5D13_0001);
    let acls = e.get_object_class(a);
    e.call_void(a, e.method(acls, "runOnUiThread", "(Ljava/lang/Runnable;)V"), &[jl(r)]);
    let exc = e.exception_clear();
    say!("basic: runOnUiThread(SDLNativeRunnable) exception={exc}; ran after {:?}", wait(&|| RUNNABLE_RAN.load(SeqCst), 5));

    // InMemoryDexClassLoader with inmem.dex
    let t = Instant::now();
    let buf = e.new_direct_byte_buffer(INMEM_DEX.as_ptr() as *mut c_void, INMEM_DEX.len() as i64);
    let lcls = e.find_class("dalvik/system/InMemoryDexClassLoader");
    let lctor = e.method(lcls, "<init>", "(Ljava/nio/ByteBuffer;Ljava/lang/ClassLoader;)V");
    let parent = e.obj(a, "getClassLoader", "()Ljava/lang/ClassLoader;", &[]);
    let loader = e.new_object(lcls, lctor, &[jl(buf), jl(parent)]);
    let exc = e.exception_clear();
    say!("basic: InMemoryDexClassLoader({} bytes) = {loader:?} exception={exc} in {:?}", INMEM_DEX.len(), t.elapsed());
    if !loader.is_null() {
        let cls = e.obj(loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[jl(e.new_string("org.libsdl.inmem.SDLInMemoryThread"))]);
        say!("basic: loadClass(org.libsdl.inmem.SDLInMemoryThread) = {} in {:?}", e.to_string(cls), t.elapsed());
        if !cls.is_null() {
            let rn = e.register_natives(cls, &[native!("run", "()V", inmem_run as extern "system" fn(_, _))]);
            let th = e.new_object(cls, e.method(cls, "<init>", "()V"), &[]);
            let start = e.method(cls, "start", "()V");
            e.call_void(th, start, &[]);
            let exc = e.exception_clear();
            say!("basic: RegisterNatives={rn}, Thread.start exception={exc}; run() ran after {:?}", wait(&|| INMEM_RAN.load(SeqCst), 5));
        }
        // the app's classes.dex again, from the bytes in libmain.so's .sdl3_android_dex
        let buf2 = e.new_direct_byte_buffer(CLASSES_DEX.as_ptr() as *mut c_void, CLASSES_DEX.len() as i64);
        let l2 = e.new_object(lcls, lctor, &[jl(buf2), jl(std::ptr::null_mut())]);
        let c2 = e.obj(l2, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[jl(e.new_string("org.libsdl.app.SDLActivity"))]);
        let exc = e.exception_clear();
        let act_cls = e.get_object_class(a);
        say!("basic: classes.dex in memory (parent null): loadClass(SDLActivity) = {} exception={exc}; same class as the running one: {}",
            e.to_string(c2), !c2.is_null() && e.is_same_object(c2, act_cls));
    }
    let t = wait(&|| !WINDOW.load(SeqCst).is_null(), 10);
    say!("basic: surface created after {t:?}; focused={}", FOCUSED.load(SeqCst));
    0
}

fn dlopen_tests(e: Env) -> i32 {
    let a = activity();
    let ai = e.obj(a, "getApplicationInfo", "()Landroid/content/pm/ApplicationInfo;", &[]);
    let aicls = e.get_object_class(ai);
    let nld = e.string(e.get_object_field(ai, e.field(aicls, "nativeLibraryDir", "Ljava/lang/String;")));
    type Main = unsafe extern "C" fn(c_int, *const *const c_char, *const *const c_char) -> c_int;
    let env0: [*const c_char; 1] = [std::ptr::null()];
    // 1. other programs, linked -shared, as PIEs with main exported, and plain PIEs
    for (file, sym) in [("libprobe_shared.so", "SDL_main"), ("libprobe_pie_exp.so", "main"), ("libprobe_pie.so", "main")] {
        let path = format!("{nld}/{file}");
        say!("dlopen: {path} ...");
        match dl::Lib::open(&path) {
            Err(err) => say!("dlopen: {file}: {err}"),
            Ok(l) => match unsafe { l.sym::<Main>(sym) } {
                Err(err) => say!("dlopen: {file}: opened; {err}"),
                Ok(f) => {
                    say!("dlopen: {file}: opened, {sym} at {:?}; calling it", f as *const c_void);
                    let argv = [c"prog".as_ptr(), c"ret".as_ptr(), c"5".as_ptr(), std::ptr::null()];
                    let r = unsafe { f(3, argv.as_ptr(), env0.as_ptr()) };
                    say!("dlopen: {file}: {sym}(\"prog ret 5\") returned {r}");
                }
            },
        }
    }
    // 2. exec the PIE from nativeLibraryDir as a child process
    let t = Instant::now();
    match std::process::Command::new(format!("{nld}/libprobe_pie.so")).args(["ret", "3"]).output() {
        Ok(o) => say!("exec: libprobe_pie.so ret 3: {:?} stdout={:?} in {:?}", o.status, String::from_utf8_lossy(&o.stdout), t.elapsed()),
        Err(err) => say!("exec: libprobe_pie.so failed: {err}"),
    }
    0
}

fn devices(e: Env) {
    let vib = service(e, "vibrator");
    if !vib.is_null() {
        say!("devices: Vibrator {}: hasVibrator={} hasAmplitudeControl={}", e.to_string(vib),
            e.boolean(vib, "hasVibrator", "()Z", &[]), e.boolean(vib, "hasAmplitudeControl", "()Z", &[]));
        let ve = e.find_class("android/os/VibrationEffect");
        let one = e.call_static_object(ve, e.static_method(ve, "createOneShot", "(JI)Landroid/os/VibrationEffect;"), &[jj(100), ji(255)]);
        let vcls = e.get_object_class(vib);
        e.call_void(vib, e.method(vcls, "vibrate", "(Landroid/os/VibrationEffect;)V"), &[jl(one)]);
        say!("devices: vibrate(100 ms) exception={}", e.exception_clear());
    }
    let vm_ = service(e, "vibrator_manager");
    if !vm_.is_null() {
        let ids = e.obj(vm_, "getVibratorIds", "()[I", &[]);
        say!("devices: VibratorManager ids={:?}", if ids.is_null() { vec![] } else { e.int_array(ids) });
    }
    let imm = service(e, "input_method");
    let list = e.obj(imm, "getInputMethodList", "()Ljava/util/List;", &[]);
    let n = e.int(list, "size", "()I", &[]);
    for i in 0..n.max(0) {
        let info = e.obj(list, "get", "(I)Ljava/lang/Object;", &[ji(i)]);
        say!("devices: input method {i}: {}", e.str_of(info, "getId", &[], "()Ljava/lang/String;"));
    }
    let pm = e.obj(activity(), "getPackageManager", "()Landroid/content/pm/PackageManager;", &[]);
    for feat in ["android.hardware.camera", "android.hardware.camera.front", "android.hardware.sensor.accelerometer",
        "android.hardware.vulkan.level", "android.hardware.vulkan.version", "android.hardware.opengles.aep",
        "android.hardware.touchscreen", "android.hardware.gamepad", "android.hardware.audio.low_latency",
        "android.hardware.microphone", "android.hardware.usb.host", "android.hardware.bluetooth_le"] {
        let has = e.boolean(pm, "hasSystemFeature", "(Ljava/lang/String;)Z", &[jl(e.new_string(feat))]);
        say!("devices: feature {feat}: {has}");
    }
    input_devices(e);
}

fn input_devices(e: Env) {
    let idc = e.find_class("android/view/InputDevice");
    let ids = e.call_static_object(idc, e.static_method(idc, "getDeviceIds", "()[I"), &[]);
    let get = e.static_method(idc, "getDevice", "(I)Landroid/view/InputDevice;");
    for id in e.int_array(ids) {
        let d = e.call_static_object(idc, get, &[ji(id)]);
        if d.is_null() {
            continue;
        }
        let src = e.int(d, "getSources", "()I", &[]);
        let gamepad = src & 0x401 == 0x401;
        let joystick = src & 0x0100_0010 == 0x0100_0010;
        say!("input device {id}: name={:?} sources=0x{src:x} gamepad={gamepad} joystick={joystick} vendor=0x{:x} product=0x{:x} virtual={} external={} controllerNumber={} descriptor={}",
            e.str_of(d, "getName", &[], "()Ljava/lang/String;"), e.int(d, "getVendorId", "()I", &[]), e.int(d, "getProductId", "()I", &[]),
            e.boolean(d, "isVirtual", "()Z", &[]), e.boolean(d, "isExternal", "()Z", &[]), e.int(d, "getControllerNumber", "()I", &[]),
            e.str_of(d, "getDescriptor", &[], "()Ljava/lang/String;"));
    }
}
