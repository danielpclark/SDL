// Tests of the Emscripten video driver under node (in a browser:
// sdl3/tests/emscripten_video.rs).

use crate::hints;
use crate::init::{self, InitFlags};
use crate::video::messagebox::{show_simple_message_box, MessageBoxFlags};
use crate::video::{current_video_driver, num_video_drivers, video_driver};

/// Under node there's no DOM: the driver isn't available, so video
/// initialization goes on to the next driver, as for any unavailable
/// driver (Note (upstream): upstream's device is created, and its first
/// JavaScript throws).
#[test]
fn unavailable_without_dom() {
    let _l = crate::test_support::test_lock();
    init::quit();

    // In the list, before offscreen and dummy, as upstream's bootstrap.
    let names: Vec<_> = (0..num_video_drivers())
        .map(|i| video_driver(i).unwrap())
        .collect();
    let at = |name| names.iter().position(|&n| n == name).unwrap();
    assert!(at("emscripten") < at("offscreen"), "{names:?}");
    assert!(at("offscreen") < at("dummy"), "{names:?}");

    assert!(!super::has_dom());
    assert!((super::EMSCRIPTEN_BOOTSTRAP.create)().is_none());

    // Asked for by name: not available.
    hints::set(hints::VIDEO_DRIVER, "emscripten").unwrap();
    let e = init::init(InitFlags::VIDEO).unwrap_err();
    assert_eq!(e.message(), "emscripten not available");

    // The first that works, after it.
    hints::set(hints::VIDEO_DRIVER, "emscripten,dummy").unwrap();
    init::init(InitFlags::VIDEO).unwrap();
    assert_eq!(current_video_driver().unwrap(), "dummy");
    init::quit();

    // Without the hint none is available under node (the dummy and
    // offscreen drivers want to be asked for).
    hints::reset(hints::VIDEO_DRIVER);
    let e = init::init(InitFlags::VIDEO).unwrap_err();
    assert_eq!(e.message(), "No available video device");

    // A message box without video tries the Emscripten driver's, which
    // fails without a DOM (Note (upstream): upstream's alert() throws).
    let e = show_simple_message_box(MessageBoxFlags::ERROR, "title", "message", None).unwrap_err();
    assert_eq!(
        e.message(),
        "Emscripten message boxes need a page (no DOM: not in a browser)"
    );
    init::quit();
}

// -- SDL_emscriptenevents.c's pure parts -------------------------------------

mod events {
    use std::ffi::{c_char, CString};
    use std::mem::{offset_of, size_of};

    use sdl3_emscripten::sys::{self, EmscriptenMouseEvent, EmscriptenWheelEvent};

    use super::super::events::{
        display_orientation, key_event_keycode, key_event_scancode, keyboard_target_element,
        lock_keys_mod_state, map_scan_code, mouse_button, wheel_steps, DropEvent, PointerEvent,
    };
    use crate::events::keyboard::{Keycode, Keymod, Scancode};
    use crate::events::mouse::{BUTTON_LEFT, BUTTON_MIDDLE, BUTTON_RIGHT, BUTTON_X1, BUTTON_X2};
    use crate::video::sysvideo::DisplayOrientation;

    /// Every DOM `code` of upstream's table (its comments) and of its
    /// switch (the `DOM_PK_*` names' codes, from `dom_pk_codes.h`), with the
    /// scancode upstream gives it.
    const DOM_CODES: &[(&str, Scancode)] = &[
        ("Unidentified", Scancode::UNKNOWN),
        ("Escape", Scancode::ESCAPE),
        ("Digit0", Scancode::N0),
        ("Digit1", Scancode::N1),
        ("Digit2", Scancode::N2),
        ("Digit3", Scancode::N3),
        ("Digit4", Scancode::N4),
        ("Digit5", Scancode::N5),
        ("Digit6", Scancode::N6),
        ("Digit7", Scancode::N7),
        ("Digit8", Scancode::N8),
        ("Digit9", Scancode::N9),
        ("Minus", Scancode::MINUS),
        ("Equal", Scancode::EQUALS),
        ("Backspace", Scancode::BACKSPACE),
        ("Tab", Scancode::TAB),
        ("KeyQ", Scancode::Q),
        ("KeyW", Scancode::W),
        ("KeyE", Scancode::E),
        ("KeyR", Scancode::R),
        ("KeyT", Scancode::T),
        ("KeyY", Scancode::Y),
        ("KeyU", Scancode::U),
        ("KeyI", Scancode::I),
        ("KeyO", Scancode::O),
        ("KeyP", Scancode::P),
        ("BracketLeft", Scancode::LEFTBRACKET),
        ("BracketRight", Scancode::RIGHTBRACKET),
        ("Enter", Scancode::RETURN),
        ("ControlLeft", Scancode::LCTRL),
        ("KeyA", Scancode::A),
        ("KeyS", Scancode::S),
        ("KeyD", Scancode::D),
        ("KeyF", Scancode::F),
        ("KeyG", Scancode::G),
        ("KeyH", Scancode::H),
        ("KeyJ", Scancode::J),
        ("KeyK", Scancode::K),
        ("KeyL", Scancode::L),
        ("Semicolon", Scancode::SEMICOLON),
        ("Quote", Scancode::APOSTROPHE),
        ("Backquote", Scancode::GRAVE),
        ("ShiftLeft", Scancode::LSHIFT),
        ("Backslash", Scancode::BACKSLASH),
        ("KeyZ", Scancode::Z),
        ("KeyX", Scancode::X),
        ("KeyC", Scancode::C),
        ("KeyV", Scancode::V),
        ("KeyB", Scancode::B),
        ("KeyN", Scancode::N),
        ("KeyM", Scancode::M),
        ("Comma", Scancode::COMMA),
        ("Period", Scancode::PERIOD),
        ("Slash", Scancode::SLASH),
        ("ShiftRight", Scancode::RSHIFT),
        ("NumpadMultiply", Scancode::KP_MULTIPLY),
        ("AltLeft", Scancode::LALT),
        ("Space", Scancode::SPACE),
        ("CapsLock", Scancode::CAPSLOCK),
        ("F1", Scancode::F1),
        ("F2", Scancode::F2),
        ("F3", Scancode::F3),
        ("F4", Scancode::F4),
        ("F5", Scancode::F5),
        ("F6", Scancode::F6),
        ("F7", Scancode::F7),
        ("F8", Scancode::F8),
        ("F9", Scancode::F9),
        ("F10", Scancode::F10),
        ("Pause", Scancode::PAUSE),
        ("ScrollLock", Scancode::SCROLLLOCK),
        ("Numpad7", Scancode::KP_7),
        ("Numpad8", Scancode::KP_8),
        ("Numpad9", Scancode::KP_9),
        ("NumpadSubtract", Scancode::KP_MINUS),
        ("Numpad4", Scancode::KP_4),
        ("Numpad5", Scancode::KP_5),
        ("Numpad6", Scancode::KP_6),
        ("NumpadAdd", Scancode::KP_PLUS),
        ("Numpad1", Scancode::KP_1),
        ("Numpad2", Scancode::KP_2),
        ("Numpad3", Scancode::KP_3),
        ("Numpad0", Scancode::KP_0),
        ("NumpadDecimal", Scancode::KP_PERIOD),
        ("PrintScreen", Scancode::PRINTSCREEN),
        ("IntlBackslash", Scancode::NONUSBACKSLASH),
        ("F11", Scancode::F11),
        ("F12", Scancode::F12),
        ("NumpadEqual", Scancode::KP_EQUALS),
        ("F13", Scancode::F13),
        ("F14", Scancode::F14),
        ("F15", Scancode::F15),
        ("F16", Scancode::F16),
        ("F17", Scancode::F17),
        ("F18", Scancode::F18),
        ("F19", Scancode::F19),
        ("F20", Scancode::F20),
        ("F21", Scancode::F21),
        ("F22", Scancode::F22),
        ("F23", Scancode::F23),
        ("KanaMode", Scancode::INTERNATIONAL2),
        ("Lang2", Scancode::LANG2),
        ("Lang1", Scancode::LANG1),
        ("IntlRo", Scancode::INTERNATIONAL1),
        ("F24", Scancode::F24),
        ("Convert", Scancode::INTERNATIONAL4),
        ("NonConvert", Scancode::INTERNATIONAL5),
        ("IntlYen", Scancode::INTERNATIONAL3),
        ("NumpadComma", Scancode::KP_COMMA),
        ("Paste", Scancode::PASTE),
        ("MediaTrackPrevious", Scancode::MEDIA_PREVIOUS_TRACK),
        ("Cut", Scancode::CUT),
        ("Copy", Scancode::COPY),
        ("MediaTrackNext", Scancode::MEDIA_NEXT_TRACK),
        ("NumpadEnter", Scancode::KP_ENTER),
        ("ControlRight", Scancode::RCTRL),
        ("AudioVolumeMute", Scancode::MUTE),
        ("MediaPlayPause", Scancode::MEDIA_PLAY_PAUSE),
        ("MediaStop", Scancode::MEDIA_STOP),
        ("Eject", Scancode::MEDIA_EJECT),
        ("AudioVolumeDown", Scancode::VOLUMEDOWN),
        ("AudioVolumeUp", Scancode::VOLUMEUP),
        ("BrowserHome", Scancode::AC_HOME),
        ("NumpadDivide", Scancode::KP_DIVIDE),
        ("AltRight", Scancode::RALT),
        ("Help", Scancode::HELP),
        ("NumLock", Scancode::NUMLOCKCLEAR),
        ("Home", Scancode::HOME),
        ("ArrowUp", Scancode::UP),
        ("PageUp", Scancode::PAGEUP),
        ("ArrowLeft", Scancode::LEFT),
        ("ArrowRight", Scancode::RIGHT),
        ("End", Scancode::END),
        ("ArrowDown", Scancode::DOWN),
        ("PageDown", Scancode::PAGEDOWN),
        ("Insert", Scancode::INSERT),
        ("Delete", Scancode::DELETE),
        ("MetaLeft", Scancode::LGUI),
        ("MetaRight", Scancode::RGUI),
        ("ContextMenu", Scancode::APPLICATION),
        ("Power", Scancode::POWER),
        ("BrowserSearch", Scancode::AC_SEARCH),
        ("BrowserFavorites", Scancode::AC_BOOKMARKS),
        ("BrowserRefresh", Scancode::AC_REFRESH),
        ("BrowserStop", Scancode::AC_STOP),
        ("BrowserForward", Scancode::AC_FORWARD),
        ("BrowserBack", Scancode::AC_BACK),
        ("MediaSelect", Scancode::MEDIA_SELECT),
    ];

    #[test]
    fn dom_code_scancodes() {
        assert_eq!(DOM_CODES.len(), 109 + 39);
        for &(code, scancode) in DOM_CODES {
            let c = CString::new(code).unwrap();
            assert_eq!(map_scan_code(&c), scancode, "{code}");
        }
        // emscripten's other names for the same keys
        for (code, scancode) in [
            ("VolumeMute", Scancode::MUTE),
            ("VolumeDown", Scancode::VOLUMEDOWN),
            ("VolumeUp", Scancode::VOLUMEUP),
            ("OSLeft", Scancode::LGUI),
            ("OSRight", Scancode::RGUI),
            ("LaunchMediaPlayer", Scancode::MEDIA_SELECT),
        ] {
            let c = CString::new(code).unwrap();
            assert_eq!(map_scan_code(&c), scancode, "{code}");
        }
        // keys emscripten knows and upstream doesn't map, and unknown codes
        for code in [
            "LaunchApp1",
            "LaunchApp2",
            "LaunchMail",
            "Unidentified",
            "",
            "Fn",
            "keya",
            "KeyA ",
        ] {
            let c = CString::new(code).unwrap();
            assert_eq!(map_scan_code(&c), Scancode::UNKNOWN, "{code:?}");
        }
    }

    #[test]
    fn key_scancodes() {
        // the code first
        assert_eq!(key_event_scancode(c"KeyA", b"q"), Scancode::A);
        assert_eq!(key_event_scancode(c"Escape", b"Sleep"), Scancode::ESCAPE);
        // then the key, for the keys without a code
        for (key, scancode) in [
            (&b"Sleep"[..], Scancode::SLEEP),
            (b"ChannelUp", Scancode::CHANNEL_INCREMENT),
            (b"ChannelDown", Scancode::CHANNEL_DECREMENT),
            (b"MediaPlay", Scancode::MEDIA_PLAY),
            (b"MediaPause", Scancode::MEDIA_PAUSE),
            (b"MediaRecord", Scancode::MEDIA_RECORD),
            (b"MediaFastForward", Scancode::MEDIA_FAST_FORWARD),
            (b"MediaRewind", Scancode::MEDIA_REWIND),
            (b"Close", Scancode::AC_CLOSE),
            (b"New", Scancode::AC_NEW),
            (b"Open", Scancode::AC_OPEN),
            (b"Print", Scancode::AC_PRINT),
            (b"Save", Scancode::AC_SAVE),
            (b"Props", Scancode::AC_PROPERTIES),
            // KaiOS's soft keys
            (b"SoftLeft", Scancode::AC_FORWARD),
            (b"SoftRight", Scancode::AC_BACK),
            (b"sleep", Scancode::UNKNOWN),
            (b"", Scancode::UNKNOWN),
        ] {
            assert_eq!(key_event_scancode(c"", key), scancode, "{key:?}");
            assert_eq!(
                key_event_scancode(c"Unidentified", key),
                scancode,
                "{key:?}"
            );
        }
    }

    #[test]
    fn key_keycodes() {
        // one character, at the standard location: the character
        assert_eq!(key_event_keycode(0, b"a"), Keycode(u32::from('a')));
        assert_eq!(key_event_keycode(0, b"A"), Keycode(u32::from('A')));
        assert_eq!(key_event_keycode(0, "é".as_bytes()), Keycode(0xE9));
        assert_eq!(key_event_keycode(0, "€".as_bytes()), Keycode(0x20AC));
        assert_eq!(key_event_keycode(0, "😀".as_bytes()), Keycode(0x1F600));
        assert_eq!(key_event_keycode(0, b" "), Keycode(u32::from(' ')));
        // elsewhere (left, right, the keypad): none
        for location in [1, 2, 3] {
            assert_eq!(key_event_keycode(location, b"1"), Keycode::UNKNOWN);
        }
        // a key name, nothing, or a byte that isn't UTF-8 (one code point to
        // SDL_utf8strlen(), U+FFFD to SDL_StepUTF8()): none
        assert_eq!(key_event_keycode(0, b"Enter"), Keycode::UNKNOWN);
        assert_eq!(key_event_keycode(0, b""), Keycode::UNKNOWN);
        assert_eq!(key_event_keycode(0, b"\xff"), Keycode::UNKNOWN);
    }

    #[test]
    fn lock_keys() {
        let locks = Keymod::CAPS | Keymod::NUM | Keymod::SCROLL;
        // as they are: no change
        assert_eq!(lock_keys_mod_state(Keymod::NONE, false, false, false), None);
        assert_eq!(
            lock_keys_mod_state(locks | Keymod::LSHIFT, true, true, true),
            None
        );
        // the browser's lock state, the other modifiers kept
        assert_eq!(
            lock_keys_mod_state(Keymod::LSHIFT, true, false, false),
            Some(Keymod::LSHIFT | Keymod::CAPS)
        );
        assert_eq!(
            lock_keys_mod_state(Keymod::CAPS | Keymod::RCTRL, false, true, true),
            Some(Keymod::RCTRL | Keymod::NUM | Keymod::SCROLL)
        );
        assert_eq!(
            lock_keys_mod_state(locks | Keymod::MODE, false, false, false),
            Some(Keymod::MODE)
        );
    }

    #[test]
    fn orientations() {
        for (index, orientation) in [
            (
                sys::EMSCRIPTEN_ORIENTATION_LANDSCAPE_PRIMARY,
                DisplayOrientation::Landscape,
            ),
            (
                sys::EMSCRIPTEN_ORIENTATION_LANDSCAPE_SECONDARY,
                DisplayOrientation::LandscapeFlipped,
            ),
            (
                sys::EMSCRIPTEN_ORIENTATION_PORTRAIT_PRIMARY,
                DisplayOrientation::Portrait,
            ),
            (
                sys::EMSCRIPTEN_ORIENTATION_PORTRAIT_SECONDARY,
                DisplayOrientation::PortraitFlipped,
            ),
            (
                sys::EMSCRIPTEN_ORIENTATION_UNSUPPORTED,
                DisplayOrientation::Unknown,
            ),
            (-1, DisplayOrientation::Unknown),
            (3, DisplayOrientation::Unknown),
        ] {
            assert_eq!(display_orientation(index), orientation, "{index}");
        }
    }

    #[test]
    fn keyboard_targets() {
        assert_eq!(keyboard_target_element(c"#none"), None);
        assert_eq!(
            keyboard_target_element(c"#window"),
            Some(sys::EMSCRIPTEN_EVENT_TARGET_WINDOW)
        );
        assert_eq!(
            keyboard_target_element(c"#document"),
            Some(sys::EMSCRIPTEN_EVENT_TARGET_DOCUMENT)
        );
        assert_eq!(
            keyboard_target_element(c"#screen"),
            Some(sys::EMSCRIPTEN_EVENT_TARGET_SCREEN)
        );
        // anything else is a selector
        for selector in [c"#canvas", c"#None", c"body", c""] {
            assert_eq!(
                keyboard_target_element(selector),
                Some(selector.as_ptr() as *const c_char)
            );
        }
    }

    fn pointer(button: i32, buttons: i32, down: i32) -> PointerEvent {
        PointerEvent {
            pointer_type: 1,
            button,
            buttons,
            down,
            ..PointerEvent::default()
        }
    }

    #[test]
    fn mouse_buttons() {
        // (the DOM's button, the bit of its buttons, SDL's)
        for (button, bit, sdl) in [
            (0, 1, BUTTON_LEFT),
            (1, 4, BUTTON_MIDDLE),
            (2, 2, BUTTON_RIGHT),
            (3, 8, BUTTON_X1),
            (4, 16, BUTTON_X2),
        ] {
            assert_eq!(mouse_button(&pointer(button, 0, 0)), (sdl, false));
            assert_eq!(mouse_button(&pointer(button, bit, 0)), (sdl, true));
            assert_eq!(mouse_button(&pointer(button, 0, 1)), (sdl, true));
            // the other buttons' bits don't count
            assert_eq!(mouse_button(&pointer(button, 31 & !bit, 0)), (sdl, false));
        }
        // no button (-1: a move), or one SDL hasn't
        for button in [-1, 5, 6] {
            assert_eq!(mouse_button(&pointer(button, 31, 1)), (0, false));
        }
    }

    fn wheel(dx: f64, dy: f64, mode: u32) -> EmscriptenWheelEvent {
        EmscriptenWheelEvent {
            mouse: EmscriptenMouseEvent {
                timestamp: 0.0,
                screenX: 0,
                screenY: 0,
                clientX: 0,
                clientY: 0,
                ctrlKey: false,
                shiftKey: false,
                altKey: false,
                metaKey: false,
                button: 0,
                buttons: 0,
                movementX: 0,
                movementY: 0,
                targetX: 0,
                targetY: 0,
                canvasX: 0,
                canvasY: 0,
                padding: 0,
            },
            deltaX: dx,
            deltaY: dy,
            deltaZ: 0.0,
            deltaMode: mode,
        }
    }

    #[test]
    fn wheel_deltas() {
        // 100 pixels, 3 lines, a 80th of a page make a step
        assert_eq!(
            wheel_steps(&wheel(-50.0, 100.0, sys::DOM_DELTA_PIXEL)),
            (-0.5, 1.0)
        );
        assert_eq!(
            wheel_steps(&wheel(1.0, -3.0, sys::DOM_DELTA_LINE)),
            (1.0 / 3.0, -1.0)
        );
        assert_eq!(
            wheel_steps(&wheel(0.5, 1.0, sys::DOM_DELTA_PAGE)),
            (40.0, 80.0)
        );
        // (an unknown mode: as they are)
        assert_eq!(wheel_steps(&wheel(2.0, 7.0, 9)), (2.0, 7.0));
        // float arithmetic, as C's
        assert_eq!(
            wheel_steps(&wheel(0.1, 53.0, sys::DOM_DELTA_PIXEL)),
            (0.1f32 / 100.0, 53.0f32 / 100.0)
        );
    }

    #[test]
    fn event_layouts() {
        // as the JavaScript writes them: 32-bit words
        assert_eq!(size_of::<PointerEvent>(), 56);
        assert_eq!(offset_of!(PointerEvent, pointer_type), 0);
        assert_eq!(offset_of!(PointerEvent, pointerid), 4);
        assert_eq!(offset_of!(PointerEvent, button), 8);
        assert_eq!(offset_of!(PointerEvent, buttons), 12);
        assert_eq!(offset_of!(PointerEvent, down), 16);
        assert_eq!(offset_of!(PointerEvent, movement_x), 20);
        assert_eq!(offset_of!(PointerEvent, movement_y), 24);
        assert_eq!(offset_of!(PointerEvent, target_x), 28);
        assert_eq!(offset_of!(PointerEvent, target_y), 32);
        assert_eq!(offset_of!(PointerEvent, pressure), 36);
        assert_eq!(offset_of!(PointerEvent, tangential_pressure), 40);
        assert_eq!(offset_of!(PointerEvent, tiltx), 44);
        assert_eq!(offset_of!(PointerEvent, tilty), 48);
        assert_eq!(offset_of!(PointerEvent, rotation), 52);
        assert_eq!(size_of::<DropEvent>(), 8);
        assert_eq!(offset_of!(DropEvent, x), 0);
        assert_eq!(offset_of!(DropEvent, y), 4);
    }
}
