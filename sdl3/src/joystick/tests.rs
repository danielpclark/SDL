// Tests of the joystick front end, the gamepad mappings and the virtual
// joystick driver.
//
// `virtual_trace_matches_c` replays the scenario of a C program run against
// upstream SDL (built with only the virtual joystick driver, on Linux) and
// compares the trace line by line with `testdata/virtual_trace_linux.txt`.
// The upstream build has the virtual driver set `joystick->nballs` in
// `VIRTUAL_JoystickOpen()`, the upstream bug fixed here, so the trace counts
// the joystick's ball and has its motion event.

// (the trace helpers serve only the Linux trace test)
#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use super::gamepad::{
    self, Gamepad, GamepadAxis, GamepadBindingInput, GamepadBindingOutput, GamepadButton,
    GamepadType,
};
use super::*;
use crate::events::queue;

struct Trace {
    out: String,
    cur: JoystickID,
}

impl Trace {
    fn p(&mut self, line: impl AsRef<str>) {
        self.out.push_str(line.as_ref());
        self.out.push('\n');
    }

    fn w(&self, which: JoystickID) -> String {
        if which == self.cur {
            "cur".to_string()
        } else {
            which.to_string()
        }
    }

    fn dump(&mut self, tag: &str) {
        self.p(format!("-- events {tag}"));
        for event in queue::get_events(EventType::FIRST, EventType::LAST, usize::MAX).unwrap() {
            let line = match &event {
                Event::JoyAxis(e) => format!("JAXIS {} {} {}", self.w(e.which), e.axis, e.value),
                Event::JoyBall(e) => {
                    format!("JBALL {} {} {} {}", self.w(e.which), e.ball, e.xrel, e.yrel)
                }
                Event::JoyHat(e) => format!("JHAT {} {} {}", self.w(e.which), e.hat, e.value),
                Event::JoyButton(e) => {
                    format!("JBUTTON {} {} {}", self.w(e.which), e.button, e.down as i32)
                }
                Event::JoyDevice(e) => format!("JDEVICE {:x} {}", e.event_type.0, self.w(e.which)),
                Event::GamepadAxis(e) => {
                    format!("GAXIS {} {} {}", self.w(e.which), e.axis, e.value)
                }
                Event::GamepadButton(e) => {
                    format!("GBUTTON {} {} {}", self.w(e.which), e.button, e.down as i32)
                }
                Event::GamepadDevice(e) => {
                    format!("GDEVICE {:x} {}", e.event_type.0, self.w(e.which))
                }
                Event::GamepadTouchpad(e) => format!(
                    "GTOUCH {:x} {} {} {} {:.3} {:.3} {:.3}",
                    e.event_type.0,
                    self.w(e.which),
                    e.touchpad,
                    e.finger,
                    e.x,
                    e.y,
                    e.pressure
                ),
                Event::GamepadSensor(e) => format!(
                    "GSENSOR {} {} {:.3} {:.3} {:.3} {}",
                    self.w(e.which),
                    e.sensor,
                    e.data[0],
                    e.data[1],
                    e.data[2],
                    e.sensor_timestamp
                ),
                other => format!("EV {:x}", other.event_type().0),
            };
            self.p(line);
        }
    }
}

fn s(name: Result<Option<String>>) -> String {
    name.ok().flatten().unwrap_or_else(|| "(null)".to_string())
}

fn err<T>(r: &Result<T>) -> String {
    r.as_ref().err().map(|e| e.to_string()).unwrap_or_default()
}

fn add_result(r: &Result<bool>) -> i32 {
    match r {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(_) => -1,
    }
}

fn virtual_trace() -> (String, Vec<String>) {
    let mut t = Trace {
        out: String::new(),
        cur: 0,
    };

    hints::set(hints::GAMECONTROLLERCONFIG, "").unwrap();
    init::init_subsystem(InitFlags::GAMEPAD).unwrap();
    t.dump("init");

    // A: a virtual wheel
    let rumbles = Arc::new(Mutex::new(Vec::new()));
    let updates = Arc::new(AtomicU32::new(0));
    let cleaned = Arc::new(AtomicU32::new(0));
    let desc = VirtualJoystickDesc {
        joystick_type: JoystickType::Wheel,
        naxes: 3,
        nbuttons: 4,
        nhats: 1,
        nballs: 1,
        rumble: Some({
            let rumbles = rumbles.clone();
            Arc::new(move |lo, hi| {
                rumbles.lock().unwrap().push((lo, hi));
                Ok(())
            })
        }),
        update: Some({
            let updates = updates.clone();
            Arc::new(move || {
                updates.fetch_add(1, Ordering::Relaxed);
            })
        }),
        cleanup: Some({
            let cleaned = cleaned.clone();
            Box::new(move || {
                cleaned.fetch_add(1, Ordering::Relaxed);
            })
        }),
        ..VirtualJoystickDesc::default()
    };
    t.p(format!("has {}", has_joystick() as i32));
    let id = attach_virtual_joystick(desc).unwrap();
    t.cur = id;
    t.p(format!(
        "has {} virtual {} name [{}] type {} gamepad {}",
        has_joystick() as i32,
        is_joystick_virtual(id) as i32,
        s(joystick_name_for_id(id)),
        joystick_type_for_id(id) as i32,
        gamepad::is_gamepad(id) as i32
    ));
    let path = joystick_path_for_id(id);
    t.p(format!(
        "path [{}] err=[{}]",
        path.as_deref().unwrap_or("(null)"),
        err(&path)
    ));
    t.p(format!("guid {}", joystick_guid_for_id(id)));
    t.dump("attach");

    let j = Joystick::open(id).unwrap();
    t.p(format!(
        "counts {} {} {} {} updates>0 {}",
        j.num_axes().unwrap(),
        j.num_buttons().unwrap(),
        j.num_hats().unwrap(),
        j.num_balls().unwrap(),
        (updates.load(Ordering::Relaxed) > 0) as i32
    ));
    let props = j.properties().unwrap();
    t.p(format!(
        "props rumble {} led {}",
        props
            .get_bool(PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN)
            .unwrap_or(false) as i32,
        props
            .get_bool(PROP_JOYSTICK_CAP_RGB_LED_BOOLEAN)
            .unwrap_or(false) as i32
    ));
    t.p(format!("axis3 err=[{}]", err(&j.axis(3))));
    t.dump("open");

    j.set_virtual_axis(0, 12000).unwrap();
    j.set_virtual_button(2, true).unwrap();
    j.set_virtual_hat(0, HAT_LEFTUP).unwrap();
    j.set_virtual_ball(0, 5, -5).unwrap();
    t.p(format!("axis5 err=[{}]", err(&j.set_virtual_axis(5, 1))));
    update_joysticks();
    t.p(format!(
        "state {} {} {}",
        j.axis(0).unwrap(),
        j.button(2).unwrap() as i32,
        j.hat(0).unwrap()
    ));
    t.dump("update1");

    j.set_virtual_axis(0, 12001).unwrap();
    j.set_virtual_axis(1, -32768).unwrap();
    update_joysticks();
    t.dump("update2");

    let _ = j.rumble(100, 200, 1000);
    let _ = j.rumble(100, 200, 1000);
    let _ = j.rumble(0, 0, 0);
    {
        let log = rumbles.lock().unwrap();
        t.p(format!(
            "rumbles {} [{} {}] [{} {}]",
            log.len(),
            log[0].0,
            log[0].1,
            log[1].0,
            log[1].1
        ));
    }
    let led = j.set_led(1, 2, 3);
    t.p(format!("led {} err=[{}]", led.is_ok() as i32, err(&led)));
    j.set_player_index(2).unwrap();
    let from = Joystick::from_player_index(2).is_some_and(|other| other.id() == j.id());
    t.p(format!(
        "player {} {} from {}",
        j.player_index(),
        joystick_player_index_for_id(id),
        from as i32
    ));
    let _ = j.rumble(1, 2, 0);

    detach_virtual_joystick(id).unwrap();
    t.p(format!(
        "cleaned {} connected {} player {}",
        cleaned.load(Ordering::Relaxed),
        j.connected() as i32,
        j.player_index()
    ));
    t.dump("detach");
    t.p(format!("name after [{}]", s(joystick_name_for_id(id))));
    let again = detach_virtual_joystick(id);
    t.p(format!(
        "detach again {} err=[{}]",
        again.is_ok() as i32,
        err(&again)
    ));
    let after = j.set_virtual_axis(0, 1);
    t.p(format!(
        "set after {} err=[{}]",
        after.is_ok() as i32,
        err(&after)
    ));
    drop(j);
    t.p(format!("rumbles {}", rumbles.lock().unwrap().len()));
    t.dump("close");

    // B: a virtual gamepad
    let desc = VirtualJoystickDesc {
        joystick_type: JoystickType::Gamepad,
        naxes: 6,
        nbuttons: 15,
        touchpads: vec![VirtualJoystickTouchpadDesc { nfingers: 2 }],
        sensors: vec![VirtualJoystickSensorDesc {
            sensor_type: SensorType::Gyro,
            rate: 250.0,
        }],
        name: Some("Test, Pad".to_string()),
        ..VirtualJoystickDesc::default()
    };
    let id = attach_virtual_joystick(desc).unwrap();
    t.cur = id;
    t.p(format!(
        "gamepad {} player {}",
        gamepad::is_gamepad(id) as i32,
        joystick_player_index_for_id(id)
    ));
    t.dump("attach pad");
    t.p(format!(
        "mapping [{}]",
        gamepad::gamepad_mapping_for_id(id).unwrap_or_else(|| "(null)".to_string())
    ));
    t.p(format!(
        "padname [{}] type {} real {}",
        s(gamepad::gamepad_name_for_id(id)),
        gamepad::gamepad_type_for_id(id) as i32,
        gamepad::real_gamepad_type_for_id(id) as i32
    ));
    let pad = Gamepad::open(id).unwrap();
    t.p(format!(
        "name [{}] type {} label {} hasLT {} hasMisc1 {} touchpads {} fingers {} gyro {} rate {:.1} LT {}",
        s(pad.name()),
        pad.gamepad_type() as i32,
        pad.button_label(GamepadButton::South) as i32,
        pad.has_axis(GamepadAxis::LeftTrigger) as i32,
        pad.has_button(GamepadButton::Misc1) as i32,
        pad.num_touchpads(),
        pad.num_touchpad_fingers(0),
        pad.has_sensor(SensorType::Gyro) as i32,
        pad.sensor_data_rate(SensorType::Gyro),
        pad.axis(GamepadAxis::LeftTrigger)
    ));
    for b in pad.bindings().unwrap() {
        let (input_type, input) = match b.input {
            GamepadBindingInput::Button(button) => (1, [button, 0, 0]),
            GamepadBindingInput::Axis {
                axis,
                axis_min,
                axis_max,
            } => (2, [axis, axis_min, axis_max]),
            GamepadBindingInput::Hat { hat, hat_mask } => (3, [hat, hat_mask, 0]),
        };
        let (output_type, output) = match b.output {
            GamepadBindingOutput::Button(button) => (1, [button as i32, 0, 0]),
            GamepadBindingOutput::Axis {
                axis,
                axis_min,
                axis_max,
            } => (2, [axis as i32, axis_min, axis_max]),
        };
        t.p(format!(
            "bind in {input_type} [{} {} {}] out {output_type} [{} {} {}]",
            input[0], input[1], input[2], output[0], output[1], output[2]
        ));
    }
    t.dump("open pad");

    let j = pad.joystick().unwrap();
    j.set_virtual_button(0, true).unwrap();
    j.set_virtual_axis(4, 0).unwrap();
    j.set_virtual_axis(1, -20000).unwrap();
    j.set_virtual_hat(0, 1).unwrap_err();
    j.set_virtual_touchpad(0, 1, true, 0.25, 2.0, 0.5).unwrap();
    t.p(format!(
        "sensor on {}",
        pad.set_sensor_enabled(SensorType::Gyro, true).is_ok() as i32
    ));
    j.send_virtual_sensor_data(SensorType::Gyro, 77, &[1.0, 2.0, 3.0, 4.0])
        .unwrap();
    gamepad::update_gamepads();
    let (down, x, y, pressure) = pad.touchpad_finger(0, 1).unwrap();
    let mut data = [0.0f32; 4];
    pad.sensor_data(SensorType::Gyro, &mut data).unwrap();
    t.p(format!(
        "south {} LT {} LY {} finger {} {:.3} {:.3} {:.3} data {:.1} {:.1} {:.1} {:.1}",
        pad.button(GamepadButton::South) as i32,
        pad.axis(GamepadAxis::LeftTrigger),
        pad.axis(GamepadAxis::LeftY),
        down as i32,
        x,
        y,
        pressure,
        data[0],
        data[1],
        data[2],
        data[3]
    ));
    t.dump("update pad");

    j.set_virtual_touchpad(0, 1, false, 0.0, 0.0, 0.0).unwrap();
    j.set_virtual_button(0, false).unwrap();
    j.set_virtual_axis(4, 32767).unwrap();
    gamepad::update_gamepads();
    t.dump("update pad 2");

    let guid = joystick_guid_for_id(id);
    let remap = gamepad::add_gamepad_mapping(&format!(
        "{guid},Remapped,a:b1,b:b0,lefttrigger:+a4,leftx:-a0~,dpup:h0.1,"
    ));
    t.p(format!("remap {}", add_result(&remap)));
    t.p(format!(
        "remapped name [{}] east {} south {} LT {}",
        s(pad.name()),
        pad.button(GamepadButton::East) as i32,
        pad.button(GamepadButton::South) as i32,
        pad.axis(GamepadAxis::LeftTrigger)
    ));
    t.p(format!("mapping [{}]", pad.mapping().unwrap()));
    t.dump("remap");
    let remap = gamepad::add_gamepad_mapping(&format!(
        "{guid},Remapped2,a:b0,hint:!SDL_JOYTEST_NOT_SET:=0,"
    ));
    t.p(format!("remap hint {}", add_result(&remap)));
    t.p(format!("name [{}]", s(pad.name())));
    t.dump("remap hint");
    t.p(format!(
        "set mapping {}",
        gamepad::set_gamepad_mapping(id, None).is_ok() as i32
    ));
    t.p(format!(
        "name [{}] south {}",
        s(pad.name()),
        pad.button(GamepadButton::South) as i32
    ));
    t.dump("set mapping");

    drop(j);
    detach_virtual_joystick(id).unwrap();
    t.dump("detach pad");
    t.p(format!("connected {}", pad.connected() as i32));
    drop(pad);
    t.dump("close pad");

    // C: the mapping database
    for mapping in [
        "03000000aaaa0000bbbb000000000000,Test Pad,a:b0,b:b1,x:b2,y:b3,crc:1234,",
        "03001234aaaa0000bbbb000000000000,Test Pad CRC,a:b1,",
        "03000000aaaa0000bbbb000000000000,Test Pad Again,a:b5,",
        "03000000aaaa0000cccc000000000000,  Labels ,a:b0,b:b1,x:b2,y:b3,hint:SDL_GAMECONTROLLER_USE_BUTTON_LABELS:=1,",
        "03000000aaaa0000dddd000000000000,Cube,a:b0,b:b1,x:b2,y:b3,hint:SDL_GAMECONTROLLER_USE_GAMECUBE_LABELS:=1,",
        "030000007e0500003703000000000000,GC Adapter,a:b0,b:b1,x:b2,y:b3,",
        "03000000aaaa0000eeee000000000000,Hinted,a:b0,hint:SDL_JOYTEST_UNSET:=0,",
        "03000000aaaa0000eeee000000000000,Hinted On,a:b0,hint:!SDL_JOYTEST_UNSET:=0,",
        "nocomma",
        "03000000aaaa0000ffff000000000000,noname",
        "xinput,X,a:b0,",
        "05000000aaaa0000bbbb000001000000,Version,a:b0,platform:Linux,",
    ] {
        let result = gamepad::add_gamepad_mapping(mapping);
        t.p(format!("add {} [{mapping}] err=[{}]", add_result(&result), err(&result)));
    }
    for guid in [
        "03000000aaaa0000bbbb000000000000",
        "03001234aaaa0000bbbb000000000000",
        "03005678aaaa0000bbbb000000000000",
        "03000000aaaa0000cccc000000000000",
        "03000000aaaa0000dddd000000000000",
        "030000007e0500003703000000000000",
        "03000000aaaa0000eeee000000000000",
        "05000000aaaa0000bbbb000002000000",
        "03000000aaaa0000bbbb000002000000",
        "030000005e0400008e02000014010000",
        "05000000000000000000000000000000",
    ] {
        let result = gamepad::gamepad_mapping_for_guid(Guid::parse_lossy(guid));
        t.p(format!(
            "lookup {guid} -> [{}] err=[{}]",
            result.as_deref().unwrap_or("(null)"),
            err(&result)
        ));
    }
    let mappings = gamepad::gamepad_mappings();
    t.p(format!("mappings {}", mappings.len()));
    for name in [
        "xbox360",
        "+PS5",
        "-switchpro",
        "Steam",
        "bogus",
        "",
        "unknown",
    ] {
        let gamepad_type = GamepadType::from_string(name);
        t.p(format!(
            "type [{name}] {} [{}]",
            gamepad_type as i32,
            gamepad_type.as_str().unwrap_or("(null)")
        ));
    }
    t.p(format!(
        "button [{}] {} axis [{}] {}",
        GamepadButton::Misc6.as_str().unwrap(),
        GamepadButton::from_string("PADDLE2") as i32,
        GamepadAxis::RightTrigger.as_str().unwrap(),
        GamepadAxis::from_string("-lefty") as i32
    ));
    t.p(format!(
        "label {} {}",
        gamepad::gamepad_button_label_for_type(GamepadType::Ps4, GamepadButton::West) as i32,
        gamepad::gamepad_button_label_for_type(GamepadType::Gamecube, GamepadButton::East) as i32
    ));

    init::quit_subsystem(InitFlags::GAMEPAD);
    hints::reset(hints::GAMECONTROLLERCONFIG);
    (t.out, mappings)
}

#[cfg(target_os = "linux")]
#[test]
fn virtual_trace_matches_c() {
    let _l = crate::test_support::test_lock();
    init::set_main_ready();

    let (trace, mappings) = virtual_trace();

    let expected = include_str!("testdata/virtual_trace_linux.txt");
    for (i, (got, want)) in trace.lines().zip(expected.lines()).enumerate() {
        assert_eq!(got, want, "trace line {}", i + 1);
    }
    assert_eq!(trace.lines().count(), expected.lines().count());

    // The whole database, as SDL_GetGamepadMappings() lists it
    let mut all = String::new();
    for mapping in &mappings {
        let _ = writeln!(all, "{mapping}");
    }
    assert_eq!(
        (all.len(), crate::stdlib::crc32(0, all.as_bytes())),
        (79215, 0x2103d9ce)
    );
}

#[test]
fn virtual_joystick_without_gamepads() {
    let _l = crate::test_support::test_lock();
    init::set_main_ready();
    init::init_subsystem(InitFlags::JOYSTICK).unwrap();
    queue::flush_events(EventType::FIRST, EventType::LAST);

    let id = attach_virtual_joystick(VirtualJoystickDesc {
        joystick_type: JoystickType::Gamepad,
        naxes: 2,
        nbuttons: 2,
        ..VirtualJoystickDesc::default()
    })
    .unwrap();
    // The gamepad subsystem isn't running, so there's no gamepad event
    let events = queue::get_events(EventType::FIRST, EventType::LAST, usize::MAX).unwrap();
    assert_eq!(events.len(), 1);
    assert!(gamepad::is_gamepad(id));
    let j = Joystick::open(id).unwrap();
    // Two axes are assumed to be centered at zero
    assert_eq!(j.axis_initial_state(0).unwrap(), Some(0));
    assert_eq!(j.joystick_type(), JoystickType::Gamepad);
    assert!(Gamepad::open(id).is_ok());
    drop(j);

    // A device index past the end has no (valid) instance id
    {
        let _lock = lock_joysticks();
        let driver = &virtual_joystick::VIRTUAL_JOYSTICK_DRIVER;
        assert_eq!(driver.count(), 1);
        assert_eq!(driver.device_instance_id(0), id);
        assert_eq!(driver.device_instance_id(1), 0);
    }

    detach_virtual_joystick(id).unwrap();
    init::quit_subsystem(InitFlags::JOYSTICK);
    assert!(Joystick::open(id).is_err());
}

#[test]
fn driver_order() {
    // The platform drivers come before the virtual driver, as upstream lists them
    let virtual_driver: &dyn JoystickDriver = &virtual_joystick::VIRTUAL_JOYSTICK_DRIVER;
    assert!(std::ptr::addr_eq(
        JOYSTICK_DRIVERS[VIRTUAL_DRIVER_INDEX],
        virtual_driver
    ));
    #[cfg(windows)]
    {
        // RawInput, then Windows (DirectInput and XInput)
        let rawinput_driver: &dyn JoystickDriver = &windows::rawinput::RAWINPUT_JOYSTICK_DRIVER;
        assert!(std::ptr::addr_eq(
            JOYSTICK_DRIVERS[RAWINPUT_DRIVER_INDEX],
            rawinput_driver
        ));
        let windows_driver: &dyn JoystickDriver = &windows::WINDOWS_JOYSTICK_DRIVER;
        assert!(std::ptr::addr_eq(
            JOYSTICK_DRIVERS[WINDOWS_DRIVER_INDEX],
            windows_driver
        ));
        const {
            assert!(RAWINPUT_DRIVER_INDEX < WINDOWS_DRIVER_INDEX);
            assert!(WINDOWS_DRIVER_INDEX < VIRTUAL_DRIVER_INDEX);
        }
    }
    #[cfg(target_os = "linux")]
    {
        let linux_driver: &dyn JoystickDriver = &linux::LINUX_JOYSTICK_DRIVER;
        assert!(std::ptr::addr_eq(
            JOYSTICK_DRIVERS[LINUX_DRIVER_INDEX],
            linux_driver
        ));
    }
}
