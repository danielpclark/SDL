// Tests for the PipeWire camera driver: the EnumFormat pods it builds and
// the specs it collects from pods, against the bytes and results of the C
// headers and SDL_camera_pipewire.c's collect_format() (PipeWire 1.2's
// spa_pod_builder_add_object(), printed by a throwaway C program, scratchpad
// camera-ref/pw_ref.c), the format table, the param lists, the ABI, and,
// when a PipeWire server is reachable, a camera made by a stream of our own.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use super::*;
use crate::camera::{
    camera_name, camera_supported_formats, cameras, current_camera_driver, Camera,
};
use crate::init::InitFlags;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

const OPEN_MJPG: &str = "\
    680000000f0000000300040003000000010000000000000004000000030000000200000000000000\
    0200000000000000040000000300000002000200000000000300020000000000080000000a000000\
    00050000d00200000400020000000000080000000b0000001e00000001000000";
const OPEN_RAW: &str = "\
    800000000f0000000300040003000000010000000000000004000000030000000200000000000000\
    02000000000000000400000003000000010000000000000001000200000000000400000003000000\
    04000000000000000300020000000000080000000a00000080020000e00100000400020000000000\
    080000000b00000030750000e9030000";
const ENUM_RAW: &str = "\
    d80000000f0000000300040003000000010000000000000004000000030000000200000000000000\
    02000000000000000400000003000000010000000000000001000200000000002000000013000000\
    03000000000000000400000003000000040000000400000019000000170000000300020000000000\
    28000000130000000300000000000000080000000a00000080020000e001000080020000e0010000\
    40010000f0000000040002000000000028000000130000000300000000000000080000000b000000\
    1e000000010000001e000000010000000f00000001000000";
const MJPG: &str = "\
    680000000f0000000300040003000000010000000000000004000000030000000200000000000000\
    0200000000000000040000000300000002000200000000000300020000000000080000000a000000\
    80070000380400000400020000000000080000000b0000003c00000001000000";
const RANGE: &str = "\
    c00000000f0000000300040003000000010000000000000004000000030000000200000000000000\
    02000000000000000400000003000000010000000000000001000200000000000400000003000000\
    0700000000000000030002000000000028000000130000000100000000000000080000000a000000\
    40010000f00000000100000001000000001000000010000004000200000000002800000013000000\
    0100000000000000080000000b000000190000000100000000000000010000007800000001000000";
const MIXED: &str = "\
    b80000000f0000000300040003000000010000000000000004000000030000000200000000000000\
    02000000000000001c00000013000000030000000000000004000000030000000100000001000000\
    0200020000000000010002000000000004000000030000000c000000000000000300020000000000\
    080000000a0000002003000058020000040002000000000028000000130000000100000000000000\
    080000000b000000190000000100000000000000010000007800000001000000";

fn spec(format: PixelFormat, w: i32, h: i32, num: i32, den: i32) -> CameraSpec {
    CameraSpec {
        format,
        colorspace: Colorspace::UNKNOWN,
        width: w,
        height: h,
        framerate_numerator: num,
        framerate_denominator: den,
    }
}

fn build(spec: &CameraSpec, size: usize) -> Option<String> {
    let mut buf = [0u64; 128];
    // SAFETY: viewing the u64 buffer as bytes.
    let bytes = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), size) };
    let range = build_enum_format(bytes, spec)?;
    Some(hex(&bytes[range]))
}

#[test]
fn open_pods_match_the_c_builder() {
    assert_eq!(
        build(&spec(PixelFormat::MJPG, 1280, 720, 30, 1), 1024).unwrap(),
        OPEN_MJPG
    );
    assert_eq!(
        build(&spec(PixelFormat::YUY2, 640, 480, 30000, 1001), 1024).unwrap(),
        OPEN_RAW
    );
    // "open-small null"
    assert_eq!(build(&spec(PixelFormat::YUY2, 640, 480, 30, 1), 64), None);
}

fn collect(pod: &str) -> Vec<(PixelFormat, Colorspace, i32, i32, i32, i32)> {
    let bytes = unhex(pod);
    let mut specs = Vec::new();
    collect_format(&mut specs, Pod::new(&bytes).unwrap());
    specs
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
        .collect()
}

#[test]
fn collected_specs_match_the_c_driver() {
    // pw_ref.txt (with SDL's formats for the stand-ins: 1004 = YUY2, 1023 =
    // NV12, 2 = BT709_LIMITED; 999 = MJPG, 3 = JPEG): an Enum choice lists
    // its default first, so values repeat (SDL_AddCamera() weeds them out).
    let mut want = Vec::new();
    for fmt in [PixelFormat::YUY2, PixelFormat::YUY2, PixelFormat::NV12] {
        for (w, h) in [(640, 480), (640, 480), (320, 240)] {
            for (n, d) in [(30, 1), (30, 1), (15, 1)] {
                want.push((fmt, Colorspace::BT709_LIMITED, w, h, n, d));
            }
        }
    }
    assert_eq!(want.len(), 27); // "enum-raw-total 27"
    assert_eq!(collect(ENUM_RAW), want);

    assert_eq!(
        collect(MJPG),
        [(PixelFormat::MJPG, Colorspace::JPEG, 1920, 1080, 60, 1)]
    );
    // Ranges aren't taken (and logged as unimplemented).
    assert_eq!(collect(RANGE), []);
    assert_eq!(collect(MIXED), []);

    // Not an object, or no subtype: nothing.
    let int = [4u8, 0, 0, 0, 4, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0];
    let mut specs = Vec::new();
    collect_format(&mut specs, Pod::new(&int).unwrap());
    assert!(specs.is_empty());

    // A truncated value list stops at the pod's end instead of reading on.
    let mut short = unhex(MJPG);
    let size_prop = short.len() - 40; // the size property's value
    assert_eq!(short[size_prop + 4], 10); // a Rectangle
    short[size_prop] = 4; // (claims 4 bytes)
    let pod = Pod::new(&short).unwrap();
    let mut specs = Vec::new();
    collect_format(&mut specs, pod);
    assert!(specs.is_empty());
}

#[test]
fn format_table() {
    for (sdl, cs, id) in [
        (PixelFormat::RGBX32, Colorspace::SRGB, 7),
        (PixelFormat::XRGB32, Colorspace::SRGB, 9),
        (PixelFormat::BGRX32, Colorspace::SRGB, 8),
        (PixelFormat::XBGR32, Colorspace::SRGB, 10),
        (PixelFormat::RGBA32, Colorspace::SRGB, 11),
        (PixelFormat::ARGB32, Colorspace::SRGB, 13),
        (PixelFormat::BGRA32, Colorspace::SRGB, 12),
        (PixelFormat::ABGR32, Colorspace::SRGB, 14),
        (PixelFormat::RGB24, Colorspace::SRGB, 15),
        (PixelFormat::BGR24, Colorspace::SRGB, 16),
        (PixelFormat::YV12, Colorspace::BT709_LIMITED, 3),
        (PixelFormat::IYUV, Colorspace::BT709_LIMITED, 2),
        (PixelFormat::YUY2, Colorspace::BT709_LIMITED, 4),
        (PixelFormat::UYVY, Colorspace::BT709_LIMITED, 5),
        (PixelFormat::YVYU, Colorspace::BT709_LIMITED, 19),
        (PixelFormat::NV12, Colorspace::BT709_LIMITED, 23),
        (PixelFormat::NV21, Colorspace::BT709_LIMITED, 24),
    ] {
        assert_eq!(sdl_format_to_id(sdl), id, "{sdl:?}");
        assert_eq!(id_to_sdl_format(id), (sdl, cs));
    }
    assert_eq!(
        sdl_format_to_id(PixelFormat::MJPG),
        SPA_VIDEO_FORMAT_UNKNOWN
    );
    assert_eq!(
        id_to_sdl_format(25), // SPA_VIDEO_FORMAT_GRAY8
        (PixelFormat::UNKNOWN, Colorspace::UNKNOWN)
    );
    // pw_ref.txt "consts"
    assert_eq!(
        (
            SPA_MEDIA_TYPE_VIDEO,
            SPA_MEDIA_SUBTYPE_RAW,
            SPA_MEDIA_SUBTYPE_MJPG
        ),
        (2, 1, 131074)
    );
    assert_eq!(
        (
            SPA_FORMAT_VIDEO_FORMAT,
            SPA_FORMAT_VIDEO_SIZE,
            SPA_FORMAT_VIDEO_FRAMERATE
        ),
        (0x20001, 0x20003, 0x20004)
    );
    assert_eq!((SPA_TYPE_RECTANGLE, SPA_TYPE_FRACTION), (10, 11));
    assert!(spa_result_is_async(0x4000_0005));
    assert!(!spa_result_is_async(5));
    assert!(!spa_result_is_async(-22));
}

fn p(id: u32, seq: i32, param: Option<&[u8]>) -> Param {
    Param {
        id,
        seq,
        param: param.map(<[u8]>::to_vec),
    }
}

#[test]
fn param_lists() {
    let pod = unhex(MJPG);
    let pod = Pod::new(&pod).unwrap();
    let bytes = pod.as_bytes();

    // param_add(): with no pod, the list's params of that id are cleared
    // and a marker added.
    let mut pending = Vec::new();
    assert!(param_add(&mut pending, 1, 3, Some(pod)));
    assert!(param_add(&mut pending, 1, 4, Some(pod)));
    assert!(param_add(&mut pending, 2, 3, None));
    assert_eq!(pending, [p(4, 1, Some(bytes)), p(3, 2, None)]);
    // The id from the object (SPA_PARAM_EnumFormat), or EINVAL.
    assert!(param_add(&mut pending, 5, SPA_ID_INVALID, Some(pod)));
    assert_eq!(pending[2], p(3, 5, Some(bytes)));
    assert!(!param_add(&mut pending, 5, SPA_ID_INVALID, None));
    let not_object = [4u8, 0, 0, 0, 4, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0];
    assert!(!param_add(
        &mut pending,
        5,
        SPA_ID_INVALID,
        Pod::new(&not_object)
    ));
    assert_eq!(pending.len(), 3);

    // param_clear()
    let mut list = vec![
        p(3, 0, Some(b"a")),
        p(4, 0, Some(b"b")),
        p(3, 0, Some(b"c")),
    ];
    assert_eq!(param_clear(&mut list, 3), 2);
    assert_eq!(list, [p(4, 0, Some(b"b"))]);
    assert_eq!(param_clear(&mut list, SPA_ID_INVALID), 1);
    assert!(list.is_empty());

    // param_update(): stale results (another seq) are dropped, markers
    // clear the list, the rest is appended.
    let info = |id, seq| SpaParamInfo {
        id,
        flags: 0,
        user: 0,
        seq,
        padding: [0; 4],
    };
    let mut list = vec![p(3, 1, Some(b"old")), p(4, 1, Some(b"keep"))];
    let mut pending = vec![
        p(3, 7, None),
        p(3, 6, Some(b"stale")),
        p(3, 7, Some(b"new1")),
        p(3, 7, Some(b"new2")),
    ];
    param_update(&mut list, &mut pending, &[info(3, 7)]);
    assert!(pending.is_empty());
    assert_eq!(
        list,
        [
            p(4, 1, Some(b"keep")),
            p(3, 7, Some(b"new1")),
            p(3, 7, Some(b"new2"))
        ]
    );
}

#[test]
fn abi_layouts() {
    use std::mem::{offset_of, size_of};
    #[cfg(target_pointer_width = "64")]
    {
        // pw_ref.txt "sizes"
        assert_eq!(size_of::<PwStreamEvents>(), 96);
        assert_eq!(size_of::<PwProxyEvents>(), 56);
        assert_eq!(size_of::<PwNodeInfo>(), 72);
        assert_eq!(offset_of!(PwProxyEvents, removed), 24);
        assert_eq!(offset_of!(PwStreamEvents, remove_buffer), 56);
        assert_eq!(offset_of!(PwStreamEvents, param_changed), 40);
        assert_eq!(offset_of!(PwBuffer, time), 32);
    }
    // "versions"
    assert_eq!(PW_VERSION_STREAM_EVENTS, 2);
    assert_eq!(PW_VERSION_PROXY_EVENTS, 1);
    assert_eq!(PW_VERSION_NODE, 3);
    assert_eq!(PW_VERSION_NODE_EVENTS, 0);
}

#[test]
fn version_parsing() {
    assert_eq!(parse_version("1.2.7"), (1, 2, 7));
    assert_eq!(parse_version("0.3.48"), (0, 3, 48));
    assert_eq!(parse_version("1.2"), (0, 0, 0));
    assert_eq!(parse_version(""), (0, 0, 0));
}

// --- against a server ---

/// A camera of our own: an output stream with media.class Video/Source
/// offering one fixed format, YUY2 160x120 at 30 fps, whose frames are a
/// uniform gray (getting lighter).
struct FakeCamera {
    lib: Arc<PwLib>,
    thread_loop: *mut PwThreadLoop,
    context: *mut PwContext,
    core: *mut PwCore,
    stream: *mut PwStream,
    listener: Box<UnsafeCell<SpaHook>>,
}

const FAKE_W: u32 = 160;
const FAKE_H: u32 = 120;
static FAKE_FRAMES: AtomicU32 = AtomicU32::new(0);

unsafe extern "C" fn fake_process(data: *mut c_void) {
    // SAFETY: the userdata is the live FakeCamera; its stream is live; a
    // dequeued buffer's data is mapped, `maxsize` bytes.
    unsafe {
        let fake = &*data.cast::<FakeCamera>();
        let b = (fake.lib.pw_stream_dequeue_buffer)(fake.stream);
        if b.is_null() {
            return;
        }
        let d = &mut *(*(*b).buffer).datas;
        if !d.data.is_null() {
            let size = (FAKE_W * FAKE_H * 2).min(d.maxsize);
            let n = FAKE_FRAMES.fetch_add(1, Ordering::SeqCst);
            let y = (n % 200) as u8 + 16;
            let pixels = std::slice::from_raw_parts_mut(d.data.cast::<u8>(), size as usize);
            for (i, b) in pixels.iter_mut().enumerate() {
                *b = if i % 2 == 0 { y } else { 128 }; // Y0 U Y1 V: gray
            }
            let chunk = &mut *d.chunk;
            chunk.offset = 0;
            chunk.size = size;
            chunk.stride = (FAKE_W * 2) as i32;
        }
        (fake.lib.pw_stream_queue_buffer)(fake.stream, b);
    }
}

static FAKE_EVENTS: PwStreamEvents = PwStreamEvents {
    version: PW_VERSION_STREAM_EVENTS,
    destroy: None,
    state_changed: None,
    control_info: None,
    io_changed: None,
    param_changed: None,
    add_buffer: None,
    remove_buffer: None,
    process: Some(fake_process),
    drained: None,
    command: None,
    trigger_done: None,
};

impl FakeCamera {
    fn new(lib: &Arc<PwLib>) -> Option<Box<FakeCamera>> {
        let mut fake = Box::new(FakeCamera {
            lib: lib.clone(),
            thread_loop: ptr::null_mut(),
            context: ptr::null_mut(),
            core: ptr::null_mut(),
            stream: ptr::null_mut(),
            listener: Box::new(SpaHook::zeroed()),
        });
        // SAFETY: plain libpipewire setup calls on objects made here; the
        // listener is zeroed and boxed, and the FakeCamera (its userdata)
        // outlives the stream (see Drop).
        unsafe {
            fake.thread_loop = (lib.pw_thread_loop_new)(c"SDLFakeCamera".as_ptr(), ptr::null());
            if fake.thread_loop.is_null() {
                return None;
            }
            fake.context = (lib.pw_context_new)(
                (lib.pw_thread_loop_get_loop)(fake.thread_loop),
                ptr::null_mut(),
                0,
            );
            if fake.context.is_null() {
                return None;
            }
            fake.core = (lib.pw_context_connect)(fake.context, ptr::null_mut(), 0);
            if fake.core.is_null() {
                return None;
            }
            let props = (lib.pw_properties_new)(
                PW_KEY_MEDIA_CLASS_C.as_ptr(),
                c"Video/Source".as_ptr(),
                PW_KEY_NODE_NAME.as_ptr(),
                c"sdl-fake-camera".as_ptr(),
                PW_KEY_NODE_DESCRIPTION.as_ptr(),
                c"SDL Fake Camera".as_ptr(),
                ptr::null::<c_char>(),
            );
            fake.stream = (lib.pw_stream_new)(fake.core, c"sdl-fake-camera".as_ptr(), props);
            if fake.stream.is_null() {
                return None;
            }
            let data: *mut c_void = ptr::from_mut(&mut *fake).cast();
            (lib.pw_stream_add_listener)(fake.stream, fake.listener.get(), &FAKE_EVENTS, data);

            let mut buf = [0u64; 128];
            let bytes = std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), 1024);
            let format = build_enum_format(
                &mut bytes[..512],
                &spec(PixelFormat::YUY2, FAKE_W as i32, FAKE_H as i32, 30, 1),
            )?;
            // SPA_TYPE_OBJECT_ParamBuffers / SPA_PARAM_Buffers: 4 buffers of
            // one block of a frame each.
            let (head, tail) = bytes.split_at_mut(512);
            let mut b = SpaPodBuilder::new(tail);
            b.push_object(0x40004, 5);
            for (key, value) in [
                (1, 4),
                (2, 1),
                (3, (FAKE_W * FAKE_H * 2) as i32),
                (4, (FAKE_W * 2) as i32),
            ] {
                b.prop(key, 0);
                b.int(value);
            }
            let buffers = b.pop()?;
            let mut params = [head[format].as_ptr(), tail[buffers].as_ptr()];
            let res = (lib.pw_stream_connect)(
                fake.stream,
                PW_DIRECTION_OUTPUT,
                PW_ID_ANY,
                PW_STREAM_FLAG_MAP_BUFFERS,
                params.as_mut_ptr(),
                2,
            );
            if res < 0 || (lib.pw_thread_loop_start)(fake.thread_loop) != 0 {
                return None;
            }
        }
        Some(fake)
    }
}

const PW_KEY_MEDIA_CLASS_C: &std::ffi::CStr = c"media.class";

impl Drop for FakeCamera {
    fn drop(&mut self) {
        let lib = &self.lib;
        // SAFETY: the objects made in new(), destroyed once, the stream with
        // the loop locked, then the loop stopped.
        unsafe {
            if !self.thread_loop.is_null() {
                (lib.pw_thread_loop_lock)(self.thread_loop);
            }
            if !self.stream.is_null() {
                (lib.pw_stream_destroy)(self.stream);
            }
            if !self.core.is_null() {
                (lib.pw_core_disconnect)(self.core);
            }
            if !self.context.is_null() {
                (lib.pw_context_destroy)(self.context);
            }
            if !self.thread_loop.is_null() {
                (lib.pw_thread_loop_unlock)(self.thread_loop);
                (lib.pw_thread_loop_stop)(self.thread_loop);
                (lib.pw_thread_loop_destroy)(self.thread_loop);
            }
        }
    }
}

fn wait_for(what: &dyn Fn() -> bool) -> bool {
    let start = Instant::now();
    while !what() && start.elapsed() < Duration::from_secs(10) {
        crate::events::pump();
        std::thread::sleep(Duration::from_millis(10));
    }
    what()
}

#[test]
fn a_camera_through_a_server() {
    let _l = crate::test_support::test_lock();
    let lib = match init_pipewire_library() {
        Ok(lib) => lib,
        Err(e) => {
            crate::test_support::skip("pipewire", e.message());
            return;
        }
    };

    crate::hints::set(crate::hints::CAMERA_DRIVER, "pipewire").unwrap();
    let r = crate::init::init_subsystem(InitFlags::CAMERA);
    crate::hints::reset(crate::hints::CAMERA_DRIVER);
    if let Err(e) = r {
        crate::test_support::skip("pipewire", format_args!("no server ({})", e.message()));
        deinit_pipewire_library(&lib);
        return;
    }
    assert_eq!(current_camera_driver(), Some("pipewire"));
    let before = cameras().unwrap();
    eprintln!("note: PipeWire cameras before: {}", before.len());

    let Some(fake) = FakeCamera::new(&lib) else {
        crate::test_support::skip("pipewire", "couldn't make a video source");
        crate::init::quit_subsystem(InitFlags::CAMERA);
        deinit_pipewire_library(&lib);
        return;
    };

    // Hotplug: it shows up, with its one spec.
    let find = || {
        cameras()
            .unwrap()
            .into_iter()
            .find(|&id| camera_name(id).is_ok_and(|n| n == "SDL Fake Camera"))
    };
    assert!(wait_for(&|| find().is_some()), "the camera was added");
    let id = find().unwrap();
    let specs = camera_supported_formats(id).unwrap();
    assert_eq!(specs.len(), 1);
    assert_eq!(
        (specs[0].format, specs[0].width, specs[0].height),
        (PixelFormat::YUY2, FAKE_W as i32, FAKE_H as i32)
    );
    assert_eq!(
        (specs[0].framerate_numerator, specs[0].framerate_denominator),
        (30, 1)
    );

    // Frames come through (converted to XRGB8888 here).
    let camera = Camera::open(
        id,
        Some(&CameraSpec {
            format: PixelFormat::XRGB8888,
            ..specs[0]
        }),
    )
    .unwrap();
    assert!(
        wait_for(&|| camera.permission_state() == crate::camera::CameraPermissionState::Approved),
        "the stream streams"
    );
    let start = Instant::now();
    let frame = loop {
        if let Some(frame) = camera.acquire_frame().unwrap() {
            break frame;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "a frame arrived");
        std::thread::sleep(Duration::from_millis(5));
    };
    let (w, h, format) = (frame.width(), frame.height(), frame.format());
    let first = frame.pixels().map(|p| p[..4].to_vec());
    drop(frame);
    assert_eq!(
        (w, h, format),
        (FAKE_W as i32, FAKE_H as i32, PixelFormat::XRGB8888)
    );
    // A uniform YUY2 frame is a uniform gray.
    let first = first.unwrap();
    assert_eq!(first[0], first[1]);
    assert_eq!(first[1], first[2]);
    eprintln!("note: PipeWire camera frame pixel: {first:?}");
    drop(camera);

    drop(fake);
    crate::init::quit_subsystem(InitFlags::CAMERA);
    deinit_pipewire_library(&lib);
}

/// Hardware: grab frames from the first camera the PipeWire server offers
/// (through libcamera or its V4L2 monitor).
#[test]
#[ignore = "hardware: needs a camera and a PipeWire server"]
fn hardware_capture_from_the_first_camera() {
    let _l = crate::test_support::test_lock();
    crate::camera::tests::hardware_capture("pipewire", "pipewire");
}
