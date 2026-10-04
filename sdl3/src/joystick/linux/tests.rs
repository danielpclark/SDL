// Tests for the Linux joystick driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use super::*;
use crate::core::linux::guess_tests::{GuessTest, GUESS_TESTS};
use crate::joystick::gamepad::InputMapping;

fn lock() -> std::sync::MutexGuard<'static, ()> {
    crate::test_support::test_lock()
}

/// A recorded device of test/testevdev.c.
fn recorded(name: &str) -> &'static GuessTest {
    GUESS_TESTS
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("no recorded device {name:?}"))
}

/// Little-endian capability bytes as a kernel bitmask.
fn bitmask<const N: usize>(bytes: &[u8]) -> [c_ulong; N] {
    const LONG: usize = size_of::<c_ulong>();
    let mut out = [0; N];
    for (i, chunk) in bytes.chunks(LONG).enumerate().take(N) {
        let mut word = [0u8; LONG];
        word[..chunk.len()].copy_from_slice(chunk);
        out[i] = c_ulong::from_le_bytes(word);
    }
    out
}

/// A plausible calibration for an axis: hats -1..1, triggers 0..255 and
/// sticks -32768..32767 with some fuzz and flat.
fn typical_absinfo(axis: usize) -> Option<input_absinfo> {
    Some(match axis {
        ABS_HAT0X..=ABS_HAT3Y => input_absinfo {
            minimum: -1,
            maximum: 1,
            ..Default::default()
        },
        ABS_Z | ABS_RZ | ABS_GAS | ABS_BRAKE => input_absinfo {
            minimum: 0,
            maximum: 255,
            ..Default::default()
        },
        _ => input_absinfo {
            minimum: -32768,
            maximum: 32767,
            fuzz: 16,
            flat: 128,
            ..Default::default()
        },
    })
}

/// What `PrepareJoystickHwdata()` and `ConfigJoystick()` make of a device
/// with these capabilities (without opening anything).
fn configure(
    t: &GuessTest,
    absinfo: impl FnMut(usize) -> Option<input_absinfo>,
) -> (HwData, Counts) {
    let mut hwdata = HwData::new(1);
    hwdata.key_map = [0xFF; KEY_CNT];
    hwdata.abs_map = [0xFF; ABS_CNT];
    let mut counts = Counts::default();
    let keybit: KeyBits = bitmask(t.keys);
    let absbit: AbsBits = bitmask(t.abs);
    let relbit: RelBits = bitmask(t.rel);
    let _lock = lock_joysticks();
    config_evdev_inputs(&mut hwdata, &mut counts, &keybit, &absbit, &relbit, absinfo);
    allocate_data(&mut hwdata, &counts);
    (hwdata, counts)
}

/// A mapping as the gamepad module writes it into a mapping string
/// (`SDL_PrivateAppendToMappingString()`), sorted by name.
fn mapping_string(m: &GamepadMapping) -> String {
    let entries: [(&str, &InputMapping); 32] = [
        ("a", &m.a),
        ("b", &m.b),
        ("x", &m.x),
        ("y", &m.y),
        ("back", &m.back),
        ("guide", &m.guide),
        ("start", &m.start),
        ("leftstick", &m.leftstick),
        ("rightstick", &m.rightstick),
        ("leftshoulder", &m.leftshoulder),
        ("rightshoulder", &m.rightshoulder),
        ("dpup", &m.dpup),
        ("dpdown", &m.dpdown),
        ("dpleft", &m.dpleft),
        ("dpright", &m.dpright),
        ("misc1", &m.misc1),
        ("misc2", &m.misc2),
        ("misc3", &m.misc3),
        ("misc4", &m.misc4),
        ("misc5", &m.misc5),
        ("misc6", &m.misc6),
        ("paddle1", &m.right_paddle1),
        ("paddle2", &m.left_paddle1),
        ("paddle3", &m.right_paddle2),
        ("paddle4", &m.left_paddle2),
        ("leftx", &m.leftx),
        ("lefty", &m.lefty),
        ("rightx", &m.rightx),
        ("righty", &m.righty),
        ("lefttrigger", &m.lefttrigger),
        ("righttrigger", &m.righttrigger),
        ("touchpad", &m.touchpad),
    ];
    let mut parts: Vec<String> = entries
        .iter()
        .filter_map(|(name, input)| {
            let value = match input.kind {
                MappingKind::None => return None,
                MappingKind::Button => format!("b{}", input.target),
                MappingKind::Axis => format!("a{}", input.target),
                MappingKind::Hat => format!("h{}.{}", input.target >> 4, input.target & 0x0F),
            };
            Some(format!("{name}:{value}"))
        })
        .collect();
    parts.sort();
    parts.join(",")
}

/// The inputs of a database mapping, sorted by name.
fn database_mapping(guid: &str) -> String {
    let line = crate::joystick::gamepad_db::SECTIONS
        .iter()
        .flat_map(|s| s.iter())
        .find(|line| line.starts_with(guid))
        .unwrap_or_else(|| panic!("no mapping for {guid}"));
    let mut parts: Vec<&str> = line
        .split(',')
        .skip(2)
        .filter(|p| !p.is_empty() && !p.starts_with("crc:"))
        .collect();
    parts.sort();
    parts.join(",")
}

#[test]
fn device_nodes() {
    assert!(str_is_integer("0"));
    assert!(str_is_integer("123"));
    assert!(!str_is_integer(""));
    assert!(!str_is_integer("1a"));
    assert!(is_joystick_js_node("/dev/input/js0"));
    assert!(is_joystick_js_node("js12"));
    assert!(!is_joystick_js_node("/dev/input/js"));
    assert!(!is_joystick_js_node("/dev/input/event3"));
    assert!(is_joystick_event_node("/dev/input/event3"));
    assert!(!is_joystick_event_node("/dev/input/event"));
    assert!(!is_joystick_event_node("/dev/input/mouse0"));
    assert!(is_joystick_device_node(false, "event7"));
    assert!(!is_joystick_device_node(true, "event7"));
    assert!(is_joystick_device_node(true, "js7"));

    let mut nodes: Vec<String> = ["js10", "js2", "js1"].map(String::from).to_vec();
    sort_entries(true, &mut nodes);
    assert_eq!(nodes, ["js1", "js2", "js10"]);
}

#[test]
fn digital_hat_guesses() {
    let _l = lock();
    let hat = input_absinfo {
        minimum: -1,
        maximum: 1,
        ..Default::default()
    };
    let analog = input_absinfo {
        minimum: 0,
        maximum: 255,
        fuzz: 1,
        flat: 15,
        resolution: 0,
        value: 128,
    };
    let unfuzzy = input_absinfo {
        minimum: 0,
        maximum: 255,
        ..Default::default()
    };
    assert!(!guess_if_axes_are_digital_hat(None, None));
    assert!(guess_if_axes_are_digital_hat(Some(&hat), Some(&hat)));
    assert!(guess_if_axes_are_digital_hat(Some(&hat), None));
    assert!(guess_if_axes_are_digital_hat(
        Some(&unfuzzy),
        Some(&unfuzzy)
    ));
    assert!(!guess_if_axes_are_digital_hat(Some(&analog), Some(&hat)));
    hints::set(hints::JOYSTICK_LINUX_DIGITAL_HATS, "1").unwrap();
    assert!(guess_if_axes_are_digital_hat(Some(&analog), Some(&analog)));
    hints::reset(hints::JOYSTICK_LINUX_DIGITAL_HATS);
}

#[test]
fn axis_correction() {
    let mut hwdata = HwData::new(1);
    // An uncalibrated axis is only clamped
    assert_eq!(axis_correct(&hwdata, ABS_X, 40000), 32767);
    assert_eq!(axis_correct(&hwdata, ABS_X, -40000), -32768);
    assert_eq!(axis_correct(&hwdata, ABS_X, 1234), 1234);
    // So are codes past the table (ABS_CNT)
    assert_eq!(axis_correct(&hwdata, ABS_CNT, 99999), 32767);

    // Scaling: 0..255 onto -32768..32767,
    // floor((value - min) * (65535 / 255) - 32768 + 0.5)
    let c = &mut hwdata.abs_correct[ABS_Z];
    c.minimum = 0;
    c.maximum = 255;
    c.scale = 65535.0 / 255.0;
    assert_eq!(axis_correct(&hwdata, ABS_Z, 0), -32768);
    assert_eq!(axis_correct(&hwdata, ABS_Z, 255), 32767);
    assert_eq!(axis_correct(&hwdata, ABS_Z, 128), 128 * 257 - 32768);
    assert_eq!(axis_correct(&hwdata, ABS_Z, 300), 32767);

    // Deadzones, with the coefficients ConfigJoystick() computes for
    // 0..255 with flat 15: coef = (240, 270, (1 << 28) / 195)
    let c = &mut hwdata.abs_correct[ABS_RZ];
    c.minimum = 0;
    c.maximum = 255;
    c.use_deadzones = true;
    c.coef = [255 - 30, 255 + 30, (1 << 28) / (255 - 60)];
    // Inside the flat zone around the centre
    assert_eq!(axis_correct(&hwdata, ABS_RZ, 120), 0);
    assert_eq!(axis_correct(&hwdata, ABS_RZ, 140), 0);
    // ((2 * 200 - 285) * coef2) >> 13
    let coef2 = (1 << 28) / 195;
    assert_eq!(axis_correct(&hwdata, ABS_RZ, 200), (115 * coef2) >> 13);
    assert_eq!(axis_correct(&hwdata, ABS_RZ, 255), 32767);
    assert_eq!(
        axis_correct(&hwdata, ABS_RZ, 50),
        (((100 - 225) * coef2) >> 13).max(-32768)
    );
}

#[test]
fn deadzone_coefficients() {
    let _l = lock();
    hints::set(hints::JOYSTICK_LINUX_DEADZONES, "1").unwrap();
    let (hwdata, counts) = configure(recorded("Xbox 360 wired USB controller"), |axis| {
        if (ABS_HAT0X..=ABS_HAT3Y).contains(&axis) {
            return typical_absinfo(axis);
        }
        Some(input_absinfo {
            minimum: 0,
            maximum: 255,
            flat: 15,
            ..Default::default()
        })
    });
    assert_eq!(counts.naxes, 6);
    // Calibrated hat axes would be analog axes instead
    let (_, counts) = configure(recorded("Xbox 360 wired USB controller"), |_| {
        Some(input_absinfo {
            minimum: 0,
            maximum: 255,
            flat: 15,
            ..Default::default()
        })
    });
    hints::reset(hints::JOYSTICK_LINUX_DEADZONES);
    assert_eq!((counts.naxes, counts.nhats), (8, 0));
    let c = hwdata.abs_correct[ABS_X];
    assert!(c.use_deadzones);
    assert_eq!(c.coef, [255 - 30, 255 + 30, (1 << 28) / (255 - 60)]);
}

#[test]
fn hat_positions() {
    let mut hwdata = HwData::new(1);
    hwdata.has_hat[0] = true;
    hwdata.hats_indices[0] = 0;
    hwdata.hat_correct[0] = HatAxisCorrect {
        use_deadzones: true,
        minimum: [-1, -1],
        maximum: [1, 1],
    };
    hwdata.hats = vec![[1, 1]];
    let mut out = Vec::new();
    let hats = |out: &Vec<Pending>| -> Vec<u8> {
        out.iter()
            .map(|p| match p {
                Pending::Hat(_, 0, v) => *v,
                other => panic!("{other:?}"),
            })
            .collect()
    };

    handle_hat(0, &mut hwdata, 0, 0, 1, &mut out); // right
    handle_hat(0, &mut hwdata, 0, 1, -1, &mut out); // up
    handle_hat(0, &mut hwdata, 0, 1, -1, &mut out); // (no change)
    handle_hat(0, &mut hwdata, 0, 0, 0, &mut out); // centre x
    handle_hat(0, &mut hwdata, 0, 1, 1, &mut out); // down
    handle_hat(0, &mut hwdata, 0, 0, -1, &mut out); // left
    assert_eq!(
        hats(&out),
        [HAT_RIGHT, HAT_RIGHTUP, HAT_UP, HAT_DOWN, HAT_LEFTDOWN]
    );

    // An analog axis treated as a hat: values beyond the range widen it,
    // and with deadzones a third of the range is needed
    out.clear();
    hwdata.hats = vec![[1, 1]];
    hwdata.hat_correct[0].minimum = [-100, -100];
    hwdata.hat_correct[0].maximum = [100, 100];
    handle_hat(0, &mut hwdata, 0, 0, 20, &mut out); // inside the deadzone
    handle_hat(0, &mut hwdata, 0, 0, 40, &mut out); // right
    handle_hat(0, &mut hwdata, 0, 0, 150, &mut out); // (still right, max widened)
    assert_eq!(hwdata.hat_correct[0].maximum[0], 150);
    handle_hat(0, &mut hwdata, 0, 0, 45, &mut out); // back inside the (wider) deadzone
    assert_eq!(hats(&out), [HAT_RIGHT, HAT_CENTERED]);

    // Without deadzones any non-zero value counts
    out.clear();
    hwdata.hat_correct[0].use_deadzones = false;
    handle_hat(0, &mut hwdata, 0, 1, -3, &mut out);
    assert_eq!(hats(&out), [HAT_UP]);
}

#[test]
fn xbox_360_controller() {
    let _l = lock();
    let t = recorded("Xbox 360 wired USB controller");
    let (hwdata, counts) = configure(t, typical_absinfo);
    assert_eq!(
        counts,
        Counts {
            naxes: 6,
            nbuttons: 11,
            nhats: 1,
            nballs: 0
        }
    );
    // Buttons from BTN_JOYSTICK up, axes in code order
    let buttons = [
        BTN_A, BTN_B, BTN_X, BTN_Y, BTN_TL, BTN_TR, BTN_SELECT, BTN_START, BTN_MODE, BTN_THUMBL,
        BTN_THUMBR,
    ];
    for (i, &code) in buttons.iter().enumerate() {
        assert!(hwdata.has_key[code]);
        assert_eq!(hwdata.key_map[code] as usize, i);
    }
    for axis in ABS_X..=ABS_RZ {
        assert_eq!(hwdata.abs_map[axis] as usize, axis);
    }
    assert!(hwdata.has_hat[0] && !hwdata.has_abs[ABS_HAT0X]);
    assert_eq!(hwdata.hats, [[1, 1]]);
    // A -32768..32767 axis is passed through
    assert_eq!(axis_correct(&hwdata, ABS_X, -32768), -32768);
    assert_eq!(axis_correct(&hwdata, ABS_X, 32767), 32767);
    assert_eq!(axis_correct(&hwdata, ABS_X, 0), 0);

    // The layout of the Linux Gamepad Specification for xpad
    let mapping = generate_gamepad_mapping(&hwdata, 0x045e, Some("xpad")).unwrap();
    assert_eq!(
        mapping_string(&mapping),
        "a:b0,b:b1,back:b6,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b8,\
         leftshoulder:b4,leftstick:b9,lefttrigger:a2,leftx:a0,lefty:a1,rightshoulder:b5,\
         rightstick:b10,righttrigger:a5,rightx:a3,righty:a4,start:b7,x:b2,y:b3"
    );

    // The GUID IsJoystick() creates for it
    let guid = create_joystick_guid(
        0x0003,
        0x045e,
        0x028e,
        0x0114,
        None,
        Some("Microsoft X-Box 360 pad"),
        0,
        0,
    );
    let crc = crate::stdlib::crc16(0, b"Microsoft X-Box 360 pad");
    let mut expected = [0u8; 16];
    expected[0] = 0x03;
    expected[2..4].copy_from_slice(&crc.to_le_bytes());
    expected[4..6].copy_from_slice(&0x045eu16.to_le_bytes());
    expected[8..10].copy_from_slice(&0x028eu16.to_le_bytes());
    expected[12..14].copy_from_slice(&0x0114u16.to_le_bytes());
    assert_eq!(guid.0, expected);
    assert_eq!(joystick_guid_info(guid), (0x045e, 0x028e, 0x0114, crc));
}

#[test]
fn dualshock_4_matches_the_database() {
    let _l = lock();
    // 0003:054c:09cc v8111 through hid-playstation
    let t = recorded("DualShock 4 - gamepad");
    let (hwdata, counts) = configure(t, typical_absinfo);
    assert_eq!(counts.nbuttons, 13);
    let mapping = generate_gamepad_mapping(&hwdata, super::super::USB_VENDOR_SONY, None).unwrap();
    assert_eq!(
        mapping_string(&mapping),
        database_mapping("030000004c050000cc09000011810000")
    );
}

#[test]
fn android_style_and_paddles() {
    let _l = lock();
    // Android-style axes: Z/RZ right stick, BRAKE/GAS triggers; KEY_RECORD
    let t = recorded("X-Box One Elite 2 via Bluetooth");
    let (hwdata, _) = configure(t, typical_absinfo);
    let mapping = generate_gamepad_mapping(&hwdata, 0x045e, None).unwrap();
    let s = mapping_string(&mapping);
    let a = |code| hwdata.abs_map[code];
    assert!(s.contains(&format!("lefttrigger:a{}", a(ABS_BRAKE))), "{s}");
    assert!(s.contains(&format!("righttrigger:a{}", a(ABS_GAS))), "{s}");
    assert!(s.contains(&format!("rightx:a{}", a(ABS_Z))), "{s}");
    assert!(s.contains(&format!("righty:a{}", a(ABS_RZ))), "{s}");
    assert!(
        s.contains(&format!("misc1:b{}", hwdata.key_map[KEY_RECORD])),
        "{s}"
    );

    // Paddles as BTN_TRIGGER_HAPPY5..8 on Microsoft devices
    let t = recorded("X-Box One Elite 2 via USB");
    let (hwdata, _) = configure(t, typical_absinfo);
    let m = generate_gamepad_mapping(&hwdata, 0x045e, Some("xpad")).unwrap();
    assert_eq!(m.right_paddle1.target, hwdata.key_map[BTN_TRIGGER_HAPPY5]);
    assert_eq!(m.left_paddle1.target, hwdata.key_map[BTN_TRIGGER_HAPPY7]);
    assert_eq!(m.right_paddle2.target, hwdata.key_map[BTN_TRIGGER_HAPPY6]);
    assert_eq!(m.left_paddle2.target, hwdata.key_map[BTN_TRIGGER_HAPPY8]);
    // ... but not on others
    let m = generate_gamepad_mapping(&hwdata, 0x1234, Some("xpad")).unwrap();
    assert_eq!(m.right_paddle1.kind, MappingKind::None);

    // Not a gamepad: no BTN_GAMEPAD
    let t = recorded("Saitek ST290 Pro flight stick");
    let (hwdata, counts) = configure(t, typical_absinfo);
    assert!(counts.naxes > 0);
    assert!(generate_gamepad_mapping(&hwdata, 0x06a3, None).is_none());
}

/// A non-blocking pipe standing in for a device node.
struct Pipe {
    read: RawFd,
    write: RawFd,
}

impl Pipe {
    fn new() -> Pipe {
        let mut fds = [0; 2];
        // SAFETY: fds is a writable array of two descriptors.
        assert_eq!(
            unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) },
            0
        );
        Pipe {
            read: fds[0],
            write: fds[1],
        }
    }

    fn send<T: Copy>(&self, events: &[T]) {
        let bytes = std::mem::size_of_val(events);
        // SAFETY: events is readable for its size in bytes.
        let n = unsafe { libc::write(self.write, events.as_ptr().cast(), bytes) };
        assert_eq!(n, bytes as isize);
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        close_fd(self.read);
        close_fd(self.write);
    }
}

fn ev(type_: u16, code: usize, value: i32) -> input_event {
    input_event {
        type_,
        code: code as u16,
        value,
        ..Default::default()
    }
}

/// The pending events without their timestamps (which come from the clock).
fn untimed(events: Vec<Pending>) -> Vec<Pending> {
    events
        .into_iter()
        .map(|p| match p {
            Pending::Axis(_, a, v) => Pending::Axis(0, a, v),
            Pending::Button(_, b, d) => Pending::Button(0, b, d),
            Pending::Hat(_, h, v) => Pending::Hat(0, h, v),
            Pending::Ball(_, b, x, y) => Pending::Ball(0, b, x, y),
            Pending::Sensor(_, t, s, d) => Pending::Sensor(0, t, s, d),
        })
        .collect()
}

#[test]
fn input_events() {
    let _l = lock();
    let pipe = Pipe::new();
    let t = recorded("Xbox 360 wired USB controller");
    let (mut hwdata, _) = configure(t, typical_absinfo);
    hwdata.fd = pipe.read;

    pipe.send(&[
        ev(EV_KEY, BTN_A, 1),
        ev(EV_KEY, BTN_START, 1),
        ev(EV_ABS, ABS_X, -32768),
        ev(EV_ABS, ABS_HAT0X, 1),
        ev(EV_ABS, ABS_HAT0Y, -1),
        ev(EV_ABS, ABS_RZ, 255),
        ev(EV_SYN, SYN_REPORT as usize, 0),
        ev(EV_KEY, BTN_A, 0),
        // An unmapped key is sent as button 255, which the front end drops
        ev(EV_KEY, BTN_TRIGGER_HAPPY1, 1),
        ev(EV_SYN, SYN_REPORT as usize, 0),
    ]);
    let _lock = lock_joysticks();
    assert_eq!(
        untimed(update_events(&mut hwdata)),
        [
            Pending::Button(0, 0, true),
            Pending::Button(0, 7, true),
            Pending::Axis(0, 0, -32768),
            Pending::Hat(0, 0, HAT_RIGHT),
            Pending::Hat(0, 0, HAT_RIGHTUP),
            Pending::Axis(0, 5, 32767),
            Pending::Button(0, 0, false),
            Pending::Button(0, 0xFF, true),
        ]
    );
    assert!(!hwdata.gone);

    // After SYN_DROPPED the rest of the packet is ignored, then the state
    // is polled (which fails on a pipe, so nothing is sent)
    pipe.send(&[
        ev(EV_KEY, BTN_B, 1),
        ev(EV_SYN, SYN_DROPPED as usize, 0),
        ev(EV_KEY, BTN_X, 1),
        ev(EV_ABS, ABS_Y, 77),
        ev(EV_SYN, SYN_REPORT as usize, 0),
        ev(EV_KEY, BTN_Y, 1),
    ]);
    assert_eq!(
        untimed(update_events(&mut hwdata)),
        [Pending::Button(0, 1, true), Pending::Button(0, 3, true)]
    );
    assert!(!hwdata.recovering_from_dropped);

    // Nothing to read
    assert!(update_events(&mut hwdata).is_empty());
}

#[test]
fn ball_motion() {
    let _l = lock();
    let pipe = Pipe::new();
    // A mouse-like device with relative axes has a ball
    let t = recorded("Fake pointing stick with no buttons");
    let (mut hwdata, counts) = configure(t, typical_absinfo);
    assert_eq!(counts.nballs, 1);
    hwdata.fd = pipe.read;
    pipe.send(&[
        ev(EV_REL, REL_X as usize, 3),
        ev(EV_REL, REL_Y as usize, -2),
        ev(EV_REL, REL_X as usize, 4),
        ev(EV_SYN, SYN_REPORT as usize, 0),
    ]);
    let _lock = lock_joysticks();
    // Accumulated and delivered once per update
    assert_eq!(update_events(&mut hwdata), [Pending::Ball(0, 0, 7, -2)]);
    assert!(update_events(&mut hwdata).is_empty());
}

#[test]
fn sensor_events() {
    let _l = lock();
    let pipe = Pipe::new();
    let sensor = Pipe::new();
    let t = recorded("DualShock 4 - gamepad");
    let (mut hwdata, _) = configure(t, typical_absinfo);
    hwdata.fd = pipe.read;
    hwdata.fd_sensor = sensor.read;
    hwdata.has_accelerometer = true;
    hwdata.has_gyro = true;
    hwdata.report_sensor = true;
    hwdata.accelerometer_scale = [8192.0; 3];
    hwdata.gyro_scale = [1024.0; 3];

    sensor.send(&[
        ev(EV_ABS, ABS_X, 8192),
        ev(EV_ABS, ABS_Y, -8192),
        ev(EV_ABS, ABS_Z, 0),
        ev(EV_ABS, ABS_RX, 1024 * 180),
        ev(EV_MSC, MSC_TIMESTAMP as usize, 100),
        ev(EV_SYN, SYN_REPORT as usize, 0),
        ev(EV_MSC, MSC_TIMESTAMP as usize, 50), // wrapped around
        ev(EV_SYN, SYN_REPORT as usize, 0),
    ]);
    let _lock = lock_joysticks();
    let pi = std::f32::consts::PI;
    let tick2 = 100 + (i32::MAX - 100 + 50 + 1) as u64;
    assert_eq!(
        untimed(update_events(&mut hwdata)),
        [
            Pending::Sensor(0, SensorType::Gyro, 100_000, [pi, 0.0, 0.0]),
            Pending::Sensor(
                0,
                SensorType::Accel,
                100_000,
                [STANDARD_GRAVITY, -STANDARD_GRAVITY, 0.0]
            ),
            Pending::Sensor(0, SensorType::Gyro, tick2 * 1000, [pi, 0.0, 0.0]),
            Pending::Sensor(
                0,
                SensorType::Accel,
                tick2 * 1000,
                [STANDARD_GRAVITY, -STANDARD_GRAVITY, 0.0]
            ),
        ]
    );

    // The Nintendo driver's axis order
    hwdata.item_vendor = super::super::USB_VENDOR_NINTENDO;
    assert_eq!(
        correct_sensor_data(&hwdata, &[1.0, 2.0, 3.0]),
        [-2.0, 3.0, -1.0]
    );
    hwdata.fd = -1;
    hwdata.fd_sensor = -1;
}

#[test]
fn classic_events() {
    let _l = lock();
    let pipe = Pipe::new();
    let mut hwdata = HwData::new(1);
    hwdata.key_map = [0xFF; KEY_CNT];
    hwdata.abs_map = [0xFF; ABS_CNT];
    hwdata.classic = true;
    hwdata.fd = pipe.read;
    // What JSIOCGBTNMAP/JSIOCGAXMAP would say for a pad with two buttons,
    // two axes and a hat
    let mut key_pam = vec![0u16; KEY_MAX - BTN_MISC + 1];
    key_pam[0] = BTN_A as u16;
    key_pam[1] = BTN_B as u16;
    let mut abs_pam = vec![0u8; ABS_CNT];
    abs_pam[0] = ABS_X as u8;
    abs_pam[1] = ABS_Y as u8;
    abs_pam[2] = ABS_HAT0X as u8;
    abs_pam[3] = ABS_HAT0Y as u8;
    for (i, &code) in [BTN_A, BTN_B].iter().enumerate() {
        hwdata.key_map[code] = i as u8;
        hwdata.has_key[code] = true;
    }
    hwdata.abs_map[ABS_X] = 0;
    hwdata.abs_map[ABS_Y] = 1;
    hwdata.has_hat[0] = true;
    hwdata.hat_correct[0].minimum = [-1, -1];
    hwdata.hat_correct[0].maximum = [1, 1];
    hwdata.hats = vec![[1, 1]];
    hwdata.key_pam = Some(key_pam);
    hwdata.abs_pam = Some(abs_pam);

    let js = |type_: u8, number: u8, value: i16| js_event {
        time: 0,
        value,
        type_,
        number,
    };
    pipe.send(&[
        js(JS_EVENT_BUTTON, 1, 1),
        js(JS_EVENT_AXIS, 1, -200),
        js(JS_EVENT_AXIS, 2, 32767),
        // Initial-state events (JS_EVENT_INIT set) are ignored
        js(JS_EVENT_BUTTON | 0x80, 0, 1),
    ]);
    let _lock = lock_joysticks();
    assert_eq!(
        untimed(update_events(&mut hwdata)),
        [
            Pending::Button(0, 1, true),
            Pending::Axis(0, 1, -200),
            Pending::Hat(0, 0, HAT_RIGHT),
        ]
    );
    hwdata.fd = -1;
}

#[test]
fn classic_highest_codes() {
    let _l = lock();
    let pipe = Pipe::new();
    let mut hwdata = HwData::new(1);
    hwdata.key_map = [0xFF; KEY_CNT];
    hwdata.abs_map = [0xFF; ABS_CNT];
    hwdata.fd = pipe.read;
    // joydev maps buttons up to KEY_MAX and axes up to ABS_MAX
    let mut key_pam = vec![0u16; KEY_MAX - BTN_MISC + 1];
    key_pam[0] = BTN_A as u16;
    key_pam[1] = KEY_MAX as u16;
    let mut abs_pam = vec![0u8; ABS_CNT];
    abs_pam[0] = ABS_X as u8;
    abs_pam[1] = ABS_MAX as u8;
    let mut counts = Counts::default();
    let _lock = lock_joysticks();
    map_classic_inputs(
        &mut hwdata,
        &mut counts,
        Some((key_pam, 2)),
        Some((abs_pam, 2)),
    );
    assert_eq!((counts.nbuttons, counts.naxes), (2, 2));
    assert!(hwdata.has_key[KEY_MAX] && hwdata.has_abs[ABS_MAX]);

    let js = |type_: u8, number: u8, value: i16| js_event {
        time: 0,
        value,
        type_,
        number,
    };
    pipe.send(&[js(JS_EVENT_BUTTON, 1, 1), js(JS_EVENT_AXIS, 1, -200)]);
    // The last button and axis are reported, not dropped as unmapped
    assert_eq!(
        untimed(update_events(&mut hwdata)),
        [Pending::Button(0, 1, true), Pending::Axis(0, 1, -200)]
    );
    hwdata.fd = -1;
}

#[test]
fn temporary_close_keeps_the_device_linked() {
    let item = |device_instance, path: &str| JoylistItem {
        device_instance,
        path: path.to_owned(),
        vendor: 0,
        name: String::new(),
        driver: None,
        guid: Guid::ZERO,
        devnum: 0,
        steam_virtual_gamepad_slot: -1,
        hwdata: true,
        checked_mapping: false,
        mapping: None,
    };
    let mut s = LinuxState {
        classic_joysticks: false,
        enumeration_method: EnumerationMethod::Unset,
        joylist: vec![item(7, "/dev/input/event7"), item(8, "/dev/input/event8")],
        sensorlist: Vec::new(),
        inotify_fd: -1,
        last_joy_detect_time: 0,
        last_input_dir_mtime: 0,
        open: Vec::new(),
    };

    // The fake joystick of LINUX_JoystickGetGamepadMapping() for a device
    // that's also open as a real joystick
    let mut temporary = HwData::new(0);
    temporary.item = true;
    temporary.fname = "/dev/input/event7".to_owned();
    close_hwdata(&mut s, &mut temporary);
    assert!(s.joylist.iter().all(|item| item.hwdata));

    // Closing the real joystick unlinks its device only
    let mut real = HwData::new(7);
    real.item = true;
    real.fname = "/dev/input/event7".to_owned();
    close_hwdata(&mut s, &mut real);
    assert!(!s.joylist[0].hwdata && s.joylist[1].hwdata);
}

#[test]
fn not_a_joystick() {
    let _l = lock();
    // A character device that isn't an input device is never added
    hints::set(hints::JOYSTICK_DEVICE, "/dev/null:/nonexistent").unwrap();
    crate::init::init_subsystem(crate::init::InitFlags::JOYSTICK).unwrap();
    {
        let _lock = lock_joysticks();
        assert!(with_state(|s| !s
            .joylist
            .iter()
            .any(|item| item.path == "/dev/null")));
    }
    crate::init::quit_subsystem(crate::init::InitFlags::JOYSTICK);
    hints::reset(hints::JOYSTICK_DEVICE);
}

/// End-to-end tests with virtual devices created through uinput, when this
/// system allows it (they are skipped otherwise, as on CI).
mod uinput {
    use super::*;
    use crate::joystick::{joysticks, Joystick};

    const UI_DEV_CREATE: c_ulong = io(b'U', 1);
    const UI_DEV_DESTROY: c_ulong = io(b'U', 2);
    const UI_DEV_SETUP: c_ulong = iow(b'U', 3, size_of::<UinputSetup>());
    const UI_ABS_SETUP: c_ulong = iow(b'U', 4, size_of::<UinputAbsSetup>());
    const UI_SET_EVBIT: c_ulong = iow(b'U', 100, size_of::<c_int>());
    const UI_SET_KEYBIT: c_ulong = iow(b'U', 101, size_of::<c_int>());
    const UI_SET_ABSBIT: c_ulong = iow(b'U', 103, size_of::<c_int>());

    /// `UI_GET_SYSNAME(len)`
    const fn ui_get_sysname(len: usize) -> c_ulong {
        ior(b'U', 44, len)
    }

    /// `struct uinput_setup`
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct UinputSetup {
        id: input_id,
        name: [u8; 80],
        ff_effects_max: u32,
    }

    /// `struct uinput_abs_setup`
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct UinputAbsSetup {
        code: u16,
        absinfo: input_absinfo,
    }

    /// A virtual gamepad: an Xbox 360-like layout.
    struct VirtualPad {
        fd: RawFd,
        node: String,
    }

    impl VirtualPad {
        fn create() -> Option<VirtualPad> {
            let fd = open_path(
                "/dev/uinput",
                libc::O_WRONLY | libc::O_NONBLOCK | libc::O_CLOEXEC,
            );
            if fd < 0 {
                return None;
            }
            let set = |request, value: usize| ioctl_int(fd, request, value as c_int) >= 0;
            let mut ok = set(UI_SET_EVBIT, EV_KEY as usize) && set(UI_SET_EVBIT, EV_ABS as usize);
            for key in [
                BTN_A, BTN_B, BTN_X, BTN_Y, BTN_TL, BTN_TR, BTN_SELECT, BTN_START, BTN_MODE,
                BTN_THUMBL, BTN_THUMBR,
            ] {
                ok &= set(UI_SET_KEYBIT, key);
            }
            for axis in [
                ABS_X, ABS_Y, ABS_Z, ABS_RX, ABS_RY, ABS_RZ, ABS_HAT0X, ABS_HAT0Y,
            ] {
                ok &= set(UI_SET_ABSBIT, axis);
                let mut abs = UinputAbsSetup {
                    code: axis as u16,
                    absinfo: typical_absinfo(axis).unwrap(),
                };
                ok &= ioctl_read(fd, UI_ABS_SETUP, &mut abs) >= 0;
            }
            let mut setup = UinputSetup {
                id: input_id {
                    bustype: 0x0003,
                    vendor: 0x045e,
                    product: 0x028e,
                    version: 0x0114,
                },
                name: [0; 80],
                ff_effects_max: 0,
            };
            let name = b"SDL uinput test pad";
            setup.name[..name.len()].copy_from_slice(name);
            ok &= ioctl_read(fd, UI_DEV_SETUP, &mut setup) >= 0;
            ok &= ioctl_int(fd, UI_DEV_CREATE, 0) >= 0;
            if !ok {
                close_fd(fd);
                return None;
            }
            let (result, sysname) = ioctl_string(fd, ui_get_sysname, 64);
            let node = (result >= 0)
                .then(|| {
                    let dir = format!("/sys/devices/virtual/input/{sysname}");
                    // Wait for the event node to appear
                    for _ in 0..100 {
                        let found = scan_directory(&dir, is_joystick_event_node);
                        if let Some(event) = found.first() {
                            let path = format!("/dev/input/{event}");
                            if std::fs::metadata(&path).is_ok() {
                                return Some(path);
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    None
                })
                .flatten();
            let Some(node) = node else {
                ioctl_int(fd, UI_DEV_DESTROY, 0);
                close_fd(fd);
                return None;
            };
            Some(VirtualPad { fd, node })
        }

        fn emit(&self, events: &[input_event]) {
            for event in events {
                assert!(write_event(self.fd, event) >= 0);
            }
        }
    }

    impl Drop for VirtualPad {
        fn drop(&mut self) {
            ioctl_int(self.fd, UI_DEV_DESTROY, 0);
            close_fd(self.fd);
        }
    }

    fn pump_until(mut done: impl FnMut() -> bool) -> bool {
        for _ in 0..300 {
            crate::events::queue::pump_event_maintenance();
            crate::joystick::update_joysticks();
            if done() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn virtual_gamepad_end_to_end() {
        let _l = lock();
        let Some(pad) = VirtualPad::create() else {
            crate::test_support::skip("uinput", "/dev/uinput isn't usable here");
            return;
        };
        // Use the node directly, so the test doesn't depend on udev
        // permissions or the device directory being watched
        hints::set(hints::JOYSTICK_DEVICE, &pad.node).unwrap();
        crate::init::init_subsystem(
            crate::init::InitFlags::JOYSTICK | crate::init::InitFlags::GAMEPAD,
        )
        .unwrap();

        let id = {
            let _lock = lock_joysticks();
            with_state(|s| {
                s.joylist
                    .iter()
                    .find(|item| item.path == pad.node)
                    .map(|item| item.device_instance)
            })
        };
        let id = id.expect("the uinput device was detected");
        assert!(joysticks().contains(&id));
        let joystick = Joystick::open(id).unwrap();
        assert_eq!(joystick.num_axes().unwrap(), 6);
        assert_eq!(joystick.num_buttons().unwrap(), 11);
        assert_eq!(joystick.num_hats().unwrap(), 1);
        assert_eq!(joystick.vendor(), 0x045e);
        assert_eq!(joystick.product(), 0x028e);

        pad.emit(&[
            ev(EV_KEY, BTN_B, 1),
            ev(EV_ABS, ABS_X, -32768),
            ev(EV_ABS, ABS_HAT0Y, 1),
            ev(EV_SYN, SYN_REPORT as usize, 0),
        ]);
        assert!(pump_until(|| joystick.button(1).unwrap_or(false)));
        assert!(pump_until(|| joystick.axis(0).unwrap_or(0) == -32768));
        assert!(pump_until(|| joystick.hat(0).unwrap_or(0) == HAT_DOWN));

        // The generated gamepad mapping
        let mapping = {
            let _lock = lock_joysticks();
            let index =
                with_state(|s| s.joylist.iter().position(|item| item.device_instance == id))
                    .unwrap();
            LINUX_JOYSTICK_DRIVER.gamepad_mapping(index).unwrap()
        };
        assert_eq!(
            mapping_string(&mapping),
            "a:b0,b:b1,back:b6,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b8,\
             leftshoulder:b4,leftstick:b9,lefttrigger:a2,leftx:a0,lefty:a1,rightshoulder:b5,\
             rightstick:b10,righttrigger:a5,rightx:a3,righty:a4,start:b7,x:b2,y:b3"
        );
        assert!(crate::joystick::gamepad::is_gamepad(id));

        // Hotplug removal: the read fails with ENODEV, then the device is removed
        drop(pad);
        assert!(pump_until(|| !joysticks().contains(&id)));
        drop(joystick);

        crate::init::quit_subsystem(
            crate::init::InitFlags::JOYSTICK | crate::init::InitFlags::GAMEPAD,
        );
        hints::reset(hints::JOYSTICK_DEVICE);
    }
}
