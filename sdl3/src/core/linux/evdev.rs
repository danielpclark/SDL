// Rust translation of src/core/linux/SDL_evdev.c and SDL_evdev.h from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The console input reader: keyboards, mice and touch devices read
//! straight from their evdev nodes (`/dev/input/event*`), found and
//! followed through libudev, for the video drivers without a window
//! system (KMS/DRM, and the dummy driver's `evdev` variant). The console
//! keyboard's text and virtual console switching are
//! [`super::evdev_kbd`]'s. Also the event timestamps the joystick driver
//! shares.
//!
//! This is based on the linux joystick driver
//! References: <https://www.kernel.org/doc/Documentation/input/input.txt>
//!             <https://www.kernel.org/doc/Documentation/input/event-codes.txt>
//!             /usr/include/linux/input.h
//!             The evtest application is also useful to debug the protocol
//!
//! Upstream's udev build (`SDL_USE_LIBUDEV`): without libudev it reads the
//! devices `SDL_HINT_EVDEV_DEVICES` lists, a branch not translated, as
//! libudev is always loaded here.

use std::cell::RefCell;
use std::ffi::{c_int, c_uint, c_ulong, CString};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::evdev_capabilities::{nbits, test_bit, DeviceClass};
use super::evdev_kbd::{EvdevKeyboardState, VtSwitchCallback};
use super::input::*;
use super::udev::{self, UdevDeviceEvent};
use crate::error::Result;
use crate::events::keyboard::{add_keyboard, remove_keyboard, send_keyboard_key, Scancode};
use crate::events::mouse::{self, MouseWheelDirection};
use crate::events::scancode_tables::{get_scancode_from_table, ScancodeTable};
use crate::events::touch::{self, TouchDeviceType};
use crate::events::EventType;
use crate::thread::ReentrantMutex;

/// What changed in a touch slot since the last `SYN_REPORT`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum EvdevTouchSlotDelta {
    /// `EVDEV_TOUCH_SLOTDELTA_NONE`
    #[default]
    None,
    /// `EVDEV_TOUCH_SLOTDELTA_DOWN`
    Down,
    /// `EVDEV_TOUCH_SLOTDELTA_UP`
    Up,
    /// `EVDEV_TOUCH_SLOTDELTA_MOVE`
    Move,
}

/// A touch slot (`touchscreen_data->slots[i]`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TouchSlot {
    delta: EvdevTouchSlotDelta,
    tracking_id: c_int,
    x: c_int,
    y: c_int,
    pressure: c_int,
}

/// `SDL_evdevlist_item.touchscreen_data`
#[derive(Debug, Default)]
struct TouchscreenData {
    name: String,

    min_x: c_int,
    max_x: c_int,
    range_x: c_int,
    min_y: c_int,
    max_y: c_int,
    range_y: c_int,
    min_pressure: c_int,
    max_pressure: c_int,
    range_pressure: c_int,

    max_slots: c_int,
    current_slot: c_int,
    slots: Vec<TouchSlot>,
}

impl TouchscreenData {
    /// `&slots[current_slot]`. Note (upstream): a slot number past
    /// `max_slots` from the device writes past the slots; here it's
    /// ignored.
    fn current(&mut self) -> Option<&mut TouchSlot> {
        let slot = usize::try_from(self.current_slot).ok()?;
        self.slots.get_mut(slot)
    }
}

/// An open device. Translation of `SDL_evdevlist_item`.
#[derive(Debug)]
struct EvdevListItem {
    path: String,
    fd: OwnedFd,
    udev_class: DeviceClass,

    // TODO: use this for every device, not just touchscreen
    out_of_sync: bool,

    /* TODO: expand on this to have data for every possible class (mouse,
    keyboard, touchpad, etc.). Also there's probably some things in here we
    can pull out to the SDL_evdevlist_item i.e. name */
    is_touchscreen: bool,
    touchscreen_data: Option<Box<TouchscreenData>>,

    // Mouse state
    high_res_wheel: bool,
    high_res_hwheel: bool,
    relative_mouse: bool,
    mouse_x: c_int,
    mouse_y: c_int,
    mouse_wheel: c_int,
    mouse_hwheel: c_int,
    min_x: c_int,
    max_x: c_int,
    range_x: c_int,
    min_y: c_int,
    max_y: c_int,
    range_y: c_int,
}

impl EvdevListItem {
    fn fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    /// The device's ID as a keyboard, mouse and touch device: its file
    /// descriptor ("I guess our fd is unique enough").
    fn id(&self) -> u32 {
        self.fd() as u32
    }

    /// `item->touchscreen_data` of a touchscreen.
    fn touch(&mut self) -> &mut TouchscreenData {
        self.touchscreen_data
            .as_deref_mut()
            .expect("a touchscreen has its data")
    }
}

/// Translation of `SDL_EVDEV_PrivateData`.
struct EvdevPrivateData {
    ref_count: c_int,
    num_devices: c_int,
    // (upstream's first/last list, in the order the devices were added)
    devices: Vec<EvdevListItem>,
    // (None until SDL_EVDEV_Init() opens it, after the first scan)
    kbd: Option<EvdevKeyboardState>,
}

/// Translation of `static SDL_EVDEV_PrivateData *_this`. Reentrant, as the
/// events the reader sends reach the application's event watchers, which
/// may pump events: a nested call finds the state borrowed and does
/// nothing (Note: upstream reenters the reader).
static THIS: ReentrantMutex<RefCell<Option<EvdevPrivateData>>> =
    ReentrantMutex::new(RefCell::new(None));

/// Run `f` on the reader's state, unless a caller up the stack holds it
/// (`None` then).
fn with_this<R>(f: impl FnOnce(&mut Option<EvdevPrivateData>) -> R) -> Option<R> {
    let guard = THIS.lock();
    let mut this = guard.try_borrow_mut().ok()?;
    Some(f(&mut this))
}

static EVDEV_MOUSE_BUTTONS: [u8; 8] = [
    mouse::BUTTON_LEFT,   // BTN_LEFT        0x110
    mouse::BUTTON_RIGHT,  // BTN_RIGHT       0x111
    mouse::BUTTON_MIDDLE, // BTN_MIDDLE      0x112
    mouse::BUTTON_X1,     // BTN_SIDE        0x113
    mouse::BUTTON_X2,     // BTN_EXTRA       0x114
    mouse::BUTTON_X2 + 1, // BTN_FORWARD     0x115
    mouse::BUTTON_X2 + 2, // BTN_BACK        0x116
    mouse::BUTTON_X2 + 3, // BTN_TASK        0x117
];

/// Translation of `SDL_EVDEV_SetRelativeMouseMode()`.
fn sdl_evdev_set_relative_mouse_mode(_enabled: bool) -> Result<()> {
    // Mice already send relative events through this interface
    Ok(())
}

/// Translation of `SDL_EVDEV_UpdateKeyboardMute()`.
fn update_keyboard_mute(this: &mut EvdevPrivateData) {
    let muted = device_count(this, DeviceClass::KEYBOARD) > 0;
    if let Some(kbd) = &mut this.kbd {
        kbd.set_muted(muted);
    }
}

/// Start reading the input devices (counted: each call needs a [`quit`]).
/// Translation of `SDL_EVDEV_Init()`.
pub(crate) fn init() -> Result<()> {
    let created = with_this(|this| {
        if this.is_some() {
            return false;
        }
        *this = Some(EvdevPrivateData {
            ref_count: 0,
            num_devices: 0,
            devices: Vec::new(),
            kbd: None,
        });
        true
    })
    .unwrap_or(false);

    if created {
        let unwind = |e| {
            with_this(|this| *this = None);
            e
        };
        udev::init().map_err(unwind)?;

        // Set up the udev callback
        if let Err(e) = udev::add_callback(udev_callback) {
            udev::quit();
            return Err(unwind(e));
        }

        // Force a scan to build the initial device list
        let _ = udev::scan();

        with_this(|this| {
            if let Some(this) = this {
                this.kbd = Some(EvdevKeyboardState::init());

                update_keyboard_mute(this);
            }
        });
    }

    mouse::with_mouse(|mouse| {
        mouse.set_relative_mouse_mode_hook = Some(sdl_evdev_set_relative_mouse_mode)
    });

    with_this(|this| {
        if let Some(this) = this {
            this.ref_count += 1;
        }
    });

    Ok(())
}

/// Release a reference taken by [`init`], closing the devices and
/// restoring the console with the last one. Translation of
/// `SDL_EVDEV_Quit()`.
pub(crate) fn quit() {
    let last = with_this(|this| {
        let Some(data) = this else {
            return false;
        };
        data.ref_count -= 1;
        data.ref_count < 1
    })
    .unwrap_or(false);

    if last {
        udev::del_callback(udev_callback);
        udev::quit();

        with_this(|this| {
            let Some(data) = this else {
                return;
            };
            // Remove existing devices
            while let Some(path) = data.devices.first().map(|item| item.path.clone()) {
                device_removed(data, &path);
            }

            if let Some(kbd) = data.kbd.take() {
                kbd.quit();
            }

            crate::sdl_assert!(data.devices.is_empty());
            crate::sdl_assert!(data.num_devices == 0);

            *this = None;
        });
    }
}

/// Translation of `SDL_EVDEV_udev_callback()`.
fn udev_callback(udev_event: UdevDeviceEvent, udev_class: DeviceClass, dev_path: &str) {
    with_this(|this| {
        let Some(this) = this else {
            return;
        };
        match udev_event {
            UdevDeviceEvent::Added => {
                if !udev_class.intersects(
                    DeviceClass::MOUSE
                        | DeviceClass::HAS_KEYS
                        | DeviceClass::TOUCHSCREEN
                        | DeviceClass::TOUCHPAD,
                ) {
                    return;
                }

                if udev_class.intersects(DeviceClass::JOYSTICK) {
                    return;
                }

                // FIXME (upstream): a mouse's mousedev node
                // (/dev/input/mouseN) comes here too, with its device's
                // properties, and is opened and read as an evdev node (its
                // PS/2 packets taken for input_events), as an extra mouse
                // without a name; it's root's only, as the event nodes are.
                device_added(this, dev_path, udev_class);
            }
            UdevDeviceEvent::Removed => {
                device_removed(this, dev_path);
            }
        }
    });
}

/// Set the handlers of virtual console switches (see
/// [`EvdevKeyboardState::set_vt_switch_callbacks`]). Translation of
/// `SDL_EVDEV_SetVTSwitchCallbacks()`.
pub(crate) fn set_vt_switch_callbacks(
    release_callback: Option<VtSwitchCallback>,
    acquire_callback: Option<VtSwitchCallback>,
) {
    // Note (upstream): this dereferences _this, NULL when SDL_EVDEV_Init()
    // failed (as KMSDRM_VideoInit() ignores); here nothing is set then.
    with_this(|this| {
        if let Some(kbd) = this.as_mut().and_then(|this| this.kbd.as_mut()) {
            kbd.set_vt_switch_callbacks(release_callback, acquire_callback);
        }
    });
}

/// `SDL_EVDEV_GetDeviceCount()` on the locked state.
fn device_count(this: &EvdevPrivateData, device_class: DeviceClass) -> c_int {
    this.devices
        .iter()
        .filter(|item| (item.udev_class & device_class) == device_class)
        .count() as c_int
}

/// The number of open devices of all the classes `device_class` has.
/// Translation of `SDL_EVDEV_GetDeviceCount()`.
pub(crate) fn get_device_count(device_class: DeviceClass) -> c_int {
    // Note (upstream): this dereferences _this, NULL when the reader isn't
    // initialized; here the count is 0 then.
    with_this(|this| {
        this.as_ref()
            .map_or(0, |this| device_count(this, device_class))
    })
    .unwrap_or(0)
}

/// Read the devices' events and send them. Translation of
/// `SDL_EVDEV_Poll()`.
pub(crate) fn poll() {
    with_this(|this| {
        if let Some(this) = this {
            poll_locked(this);
        }
    });
}

fn poll_locked(this: &mut EvdevPrivateData) {
    let mut events = [input_event::default(); 32];

    if let Some(kbd) = &mut this.kbd {
        kbd.update();
    }

    for index in 0..this.devices.len() {
        loop {
            // SAFETY: the buffer is writable and as long as given;
            // input_event is plain data.
            let len = unsafe {
                libc::read(
                    this.devices[index].fd(),
                    events.as_mut_ptr().cast(),
                    std::mem::size_of_val(&events),
                )
            };
            if len <= 0 {
                break;
            }
            let len = len as usize / size_of::<input_event>();
            for event in &events[..len] {
                let item = &this.devices[index];
                /* special handling for touchscreen, that should eventually be
                used for all devices */
                if item.out_of_sync
                    && item.is_touchscreen
                    && event.type_ == EV_SYN
                    && event.code != SYN_REPORT
                {
                    break;
                }

                handle_event(this, index, event);
            }
        }
    }
}

/// The body of `SDL_EVDEV_Poll()`'s event loop: one event of a device.
fn handle_event(this: &mut EvdevPrivateData, index: usize, event: &input_event) {
    let item = &mut this.devices[index];

    match event.type_ {
        EV_KEY => {
            let code = usize::from(event.code);
            if code >= BTN_MOUSE && code < BTN_MOUSE + EVDEV_MOUSE_BUTTONS.len() {
                let timestamp = get_event_timestamp(event);
                let mouse_button = code - BTN_MOUSE;
                mouse::send_mouse_button(
                    Duration::from_nanos(timestamp),
                    mouse::mouse_focus(),
                    item.id(),
                    EVDEV_MOUSE_BUTTONS[mouse_button],
                    event.value != 0,
                );
                return;
            }

            /* BTN_TOUCH event value 1 indicates there is contact with
            a touchscreen or trackpad (earliest finger's current
            position is sent in EV_ABS ABS_X/ABS_Y, switching to
            next finger after earliest is released) */
            if item.is_touchscreen && code == BTN_TOUCH {
                // FIXME (upstream): nothing gives the single slot a
                // tracking ID, so its touches are finger 0, which
                // SDL_AddFinger() asserts against (a build with debug
                // assertions stops there; one without tracks finger 0).
                let touch = item.touch();
                if touch.max_slots == 1 {
                    touch.slots[0].delta = if event.value != 0 {
                        EvdevTouchSlotDelta::Down
                    } else {
                        EvdevTouchSlotDelta::Up
                    };
                }
                return;
            }

            // Probably keyboard
            {
                let timestamp = Duration::from_nanos(get_event_timestamp(event));
                let scancode = translate_keycode(c_int::from(event.code));
                if event.value == 0 {
                    send_keyboard_key(
                        timestamp,
                        item.id(),
                        c_int::from(event.code),
                        scancode,
                        false,
                    );
                } else if event.value == 1 || event.value == 2
                /* key repeated */
                {
                    send_keyboard_key(
                        timestamp,
                        item.id(),
                        c_int::from(event.code),
                        scancode,
                        true,
                    );
                }
                if let Some(kbd) = &mut this.kbd {
                    kbd.keycode(c_uint::from(event.code), event.value);
                }
            }
        }
        EV_ABS => match usize::from(event.code) {
            ABS_MT_SLOT => {
                if !item.is_touchscreen {
                    // FIXME: temp hack
                    return;
                }
                item.touch().current_slot = event.value;
            }
            ABS_MT_TRACKING_ID => {
                if !item.is_touchscreen {
                    // FIXME: temp hack
                    return;
                }
                if let Some(slot) = item.touch().current() {
                    if event.value >= 0 {
                        slot.tracking_id = event.value.wrapping_add(1);
                        slot.delta = EvdevTouchSlotDelta::Down;
                    } else {
                        slot.delta = EvdevTouchSlotDelta::Up;
                    }
                }
            }
            code @ (ABS_MT_POSITION_X | ABS_MT_POSITION_Y | ABS_MT_PRESSURE) => {
                if !item.is_touchscreen {
                    // FIXME: temp hack
                    return;
                }
                if let Some(slot) = item.touch().current() {
                    match code {
                        ABS_MT_POSITION_X => slot.x = event.value,
                        ABS_MT_POSITION_Y => slot.y = event.value,
                        _ => slot.pressure = event.value,
                    }
                    if slot.delta == EvdevTouchSlotDelta::None {
                        slot.delta = EvdevTouchSlotDelta::Move;
                    }
                }
            }
            ABS_X => {
                if item.is_touchscreen {
                    let touch = item.touch();
                    if touch.max_slots != 1 {
                        return;
                    }
                    touch.slots[0].x = event.value;
                } else if !item.relative_mouse {
                    item.mouse_x = event.value;
                }
            }
            ABS_Y => {
                if item.is_touchscreen {
                    let touch = item.touch();
                    if touch.max_slots != 1 {
                        return;
                    }
                    touch.slots[0].y = event.value;
                } else if !item.relative_mouse {
                    item.mouse_y = event.value;
                }
            }
            _ => {}
        },
        EV_REL => match event.code {
            REL_X => {
                if item.relative_mouse {
                    item.mouse_x = item.mouse_x.wrapping_add(event.value);
                }
            }
            REL_Y => {
                if item.relative_mouse {
                    item.mouse_y = item.mouse_y.wrapping_add(event.value);
                }
            }
            REL_WHEEL => {
                if !item.high_res_wheel {
                    item.mouse_wheel = item.mouse_wheel.wrapping_add(event.value);
                }
            }
            REL_WHEEL_HI_RES => {
                crate::sdl_assert!(item.high_res_wheel);
                item.mouse_wheel = item.mouse_wheel.wrapping_add(event.value);
            }
            REL_HWHEEL => {
                if !item.high_res_hwheel {
                    item.mouse_hwheel = item.mouse_hwheel.wrapping_add(event.value);
                }
            }
            REL_HWHEEL_HI_RES => {
                crate::sdl_assert!(item.high_res_hwheel);
                item.mouse_hwheel = item.mouse_hwheel.wrapping_add(event.value);
            }
            _ => {}
        },
        EV_SYN => match event.code {
            SYN_REPORT => syn_report(item, event),
            SYN_DROPPED => {
                if item.is_touchscreen {
                    item.out_of_sync = true;
                }
                sync_device(item);
            }
            _ => {}
        },
        _ => {}
    }
}

/// `SDL_EVDEV_Poll()`'s `SYN_REPORT`: a device's changes since the last
/// (the mouse's focus read at each send, as upstream reads `mouse->focus`).
fn syn_report(item: &mut EvdevListItem, event: &input_event) {
    // Send mouse axis changes together to ensure consistency and reduce event processing overhead
    if item.relative_mouse {
        if item.mouse_x != 0 || item.mouse_y != 0 {
            let timestamp = get_event_timestamp(event);
            mouse::send_mouse_motion(
                Duration::from_nanos(timestamp),
                mouse::mouse_focus(),
                item.id(),
                item.relative_mouse,
                item.mouse_x as f32,
                item.mouse_y as f32,
            );
            item.mouse_x = 0;
            item.mouse_y = 0;
        }
    } else if item.range_x > 0 && item.range_y > 0 {
        let mut screen_w = 0;
        let mut screen_h = 0;
        let mut mode = None;

        if let Some(focus) = mouse::mouse_focus() {
            mode = crate::video::window::display_for_window(focus)
                .and_then(crate::video::display::current_display_mode)
                .ok();
        }
        if mode.is_none() {
            mode = crate::video::display::primary_display()
                .and_then(crate::video::display::current_display_mode)
                .ok();
        }
        if let Some(mode) = mode {
            screen_w = mode.w;
            screen_h = mode.h;
        }
        mouse::send_mouse_motion(
            Duration::from_nanos(get_event_timestamp(event)),
            mouse::mouse_focus(),
            item.id(),
            item.relative_mouse,
            (item.mouse_x.wrapping_sub(item.min_x)) as f32 * screen_w as f32 / item.range_x as f32,
            (item.mouse_y.wrapping_sub(item.min_y)) as f32 * screen_h as f32 / item.range_y as f32,
        );
    }

    if item.mouse_wheel != 0 || item.mouse_hwheel != 0 {
        let timestamp = get_event_timestamp(event);
        mouse::send_mouse_wheel(
            Duration::from_nanos(timestamp),
            mouse::mouse_focus(),
            item.id(),
            item.mouse_hwheel as f32 / if item.high_res_hwheel { 120.0 } else { 1.0 },
            item.mouse_wheel as f32 / if item.high_res_wheel { 120.0 } else { 1.0 },
            MouseWheelDirection::Normal,
        );
        item.mouse_wheel = 0;
        item.mouse_hwheel = 0;
    }

    if !item.is_touchscreen {
        // FIXME: temp hack
        return;
    }

    let fd = item.fd();
    let touch = item.touch();
    for j in 0..touch.slots.len() {
        let slot = touch.slots[j];
        let norm_x = (slot.x.wrapping_sub(touch.min_x)) as f32 / touch.range_x as f32;
        let norm_y = (slot.y.wrapping_sub(touch.min_y)) as f32 / touch.range_y as f32;

        let norm_pressure = if touch.range_pressure > 0 {
            (slot.pressure.wrapping_sub(touch.min_pressure)) as f32 / touch.range_pressure as f32
        } else {
            // This touchscreen does not support pressure
            1.0
        };

        /* FIXME: the touch's window shouldn't be null, but
         * the coordinate space of touch positions needs to
         * be window-relative in that case. */
        match slot.delta {
            EvdevTouchSlotDelta::Down => {
                touch::send_touch(
                    Duration::from_nanos(get_event_timestamp(event)),
                    fd as touch::TouchID,
                    slot.tracking_id as touch::FingerID,
                    mouse::mouse_focus(),
                    EventType::FINGER_DOWN,
                    norm_x,
                    norm_y,
                    norm_pressure,
                );
                touch.slots[j].delta = EvdevTouchSlotDelta::None;
            }
            EvdevTouchSlotDelta::Up => {
                touch::send_touch(
                    Duration::from_nanos(get_event_timestamp(event)),
                    fd as touch::TouchID,
                    slot.tracking_id as touch::FingerID,
                    mouse::mouse_focus(),
                    EventType::FINGER_UP,
                    norm_x,
                    norm_y,
                    norm_pressure,
                );
                touch.slots[j].tracking_id = 0;
                touch.slots[j].delta = EvdevTouchSlotDelta::None;
            }
            EvdevTouchSlotDelta::Move => {
                touch::send_touch_motion(
                    Duration::from_nanos(get_event_timestamp(event)),
                    fd as touch::TouchID,
                    slot.tracking_id as touch::FingerID,
                    mouse::mouse_focus(),
                    norm_x,
                    norm_y,
                    norm_pressure,
                );
                touch.slots[j].delta = EvdevTouchSlotDelta::None;
            }
            EvdevTouchSlotDelta::None => {}
        }
    }

    if item.out_of_sync {
        item.out_of_sync = false;
    }
}

/// Translation of `SDL_EVDEV_translate_keycode()`.
fn translate_keycode(keycode: c_int) -> Scancode {
    // (upstream's DEBUG_SCANCODES log of unknown keys isn't translated)
    get_scancode_from_table(ScancodeTable::Linux, keycode)
}

/// `EVIOCGNAME` into a buffer of `N` bytes (pre-set to the empty string):
/// the name up to its NUL (or the buffer's end: the kernel doesn't
/// terminate a name it truncates), and ioctl's result.
fn device_name<const N: usize>(fd: RawFd) -> (String, c_int) {
    let mut name = [0u8; N];
    let ret = ioctl_read(fd, eviocgname(N), &mut name);
    let len = name.iter().position(|&b| b == 0).unwrap_or(N);
    (String::from_utf8_lossy(&name[..len]).into_owned(), ret)
}

/// Translation of `SDL_EVDEV_init_keyboard()`.
fn init_keyboard(item: &mut EvdevListItem, _udev_class: DeviceClass) -> bool {
    let (name, _) = device_name::<128>(item.fd());

    add_keyboard(item.id(), Some(&name));

    true
}

/// Translation of `SDL_EVDEV_destroy_keyboard()`.
fn destroy_keyboard(item: &EvdevListItem) {
    remove_keyboard(item.id());
}

/// Translation of `SDL_EVDEV_init_mouse()`.
fn init_mouse(item: &mut EvdevListItem, _udev_class: DeviceClass) -> bool {
    let mut abs_info = input_absinfo::default();

    let (name, _) = device_name::<128>(item.fd());

    mouse::add_mouse(item.id(), Some(&name));

    let ret = ioctl_read(item.fd(), eviocgabs(ABS_X), &mut abs_info);
    if ret < 0 {
        // no absolute mode info, continue
        return true;
    }
    item.min_x = abs_info.minimum;
    item.max_x = abs_info.maximum;
    item.range_x = abs_info.maximum.wrapping_sub(abs_info.minimum);

    let ret = ioctl_read(item.fd(), eviocgabs(ABS_Y), &mut abs_info);
    if ret < 0 {
        // no absolute mode info, continue
        return true;
    }
    item.min_y = abs_info.minimum;
    item.max_y = abs_info.maximum;
    item.range_y = abs_info.maximum.wrapping_sub(abs_info.minimum);

    true
}

/// Translation of `SDL_EVDEV_destroy_mouse()`.
fn destroy_mouse(item: &EvdevListItem) {
    mouse::remove_mouse(item.id());
}

/// Translation of `SDL_EVDEV_init_touchscreen()`.
fn init_touchscreen(item: &mut EvdevListItem, udev_class: DeviceClass) -> bool {
    let mut abs_info = input_absinfo::default();

    if !item.is_touchscreen {
        return true;
    }

    let mut touchscreen_data = Box::<TouchscreenData>::default();

    let (name, ret) = device_name::<64>(item.fd());
    if ret < 0 {
        crate::log::error!(
            crate::log::Category::Input,
            "Failed to get evdev touchscreen name"
        );
        return false;
    }

    touchscreen_data.name = name;

    let ret = ioctl_read(item.fd(), eviocgabs(ABS_MT_SLOT), &mut abs_info);
    if ret < 0 {
        crate::log::error!(
            crate::log::Category::Input,
            "Failed to get evdev touchscreen limits"
        );
        return false;
    }

    let (xreq, yreq): (c_ulong, c_ulong);
    if abs_info.maximum == 0 {
        touchscreen_data.max_slots = 1;
        xreq = eviocgabs(ABS_X);
        yreq = eviocgabs(ABS_Y);
    } else {
        touchscreen_data.max_slots = abs_info.maximum.wrapping_add(1);
        xreq = eviocgabs(ABS_MT_POSITION_X);
        yreq = eviocgabs(ABS_MT_POSITION_Y);
    }

    let ret = ioctl_read(item.fd(), xreq, &mut abs_info);
    if ret < 0 {
        crate::log::error!(
            crate::log::Category::Input,
            "Failed to get evdev touchscreen limits"
        );
        return false;
    }
    touchscreen_data.min_x = abs_info.minimum;
    touchscreen_data.max_x = abs_info.maximum;
    touchscreen_data.range_x = abs_info.maximum.wrapping_sub(abs_info.minimum);

    let ret = ioctl_read(item.fd(), yreq, &mut abs_info);
    if ret < 0 {
        crate::log::error!(
            crate::log::Category::Input,
            "Failed to get evdev touchscreen limits"
        );
        return false;
    }
    touchscreen_data.min_y = abs_info.minimum;
    touchscreen_data.max_y = abs_info.maximum;
    touchscreen_data.range_y = abs_info.maximum.wrapping_sub(abs_info.minimum);

    let ret = ioctl_read(item.fd(), eviocgabs(ABS_MT_PRESSURE), &mut abs_info);
    if ret < 0 {
        crate::log::error!(
            crate::log::Category::Input,
            "Failed to get evdev touchscreen limits"
        );
        return false;
    }
    touchscreen_data.min_pressure = abs_info.minimum;
    touchscreen_data.max_pressure = abs_info.maximum;
    touchscreen_data.range_pressure = abs_info.maximum.wrapping_sub(abs_info.minimum);

    // Note (upstream): a negative slot count fails the allocation; here
    // it's no slots.
    touchscreen_data.slots =
        vec![TouchSlot::default(); usize::try_from(touchscreen_data.max_slots).unwrap_or(0)];

    touch::add_touch(
        item.id() as touch::TouchID, // I guess our fd is unique enough
        if udev_class.intersects(DeviceClass::TOUCHPAD) {
            TouchDeviceType::IndirectAbsolute
        } else {
            TouchDeviceType::Direct
        },
        &touchscreen_data.name,
    );

    item.touchscreen_data = Some(touchscreen_data);

    true
}

/// Translation of `SDL_EVDEV_destroy_touchscreen()`.
fn destroy_touchscreen(item: &mut EvdevListItem) {
    if !item.is_touchscreen {
        return;
    }

    touch::del_touch(item.id() as touch::TouchID);
    item.touchscreen_data = None;
}

/// Read a touchscreen's slots after the events were dropped
/// (`SYN_DROPPED`). Translation of `SDL_EVDEV_sync_device()`.
fn sync_device(item: &mut EvdevListItem) {
    let mut abs_info = input_absinfo::default();

    // TODO: sync devices other than touchscreen
    if !item.is_touchscreen {
        return;
    }
    let fd = item.fd();
    let touch = item.touch();

    /*
     * struct input_mt_request_layout {
     *     __u32 code;
     *     __s32 values[num_slots];
     * };
     *
     * this is the structure we're trying to emulate
     */
    let max_slots = touch.slots.len();
    let mut mt_req = vec![0i32; 1 + max_slots];
    let mt_req_size = size_of::<i32>() * mt_req.len();

    let read_slots = |code: usize, mt_req: &mut Vec<i32>| {
        mt_req[0] = code as i32;
        // SAFETY: the buffer is mt_req_size bytes, as the request says.
        unsafe { ioctl_ptr(fd, eviocgmtslots(mt_req_size), mt_req.as_mut_ptr().cast()) >= 0 }
    };

    if !read_slots(ABS_MT_TRACKING_ID, &mut mt_req) {
        return;
    }
    for (slot, &value) in touch.slots.iter_mut().zip(&mt_req[1..]) {
        /*
         * This doesn't account for the very edge case of the user removing their
         * finger and replacing it on the screen during the time we're out of sync,
         * which'll mean that we're not going from down -> up or up -> down, we're
         * going from down -> down but with a different tracking id, meaning we'd
         * have to tell SDL of the two events, but since we wait till SYN_REPORT in
         * SDL_EVDEV_Poll to tell SDL, the current structure of this code doesn't
         * allow it. Lets just pray to God it doesn't happen.
         */
        if slot.tracking_id == 0 && value >= 0 {
            slot.tracking_id = value.wrapping_add(1);
            slot.delta = EvdevTouchSlotDelta::Down;
        } else if slot.tracking_id != 0 && value < 0 {
            slot.tracking_id = 0;
            slot.delta = EvdevTouchSlotDelta::Up;
        }
    }

    for code in [ABS_MT_POSITION_X, ABS_MT_POSITION_Y, ABS_MT_PRESSURE] {
        if !read_slots(code, &mut mt_req) {
            return;
        }
        for (slot, &value) in touch.slots.iter_mut().zip(&mt_req[1..]) {
            let field = match code {
                ABS_MT_POSITION_X => &mut slot.x,
                ABS_MT_POSITION_Y => &mut slot.y,
                _ => &mut slot.pressure,
            };
            if slot.tracking_id != 0 && *field != value {
                *field = value;
                if slot.delta == EvdevTouchSlotDelta::None {
                    slot.delta = EvdevTouchSlotDelta::Move;
                }
            }
        }
    }

    let ret = ioctl_read(fd, eviocgabs(ABS_MT_SLOT), &mut abs_info);
    if ret < 0 {
        return;
    }
    touch.current_slot = abs_info.value;
}

/// Open a device the udev callback reports. Translation of
/// `SDL_EVDEV_device_added()`.
fn device_added(this: &mut EvdevPrivateData, dev_path: &str, udev_class: DeviceClass) -> bool {
    let mut relbit = [0 as c_ulong; nbits(REL_MAX)];

    // Check to make sure it's not already in list.
    if this.devices.iter().any(|item| item.path == dev_path) {
        return false; // already have this one
    }

    let Ok(c_path) = CString::new(dev_path) else {
        return false;
    };
    // SAFETY: the path is NUL-terminated.
    let fd = unsafe {
        libc::open(
            c_path.as_ptr(),
            libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        crate::log::error!(
            crate::log::Category::Input,
            "Couldn't open {}: {}",
            dev_path,
            strerror()
        );
        return false;
    }

    let mut item = EvdevListItem {
        path: dev_path.to_owned(),
        // SAFETY: fd was just opened, and is owned by the item from here on.
        fd: unsafe { OwnedFd::from_raw_fd(fd) },
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

    if ioctl_read(
        item.fd(),
        eviocgbit(EV_REL, std::mem::size_of_val(&relbit)),
        &mut relbit,
    ) >= 0
    {
        item.relative_mouse =
            test_bit(usize::from(REL_X), &relbit) && test_bit(usize::from(REL_Y), &relbit);
        item.high_res_wheel = test_bit(usize::from(REL_WHEEL_HI_RES), &relbit);
        item.high_res_hwheel = test_bit(usize::from(REL_HWHEEL_HI_RES), &relbit);
    }

    // For now, we just treat a touchpad like a touchscreen
    if udev_class.intersects(DeviceClass::TOUCHSCREEN | DeviceClass::TOUCHPAD) {
        item.is_touchscreen = true;
        if !init_touchscreen(&mut item, udev_class) {
            return false;
        }
    }

    if udev_class.intersects(DeviceClass::MOUSE) && !init_mouse(&mut item, udev_class) {
        return false;
    }

    if udev_class.intersects(DeviceClass::KEYBOARD) && !init_keyboard(&mut item, udev_class) {
        return false;
    }

    this.devices.push(item);

    let item = this.devices.last_mut().expect("just added");
    sync_device(item);

    update_keyboard_mute(this);

    this.num_devices += 1;
    true
}

/// Close a device the udev callback reports removed. Translation of
/// `SDL_EVDEV_device_removed()`.
fn device_removed(this: &mut EvdevPrivateData, dev_path: &str) -> bool {
    // found it, remove it.
    let Some(index) = this.devices.iter().position(|item| item.path == dev_path) else {
        return false;
    };
    let mut item = this.devices.remove(index);

    if item.is_touchscreen {
        destroy_touchscreen(&mut item);
    }
    if item.udev_class.intersects(DeviceClass::MOUSE) {
        destroy_mouse(&item);
    }
    if item.udev_class.intersects(DeviceClass::KEYBOARD) {
        destroy_keyboard(&item);
    }
    drop(item); // (closes the device)
    update_keyboard_mute(this);
    this.num_devices -= 1;
    true
}

/// Translation of the `static Uint64 timestamp_offset` of
/// `SDL_EVDEV_GetEventTimestamp()`.
static TIMESTAMP_OFFSET: AtomicU64 = AtomicU64::new(0);

/// The SDL timestamp (nanoseconds on the `ticks_ns()` clock) of an evdev
/// event. Translation of `SDL_EVDEV_GetEventTimestamp()`.
pub(crate) fn get_event_timestamp(event: &input_event) -> u64 {
    let now = crate::timer::ticks_ns();

    /* The kernel internally has nanosecond timestamps, but converts it
    to microseconds when delivering the events */
    let mut timestamp = event.time.tv_sec as u64;
    timestamp = timestamp.wrapping_mul(1_000_000_000);
    timestamp = timestamp.wrapping_add((event.time.tv_usec as u64).wrapping_mul(1000));

    let mut timestamp_offset = TIMESTAMP_OFFSET.load(Ordering::Relaxed);
    if timestamp_offset == 0 {
        timestamp_offset = now.wrapping_sub(timestamp);
    }
    timestamp = timestamp.wrapping_add(timestamp_offset);

    if timestamp > now {
        timestamp_offset = timestamp_offset.wrapping_sub(timestamp - now);
        timestamp = now;
    }
    TIMESTAMP_OFFSET.store(timestamp_offset, Ordering::Relaxed);
    timestamp
}

#[cfg(test)]
mod tests;
