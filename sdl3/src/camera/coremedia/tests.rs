// Tests for the CoreMedia camera driver. The expected values come from
// upstream's C: CoreMediaFormatToSDL(), GatherCameraSpecs() and
// COREMEDIA_OpenDevice()'s format choice copied into a C program over fake
// formats, compiled against the macOS SDK and upstream's headers, and the
// SDK's constants and structure layouts printed by another.

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
        crate::test_support::skip("camera", "AVFoundation found no camera");
    }
}

/// When this process may use the camera already, open the first one and
/// take a frame. Without that permission, opening would make the system
/// ask the user, so this skips instead.
#[test]
fn opens_a_camera_when_access_is_granted() {
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
