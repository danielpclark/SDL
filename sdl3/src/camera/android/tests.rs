// Tests of the Android camera driver (the program spawned on the device: no
// Java VM, so no permission's request, and so no open camera; in an app,
// with the emulator's cameras: sdl3/tests/android_camera.rs).

use super::*;
use crate::camera::{camera_name, camera_position, camera_supported_formats, cameras, Camera};
use crate::init::{self, InitFlags};

#[test]
fn formats_and_errors() {
    for (android, sdl) in [
        (AIMAGE_FORMAT_YUV_420_888, PixelFormat::NV12),
        (AIMAGE_FORMAT_RGB_565, PixelFormat::RGB565),
        (AIMAGE_FORMAT_RGB_888, PixelFormat::XRGB8888),
        (AIMAGE_FORMAT_RGBA_8888, PixelFormat::RGBA8888),
        (AIMAGE_FORMAT_RGBX_8888, PixelFormat::RGBX8888),
    ] {
        assert_eq!(format_android_to_sdl(android).0, sdl);
        assert_eq!(format_sdl_to_android(sdl), android);
    }
    assert_eq!(
        format_android_to_sdl(AIMAGE_FORMAT_YUV_420_888).1,
        Colorspace::BT709_LIMITED
    );
    assert_eq!(
        format_android_to_sdl(AIMAGE_FORMAT_RGBA_FP16),
        (PixelFormat::RGBA64_FLOAT, Colorspace::SRGB)
    );
    // (the half-float one only one way, as upstream's)
    assert_eq!(format_sdl_to_android(PixelFormat::RGBA64_FLOAT), 0);
    // (JPEG, PRIVATE)
    for fmt in [0x100, 0x22] {
        assert_eq!(
            format_android_to_sdl(fmt),
            (PixelFormat::UNKNOWN, Colorspace::UNKNOWN)
        );
    }

    assert_eq!(
        set_camera_error("Failed to open camera", ACAMERA_ERROR_PERMISSION_DENIED).message(),
        "Failed to open camera: permission denied"
    );
    assert_eq!(
        set_camera_error("Error ACaptureRequest_addTarget", -1).message(),
        "Error ACaptureRequest_addTarget: Unknown error #-1"
    );
    assert_eq!(
        set_media_error(
            "Error AImageReader_new",
            AMEDIA_IMGREADER_MAX_IMAGES_ACQUIRED
        )
        .message(),
        "Error AImageReader_new: maximum images acquired"
    );
    assert_eq!(camera_status_str(ACAMERA_OK), Some("no error"));
    assert_eq!(
        media_status_str(AMEDIA_ERROR_WOULD_BLOCK),
        Some("operation would block")
    );
}

/// The camera manager's cameras, as a spawned program sees them: their
/// names, positions and formats; opening one fails without a Java VM (the
/// permission's request needs the app's activity), and the camera stays.
#[test]
fn cameras_without_a_java_vm() {
    let _l = crate::test_support::test_lock();
    crate::hints::set(crate::hints::CAMERA_DRIVER, "android").unwrap();
    let r = init::init_subsystem(InitFlags::CAMERA);
    crate::hints::reset(crate::hints::CAMERA_DRIVER);
    r.unwrap();
    assert_eq!(crate::camera::current_camera_driver(), Some("android"));

    let ids = cameras().unwrap();
    println!("android cameras: {}", ids.len());
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
            assert!(s.width > 0 && s.height > 0);
            assert_eq!((s.framerate_numerator, s.framerate_denominator), (30, 1));
            assert_ne!(format_sdl_to_android(s.format), 0, "{:?}", s.format);
        }
        assert!(!specs.is_empty());
    }
    if let Some(&id) = ids.first() {
        assert_eq!(
            Camera::open(id, None).unwrap_err().message(),
            "Failed, there is no JavaVM"
        );
        assert_eq!(cameras().unwrap(), ids);
    } else {
        crate::test_support::skip("camera", "the camera manager has no camera");
    }
    init::quit_subsystem(InitFlags::CAMERA);
}
