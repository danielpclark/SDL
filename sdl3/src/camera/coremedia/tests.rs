// Tests for the CoreMedia camera driver. The expected values come from
// upstream's C: CoreMediaFormatToSDL(), GatherCameraSpecs() and
// COREMEDIA_OpenDevice()'s format choice copied into a C program over fake
// formats, compiled against the macOS SDK and upstream's headers, and the
// SDK's constants and structure layouts printed by another; on iOS,
// COREMEDIA_AcquireFrame()'s rotation tables and COREMEDIA_OpenDevice()'s
// orientation guess copied into a C program run on the iOS Simulator, with
// upstream's C SDL there.

use std::mem::{offset_of, size_of};
use std::time::{Duration, Instant};

use super::*;
use crate::camera::{
    camera_name, camera_position, camera_supported_formats, cameras, current_camera_driver, Camera,
};
use crate::events::pump;
use crate::hints;
use crate::init::{self, InitFlags};

#[test]
fn constants_and_layouts_match_the_sdk() {
    assert_eq!(K_CM_PIXEL_FORMAT_16LE555, 0x4c35_3535);
    assert_eq!(K_CM_PIXEL_FORMAT_16LE5551, 0x3535_3531);
    assert_eq!(K_CM_PIXEL_FORMAT_16LE565, 0x4c35_3635);
    assert_eq!(K_CM_PIXEL_FORMAT_24RGB, 0x18);
    assert_eq!(K_CM_PIXEL_FORMAT_32ARGB, 0x20);
    assert_eq!(K_CM_PIXEL_FORMAT_32BGRA, 0x4247_5241);
    assert_eq!(K_CM_PIXEL_FORMAT_422YPCBCR8, 0x3276_7579);
    assert_eq!(K_CM_PIXEL_FORMAT_422YPCBCR8_YUVS, 0x7975_7673);
    assert_eq!(
        K_CV_PIXEL_FORMAT_TYPE_420YPCBCR8_BIPLANAR_VIDEO_RANGE,
        0x3432_3076
    );
    assert_eq!(
        K_CV_PIXEL_FORMAT_TYPE_420YPCBCR8_BIPLANAR_FULL_RANGE,
        0x3432_3066
    );
    assert_eq!(
        K_CV_PIXEL_FORMAT_TYPE_420YPCBCR10_BIPLANAR_VIDEO_RANGE,
        0x7834_3230
    );
    assert_eq!(
        K_CV_PIXEL_FORMAT_TYPE_420YPCBCR10_BIPLANAR_FULL_RANGE,
        0x7866_3230
    );
    assert_eq!(K_CM_MEDIA_TYPE_VIDEO, 0x7669_6465);
    assert_eq!(
        (
            AV_CAPTURE_DEVICE_POSITION_UNSPECIFIED,
            AV_CAPTURE_DEVICE_POSITION_BACK,
            AV_CAPTURE_DEVICE_POSITION_FRONT
        ),
        (0, 1, 2)
    );
    assert_eq!(
        (
            AV_AUTHORIZATION_STATUS_NOT_DETERMINED,
            AV_AUTHORIZATION_STATUS_AUTHORIZED
        ),
        (0, 3)
    );
    assert_eq!(size_of::<CMTime>(), 24);
    assert_eq!(size_of::<CMSampleTimingInfo>(), 72);
    assert_eq!(offset_of!(CMSampleTimingInfo, presentation_time_stamp), 24);
    assert_eq!(size_of::<CMVideoDimensions>(), 8);
    // The constants upstream references under @available, and the others.
    // SAFETY: NSString constants of the linked frameworks.
    unsafe {
        assert_eq!(string_from_ns(AVMediaTypeVideo).as_deref(), Some("vide"));
        assert_eq!(
            string_from_ns(AVCaptureSessionPresetHigh).as_deref(),
            Some("AVCaptureSessionPresetHigh")
        );
        // (macOS's: the iOS device types are checked below)
        #[cfg(target_os = "macos")]
        assert_eq!(
            string_from_ns(weak_ns_string_constant(
                c"AVCaptureDeviceTypeExternalUnknown"
            ))
            .as_deref(),
            Some("AVCaptureDeviceTypeExternal")
        );
        assert_eq!(
            string_from_ns(weak_ns_string_constant(
                c"AVCaptureDeviceTypeBuiltInWideAngleCamera"
            ))
            .as_deref(),
            Some("AVCaptureDeviceTypeBuiltInWideAngleCamera")
        );
    }
    assert!(weak_ns_string_constant(c"AVCaptureDeviceTypeNoSuchCamera").is_null());
    // SAFETY: a CMTime made and compared by value.
    unsafe {
        let t = CMTimeMake(1001, 30000);
        assert_eq!(
            (t.value, t.timescale, t.flags, t.epoch),
            (1001, 30000, 1, 0)
        );
        assert_eq!(CMTimeCompare(t, CMTimeMake(1, 30)), 1);
        assert_eq!(CMTimeCompare(CMTimeMake(1, 30), t), -1);
        assert_eq!(CMTimeCompare(CMTimeMake(2, 60), CMTimeMake(1, 30)), 0);
        assert!((CMTimeGetSeconds(t) - 1001.0 / 30000.0).abs() < 1e-12);
    }
}

#[test]
fn format_table_matches_the_c_driver() {
    // (FourCharCode, SDL_PixelFormat, SDL_Colorspace) from CoreMediaFormatToSDL().
    const EXPECTED: [(u32, u32, u32); 15] = [
        (0x4c353535, 0x15130f02, 0x120005a0),
        (0x35353531, 0x15441002, 0x120005a0),
        (0x4c353635, 0x15151002, 0x120005a0),
        (0x00000018, 0x17101803, 0x120005a0),
        (0x00000020, 0x16862004, 0x120005a0),
        (0x42475241, 0x16362004, 0x120005a0),
        (0x32767579, 0x59565955, 0x21100421),
        (0x79757673, 0x32595559, 0x21100421),
        (0x34323076, 0x3231564e, 0x21100421),
        (0x34323066, 0x3231564e, 0x22100421),
        (0x78343230, 0x30313050, 0x21102609),
        (0x78663230, 0x30313050, 0x22102609),
        (0x6a706567, 0x00000000, 0x00000000),
        (0x00000000, 0x00000000, 0x00000000),
        (0x4c353536, 0x00000000, 0x00000000),
    ];
    for (code, format, colorspace) in EXPECTED {
        let (f, c) = core_media_format_to_sdl(code);
        assert_eq!((f.0, c.0), (format, colorspace), "{code:08x}");
    }
}

/// The fake formats of the C program.
fn fake_formats() -> Vec<FormatInfo> {
    let f = |media: &[u8; 4], sub: u32, w, h, ranges: &[(f64, f64)]| FormatInfo {
        media_type: fourcc(media),
        sub_type: sub,
        dims: CMVideoDimensions {
            width: w,
            height: h,
        },
        frame_rate_ranges: ranges.to_vec(),
    };
    let v420 = fourcc(b"420v");
    vec![
        f(b"vide", v420, 1280, 720, &[(15.0, 30.0)]),
        f(b"vide", v420, 1280, 720, &[(30.0, 30.0)]),
        f(
            b"vide",
            v420,
            1280,
            720,
            &[(10.0, 60.0), (29.97002997002997, 29.97002997002997)],
        ),
        f(b"vide", fourcc(b"BGRA"), 1920, 1080, &[(60.0, 60.0)]),
        f(b"vide", fourcc(b"420f"), 640, 480, &[(15.0, 30.0)]),
        f(b"vide", v420, 640, 480, &[(1.0, 5.0), (25.0, 25.0)]),
        f(b"vide", v420, 640, 480, &[(24.9, 25.05), (4.995, 5.0)]),
        f(b"soun", v420, 320, 240, &[(30.0, 30.0)]),
        f(b"vide", fourcc(b"jpeg"), 320, 240, &[(30.0, 30.0)]),
        f(
            b"vide",
            fourcc(b"yuvs"),
            320,
            240,
            &[(7.5, 7.5), (14.985014985014985, 30.0), (0.5, 120.0)],
        ),
        f(b"vide", fourcc(b"x420"), 3840, 2160, &[]),
    ]
}

#[test]
fn specs_match_the_c_driver() {
    // GatherCameraSpecs() over the fake formats: (format, colorspace, w, h,
    // numerator, denominator), in the order added.
    const EXPECTED: [(u32, u32, i32, i32, i32, i32); 21] = [
        (0x3231564e, 0x21100421, 1280, 720, 15, 1),
        (0x3231564e, 0x21100421, 1280, 720, 30, 1),
        (0x3231564e, 0x21100421, 1280, 720, 30, 1),
        (0x3231564e, 0x21100421, 1280, 720, 10, 1),
        (0x3231564e, 0x21100421, 1280, 720, 60, 1),
        (0x3231564e, 0x21100421, 1280, 720, 19001, 634),
        (0x16362004, 0x120005a0, 1920, 1080, 60, 1),
        (0x3231564e, 0x22100421, 640, 480, 15, 1),
        (0x3231564e, 0x22100421, 640, 480, 30, 1),
        (0x3231564e, 0x21100421, 640, 480, 1, 1),
        (0x3231564e, 0x21100421, 640, 480, 5, 1),
        (0x3231564e, 0x21100421, 640, 480, 25, 1),
        (0x3231564e, 0x21100421, 640, 480, 249, 10),
        (0x3231564e, 0x21100421, 640, 480, 501, 20),
        (0x3231564e, 0x21100421, 640, 480, 999, 200),
        (0x3231564e, 0x21100421, 640, 480, 5, 1),
        (0x32595559, 0x21100421, 320, 240, 15, 2),
        (0x32595559, 0x21100421, 320, 240, 10999, 734),
        (0x32595559, 0x21100421, 320, 240, 30, 1),
        (0x32595559, 0x21100421, 320, 240, 1, 2),
        (0x32595559, 0x21100421, 320, 240, 120, 1),
    ];
    let specs: Vec<_> = gather_camera_specs(&fake_formats())
        .into_iter()
        .map(|s| {
            (
                s.format.0,
                s.colorspace.0,
                s.width,
                s.height,
                s.framerate_numerator,
                s.framerate_denominator,
            )
        })
        .collect();
    assert_eq!(specs, EXPECTED);
}

#[test]
fn format_choice_matches_the_c_driver() {
    let spec = |format, colorspace, width, height, num, den| CameraSpec {
        format,
        colorspace,
        width,
        height,
        framerate_numerator: num,
        framerate_denominator: den,
    };
    let (nv12, l709, f709) = (
        PixelFormat::NV12,
        Colorspace::BT709_LIMITED,
        Colorspace::BT709_FULL,
    );
    let cases = [
        (spec(nv12, l709, 1280, 720, 30, 1), Some(0)),
        (spec(nv12, l709, 1280, 720, 30000, 1001), Some(0)),
        (spec(nv12, l709, 1280, 720, 15, 1), Some(0)),
        (spec(nv12, l709, 1280, 720, 61, 1), None),
        (spec(nv12, l709, 1280, 720, 10, 1), Some(2)),
        (
            spec(PixelFormat::BGRA32, Colorspace::SRGB, 1920, 1080, 60, 1),
            Some(3),
        ),
        (
            spec(PixelFormat::BGRA32, Colorspace::SRGB, 1920, 1080, 30, 1),
            None,
        ),
        (spec(nv12, f709, 640, 480, 30, 1), Some(4)),
        (spec(nv12, l709, 640, 480, 25, 1), Some(5)),
        (spec(nv12, l709, 640, 480, 3, 1), Some(5)),
        (spec(nv12, l709, 640, 480, 5, 1), Some(5)),
        (spec(nv12, l709, 640, 480, 999, 200), Some(5)),
        (spec(PixelFormat::YUY2, l709, 320, 240, 15, 1), Some(9)),
        (spec(PixelFormat::YUY2, l709, 320, 240, 1, 1), Some(9)),
        (
            spec(
                PixelFormat::P010,
                Colorspace::BT2020_LIMITED,
                3840,
                2160,
                30,
                1,
            ),
            None,
        ),
        (spec(nv12, l709, 320, 240, 30, 1), Some(7)),
        (spec(nv12, l709, 1280, 720, 0, 0), None),
    ];
    let formats = fake_formats();
    for (s, expected) in cases {
        assert_eq!(choose_format(&formats, &s), expected, "{s:?}");
    }
}

/// Start the driver, or skip (it always starts on macOS, but say why if not).
fn start_driver() -> bool {
    hints::set(hints::CAMERA_DRIVER, "coremedia").unwrap();
    let r = init::init_subsystem(InitFlags::CAMERA);
    hints::reset(hints::CAMERA_DRIVER);
    match r {
        Ok(()) => {
            assert_eq!(current_camera_driver(), Some("coremedia"));
            true
        }
        Err(e) => {
            crate::test_support::skip(
                "camera",
                format_args!("the coremedia camera driver didn't start: {}", e.message()),
            );
            false
        }
    }
}

/// The driver lists the cameras AVFoundation finds, each with the specs of
/// its formats (sorted and without duplicates, as the camera core keeps
/// them), without opening any.
#[test]
fn lists_cameras_and_their_specs() {
    let _l = crate::test_support::test_lock();
    if !start_driver() {
        return;
    }
    let ids = cameras().unwrap();
    println!("coremedia cameras: {}", ids.len());
    for &id in &ids {
        let name = camera_name(id).unwrap();
        let specs = camera_supported_formats(id).unwrap();
        println!(
            "{name} pos={} specs={}",
            camera_position(id) as i32,
            specs.len()
        );
        for s in &specs {
            println!(
                "  {} 0x{:08x} {}x{} {}/{}",
                s.format.name(),
                s.colorspace.0,
                s.width,
                s.height,
                s.framerate_numerator,
                s.framerate_denominator
            );
        }
        assert!(!name.is_empty());
        assert!(!specs.is_empty());
        let device = find_physical_camera_by_callback(|d| d.instance_id == id).unwrap();
        let avdevice = handle_of(&device).expect("a CoreMedia device");
        let mut expected = autoreleasepool(|| {
            // SAFETY: the device's AVCaptureDevice, in a pool.
            let formats: Vec<FormatInfo> = unsafe { device_formats(avdevice) }
                .into_iter()
                .map(|(_, info)| info)
                .collect();
            gather_camera_specs(&formats)
        });
        expected.sort_by(crate::camera::camera_spec_cmp);
        expected.dedup();
        assert_eq!(specs, expected);
    }
    // Detecting again finds the same devices (MaybeAddDevice() skips the
    // ones it has).
    crate::camera::current_driver().unwrap().detect_devices();
    assert_eq!(cameras().unwrap().len(), ids.len());
    init::quit_subsystem(InitFlags::CAMERA);
    if ids.is_empty() {
        // (the simulator has none: no_camera_on_the_simulator() checks
        // what the driver does then)
        if cfg!(target_abi = "sim") {
            return;
        }
        crate::test_support::skip("camera", "AVFoundation found no camera");
    }
}

/// When this process may use the camera already, open the first one and
/// take a frame. Without that permission, opening would make the system
/// ask the user, so this skips instead.
#[test]
fn opens_a_camera_when_access_is_granted() {
    if cfg!(target_abi = "sim") {
        // (no camera there, and a spawned process can't be granted one:
        // no_camera_on_the_simulator() and tests/uikit_camera.rs check
        // that)
        return;
    }
    let _l = crate::test_support::test_lock();
    let status = video_authorization_status();
    if status != AV_AUTHORIZATION_STATUS_AUTHORIZED {
        crate::test_support::skip(
            "camera",
            format_args!(
                "this process may not use the camera (AVAuthorizationStatus {status}; \
                 grant it in System Settings > Privacy & Security > Camera)"
            ),
        );
        return;
    }
    if !start_driver() {
        return;
    }
    let Some(&id) = cameras().unwrap().first() else {
        init::quit_subsystem(InitFlags::CAMERA);
        crate::test_support::skip("camera", "AVFoundation found no camera");
        return;
    };
    let camera = Camera::open(id, None).unwrap();
    let spec = camera.format().unwrap();
    let start = Instant::now();
    let mut got = false;
    while start.elapsed() < Duration::from_secs(10) {
        pump();
        if let Some(frame) = camera.acquire_frame().unwrap() {
            assert_eq!(
                (frame.width(), frame.height(), frame.format()),
                (spec.width, spec.height, spec.format)
            );
            got = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(camera.permission_state(), CameraPermissionState::Approved);
    assert!(got, "no frame from {:?} in 10 seconds", camera_name(id));
    drop(camera);
    // Again, to close a running session twice in a row.
    let camera = Camera::open(id, None).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    drop(camera);
    init::quit_subsystem(InitFlags::CAMERA);
}

/// Hardware: grab frames from the first CoreMedia camera (a webcam).
#[test]
#[ignore = "hardware: needs a camera and camera access for the terminal"]
fn hardware_capture_from_the_first_camera() {
    let _l = crate::test_support::test_lock();
    crate::camera::tests::hardware_capture("coremedia", "camera");
}

// ---------------------------------------------------------------------------
// iOS

/// The device types the iOS build's discovery session asks for, with their
/// constants' values (upstream's C, on the iOS Simulator).
#[cfg(target_os = "ios")]
#[test]
fn ios_device_types_match_the_sdk() {
    let names: Vec<_> = DEVICE_TYPES
        .iter()
        .map(|name| {
            let value = weak_ns_string_constant(name);
            assert!(!value.is_null(), "{name:?}");
            // SAFETY: an NSString constant of AVFoundation.
            unsafe { string_from_ns(value) }.unwrap()
        })
        .collect();
    assert_eq!(names, IOS_DEVICE_TYPES);
}

/// The values of the device types' constants, from upstream's C.
#[cfg(target_os = "ios")]
const IOS_DEVICE_TYPES: [&str; 6] = [
    "AVCaptureDeviceTypeBuiltInTelephotoCamera",
    "AVCaptureDeviceTypeBuiltInDualCamera",
    "AVCaptureDeviceTypeBuiltInDualWideCamera",
    "AVCaptureDeviceTypeBuiltInTripleCamera",
    "AVCaptureDeviceTypeBuiltInUltraWideCamera",
    "AVCaptureDeviceTypeBuiltInWideAngleCamera",
];

/// `COREMEDIA_AcquireFrame()`'s rotations and `COREMEDIA_OpenDevice()`'s
/// guess of the device orientation, against upstream's (copied into a C
/// program run on the iOS Simulator, with the SDK's orientation values).
#[cfg(target_os = "ios")]
#[test]
fn rotations_match_the_c_driver() {
    use crate::core::apple::uikit::device_orientation_is_valid_interface_orientation;
    use crate::video::drivers::uikit::sys::{
        UIInterfaceOrientationLandscapeLeft, UIInterfaceOrientationLandscapeRight,
        UIInterfaceOrientationPortrait, UIInterfaceOrientationPortraitUpsideDown,
    };
    let mut trace = vec![format!(
        "interface orientations: portrait={UIInterfaceOrientationPortrait} \
         upside_down={UIInterfaceOrientationPortraitUpsideDown} \
         landscape_left={UIInterfaceOrientationLandscapeLeft} \
         landscape_right={UIInterfaceOrientationLandscapeRight}"
    )];
    for d in -1..=7 {
        trace.push(format!(
            "valid device orientation {d}: {}",
            device_orientation_is_valid_interface_orientation(d) as i32
        ));
    }
    for (name, position) in [
        ("back", CameraPosition::BackFacing),
        ("front", CameraPosition::FrontFacing),
    ] {
        for u in 1..=4 {
            for d in 1..=4 {
                trace.push(format!(
                    "rotation {name} ui={u} device={d}: {}",
                    frame_rotation(position, u, d)
                ));
            }
        }
    }
    for u in -1..=6 {
        trace.push(format!(
            "guess ui={u}: {}",
            device_orientation_from_interface(u)
        ));
    }
    compare(&trace, ROTATIONS_EXPECTED);

    // Out of upstream's tables (which it would read outside of): no
    // rotation. An unknown position is the front camera's, as upstream's.
    for (u, d) in [(0, 1), (5, 1), (-1, 2), (1, 0), (1, 5), (2, 6)] {
        assert_eq!(frame_rotation(CameraPosition::BackFacing, u, d), 0.0);
        assert_eq!(frame_rotation(CameraPosition::FrontFacing, u, d), 0.0);
    }
    for u in 1..=4 {
        for d in 1..=4 {
            assert_eq!(
                frame_rotation(CameraPosition::Unknown, u, d),
                frame_rotation(CameraPosition::FrontFacing, u, d)
            );
        }
    }
}

/// Upstream's trace of `rotations_match_the_c_driver()`.
#[cfg(target_os = "ios")]
const ROTATIONS_EXPECTED: &str = "\
interface orientations: portrait=1 upside_down=2 landscape_left=4 landscape_right=3
valid device orientation -1: 0
valid device orientation 0: 0
valid device orientation 1: 1
valid device orientation 2: 1
valid device orientation 3: 1
valid device orientation 4: 1
valid device orientation 5: 0
valid device orientation 6: 0
valid device orientation 7: 0
rotation back ui=1 device=1: 90
rotation back ui=1 device=2: 90
rotation back ui=1 device=3: 90
rotation back ui=1 device=4: 90
rotation back ui=2 device=1: 270
rotation back ui=2 device=2: 270
rotation back ui=2 device=3: 270
rotation back ui=2 device=4: 270
rotation back ui=3 device=1: 0
rotation back ui=3 device=2: 0
rotation back ui=3 device=3: 0
rotation back ui=3 device=4: 0
rotation back ui=4 device=1: 180
rotation back ui=4 device=2: 180
rotation back ui=4 device=3: 180
rotation back ui=4 device=4: 180
rotation front ui=1 device=1: 90
rotation front ui=1 device=2: 90
rotation front ui=1 device=3: 270
rotation front ui=1 device=4: 270
rotation front ui=2 device=1: 270
rotation front ui=2 device=2: 270
rotation front ui=2 device=3: 90
rotation front ui=2 device=4: 90
rotation front ui=3 device=1: 0
rotation front ui=3 device=2: 0
rotation front ui=3 device=3: 180
rotation front ui=3 device=4: 180
rotation front ui=4 device=1: 180
rotation front ui=4 device=2: 180
rotation front ui=4 device=3: 0
rotation front ui=4 device=4: 0
guess ui=-1: 1
guess ui=0: 1
guess ui=1: 1
guess ui=2: 2
guess ui=3: 3
guess ui=4: 4
guess ui=5: 1
guess ui=6: 1
";

/// Compare a trace with upstream's, printing it, with the first line that
/// differs.
#[cfg(target_os = "ios")]
fn compare(trace: &[String], expected: &str) {
    for line in trace {
        println!("trace: {line}");
    }
    let expected: Vec<&str> = expected.lines().collect();
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
}

/// The camera's checks in a process of the simulated device (spawned):
/// the drivers, CoreMedia starting with no camera, opening cameras that
/// aren't there, their formats, names and positions, the dummy driver,
/// and the camera access AVFoundation reports, against upstream's C
/// spawned the same way. (`tests/uikit_camera.rs` checks the same in an
/// app the runner grants or denies the camera.)
#[cfg(target_abi = "sim")]
#[test]
fn no_camera_on_the_simulator() {
    let _l = crate::test_support::test_lock();
    let mut trace = Vec::new();
    autoreleasepool(|| {
        // SAFETY: in a pool; the devices are autoreleased into it.
        let devices = unsafe { video_devices() };
        trace.push(format!("discovery session devices: {}", devices.len()));
    });
    trace.push(format!("UIDevice orientation: {}", uidevice_orientation()));
    trace.push(format!(
        "status bar orientation: {}",
        crate::video::drivers::uikit::get_interface_orientation()
    ));
    trace.extend(camera_checks("spawned"));
    compare(&trace, SPAWNED_EXPECTED);
}

/// Upstream's trace of `no_camera_on_the_simulator()` (an iPhone 16
/// simulator with iOS 18.5).
#[cfg(target_abi = "sim")]
const SPAWNED_EXPECTED: &str = "\
discovery session devices: 0
UIDevice orientation: 0
status bar orientation: 0
spawned: authorization status: 2
spawned: camera drivers: coremedia, dummy
spawned: init camera: ok
spawned: camera driver: coremedia
spawned: cameras: 0
spawned: camera events: (none)
spawned: open 0: Invalid camera device instance ID
spawned: open 0 with a spec: Invalid camera device instance ID
spawned: supported formats 0: Invalid camera device instance ID count=0
spawned: name 0: Invalid camera device instance ID
spawned: position 0: 0
spawned: open 12345: Invalid camera device instance ID
spawned: open 12345 with a spec: Invalid camera device instance ID
spawned: supported formats 12345: Invalid camera device instance ID count=0
spawned: name 12345: Invalid camera device instance ID
spawned: position 12345: 0
spawned: camera events: (none)
spawned: quit camera: driver=(null)
spawned: dummy: init ok driver=dummy cameras=0
spawned: coremedia again: init ok driver=coremedia cameras=0
spawned: authorization status: 2
";

/// The camera events queued: "A(id), B(id)" or "(none)".
#[cfg(target_abi = "sim")]
fn camera_event_names() -> String {
    use crate::events::{get_events, Event, EventType};
    pump();
    let events: Vec<String> = get_events(
        EventType::CAMERA_DEVICE_FIRST,
        EventType::CAMERA_DEVICE_LAST,
        100,
    )
    .unwrap()
    .into_iter()
    .map(|e| match e {
        Event::CameraDevice(e) => {
            let name = match e.event_type {
                EventType::CAMERA_DEVICE_ADDED => "CAMERA_DEVICE_ADDED",
                EventType::CAMERA_DEVICE_REMOVED => "CAMERA_DEVICE_REMOVED",
                EventType::CAMERA_DEVICE_APPROVED => "CAMERA_DEVICE_APPROVED",
                _ => "CAMERA_DEVICE_DENIED",
            };
            format!("{name}({})", e.which)
        }
        other => panic!("unexpected event {other:?}"),
    })
    .collect();
    if events.is_empty() {
        "(none)".into()
    } else {
        events.join(", ")
    }
}

/// What the C programs' `camera_checks()` traces (as `tests/uikit_camera.rs`
/// does; the test driver this build has first is left out of the list).
#[cfg(target_abi = "sim")]
fn camera_checks(label: &str) -> Vec<String> {
    let mut trace = Vec::new();
    let mut t = |line: String| trace.push(format!("{label}: {line}"));
    let driver = || current_camera_driver().unwrap_or("(null)");
    let message = |r: Result<()>| r.err().map_or("ok".into(), |e| e.message().to_string());
    t(format!(
        "authorization status: {}",
        video_authorization_status()
    ));
    let drivers: Vec<&str> = (0..crate::camera::num_camera_drivers())
        .map(|i| crate::camera::camera_driver(i).unwrap())
        .filter(|&name| name != "test")
        .collect();
    t(format!("camera drivers: {}", drivers.join(", ")));
    let r = init::init_subsystem(InitFlags::CAMERA);
    t(format!("init camera: {}", message(r)));
    t(format!("camera driver: {}", driver()));
    let ids = cameras();
    t(format!(
        "cameras: {}",
        ids.as_ref().map_or(-1, |ids| ids.len() as i32)
    ));
    for (i, &id) in ids.iter().flatten().enumerate() {
        t(format!(
            "camera {i}: {} position={} formats={}",
            camera_name(id).unwrap_or_default(),
            camera_position(id) as i32,
            camera_supported_formats(id).map_or(-1, |s| s.len() as i32)
        ));
    }
    t(format!("camera events: {}", camera_event_names()));

    for id in [0, 12345] {
        let opened = |r: Result<Camera>| match r {
            Ok(_) => "opened".to_string(),
            Err(e) => e.message().to_string(),
        };
        t(format!("open {id}: {}", opened(Camera::open(id, None))));
        let spec = CameraSpec {
            format: PixelFormat::NV12,
            colorspace: Colorspace::BT709_LIMITED,
            width: 640,
            height: 480,
            framerate_numerator: 30,
            framerate_denominator: 1,
        };
        t(format!(
            "open {id} with a spec: {}",
            opened(Camera::open(id, Some(&spec)))
        ));
        let formats = camera_supported_formats(id);
        t(format!(
            "supported formats {id}: {} count={}",
            formats
                .as_ref()
                .map_or_else(|e| e.message().to_string(), |_| "a list".into()),
            formats.as_ref().map_or(0, |f| f.len())
        ));
        t(format!(
            "name {id}: {}",
            camera_name(id).unwrap_or_else(|e| e.message().to_string())
        ));
        t(format!("position {id}: {}", camera_position(id) as i32));
    }
    t(format!("camera events: {}", camera_event_names()));
    init::quit_subsystem(InitFlags::CAMERA);
    t(format!("quit camera: driver={}", driver()));

    for (name, label) in [("dummy", "dummy"), ("coremedia", "coremedia again")] {
        hints::set(hints::CAMERA_DRIVER, name).unwrap();
        let r = init::init_subsystem(InitFlags::CAMERA);
        let ids = cameras();
        t(format!(
            "{label}: init {} driver={} cameras={}",
            message(r),
            driver(),
            ids.as_ref().map_or(-1, |ids| ids.len() as i32)
        ));
        init::quit_subsystem(InitFlags::CAMERA);
        hints::reset(hints::CAMERA_DRIVER);
    }
    t(format!(
        "authorization status: {}",
        video_authorization_status()
    ));
    trace
}
