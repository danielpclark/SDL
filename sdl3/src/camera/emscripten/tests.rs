// Tests of the Emscripten camera driver under node (in a browser:
// sdl3/tests/emscripten_camera.rs).
//
// Node's `navigator` has no `mediaDevices`, so there the driver starts and
// reports no camera, as upstream's. With a stand-in
// `navigator.mediaDevices` it reports the browser's one camera, and opening
// it asks the stand-in's `getUserMedia()`, which records the constraints and
// never answers; the tests then call the driver's callbacks by their
// names, as the JavaScript does when the browser answers and for each frame
// (without a DOM, a stand-in 2D context gives the frames' pixels).

use std::ffi::CString;

use sdl3_emscripten::{em_asm, em_asm_int};

use super::*;
use crate::camera::{
    camera_name, camera_position, camera_supported_formats, cameras, current_camera_driver, Camera,
    CameraPermissionState,
};
use crate::events::{get_events, pump, Event, EventType};
use crate::init::{self, InitFlags};

// -- JavaScript ---------------------------------------------------------------

/// The value of a JavaScript expression, as a string.
fn js_str(expr: &str) -> String {
    let code = CString::new(expr).unwrap();
    // SAFETY: plain JavaScript; the code is NUL-terminated.
    let len = unsafe {
        em_asm_int!(
            "Module['sdl3_test_s'] = new TextEncoder().encode(String(eval(UTF8ToString($0)))); return Module['sdl3_test_s'].length;",
            code.as_ptr()
        )
    };
    let mut buf = vec![0u8; len as usize];
    // SAFETY: writes the `len` bytes made above.
    unsafe {
        em_asm!(
            "HEAPU8.set(Module['sdl3_test_s'], $0); delete Module['sdl3_test_s'];",
            buf.as_mut_ptr()
        )
    };
    String::from_utf8(buf).unwrap()
}

/// Run JavaScript statements.
fn js_run(code: &str) {
    let code = CString::new(format!("(function() {{ {code} }})()")).unwrap();
    // SAFETY: plain JavaScript; the code is NUL-terminated.
    unsafe { em_asm!("eval(UTF8ToString($0));", code.as_ptr()) };
}

/// Whether this JavaScript has media devices of its own (a browser).
fn has_media_devices() -> bool {
    js_str("typeof navigator === 'object' && navigator.mediaDevices !== undefined") == "true"
}

/// A stand-in `navigator.mediaDevices`: its `getUserMedia(constraints)`
/// records the constraints (as JSON) in `Module.sdl3_test_cam.asked` and
/// returns a promise that never settles. Taken away when dropped.
struct MediaDevices;

impl MediaDevices {
    fn new() -> MediaDevices {
        js_run(
            r#"
            var M = Module['sdl3_test_cam'] = { asked: [], draws: [], stopped: 0 };
            Object.defineProperty(navigator, 'mediaDevices', {
                value: { getUserMedia: function(c) { M.asked.push(JSON.stringify(c)); return new Promise(function() {}); } },
                configurable: true, writable: true
            });
        "#,
        );
        MediaDevices
    }

    /// The constraints asked for since the last call, `|`-separated.
    fn asked(&self) -> String {
        js_str("(function() { var M = Module['sdl3_test_cam']; var a = M.asked.join('|'); M.asked = []; return a; })()")
    }

    /// A stand-in for the streaming state the JavaScript sets up once a
    /// camera is approved: a `stream` whose track counts its `stop()`s, and
    /// a 2D context whose `drawImage()` records its arguments and whose
    /// `getImageData(x, y, w, h)` gives `w` × `h` pixels, byte `i` of them
    /// `(seed + i) & 255` (alpha too).
    fn stream(&self, seed: u32) {
        js_run(&format!(
            r#"
            var M = Module['sdl3_test_cam'];
            var C = Module['SDL3'].camera;
            C.video = {{ sdl3_test: true }};
            C.stream = {{ getTracks: function() {{ return [{{ stop: function() {{ M.stopped++; }} }}]; }} }};
            C.ctx2d = {{
                drawImage: function(v, x, y, w, h) {{ M.draws.push((v.sdl3_test ? 'video' : '?') + ' ' + [x, y, w, h].join(',')); }},
                getImageData: function(x, y, w, h) {{
                    var d = new Uint8ClampedArray(w * h * 4);
                    for (var i = 0; i < d.length; i++) d[i] = ({seed} + i) & 255;
                    return {{ data: d }};
                }}
            }};
        "#
        ));
    }
}

impl Drop for MediaDevices {
    fn drop(&mut self) {
        js_run("delete navigator.mediaDevices; delete Module['sdl3_test_cam'];");
    }
}

/// `_SDLEmscriptenCameraPermissionOutcome(device, approved, w, h, fps)`,
/// as the JavaScript calls it; its result.
fn outcome(devid: CameraID, approved: i32, w: i32, h: i32, fps: &str) -> String {
    js_str(&format!(
        "Module._SDLEmscriptenCameraPermissionOutcome({devid}, {approved}, {w}, {h}, {fps})"
    ))
}

/// `_SDLEmscriptenThreadIterate(device)`, as the frame loop calls it; its
/// result.
fn iterate(devid: CameraID) -> String {
    js_str(&format!("Module._SDLEmscriptenThreadIterate({devid})"))
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

/// Start the camera subsystem (the Emscripten driver, the first one);
/// its one camera, with its added event taken.
fn start() -> CameraID {
    crate::hints::reset(crate::hints::CAMERA_DRIVER);
    init::init_subsystem(InitFlags::CAMERA).unwrap();
    assert_eq!(current_camera_driver(), Some("emscripten"));
    let ids = cameras().unwrap();
    assert_eq!(ids.len(), 1);
    assert_eq!(camera_events(), [(EventType::CAMERA_DEVICE_ADDED, ids[0])]);
    ids[0]
}

fn stop() {
    init::quit_subsystem(InitFlags::CAMERA);
    assert_eq!(current_camera_driver(), None);
}

// -- the tests ----------------------------------------------------------------

#[test]
fn no_camera_without_media_devices() {
    let _l = crate::test_support::test_lock();
    if has_media_devices() {
        return; // (a browser)
    }
    // The driver starts (it's the first), as upstream's, with nothing to
    // report.
    crate::hints::reset(crate::hints::CAMERA_DRIVER);
    init::init_subsystem(InitFlags::CAMERA).unwrap();
    assert_eq!(current_camera_driver(), Some("emscripten"));
    assert!(cameras().unwrap().is_empty());
    assert_eq!(js_str("JSON.stringify(Module['SDL3'].camera)"), "{}");
    stop();
    assert_eq!(js_str("typeof Module['SDL3'].camera"), "undefined");
}

#[test]
fn one_camera_and_the_constraints_asked_for() {
    let _l = crate::test_support::test_lock();
    if has_media_devices() {
        return; // (a browser)
    }
    let media = MediaDevices::new();
    let id = start();
    assert_eq!(camera_name(id).unwrap(), "Web browser's camera");
    assert_eq!(camera_position(id), CameraPosition::Unknown);
    assert!(camera_supported_formats(id).unwrap().is_empty());
    assert_eq!(media.asked(), ""); // (nothing asked at initialization)

    // Any video, without a request; pending until the browser answers.
    let camera = Camera::open(id, None).unwrap();
    assert_eq!(media.asked(), r#"{"video":true}"#);
    assert_eq!(camera.permission_state(), CameraPermissionState::Pending);
    assert_eq!(
        camera.format().unwrap_err().message(),
        "Camera permission has not been granted"
    );
    assert_eq!(
        camera.acquire_frame().unwrap_err().message(),
        "Camera permission has not been granted"
    );
    assert_eq!(
        Camera::open(id, None).unwrap_err().message(),
        "Camera already opened"
    );
    drop(camera);
    // (no stream: closing leaves the table as it is)
    assert_eq!(js_str("JSON.stringify(Module['SDL3'].camera)"), "{}");
    assert!(camera_events().is_empty());

    let ask = |format, width, height, num, den| {
        let spec = CameraSpec {
            format,
            width,
            height,
            framerate_numerator: num,
            framerate_denominator: den,
            ..CameraSpec::default()
        };
        drop(Camera::open(id, Some(&spec)).unwrap());
        media.asked()
    };
    // A size and a frame rate, as the ideal ones.
    assert_eq!(
        ask(PixelFormat::XRGB8888, 320, 240, 30, 1),
        r#"{"video":{"width":320,"height":240,"frameRate":{"ideal":30}}}"#
    );
    assert_eq!(
        ask(PixelFormat::UNKNOWN, 640, 480, 30000, 1001),
        r#"{"video":{"width":640,"height":480,"frameRate":{"ideal":29.97002997002997}}}"#
    );
    assert_eq!(
        ask(PixelFormat::UNKNOWN, 640, 480, 0, 1),
        r#"{"video":{"width":640,"height":480}}"#
    );
    // Note (upstream): a frame rate without a size is dropped.
    assert_eq!(ask(PixelFormat::UNKNOWN, 0, 0, 30, 1), r#"{"video":true}"#);
    stop();
    drop(media);
}

#[test]
fn approved_frames_and_close() {
    let _l = crate::test_support::test_lock();
    if has_media_devices() {
        return; // (a browser)
    }
    let media = MediaDevices::new();
    let id = start();
    let camera = Camera::open(id, None).unwrap();

    // The browser's answer: a 4x2 stream at 25 fps.
    assert_eq!(outcome(id, 1, 4, 2, "25"), "1");
    assert_eq!(camera.permission_state(), CameraPermissionState::Approved);
    assert_eq!(camera_events(), [(EventType::CAMERA_DEVICE_APPROVED, id)]);
    let spec = camera.format().unwrap();
    assert_eq!(
        (
            spec.format,
            spec.width,
            spec.height,
            spec.framerate_numerator,
            spec.framerate_denominator
        ),
        (PixelFormat::RGBA32, 4, 2, 25, 1)
    );

    // The frames: the first one is dropped, the next one is the app's.
    media.stream(7);
    assert_eq!(iterate(id), "1");
    assert!(camera.acquire_frame().unwrap().is_none());
    let before = crate::timer::ticks_ns();
    assert_eq!(iterate(id), "1");
    let frame = camera.acquire_frame().unwrap().expect("a frame");
    assert_eq!(
        (frame.width(), frame.height(), frame.format(), frame.pitch()),
        (4, 2, PixelFormat::RGBA32, 16)
    );
    let want: Vec<u8> = (0..32u32).map(|i| (7 + i) as u8).collect();
    assert_eq!(frame.pixels().unwrap(), &want[..]);
    assert!(frame.timestamp_ns() >= before && frame.timestamp_ns() <= crate::timer::ticks_ns());
    assert_eq!(
        js_str("Module['sdl3_test_cam'].draws.join('|')"),
        "video 0,0,4,2|video 0,0,4,2"
    );
    drop(frame);

    // Closing stops the stream's tracks and empties the table.
    drop(camera);
    assert_eq!(js_str("String(Module['sdl3_test_cam'].stopped)"), "1");
    assert_eq!(js_str("JSON.stringify(Module['SDL3'].camera)"), "{}");
    // (an iteration that comes after, from a loop of the closed camera,
    // does nothing)
    assert_eq!(iterate(id), "0");
    assert!(camera_events().is_empty());
    stop();
    drop(media);
}

#[test]
fn a_requested_format_is_converted_and_scaled() {
    let _l = crate::test_support::test_lock();
    if has_media_devices() {
        return; // (a browser)
    }
    let media = MediaDevices::new();
    let id = start();
    let spec = CameraSpec {
        format: PixelFormat::XRGB8888,
        width: 2,
        height: 1,
        framerate_numerator: 25,
        framerate_denominator: 1,
        ..CameraSpec::default()
    };
    // (the core prepares the surfaces now, for the requested spec, and the
    // answer prepares them again: Note (upstream))
    let camera = Camera::open(id, Some(&spec)).unwrap();
    assert_eq!(outcome(id, 1, 4, 2, "29.97"), "1");
    let got = camera.format().unwrap();
    assert_eq!(
        (
            got.format,
            got.width,
            got.height,
            got.framerate_numerator,
            got.framerate_denominator
        ),
        (PixelFormat::XRGB8888, 2, 1, 25, 1)
    );
    media.stream(0);
    assert_eq!(iterate(id), "1");
    assert_eq!(iterate(id), "1");
    let frame = camera.acquire_frame().unwrap().expect("a frame");
    assert_eq!(
        (frame.width(), frame.height(), frame.format(), frame.pitch()),
        (2, 1, PixelFormat::XRGB8888, 8)
    );
    drop(frame);
    drop(camera);

    // The browser's frame rate as an integer.
    let camera = Camera::open(id, None).unwrap();
    assert_eq!(outcome(id, 1, 4, 2, "29.97"), "1");
    assert_eq!(camera.format().unwrap().framerate_numerator, 29);
    drop(camera);
    stop();
    drop(media);
}

#[test]
fn denied_closed_and_unknown_cameras() {
    let _l = crate::test_support::test_lock();
    if has_media_devices() {
        return; // (a browser)
    }
    let media = MediaDevices::new();
    let id = start();

    // A failure is a denial.
    let camera = Camera::open(id, None).unwrap();
    assert_eq!(outcome(id, 0, 0, 0, "0"), "0");
    assert_eq!(camera.permission_state(), CameraPermissionState::Denied);
    assert_eq!(camera_events(), [(EventType::CAMERA_DEVICE_DENIED, id)]);
    assert!(camera.acquire_frame().is_err());
    // (a denied camera's iterations end)
    assert_eq!(iterate(id), "0");
    drop(camera);

    // Note (upstream): an answer for a camera closed meanwhile is refused.
    drop(Camera::open(id, None).unwrap());
    assert_eq!(outcome(id, 1, 4, 2, "25"), "0");
    assert!(camera_events().is_empty());
    let camera = Camera::open(id, None).unwrap();
    assert_eq!(camera.permission_state(), CameraPermissionState::Pending);
    drop(camera);

    // Nor is there anything for a camera that isn't there.
    assert_eq!(outcome(id + 1000, 1, 4, 2, "25"), "0");
    assert_eq!(iterate(id + 1000), "0");

    // Without a 2D context, the frame is an error: the camera is lost.
    let camera = Camera::open(id, None).unwrap();
    assert_eq!(outcome(id, 1, 4, 2, "25"), "1");
    assert_eq!(camera_events(), [(EventType::CAMERA_DEVICE_APPROVED, id)]);
    assert_eq!(iterate(id), "1");
    assert_eq!(camera_events(), [(EventType::CAMERA_DEVICE_REMOVED, id)]);
    drop(camera);
    stop();
    drop(media);
}
