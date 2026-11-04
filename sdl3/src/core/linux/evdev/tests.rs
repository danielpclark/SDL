// Tests of the console input reader: its device list (on device nodes that
// aren't input devices), and its event handling (keys, mice, touch slots)
// on events written into a pipe, as a device's node delivers them.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use super::*;
use crate::events::window::WindowFlags;
use crate::events::{Event, TouchFingerEvent};
use crate::init::{self, InitFlags};
use crate::video::Window;

#[test]
fn timestamps_follow_the_event_clock_and_never_run_ahead() {
    let event = |sec: i64, usec: i64| input_event {
        time: input_event_time {
            tv_sec: sec as _,
            tv_usec: usec as _,
        },
        ..input_event::default()
    };
    let a = get_event_timestamp(&event(100, 0));
    let b = get_event_timestamp(&event(100, 500));
    assert!(b >= a);
    assert!(b <= crate::timer::ticks_ns());
    // An event from the future is clamped to now, and pulls the offset back
    let c = get_event_timestamp(&event(1_000_000, 0));
    assert!(c <= crate::timer::ticks_ns());
    let d = get_event_timestamp(&event(1_000_000, 0));
    assert!(d >= c.saturating_sub(1_000_000_000));
}

/// The dummy video driver (a 1024x768 display) with a shown window, which
/// has the mouse and keyboard focus.
struct Session {
    _lock: std::sync::MutexGuard<'static, ()>,
    window: Window,
}

impl Session {
    fn new() -> Session {
        let lock = crate::test_support::test_lock();
        crate::hints::set(crate::hints::VIDEO_DRIVER, "dummy").unwrap();
        init::init(InitFlags::VIDEO).unwrap();
        // (the display's size, at its origin: the absolute mice stay in it)
        let window = Window::create("evdev", 1024, 768, WindowFlags::BORDERLESS).unwrap();
        window.set_position(0, 0).unwrap();
        window.start_text_input().unwrap();
        while crate::events::poll().is_some() {}
        Session {
            _lock: lock,
            window,
        }
    }

    fn events(&self) -> Vec<Event> {
        std::iter::from_fn(crate::events::poll).collect()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.window.destroy();
        init::quit();
        crate::hints::reset_all();
    }
}

fn reader() -> EvdevPrivateData {
    EvdevPrivateData {
        ref_count: 1,
        num_devices: 0,
        devices: Vec::new(),
        kbd: Some(EvdevKeyboardState::without_console()),
    }
}

#[test]
fn the_device_list() {
    let s = Session::new();
    let mut this = reader();
    let keyboard = DeviceClass::KEYBOARD | DeviceClass::HAS_KEYS;

    assert!(!device_added(&mut this, "/nonexistent/event0", keyboard));
    // (/dev/null answers none of the evdev ioctls: an empty name, no
    // relative axes)
    assert!(device_added(&mut this, "/dev/null", keyboard));
    assert!(!device_added(&mut this, "/dev/null", DeviceClass::MOUSE));
    assert!(device_added(&mut this, "/dev/zero", DeviceClass::MOUSE));
    // (a touchscreen has to have a name)
    assert!(!device_added(
        &mut this,
        "/dev/full",
        DeviceClass::TOUCHSCREEN
    ));
    assert_eq!(this.num_devices, 2);
    let paths: Vec<_> = this.devices.iter().map(|item| item.path.as_str()).collect();
    assert_eq!(paths, ["/dev/null", "/dev/zero"]);
    assert_eq!(device_count(&this, DeviceClass::KEYBOARD), 1);
    assert_eq!(device_count(&this, DeviceClass::MOUSE), 1);
    assert_eq!(device_count(&this, DeviceClass::HAS_KEYS), 1);
    assert_eq!(
        device_count(&this, DeviceClass::KEYBOARD | DeviceClass::MOUSE),
        0
    );
    assert!(!this.devices[0].relative_mouse);

    let keyboard_id = this.devices[0].id();
    let mouse_id = this.devices[1].id();
    assert!(crate::events::keyboard::keyboards().contains(&keyboard_id));
    assert_eq!(
        crate::events::keyboard::keyboard_name(keyboard_id).unwrap(),
        ""
    );
    assert!(mouse::mice().contains(&mouse_id));
    assert!(!crate::events::keyboard::keyboards().contains(&mouse_id));
    // A keyboard mutes the console's
    assert!(this.kbd.as_ref().unwrap().muted());

    assert!(!device_removed(&mut this, "/dev/full"));
    assert!(device_removed(&mut this, "/dev/null"));
    assert!(!crate::events::keyboard::keyboards().contains(&keyboard_id));
    assert!(!this.kbd.as_ref().unwrap().muted());
    assert_eq!(device_count(&this, DeviceClass::KEYBOARD), 0);
    assert!(device_removed(&mut this, "/dev/zero"));
    assert!(!mouse::mice().contains(&mouse_id));
    assert_eq!(this.num_devices, 0);
    assert!(this.devices.is_empty());
    let added_removed: Vec<_> = s
        .events()
        .into_iter()
        .filter_map(|e| match e {
            Event::KeyboardDevice(d) => Some(("keyboard", d.event_type, d.which)),
            Event::MouseDevice(d) => Some(("mouse", d.event_type, d.which)),
            _ => None,
        })
        .collect();
    assert_eq!(
        added_removed,
        [
            ("keyboard", EventType::KEYBOARD_ADDED, keyboard_id),
            ("mouse", EventType::MOUSE_ADDED, mouse_id),
            ("keyboard", EventType::KEYBOARD_REMOVED, keyboard_id),
            ("mouse", EventType::MOUSE_REMOVED, mouse_id),
        ]
    );
}

#[test]
fn the_udev_callback_takes_input_devices_but_joysticks() {
    let _s = Session::new();
    *THIS.lock().borrow_mut() = Some(reader());
    udev_callback(UdevDeviceEvent::Added, DeviceClass::SOUND, "/dev/null");
    udev_callback(
        UdevDeviceEvent::Added,
        DeviceClass::JOYSTICK | DeviceClass::HAS_KEYS,
        "/dev/null",
    );
    assert_eq!(get_device_count(DeviceClass::UNKNOWN), 0);
    udev_callback(UdevDeviceEvent::Added, DeviceClass::HAS_KEYS, "/dev/null");
    udev_callback(UdevDeviceEvent::Added, DeviceClass::MOUSE, "/dev/zero");
    assert_eq!(get_device_count(DeviceClass::UNKNOWN), 2);
    assert_eq!(get_device_count(DeviceClass::KEYBOARD), 0);
    // (a keyboard without the keyboard class isn't one)
    assert!(!THIS
        .lock()
        .borrow()
        .as_ref()
        .unwrap()
        .kbd
        .as_ref()
        .unwrap()
        .muted());
    udev_callback(UdevDeviceEvent::Removed, DeviceClass::UNKNOWN, "/dev/null");
    udev_callback(UdevDeviceEvent::Removed, DeviceClass::UNKNOWN, "/dev/zero");
    assert_eq!(get_device_count(DeviceClass::UNKNOWN), 0);
    *THIS.lock().borrow_mut() = None;
    assert_eq!(get_device_count(DeviceClass::UNKNOWN), 0);
    // (the state reentered: nothing happens)
    *THIS.lock().borrow_mut() = Some(reader());
    with_this(|_| {
        udev_callback(UdevDeviceEvent::Added, DeviceClass::MOUSE, "/dev/zero");
        poll();
        assert_eq!(get_device_count(DeviceClass::UNKNOWN), 0);
    });
    assert_eq!(get_device_count(DeviceClass::MOUSE), 0);
    *THIS.lock().borrow_mut() = None;
}

/// A device whose node is a pipe's read end: [`Pipe::send`] writes its
/// events.
struct Pipe {
    write: OwnedFd,
}

impl Pipe {
    /// A device of the class, its pipe and its ID.
    fn device(this: &mut EvdevPrivateData, udev_class: DeviceClass) -> (Pipe, u32) {
        let mut fds = [0; 2];
        // SAFETY: fds has room for the two descriptors.
        assert_eq!(
            unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) },
            0
        );
        // SAFETY: the descriptors were just made, and are owned from here on.
        let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        let item = EvdevListItem {
            path: format!("/dev/input/event-pipe{}", fds[0]),
            fd: read,
            udev_class,
            out_of_sync: false,
            is_touchscreen: false,
            touchscreen_data: None,
            high_res_wheel: false,
            high_res_hwheel: false,
            relative_mouse: false,
            mouse_x: 0,
            mouse_y: 0,
            mouse_wheel: 0,
            mouse_hwheel: 0,
            min_x: 0,
            max_x: 0,
            range_x: 0,
            min_y: 0,
            max_y: 0,
            range_y: 0,
        };
        let id = item.id();
        // (as device_added() does)
        if udev_class.intersects(DeviceClass::MOUSE) {
            mouse::add_mouse(id, Some("pipe mouse"));
        }
        if udev_class.intersects(DeviceClass::KEYBOARD) {
            add_keyboard(id, Some("pipe keyboard"));
        }
        this.devices.push(item);
        this.num_devices += 1;
        (Pipe { write }, id)
    }

    fn send(&self, events: &[(u16, usize, i32)]) {
        let events: Vec<input_event> = events
            .iter()
            .map(|&(type_, code, value)| input_event {
                type_,
                code: code as u16,
                value,
                ..input_event::default()
            })
            .collect();
        let size = std::mem::size_of_val(events.as_slice());
        // SAFETY: the buffer is size bytes of plain data.
        let written = unsafe { libc::write(self.write.as_raw_fd(), events.as_ptr().cast(), size) };
        assert_eq!(written, size as isize);
    }
}

const SYN: (u16, usize, i32) = (EV_SYN, SYN_REPORT as usize, 0);
const KEY_A: usize = 30;
const KEY_LEFTSHIFT: usize = 42;

#[test]
fn keys_and_their_text() {
    let s = Session::new();
    let mut this = reader();
    let (pipe, id) = Pipe::device(&mut this, DeviceClass::KEYBOARD | DeviceClass::HAS_KEYS);
    pipe.send(&[
        (EV_KEY, KEY_A, 1),
        SYN,
        (EV_KEY, KEY_A, 2),
        SYN,
        (EV_KEY, KEY_A, 0),
        (EV_KEY, KEY_LEFTSHIFT, 1),
        (EV_KEY, KEY_A, 1),
        (EV_KEY, KEY_A, 0),
        (EV_KEY, KEY_LEFTSHIFT, 0),
        SYN,
        // (an unknown key code, and a value that's neither press, repeat
        // nor release)
        (EV_KEY, 0x2ff, 1),
        (EV_KEY, KEY_A, 3),
    ]);
    poll_locked(&mut this);
    let events: Vec<String> = s
        .events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Key(k) => {
                assert_eq!(k.which, id);
                Some(format!(
                    "{} {} raw {}",
                    k.scancode.0,
                    if k.down { "down" } else { "up" },
                    k.raw
                ))
            }
            Event::TextInput(t) => Some(format!("text {:?}", t.text)),
            _ => None,
        })
        .collect();
    assert_eq!(
        events,
        [
            "4 down raw 30",
            "text \"a\"",
            "4 down raw 30",
            "text \"a\"",
            "4 up raw 30",
            "225 down raw 42",
            "4 down raw 30",
            "text \"A\"",
            "4 up raw 30",
            "225 up raw 42",
            "0 down raw 767",
            // (SDL_EVDEV_kbd_keycode() takes any value but 0 as a press)
            "text \"a\"",
        ]
    );
}

#[test]
fn mice_relative_and_absolute() {
    let s = Session::new();
    let mut this = reader();
    let (rel, rel_id) = Pipe::device(&mut this, DeviceClass::MOUSE);
    this.devices[0].relative_mouse = true;
    this.devices[0].high_res_wheel = true;
    let (abs, abs_id) = Pipe::device(&mut this, DeviceClass::MOUSE);
    this.devices[1].min_x = 0;
    this.devices[1].range_x = 32767;
    this.devices[1].min_y = 100;
    this.devices[1].range_y = 1000;

    rel.send(&[
        (EV_REL, usize::from(REL_X), 5),
        (EV_REL, usize::from(REL_Y), -3),
        (EV_REL, usize::from(REL_X), 2),
        // (the wheel's low-resolution events are ignored with the
        // high-resolution ones)
        (EV_REL, usize::from(REL_WHEEL), 1),
        (EV_REL, usize::from(REL_WHEEL_HI_RES), 60),
        (EV_REL, usize::from(REL_HWHEEL), -2),
        SYN,
        (EV_KEY, BTN_LEFT, 1),
        (EV_KEY, BTN_LEFT, 0),
        (EV_KEY, BTN_TASK, 1),
        SYN,
        // (nothing moved: no motion)
        SYN,
    ]);
    abs.send(&[
        (EV_ABS, ABS_X, 16384),
        (EV_ABS, ABS_Y, 600),
        SYN,
        (EV_ABS, ABS_X, 30000),
        SYN,
    ]);
    poll_locked(&mut this);
    let events: Vec<String> = s
        .events()
        .into_iter()
        .filter_map(|e| match e {
            // (whose mouse is the global one, outside the relative mode)
            Event::MouseMotion(m) => {
                assert_eq!(m.which, mouse::GLOBAL_MOUSE_ID);
                Some(format!(
                    "motion at ({}, {}) rel ({}, {})",
                    m.x, m.y, m.xrel, m.yrel
                ))
            }
            Event::MouseWheel(w) => Some(format!("wheel ({}, {})", w.x, w.y)),
            Event::MouseButton(b) => Some(format!("button {} {}", b.button, b.down)),
            _ => None,
        })
        .collect();
    // (the relative mouse from the window's origin, clamped to it; the
    // absolute one on the window's display: x (16384 - 0) * 1024 / 32767,
    // y (600 - 100) * 768 / 1000)
    let x1 = 16384.0f32 * 1024.0 / 32767.0;
    let x2 = 30000.0f32 * 1024.0 / 32767.0;
    assert_eq!(
        events,
        [
            "motion at (7, 0) rel (7, -3)".to_owned(),
            "wheel (-2, 0.5)".to_owned(),
            "button 1 true".to_owned(),
            "button 1 false".to_owned(),
            "button 8 true".to_owned(),
            format!("motion at ({x1}, 384) rel ({}, 384)", x1 - 7.0),
            format!("motion at ({x2}, 384) rel ({}, 0)", x2 - x1),
        ]
    );
    let _ = (rel_id, abs_id);
}

/// A multitouch screen of `slots` slots: x and y 0..1000, pressure
/// 0..255.
fn touchscreen(this: &mut EvdevPrivateData, slots: c_int) -> (Pipe, u32) {
    let (pipe, id) = Pipe::device(this, DeviceClass::TOUCHSCREEN);
    let item = this.devices.last_mut().unwrap();
    item.is_touchscreen = true;
    item.touchscreen_data = Some(Box::new(TouchscreenData {
        name: "pipe touchscreen".into(),
        max_x: 1000,
        range_x: 1000,
        max_y: 1000,
        range_y: 1000,
        max_pressure: 255,
        range_pressure: 255,
        max_slots: slots,
        slots: vec![TouchSlot::default(); slots as usize],
        ..TouchscreenData::default()
    }));
    touch::add_touch(
        id as touch::TouchID,
        TouchDeviceType::Direct,
        "pipe touchscreen",
    );
    (pipe, id)
}

fn fingers(s: &Session) -> Vec<String> {
    s.events()
        .into_iter()
        .filter_map(|e| match e {
            Event::TouchFinger(TouchFingerEvent {
                event_type,
                finger_id,
                x,
                y,
                pressure,
                ..
            }) => Some(format!(
                "{} {finger_id} {x} {y} {pressure:.2}",
                match event_type {
                    EventType::FINGER_DOWN => "down",
                    EventType::FINGER_UP => "up",
                    _ => "motion",
                }
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn touch_slots() {
    let s = Session::new();
    let mut this = reader();
    let (pipe, _) = touchscreen(&mut this, 2);
    pipe.send(&[
        (EV_ABS, ABS_MT_SLOT, 0),
        (EV_ABS, ABS_MT_TRACKING_ID, 5),
        (EV_ABS, ABS_MT_POSITION_X, 100),
        (EV_ABS, ABS_MT_POSITION_Y, 200),
        (EV_ABS, ABS_MT_PRESSURE, 255),
        SYN,
    ]);
    poll_locked(&mut this);
    // (the finger is the tracking ID + 1)
    assert_eq!(fingers(&s), ["down 6 0.1 0.2 1.00"]);
    assert_eq!(this.devices[0].touch().slots[0].tracking_id, 6);
    pipe.send(&[
        (EV_ABS, ABS_MT_SLOT, 1),
        (EV_ABS, ABS_MT_TRACKING_ID, 7),
        (EV_ABS, ABS_MT_POSITION_X, 900),
        (EV_ABS, ABS_MT_POSITION_Y, 800),
        SYN,
        (EV_ABS, ABS_MT_SLOT, 0),
        (EV_ABS, ABS_MT_POSITION_X, 150),
        SYN,
        // (a slot past the device's: ignored)
        (EV_ABS, ABS_MT_SLOT, 5),
        (EV_ABS, ABS_MT_POSITION_X, 300),
        SYN,
        (EV_ABS, ABS_MT_SLOT, 0),
        (EV_ABS, ABS_MT_TRACKING_ID, -1),
        (EV_ABS, ABS_MT_SLOT, 1),
        (EV_ABS, ABS_MT_TRACKING_ID, -1),
        SYN,
    ]);
    poll_locked(&mut this);
    assert_eq!(
        fingers(&s),
        [
            "down 8 0.9 0.8 0.00",
            "motion 6 0.15 0.2 1.00",
            "up 6 0.15 0.2 1.00",
            "up 8 0.9 0.8 0.00"
        ]
    );
    let slots = &this.devices[0].touch().slots;
    assert_eq!((slots[0].tracking_id, slots[1].tracking_id), (0, 0));
    assert!(slots
        .iter()
        .all(|slot| slot.delta == EvdevTouchSlotDelta::None));

    // SYN_DROPPED: the device is out of sync (and resynchronized from the
    // slots it reports, which a pipe doesn't) until the next SYN_REPORT;
    // meanwhile, another SYN event ends the read's events
    pipe.send(&[
        (EV_SYN, usize::from(SYN_DROPPED), 0),
        (EV_ABS, ABS_MT_SLOT, 0),
        (EV_ABS, ABS_MT_TRACKING_ID, 9),
        (EV_SYN, usize::from(SYN_DROPPED), 0),
        (EV_ABS, ABS_MT_POSITION_X, 500),
        SYN,
    ]);
    poll_locked(&mut this);
    assert!(this.devices[0].out_of_sync);
    assert_eq!(fingers(&s), Vec::<String>::new());
    pipe.send(&[SYN]);
    poll_locked(&mut this);
    assert!(!this.devices[0].out_of_sync);
    assert_eq!(fingers(&s), ["down 10 0.15 0.2 1.00"]);
}

#[test]
fn single_touch_and_no_pressure() {
    let s = Session::new();
    let mut this = reader();
    let (pipe, _) = touchscreen(&mut this, 1);
    this.devices[0].touch().range_pressure = 0;
    pipe.send(&[
        (EV_KEY, BTN_TOUCH, 1),
        (EV_ABS, ABS_X, 250),
        (EV_ABS, ABS_Y, 750),
    ]);
    poll_locked(&mut this);
    let slot = this.devices[0].touch().slots[0];
    assert_eq!(
        (slot.delta, slot.x, slot.y, slot.tracking_id),
        (EvdevTouchSlotDelta::Down, 250, 750, 0)
    );
    // (no motion: only the multitouch positions make it)
    pipe.send(&[(EV_ABS, ABS_MT_POSITION_X, 500), (EV_ABS, ABS_X, 500)]);
    poll_locked(&mut this);
    let slot = this.devices[0].touch().slots[0];
    assert_eq!((slot.delta, slot.x), (EvdevTouchSlotDelta::Down, 500));
    // FIXME (upstream): the single slot's tracking ID stays 0 (see
    // handle_event()), and SDL_AddFinger() asserts against finger 0: only
    // a build without debug assertions sends the touch (as finger 0)
    #[cfg(not(debug_assertions))]
    {
        pipe.send(&[SYN, (EV_KEY, BTN_TOUCH, 0), SYN]);
        poll_locked(&mut this);
        assert_eq!(fingers(&s), ["down 0 0.5 0.75 1.00", "up 0 0.5 0.75 1.00"]);
    }
    #[cfg(debug_assertions)]
    let _ = &s;
}
