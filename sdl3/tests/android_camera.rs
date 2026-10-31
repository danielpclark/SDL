// Tests of the Android camera driver in an app.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (`harness = false`), run by `tools/android_run.py`, whose
//! manifest asks for `CAMERA` (not granted at first), and whose main
//! function makes a window and traces the android camera driver (the
//! camera2 NDK) with the emulator's emulated cameras (a back-facing and a
//! front-facing one):
//!
//! * the camera drivers, the one that came up, the cameras (sorted by
//!   name: upstream's list is its hash table's order) with their positions
//!   and formats, and the events that add them;
//! * the back camera opened without the permission: pending, the system's
//!   dialog answered "deny" by the runner (from a thread of the program's,
//!   while SDL's main thread pumps the events), its denial (the event, the
//!   state, no frame), closed;
//! * the permission granted (`pm grant`): the back camera opened in its
//!   best format (approved at once), its format, frames (their size,
//!   format and rotation, the display's portrait orientation and the
//!   sensor's, and that their timestamps advance), closed; the front
//!   camera opened as RGBA at 320x240 (converted and scaled from what it
//!   gives), the same;
//! * the events after each close (none: the camera manager's availability
//!   callbacks, the camera unavailable while the app has it open and
//!   available again after, don't add or remove one);
//! * the subsystem started again: the same cameras, added again.
//!
//! Upstream's assertion that each plane of a frame has the Y plane's row
//! stride fails on the emulator's YUV frames (their chroma planes' strides
//! are half of it): both programs ignore assertions (`SDL_ASSERT`
//! `always_ignore`, as a release build compiles them out), and trace how
//! many failed.
//!
//! What depends on timing isn't traced: the frames' timestamps and
//! contents (the emulated cameras' scene moves), how many frames come, and
//! the application's events (the activity's pause and resume around the
//! dialog).
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! (built with upstream's android-project, Gradle and CMake, its manifest
//! asking for `CAMERA`, its `GatherCameraSpecs()` patched as the
//! translation's: see `camera/android.rs`), run by the runner's
//! `--run-apk`, on the same emulator (API 35, Google APIs, x86-64).
//! Elsewhere this program does nothing.

#[cfg(not(target_os = "android"))]
fn main() {}

/// The runner's words: run as an app, which asks for the camera.
#[cfg(target_os = "android")]
#[used]
#[link_section = ".sdl3_runner"]
static RUNNER: [u8; 23] = *b"app permissions=CAMERA\0";

#[cfg(target_os = "android")]
mod android_support;

#[cfg(target_os = "android")]
fn main() -> std::process::ExitCode {
    android_support::start();
    let status = android_support::checked(app::main);
    android_support::finish(status)
}

#[cfg(target_os = "android")]
mod app {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::camera::{self, Camera, CameraID, CameraPosition, CameraSpec};
    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event, EventType};
    use sdl3::hints::{self, Priority};
    use sdl3::init::{self, InitFlags};
    use sdl3::video::surface::PROP_SURFACE_ROTATION_FLOAT;
    use sdl3::video::{Colorspace, PixelFormat, Window};

    use super::android_support::{check_trace, event_name, request};

    /// Upstream's trace (the C program, on the emulator: API 35, Google
    /// APIs, x86-64).
    const EXPECTED: &str = "\
drivers: android, dummy
init: ok
init: driver: android
window: ok
init: cameras: 2
init: camera Back-facing camera: position=2 formats=8
init: camera Back-facing camera: format=0x3231564e colorspace=0x21100421 1856x1392 30/1
init: camera Back-facing camera: format=0x3231564e colorspace=0x21100421 1280x720 30/1
init: camera Back-facing camera: format=0x3231564e colorspace=0x21100421 1024x768 30/1
init: camera Back-facing camera: format=0x3231564e colorspace=0x21100421 640x480 30/1
init: camera Back-facing camera: format=0x3231564e colorspace=0x21100421 320x240 30/1
init: camera Back-facing camera: format=0x3231564e colorspace=0x21100421 176x144 30/1
init: camera Back-facing camera: format=0x3231564e colorspace=0x21100421 160x120 30/1
init: camera Back-facing camera: format=0x16462004 colorspace=0x120005a0 1600x1200 30/1
init: camera Front-facing camera: position=1 formats=15
init: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 1920x1440 30/1
init: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 1920x1080 30/1
init: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 1920x960 30/1
init: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 1600x1200 30/1
init: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 1440x1080 30/1
init: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 1280x960 30/1
init: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 1280x720 30/1
init: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 1024x768 30/1
init: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 800x600 30/1
init: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 720x480 30/1
init: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 640x480 30/1
init: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 640x360 30/1
init: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 352x288 30/1
init: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 320x240 30/1
init: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 176x144 30/1
init: events: CAMERA_DEVICE_ADDED(Back-facing camera), CAMERA_DEVICE_ADDED(Front-facing camera)
denied: open Back-facing camera: ok
denied: permission: 0
denied: events: CAMERA_DEVICE_DENIED(Back-facing camera)
denied: dialog: tapped permission_deny_button
denied: permission: -1
denied: format: error: Camera permission has not been granted
denied: frame: error: Camera permission has not been granted
denied: closed
denied: closed: events: (none)
back: open Back-facing camera: ok
back: permission: 1
back: events: CAMERA_DEVICE_APPROVED(Back-facing camera)
back: format: format=0x3231564e colorspace=0x21100421 1856x1392 30/1
back: frame: format=0x3231564e 1856x1392 pitch=1856 rotation=90
back: frames advance: yes
back: closed
back: closed: events: (none)
front: open Front-facing camera: ok
front: permission: 1
front: events: CAMERA_DEVICE_APPROVED(Front-facing camera)
front: format: format=0x16762004 colorspace=0x120005a0 320x240 30/1
front: frame: format=0x16762004 320x240 pitch=1280 rotation=270
front: frames advance: yes
front: closed
front: closed: events: (none)
quit
init again: ok
init again: cameras: 2
init again: camera Back-facing camera: position=2 formats=8
init again: camera Back-facing camera: format=0x3231564e colorspace=0x21100421 1856x1392 30/1
init again: camera Back-facing camera: format=0x3231564e colorspace=0x21100421 1280x720 30/1
init again: camera Back-facing camera: format=0x3231564e colorspace=0x21100421 1024x768 30/1
init again: camera Back-facing camera: format=0x3231564e colorspace=0x21100421 640x480 30/1
init again: camera Back-facing camera: format=0x3231564e colorspace=0x21100421 320x240 30/1
init again: camera Back-facing camera: format=0x3231564e colorspace=0x21100421 176x144 30/1
init again: camera Back-facing camera: format=0x3231564e colorspace=0x21100421 160x120 30/1
init again: camera Back-facing camera: format=0x16462004 colorspace=0x120005a0 1600x1200 30/1
init again: camera Front-facing camera: position=1 formats=15
init again: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 1920x1440 30/1
init again: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 1920x1080 30/1
init again: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 1920x960 30/1
init again: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 1600x1200 30/1
init again: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 1440x1080 30/1
init again: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 1280x960 30/1
init again: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 1280x720 30/1
init again: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 1024x768 30/1
init again: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 800x600 30/1
init again: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 720x480 30/1
init again: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 640x480 30/1
init again: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 640x360 30/1
init again: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 352x288 30/1
init again: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 320x240 30/1
init again: camera Front-facing camera: format=0x3231564e colorspace=0x21100421 176x144 30/1
init again: events: CAMERA_DEVICE_ADDED(Back-facing camera), CAMERA_DEVICE_ADDED(Front-facing camera)
assertions: 1
main returns
";

    static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn t(line: impl Into<String>) {
        let line = line.into();
        println!("trace: {line}");
        TRACE.lock().unwrap().push(line);
    }

    fn result<T>(r: &sdl3::Result<T>) -> String {
        match r {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("error: {}", e.message()),
        }
    }

    fn spec_text(s: &CameraSpec) -> String {
        format!(
            "format=0x{:08x} colorspace=0x{:08x} {}x{} {}/{}",
            s.format.0,
            s.colorspace.0,
            s.width,
            s.height,
            s.framerate_numerator,
            s.framerate_denominator
        )
    }

    /// A camera's name, the trace's label of it.
    fn label(id: CameraID) -> String {
        camera::camera_name(id).unwrap_or_else(|_| format!("(camera {id})"))
    }

    /// The cameras, sorted by name.
    fn trace_cameras(step: &str) -> Vec<CameraID> {
        let mut ids = camera::cameras().unwrap_or_default();
        ids.sort_by_key(|&id| label(id));
        t(format!("{step}: cameras: {}", ids.len()));
        for &id in &ids {
            let formats = camera::camera_supported_formats(id).unwrap_or_default();
            t(format!(
                "{step}: camera {}: position={} formats={}",
                label(id),
                camera::camera_position(id) as i32,
                formats.len()
            ));
            for spec in &formats {
                t(format!("{step}: camera {}: {}", label(id), spec_text(spec)));
            }
        }
        ids
    }

    /// The camera events, pumped for `ms`, or until one of type `until`.
    fn camera_events(step: &str, ms: u64, until: Option<EventType>) {
        let end = Instant::now() + Duration::from_millis(ms);
        let mut seen = Vec::new();
        let mut done = false;
        while !done && Instant::now() < end {
            events::pump();
            while let Some(event) = events::poll() {
                if let Event::CameraDevice(e) = event {
                    seen.push(format!("{}({})", event_name(e.event_type), label(e.which)));
                    if Some(e.event_type) == until {
                        done = true;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        t(format!(
            "{step}: events: {}",
            if seen.is_empty() {
                "(none)".to_string()
            } else {
                seen.join(", ")
            }
        ));
    }

    /// Frames of an approved camera: the first one's size, format and
    /// rotation, and whether the timestamps of three advance.
    fn trace_frames(step: &str, camera: &Camera) {
        let end = Instant::now() + Duration::from_secs(15);
        let mut stamps = Vec::new();
        while stamps.len() < 3 && Instant::now() < end {
            events::pump();
            match camera.acquire_frame() {
                Ok(Some(mut frame)) => {
                    if stamps.is_empty() {
                        let rotation = frame
                            .properties()
                            .get_float(PROP_SURFACE_ROTATION_FLOAT)
                            .unwrap_or(0.0);
                        t(format!(
                            "{step}: frame: format=0x{:08x} {}x{} pitch={} rotation={rotation:.0}",
                            frame.format().0,
                            frame.width(),
                            frame.height(),
                            frame.pitch()
                        ));
                    }
                    stamps.push(frame.timestamp_ns());
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                Err(e) => {
                    t(format!("{step}: frame: error: {}", e.message()));
                    return;
                }
            }
        }
        if stamps.len() < 3 {
            t(format!("{step}: frames: {} in 15 s", stamps.len()));
        } else {
            t(format!(
                "{step}: frames advance: {}",
                if stamps.windows(2).all(|w| w[1] > w[0]) {
                    "yes"
                } else {
                    "no"
                }
            ));
        }
    }

    fn open(step: &str, id: Option<CameraID>, spec: Option<&CameraSpec>) -> Option<Camera> {
        let Some(id) = id else {
            t(format!("{step}: no camera"));
            return None;
        };
        let r = Camera::open(id, spec);
        t(format!("{step}: open {}: {}", label(id), result(&r)));
        let camera = r.ok()?;
        t(format!(
            "{step}: permission: {}",
            camera.permission_state() as i32
        ));
        Some(camera)
    }

    fn by_position(ids: &[CameraID], position: CameraPosition) -> Option<CameraID> {
        ids.iter()
            .copied()
            .find(|&id| camera::camera_position(id) == position)
    }

    pub fn main() {
        // (upstream's assertion that each plane of a frame has the Y
        // plane's row stride fails on the emulator's YUV frames, whose
        // chroma planes' are half of it: assertions are ignored, as a
        // release build compiles them out, and counted)
        hints::set_with_priority(hints::ASSERT, Some("always_ignore"), Priority::Override).unwrap();
        let drivers: Vec<&str> = (0..camera::num_camera_drivers())
            .filter_map(|i| camera::camera_driver(i).ok())
            .collect();
        t(format!("drivers: {}", drivers.join(", ")));
        let r = init::init(InitFlags::VIDEO | InitFlags::CAMERA);
        t(format!("init: {}", result(&r)));
        t(format!(
            "init: driver: {}",
            camera::current_camera_driver().unwrap_or("(null)")
        ));
        let window = Window::create("SDL camera", 320, 240, WindowFlags::RESIZABLE);
        t(format!("window: {}", result(&window)));
        // (the cameras the availability callbacks would add, if any)
        events::pump();
        std::thread::sleep(Duration::from_millis(500));
        let ids = trace_cameras("init");
        camera_events("init", 1000, None);
        let back = by_position(&ids, CameraPosition::BackFacing);
        let front = by_position(&ids, CameraPosition::FrontFacing);

        // Denied (the system's dialog)
        if let Some(camera) = open("denied", back, None) {
            let thread = std::thread::Builder::new()
                .name("ask".into())
                .spawn(|| request("permission-dialog deny"))
                .unwrap();
            camera_events("denied", 30_000, Some(EventType::CAMERA_DEVICE_DENIED));
            let answer = thread.join().expect("the request's thread");
            for line in answer {
                t(format!("denied: {line}"));
            }
            t(format!(
                "denied: permission: {}",
                camera.permission_state() as i32
            ));
            t(format!("denied: format: {}", result(&camera.format())));
            t(format!(
                "denied: frame: {}",
                result(&camera.acquire_frame())
            ));
            drop(camera);
            t("denied: closed");
            camera_events("denied: closed", 1000, None);
        }

        // Granted
        for line in request("permission grant CAMERA") {
            t(format!("granted: {line}"));
        }
        if let Some(camera) = open("back", back, None) {
            camera_events("back", 5000, Some(EventType::CAMERA_DEVICE_APPROVED));
            match camera.format() {
                Ok(spec) => t(format!("back: format: {}", spec_text(&spec))),
                Err(e) => t(format!("back: format: error: {}", e.message())),
            }
            trace_frames("back", &camera);
            drop(camera);
            t("back: closed");
            camera_events("back: closed", 1500, None);
        }
        let rgba = CameraSpec {
            format: PixelFormat::RGBA32,
            colorspace: Colorspace::SRGB,
            width: 320,
            height: 240,
            framerate_numerator: 30,
            framerate_denominator: 1,
        };
        if let Some(camera) = open("front", front, Some(&rgba)) {
            camera_events("front", 5000, Some(EventType::CAMERA_DEVICE_APPROVED));
            match camera.format() {
                Ok(spec) => t(format!("front: format: {}", spec_text(&spec))),
                Err(e) => t(format!("front: format: error: {}", e.message())),
            }
            trace_frames("front", &camera);
            drop(camera);
            t("front: closed");
            camera_events("front: closed", 1500, None);
        }

        init::quit_subsystem(InitFlags::CAMERA);
        t("quit");
        let r = init::init_subsystem(InitFlags::CAMERA);
        t(format!("init again: {}", result(&r)));
        std::thread::sleep(Duration::from_millis(500));
        trace_cameras("init again");
        camera_events("init again", 1000, None);
        drop(window);
        t(format!(
            "assertions: {}",
            sdl3::assert::assertion_report().len()
        ));
        init::quit();
        t("main returns");
        let trace = TRACE.lock().unwrap().clone();
        check_trace(&trace, EXPECTED);
    }
}
