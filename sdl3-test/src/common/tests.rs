// Tests of the common state: the options, usage and event logs of upstream's
// SDL_test_common.c, and a session on the dummy video driver.

use std::time::Duration;

use sdl3::events::keyboard::Scancode;
use sdl3::events::{
    DisplayEvent, DropEvent, GamepadButtonEvent, JoyHatEvent, KeyboardEvent, MouseMotionEvent,
    MouseWheelEvent, TouchFingerEvent, UserEvent, WindowEvent,
};
use sdl3::video::pixels::Color;

use super::*;
use crate::harness::{TestCase, TestData, TestStatus, TestSuite, TestSuiteRunner};
use crate::test_support::{test_lock, Capture};

fn state(args: &[&str], flags: InitFlags) -> CommonState {
    CommonState::new(args.iter().map(|a| (*a).to_owned()).collect(), flags)
}

/// Parse every argument, returning what each call returned.
fn parse(state: &mut CommonState) -> Vec<i32> {
    let mut results = Vec::new();
    let mut i = 1;
    while i < state.argv.len() {
        let consumed = state.common_arg(i);
        results.push(consumed);
        if consumed <= 0 {
            break;
        }
        i += consumed as usize;
    }
    results
}

#[test]
fn defaults() {
    let _l = test_lock();
    let state = state(&["prog"], InitFlags::VIDEO);
    assert_eq!(state.window_title, "prog");
    assert_eq!(state.window_flags, WindowFlags::HIDDEN);
    assert_eq!(state.window_x, video::WINDOWPOS_UNDEFINED);
    assert_eq!((state.window_w, state.window_h), (640, 480));
    assert_eq!(state.num_windows, 1);
    assert_eq!(state.audio_freq, 22050);
    assert_eq!(state.audio_format, AudioFormat::S16);
    assert_eq!(state.audio_channels, 2);
    assert_eq!(
        (
            state.gl_red_size,
            state.gl_depth_size,
            state.gl_double_buffer
        ),
        (8, 16, 1)
    );
    assert_eq!((state.gl_retained_backing, state.gl_accelerated), (1, -1));
    assert_eq!(state.logical_presentation, LogicalPresentation::Disabled);
}

#[test]
fn video_and_audio_options() {
    let _l = test_lock();
    let _capture = Capture::new();
    let mut s = state(
        &[
            "prog",
            "--geometry",
            "320x200",
            "--min-geometry",
            "10x20",
            "--max-geometry",
            "1000x2000",
            "--position",
            "5,-6",
            "--confine-cursor",
            "1,2,3,4,",
            "--aspect",
            "1.5-2",
            "--logical",
            "160x100",
            "--logical-presentation",
            "LETTERBOX",
            "--scale",
            "2.5",
            "--depth",
            "24",
            "--refresh",
            "59.94",
            "--display",
            "1",
            "--windows",
            "3",
            "--title",
            "My test",
            "--icon",
            "icon.bmp",
            "--resizable",
            "--noframe",
            "--vsync",
            "--hide-cursor",
            "--gldebug",
            "--info",
            "event_motion",
            "--rate",
            "48000",
            "--format",
            "f32be",
            "--channels",
            "262",
            "--quit-after-ms",
            "50",
        ],
        InitFlags::VIDEO | InitFlags::AUDIO,
    );
    let results = parse(&mut s);
    assert!(results.iter().all(|&r| r > 0), "{results:?}");
    assert_eq!((s.window_w, s.window_h), (320, 200));
    assert_eq!((s.window_min_w, s.window_min_h), (10, 20));
    assert_eq!((s.window_max_w, s.window_max_h), (1000, 2000));
    assert_eq!((s.window_x, s.window_y), (5, -6));
    assert_eq!(s.confine, Rect::new(1, 2, 3, 4));
    assert_eq!((s.window_min_aspect, s.window_max_aspect), (1.5, 2.0));
    assert_eq!((s.logical_w, s.logical_h), (160, 100));
    assert_eq!(s.logical_presentation, LogicalPresentation::Letterbox);
    assert_eq!((s.scale, s.depth, s.refresh_rate), (2.5, 24, 59.94));
    assert_eq!((s.display_index, s.num_windows), (1, 3));
    assert_eq!(s.window_title, "My test");
    assert_eq!(s.window_icon.as_deref(), Some("icon.bmp"));
    assert_eq!(
        s.window_flags,
        WindowFlags::HIDDEN | WindowFlags::RESIZABLE | WindowFlags::BORDERLESS
    );
    assert_eq!((s.render_vsync, s.hide_cursor, s.gl_debug), (1, true, 1));
    assert_eq!(s.verbose, VerboseFlags::EVENT | VerboseFlags::MOTION);
    assert_eq!(s.audio_freq, 48000);
    assert_eq!(s.audio_format, AudioFormat::F32BE);
    // (stored through a Uint8)
    assert_eq!(s.audio_channels, 6);
    assert_eq!(s.quit_after_ms_interval, 50);

    // An aspect without a maximum is both; fullscreen takes one window.
    let mut s = state(
        &[
            "prog",
            "--aspect",
            "0.75",
            "--fullscreen",
            "--windows",
            "4",
            "--center",
            "--auto-scale-content",
        ],
        InitFlags::VIDEO,
    );
    assert_eq!(parse(&mut s), [2, 1, 2, 1, 1]);
    assert_eq!((s.window_min_aspect, s.window_max_aspect), (0.75, 0.75));
    assert!(s.window_flags.contains(WindowFlags::FULLSCREEN) && s.fullscreen_exclusive);
    assert_eq!(s.num_windows, 1);
    assert_eq!(
        (s.window_x, s.window_y),
        (video::WINDOWPOS_CENTERED, video::WINDOWPOS_CENTERED)
    );
    assert!(s.auto_scale_content);
    assert_eq!(s.logical_presentation, LogicalPresentation::Stretch);

    // The flags.
    for (arg, flag) in [
        ("--metal-window", WindowFlags::METAL),
        ("--opengl-window", WindowFlags::OPENGL),
        ("--vulkan-window", WindowFlags::VULKAN),
        ("--fill-document", WindowFlags::FILL_DOCUMENT),
        ("--fullscreen-desktop", WindowFlags::FULLSCREEN),
        ("--high-pixel-density", WindowFlags::HIGH_PIXEL_DENSITY),
        ("--transparent", WindowFlags::TRANSPARENT),
        ("--always-on-top", WindowFlags::ALWAYS_ON_TOP),
        ("--minimize", WindowFlags::MINIMIZED),
        ("--maximize", WindowFlags::MAXIMIZED),
        ("--hidden", WindowFlags::HIDDEN),
        ("--input-focus", WindowFlags::INPUT_FOCUS),
        ("--mouse-focus", WindowFlags::MOUSE_FOCUS),
        ("--grab", WindowFlags::MOUSE_GRABBED),
        ("--keyboard-grab", WindowFlags::KEYBOARD_GRABBED),
        ("--utility", WindowFlags::UTILITY),
    ] {
        let mut s = state(&["prog", arg], InitFlags::VIDEO);
        assert_eq!(s.common_arg(1), 1, "{arg}");
        assert!(s.window_flags.contains(flag), "{arg}");
    }
    let mut s = state(&["prog", "--fullscreen-desktop"], InitFlags::VIDEO);
    s.common_arg(1);
    assert!(!s.fullscreen_exclusive);
    let mut s = state(
        &["prog", "--usable-bounds", "--flash-on-focus-loss"],
        InitFlags::VIDEO,
    );
    assert_eq!(parse(&mut s), [1, 1]);
    assert!(s.fill_usable_bounds && s.flash_on_focus_loss);
}

#[test]
fn invalid_options() {
    let _l = test_lock();
    let _capture = Capture::new();
    for args in [
        &["prog", "--geometry"][..],
        &["prog", "--geometry", "320"],
        &["prog", "--position", "5"],
        &["prog", "--confine-cursor", "1,2,3,4"],
        &["prog", "--windows", "x"],
        &["prog", "--logical-presentation", "match"],
        &["prog", "--quit-after-ms", "0"],
        &["prog", "--format", "S24"],
        &["prog", "--info", "nothing"],
        &["prog", "--log", "nothing"],
        &["prog", "--log"],
        &["prog", "-h"],
        &["prog", "--HELP"],
    ] {
        let mut s = state(args, InitFlags::VIDEO | InitFlags::AUDIO);
        assert_eq!(s.common_arg(1), -1, "{args:?}");
    }
    // Not common ones, or for a subsystem that isn't used.
    let mut s = state(&["prog", "--mine", "--rate", "1"], InitFlags::VIDEO);
    assert_eq!(s.common_arg(1), 0);
    assert_eq!(s.common_arg(2), 0);
    assert_eq!(s.common_arg(9), 0);
    let mut s = state(&["prog", "--geometry", "1x1"], InitFlags::AUDIO);
    assert_eq!(s.common_arg(1), 0);
    // Case matters for the Xcode flag only.
    let mut s = state(
        &["prog", "-NSDocumentRevisionsDebugMode", "YES"],
        InitFlags::NONE,
    );
    assert_eq!(s.common_arg(1), 2);
    let mut s = state(
        &["prog", "-nsdocumentrevisionsdebugmode", "YES"],
        InitFlags::NONE,
    );
    assert_eq!(s.common_arg(1), 0);
}

#[test]
fn logging_options_and_drivers() {
    let _l = test_lock();
    let _capture = Capture::new();
    let mut s = state(
        &[
            "prog",
            "--log",
            "video",
            "--no-color",
            "--no-time",
            "--video",
            "dummy",
            "--renderer",
            "software",
            "--gpu",
            "vulkan",
            "--audio",
            "disk",
        ],
        InitFlags::VIDEO | InitFlags::AUDIO,
    );
    crate::internal::set_color(true);
    crate::log::set_timestamps(true);
    assert_eq!(parse(&mut s), [2, 1, 1, 2, 2, 2, 2]);
    assert_eq!(
        sdl3::log::priority(Category::Video),
        sdl3::log::Threshold::AtLeast(Priority::Verbose)
    );
    assert!(!crate::internal::color());
    assert!(!crate::log::timestamps());
    assert_eq!(s.videodriver.as_deref(), Some("dummy"));
    assert_eq!(hints::get(hints::VIDEO_DRIVER).as_deref(), Some("dummy"));
    assert_eq!(
        hints::get(hints::RENDER_DRIVER).as_deref(),
        Some("software")
    );
    assert_eq!(hints::get(hints::GPU_DRIVER).as_deref(), Some("vulkan"));
    assert_eq!(hints::get(hints::AUDIO_DRIVER).as_deref(), Some("disk"));
    for hint in [
        hints::VIDEO_DRIVER,
        hints::RENDER_DRIVER,
        hints::GPU_DRIVER,
        hints::AUDIO_DRIVER,
    ] {
        hints::reset(hint);
    }
}

#[test]
fn usage_and_harness_options() {
    static NOTHING: TestSuite = TestSuite {
        name: "Nothing",
        test_set_up: None,
        test_cases: &[],
        test_tear_down: None,
    };
    let _l = test_lock();
    let capture = Capture::new();
    let mut s = state(
        &["prog", "--seed", "S", "--iterations", "2", "--bogus"],
        InitFlags::VIDEO,
    );
    let suites = [&NOTHING];
    let runner = TestSuiteRunner::new(&mut s, &suites);
    assert_eq!(s.common_arg(1), 2);
    assert_eq!(s.common_arg(3), 2);
    assert_eq!(runner.user().run_seed.as_deref(), Some("S"));
    assert_eq!(runner.user().test_iterations, 2);

    // A bad argument logs the usage: the common and video options (no
    // audio ones without the subsystem), then the harness's.
    assert!(!s.default_args());
    let mut expected = vec!["USAGE: prog".to_owned()];
    expected.extend(COMMON_USAGE.iter().map(|o| format!("    {o}")));
    expected.extend(VIDEO_USAGE.iter().map(|o| format!("    {o}")));
    expected.extend(TestSuiteRunner::usage().iter().map(|o| format!("    {o}")));
    assert_eq!(capture.messages(), expected);
    assert_eq!(expected[1], "    [-h | --help]");
    assert_eq!(expected.last().unwrap(), "    [--random-order]");

    // The application's own, and the audio ones.
    capture.clear();
    let mut s = state(&["prog"], InitFlags::AUDIO);
    s.log_usage("argv0", Some(&["[--mine]"]));
    let messages = capture.messages();
    assert_eq!(messages[0], "USAGE: argv0");
    assert_eq!(messages[8], "    [--audio driver]");
    assert_eq!(messages.last().unwrap(), "    [--mine]");
    assert_eq!(messages.len(), 1 + 7 + 4 + 1);
}

fn window_event(event_type: EventType, window_id: WindowID, data1: i32, data2: i32) -> Event {
    Event::Window(WindowEvent {
        event_type,
        timestamp: Duration::ZERO,
        window_id,
        data1,
        data2,
    })
}

fn key(key: Keycode, modifiers: Keymod, window_id: WindowID, down: bool) -> Event {
    Event::Key(KeyboardEvent {
        timestamp: Duration::ZERO,
        window_id,
        which: 0,
        scancode: Scancode::A,
        key,
        modifiers,
        raw: 0,
        down,
        repeat: false,
    })
}

#[test]
fn event_log() {
    let _l = test_lock();
    let capture = Capture::new();
    let common = |event_type| CommonEvent {
        event_type,
        timestamp: Duration::ZERO,
    };
    let events = [
        Event::Quit(common(EventType::QUIT)),
        Event::App(common(EventType::LOW_MEMORY)),
        Event::App(common(EventType::KEYMAP_CHANGED)),
        Event::Display(DisplayEvent {
            event_type: EventType::DISPLAY_ORIENTATION,
            timestamp: Duration::ZERO,
            display_id: 7,
            data1: 3,
            data2: 0,
        }),
        Event::Display(DisplayEvent {
            event_type: EventType::DISPLAY_DESKTOP_MODE_CHANGED,
            timestamp: Duration::ZERO,
            display_id: 7,
            data1: 800,
            data2: 600,
        }),
        window_event(EventType::WINDOW_MOVED, 3, -10, 20),
        window_event(EventType::WINDOW_RESIZED, 3, 640, 480),
        window_event(EventType::WINDOW_HDR_STATE_CHANGED, 3, 1, 0),
        window_event(EventType::WINDOW_MOUSE_ENTER, 3, 0, 0),
        window_event(EventType::WINDOW_SETTINGS_CHANGED, 3, 0, 0),
        key(
            Keycode::A,
            Keymod::LSHIFT | Keymod::RALT | Keymod::NUM,
            3,
            true,
        ),
        key(Keycode::ESCAPE, Keymod::NONE, 3, false),
        Event::MouseMotion(MouseMotionEvent {
            timestamp: Duration::ZERO,
            window_id: 3,
            which: 0,
            state: MouseButtonFlags::NONE,
            x: 10.5,
            y: 1234567.0,
            xrel: -0.25,
            yrel: 0.0,
        }),
        Event::MouseWheel(MouseWheelEvent {
            timestamp: Duration::ZERO,
            window_id: 3,
            which: 0,
            x: 0.0,
            y: -1.0,
            direction: sdl3::events::mouse::MouseWheelDirection::Flipped,
            mouse_x: 0.0,
            mouse_y: 0.0,
            integer_x: 0,
            integer_y: -1,
        }),
        Event::JoyHat(JoyHatEvent {
            timestamp: Duration::ZERO,
            which: 2,
            hat: 1,
            value: sdl3::joystick::HAT_LEFTDOWN,
        }),
        Event::GamepadButton(GamepadButtonEvent {
            timestamp: Duration::ZERO,
            which: 2,
            button: 13,
            down: true,
        }),
        Event::GamepadButton(GamepadButtonEvent {
            timestamp: Duration::ZERO,
            which: 2,
            button: 40,
            down: false,
        }),
        Event::TouchFinger(TouchFingerEvent {
            event_type: EventType::FINGER_CANCELED,
            timestamp: Duration::ZERO,
            touch_id: 5,
            finger_id: 6,
            x: 0.5,
            y: 0.25,
            dx: 0.0,
            dy: 0.0,
            pressure: 1.0,
            window_id: 0,
        }),
        Event::Drop(DropEvent {
            event_type: EventType::DROP_FILE,
            timestamp: Duration::ZERO,
            window_id: 3,
            x: 0.0,
            y: 0.0,
            source: None,
            data: Some("/tmp/a b".to_owned()),
        }),
        Event::User(UserEvent {
            event_type: EventType::USER,
            timestamp: Duration::ZERO,
            window_id: 0,
            code: -42,
            data1: None,
            data2: None,
        }),
        Event::Other(common(EventType(0x8123))),
    ];
    for event in &events {
        print_event(event);
    }
    assert_eq!(
        capture.messages(),
        [
            "SDL EVENT: Quit requested",
            "SDL EVENT: App running low on memory",
            "SDL EVENT: Keymap changed",
            "SDL EVENT: Display 7 changed orientation to PORTRAIT",
            "SDL EVENT: Display 7 desktop mode changed to 800x600",
            "SDL EVENT: Window 3 moved to -10,20",
            "SDL EVENT: Window 3 resized to 640x480",
            "SDL EVENT: Window 3 HDR enabled",
            "SDL EVENT: Mouse entered window 3",
            "Unknown event 0x021b",
            "SDL EVENT: Keyboard: key pressed in window 3: scancode 0x00000004 = A, keycode 0x00000061 = A, mods = LSHIFT | RALT | NUM",
            "SDL EVENT: Keyboard: key released in window 3: scancode 0x00000004 = A, keycode 0x0000001B = Escape, mods = NONE",
            "SDL EVENT: Mouse: moved to 10.5,1.23457e+06 (-0.25,0) in window 3",
            "SDL EVENT: Mouse: wheel scrolled 0 in x and -1 in y (reversed: 1) in window 3",
            "SDL EVENT: Joystick 2: hat 1 moved to LEFTDOWN",
            "SDL EVENT: Gamepad 2button 13 ('DPAD_LEFT') down",
            "SDL EVENT: Gamepad 2 button 40 ('???') up",
            "SDL EVENT: Finger: cancel touch=5, finger=6, x=0.500000, y=0.250000, dx=0.000000, dy=0.000000, pressure=1.000000",
            "SDL EVENT: Drag and drop file in window 3: '/tmp/a b'",
            "SDL EVENT: User event -42",
            "Unknown event 0x8123",
        ]
    );
}

#[test]
fn hit_test() {
    let _l = test_lock();
    let _capture = Capture::new();
    // Without the window, it's 0x0: the top comes before the bottom, the
    // left before the right.
    let at = |x, y| example_hit_test_callback(0, Point { x, y });
    assert_eq!(at(1, 1), HitTestResult::ResizeTopLeft);
    assert_eq!(at(1, -9), HitTestResult::ResizeTopLeft);
    assert_eq!(at(-9, 8), HitTestResult::ResizeBottomLeft);
    assert_eq!(at(8, 0), HitTestResult::ResizeTopRight);
    assert_eq!(at(8, 8), HitTestResult::ResizeBottomRight);
}

fn press(key: Keycode, modifiers: Keymod, window: Window) -> Event {
    self::key(key, modifiers, window.id(), true)
}

/// The pixels of a surface as `#` (not black) and `.` (black).
fn picture(surface: &Surface<'_>) -> Vec<String> {
    (0..surface.height())
        .map(|y| {
            (0..surface.width())
                .map(|x| {
                    if surface.read_pixel(x, y).unwrap() == Color::new(0, 0, 0, 255) {
                        '.'
                    } else {
                        '#'
                    }
                })
                .collect()
        })
        .collect()
}

#[test]
fn dummy_session() {
    let _l = test_lock();
    let capture = Capture::new();
    let mut s = state(
        &[
            "prog",
            "--video",
            "dummy",
            "--geometry",
            "320x300",
            "--info",
            "all",
            "--title",
            "Session",
        ],
        InitFlags::VIDEO,
    );
    assert!(s.default_args());
    s.init().unwrap();
    let messages = capture.messages();
    assert!(
        messages.contains(&"Video driver: dummy".to_owned()),
        "{messages:#?}"
    );
    assert!(messages.contains(&"Current renderer:".to_owned()));
    assert!(messages
        .iter()
        .any(|m| m.starts_with("Built-in video drivers:")));
    assert!(messages
        .iter()
        .any(|m| m.starts_with("Number of displays: ")));
    assert!(messages.iter().any(|m| m.starts_with("  Desktop mode: ")));
    assert!(messages.iter().any(|m| m == "  Renderer software:"));
    assert!(messages
        .iter()
        .any(|m| m.starts_with("    Texture formats: ")));
    assert_eq!(s.windows.len(), 1);
    assert!(s.renderers[0].is_some());
    let window = s.windows[0];
    assert_eq!(window.title().unwrap(), "Session");
    assert_eq!(window.size().unwrap(), (320, 300));
    assert!(!window.flags().unwrap().contains(WindowFlags::HIDDEN));
    assert_eq!((s.logical_w, s.logical_h), (320, 300));

    // The window's hit test regions (not borderless and resizable, so set
    // here).
    let at = |x, y| example_hit_test_callback(window.id(), Point { x, y });
    assert_eq!(at(100, 20), HitTestResult::Draggable);
    assert_eq!(at(100, 2), HitTestResult::ResizeTop);
    assert_eq!(at(100, 295), HitTestResult::ResizeBottom);
    assert_eq!(at(2, 100), HitTestResult::ResizeLeft);
    assert_eq!(at(315, 100), HitTestResult::ResizeRight);
    assert_eq!(at(315, 299), HitTestResult::ResizeBottomRight);
    assert_eq!(at(100, 100), HitTestResult::Normal);

    // The hotkeys.
    capture.clear();
    let mut done = false;
    s.event(&press(Keycode::EQUALS, Keymod::LCTRL, window), &mut done);
    assert_eq!(window.size().unwrap(), (640, 600));
    s.event(&press(Keycode::MINUS, Keymod::RCTRL, window), &mut done);
    assert_eq!(window.size().unwrap(), (320, 300));
    // (without Ctrl, nothing)
    s.event(&press(Keycode::EQUALS, Keymod::NONE, window), &mut done);
    assert_eq!(window.size().unwrap(), (320, 300));
    let (x, y) = window.position().unwrap();
    s.event(&press(Keycode::RIGHT, Keymod::LSHIFT, window), &mut done);
    assert_eq!(window.position().unwrap(), (x + 100, y));
    s.event(&press(Keycode::A, Keymod::LCTRL, window), &mut done);
    assert_eq!(window.aspect_ratio().unwrap(), (1.0, 1.0));
    s.event(&press(Keycode::A, Keymod::LCTRL, window), &mut done);
    assert_eq!(window.aspect_ratio().unwrap(), (0.0, 0.0));
    s.event(&press(Keycode::P, Keymod::LALT, window), &mut done);
    s.event(&press(Keycode::P, Keymod::LCTRL, window), &mut done);
    assert!(!done);
    let messages = capture.messages();
    assert!(
        messages
            .iter()
            .any(|m| m.starts_with("Setting position to (")),
        "{messages:#?}"
    );
    assert!(messages.contains(&"Setting progress state to INDETERMINATE".to_owned()));
    assert!(messages
        .iter()
        .any(|m| m.starts_with("Setting progress value to ")));
    // The events are logged with --info all (but not motion).
    assert!(messages
        .iter()
        .any(|m| m.starts_with("SDL EVENT: Keyboard: key pressed")));

    // Closing hides the window; Escape and quit end.
    s.event(
        &window_event(EventType::WINDOW_CLOSE_REQUESTED, window.id(), 0, 0),
        &mut done,
    );
    assert!(window.flags().unwrap().contains(WindowFlags::HIDDEN));
    assert!(!done);
    assert_eq!(
        s.event_main_callbacks(&press(Keycode::ESCAPE, Keymod::NONE, window)),
        AppResult::Success
    );
    s.event(
        &Event::Quit(CommonEvent {
            event_type: EventType::QUIT,
            timestamp: Duration::ZERO,
        }),
        &mut done,
    );
    assert!(done);

    // The window information, drawn as upstream lays it out: white headers,
    // gray lines, 10 pixels apart (and no line for the logical presentation).
    let renderer = s.renderers[0].as_mut().unwrap();
    renderer.set_draw_color(0, 0, 0, 255);
    renderer.clear().unwrap();
    let used = draw_window_info(renderer, &window);
    let drawn = renderer.read_pixels(None).unwrap();

    let display = window.display().unwrap();
    let mode = |m: DisplayMode| {
        format!(
            "{}x{}@{}x {}Hz, ({})",
            m.w,
            m.h,
            fmt_g(m.pixel_density as f64),
            fmt_g(m.refresh_rate as f64),
            m.format.name()
        )
    };
    let bounds = video::display_bounds(display).unwrap();
    let viewport = renderer.viewport();
    let orientation = |o| {
        let mut text = String::new();
        print_display_orientation(&mut text, o);
        text
    };
    // (Ctrl-A made it square)
    let size = window.size().unwrap();
    assert_eq!(size.0, size.1);
    let output = renderer.output_size().unwrap();
    let current = renderer.current_output_size();
    let mut flags = String::new();
    print_window_flags(&mut flags, window.flags().unwrap());
    let (x, y) = window.position().unwrap();
    let lines: Vec<Option<String>> = vec![
        Some("-- Video --".into()),
        Some("SDL_GetCurrentVideoDriver: dummy".into()),
        Some("-- Renderer --".into()),
        Some("SDL_GetRendererName: software".into()),
        Some(format!(
            "SDL_GetRenderOutputSize: {}x{}",
            output.0, output.1
        )),
        Some(format!(
            "SDL_GetCurrentRenderOutputSize: {}x{}",
            current.0, current.1
        )),
        Some(format!(
            "SDL_GetRenderViewport: {},{}, {}x{}",
            viewport.x, viewport.y, viewport.w, viewport.h
        )),
        Some("SDL_GetRenderScale: 1,1".into()),
        None,
        Some("-- Window --".into()),
        Some(format!("SDL_GetWindowPosition: {x},{y}")),
        Some(format!("SDL_GetWindowSize: {}x{}", size.0, size.1)),
        Some(format!("SDL_GetWindowSafeArea: 0,0 {}x{}", size.0, size.1)),
        Some(format!("SDL_GetWindowFlags: {flags}")),
        Some("-- Display --".into()),
        Some(format!("SDL_GetDisplayForWindow: {display}")),
        Some(format!(
            "SDL_GetDisplayName: {}",
            or_null(video::display_name(display))
        )),
        Some(format!(
            "SDL_GetDisplayBounds: {},{}, {}x{}",
            bounds.x, bounds.y, bounds.w, bounds.h
        )),
        Some(format!(
            "SDL_GetCurrentDisplayMode: {}",
            mode(video::current_display_mode(display).unwrap())
        )),
        Some(format!(
            "SDL_GetDesktopDisplayMode: {}",
            mode(video::desktop_display_mode(display).unwrap())
        )),
        Some(format!(
            "SDL_GetNaturalDisplayOrientation: {}",
            orientation(video::natural_display_orientation(display))
        )),
        Some(format!(
            "SDL_GetCurrentDisplayOrientation: {}",
            orientation(video::current_display_orientation(display))
        )),
        Some("SDL_GetDisplayContentScale: 1".into()),
        Some("-- Mouse --".into()),
        Some("SDL_GetMouseState: 0,0 ".into()),
        Some("SDL_GetGlobalMouseState: 0,0 ".into()),
        Some("-- Keyboard --".into()),
        Some("SDL_GetModState: ".into()),
    ];
    assert_eq!(used, lines.len() as f32 * 10.0);
    renderer.set_draw_color(0, 0, 0, 255);
    renderer.clear().unwrap();
    for (i, line) in lines.iter().enumerate() {
        if let Some(line) = line {
            if line.starts_with("--") {
                renderer.set_draw_color(255, 255, 255, 255);
            } else {
                renderer.set_draw_color(170, 170, 170, 255);
            }
            draw_string(renderer, 0.0, i as f32 * 10.0, line).unwrap();
        }
    }
    let expected = renderer.read_pixels(None).unwrap();
    let (drawn_rows, expected_rows) = (picture(&drawn), picture(&expected));
    for (i, line) in lines.iter().enumerate() {
        let band = |rows: &[String]| rows[i * 10..(i * 10 + 10).min(rows.len())].to_vec();
        assert_eq!(
            band(&drawn_rows),
            band(&expected_rows),
            "line {i}: {line:?}"
        );
    }
    let white = Color::new(255, 255, 255, 255);
    assert!((0..8).any(|y| (0..88).any(|x| drawn.read_pixel(x, y).unwrap() == white)));
    assert!(picture(&drawn)[12].contains('#'));

    s.quit();
    assert!(Window::from_id(window.id()).is_err());
    hints::reset(hints::VIDEO_DRIVER);
}

#[test]
fn printers() {
    let mut text = String::new();
    print_window_flags(
        &mut text,
        WindowFlags::OPENGL | WindowFlags::HIDDEN | WindowFlags(1 << 40),
    );
    assert_eq!(text, "OPENGL | HIDDEN");
    let mut text = String::new();
    print_window_flag(&mut text, WindowFlags(1 << 40));
    assert_eq!(text, "0x0000010000000000");
    let mut text = String::new();
    print_mod_state(&mut text, Keymod::CTRL | Keymod::SCROLL);
    assert_eq!(text, "LCTRL | RCTRL | SCROLL");
    let mut text = String::new();
    print_button_mask(
        &mut text,
        MouseButtonFlags::LMASK | MouseButtonFlags::X2MASK,
    );
    assert_eq!(text, "SDL_BUTTON_MASK(1) | SDL_BUTTON_MASK(5)");
    let mut text = String::new();
    print_pixel_format(&mut text, PixelFormat::ARGB8888);
    assert_eq!(text, "ARGB8888");
    let mut text = String::new();
    print_logical_presentation(&mut text, LogicalPresentation::IntegerScale);
    assert_eq!(text, "INTEGER_SCALE");
    let mut text = String::new();
    print_display_orientation(&mut text, DisplayOrientation::LandscapeFlipped);
    assert_eq!(text, "LANDSCAPE_FLIPPED");
    assert_eq!(gamepad_axis_name(5), "RIGHT_TRIGGER");
    assert_eq!(gamepad_axis_name(-1), "INVALID");
    assert_eq!(gamepad_axis_name(6), "???");
    assert_eq!(gamepad_button_name(0), "SOUTH");
    assert_eq!(gamepad_button_name(16), "???");
    assert_eq!(cap_sense_name(3), "RIGHT_GRIP");
    assert_eq!(display_orientation_name(9), "???");
}

/// A suite that checks it gets its options through the common state.
fn passes(_: &mut TestData) -> TestStatus {
    crate::assert_pass!("Passed");
    TestStatus::Completed
}

#[test]
fn harness_through_common_state() {
    static PASSES: TestCase = TestCase {
        test_case: passes,
        name: "passes",
        description: "",
        enabled: true,
    };
    static SUITE: TestSuite = TestSuite {
        name: "Suite",
        test_set_up: None,
        test_cases: &[&PASSES],
        test_tear_down: None,
    };
    let _l = test_lock();
    let capture = Capture::new();
    let mut s = state(
        &[
            "prog",
            "--filter",
            "passes",
            "--execKey",
            "77",
            "--random-order",
        ],
        InitFlags::NONE,
    );
    let suites = [&SUITE];
    let mut runner = TestSuiteRunner::new(&mut s, &suites);
    assert!(s.default_args());
    assert_eq!(runner.execute(), 0);
    let messages = capture.messages();
    assert!(
        messages.contains(&" : Test Iteration 1: execKey 77".to_owned()),
        "{messages:#?}"
    );
    assert!(
        messages.contains(&" : Filtering: running only test 'passes' in suite 'Suite'".to_owned())
    );
    // (the filter turned the random order off)
    assert!(!runner.user().random_order);
}
