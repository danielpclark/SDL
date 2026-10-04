// Tests for the camera front end, driven by a test backend that produces
// frames.

use std::sync::atomic::{AtomicI32, AtomicU32, AtomicUsize};

use super::*;
use crate::events::{flush_events, get_events, pump};
use crate::init::{self, InitFlags};
use crate::video::Color;

// The test backend's knobs and counters (the tests run one at a time).
/// Frames acquired after this many fail (the device is lost).
static FAIL_AT: AtomicU32 = AtomicU32::new(u32::MAX);
/// 1: approve on open, -1: deny, 0: leave pending.
static PERMISSION: AtomicI32 = AtomicI32::new(1);
static RELEASED: AtomicUsize = AtomicUsize::new(0);
static CLOSED: AtomicUsize = AtomicUsize::new(0);
static FREED_HANDLES: AtomicUsize = AtomicUsize::new(0);

fn spec(format: PixelFormat, width: i32, height: i32, num: i32, den: i32) -> CameraSpec {
    let colorspace = if format.is_fourcc() {
        Colorspace::BT709_LIMITED
    } else {
        Colorspace::SRGB
    };
    CameraSpec {
        format,
        colorspace,
        width,
        height,
        framerate_numerator: num,
        framerate_denominator: den,
    }
}

fn camera_a_specs() -> Vec<CameraSpec> {
    vec![
        spec(PixelFormat::XRGB8888, 320, 240, 15, 1),
        spec(PixelFormat::XRGB8888, 640, 480, 30, 1),
        spec(PixelFormat::XRGB8888, 640, 480, 30, 1),
        spec(PixelFormat::RGB565, 640, 480, 30, 1),
        spec(PixelFormat::XRGB8888, 320, 240, 30, 1),
        spec(PixelFormat::NV12, 640, 480, 30, 1),
        spec(PixelFormat::XRGB8888, 640, 360, 30, 1),
        spec(PixelFormat::XRGB8888, 160, 120, 30, 1),
    ]
}

struct TestDriver;

impl CameraDriverImpl for TestDriver {
    fn detect_devices(&self) {
        add_camera(
            "Test Camera A",
            CameraPosition::FrontFacing,
            &camera_a_specs(),
            Box::new(0u32),
        );
        add_camera(
            "Test Camera B",
            CameraPosition::BackFacing,
            &[],
            Box::new(1u32),
        );
    }

    fn open_device(
        &self,
        device: &Arc<CameraDevice>,
        spec: &CameraSpec,
    ) -> Result<Arc<dyn CameraBackend>> {
        match PERMISSION.load(Ordering::SeqCst) {
            1 => camera_permission_outcome(device, true),
            -1 => camera_permission_outcome(device, false),
            _ => {}
        }
        Ok(Arc::new(TestBackend {
            spec: *spec,
            frame: AtomicU32::new(0),
        }))
    }

    fn free_device_handle(&self, _device: &CameraDevice) {
        FREED_HANDLES.fetch_add(1, Ordering::SeqCst);
    }
}

/// Produces frames whose pixels all hold the frame number.
struct TestBackend {
    spec: CameraSpec,
    frame: AtomicU32,
}

impl CameraBackend for TestBackend {
    fn wait_device(&self, _device: &CameraDevice) -> bool {
        crate::timer::delay(Duration::from_millis(2));
        true
    }

    fn acquire_frame(&self, _device: &CameraDevice) -> CameraFrameResult {
        let n = self.frame.fetch_add(1, Ordering::SeqCst);
        if n >= FAIL_AT.load(Ordering::SeqCst) {
            return CameraFrameResult::Error;
        }
        let (w, h) = (self.spec.width as usize, self.spec.height as usize);
        let (pixels, pitch) = match self.spec.format {
            PixelFormat::NV12 => {
                let mut p = vec![128u8; w * h + w * h / 2];
                p[..w * h].fill(16 + (n % 200) as u8);
                (p, w)
            }
            PixelFormat::RGB565 => (
                (0..w * h).flat_map(|_| (n as u16).to_ne_bytes()).collect(),
                w * 2,
            ),
            _ => ((0..w * h).flat_map(|_| n.to_ne_bytes()).collect(), w * 4),
        };
        CameraFrameResult::Ready(AcquiredFrame {
            pixels,
            pitch: pitch as i32,
            timestamp_ns: 5_000_000_000 + n as u64 * 1_000_000,
            rotation: 90.0,
        })
    }

    fn release_frame(&self, _device: &CameraDevice, pixels: Vec<u8>) {
        assert!(!pixels.is_empty());
        RELEASED.fetch_add(1, Ordering::SeqCst);
    }

    fn close_device(&self, _device: &CameraDevice) {
        CLOSED.fetch_add(1, Ordering::SeqCst);
    }
}

fn testcamera_init() -> Result<Arc<dyn CameraDriverImpl>> {
    Ok(Arc::new(TestDriver))
}

pub(super) static TESTCAMERA_BOOTSTRAP: CameraBootStrap = CameraBootStrap {
    name: "test",
    desc: "SDL test camera driver",
    init: testcamera_init,
    demand_only: true,
};

/// Start the camera subsystem with the test driver; returns the two
/// cameras.
fn start() -> (CameraID, CameraID) {
    FAIL_AT.store(u32::MAX, Ordering::SeqCst);
    PERMISSION.store(1, Ordering::SeqCst);
    hints::set(hints::CAMERA_DRIVER, "TEST").unwrap();
    init::init_subsystem(InitFlags::CAMERA).unwrap();
    hints::reset(hints::CAMERA_DRIVER);
    let ids = cameras().unwrap();
    assert_eq!(ids.len(), 2);
    (ids[0], ids[1])
}

fn stop() {
    init::quit_subsystem(InitFlags::CAMERA);
    assert_eq!(current_camera_driver(), None);
}

/// Wait (up to two seconds) for a frame.
fn next_frame(camera: &Camera) -> CameraFrame {
    let start = crate::timer::ticks_ms();
    loop {
        if let Some(frame) = camera.acquire_frame().unwrap() {
            return frame;
        }
        assert!(crate::timer::ticks_ms() - start < 2000, "no camera frame");
        crate::timer::delay(Duration::from_millis(1));
    }
}

fn camera_events() -> Vec<(EventType, CameraID)> {
    pump();
    get_events(
        EventType::CAMERA_DEVICE_FIRST,
        EventType::CAMERA_DEVICE_LAST,
        100,
    )
    .unwrap()
    .into_iter()
    .map(|e| match e {
        Event::CameraDevice(e) => (e.event_type, e.which),
        other => panic!("unexpected event {other:?}"),
    })
    .collect()
}

#[test]
fn driver_selection() {
    let _l = crate::test_support::test_lock();
    // The test driver, the platform drivers in upstream's order, dummy.
    let names: Vec<&str> = (0..num_camera_drivers())
        .map(|i| camera_driver(i).unwrap())
        .collect();
    #[cfg(target_os = "linux")]
    assert_eq!(names, ["test", "v4l2", "pipewire", "dummy"]);
    #[cfg(windows)]
    assert_eq!(names, ["test", "mediafoundation", "dummy"]);
    #[cfg(not(any(target_os = "linux", windows)))]
    assert_eq!(names, ["test", "dummy"]);
    assert!(camera_driver(names.len()).is_err());
    assert_eq!(
        cameras().unwrap_err().message(),
        "Camera subsystem is not initialized"
    );

    // Without a hint, the first platform driver that starts is used (the
    // test and dummy drivers are demand-only); without one, none is.
    hints::reset(hints::CAMERA_DRIVER);
    match init::init_subsystem(InitFlags::CAMERA) {
        Ok(()) => {
            let name = current_camera_driver().unwrap();
            assert!(name != "test" && name != "dummy", "{name}");
            stop();
        }
        Err(e) => {
            assert!(names.len() == 2 || cfg!(windows), "{}", e.message()); // (V4L2 always starts)
            if names.len() == 2 {
                assert_eq!(e.message(), "No available camera driver");
            }
            assert_eq!(current_camera_driver(), None);
        }
    }
    hints::set(hints::CAMERA_DRIVER, "nonexistent").unwrap();
    assert_eq!(
        init::init_subsystem(InitFlags::CAMERA)
            .unwrap_err()
            .message(),
        "Camera driver 'nonexistent' not available"
    );
    assert_eq!(init::was_init(InitFlags::CAMERA), InitFlags::NONE);

    hints::set(hints::CAMERA_DRIVER, ",dummy").unwrap();
    assert!(init::init_subsystem(InitFlags::CAMERA).is_err());

    hints::set(hints::CAMERA_DRIVER, "nonexistent,dummy").unwrap();
    init::init_subsystem(InitFlags::CAMERA).unwrap();
    assert_eq!(current_camera_driver(), Some("dummy"));
    assert!(cameras().unwrap().is_empty());
    assert_eq!(
        Camera::open(1, None).unwrap_err().message(),
        "Invalid camera device instance ID"
    );
    hints::reset(hints::CAMERA_DRIVER);
    stop();
}

#[test]
fn zombie_frame_pacing() {
    let d = |num, den| zombie_frame_delay_ms(&spec(PixelFormat::XRGB8888, 1, 1, num, den));
    assert_eq!(d(30, 1), 33);
    assert_eq!(d(15, 1), 66);
    assert_eq!(d(30000, 1001), 33);
    assert_eq!(d(1, 2), 2000);
    // No usable framerate: one frame interval at the fallback rate, not a
    // zero (or undefined) delay that would spin the zombie thread.
    let fallback = 1000 / ZOMBIE_FALLBACK_FPS;
    assert!(fallback > 0);
    assert_eq!(zombie_frame_delay_ms(&CameraSpec::default()), fallback);
    assert_eq!(d(0, 1), fallback);
    assert_eq!(d(30, 0), fallback);
    assert_eq!(d(-30, 1), fallback);
}

#[test]
fn spec_sorting_and_choice() {
    let mut specs = camera_a_specs();
    specs.sort_by(camera_spec_cmp);
    specs.dedup();
    let x = PixelFormat::XRGB8888;
    assert_eq!(
        specs,
        vec![
            spec(PixelFormat::NV12, 640, 480, 30, 1),
            spec(x, 640, 480, 30, 1),
            spec(x, 640, 360, 30, 1),
            spec(x, 320, 240, 30, 1),
            spec(x, 320, 240, 15, 1),
            spec(x, 160, 120, 30, 1),
            spec(PixelFormat::RGB565, 640, 480, 30, 1),
        ]
    );

    // Full range before limited range, all else being equal.
    let mut limited = spec(x, 160, 120, 30, 1);
    limited.colorspace = Colorspace::BT709_LIMITED;
    assert_eq!(
        camera_spec_cmp(&spec(x, 160, 120, 30, 1), &limited),
        CmpOrdering::Less
    );

    assert_eq!(choose_best_camera_spec(&specs, None), specs[0]);
    let want = spec(x, 320, 240, 30, 1);
    assert_eq!(choose_best_camera_spec(&specs, Some(&want)), want);

    // Closest aspect ratio, then the first format of that size, then the
    // closest framerate (to 0 FPS when none is asked for).
    let mut want = CameraSpec {
        format: PixelFormat::RGBA8888,
        width: 1280,
        height: 720,
        ..Default::default()
    };
    assert_eq!(
        choose_best_camera_spec(&specs, Some(&want)),
        spec(x, 640, 360, 30, 1)
    );
    want.width = 320;
    want.height = 240;
    assert_eq!(
        choose_best_camera_spec(&specs, Some(&want)),
        spec(x, 320, 240, 15, 1)
    );

    let want = spec(PixelFormat::RGB565, 640, 480, 15, 1);
    assert_eq!(
        choose_best_camera_spec(&specs, Some(&want)),
        spec(PixelFormat::RGB565, 640, 480, 30, 1)
    );

    // Any spec goes for a device that lists none.
    assert_eq!(choose_best_camera_spec(&[], Some(&want)), want);
    assert_eq!(choose_best_camera_spec(&[], None), CameraSpec::default());
}

#[test]
fn devices_and_events() {
    let _l = crate::test_support::test_lock();
    let (a, b) = start();
    flush_events(EventType::FIRST, EventType::LAST);
    // (the events were queued when the devices were detected)
    init::quit_subsystem(InitFlags::CAMERA);

    let (a2, b2) = start();
    assert!(b > a && a2 > b && b2 > a2);
    assert_eq!(
        camera_events(),
        vec![
            (EventType::CAMERA_DEVICE_ADDED, a2),
            (EventType::CAMERA_DEVICE_ADDED, b2)
        ]
    );
    assert_eq!(camera_name(a2).unwrap(), "Test Camera A");
    assert_eq!(camera_name(b2).unwrap(), "Test Camera B");
    assert_eq!(camera_position(a2), CameraPosition::FrontFacing);
    assert_eq!(camera_position(b2), CameraPosition::BackFacing);
    assert_eq!(camera_position(12345), CameraPosition::Unknown);
    assert_eq!(camera_supported_formats(a2).unwrap().len(), 7);
    assert!(camera_supported_formats(b2).unwrap().is_empty());

    let freed = FREED_HANDLES.load(Ordering::SeqCst);
    stop();
    assert_eq!(FREED_HANDLES.load(Ordering::SeqCst), freed + 2);
}

#[test]
fn capture_without_conversion() {
    let _l = crate::test_support::test_lock();
    let (a, _) = start();
    camera_events();

    let want = spec(PixelFormat::XRGB8888, 320, 240, 30, 1);
    let camera = Camera::open(a, Some(&want)).unwrap();
    assert_eq!(
        Camera::open(a, None).unwrap_err().message(),
        "Camera already opened"
    );
    assert_eq!(camera.id(), a);
    assert_eq!(camera.permission_state(), CameraPermissionState::Approved);
    assert_eq!(camera.format().unwrap(), want);
    assert_eq!(
        camera_events(),
        vec![(EventType::CAMERA_DEVICE_APPROVED, a)]
    );

    // The first frame is dropped; frames are in order with increasing
    // timestamps near SDL's ticks.
    let released = RELEASED.load(Ordering::SeqCst);
    let mut last_ts = 0;
    for n in 1..4u8 {
        let mut frame = next_frame(&camera);
        assert_eq!((frame.width(), frame.height()), (320, 240));
        assert_eq!(frame.format(), PixelFormat::XRGB8888);
        assert_eq!(
            frame.read_pixel(5, 5).unwrap(),
            Color {
                r: 0,
                g: 0,
                b: n,
                a: 255
            }
        );
        assert_eq!(
            frame
                .properties()
                .get_float(PROP_SURFACE_ROTATION_FLOAT)
                .unwrap_or_default(),
            90.0
        );
        assert!(frame.timestamp_ns() > last_ts);
        assert!(frame.timestamp_ns() <= crate::timer::ticks_ns());
        last_ts = frame.timestamp_ns();
        camera.release_frame(frame);
    }
    // (the dropped frame and the three released ones)
    assert!(RELEASED.load(Ordering::SeqCst) >= released + 4);

    // While the app holds all eight surfaces, new frames are dropped.
    let held: Vec<CameraFrame> = (0..NUM_OUTPUT_SURFACES)
        .map(|_| next_frame(&camera))
        .collect();
    let released = RELEASED.load(Ordering::SeqCst);
    let start = crate::timer::ticks_ms();
    while RELEASED.load(Ordering::SeqCst) == released {
        assert!(crate::timer::ticks_ms() - start < 2000, "no frame dropped");
        crate::timer::delay(Duration::from_millis(1));
    }
    assert!(camera.acquire_frame().unwrap().is_none());
    drop(held);
    let frame = next_frame(&camera);

    // A frame held across the close stays valid.
    let closed = CLOSED.load(Ordering::SeqCst);
    drop(camera);
    assert_eq!(CLOSED.load(Ordering::SeqCst), closed + 1);
    assert_eq!(frame.width(), 320);
    assert!(frame.read_pixel(0, 0).is_ok());
    drop(frame);

    // It can be opened again.
    let camera = Camera::open(a, None).unwrap();
    assert_eq!(
        camera.format().unwrap(),
        spec(PixelFormat::NV12, 640, 480, 30, 1)
    );
    drop(camera);
    stop();
}

#[test]
fn capture_with_conversion_and_scaling() {
    let _l = crate::test_support::test_lock();
    let (a, _) = start();

    let check = |want: CameraSpec, expect: Color| {
        let camera = Camera::open(a, Some(&want)).unwrap();
        let released = RELEASED.load(Ordering::SeqCst);
        let frame = next_frame(&camera);
        assert_eq!(frame.format(), want.format);
        assert_eq!((frame.width(), frame.height()), (want.width, want.height));
        let mut c = frame.read_pixel(want.width / 2, want.height / 2).unwrap();
        if expect.b == 0 {
            c.b = 0; // (the frame number)
        }
        assert_eq!(c, expect);
        // The device's buffers went back right after the copy.
        assert!(RELEASED.load(Ordering::SeqCst) > released);
        drop(frame);
        drop(camera);
    };

    // conversion only (XRGB8888 320x240 at the device)
    let rgba = PixelFormat::RGBA8888;
    check(
        spec(rgba, 320, 240, 0, 0),
        Color {
            r: 0,
            g: 0,
            b: 1,
            a: 255,
        },
    );
    // downscaling only (from XRGB8888 160x120)
    check(
        spec(PixelFormat::XRGB8888, 80, 60, 0, 0),
        Color {
            r: 0,
            g: 0,
            b: 1,
            a: 255,
        },
    );
    // downscaling, then conversion
    check(
        spec(rgba, 80, 60, 0, 0),
        Color {
            r: 0,
            g: 0,
            b: 1,
            a: 255,
        },
    );

    // conversion from NV12 640x480 (gray), then upscaling
    let camera = Camera::open(a, Some(&spec(rgba, 800, 600, 0, 0))).unwrap();
    let frame = next_frame(&camera);
    assert_eq!((frame.width(), frame.height()), (800, 600));
    let c = frame.read_pixel(400, 300).unwrap();
    assert!(c.r == c.g && c.g == c.b && c.a == 255, "{c:?}");
    drop(frame);
    drop(camera);
    stop();
}

#[test]
fn disconnect_makes_zombie() {
    let _l = crate::test_support::test_lock();
    let (a, b) = start();
    camera_events();

    let camera = Camera::open(a, Some(&spec(PixelFormat::XRGB8888, 160, 120, 30, 1))).unwrap();
    FAIL_AT.store(3, Ordering::SeqCst);
    let mut frames = 0;
    loop {
        let frame = next_frame(&camera);
        frames += 1;
        assert!(frames < 100);
        if frame.read_pixel(0, 0).unwrap()
            == (Color {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            })
        {
            break; // a black zombie frame
        }
    }
    assert_eq!(
        camera_events(),
        vec![
            (EventType::CAMERA_DEVICE_APPROVED, a),
            (EventType::CAMERA_DEVICE_REMOVED, a)
        ]
    );
    // Still listed until the app closes it.
    assert_eq!(cameras().unwrap(), vec![a, b]);
    let _ = next_frame(&camera);
    drop(camera);
    assert_eq!(cameras().unwrap(), vec![b]);
    stop();
}

#[test]
fn permission_denied_or_pending() {
    let _l = crate::test_support::test_lock();
    let (a, b) = start();
    camera_events();

    PERMISSION.store(-1, Ordering::SeqCst);
    let camera = Camera::open(a, None).unwrap();
    assert_eq!(camera.permission_state(), CameraPermissionState::Denied);
    assert_eq!(
        camera.acquire_frame().unwrap_err().message(),
        "Camera permission has not been granted"
    );
    assert!(camera.format().is_err());
    assert_eq!(camera_events(), vec![(EventType::CAMERA_DEVICE_DENIED, a)]);
    drop(camera);

    PERMISSION.store(0, Ordering::SeqCst);
    let camera = Camera::open(b, None).unwrap();
    assert_eq!(camera.permission_state(), CameraPermissionState::Pending);
    assert!(camera.acquire_frame().is_err());
    // The backend decides later.
    camera_permission_outcome(&camera.device, true);
    assert_eq!(camera.permission_state(), CameraPermissionState::Approved);
    // A device without specs gets what was asked for (here, nothing).
    assert_eq!(camera.format().unwrap(), CameraSpec::default());
    drop(camera);
    stop();
}

#[test]
fn failed_open_keeps_the_device() {
    let _l = crate::test_support::test_lock();
    let (a, b) = start();
    let freed = FREED_HANDLES.load(Ordering::SeqCst);
    let closed = CLOSED.load(Ordering::SeqCst);

    // The backend opens, but no output surfaces can be made in this format.
    // (an 8-bit-array format of two bytes per pixel has no masks)
    let bogus = spec(PixelFormat(0x1710_1002), 640, 480, 30, 1);
    assert!(Camera::open(a, Some(&bogus)).is_err());
    assert_eq!(CLOSED.load(Ordering::SeqCst), closed + 1, "backend closed");

    // The device is still there, and still opens.
    assert_eq!(FREED_HANDLES.load(Ordering::SeqCst), freed);
    assert_eq!(cameras().unwrap(), vec![a, b]);
    assert_eq!(camera_name(a).unwrap(), "Test Camera A");
    let camera = Camera::open(a, None).unwrap();
    next_frame(&camera);
    drop(camera);
    assert_eq!(cameras().unwrap(), vec![a, b]);
    stop();
}

#[test]
fn quit_while_open() {
    let _l = crate::test_support::test_lock();
    let (a, _) = start();
    let camera = Camera::open(a, Some(&spec(PixelFormat::XRGB8888, 160, 120, 30, 1))).unwrap();
    let frame = next_frame(&camera);
    let closed = CLOSED.load(Ordering::SeqCst);
    stop();
    assert_eq!(CLOSED.load(Ordering::SeqCst), closed + 1);
    assert!(camera.acquire_frame().is_err());
    drop(frame);
    drop(camera);
}

/// The hardware check of a camera driver (the `#[ignore]`d tests next to
/// each driver's own; docs/HARDWARE_TESTING.md): start `driver`, list its
/// cameras, open the first one in its preferred format and grab ten frames,
/// checking their size and format and that their timestamps advance. A
/// driver that doesn't start skips as `capability`, no camera as `camera`.
pub(super) fn hardware_capture(driver: &str, capability: &str) {
    use std::time::{Duration, Instant};
    crate::hints::set(crate::hints::CAMERA_DRIVER, driver).unwrap();
    let r = init::init_subsystem(InitFlags::CAMERA);
    crate::hints::reset(crate::hints::CAMERA_DRIVER);
    if let Err(e) = r {
        crate::test_support::skip(
            capability,
            format_args!("the {driver} camera driver didn't start: {}", e.message()),
        );
        return;
    }
    assert_eq!(current_camera_driver(), Some(driver));

    // Hotplug detection may still be adding devices.
    let start = Instant::now();
    let mut ids = cameras().unwrap();
    while ids.is_empty() && start.elapsed() < Duration::from_secs(2) {
        pump();
        std::thread::sleep(Duration::from_millis(20));
        ids = cameras().unwrap();
    }
    println!("{driver} cameras: {}", ids.len());
    for &id in &ids {
        let specs = camera_supported_formats(id).unwrap();
        println!(
            "  {id}: {:?} ({:?}), {} formats",
            camera_name(id).unwrap(),
            camera_position(id),
            specs.len()
        );
        for s in &specs {
            println!(
                "    {:?} {}x{} at {}/{} fps",
                s.format, s.width, s.height, s.framerate_numerator, s.framerate_denominator
            );
        }
        assert!(!specs.is_empty());
    }
    let Some(&id) = ids.first() else {
        init::quit_subsystem(InitFlags::CAMERA);
        crate::test_support::skip("camera", format_args!("{driver} found no camera"));
        return;
    };

    let camera = Camera::open(id, None).unwrap();
    let start = Instant::now();
    while camera.permission_state() == CameraPermissionState::Pending
        && start.elapsed() < Duration::from_secs(30)
    {
        pump();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        camera.permission_state(),
        CameraPermissionState::Approved,
        "camera access (Settings > Privacy > Camera on Windows)"
    );
    let spec = camera.format().unwrap();
    println!(
        "opened {:?}: {:?} {}x{} at {}/{} fps",
        camera_name(id).unwrap(),
        spec.format,
        spec.width,
        spec.height,
        spec.framerate_numerator,
        spec.framerate_denominator
    );
    assert!(spec.width > 0 && spec.height > 0);

    let mut stamps = Vec::new();
    let start = Instant::now();
    while stamps.len() < 10 {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "only {} frames in 20 seconds",
            stamps.len()
        );
        pump();
        let Some(frame) = camera.acquire_frame().unwrap() else {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        };
        assert_eq!(
            (frame.width(), frame.height(), frame.format()),
            (spec.width, spec.height, spec.format)
        );
        let pixels = frame.pixels().expect("frame pixels");
        assert!(!pixels.is_empty());
        let ts = frame.timestamp_ns();
        println!(
            "  frame {}: timestamp {ts} ns ({:+.1} ms from now), {} bytes, first {:02x?}",
            stamps.len(),
            (ts as f64 - crate::timer::ticks_ns() as f64) / 1e6,
            pixels.len(),
            &pixels[..pixels.len().min(4)]
        );
        if let Some(&last) = stamps.last() {
            assert!(ts > last, "timestamps advance: {last} then {ts}");
        }
        stamps.push(ts);
        drop(frame);
    }
    let span = (stamps[9] - stamps[0]) as f64 / 1e9;
    println!("{:.1} fps over the ten frames", 9.0 / span);
    drop(camera);
    init::quit_subsystem(InitFlags::CAMERA);
}
