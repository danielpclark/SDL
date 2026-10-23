// Tests of the Emscripten joystick driver under node (in a browser:
// sdl3/tests/emscripten_joystick.rs).
//
// Node has no Gamepad API, so the driver is unavailable there. The pure
// parts (the layout of the buttons, axes and hat, the hat from an axis, the
// bus, IDs and GUID) are checked by themselves; the driver's JavaScript (the
// names and IDs it finds in the browsers' `id` formats) and its callbacks
// with a stand-in `navigator.getGamepads()`, which emscripten's
// `emscripten_sample_gamepad_data()` and `emscripten_get_gamepad_status()`
// read too. (Without a window emscripten can't register the event handlers,
// so the tests call the driver's callbacks as its handlers would.)

use std::ffi::CString;

use sdl3_emscripten::{em_asm, em_asm_int};

use super::*;
use crate::events::queue;
use crate::events::{Event, EventType};
use crate::init::{self, InitFlags};
use crate::joystick::gamepad::{self, Gamepad, GamepadAxis, GamepadButton};
use crate::joystick::{joysticks, Joystick, HAT_LEFTDOWN, HAT_RIGHTUP};

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

/// Whether this JavaScript has a Gamepad API of its own (a browser).
fn has_gamepad_api() -> bool {
    js_str("typeof navigator === 'object' && typeof navigator.getGamepads === 'function'") == "true"
}

/// A stand-in Gamepad API: `navigator.getGamepads()` answers
/// `Module.sdl3_test_pads` (`P`, the gamepads by index, `null` where there's
/// none), and `navigator.userAgent` is `P.ua`. `P.pad(index, id, mapping,
/// buttons, axes)` makes a gamepad (all released and centered, timestamp 1),
/// `P.rumbles` holds the effects played. Taken away when dropped.
struct Pads;

impl Pads {
    fn new(ua: &str) -> Option<Pads> {
        if has_gamepad_api() {
            // (a browser: these tests are for node)
            return None;
        }
        js_run(&format!(
            r#"
            var P = Module['sdl3_test_pads'] = [];
            P.ua = '{ua}';
            P.rumbles = [];
            P.pad = function(index, id, mapping, buttons, axes) {{
                var b = [];
                for (var i = 0; i < buttons; i++) b.push({{ pressed: false, touched: false, value: 0 }});
                return {{ id: id, index: index, mapping: mapping, connected: true, timestamp: 1, axes: new Array(axes).fill(0), buttons: b }};
            }};
            P.actuator = function(effects) {{
                return {{ effects: effects, playEffect: function(type, params) {{ P.rumbles.push(type + ' ' + JSON.stringify(params)); return Promise.resolve('complete'); }} }};
            }};
            navigator.getGamepads = function() {{ return P.slice(); }};
            Object.defineProperty(navigator, 'userAgent', {{ get: function() {{ return P.ua; }}, configurable: true }});
        "#
        ));
        Some(Pads)
    }

    /// Put gamepad `index` (a `P.pad(…)` expression, or `null`) in the list.
    fn set(&self, index: usize, pad: &str) {
        js_run(&format!(
            "var P = Module['sdl3_test_pads']; P[{index}] = {pad};"
        ));
    }

    /// Run statements with `P` the list and `g` gamepad `index`; the
    /// gamepad's timestamp goes up.
    fn change(&self, index: usize, code: &str) {
        js_run(&format!(
            "var P = Module['sdl3_test_pads']; var g = P[{index}]; {code}; g.timestamp++;"
        ));
    }

    /// The `gamepadconnected` (`connected`) or `gamepaddisconnected` event
    /// of gamepad `index`, as emscripten's handler gives it to the driver's
    /// callback: the gamepad filled in by emscripten (here through
    /// `emscripten_get_gamepad_status()`, which reads the same fields).
    fn event(&self, index: usize, connected: bool) {
        let mut state = zeroed_gamepad_state();
        // SAFETY: plain calls; emscripten fills in the structure.
        unsafe {
            assert_eq!(
                sys::emscripten_sample_gamepad_data(),
                sys::EMSCRIPTEN_RESULT_SUCCESS
            );
            assert_eq!(
                sys::emscripten_get_gamepad_status(index as i32, &mut state),
                sys::EMSCRIPTEN_RESULT_SUCCESS
            );
        }
        if !connected {
            state.connected = false;
        }
        let callback = if connected {
            joystick_connected
        } else {
            joystick_disconnected
        };
        // SAFETY: the callback's own parameters.
        let handled = unsafe {
            callback(
                if connected {
                    sys::EMSCRIPTEN_EVENT_GAMEPADCONNECTED
                } else {
                    sys::EMSCRIPTEN_EVENT_GAMEPADDISCONNECTED
                },
                &state,
                std::ptr::null_mut(),
            )
        };
        assert!(handled);
    }
}

impl Drop for Pads {
    fn drop(&mut self) {
        js_run(
            "delete Module['sdl3_test_pads']; delete navigator.getGamepads; delete navigator.userAgent;",
        );
    }
}

/// The joystick IDs seen, in order: `#1` is the first.
static SEEN: std::sync::Mutex<Vec<JoystickID>> = std::sync::Mutex::new(Vec::new());

fn tag(id: JoystickID) -> usize {
    let mut seen = SEEN.lock().unwrap();
    if let Some(i) = seen.iter().position(|&s| s == id) {
        return i + 1;
    }
    seen.push(id);
    seen.len()
}

/// The trace being written (as `sdl3/tests/emscripten_joystick.rs` writes
/// it, in the same format).
static TRACE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

macro_rules! out {
    ($($arg:tt)*) => {{
        use std::fmt::Write as _;
        writeln!(TRACE.lock().unwrap(), $($arg)*).unwrap()
    }};
}

/// The events queued, into the trace (taken from the queue, which isn't
/// pumped: no update but those the test asks for).
fn events() {
    for e in queue::get_events(EventType::FIRST, EventType::LAST, usize::MAX).unwrap() {
        match e {
            Event::JoyDevice(e) => {
                let kind = match e.event_type {
                    EventType::JOYSTICK_ADDED => "added",
                    EventType::JOYSTICK_REMOVED => "removed",
                    EventType::JOYSTICK_UPDATE_COMPLETE => "update complete",
                    _ => "?",
                };
                out!("event joy {kind} #{}", tag(e.which));
            }
            Event::JoyAxis(e) => out!("event joy axis #{} {} {}", tag(e.which), e.axis, e.value),
            Event::JoyHat(e) => out!("event joy hat #{} {} {}", tag(e.which), e.hat, e.value),
            Event::JoyButton(e) => out!(
                "event joy button #{} {} {}",
                tag(e.which),
                e.button,
                e.down as i32
            ),
            Event::GamepadDevice(e) => {
                let kind = match e.event_type {
                    EventType::GAMEPAD_ADDED => "added",
                    EventType::GAMEPAD_REMOVED => "removed",
                    EventType::GAMEPAD_REMAPPED => "remapped",
                    EventType::GAMEPAD_UPDATE_COMPLETE => "update complete",
                    _ => "?",
                };
                out!("event gamepad {kind} #{}", tag(e.which));
            }
            Event::GamepadAxis(e) => out!(
                "event gamepad axis #{} {} {}",
                tag(e.which),
                e.axis,
                e.value
            ),
            Event::GamepadButton(e) => out!(
                "event gamepad button #{} {} {}",
                tag(e.which),
                e.button,
                e.down as i32
            ),
            e => out!("event {:#x}", e.event_type().0),
        }
    }
}

fn info(id: JoystickID, with_mapping: bool) {
    use crate::joystick::*;
    let name = joystick_name_for_id(id).ok().flatten();
    out!(
        "#{} name {} guid {}",
        tag(id),
        name.as_deref().unwrap_or("(null)"),
        joystick_guid_for_id(id)
    );
    let path = match joystick_path_for_id(id) {
        Ok(p) => p,
        Err(e) => e.message().to_owned(),
    };
    out!(
        "#{} ids {:04x} {:04x} {:04x} type {} path {path} player {}",
        tag(id),
        joystick_vendor_for_id(id),
        joystick_product_for_id(id),
        joystick_product_version_for_id(id),
        joystick_type_for_id(id) as i32,
        joystick_player_index_for_id(id)
    );
    let gname = gamepad::gamepad_name_for_id(id).ok().flatten();
    out!(
        "#{} gamepad {} {} type {}",
        tag(id),
        gamepad::is_gamepad(id) as i32,
        gname.as_deref().unwrap_or("(null)"),
        gamepad::gamepad_type_for_id(id)
            .as_str()
            .unwrap_or("(null)")
    );
    if with_mapping {
        out!(
            "#{} mapping {}",
            tag(id),
            gamepad::gamepad_mapping_for_id(id)
                .as_deref()
                .unwrap_or("(null)")
        );
    }
}

fn opened(j: &Joystick) {
    let props = j.properties().unwrap();
    let cap = |name: &str| props.get_bool(name).unwrap_or(false) as i32;
    out!(
        "open #{} axes {} buttons {} hats {} balls {} rumble {} trigger rumble {} led {} serial {}",
        tag(j.id()),
        j.num_axes().unwrap(),
        j.num_buttons().unwrap(),
        j.num_hats().unwrap(),
        j.num_balls().unwrap(),
        cap(PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN),
        cap(PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN),
        cap(crate::joystick::PROP_JOYSTICK_CAP_RGB_LED_BOOLEAN),
        j.serial().ok().flatten().as_deref().unwrap_or("(null)")
    );
}

fn jstate(j: &Joystick) {
    let axes: String = (0..j.num_axes().unwrap_or(0))
        .map(|i| format!(" {}", j.axis(i).unwrap_or(0)))
        .collect();
    let buttons: String = (0..j.num_buttons().unwrap_or(0))
        .map(|i| {
            if j.button(i).unwrap_or(false) {
                '1'
            } else {
                '0'
            }
        })
        .collect();
    let hats: String = (0..j.num_hats().unwrap_or(0))
        .map(|i| format!(" {}", j.hat(i).unwrap_or(0)))
        .collect();
    out!(
        "#{} connected {} axes{axes} buttons {buttons} hats{hats}",
        tag(j.id()),
        j.connected() as i32
    );
}

fn gstate(g: &Gamepad) {
    let axes: String = GamepadAxis::all()
        .map(|a| format!(" {}", g.axis(a)))
        .collect();
    let buttons: String = GamepadButton::all()
        .map(|b| if g.button(b) { '1' } else { '0' })
        .collect();
    out!("#{} gamepad axes{axes} buttons {buttons}", tag(g.id()));
}

fn rumbles() {
    out!(
        "rumbles {}",
        js_str("Module['sdl3_test_pads'].rumbles.splice(0).join(' | ') || '(none)'")
    );
}

/// A result as upstream's trace prints it (with the error of a failure).
fn result(r: Result<()>) -> String {
    match r {
        Ok(()) => "1".to_owned(),
        Err(e) => format!("0 {}", e.message()),
    }
}

// -- the pure parts -----------------------------------------------------------

#[test]
fn hat_from_axis() {
    // The eight positions, up first and clockwise, at steps of 2/7
    let positions = [
        HAT_UP,
        HAT_RIGHTUP,
        HAT_RIGHT,
        HAT_DOWN | HAT_RIGHT,
        HAT_DOWN,
        HAT_LEFTDOWN,
        HAT_LEFT,
        HAT_UP | HAT_LEFT,
    ];
    for (k, &hat) in positions.iter().enumerate() {
        let v = (2 * k as i32 - 7) as f32 / 7.0;
        assert_eq!(hat_from_emscripten_axis(v), hat, "{v}");
        // (within 0.00001)
        assert_eq!(hat_from_emscripten_axis(v + 0.000_005), hat, "{v}");
    }
    // Anything else (the browsers' 3.2857 when released) is centered
    for v in [0.0, 3.2857, 0.5, -0.9, 1.1, -1.0001, f32::NAN] {
        assert_eq!(hat_from_emscripten_axis(v), HAT_CENTERED, "{v}");
    }
}

#[test]
fn layout() {
    // A standard gamepad as Chrome has it: 17 buttons (the four d-pad ones
    // become a hat, the two trigger ones axes), 4 axes and the triggers
    let chrome = Layout::new(true, 17, 4, 2);
    assert_eq!(
        chrome,
        Layout {
            first_hat_button: 12,
            first_trigger_button: 6,
            triggers_are_buttons: true,
            hat_is_axis: false,
            nhats: 1,
            naxes: 6,
            nbuttons: 11,
        }
    );
    // SDL's buttons 0-5 are the browser's, 6-9 its 8-11 (back, start, the
    // sticks), 10 its 16 (the guide), then past its buttons
    let buttons: Vec<_> = chrome.buttons(17).collect();
    assert_eq!(
        buttons.iter().map(|&(_, b)| b).collect::<Vec<_>>(),
        [0, 1, 2, 3, 4, 5, 8, 9, 10, 11, 16, 17, 18, 19, 20, 21, 22]
    );
    assert_eq!(
        buttons.iter().map(|&(i, _)| i).collect::<Vec<_>>(),
        (0..17).collect::<Vec<_>>()
    );

    // Firefox gives the triggers as axes: six of them, which stay
    let firefox = Layout::new(true, 17, 6, 3);
    assert!(!firefox.triggers_are_buttons);
    assert_eq!((firefox.naxes, firefox.nbuttons, firefox.nhats), (6, 11, 1));

    // Without the d-pad buttons (fewer than 16) a standard gamepad is laid
    // out as the browser's
    let short = Layout::new(true, 15, 4, 2);
    assert_eq!(
        (short.first_hat_button, short.first_trigger_button),
        (-1, -1)
    );
    assert_eq!((short.naxes, short.nbuttons, short.nhats), (4, 15, 0));
    assert_eq!(
        short.buttons(15).map(|(_, b)| b).collect::<Vec<_>>(),
        (0..15).collect::<Vec<_>>()
    );

    // An unmapped gamepad with ten axes on Windows has a hat in its axis 9
    let windows = Layout::new(false, 12, 10, 5);
    assert!(windows.hat_is_axis);
    assert_eq!(
        (windows.naxes, windows.nbuttons, windows.nhats),
        (10, 12, 1)
    );
    // ... not elsewhere, nor with a standard mapping or fewer axes
    for (standard, axes, os) in [(false, 10, 2), (true, 10, 5), (false, 9, 5)] {
        let l = Layout::new(standard, 12, axes, os);
        assert!(!l.hat_is_axis);
        assert_eq!(l.nhats, 0);
    }

    // At most 64 of the browser's buttons
    assert_eq!(chrome.buttons(80).count(), 64);
}

#[test]
fn identify_and_guid() {
    // Chrome's PS4 controller on Linux: USB, the "standard" subtype
    let (bus, vendor, product, subtype) = identify(0x054c, 0x09cc, false, 2, false, true);
    assert_eq!(
        (bus, vendor, product, subtype),
        (HARDWARE_BUS_USB, 0x054c, 0x09cc, 0x82)
    );
    let guid = emscripten_guid(
        bus,
        vendor,
        product,
        subtype,
        "Wireless Controller",
        "PS4 Controller",
    );
    let crc = crate::stdlib::crc16(0, b"Wireless Controller");
    let mut want = [0u8; 16];
    want[0..2].copy_from_slice(&HARDWARE_BUS_USB.to_le_bytes());
    want[2..4].copy_from_slice(&crc.to_le_bytes());
    want[4..6].copy_from_slice(&0x054cu16.to_le_bytes());
    want[8..10].copy_from_slice(&0x09ccu16.to_le_bytes());
    want[15] = 0x82;
    assert_eq!(guid.0, want);

    // On Android (or an iPhone): Bluetooth; unmapped: the OS alone
    assert_eq!(
        identify(1, 2, false, 1, false, false),
        (HARDWARE_BUS_BLUETOOTH, 1, 2, 1)
    );
    assert_eq!(
        identify(1, 2, false, 3, false, false),
        (HARDWARE_BUS_BLUETOOTH, 1, 2, 3)
    );
    // An unknown OS: unknown bus
    assert_eq!(
        identify(1, 2, false, 0, false, true),
        (HARDWARE_BUS_UNKNOWN, 1, 2, 0x80)
    );

    // XInput without IDs: the generic Xbox 360 controller, or with trigger
    // rumble the Xbox One S (on USB or Bluetooth)
    assert_eq!(
        identify(0, 0, true, 5, false, true),
        (
            HARDWARE_BUS_USB,
            USB_VENDOR_MICROSOFT,
            USB_PRODUCT_XBOX360_XUSB_CONTROLLER,
            0x85
        )
    );
    assert_eq!(
        identify(0, 0, true, 5, true, true),
        (
            HARDWARE_BUS_USB,
            USB_VENDOR_MICROSOFT,
            USB_PRODUCT_XBOX_ONE_S,
            0x85
        )
    );
    assert_eq!(
        identify(0, 0, true, 1, true, true),
        (
            HARDWARE_BUS_BLUETOOTH,
            USB_VENDOR_MICROSOFT,
            USB_PRODUCT_XBOX_ONE_S_REV1_BLUETOOTH,
            0x81
        )
    );
    // (an XInput gamepad with IDs keeps them)
    assert_eq!(identify(0x045e, 0x028e, true, 5, true, true).2, 0x028e);

    // Without both IDs: the GUID for the SDL name, with the subtype last
    let guid = emscripten_guid(HARDWARE_BUS_USB, 0x1234, 0, 0x02, "Pad", "Pad Controller");
    let mut want = create_joystick_guid_for_name("Pad Controller");
    want.0[15] = 0x02;
    assert_eq!(guid, want);
    // (the subtype in place of the name's NUL)
    assert_eq!(&guid.0[4..15], b"Pad Control");
}

#[test]
fn axis_values() {
    assert_eq!(axis_value(0.0), 0);
    assert_eq!(axis_value(1.0), 32767);
    assert_eq!(axis_value(-1.0), -32767);
    assert_eq!(axis_value(0.5), 16383);
    assert_eq!(axis_value(-0.25), -8191);
}

// -- under node ---------------------------------------------------------------

#[test]
fn unavailable_without_the_gamepad_api() {
    if has_gamepad_api() {
        return;
    }
    let _l = crate::test_support::test_lock();
    // The driver is unavailable, as upstream's
    let e = EMSCRIPTEN_JOYSTICK_DRIVER.init().unwrap_err();
    assert_eq!(e.message(), "Gamepads not supported");
    assert_eq!(EMSCRIPTEN_JOYSTICK_DRIVER.count(), 0);
    EMSCRIPTEN_JOYSTICK_DRIVER.quit();

    // ... and the joystick and gamepad subsystems are there, with the
    // virtual driver
    init::init_subsystem(InitFlags::GAMEPAD).unwrap();
    assert!(joysticks().is_empty());
    assert!(gamepad::gamepads().is_empty());
    crate::joystick::update_joysticks();
    init::quit_subsystem(InitFlags::GAMEPAD);
}

#[test]
fn names_and_ids_from_the_id() {
    let Some(pads) = Pads::new("Mozilla/5.0 (X11; Linux x86_64)") else {
        return;
    };
    let _l = crate::test_support::test_lock();
    let ids = [
        // Chrome, a standard gamepad
        (
            "Wireless Controller (STANDARD GAMEPAD Vendor: 054c Product: 09cc)",
            0x054c,
            0x09cc,
            false,
            "Wireless Controller",
        ),
        // Chrome, unmapped
        (
            "usb gamepad            (Vendor: 0810 Product: e501)",
            0x0810,
            0xe501,
            false,
            "usb gamepad",
        ),
        // Chrome, XInput
        (
            "Xbox 360 Controller (XInput STANDARD GAMEPAD)",
            0,
            0,
            true,
            "Xbox 360 Controller",
        ),
        // Firefox and Safari
        (
            "046d-c216-Logitech Dual Action",
            0x046d,
            0xc216,
            false,
            "Logitech Dual Action",
        ),
        (
            "46d-c216-Logicool Dual Action",
            0x046d,
            0xc216,
            false,
            "Logicool Dual Action",
        ),
        // Firefox, XInput
        ("xinput", 0, 0, true, "xinput"),
        // A name with dashes and no IDs
        ("my-pad-thing", 0, 0, false, "my-pad-thing"),
        // Firefox's dashes found after the IDs, whatever their length
        ("1-2-a-b", 1, 2, false, "a-b"),
    ];
    for (id, vendor, product, xinput, name) in ids {
        pads.set(0, &format!("P.pad(0, {id:?}, 'standard', 17, 4)"));
        assert_eq!(emscripten_joystick_vendor(0), vendor, "{id}");
        assert_eq!(emscripten_joystick_product(0), product, "{id}");
        assert_eq!(is_emscripten_joystick_xinput(0), xinput, "{id}");
        assert_eq!(emscripten_normalized_name(0), name, "{id}");
    }

    // A long name is cut to 127 bytes, at a character
    let long = "é".repeat(70);
    pads.set(0, &format!("P.pad(0, {long:?}, '', 1, 1)"));
    assert_eq!(emscripten_normalized_name(0), "é".repeat(63));

    // Without a gamepad at the index: nothing
    assert_eq!(emscripten_joystick_vendor(3), 0);
    assert_eq!(emscripten_joystick_product(3), 0);
    assert!(!is_emscripten_joystick_xinput(3));
    assert_eq!(emscripten_normalized_name(3), "");

    // The operating systems, the first of upstream's list the user agent
    // names
    for (ua, os) in [
        ("Mozilla/5.0 (Linux; Android 14; Pixel 8)", 1),
        ("Mozilla/5.0 (X11; Linux x86_64)", 2),
        ("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)", 3),
        ("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)", 4),
        ("Mozilla/5.0 (Windows NT 10.0; Win64; x64)", 5),
        ("Node.js/22", 0),
    ] {
        js_run(&format!("Module['sdl3_test_pads'].ua = {ua:?};"));
        assert_eq!(emscripten_os_id(), os, "{ua}");
    }

    // Rumble: a vibrationActuator, and trigger rumble in its effects
    pads.set(0, "P.pad(0, 'x', 'standard', 17, 4)");
    // SAFETY: plain JavaScript.
    unsafe {
        assert_eq!(js::sdl3_joystick_rumble_available(0), 0);
        assert_eq!(js::sdl3_joystick_trigger_rumble_available(0), 0);
    }
    pads.change(0, "g.vibrationActuator = P.actuator(['dual-rumble'])");
    // SAFETY: plain JavaScript.
    unsafe {
        assert_eq!(js::sdl3_joystick_rumble_available(0), 1);
        assert_eq!(js::sdl3_joystick_trigger_rumble_available(0), 0);
    }
    pads.change(
        0,
        "g.vibrationActuator = P.actuator(['dual-rumble', 'trigger-rumble'])",
    );
    // SAFETY: plain JavaScript.
    unsafe {
        assert_eq!(js::sdl3_joystick_trigger_rumble_available(0), 1);
        assert_eq!(js::sdl3_joystick_update_rumble(0, 0xFFFF, 0, 0x8000, 1), 1);
        assert_eq!(js::sdl3_joystick_update_rumble(2, 1, 1, 1, 1), 0);
    }
    assert_eq!(
        js_str("Module['sdl3_test_pads'].rumbles.join('|')"),
        r#"dual-rumble {"startDelay":0,"duration":3000,"weakMagnitude":1,"strongMagnitude":0,"leftTrigger":0.5000076295109483,"rightTrigger":0.000015259021896696422}"#
    );
}

#[test]
fn gamepads_through_the_callbacks() {
    // Upstream's C trace of the same steps (lines of
    // sdl3/tests/emscripten_joystick.rs's, through the DOM events in a
    // browser): here the callbacks are called as emscripten's handlers
    // would, on a stand-in Gamepad API.
    const EXPECTED: &str = r#"event joy added #1
event gamepad added #1
#1 name Wireless Controller guid 03009b514c050000cc09000000000082
#1 ids 054c 09cc 0000 type 1 path That operation is not supported player 0
#1 gamepad 1 Wireless Controller type ps4
#1 mapping 03009b514c050000cc09000000000082,*,a:b0,b:b1,back:b6,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b10,leftshoulder:b4,leftstick:b8,lefttrigger:a4,leftx:a0,lefty:a1,rightshoulder:b5,rightstick:b9,righttrigger:a5,rightx:a2,righty:a3,start:b7,x:b2,y:b3,
open #1 axes 6 buttons 11 hats 1 balls 0 rumble 1 trigger rumble 1 led 0 serial (null)
#1 connected 1 axes 0 0 0 0 -32768 -32768 buttons 00000000000 hats 0
#1 gamepad axes 0 0 0 0 0 0 buttons 00000000000000000000000000
event gamepad button #1 0 1
event joy button #1 0 1
event gamepad button #1 6 1
event joy button #1 7 1
event gamepad button #1 5 1
event joy button #1 10 1
event gamepad button #1 14 1
event gamepad button #1 11 1
event joy hat #1 0 3
event gamepad update complete #1
event joy update complete #1
#1 connected 1 axes 16383 0 0 -32767 -32767 0 buttons 10000001001 hats 3
#1 gamepad axes 16383 0 0 -32767 0 16383 buttons 10000110000100100000000000
event gamepad button #1 0 0
event joy button #1 0 0
event gamepad button #1 2 1
event joy button #1 2 1
event gamepad axis #1 0 16383
event joy axis #1 0 16383
event gamepad axis #1 0 -24575
event joy axis #1 0 -24575
event gamepad axis #1 4 0
event joy axis #1 4 -32767
event gamepad axis #1 4 32767
event joy axis #1 4 32767
event gamepad axis #1 5 16383
event joy axis #1 5 0
event gamepad axis #1 5 32767
event joy axis #1 5 32767
event gamepad button #1 13 1
event gamepad button #1 14 0
event joy hat #1 0 9
event gamepad update complete #1
event joy update complete #1
#1 connected 1 axes -24575 0 0 -32767 32767 32767 buttons 00100001001 hats 9
event gamepad button #1 2 0
event joy button #1 2 0
event gamepad button #1 6 0
event joy button #1 7 0
event joy button #1 10 0
event gamepad axis #1 0 0
event joy axis #1 0 0
event gamepad axis #1 4 0
event joy axis #1 4 -32767
event gamepad axis #1 5 0
event joy axis #1 5 -32767
event gamepad button #1 13 0
event gamepad button #1 11 0
event joy hat #1 0 0
event gamepad update complete #1
event joy update complete #1
#1 connected 1 axes 0 0 0 0 -32767 -32767 buttons 00000000000 hats 0
rumble 1
rumbles dual-rumble {"startDelay":0,"duration":3000,"weakMagnitude":0.5000076295109483,"strongMagnitude":0.2500038147554742,"leftTrigger":0,"rightTrigger":0}
rumble again 1
rumbles (none)
triggers 1
rumbles dual-rumble {"startDelay":0,"duration":3000,"weakMagnitude":0.5000076295109483,"strongMagnitude":0.2500038147554742,"leftTrigger":1,"rightTrigger":0.07110704203860532}
event joy added #2
event gamepad added #2
#2 name Logitech Dual Action guid 0300040e6d04000016c2000000000005
#2 ids 046d c216 0000 type 1 path That operation is not supported player 1
#2 gamepad 1 Logitech Dual Action type standard
open #2 axes 10 buttons 12 hats 1 balls 0 rumble 0 trigger rumble 0 led 0 serial (null)
rumble 0 That operation is not supported
triggers 0 That operation is not supported
led 0 That operation is not supported
event joy button #2 11 1
event joy hat #2 0 8
event joy update complete #2
event gamepad update complete #1
event joy update complete #1
#2 connected 1 axes 0 0 -8191 0 0 0 0 0 0 0 buttons 000000000001 hats 8
event gamepad axis #1 0 16383
event joy axis #1 0 16383
event gamepad axis #1 5 16383
event joy axis #1 5 0
event gamepad axis #1 0 0
event gamepad axis #1 5 0
event gamepad removed #1
event joy removed #1
connected 0 0
rumble 0 That operation is not supported
#1 connected 0 axes 16383 0 0 0 -32767 0 buttons 00000000000 hats 0
event joy added #3
event gamepad added #3
open #3 axes 6 buttons 11 hats 1 balls 0 rumble 0 trigger rumble 0 led 0 serial (null)
"#;
    let Some(pads) = Pads::new(
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36",
    ) else {
        return;
    };
    let _l = crate::test_support::test_lock();
    SEEN.lock().unwrap().clear();
    TRACE.lock().unwrap().clear();
    // (the stand-in comes after the initialization, which fails without a
    // window to register the event handlers on)
    js_run("Module.sdl3_test_saved = navigator.getGamepads; delete navigator.getGamepads;");
    init::init_subsystem(InitFlags::GAMEPAD).unwrap();
    js_run("navigator.getGamepads = Module.sdl3_test_saved;");
    queue::flush_events(EventType::FIRST, EventType::LAST);
    let update = crate::joystick::update_joysticks;
    // (pressing a button leaves the timestamp)
    let press = |index: usize, button: usize, value: f64| {
        pads.change(
            index,
            &format!(
                "g.buttons[{button}].pressed = {value} > 0; g.buttons[{button}].touched = {value} > 0; g.buttons[{button}].value = {value}; g.timestamp--"
            ),
        )
    };

    // A standard gamepad (Chrome's PS4 controller) connects, with a button
    // held and a stick pushed, and rumble for its motors and triggers
    pads.set(
        0,
        "P.pad(0, 'Wireless Controller (STANDARD GAMEPAD Vendor: 054c Product: 09cc)', 'standard', 17, 4)",
    );
    pads.change(
        0,
        "g.vibrationActuator = P.actuator(['dual-rumble', 'trigger-rumble']); g.axes[1] = 0.25; g.timestamp = 0",
    );
    press(0, 1, 1.0);
    pads.event(0, true);
    events();
    let id = crate::joystick::joysticks()[0];
    info(id, true);
    // (a second event for the same index is ignored)
    pads.event(0, true);
    assert_eq!(crate::joystick::joysticks(), [id]);
    assert!(!queue::has_events(EventType::FIRST, EventType::LAST));

    let pad = Gamepad::open(id).unwrap();
    let joy = pad.joystick().unwrap();
    opened(&joy);
    events();
    jstate(&joy);
    gstate(&pad);

    // Buttons, sticks, triggers and the d-pad
    for (b, v) in [
        (0, 1.0),
        (1, 0.0),
        (9, 1.0),
        (16, 1.0),
        (7, 0.5),
        (12, 1.0),
        (15, 1.0),
    ] {
        press(0, b, v);
    }
    pads.change(
        0,
        "g.axes[0] = 0.5; g.axes[1] = 0.25; g.axes[3] = -1; g.timestamp = 1",
    );
    update();
    events();
    jstate(&joy);
    gstate(&pad);
    // The same timestamp: nothing
    press(0, 2, 1.0);
    update();
    events();
    // Up and down, left and right at once; the triggers, released and full
    for (b, v) in [(13, 1.0), (14, 1.0), (7, 1.0), (6, 1.0), (0, 0.0)] {
        press(0, b, v);
    }
    pads.change(0, "g.axes[0] = -0.75; g.timestamp = 2");
    update();
    events();
    jstate(&joy);
    for b in [2, 6, 7, 9, 12, 13, 14, 15, 16] {
        press(0, b, 0.0);
    }
    pads.change(0, "g.axes = [0, 0, 0, 0]; g.timestamp = 3");
    update();
    events();
    jstate(&joy);

    // Rumble
    out!("rumble {}", result(pad.rumble(0x4000, 0x8000, 100)));
    rumbles();
    out!("rumble again {}", result(pad.rumble(0x4000, 0x8000, 100)));
    rumbles();
    out!(
        "triggers {}",
        result(pad.rumble_triggers(0xFFFF, 0x1234, 100))
    );
    rumbles();

    // An unmapped gamepad, as Firefox names it, on Windows: its axis 9 is
    // a hat
    js_run("Module['sdl3_test_pads'].ua = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:140.0) Gecko/20100101 Firefox/140.0';");
    pads.set(1, "P.pad(1, '046d-c216-Logitech Dual Action', '', 12, 10)");
    pads.change(1, "g.axes[9] = 3.2857; g.timestamp = 0");
    pads.event(1, true);
    events();
    let id2 = crate::joystick::joysticks()[1];
    info(id2, false);
    let joy2 = Joystick::open(id2).unwrap();
    opened(&joy2);
    events();
    out!("rumble {}", result(joy2.rumble(1, 1, 100)));
    out!("triggers {}", result(joy2.rumble_triggers(1, 1, 100)));
    out!("led {}", result(joy2.set_led(1, 2, 3)));
    press(1, 11, 1.0);
    pads.change(1, "g.axes[9] = 5 / 7; g.axes[2] = -0.25; g.timestamp = 1");
    update();
    events();
    jstate(&joy2);

    // Disconnected: the open gamepad stays, but nothing more updates it
    pads.event(0, false);
    pads.set(0, "null");
    events();
    out!(
        "connected {} {}",
        pad.connected() as i32,
        joy.connected() as i32
    );
    out!("rumble {}", result(pad.rumble(1, 1, 100)));
    update();
    queue::flush_events(EventType::FIRST, EventType::LAST);
    jstate(&joy);
    drop(joy);
    drop(pad);
    queue::flush_events(EventType::FIRST, EventType::LAST);

    // Reconnected: a new instance (without rumble now)
    js_run("Module['sdl3_test_pads'].ua = 'Mozilla/5.0 (X11; Linux x86_64)';");
    pads.set(
        0,
        "P.pad(0, 'Wireless Controller (STANDARD GAMEPAD Vendor: 054c Product: 09cc)', 'standard', 17, 4)",
    );
    pads.event(0, true);
    events();
    let id3 = *crate::joystick::joysticks().last().unwrap();
    assert!(id3 > id2);
    let pad = Gamepad::open(id3).unwrap();
    opened(&pad.joystick().unwrap());
    drop(pad);
    drop(joy2);

    js_run("delete navigator.getGamepads;");
    init::quit_subsystem(InitFlags::GAMEPAD);
    assert_eq!(EMSCRIPTEN_JOYSTICK_DRIVER.count(), 0);
    drop(pads);

    let trace = TRACE.lock().unwrap().clone();
    assert_eq!(
        trace, EXPECTED,
        "\n--- here:\n{trace}--- upstream's C:\n{EXPECTED}"
    );
}
