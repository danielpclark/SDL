// Tests of the Wayland video driver against a real compositor.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Each compositor test starts its own headless compositor in a private
//! `XDG_RUNTIME_DIR`: sway (wlroots, with its virtual keyboard, virtual
//! pointer and data control protocols for input and clipboard tests), else
//! weston, whichever is found in `PATH`. Without either the tests print a
//! note and pass. The keymap tests need only libxkbcommon.
//!
//! A second client of the compositor (the test's own connection, on the
//! proxy runtime of [`super::client`]) injects input and reads and sets the
//! clipboard; the test-only protocols it uses are declared here by hand.

use std::ffi::{CStr, OsString};
use std::io::Write;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::client::{
    arg_new, arg_raw_obj, arg_str, AsProxy, Conn, Interface, NoEvents, Obj, Proxy, RawObj,
};
use super::events::{wayland_build_keymaps, ModMasks, XkbContext, XkbKeymap, XkbState};
use super::protocols::wayland::*;
use super::sys::*;
use super::wldyn::{load_symbols, WaylandSyms};
use crate::events::keyboard::{Keycode, Keymod, Scancode};
use crate::events::mouse::{self, SystemCursor};
use crate::events::window::WindowFlags;
use crate::events::{Event, EventType};
use crate::hints;
use crate::init::{self, InitFlags};
use crate::video::window::{
    Window as SdlWindow, PROP_WINDOW_WAYLAND_SURFACE_POINTER,
    PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_POINTER,
};
use crate::video::{self, PixelFormat, Surface};

// ---------------------------------------------------------------------------
// The compositor
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Sway,
    Weston,
}

/// The environment variables a test changes (restored after it).
const ENV_VARS: [&str; 5] = [
    "XDG_RUNTIME_DIR",
    "WAYLAND_DISPLAY",
    "WAYLAND_SOCKET",
    "DISPLAY",
    "XDG_SESSION_TYPE",
];

/// A headless compositor for one test, in its own runtime directory, which
/// `WAYLAND_DISPLAY` points at while it runs.
struct Compositor {
    kind: Kind,
    child: Child,
    dir: PathBuf,
    old_env: Vec<(&'static str, Option<OsString>)>,
}

/// An executable in `PATH`.
fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// A short runtime directory (the socket path must fit in `sun_path`).
fn runtime_dir() -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let base = if Path::new("/tmp").is_dir() {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let dir = base.join(format!(
        "sdlwl-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

impl Compositor {
    /// A new compositor: sway if available, else weston (with
    /// `need_sway`, only sway).
    fn start(need_sway: bool) -> Option<Compositor> {
        let sway = find_in_path("sway");
        let weston = if need_sway {
            None
        } else {
            find_in_path("weston")
        };
        let (kind, exe) = match (sway, weston) {
            (Some(s), _) => (Kind::Sway, s),
            (None, Some(w)) => (Kind::Weston, w),
            (None, None) => {
                if need_sway {
                    eprintln!("note: sway (for its input and data control protocols) isn't in PATH; skipping");
                } else {
                    eprintln!("note: no Wayland compositor (sway or weston) in PATH; skipping");
                }
                return None;
            }
        };
        let dir = runtime_dir();
        let mut cmd = Command::new(&exe);
        cmd.env("XDG_RUNTIME_DIR", &dir)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("WAYLAND_SOCKET")
            .env_remove("DISPLAY")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        match kind {
            Kind::Sway => {
                let conf = dir.join("sway.conf");
                // Floating windows keep the size they ask for (sway tiles otherwise).
                std::fs::write(
                    &conf,
                    "output HEADLESS-1 resolution 1024x768\nfor_window [title=\".*\"] floating enable\n",
                )
                .unwrap();
                cmd.env("WLR_BACKENDS", "headless")
                    .env("WLR_RENDERER", "pixman")
                    .env("WLR_LIBINPUT_NO_DEVICES", "1")
                    .arg("-c")
                    .arg(&conf);
            }
            Kind::Weston => {
                cmd.args([
                    "--backend=headless",
                    "--socket=wayland-1",
                    "--idle-time=0",
                    "--no-config",
                    "--width=1024",
                    "--height=768",
                ]);
            }
        }
        let child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                eprintln!("note: can't start {}: {e}; skipping", exe.display());
                let _ = std::fs::remove_dir_all(&dir);
                return None;
            }
        };

        let old_env = ENV_VARS.iter().map(|&v| (v, std::env::var_os(v))).collect();
        let mut compositor = Compositor {
            kind,
            child,
            dir,
            old_env,
        };

        // Wait for the socket.
        let deadline = Instant::now() + Duration::from_secs(10);
        let socket = loop {
            let found = std::fs::read_dir(&compositor.dir).ok().and_then(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .find(|n| n.starts_with("wayland-") && !n.ends_with(".lock"))
            });
            if let Some(socket) = found {
                break Some(socket);
            }
            if Instant::now() > deadline || compositor.child.try_wait().ok().flatten().is_some() {
                break None;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let Some(socket) = socket else {
            eprintln!("note: {kind:?} didn't start; skipping");
            return None;
        };

        crate::stdlib::setenv_unsafe("XDG_RUNTIME_DIR", &compositor.dir.to_string_lossy(), true)
            .unwrap();
        crate::stdlib::setenv_unsafe("WAYLAND_DISPLAY", &socket, true).unwrap();
        crate::stdlib::setenv_unsafe("XDG_SESSION_TYPE", "wayland", true).unwrap();
        crate::stdlib::unsetenv_unsafe("WAYLAND_SOCKET").unwrap();
        crate::stdlib::unsetenv_unsafe("DISPLAY").unwrap();

        // Wait until it accepts clients and has an output.
        let syms = load_symbols()?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(conn) = Conn::connect(&syms) {
                let client = TestClient::from_conn(conn);
                if client.global("wl_output").is_some()
                    && (kind == Kind::Weston || client.global("wl_seat").is_some())
                {
                    break;
                }
            }
            if Instant::now() > deadline {
                eprintln!("note: can't connect to {kind:?}; skipping");
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        compositor.kind = kind;
        Some(compositor)
    }
}

impl Drop for Compositor {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        for (name, value) in self.old_env.drain(..) {
            match value {
                Some(v) => {
                    let _ = crate::stdlib::setenv_unsafe(name, &v.to_string_lossy(), true);
                }
                None => {
                    let _ = crate::stdlib::unsetenv_unsafe(name);
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Video initialized with the Wayland driver; quits video when dropped
/// (before the compositor goes away).
struct Video;

impl Video {
    fn init() -> Video {
        init::quit();
        hints::set(hints::VIDEO_DRIVER, "wayland").unwrap();
        init::init(InitFlags::VIDEO).unwrap();
        Video
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        init::quit();
        hints::reset(hints::VIDEO_DRIVER);
    }
}

/// Pump events until `pred` matches one (returned), for up to `secs`
/// seconds, calling `between` between pumps.
fn wait_for_with(
    secs: u64,
    mut between: impl FnMut(),
    mut pred: impl FnMut(&Event) -> bool,
) -> Option<Event> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        crate::events::pump();
        while let Some(e) = crate::events::poll() {
            if pred(&e) {
                return Some(e);
            }
        }
        between();
        std::thread::sleep(Duration::from_millis(5));
    }
    None
}

/// Pump events until `pred` matches one, for up to three seconds.
fn wait_for(pred: impl FnMut(&Event) -> bool) -> Option<Event> {
    wait_for_with(3, || {}, pred)
}

/// Pump events for a while.
fn pump_for(ms: u64) {
    let deadline = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < deadline {
        crate::events::pump();
        while crate::events::poll().is_some() {}
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Draw a frame on a window (with its framebuffer): a Wayland surface is
/// only mapped, and gets focus and state changes, once it has a buffer.
fn present(w: &SdlWindow) {
    if let Ok(surface) = w.surface() {
        if let Some(p) = surface.lock().pixels_mut() {
            p.fill(0x80);
        }
        let _ = surface.update();
    }
}

/// [`wait_for`], drawing frames on `w` meanwhile.
fn wait_presenting(w: &SdlWindow, pred: impl FnMut(&Event) -> bool) -> Option<Event> {
    let mut n = 0u32;
    wait_for_with(
        3,
        || {
            n += 1;
            if n % 4 == 1 {
                present(w);
            }
        },
        pred,
    )
}

/// [`pump_for`], drawing frames on `w` meanwhile.
fn pump_presenting(w: &SdlWindow, ms: u64) {
    let deadline = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < deadline {
        present(w);
        pump_for(20);
    }
}

fn window_event(e: &Event, window: crate::events::WindowID, t: EventType) -> bool {
    matches!(e, Event::Window(we) if we.event_type == t && we.window_id == window)
}

// ---------------------------------------------------------------------------
// The test-only protocols (declared by hand)
// ---------------------------------------------------------------------------

/// A `wl_message` of string literals.
const fn msg(
    name: &'static CStr,
    signature: &'static CStr,
    types: &'static [IfacePtr],
) -> wl_message {
    wl_message {
        name: name.as_ptr(),
        signature: signature.as_ptr(),
        types: types.as_ptr(),
    }
}

/// A `wl_interface` of message tables.
const fn iface(
    name: &'static CStr,
    version: i32,
    methods: &'static [wl_message],
    events: &'static [wl_message],
) -> wl_interface {
    wl_interface {
        name: name.as_ptr(),
        version,
        method_count: methods.len() as i32,
        methods: methods.as_ptr(),
        event_count: events.len() as i32,
        events: events.as_ptr(),
    }
}

static NONE: [IfacePtr; 5] = [None; 5];

// zwp_virtual_keyboard_manager_v1 / zwp_virtual_keyboard_v1
static VKBD_MANAGER_CREATE_TYPES: [IfacePtr; 2] = [Some(&WL_SEAT_INTERFACE), Some(&VKBD_INTERFACE)];
static VKBD_MANAGER_REQUESTS: [wl_message; 1] = [msg(
    c"create_virtual_keyboard",
    c"on",
    &VKBD_MANAGER_CREATE_TYPES,
)];
static VKBD_MANAGER_INTERFACE: wl_interface = iface(
    c"zwp_virtual_keyboard_manager_v1",
    1,
    &VKBD_MANAGER_REQUESTS,
    &[],
);
static VKBD_REQUESTS: [wl_message; 4] = [
    msg(c"keymap", c"uhu", &NONE),
    msg(c"key", c"uuu", &NONE),
    msg(c"modifiers", c"uuuu", &NONE),
    msg(c"destroy", c"", &NONE),
];
static VKBD_INTERFACE: wl_interface = iface(c"zwp_virtual_keyboard_v1", 1, &VKBD_REQUESTS, &[]);

// zwlr_virtual_pointer_manager_v1 / zwlr_virtual_pointer_v1
static VPTR_MANAGER_CREATE_TYPES: [IfacePtr; 2] = [Some(&WL_SEAT_INTERFACE), Some(&VPTR_INTERFACE)];
static VPTR_MANAGER_REQUESTS: [wl_message; 2] = [
    msg(
        c"create_virtual_pointer",
        c"?on",
        &VPTR_MANAGER_CREATE_TYPES,
    ),
    msg(c"destroy", c"", &NONE),
];
static VPTR_MANAGER_INTERFACE: wl_interface = iface(
    c"zwlr_virtual_pointer_manager_v1",
    1,
    &VPTR_MANAGER_REQUESTS,
    &[],
);
static VPTR_REQUESTS: [wl_message; 9] = [
    msg(c"motion", c"uff", &NONE),
    msg(c"motion_absolute", c"uuuuu", &NONE),
    msg(c"button", c"uuu", &NONE),
    msg(c"axis", c"uuf", &NONE),
    msg(c"frame", c"", &NONE),
    msg(c"axis_source", c"u", &NONE),
    msg(c"axis_stop", c"uu", &NONE),
    msg(c"axis_discrete", c"uufi", &NONE),
    msg(c"destroy", c"", &NONE),
];
static VPTR_INTERFACE: wl_interface = iface(c"zwlr_virtual_pointer_v1", 1, &VPTR_REQUESTS, &[]);

// zwlr_data_control_manager_v1 and its device, source and offer
static DC_MANAGER_SOURCE_TYPES: [IfacePtr; 1] = [Some(&DC_SOURCE_INTERFACE)];
static DC_MANAGER_DEVICE_TYPES: [IfacePtr; 2] =
    [Some(&DC_DEVICE_INTERFACE), Some(&WL_SEAT_INTERFACE)];
static DC_MANAGER_REQUESTS: [wl_message; 3] = [
    msg(c"create_data_source", c"n", &DC_MANAGER_SOURCE_TYPES),
    msg(c"get_data_device", c"no", &DC_MANAGER_DEVICE_TYPES),
    msg(c"destroy", c"", &NONE),
];
static DC_MANAGER_INTERFACE: wl_interface = iface(
    c"zwlr_data_control_manager_v1",
    1,
    &DC_MANAGER_REQUESTS,
    &[],
);
static DC_DEVICE_SOURCE_TYPES: [IfacePtr; 1] = [Some(&DC_SOURCE_INTERFACE)];
static DC_DEVICE_OFFER_TYPES: [IfacePtr; 1] = [Some(&DC_OFFER_INTERFACE)];
static DC_DEVICE_REQUESTS: [wl_message; 2] = [
    msg(c"set_selection", c"?o", &DC_DEVICE_SOURCE_TYPES),
    msg(c"destroy", c"", &NONE),
];
static DC_DEVICE_EVENTS: [wl_message; 3] = [
    msg(c"data_offer", c"n", &DC_DEVICE_OFFER_TYPES),
    msg(c"selection", c"?o", &DC_DEVICE_OFFER_TYPES),
    msg(c"finished", c"", &NONE),
];
static DC_DEVICE_INTERFACE: wl_interface = iface(
    c"zwlr_data_control_device_v1",
    1,
    &DC_DEVICE_REQUESTS,
    &DC_DEVICE_EVENTS,
);
static DC_SOURCE_REQUESTS: [wl_message; 2] =
    [msg(c"offer", c"s", &NONE), msg(c"destroy", c"", &NONE)];
static DC_SOURCE_EVENTS: [wl_message; 2] =
    [msg(c"send", c"sh", &NONE), msg(c"cancelled", c"", &NONE)];
static DC_SOURCE_INTERFACE: wl_interface = iface(
    c"zwlr_data_control_source_v1",
    1,
    &DC_SOURCE_REQUESTS,
    &DC_SOURCE_EVENTS,
);
static DC_OFFER_REQUESTS: [wl_message; 2] =
    [msg(c"receive", c"sh", &NONE), msg(c"destroy", c"", &NONE)];
static DC_OFFER_EVENTS: [wl_message; 1] = [msg(c"offer", c"s", &NONE)];
static DC_OFFER_INTERFACE: wl_interface = iface(
    c"zwlr_data_control_offer_v1",
    1,
    &DC_OFFER_REQUESTS,
    &DC_OFFER_EVENTS,
);

/// Declare the marker type of a test protocol interface.
macro_rules! test_interface {
    ($ty:ident, $name:literal, $table:ident, $destructor:expr, $event:ty, |$conn:ident, $op:ident, $args:ident| $decode:expr) => {
        enum $ty {}
        impl Interface for $ty {
            const NAME: &'static str = $name;
            const VERSION: u32 = 1;
            const DESTRUCTOR: Option<(u32, u32)> = $destructor;
            type Event<'a> = $event;
            fn interface() -> &'static wl_interface {
                &$table
            }
            #[allow(clippy::needless_lifetimes)] // (not all decoders borrow)
            unsafe fn decode<'a>(
                $conn: &'a Arc<Conn>,
                $op: u32,
                $args: *const wl_argument,
            ) -> Option<$event> {
                $decode
            }
        }
    };
}

test_interface!(
    VkbdManager,
    "zwp_virtual_keyboard_manager_v1",
    VKBD_MANAGER_INTERFACE,
    None,
    NoEvents,
    |_c, _o, _a| None
);
test_interface!(
    Vkbd,
    "zwp_virtual_keyboard_v1",
    VKBD_INTERFACE,
    Some((3, 1)),
    NoEvents,
    |_c, _o, _a| None
);
test_interface!(
    VptrManager,
    "zwlr_virtual_pointer_manager_v1",
    VPTR_MANAGER_INTERFACE,
    Some((1, 1)),
    NoEvents,
    |_c, _o, _a| None
);
test_interface!(
    Vptr,
    "zwlr_virtual_pointer_v1",
    VPTR_INTERFACE,
    Some((8, 1)),
    NoEvents,
    |_c, _o, _a| None
);
test_interface!(
    DcManager,
    "zwlr_data_control_manager_v1",
    DC_MANAGER_INTERFACE,
    Some((2, 1)),
    NoEvents,
    |_c, _o, _a| None
);

/// The events of a data control device.
enum DcDeviceEvent {
    DataOffer(Proxy<DcOffer>),
    Selection(Option<RawObj>),
    Finished,
}
test_interface!(
    DcDevice,
    "zwlr_data_control_device_v1",
    DC_DEVICE_INTERFACE,
    Some((1, 1)),
    DcDeviceEvent,
    |conn, opcode, args| {
        // SAFETY: libwayland passes the arguments of event `opcode`.
        unsafe {
            let a = std::slice::from_raw_parts(args, 1);
            match opcode {
                0 => arg_new::<DcOffer>(conn, &a[0]).map(DcDeviceEvent::DataOffer),
                1 => Some(DcDeviceEvent::Selection(arg_raw_obj(&a[0]))),
                2 => Some(DcDeviceEvent::Finished),
                _ => None,
            }
        }
    }
);

/// The events of a data control source.
enum DcSourceEvent<'a> {
    Send(&'a CStr, OwnedFd),
    Cancelled,
}
test_interface!(
    DcSource,
    "zwlr_data_control_source_v1",
    DC_SOURCE_INTERFACE,
    Some((1, 1)),
    DcSourceEvent<'a>,
    |_conn, opcode, args| {
        // SAFETY: libwayland passes the arguments of event `opcode`.
        unsafe {
            match opcode {
                0 => {
                    let a = std::slice::from_raw_parts(args, 2);
                    Some(DcSourceEvent::Send(
                        arg_str(&a[0])?,
                        OwnedFd::from_raw_fd(a[1].h),
                    ))
                }
                1 => Some(DcSourceEvent::Cancelled),
                _ => None,
            }
        }
    }
);

/// The events of a data control offer.
struct DcOfferEvent<'a>(&'a CStr);
test_interface!(
    DcOffer,
    "zwlr_data_control_offer_v1",
    DC_OFFER_INTERFACE,
    Some((1, 1)),
    DcOfferEvent<'a>,
    |_conn, opcode, args| {
        // SAFETY: libwayland passes the arguments of event `opcode`.
        unsafe {
            let a = std::slice::from_raw_parts(args, 1);
            if opcode == 0 {
                arg_str(&a[0]).map(DcOfferEvent)
            } else {
                None
            }
        }
    }
);

/// Send a request without a new object.
fn request<I: Interface>(obj: Obj<'_, I>, opcode: u32, args: &mut [wl_argument]) {
    // SAFETY: the callers pass arguments matching the request's signature.
    unsafe { obj.marshal(opcode, args, 0) }
}

// ---------------------------------------------------------------------------
// The second client
// ---------------------------------------------------------------------------

/// A client of the compositor besides SDL (the test's own connection).
struct TestClient {
    conn: Arc<Conn>,
    _registry: Proxy<WlRegistry>,
    globals: Arc<Mutex<Vec<(u32, String, u32)>>>,
}

impl TestClient {
    fn connect(syms: &Arc<WaylandSyms>) -> TestClient {
        TestClient::from_conn(Conn::connect(syms).expect("can't connect the test client"))
    }

    fn from_conn(conn: Arc<Conn>) -> TestClient {
        let mut registry = conn.obj().get_registry();
        let globals: Arc<Mutex<Vec<(u32, String, u32)>>> = Arc::default();
        let g = globals.clone();
        registry.listen(move |_, ev| match ev {
            WlRegistryEvent::Global {
                name,
                interface,
                version,
            } => {
                g.lock()
                    .unwrap()
                    .push((name, interface.to_string_lossy().into_owned(), version));
            }
            WlRegistryEvent::GlobalRemove { name } => g.lock().unwrap().retain(|e| e.0 != name),
        });
        let _ = conn.roundtrip();
        TestClient {
            conn,
            _registry: registry,
            globals,
        }
    }

    fn global(&self, interface: &str) -> Option<(u32, u32)> {
        self.globals
            .lock()
            .unwrap()
            .iter()
            .find(|g| g.1 == interface)
            .map(|g| (g.0, g.2))
    }

    fn bind<I: Interface>(&self, version: u32) -> Option<Proxy<I>> {
        let (name, v) = self.global(I::NAME)?;
        Some(self._registry.bind::<I>(name, version.min(v)))
    }
}

/// A hand-written xkb keymap: two layouts (with a and q swapped in the
/// second), shift, return, and a key no scancode table knows.
const TEST_KEYMAP: &str = r#"xkb_keymap {
xkb_keycodes "test" {
    minimum = 8;
    maximum = 255;
    <ESC> = 9;
    <RTRN> = 36;
    <AC01> = 38;
    <AD01> = 24;
    <LFSH> = 50;
    <I250> = 250;
    indicator 1 = "Caps Lock";
};
xkb_types "test" {
    virtual_modifiers NumLock;
    type "ONE_LEVEL" {
        modifiers = none;
        level_name[Level1] = "Any";
    };
    type "TWO_LEVEL" {
        modifiers = Shift;
        map[Shift] = Level2;
        level_name[Level1] = "Base";
        level_name[Level2] = "Shift";
    };
    type "ALPHABETIC" {
        modifiers = Shift + Lock;
        map[Shift] = Level2;
        map[Lock] = Level2;
        level_name[Level1] = "Base";
        level_name[Level2] = "Caps";
    };
};
xkb_compatibility "test" {
    interpret Any + AnyOf(all) {
        action = SetMods(modifiers = modMapMods, clearLocks);
    };
};
xkb_symbols "test" {
    name[Group1] = "English (US)";
    name[Group2] = "Swapped";
    key <ESC>  { [ Escape ] };
    key <RTRN> { [ Return ] };
    key <AC01> { type = "ALPHABETIC", symbols[Group1] = [ a, A ], symbols[Group2] = [ q, Q ] };
    key <AD01> { type = "ALPHABETIC", symbols[Group1] = [ q, Q ], symbols[Group2] = [ a, A ] };
    key <LFSH> { [ Shift_L ] };
    key <I250> { [ XF86Launch5 ] };
    modifier_map Shift { <LFSH> };
};
};
"#;

/// A file descriptor holding `data` (for `zwp_virtual_keyboard_v1.keymap`).
fn memfd_with(data: &[u8]) -> OwnedFd {
    // SAFETY: the name is NUL-terminated.
    let fd = unsafe { libc::memfd_create(c"sdl-test".as_ptr(), libc::MFD_CLOEXEC) };
    assert!(fd >= 0);
    // SAFETY: a new descriptor we own.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut f = std::fs::File::from(fd.try_clone().unwrap());
    f.write_all(data).unwrap();
    fd
}

/// A virtual keyboard on the seat of a test client, with [`TEST_KEYMAP`].
struct VirtualKeyboard {
    kbd: Proxy<Vkbd>,
    time: u32,
}

impl VirtualKeyboard {
    fn new(client: &TestClient) -> Option<VirtualKeyboard> {
        let manager = client.bind::<VkbdManager>(1)?;
        let seat = client.bind::<WlSeat>(1)?;
        let mut args = [
            wl_argument { o: seat.raw() },
            wl_argument {
                o: std::ptr::null_mut(),
            },
        ];
        // SAFETY: the arguments of create_virtual_keyboard, "on".
        let kbd = unsafe {
            manager
                .obj()
                .marshal_constructor::<Vkbd>(0, &mut args, 1, 0)
        };
        let mut keymap = TEST_KEYMAP.as_bytes().to_vec();
        keymap.push(0);
        let fd = memfd_with(&keymap);
        request(
            kbd.obj(),
            0,
            &mut [
                wl_argument { u: 1 }, // XKB_V1
                wl_argument { h: fd.as_raw_fd() },
                wl_argument {
                    u: keymap.len() as u32,
                },
            ],
        );
        client.conn.roundtrip().unwrap();
        Some(VirtualKeyboard { kbd, time: 1 })
    }

    /// Press and release an evdev key.
    fn tap(&mut self, client: &TestClient, key: u32) {
        for state in [1, 0] {
            self.time += 10;
            request(
                self.kbd.obj(),
                1,
                &mut [
                    wl_argument { u: self.time },
                    wl_argument { u: key },
                    wl_argument { u: state },
                ],
            );
        }
        let _ = client.conn.flush();
    }
}

// ---------------------------------------------------------------------------
// Tests without a compositor
// ---------------------------------------------------------------------------

#[test]
fn xkb_keymaps_are_built_per_layout() {
    let _l = crate::test_support::test_lock();
    let Some(syms) = load_symbols() else {
        eprintln!("note: libwayland-client or libxkbcommon isn't available; skipping");
        return;
    };
    let context = XkbContext::new(&syms).unwrap();
    let source = std::ffi::CString::new(TEST_KEYMAP).unwrap();
    let keymap =
        XkbKeymap::from_string(&context, &source).expect("the test keymap doesn't compile");
    let state = XkbState::new(&keymap).unwrap();
    let masks = ModMasks::of(&keymap);
    assert_ne!(masks.shift_mask, 0);
    assert_eq!(keymap.num_layouts(), 2);

    // A hardware keyboard: the scancodes come from the evdev table.
    let (maps, reserved) = wayland_build_keymaps(&keymap, &state, &masks, 2, false);
    assert_eq!(maps.len(), 2);
    assert_eq!(maps[0].keycode(Scancode::A, Keymod::NONE), Keycode::A);
    assert_eq!(
        maps[0].keycode(Scancode::A, Keymod::SHIFT),
        Keycode::from_char('A')
    );
    assert_eq!(maps[0].keycode(Scancode::Q, Keymod::NONE), Keycode::Q);
    assert_eq!(maps[1].keycode(Scancode::A, Keymod::NONE), Keycode::Q);
    assert_eq!(maps[1].keycode(Scancode::Q, Keymod::NONE), Keycode::A);
    assert_eq!(
        maps[0].keycode(Scancode::RETURN, Keymod::NONE),
        Keycode::RETURN
    );
    assert_eq!(
        maps[0].keycode(Scancode::ESCAPE, Keymod::NONE),
        Keycode::ESCAPE
    );
    assert_eq!(
        maps[0].keycode(Scancode::LSHIFT, Keymod::NONE),
        Keycode::LSHIFT
    );
    // The key no table knows gets a reserved scancode.
    assert_eq!(reserved.len(), 1);
    assert_eq!(reserved[0].key, 250 - 8);
    assert_ne!(
        maps[0].keycode(reserved[0].scancode, Keymod::NONE),
        Keycode::UNKNOWN
    );

    // A virtual keyboard: the scancodes come from the keysyms (of the first
    // layout's levels), so the swapped layout maps a to the Q key.
    let (maps, _) = wayland_build_keymaps(&keymap, &state, &masks, 2, true);
    assert_eq!(maps[0].keycode(Scancode::A, Keymod::NONE), Keycode::A);
    assert_eq!(maps[1].keycode(Scancode::A, Keymod::NONE), Keycode::Q);
}

#[test]
fn generated_protocols_describe_their_interfaces() {
    // The wl_interface tables match the generated descriptions.
    // SAFETY: the names are string literals.
    let name = |i: &wl_interface| {
        unsafe { CStr::from_ptr(i.name) }
            .to_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(name(WlSurface::interface()), WlSurface::NAME);
    assert_eq!(WlSurface::interface().version as u32, WlSurface::VERSION);
    assert_eq!(
        name(super::protocols::xdg_shell::XdgToplevel::interface()),
        "xdg_toplevel"
    );
    let compositor = WlCompositor::interface();
    // (create_surface, create_region and, since version 6, release)
    assert_eq!(compositor.method_count, 3);
    // SAFETY: the method table has method_count entries.
    let create_surface = unsafe { &*compositor.methods };
    // SAFETY: as above.
    assert_eq!(
        unsafe { CStr::from_ptr(create_surface.name) },
        c"create_surface"
    );
    // SAFETY: as above; the types of create_surface are its new surface.
    assert!(std::ptr::eq(
        unsafe { (*create_surface.types).unwrap() },
        WlSurface::interface()
    ));
}

#[test]
fn no_wayland_display_means_no_wayland_device() {
    let _l = crate::test_support::test_lock();
    let no_display = crate::test_support::NoDisplay::new();
    hints::set(hints::VIDEO_DRIVER, "wayland").unwrap();
    let e = init::init(InitFlags::VIDEO).unwrap_err();
    hints::reset(hints::VIDEO_DRIVER);
    drop(no_display);
    assert!(!e.message().is_empty());
}

// ---------------------------------------------------------------------------
// Tests with a compositor
// ---------------------------------------------------------------------------

#[test]
fn wayland_init_and_displays() {
    let _l = crate::test_support::test_lock();
    let Some(_compositor) = Compositor::start(false) else {
        return;
    };
    let _video = Video::init();
    assert_eq!(video::current_video_driver().unwrap(), "wayland");

    let displays = video::displays().unwrap();
    assert!(!displays.is_empty());
    let bounds = video::display_bounds(displays[0]).unwrap();
    assert_eq!((bounds.w, bounds.h), (1024, 768));
    assert!(!video::display_name(displays[0]).unwrap().is_empty());
    let mode = video::desktop_display_mode(displays[0]).unwrap();
    assert_eq!((mode.w, mode.h), (1024, 768));
    assert!(video::display_content_scale(displays[0]).unwrap() >= 1.0);
    // The connection is published for the application.
    assert!(crate::properties::Properties::global()
        .get_number(super::video::PROP_GLOBAL_VIDEO_WAYLAND_WL_DISPLAY_POINTER)
        .is_some_and(|p| p != 0));
    assert!(crate::properties::Properties::global()
        .get_number(super::video::PROP_GLOBAL_VIDEO_WAYLAND_WL_DISPLAY_POINTER)
        .is_some());
}

#[test]
fn wayland_window_lifecycle() {
    let _l = crate::test_support::test_lock();
    let Some(compositor) = Compositor::start(false) else {
        return;
    };
    let _video = Video::init();

    let w = SdlWindow::create(
        "Wayland test",
        320,
        240,
        WindowFlags::HIDDEN | WindowFlags::RESIZABLE,
    )
    .unwrap();
    let id = w.id();
    let props = w.properties().unwrap();
    assert!(props
        .get_number(PROP_WINDOW_WAYLAND_SURFACE_POINTER)
        .is_some_and(|p| p != 0));
    assert_eq!(w.size_in_pixels().unwrap(), (320, 240));

    w.show().unwrap();
    assert!(
        wait_presenting(&w, |e| window_event(e, id, EventType::WINDOW_SHOWN)).is_some()
            || !w.flags().unwrap().contains(WindowFlags::HIDDEN)
    );
    assert!(props
        .get_number(PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_POINTER)
        .is_some_and(|p| p != 0));
    pump_presenting(&w, 100);
    if compositor.kind == Kind::Sway {
        // A floating window keeps its size.
        assert_eq!(w.size().unwrap(), (320, 240));
    }

    // The title goes to the toplevel; the size to the compositor.
    w.set_title("Renamed ✓").unwrap();
    w.set_size(400, 300).unwrap();
    w.sync().unwrap();
    pump_presenting(&w, 200);
    if compositor.kind == Kind::Sway {
        assert_eq!(w.size().unwrap(), (400, 300));
        assert_eq!(w.size_in_pixels().unwrap(), (400, 300));
    }

    // Fullscreen covers the output.
    w.set_fullscreen(true).unwrap();
    w.sync().unwrap();
    assert!(
        wait_presenting(&w, |e| window_event(
            e,
            id,
            EventType::WINDOW_ENTER_FULLSCREEN
        ))
        .is_some()
            || w.flags().unwrap().contains(WindowFlags::FULLSCREEN)
    );
    pump_presenting(&w, 200);
    assert_eq!(w.size().unwrap(), (1024, 768));
    w.set_fullscreen(false).unwrap();
    w.sync().unwrap();
    assert!(
        wait_presenting(&w, |e| window_event(
            e,
            id,
            EventType::WINDOW_LEAVE_FULLSCREEN
        ))
        .is_some()
            || !w.flags().unwrap().contains(WindowFlags::FULLSCREEN)
    );

    // A popup menu over it.
    let popup = SdlWindow::create_popup(&w, 10, 20, 100, 50, WindowFlags::POPUP_MENU).unwrap();
    let popup_id = popup.id();
    assert!(
        wait_presenting(&popup, |e| window_event(
            e,
            popup_id,
            EventType::WINDOW_SHOWN
        ))
        .is_some()
            || !popup.flags().unwrap().contains(WindowFlags::HIDDEN)
    );
    pump_presenting(&w, 100);
    assert_eq!(popup.size().unwrap(), (100, 50));
    popup.set_position(30, 40).unwrap();
    pump_presenting(&w, 100);
    popup.destroy();

    // Hide, show again, and go.
    w.hide().unwrap();
    assert!(
        wait_presenting(&w, |e| window_event(e, id, EventType::WINDOW_HIDDEN)).is_some()
            || w.flags().unwrap().contains(WindowFlags::HIDDEN)
    );
    assert!(props
        .get_number(PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_POINTER)
        .is_none());
    w.show().unwrap();
    assert!(
        wait_presenting(&w, |e| window_event(e, id, EventType::WINDOW_SHOWN)).is_some()
            || !w.flags().unwrap().contains(WindowFlags::HIDDEN)
    );
    w.maximize().unwrap();
    w.sync().unwrap();
    pump_presenting(&w, 100);
    w.restore().unwrap();
    w.sync().unwrap();
    w.destroy();
    pump_for(50);
}

#[test]
fn wayland_framebuffer() {
    let _l = crate::test_support::test_lock();
    let Some(_compositor) = Compositor::start(false) else {
        return;
    };
    let _video = Video::init();

    let w = SdlWindow::create("framebuffer", 64, 48, WindowFlags::default()).unwrap();
    present(&w);
    pump_for(100);
    let surface = w.surface().unwrap();
    for frame in 0..10u8 {
        {
            let mut s = surface.lock();
            assert_eq!((s.width(), s.height()), (64, 48));
            let pixels = s.pixels_mut().unwrap();
            pixels.fill(frame * 20);
        }
        surface.update().unwrap();
        pump_for(20);
    }
    // Updating only parts of it.
    surface
        .update_rects(&[crate::video::Rect {
            x: 0,
            y: 0,
            w: 8,
            h: 8,
        }])
        .unwrap();
    pump_for(50);
    assert!(w.is_valid());
    drop(surface);
    w.destroy_surface().unwrap();
    w.destroy();
}

/// Clear the current context's framebuffer to a color and read a pixel
/// back.
fn gl_clear_and_read(r: f32, g: f32, b: f32) -> [u8; 4] {
    use crate::video::gl::gl_get_proc_address;
    type ClearColor = unsafe extern "C" fn(f32, f32, f32, f32);
    type Clear = unsafe extern "C" fn(u32);
    type Finish = unsafe extern "C" fn();
    type ReadPixels = unsafe extern "C" fn(i32, i32, i32, i32, u32, u32, *mut std::ffi::c_void);
    let get = |name: &str| gl_get_proc_address(name).unwrap_or_else(|| panic!("{name}"));
    let mut pixel = [0u8; 4];
    // SAFETY: the GL functions have these types; a context is current; the
    // buffer holds one RGBA pixel.
    unsafe {
        let clear_color: ClearColor = std::mem::transmute(get("glClearColor"));
        let clear: Clear = std::mem::transmute(get("glClear"));
        let finish: Finish = std::mem::transmute(get("glFinish"));
        let read_pixels: ReadPixels = std::mem::transmute(get("glReadPixels"));
        clear_color(r, g, b, 1.0);
        clear(0x4000); // GL_COLOR_BUFFER_BIT
        finish();
        // GL_RGBA, GL_UNSIGNED_BYTE
        read_pixels(1, 1, 1, 1, 0x1908, 0x1401, pixel.as_mut_ptr().cast());
    }
    pixel
}

#[test]
fn wayland_gles_default_profile() {
    let mut config = crate::video::gl::GlConfig::default();
    super::opengles::wayland_gles_set_default_profile_config(&mut config);
    assert_eq!(config.egl_platform, 0x31D8); // EGL_PLATFORM_WAYLAND_KHR
}

#[test]
fn wayland_egl_contexts() {
    use crate::video::gl::{self, GlAttr, GlContext};
    use crate::video::window::PROP_WINDOW_WAYLAND_EGL_WINDOW_POINTER;
    let _l = crate::test_support::test_lock();
    let Some(_compositor) = Compositor::start(false) else {
        return;
    };
    for es in [false, true] {
        let _video = Video::init();
        if es {
            gl::gl_set_attribute(GlAttr::ContextProfileMask, gl::GL_CONTEXT_PROFILE_ES).unwrap();
            gl::gl_set_attribute(GlAttr::ContextMajorVersion, 2).unwrap();
            gl::gl_set_attribute(GlAttr::ContextMinorVersion, 0).unwrap();
        }
        let what = if es { "OpenGL ES" } else { "OpenGL" };
        let w = match SdlWindow::create(what, 64, 48, WindowFlags::OPENGL) {
            Ok(w) => w,
            Err(e) => {
                eprintln!(
                    "note: no Wayland EGL {what} window ({}); skipping",
                    e.message()
                );
                return;
            }
        };
        let id = w.id();
        let props = w.properties().unwrap();
        assert!(props
            .get_number(PROP_WINDOW_WAYLAND_EGL_WINDOW_POINTER)
            .is_some_and(|p| p != 0));
        assert!(gl::egl_window_surface(&w).unwrap().is_some());
        assert!(gl::egl_current_display().is_ok());
        let context = match GlContext::new(&w) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "note: no Wayland EGL {what} context ({}); skipping",
                    e.message()
                );
                continue;
            }
        };
        assert!(context.is_current());

        // The swaps map the window.
        let mut n = 0u32;
        let shown = wait_for_with(
            3,
            || {
                n += 1;
                assert_eq!(&gl_clear_and_read(1.0, 0.0, 1.0)[..3], &[255, 0, 255]);
                gl::gl_swap_window(&w).unwrap();
            },
            |e| window_event(e, id, EventType::WINDOW_SHOWN),
        );
        assert!(shown.is_some() || !w.flags().unwrap().contains(WindowFlags::HIDDEN));
        pump_for(50);

        // The swap interval is clamped to adaptive vsync, and paced by the
        // frame callback (never longer than 1/20 s a swap).
        gl::gl_set_swap_interval(5).unwrap();
        assert_eq!(gl::gl_get_swap_interval().unwrap(), 1);
        gl::gl_set_swap_interval(-3).unwrap();
        assert_eq!(gl::gl_get_swap_interval().unwrap(), -1);
        let start = Instant::now();
        for frame in 0..10u8 {
            let c = f32::from(frame) / 10.0;
            gl_clear_and_read(c, c, c);
            gl::gl_swap_window(&w).unwrap();
            pump_for(5);
        }
        assert!(start.elapsed() < Duration::from_secs(3));
        gl::gl_set_swap_interval(0).unwrap();
        assert_eq!(gl::gl_get_swap_interval().unwrap(), 0);
        gl::gl_swap_window(&w).unwrap();

        // Resizing resizes the wl_egl_window.
        w.set_size(80, 60).unwrap();
        w.sync().unwrap();
        pump_for(100);
        gl::gl_swap_window(&w).unwrap();

        gl::gl_release_current().unwrap();
        assert!(!context.is_current());
        context.make_current(Some(&w)).unwrap();
        assert_eq!(&gl_clear_and_read(0.0, 1.0, 0.0)[..3], &[0, 255, 0]);
        gl::gl_swap_window(&w).unwrap();

        // A hidden window skips its swaps.
        w.hide().unwrap();
        pump_for(50);
        gl::gl_swap_window(&w).unwrap();

        drop(context);
        w.destroy();
        pump_for(50);
    }
}

#[test]
fn wayland_cursors() {
    let _l = crate::test_support::test_lock();
    let Some(compositor) = Compositor::start(false) else {
        return;
    };
    let _video = Video::init();
    let w = SdlWindow::create("cursors", 200, 150, WindowFlags::default()).unwrap();
    let id = w.id();
    pump_presenting(&w, 100);

    // Move a virtual pointer over the window, so cursors get shown on it.
    let syms = load_symbols().unwrap();
    let client = TestClient::connect(&syms);
    let pointer = if compositor.kind == Kind::Sway {
        client.bind::<VptrManager>(1).map(|manager| {
            let mut args = [
                wl_argument {
                    o: std::ptr::null_mut(),
                },
                wl_argument {
                    o: std::ptr::null_mut(),
                },
            ];
            // SAFETY: the arguments of create_virtual_pointer, "?on".
            let p = unsafe {
                manager
                    .obj()
                    .marshal_constructor::<Vptr>(0, &mut args, 1, 0)
            };
            (manager, p)
        })
    } else {
        None
    };
    if let Some((_, p)) = &pointer {
        // The window is floating in the middle of the output.
        request(
            p.obj(),
            1,
            &mut [
                wl_argument { u: 1 },
                wl_argument { u: 512 },
                wl_argument { u: 384 },
                wl_argument { u: 1024 },
                wl_argument { u: 768 },
            ],
        );
        request(p.obj(), 4, &mut []);
        let _ = client.conn.flush();
        assert!(
            wait_presenting(&w, |e| window_event(e, id, EventType::WINDOW_MOUSE_ENTER)).is_some(),
            "no pointer enter"
        );
    }

    for cursor in [
        SystemCursor::Default,
        SystemCursor::Text,
        SystemCursor::Wait,
        SystemCursor::Pointer,
        SystemCursor::NwseResize,
        SystemCursor::Move,
    ] {
        let c = mouse::create_system_cursor(cursor).unwrap();
        mouse::set_cursor(Some(&c)).unwrap();
        pump_presenting(&w, 20);
        mouse::destroy_cursor(c);
    }

    let mut s = Surface::new(16, 16, PixelFormat::ARGB8888).unwrap();
    {
        let p = s.pixels_mut().unwrap();
        for (i, px) in p.chunks_mut(4).enumerate() {
            let v: u32 = if (i % 16) < 8 { 0xffff0000 } else { 0x00000000 };
            px.copy_from_slice(&v.to_ne_bytes());
        }
    }
    let c = mouse::create_color_cursor(&s, 2, 3).unwrap();
    mouse::set_cursor(Some(&c)).unwrap();
    pump_presenting(&w, 20);
    mouse::hide_cursor();
    assert!(!mouse::cursor_visible());
    pump_presenting(&w, 20);
    mouse::show_cursor();
    assert!(mouse::cursor_visible());
    mouse::set_cursor(mouse::default_cursor().as_ref()).unwrap();
    mouse::destroy_cursor(c);

    if let Some((_, p)) = &pointer {
        // A click goes to the window.
        for state in [1, 0] {
            request(
                p.obj(),
                2,
                &mut [
                    wl_argument { u: 2 },
                    wl_argument { u: BTN_LEFT },
                    wl_argument { u: state },
                ],
            );
            request(p.obj(), 4, &mut []);
        }
        let _ = client.conn.flush();
        assert!(wait_for(
            |e| matches!(e, Event::MouseButton(b) if b.down && b.button == mouse::BUTTON_LEFT)
        )
        .is_some());
    }
    drop(pointer);
    w.destroy();
}

#[test]
fn wayland_keyboard_input() {
    let _l = crate::test_support::test_lock();
    let Some(_compositor) = Compositor::start(true) else {
        return;
    };
    let syms = load_symbols().unwrap();
    let client = TestClient::connect(&syms);
    let Some(mut kbd) = VirtualKeyboard::new(&client) else {
        eprintln!("note: the compositor has no virtual keyboards; skipping");
        return;
    };
    let _video = Video::init();
    let w = SdlWindow::create("keyboard", 200, 150, WindowFlags::default()).unwrap();
    let id = w.id();
    assert!(
        wait_presenting(&w, |e| window_event(e, id, EventType::WINDOW_FOCUS_GAINED)).is_some(),
        "the window didn't get the keyboard focus"
    );

    // The keymap of the (virtual) keyboard.
    assert_eq!(
        crate::events::keyboard::key_from_scancode(Scancode::A, Keymod::NONE, false),
        Keycode::A
    );

    w.start_text_input().unwrap();
    kbd.tap(&client, 30); // KEY_A
    let down = wait_for(|e| matches!(e, Event::Key(k) if k.down)).expect("no key down event");
    match down {
        Event::Key(k) => {
            assert_eq!(k.scancode, Scancode::A);
            assert_eq!(k.key, Keycode::A);
            assert_eq!(k.window_id, id);
        }
        _ => unreachable!(),
    }
    assert!(
        wait_for(|e| matches!(e, Event::Key(k) if !k.down && k.scancode == Scancode::A)).is_some()
    );

    kbd.tap(&client, 28); // KEY_ENTER
    assert!(
        wait_for(|e| matches!(e, Event::Key(k) if k.down && k.key == Keycode::RETURN)).is_some()
    );
    w.stop_text_input().unwrap();
    w.destroy();
}

/// Read a pipe to its end, pumping SDL events meanwhile (SDL may be the
/// one writing it).
fn read_pipe_pumping(read: OwnedFd) -> Vec<u8> {
    let mut data = Vec::new();
    let mut file = std::fs::File::from(read);
    // SAFETY: the descriptor is ours; make it non-blocking.
    unsafe {
        let flags = libc::fcntl(file.as_raw_fd(), libc::F_GETFL);
        libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut buf = [0u8; 4096];
    while Instant::now() < deadline {
        use std::io::Read;
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => data.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                crate::events::pump();
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => break,
        }
    }
    data
}

/// A pipe (read end, write end).
fn pipe() -> (OwnedFd, OwnedFd) {
    let mut fds = [0; 2];
    // SAFETY: room for the two descriptors.
    assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
    // SAFETY: two new descriptors we own.
    unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
}

#[test]
fn wayland_clipboard_between_clients() {
    let _l = crate::test_support::test_lock();
    let Some(_compositor) = Compositor::start(true) else {
        return;
    };
    let syms = load_symbols().unwrap();
    let client = TestClient::connect(&syms);
    let (Some(mut kbd), Some(manager), Some(seat)) = (
        VirtualKeyboard::new(&client),
        client.bind::<DcManager>(1),
        client.bind::<WlSeat>(1),
    ) else {
        eprintln!("note: the compositor lacks virtual keyboards or data control; skipping");
        return;
    };

    // The test client watches the selection.
    type Offers = Arc<Mutex<Vec<(Proxy<DcOffer>, Arc<Mutex<Vec<String>>>)>>>;
    let offers: Offers = Arc::default();
    let selection: Arc<Mutex<Option<usize>>> = Arc::default();
    let mut args = [
        wl_argument {
            o: std::ptr::null_mut(),
        },
        wl_argument { o: seat.raw() },
    ];
    // SAFETY: the arguments of get_data_device, "no".
    let mut device = unsafe {
        manager
            .obj()
            .marshal_constructor::<DcDevice>(1, &mut args, 1, 0)
    };
    {
        let offers = offers.clone();
        let selection = selection.clone();
        device.listen(move |_, ev| match ev {
            DcDeviceEvent::DataOffer(mut offer) => {
                let mimes: Arc<Mutex<Vec<String>>> = Arc::default();
                let m = mimes.clone();
                offer.listen(move |_, DcOfferEvent(mime)| {
                    m.lock().unwrap().push(mime.to_string_lossy().into_owned())
                });
                offers.lock().unwrap().push((offer, mimes));
            }
            DcDeviceEvent::Selection(offer) => {
                *selection.lock().unwrap() = offer.map(|o| o.0.as_ptr() as usize);
            }
            DcDeviceEvent::Finished => {}
        });
    }
    client.conn.roundtrip().unwrap();

    let _video = Video::init();
    let w = SdlWindow::create("clipboard", 200, 150, WindowFlags::default()).unwrap();
    let id = w.id();
    assert!(
        wait_presenting(&w, |e| window_event(e, id, EventType::WINDOW_FOCUS_GAINED)).is_some(),
        "the window didn't get the keyboard focus"
    );

    // A key press gives SDL the input serial that setting the selection needs.
    kbd.tap(&client, 30);
    assert!(wait_presenting(&w, |e| matches!(e, Event::Key(k) if !k.down)).is_some());

    // SDL owns the clipboard: the other client reads it.
    crate::video::clipboard::set_clipboard_text("hello from SDL").unwrap();
    assert_eq!(
        crate::video::clipboard::clipboard_text().unwrap(),
        "hello from SDL"
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    let offer = loop {
        crate::events::pump();
        client.conn.roundtrip().unwrap();
        let current = *selection.lock().unwrap();
        let offer = current.and_then(|raw| {
            let offers = offers.lock().unwrap();
            offers
                .iter()
                .find(|(o, mimes)| o.raw() as usize == raw && !mimes.lock().unwrap().is_empty())
                .map(|(o, mimes)| (o.raw(), mimes.lock().unwrap().clone()))
        });
        if let Some(offer) = offer {
            break offer;
        }
        assert!(
            Instant::now() < deadline,
            "the SDL selection didn't reach the other client"
        );
    };
    assert!(
        offer.1.iter().any(|m| m == "text/plain;charset=utf-8"),
        "{:?}",
        offer.1
    );
    let (read, write) = pipe();
    {
        let offers = offers.lock().unwrap();
        let (o, _) = offers.iter().find(|(o, _)| o.raw() == offer.0).unwrap();
        let mime = c"text/plain;charset=utf-8";
        request(
            o.obj(),
            0,
            &mut [
                wl_argument { s: mime.as_ptr() },
                wl_argument {
                    h: write.as_fd().as_raw_fd(),
                },
            ],
        );
    }
    let _ = client.conn.flush();
    drop(write);
    assert_eq!(read_pipe_pumping(read), b"hello from SDL");

    // The other client owns it: SDL reads it.
    let mut source = {
        let mut args = [wl_argument {
            o: std::ptr::null_mut(),
        }];
        // SAFETY: the arguments of create_data_source, "n".
        unsafe {
            manager
                .obj()
                .marshal_constructor::<DcSource>(0, &mut args, 1, 0)
        }
    };
    source.listen(|_, ev| match ev {
        DcSourceEvent::Send(mime, fd) => {
            let mut f = std::fs::File::from(fd);
            if mime == c"text/plain;charset=utf-8" {
                let _ = f.write_all(b"hello from the other client");
            }
        }
        DcSourceEvent::Cancelled => {}
    });
    let mime = c"text/plain;charset=utf-8";
    request(source.obj(), 0, &mut [wl_argument { s: mime.as_ptr() }]);
    request(device.obj(), 0, &mut [wl_argument { o: source.raw() }]);
    client.conn.roundtrip().unwrap();

    // The other client answers on its own thread while SDL reads.
    let done = Arc::new(AtomicBool::new(false));
    let dispatcher = {
        let conn = client.conn.clone();
        let done = done.clone();
        std::thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                let _ = conn.roundtrip();
                std::thread::sleep(Duration::from_millis(5));
            }
        })
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut text = String::new();
    while Instant::now() < deadline {
        pump_presenting(&w, 20);
        text = crate::video::clipboard::clipboard_text().unwrap_or_default();
        if text == "hello from the other client" {
            break;
        }
    }
    done.store(true, Ordering::Release);
    dispatcher.join().unwrap();
    assert_eq!(text, "hello from the other client");
    assert!(crate::video::clipboard::has_clipboard_text().unwrap());

    w.destroy();
    drop(source);
    drop(device);
}

#[test]
fn wayland_registry_of_the_test_client() {
    let _l = crate::test_support::test_lock();
    let Some(_compositor) = Compositor::start(false) else {
        return;
    };
    let syms = load_symbols().unwrap();
    let client = TestClient::connect(&syms);
    for required in ["wl_compositor", "wl_shm", "xdg_wm_base", "wl_output"] {
        assert!(client.global(required).is_some(), "no {required}");
    }
    // A sync point fires its callback.
    let fired = Arc::new(AtomicBool::new(false));
    let callbacks = super::client::Callbacks::new();
    let f = fired.clone();
    callbacks.add(client.conn.obj().sync(), move |_| {
        f.store(true, Ordering::Release)
    });
    client.conn.roundtrip().unwrap();
    assert!(fired.load(Ordering::Acquire));
    assert_eq!(client.conn.error(), 0);
}
