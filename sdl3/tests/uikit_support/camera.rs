// The body of the UIKit camera programs (uikit_camera, uikit_camera_denied).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app the runner grants (`uikit_camera`) or denies
//! (`uikit_camera_denied`) the camera traces, in its main function (with
//! the main run loop running between the steps):
//!
//! * the app's state, the status bar's orientation and `UIDevice`'s,
//!   before, while and after it generates orientation notifications (what
//!   the CoreMedia driver tracks while a camera is open);
//! * `+[AVCaptureDevice authorizationStatusForMediaType:]` for video,
//!   before and after;
//! * the camera drivers, CoreMedia starting, its cameras (none on the
//!   simulator) and the camera events;
//! * cameras that aren't there opened (with and without a spec), their
//!   formats, names and positions;
//! * the dummy driver asked for, and CoreMedia again.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! run the same way on the same simulator (an iPhone 16 with iOS 18.5).
//!
//! On a device with cameras (a hardware check, docs/HARDWARE_TESTING.md)
//! it doesn't compare the trace: it opens the first back camera (else the
//! first), waits for the person to allow it, and takes frames for ten
//! seconds, checking each one's size, format, timestamp and rotation (the
//! rotation that makes it upright for the interface's and the device's
//! orientations then), and printing them as the device turns.

use std::ffi::c_void;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sdl3::camera::{self, Camera, CameraID, CameraPermissionState, CameraPosition, CameraSpec};
use sdl3::events::{self, Event, EventType};
use sdl3::hints;
use sdl3::init::{self, InitFlags};
use sdl3::video::surface::PROP_SURFACE_ROTATION_FLOAT;
use sdl3::video::{Colorspace, PixelFormat};

use super::objc::{self, Id};
use super::{check_trace, checked, elapsed, exit};

static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn t(line: impl Into<String>) {
    let line = line.into();
    println!("trace: {line}");
    TRACE.lock().unwrap().push(line);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: *const c_void;
    fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after: u8) -> i32;
}

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVMediaTypeVideo: Id;
}

/// Run the main thread's run loop for `ms` milliseconds.
fn run_loop(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        // SAFETY: the main thread's run loop, in its default mode.
        unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, 0) };
    }
}

fn application_state() -> isize {
    // SAFETY: a UIApplication property.
    unsafe { objc::send0(objc::shared_application(), "applicationState") }
}

/// `[UIApplication sharedApplication].statusBarOrientation` (what the
/// driver takes the interface's orientation from before iOS 27).
fn status_bar_orientation() -> isize {
    // SAFETY: a UIApplication property.
    unsafe { objc::send0(objc::shared_application(), "statusBarOrientation") }
}

fn uidevice() -> Id {
    // SAFETY: UIDevice's class method.
    unsafe { objc::send0(objc::class("UIDevice"), "currentDevice") }
}

fn device_orientation() -> isize {
    // SAFETY: a UIDevice property.
    unsafe { objc::send0(uidevice(), "orientation") }
}

fn generating() -> i32 {
    // SAFETY: a UIDevice property (a BOOL).
    let b: bool = unsafe { objc::send0(uidevice(), "isGeneratingDeviceOrientationNotifications") };
    b as i32
}

/// `+[AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo]`.
fn authorization_status() -> isize {
    // SAFETY: AVCaptureDevice's class method with AVFoundation's constant.
    unsafe {
        objc::send1(
            objc::class("AVCaptureDevice"),
            "authorizationStatusForMediaType:",
            AVMediaTypeVideo,
        )
    }
}

/// The camera events queued (pumped): "A(id), B(id)" or "(none)".
fn camera_events(label: &str) {
    events::pump();
    let mut seen = Vec::new();
    while let Some(event) = events::poll() {
        if let Event::CameraDevice(e) = event {
            let name = match e.event_type {
                EventType::CAMERA_DEVICE_ADDED => "CAMERA_DEVICE_ADDED",
                EventType::CAMERA_DEVICE_REMOVED => "CAMERA_DEVICE_REMOVED",
                EventType::CAMERA_DEVICE_APPROVED => "CAMERA_DEVICE_APPROVED",
                _ => "CAMERA_DEVICE_DENIED",
            };
            seen.push(format!("{name}({})", e.which));
        }
    }
    t(format!(
        "{label}: camera events: {}",
        if seen.is_empty() {
            "(none)".to_string()
        } else {
            seen.join(", ")
        }
    ));
}

fn driver() -> &'static str {
    camera::current_camera_driver().unwrap_or("(null)")
}

fn ok_or_message(r: sdl3::Result<()>) -> String {
    r.err().map_or("ok".into(), |e| e.message().to_string())
}

/// The C programs' `camera_checks()`.
fn camera_checks(label: &str) {
    t(format!(
        "{label}: authorization status: {}",
        authorization_status()
    ));
    let drivers: Vec<&str> = (0..camera::num_camera_drivers())
        .map(|i| camera::camera_driver(i).unwrap())
        .collect();
    t(format!("{label}: camera drivers: {}", drivers.join(", ")));
    let r = init::init_subsystem(InitFlags::CAMERA);
    t(format!("{label}: init camera: {}", ok_or_message(r)));
    t(format!("{label}: camera driver: {}", driver()));
    let ids = camera::cameras();
    t(format!(
        "{label}: cameras: {}",
        ids.as_ref().map_or(-1, |ids| ids.len() as i32)
    ));
    for (i, &id) in ids.iter().flatten().enumerate() {
        t(format!(
            "{label}: camera {i}: {} position={} formats={}",
            camera::camera_name(id).unwrap_or_default(),
            camera::camera_position(id) as i32,
            camera::camera_supported_formats(id).map_or(-1, |s| s.len() as i32)
        ));
    }
    camera_events(label);

    for id in [0, 12345] {
        let opened = |r: sdl3::Result<Camera>| match r {
            Ok(_) => "opened".to_string(),
            Err(e) => e.message().to_string(),
        };
        t(format!(
            "{label}: open {id}: {}",
            opened(Camera::open(id, None))
        ));
        let spec = CameraSpec {
            format: PixelFormat::NV12,
            colorspace: Colorspace::BT709_LIMITED,
            width: 640,
            height: 480,
            framerate_numerator: 30,
            framerate_denominator: 1,
        };
        t(format!(
            "{label}: open {id} with a spec: {}",
            opened(Camera::open(id, Some(&spec)))
        ));
        let formats = camera::camera_supported_formats(id);
        t(format!(
            "{label}: supported formats {id}: {} count={}",
            formats
                .as_ref()
                .map_or_else(|e| e.message().to_string(), |_| "a list".into()),
            formats.as_ref().map_or(0, |f| f.len())
        ));
        t(format!(
            "{label}: name {id}: {}",
            camera::camera_name(id).unwrap_or_else(|e| e.message().to_string())
        ));
        t(format!(
            "{label}: position {id}: {}",
            camera::camera_position(id) as i32
        ));
    }
    camera_events(label);
    init::quit_subsystem(InitFlags::CAMERA);
    t(format!("{label}: quit camera: driver={}", driver()));

    for (name, what) in [("dummy", "dummy"), ("coremedia", "coremedia again")] {
        hints::set(hints::CAMERA_DRIVER, name).unwrap();
        let r = init::init_subsystem(InitFlags::CAMERA);
        let ids = camera::cameras();
        t(format!(
            "{label}: {what}: init {} driver={} cameras={}",
            ok_or_message(r),
            driver(),
            ids.as_ref().map_or(-1, |ids| ids.len() as i32)
        ));
        init::quit_subsystem(InitFlags::CAMERA);
        hints::reset(hints::CAMERA_DRIVER);
    }
    t(format!(
        "{label}: authorization status: {}",
        authorization_status()
    ));
}

/// Run the program: `expected` is upstream's trace on the simulator.
pub fn main(program: &'static str, expected: &'static str) {
    elapsed();
    let status = sdl3::app::run_app(move |_args| {
        let status = checked(|| main_function(program, expected));
        exit(status);
    });
    println!("{program}: run_app returned {status}, which it never does");
    exit(1);
}

fn main_function(program: &str, expected: &str) {
    println!("{program}: {:.3} s: main", elapsed());
    let end = Instant::now() + Duration::from_secs(30);
    while application_state() != 0 && Instant::now() < end {
        run_loop(10);
    }
    t(format!(
        "app state: {}",
        if application_state() == 0 {
            "active"
        } else {
            "not active"
        }
    ));
    t(format!(
        "status bar orientation: {}",
        status_bar_orientation()
    ));
    t(format!(
        "UIDevice orientation: {} generating={}",
        device_orientation(),
        generating()
    ));
    // SAFETY: UIDevice's methods without arguments.
    unsafe {
        let () = objc::send0(uidevice(), "beginGeneratingDeviceOrientationNotifications");
    }
    run_loop(200);
    t(format!(
        "UIDevice orientation (tracking): {} generating={}",
        device_orientation(),
        generating()
    ));
    // SAFETY: as above.
    unsafe {
        let () = objc::send0(uidevice(), "endGeneratingDeviceOrientationNotifications");
    }
    t(format!(
        "UIDevice orientation tracking ended: generating={}",
        generating()
    ));

    if cfg!(not(target_abi = "sim")) && hardware(program) {
        // (a device with cameras: the hardware check ran instead)
        init::quit();
        return;
    }

    camera_checks("app");
    run_loop(100);
    camera_events("app: end");
    t(format!(
        "UIDevice generating orientation notifications: {}",
        generating()
    ));
    init::quit();
    t("done");
    check_trace(&TRACE.lock().unwrap(), expected);
}

/// `COREMEDIA_AcquireFrame()`'s rotation for a camera at `back` (else the
/// front's), with the interface in `ui` and the device in `device` (both
/// valid), from upstream's tables, which `camera::coremedia`'s tests check
/// against its C.
fn expected_rotation(back: bool, ui: isize, device: isize) -> f32 {
    const BACK: [[u16; 4]; 4] = [
        [90, 90, 90, 90],
        [270, 270, 270, 270],
        [0, 0, 0, 0],
        [180, 180, 180, 180],
    ];
    const FRONT: [[u16; 4]; 4] = [
        [90, 90, 270, 270],
        [270, 270, 90, 90],
        [0, 0, 180, 180],
        [180, 180, 0, 0],
    ];
    let table = if back { &BACK } else { &FRONT };
    table[(ui - 1) as usize][(device - 1) as usize] as f32
}

/// The hardware check, on a device with cameras: false (nothing done) when
/// CoreMedia finds none.
fn hardware(program: &str) -> bool {
    init::init_subsystem(InitFlags::CAMERA).unwrap();
    assert_eq!(camera::current_camera_driver(), Some("coremedia"));
    let ids: Vec<CameraID> = camera::cameras().unwrap();
    if ids.is_empty() {
        init::quit_subsystem(InitFlags::CAMERA);
        return false;
    }
    println!(
        "{program}: hardware: authorization status {}",
        authorization_status()
    );
    for &id in &ids {
        let specs = camera::camera_supported_formats(id).unwrap();
        println!(
            "{program}: hardware: camera {id}: {} position={:?} formats={}",
            camera::camera_name(id).unwrap(),
            camera::camera_position(id),
            specs.len()
        );
        assert!(!specs.is_empty());
    }
    let id = ids
        .iter()
        .copied()
        .find(|&id| camera::camera_position(id) == CameraPosition::BackFacing)
        .unwrap_or(ids[0]);
    let back = camera::camera_position(id) == CameraPosition::BackFacing;
    let cam = Camera::open(id, None).unwrap();
    let spec = cam.format().unwrap();
    println!("{program}: hardware: opened {id}: {spec:?}; allow the camera if asked");
    // (the system asks the person the first time; the driver reports the
    // answer with the first frame after it)
    let end = Instant::now() + Duration::from_secs(120);
    while cam.permission_state() == CameraPermissionState::Pending && Instant::now() < end {
        run_loop(10);
        // (frames before the answer are dropped)
        let _ = cam.acquire_frame();
    }
    let state = cam.permission_state();
    println!("{program}: hardware: permission {state:?}");
    assert_eq!(
        state,
        CameraPermissionState::Approved,
        "the camera wasn't allowed"
    );
    // SAFETY: UIDevice's method without arguments, while the frames come.
    unsafe {
        let () = objc::send0(uidevice(), "beginGeneratingDeviceOrientationNotifications");
    }
    let start = Instant::now();
    let (mut frames, mut last_ts, mut last_print) = (0, 0u64, Instant::now());
    let mut last_device = 1;
    let mut mismatches = 0;
    while start.elapsed() < Duration::from_secs(10) {
        run_loop(5);
        let ui = status_bar_orientation();
        let mut device = device_orientation();
        let Some(mut frame) = cam.acquire_frame().unwrap() else {
            continue;
        };
        let rotation = frame
            .properties()
            .get_float(PROP_SURFACE_ROTATION_FLOAT)
            .unwrap_or_default();
        assert_eq!(
            (frame.width(), frame.height(), frame.format()),
            (spec.width, spec.height, spec.format)
        );
        assert!(frame.timestamp_ns() > last_ts, "timestamps advance");
        last_ts = frame.timestamp_ns();
        if (1..=4).contains(&device) {
            last_device = device;
        } else {
            device = last_device;
        }
        // (the orientations read just before the frame came: they can
        // change in between as the device turns)
        let expected = if (1..=4).contains(&ui) {
            expected_rotation(back, ui, device)
        } else {
            0.0
        };
        if rotation != expected {
            mismatches += 1;
        }
        assert!([0.0, 90.0, 180.0, 270.0].contains(&rotation), "{rotation}");
        frames += 1;
        if last_print.elapsed() > Duration::from_secs(1) {
            println!(
                "{program}: hardware: {frames} frames; interface {ui}, device {device}: \
                 rotation {rotation} (expected {expected})"
            );
            last_print = Instant::now();
        }
        drop(frame);
    }
    // SAFETY: as above.
    unsafe {
        let () = objc::send0(uidevice(), "endGeneratingDeviceOrientationNotifications");
    }
    println!(
        "{program}: hardware: {frames} frames in 10 s, {mismatches} with another rotation than \
         the orientations read before it give"
    );
    assert!(frames >= 10, "frames came");
    assert!(mismatches <= 2, "the rotations follow the orientations");
    drop(cam);
    init::quit_subsystem(InitFlags::CAMERA);
    true
}
