// Tests for the V4L2 camera driver: the ioctl numbers, struct layouts and
// fourccs against <linux/videodev2.h>, and the format/size/frame-rate
// enumeration against MaybeAddDevice()/AddCameraFormat() run over the same
// fake device by a throwaway C program (scratchpad camera-ref/v4l2_ref.c).

use std::cell::Cell;

use super::*;
use crate::init::InitFlags;

#[test]
#[cfg(target_arch = "x86_64")]
fn ioctls_and_layouts_match_videodev2_h() {
    use std::mem::offset_of;
    let ioctls: [(c_ulong, c_ulong); 16] = [
        (VIDIOC_QUERYCAP, 0x80685600),
        (VIDIOC_ENUM_FMT, 0xc0405602),
        (VIDIOC_G_FMT, 0xc0d05604),
        (VIDIOC_S_FMT, 0xc0d05605),
        (VIDIOC_REQBUFS, 0xc0145608),
        (VIDIOC_QUERYBUF, 0xc0585609),
        (VIDIOC_QBUF, 0xc058560f),
        (VIDIOC_DQBUF, 0xc0585611),
        (VIDIOC_STREAMON, 0x40045612),
        (VIDIOC_STREAMOFF, 0x40045613),
        (VIDIOC_G_PARM, 0xc0cc5615),
        (VIDIOC_S_PARM, 0xc0cc5616),
        (VIDIOC_CROPCAP, 0xc02c563a),
        (VIDIOC_S_CROP, 0x4014563c),
        (VIDIOC_ENUM_FRAMESIZES, 0xc02c564a),
        (VIDIOC_ENUM_FRAMEINTERVALS, 0xc034564b),
    ];
    for (i, (got, want)) in ioctls.iter().enumerate() {
        assert_eq!(got, want, "ioctl {i}");
    }
    assert_eq!(size_of::<V4l2Capability>(), 104);
    assert_eq!(size_of::<V4l2Fmtdesc>(), 64);
    assert_eq!(size_of::<V4l2Frmsizeenum>(), 44);
    assert_eq!(size_of::<V4l2Frmivalenum>(), 52);
    assert_eq!(size_of::<V4l2Cropcap>(), 44);
    assert_eq!(size_of::<V4l2Crop>(), 20);
    assert_eq!(size_of::<V4l2Format>(), 208);
    assert_eq!(size_of::<V4l2PixFormat>(), 48);
    assert_eq!(size_of::<V4l2Streamparm>(), 204);
    assert_eq!(size_of::<V4l2Captureparm>(), 40);
    assert_eq!(size_of::<V4l2Requestbuffers>(), 20);
    assert_eq!(size_of::<V4l2Buffer>(), 88);
    assert_eq!(offset_of!(V4l2Capability, device_caps), 88);
    assert_eq!(offset_of!(V4l2Format, fmt), 8);
    assert_eq!(offset_of!(V4l2Streamparm, capture), 4);
    assert_eq!(offset_of!(V4l2Buffer, timestamp), 24);
    assert_eq!(offset_of!(V4l2Buffer, sequence), 56);
    assert_eq!(offset_of!(V4l2Buffer, m), 64);
    assert_eq!(offset_of!(V4l2Buffer, length), 72);
    assert_eq!(offset_of!(V4l2Frmsizeenum, u), 12);
    assert_eq!(offset_of!(V4l2Frmivalenum, u), 20);
    assert_eq!(offset_of!(V4l2Cropcap, defrect), 20);
}

#[test]
fn fourccs_and_format_mapping() {
    assert_eq!(V4L2_PIX_FMT_YUYV, 0x56595559);
    assert_eq!(V4L2_PIX_FMT_MJPEG, 0x47504a4d);
    assert_eq!(V4L2_PIX_FMT_RGBX32, 0x34324258);
    assert_eq!(V4L2_CAP_VIDEO_CAPTURE, 0x0000_0001);
    assert_eq!(V4L2_CAP_READWRITE, 0x0100_0000);
    assert_eq!(V4L2_CAP_STREAMING, 0x0400_0000);

    for (v4l2, sdl, cs) in [
        (
            V4L2_PIX_FMT_YUYV,
            PixelFormat::YUY2,
            Colorspace::BT709_LIMITED,
        ),
        (V4L2_PIX_FMT_MJPEG, PixelFormat::MJPG, Colorspace::SRGB),
        (V4L2_PIX_FMT_RGBX32, PixelFormat::RGBX32, Colorspace::SRGB),
    ] {
        assert_eq!(format_v4l2_to_sdl(v4l2), (sdl, cs));
        assert_eq!(format_sdl_to_v4l2(sdl), v4l2);
    }
    let h264 = v4l2_fourcc(b'H', b'2', b'6', b'4');
    assert_eq!(
        format_v4l2_to_sdl(h264),
        (PixelFormat::UNKNOWN, Colorspace::UNKNOWN)
    );
    assert_eq!(format_sdl_to_v4l2(PixelFormat::NV12), 0);
    assert_eq!(format_sdl_to_v4l2(PixelFormat::UNKNOWN), 0);
}

/// The fake device of v4l2_ref.c: YUYV in two discrete sizes (discrete and
/// stepwise intervals), MJPG in stepwise sizes (stepwise and continuous
/// intervals), H264 (not for SDL) and RGBX32 in continuous sizes (one
/// discrete interval for one of them).
struct FakeDevice {
    calls: Cell<usize>,
}

impl FakeDevice {
    fn count(&self) -> bool {
        self.calls.set(self.calls.get() + 1);
        assert!(self.calls.get() < 10_000, "the enumeration doesn't end");
        true
    }
}

impl FormatEnumerator for FakeDevice {
    fn enum_fmt(&self, d: &mut V4l2Fmtdesc) -> bool {
        self.count();
        let fmts = [
            V4L2_PIX_FMT_YUYV,
            V4L2_PIX_FMT_MJPEG,
            v4l2_fourcc(b'H', b'2', b'6', b'4'),
            V4L2_PIX_FMT_RGBX32,
        ];
        assert_eq!(d.type_, V4L2_BUF_TYPE_VIDEO_CAPTURE);
        match fmts.get(d.index as usize) {
            Some(&f) => {
                d.pixelformat = f;
                true
            }
            None => false,
        }
    }

    fn enum_framesizes(&self, s: &mut V4l2Frmsizeenum) -> bool {
        self.count();
        match s.pixel_format {
            V4L2_PIX_FMT_YUYV if s.index < 2 => {
                s.type_ = V4L2_FRMSIZE_TYPE_DISCRETE;
                s.u[0] = if s.index != 0 { 320 } else { 640 };
                s.u[1] = if s.index != 0 { 240 } else { 480 };
                true
            }
            V4L2_PIX_FMT_MJPEG if s.index < 1 => {
                s.type_ = V4L2_FRMSIZE_TYPE_STEPWISE;
                s.u = [160, 320, 80, 120, 240, 60];
                true
            }
            V4L2_PIX_FMT_RGBX32 if s.index < 1 => {
                s.type_ = V4L2_FRMSIZE_TYPE_CONTINUOUS;
                s.u = [64, 64, 1, 48, 49, 1];
                true
            }
            _ => false,
        }
    }

    fn enum_frameintervals(&self, i: &mut V4l2Frmivalenum) -> bool {
        self.count();
        match (i.pixel_format, i.width, i.height) {
            (V4L2_PIX_FMT_YUYV, 640, _) => {
                if i.index >= 2 {
                    return false;
                }
                i.type_ = V4L2_FRMIVAL_TYPE_DISCRETE;
                i.u[0] = 1;
                i.u[1] = if i.index != 0 { 15 } else { 30 };
                true
            }
            (V4L2_PIX_FMT_YUYV, _, _) => {
                i.type_ = V4L2_FRMIVAL_TYPE_STEPWISE;
                i.u = [1, 30, 3, 30, 1, 0];
                true
            }
            (V4L2_PIX_FMT_MJPEG, 320, 240) => {
                i.type_ = V4L2_FRMIVAL_TYPE_CONTINUOUS;
                i.u = [1, 60, 1, 24, 1, 1];
                true
            }
            (V4L2_PIX_FMT_MJPEG, _, _) => {
                i.type_ = V4L2_FRMIVAL_TYPE_STEPWISE;
                i.u = [1, 5, 5, 5, 2, 5];
                true
            }
            (V4L2_PIX_FMT_RGBX32, _, 48) if i.index < 1 => {
                i.type_ = V4L2_FRMIVAL_TYPE_DISCRETE;
                i.u[0] = 1001;
                i.u[1] = 30000;
                true
            }
            _ => false,
        }
    }
}

#[test]
fn enumeration_matches_the_c_driver() {
    let dev = FakeDevice {
        calls: Cell::new(0),
    };
    let specs = gather_specs(&dev);

    // v4l2_ref.txt: "spec <format> <colorspace> <w> <h> <num> <den>", with
    // 1 = YUY2/BT709_LIMITED, 2 = MJPG/SRGB, 3 = RGBX32/SRGB.
    let yuy2 = |w, h, n, d| (PixelFormat::YUY2, Colorspace::BT709_LIMITED, w, h, n, d);
    let mjpg = |w, h, n, d| (PixelFormat::MJPG, Colorspace::SRGB, w, h, n, d);
    let mut want = vec![
        yuy2(640, 480, 30, 1),
        yuy2(640, 480, 15, 1),
        yuy2(320, 240, 30, 1),
        yuy2(320, 240, 30, 2),
        yuy2(320, 240, 30, 3),
    ];
    for w in [160, 240, 320] {
        for h in [120, 180, 240] {
            if (w, h) == (320, 240) {
                // (60000/1000 is just outside: the float minimum rate
                // compares above the double 1/60.)
                for (n, d) in [
                    (24000, 1000),
                    (30000, 1001),
                    (30000, 1000),
                    (50000, 1000),
                    (60000, 1001),
                ] {
                    want.push(mjpg(w, h, n, d));
                }
            } else {
                want.extend([mjpg(w, h, 5, 1), mjpg(w, h, 10, 3), mjpg(w, h, 15, 5)]);
            }
        }
    }
    want.push((PixelFormat::RGBX32, Colorspace::SRGB, 64, 48, 30000, 1001));
    assert_eq!(want.len(), 35); // "total 35"

    let got: Vec<_> = specs
        .iter()
        .map(|s| {
            (
                s.format,
                s.colorspace,
                s.width,
                s.height,
                s.framerate_numerator,
                s.framerate_denominator,
            )
        })
        .collect();
    assert_eq!(got, want);
}

#[test]
fn step_counting() {
    assert_eq!(steps(1, 3, 1).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(steps(160, 320, 80).collect::<Vec<_>>(), [160, 240, 320]);
    assert_eq!(steps(5, 4, 1).count(), 0);
    // (endless loops upstream)
    assert_eq!(steps(7, 9, 0).collect::<Vec<_>>(), [7]);
    assert_eq!(
        steps(i32::MAX - 1, i32::MAX, 1).collect::<Vec<_>>(),
        [i32::MAX - 1, i32::MAX]
    );
    assert_eq!(
        steps(0, i32::MAX, i32::MAX).collect::<Vec<_>>(),
        [0, i32::MAX]
    );
    assert_eq!(steps(1, 10, -1).collect::<Vec<_>>(), [1]);
}

#[test]
fn dev_entries() {
    assert_eq!(video_device_number("video0"), Some(0));
    assert_eq!(video_device_number("video12"), Some(12));
    assert_eq!(video_device_number("video3x"), Some(3));
    assert_eq!(video_device_number("video-1"), Some(-1));
    assert_eq!(video_device_number("video"), None);
    assert_eq!(video_device_number("videox"), None);
    assert_eq!(video_device_number("vbi0"), None);
    assert_eq!(video_device_number("xvideo0"), None);
}

#[test]
fn buffers_and_timestamps() {
    let mut buf = V4l2Buffer::new(V4L2_MEMORY_MMAP);
    assert_eq!((buf.type_, buf.memory), (V4L2_BUF_TYPE_VIDEO_CAPTURE, 1));
    buf.m = 0x1234_5678;
    assert_eq!(buf.offset(), 0x1234_5678);
    buf.timestamp.tv_sec = 3;
    buf.timestamp.tv_usec = 250;
    assert_eq!(buf.timestamp_ns(), 3_000_250_000);

    // Each buffer has an address of its own, even when empty.
    let a = alloc_buffer(0);
    let b = alloc_buffer(0);
    assert_ne!(a.as_ptr(), b.as_ptr());
    assert_eq!(alloc_buffer(5), [0; 5]);
    let ub = alloc_buffer_user_ptr(3, 16);
    assert_eq!(ub.len(), 3);
    for b in &ub {
        assert_eq!(b.start, b.data.as_ref().unwrap().as_ptr() as usize);
        assert_eq!(b.length, 16);
        assert!(!b.available);
    }
}

#[test]
fn opening_a_missing_device_fails_cleanly() {
    let _l = crate::test_support::test_lock();
    crate::hints::set(crate::hints::CAMERA_DRIVER, "v4l2").unwrap();
    let r = crate::init::init_subsystem(InitFlags::CAMERA);
    crate::hints::reset(crate::hints::CAMERA_DRIVER);
    r.unwrap();
    assert_eq!(super::super::current_camera_driver(), Some("v4l2"));
    let cameras = super::super::cameras().unwrap();
    eprintln!("note: V4L2 cameras here: {}", cameras.len());
    for id in cameras {
        eprintln!(
            "note:   {} ({} specs)",
            super::super::camera_name(id).unwrap(),
            super::super::camera_supported_formats(id).unwrap().len()
        );
    }

    // Opening a node that isn't there reports it like upstream.
    let err = open_device_node("/dev/sdl-no-such-video-device").unwrap_err();
    assert_eq!(err.raw_os_error(), Some(libc::ENOENT));
    // And a file that's no character device is skipped.
    maybe_add_device("/dev/null"); // (a character device, but no V4L2 one)
    maybe_add_device("/");
    crate::init::quit_subsystem(InitFlags::CAMERA);
}
