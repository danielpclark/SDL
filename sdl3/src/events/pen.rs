// Rust translation of src/events/SDL_pen.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Pressure-sensitive pen handling code for SDL.

use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use super::mouse::{
    self, BUTTON_LEFT, BUTTON_MIDDLE, BUTTON_RIGHT, BUTTON_X1, BUTTON_X2, PEN_MOUSE_ID,
};
use super::queue;
use super::touch::{self, PEN_TOUCH_ID};
use super::window;
use super::{
    Event, EventType, PenAxisEvent, PenButtonEvent, PenMotionEvent, PenProximityEvent,
    PenTouchEvent, WindowID,
};
use crate::hints;

/// SDL pen instance IDs. Translation of `SDL_PenID`.
pub type PenID = u32;

/// Pen input flags, as reported by various pen events' `pen_state` field.
/// Translation of `SDL_PenInputFlags`.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PenInputFlags(pub u32);

impl PenInputFlags {
    pub const NONE: PenInputFlags = PenInputFlags(0);
    /// pen is pressed down
    pub const DOWN: PenInputFlags = PenInputFlags(1 << 0);
    /// button 1 is pressed
    pub const BUTTON_1: PenInputFlags = PenInputFlags(1 << 1);
    /// button 2 is pressed
    pub const BUTTON_2: PenInputFlags = PenInputFlags(1 << 2);
    /// button 3 is pressed
    pub const BUTTON_3: PenInputFlags = PenInputFlags(1 << 3);
    /// button 4 is pressed
    pub const BUTTON_4: PenInputFlags = PenInputFlags(1 << 4);
    /// button 5 is pressed
    pub const BUTTON_5: PenInputFlags = PenInputFlags(1 << 5);
    /// eraser tip is used
    pub const ERASER_TIP: PenInputFlags = PenInputFlags(1 << 30);
    /// pen is in proximity (hovering or touching) of the device
    pub const IN_PROXIMITY: PenInputFlags = PenInputFlags(1 << 31);

    /// The flag for a button index (first button is 1).
    pub const fn button(button: u8) -> PenInputFlags {
        PenInputFlags(1u32 << (button as u32 & 31))
    }
    pub const fn contains(self, other: PenInputFlags) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn intersects(self, other: PenInputFlags) -> bool {
        self.0 & other.0 != 0
    }
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for PenInputFlags {
    type Output = PenInputFlags;
    fn bitor(self, rhs: PenInputFlags) -> PenInputFlags {
        PenInputFlags(self.0 | rhs.0)
    }
}
impl std::ops::BitOrAssign for PenInputFlags {
    fn bitor_assign(&mut self, rhs: PenInputFlags) {
        self.0 |= rhs.0;
    }
}
impl std::ops::BitAnd for PenInputFlags {
    type Output = PenInputFlags;
    fn bitand(self, rhs: PenInputFlags) -> PenInputFlags {
        PenInputFlags(self.0 & rhs.0)
    }
}
impl std::ops::BitAndAssign for PenInputFlags {
    fn bitand_assign(&mut self, rhs: PenInputFlags) {
        self.0 &= rhs.0;
    }
}
impl std::ops::Not for PenInputFlags {
    type Output = PenInputFlags;
    fn not(self) -> PenInputFlags {
        PenInputFlags(!self.0)
    }
}
impl std::fmt::Debug for PenInputFlags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PenInputFlags({:#x})", self.0)
    }
}

/// Pen axis indices. Translation of `SDL_PenAxis`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum PenAxis {
    /// Pen pressure.  Unidirectional: 0 to 1.0
    Pressure = 0,
    /// Pen horizontal tilt angle.  Bidirectional: -90.0 to 90.0 (left-to-right).
    XTilt,
    /// Pen vertical tilt angle.  Bidirectional: -90.0 to 90.0 (top-to-down).
    YTilt,
    /// Pen distance to drawing surface.  Unidirectional: 0.0 to 1.0
    Distance,
    /// Pen barrel rotation.  Bidirectional: -180 to 179.9 (clockwise, 0 is facing up, -180.0 is facing down).
    Rotation,
    /// Pen finger wheel or slider (e.g., Airbrush Pen).  Unidirectional: 0 to 1.0
    Slider,
    /// Pressure from squeezing the pen ("barrel pressure").
    TangentialPressure,
}

impl PenAxis {
    /// Translation of `SDL_PEN_AXIS_COUNT`.
    pub const COUNT: usize = 7;

    /// Every axis, in index order.
    pub const ALL: [PenAxis; PenAxis::COUNT] = [
        PenAxis::Pressure,
        PenAxis::XTilt,
        PenAxis::YTilt,
        PenAxis::Distance,
        PenAxis::Rotation,
        PenAxis::Slider,
        PenAxis::TangentialPressure,
    ];

    /// The axis for an `SDL_PenAxis` integer value.
    pub fn from_index(index: i32) -> Option<PenAxis> {
        usize::try_from(index)
            .ok()
            .and_then(|i| PenAxis::ALL.get(i).copied())
    }

    /// The name used by `SDL_GetEventDescription()` for this axis.
    pub fn name(self) -> &'static str {
        match self {
            PenAxis::Pressure => "PRESSURE",
            PenAxis::XTilt => "XTILT",
            PenAxis::YTilt => "YTILT",
            PenAxis::Distance => "DISTANCE",
            PenAxis::Rotation => "ROTATION",
            PenAxis::Slider => "SLIDER",
            PenAxis::TangentialPressure => "TANGENTIAL_PRESSURE",
        }
    }
}

/// Pen device types. Translation of `SDL_PenDeviceType`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum PenDeviceType {
    /// Invalid device type
    Invalid = -1,
    /// Unknown device type
    #[default]
    Unknown,
    /// Direct device type, e.g. a tablet with a built-in display
    Direct,
    /// Indirect device type, e.g. a tablet without a display
    Indirect,
}

/// Pen capability flags. Translation of `SDL_PenCapabilityFlags`.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct PenCapabilityFlags(pub u32);

impl PenCapabilityFlags {
    pub const PRESSURE: PenCapabilityFlags = PenCapabilityFlags(1 << 0);
    pub const XTILT: PenCapabilityFlags = PenCapabilityFlags(1 << 1);
    pub const YTILT: PenCapabilityFlags = PenCapabilityFlags(1 << 2);
    pub const DISTANCE: PenCapabilityFlags = PenCapabilityFlags(1 << 3);
    pub const ROTATION: PenCapabilityFlags = PenCapabilityFlags(1 << 4);
    pub const SLIDER: PenCapabilityFlags = PenCapabilityFlags(1 << 5);
    pub const TANGENTIAL_PRESSURE: PenCapabilityFlags = PenCapabilityFlags(1 << 6);
    pub const ERASER: PenCapabilityFlags = PenCapabilityFlags(1 << 30);

    /// Translation of `SDL_GetPenCapabilityFromAxis()`.
    pub fn from_axis(axis: PenAxis) -> PenCapabilityFlags {
        // the initial capability bits happen to match up, but as
        // more features show up later, the bits may no longer be contiguous!
        if (axis as u8) <= (PenAxis::Slider as u8) {
            return PenCapabilityFlags(1u32 << (axis as u32));
        }
        PenCapabilityFlags(0) // oh well.
    }
}

/// Pen types. Translation of `SDL_PenSubtype`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum PenSubtype {
    #[default]
    Unknown = 0,
    Eraser,
    Pen,
    Pencil,
    Brush,
    Airbrush,
}

/// Static pen information. Translation of `SDL_PenInfo`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PenInfo {
    pub capabilities: PenCapabilityFlags,
    pub max_tilt: i32,
    pub wacom_id: u32,
    pub num_buttons: i32,
    pub subtype: PenSubtype,
    pub device_type: PenDeviceType,
}

/// Translation of `struct SDL_Pen`.
struct Pen {
    instance_id: PenID,
    /// Only read by `SDL_GetPenName()`, which upstream keeps under `#if 0`.
    #[allow(dead_code)]
    name: String,
    info: PenInfo,
    axes: [f32; PenAxis::COUNT],
    x: f32,
    y: f32,
    input_state: PenInputFlags,
    pending_proximity_out: bool,
    pending_proximity_window_id: WindowID,
    /// The backend's handle (`driverdata`).
    handle: Arc<dyn Any + Send + Sync>,
}

// we assume there's usually 0-1 pens in most cases and this list doesn't
// usually change after startup, so a simple array with a RWlock is fine for now.
static PEN_DEVICES: RwLock<Vec<Pen>> = RwLock::new(Vec::new());
/// used for synthetic mouse/touch events. Translation of `pen_touching`.
static PEN_TOUCHING: AtomicU32 = AtomicU32::new(0);
static PENDING_PROXIMITY_OUT: AtomicBool = AtomicBool::new(false);
static HINT_CALLBACK: std::sync::Mutex<Option<hints::Callback>> = std::sync::Mutex::new(None);

fn read_pens() -> RwLockReadGuard<'static, Vec<Pen>> {
    PEN_DEVICES.read().unwrap_or_else(|e| e.into_inner())
}

fn write_pens() -> RwLockWriteGuard<'static, Vec<Pen>> {
    PEN_DEVICES.write().unwrap_or_else(|e| e.into_inner())
}

// You must hold the lock before calling this, and result is only safe while lock is held!
/// Translation of `FindPenByInstanceId()`.
fn find_pen_by_instance_id(pens: &[Pen], instance_id: PenID) -> Option<usize> {
    if instance_id != 0 {
        return pens.iter().position(|p| p.instance_id == instance_id);
    }
    // SDL_SetError("Invalid pen instance ID")
    None
}

/// Find the pen registered with a given backend handle (by pointer identity).
/// Translation of `SDL_FindPenByHandle()`; returns 0 if none.
pub fn find_pen_by_handle(handle: &Arc<dyn Any + Send + Sync>) -> PenID {
    read_pens()
        .iter()
        .find(|p| Arc::ptr_eq(&p.handle, handle))
        .map(|p| p.instance_id)
        .unwrap_or(0)
}

/// Find the first pen whose backend handle satisfies `callback`.
/// Translation of `SDL_FindPenByCallback()`; returns 0 if none.
pub fn find_pen_by_callback(callback: impl Fn(&(dyn Any + Send + Sync)) -> bool) -> PenID {
    read_pens()
        .iter()
        .find(|p| callback(&*p.handle))
        .map(|p| p.instance_id)
        .unwrap_or(0)
}

/// Translation of `UpdateTouchEmulationDevicePresence()`.
fn update_touch_emulation_device_presence() {
    let has_pen = !read_pens().is_empty();

    let (add, del) = mouse::with_mouse(|mouse| {
        if !mouse.pen_touch_events || !has_pen {
            if mouse.added_pen_touch_device {
                mouse.added_pen_touch_device = false;
                return (false, true);
            }
        } else if !mouse.added_pen_touch_device {
            mouse.added_pen_touch_device = true;
            return (true, false);
        }
        (false, false)
    });
    if del {
        touch::del_touch(PEN_TOUCH_ID);
    }
    if add {
        touch::add_touch(PEN_TOUCH_ID, touch::TouchDeviceType::Direct, "pen_input");
    }
}

/// Translation of `SDL_PenTouchEventsChanged()`.
fn pen_touch_events_changed(hint: Option<&str>) {
    mouse::with_mouse(|mouse| mouse.pen_touch_events = hints::string_to_bool(hint, true));
    update_touch_emulation_device_presence();
}

// public API ...

/// Initialize the pen subsystem (called by the video subsystem's init).
/// Translation of `SDL_InitPen()`.
pub fn init_pen() -> crate::Result<()> {
    debug_assert!(read_pens().is_empty());
    let cb = hints::watch(hints::PEN_TOUCH_EVENTS, |c| {
        pen_touch_events_changed(c.new_value)
    })?;
    *HINT_CALLBACK.lock().unwrap_or_else(|e| e.into_inner()) = Some(cb);
    Ok(())
}

/// Shut down the pen subsystem. Translation of `SDL_QuitPen()`.
pub fn quit_pen() {
    *HINT_CALLBACK.lock().unwrap_or_else(|e| e.into_inner()) = None;
    remove_all_pen_devices(|_, _| {});
}

/// The pen's current input state and axis values.
/// Translation of `SDL_GetPenStatus()`; empty flags and zero axes if the pen is unknown.
pub fn pen_status(instance_id: PenID) -> (PenInputFlags, [f32; PenAxis::COUNT]) {
    let pens = read_pens();
    match find_pen_by_instance_id(&pens, instance_id) {
        Some(i) => (pens[i].input_state, pens[i].axes),
        None => (PenInputFlags::NONE, [0.0; PenAxis::COUNT]),
    }
}

/// Translation of `SDL_GetPenDeviceType()`.
pub fn pen_device_type(instance_id: PenID) -> PenDeviceType {
    let pens = read_pens();
    find_pen_by_instance_id(&pens, instance_id)
        .map(|i| pens[i].info.device_type)
        .unwrap_or(PenDeviceType::Invalid)
}

/// Register a pen device; returns its new id. Translation of `SDL_AddPenDevice()`.
///
/// `handle` identifies the pen to the backend (compare with [`find_pen_by_handle`]).
pub fn add_pen_device(
    timestamp: Duration,
    name: Option<&str>,
    window: Option<WindowID>,
    info: Option<&PenInfo>,
    handle: Arc<dyn Any + Send + Sync>,
    in_proximity: bool,
) -> PenID {
    debug_assert!(find_pen_by_handle(&handle) == 0); // Backends shouldn't double-add pens!

    let result: PenID = crate::utils::next_object_id();
    write_pens().push(Pen {
        instance_id: result,
        name: name.unwrap_or("Unnamed pen").to_owned(),
        info: info.copied().unwrap_or_default(),
        axes: [0.0; PenAxis::COUNT],
        x: 0.0,
        y: 0.0,
        input_state: PenInputFlags::NONE,
        pending_proximity_out: false,
        pending_proximity_window_id: 0,
        handle,
        // axes and input state defaults to zero.
    });

    update_touch_emulation_device_presence();

    if result != 0 && in_proximity {
        send_pen_proximity(timestamp, result, window, true, true);
    }

    result
}

/// Unregister a pen device, sending a final proximity-out. Translation of `SDL_RemovePenDevice()`.
pub fn remove_pen_device(timestamp: Duration, window: Option<WindowID>, instance_id: PenID) {
    if instance_id == 0 {
        return;
    }

    send_pen_proximity(timestamp, instance_id, window, false, true); // bye bye

    {
        let mut pens = write_pens();
        if let Some(idx) = find_pen_by_instance_id(&pens, instance_id) {
            pens.remove(idx);
        }
    }

    update_touch_emulation_device_presence();
}

// This presumably is happening during video quit, so we don't send PROXIMITY_OUT events here.
/// Translation of `SDL_RemoveAllPenDevices()`; `callback` sees each pen's id and handle.
pub fn remove_all_pen_devices(callback: impl Fn(PenID, &Arc<dyn Any + Send + Sync>)) {
    let mut pens = write_pens();
    for pen in pens.iter() {
        callback(pen.instance_id, &pen.handle);
    }
    pens.clear();
    PEN_TOUCHING.store(0, Ordering::Relaxed);
}

/// The window id for an event (0 for none).
fn wid(window: Option<WindowID>) -> WindowID {
    window.unwrap_or(0)
}

/// Report the pen touching or lifting off the surface. Translation of `SDL_SendPenTouch()`.
pub fn send_pen_touch(
    timestamp: Duration,
    instance_id: PenID,
    window: Option<WindowID>,
    eraser: bool,
    down: bool,
) {
    let mut send_event = false;
    let mut input_state = PenInputFlags::NONE;
    let mut device_type = PenDeviceType::Unknown;
    let mut x = 0.0f32;
    let mut y = 0.0f32;
    let mut pressure = 0.0f32;

    // note that upstream locks for _reading_ because the lock protects the
    // pen_devices array from being reallocated from under us, not the data in it;
    // we assume only one thread (in the backend) is modifying an individual pen at
    // a time, so it can update input state cleanly here. Rust needs the write
    // lock to mutate, which is held only for this bookkeeping.
    {
        let mut pens = write_pens();
        if let Some(i) = find_pen_by_instance_id(&pens, instance_id) {
            let pen = &mut pens[i];
            input_state = pen.input_state;
            device_type = pen.info.device_type;
            x = pen.x;
            y = pen.y;
            pressure = pen.axes[PenAxis::Pressure as usize];
            if down && !input_state.contains(PenInputFlags::DOWN) {
                input_state |= PenInputFlags::DOWN;
                send_event = true;
            } else if !down && input_state.contains(PenInputFlags::DOWN) {
                input_state &= !PenInputFlags::DOWN;
                send_event = true;
            }

            if eraser && !input_state.contains(PenInputFlags::ERASER_TIP) {
                input_state |= PenInputFlags::ERASER_TIP;
                send_event = true;
            } else if !eraser && input_state.contains(PenInputFlags::ERASER_TIP) {
                input_state &= !PenInputFlags::ERASER_TIP;
                send_event = true;
            }

            pen.input_state = input_state;
        }
    }

    if send_event {
        let evtype = if down {
            EventType::PEN_DOWN
        } else {
            EventType::PEN_UP
        };
        if queue::event_enabled(evtype) {
            let _ = queue::push(Event::PenTouch(PenTouchEvent {
                timestamp,
                window_id: wid(window),
                which: instance_id,
                pen_state: input_state,
                x,
                y,
                eraser,
                down,
                device_type,
            }));
        }

        if let Some(window_id) = window {
            let (pen_mouse_events, pen_touch_events) =
                mouse::with_mouse(|m| (m.pen_mouse_events, m.pen_touch_events));
            let pen_touching = PEN_TOUCHING.load(Ordering::Relaxed);
            if pen_mouse_events {
                if down {
                    if pen_touching == 0 {
                        mouse::send_mouse_motion(timestamp, window, PEN_MOUSE_ID, false, x, y);
                        mouse::send_mouse_button(
                            timestamp,
                            window,
                            PEN_MOUSE_ID,
                            BUTTON_LEFT,
                            true,
                        );
                    }
                } else if pen_touching == instance_id {
                    mouse::send_mouse_button(timestamp, window, PEN_MOUSE_ID, BUTTON_LEFT, false);
                }
            }

            if pen_touch_events {
                let touchtype = if down {
                    EventType::FINGER_DOWN
                } else {
                    EventType::FINGER_UP
                };
                if let Some(w) = window::window(window_id) {
                    let normalized_x = x / w.w as f32;
                    let normalized_y = y / w.h as f32;
                    if pen_touching == 0 || pen_touching == instance_id {
                        touch::send_touch(
                            timestamp,
                            PEN_TOUCH_ID,
                            BUTTON_LEFT as touch::FingerID,
                            window,
                            touchtype,
                            normalized_x,
                            normalized_y,
                            pressure,
                        );
                    }
                }
            }
        }

        if down {
            let _ =
                PEN_TOUCHING.compare_exchange(0, instance_id, Ordering::Relaxed, Ordering::Relaxed);
        } else {
            let _ =
                PEN_TOUCHING.compare_exchange(instance_id, 0, Ordering::Relaxed, Ordering::Relaxed);
        }
    }
}

/// Report an axis change. Translation of `SDL_SendPenAxis()`.
pub fn send_pen_axis(
    timestamp: Duration,
    instance_id: PenID,
    window: Option<WindowID>,
    axis: PenAxis,
    value: f32,
) {
    let mut send_event = false;
    let mut input_state = PenInputFlags::NONE;
    let mut device_type = PenDeviceType::Unknown;
    let mut x = 0.0f32;
    let mut y = 0.0f32;

    {
        let mut pens = write_pens();
        if let Some(i) = find_pen_by_instance_id(&pens, instance_id) {
            let pen = &mut pens[i];
            if pen.axes[axis as usize] != value {
                pen.axes[axis as usize] = value;
                input_state = pen.input_state;
                device_type = pen.info.device_type;
                x = pen.x;
                y = pen.y;
                send_event = true;
            }
        }
    }

    if send_event && queue::event_enabled(EventType::PEN_AXIS) {
        let _ = queue::push(Event::PenAxis(PenAxisEvent {
            timestamp,
            window_id: wid(window),
            which: instance_id,
            pen_state: input_state,
            x,
            y,
            axis,
            value,
            device_type,
        }));

        if let Some(window_id) = window {
            if axis == PenAxis::Pressure
                && PEN_TOUCHING.load(Ordering::Relaxed) == instance_id
                && mouse::with_mouse(|m| m.pen_touch_events)
            {
                if let Some(w) = window::window(window_id) {
                    let normalized_x = x / w.w as f32;
                    let normalized_y = y / w.h as f32;
                    touch::send_touch_motion(
                        timestamp,
                        PEN_TOUCH_ID,
                        BUTTON_LEFT as touch::FingerID,
                        window,
                        normalized_x,
                        normalized_y,
                        value,
                    );
                }
            }
        }
    }
}

/// Translation of `EnsurePenProximity()`. Returns true if a proximity-in must be sent
/// (the caller does so after releasing the lock).
fn ensure_pen_proximity(pen: &mut Pen) -> bool {
    if pen.pending_proximity_out {
        pen.pending_proximity_out = false;
        false
    } else {
        !pen.input_state.contains(PenInputFlags::IN_PROXIMITY)
    }
}

/// Report pen motion. Translation of `SDL_SendPenMotion()`.
pub fn send_pen_motion(
    timestamp: Duration,
    instance_id: PenID,
    window: Option<WindowID>,
    x: f32,
    y: f32,
) {
    let mut send_event = false;
    let mut input_state = PenInputFlags::NONE;
    let mut device_type = PenDeviceType::Unknown;
    let mut pressure = 0.0f32;
    let mut need_proximity = false;

    {
        let mut pens = write_pens();
        if let Some(i) = find_pen_by_instance_id(&pens, instance_id) {
            let pen = &mut pens[i];
            need_proximity = ensure_pen_proximity(pen);
            if pen.x != x || pen.y != y {
                pen.x = x;
                pen.y = y;
                input_state = pen.input_state;
                device_type = pen.info.device_type;
                pressure = pen.axes[PenAxis::Pressure as usize];
                send_event = true;
            }
        }
    }
    if need_proximity {
        send_pen_proximity(timestamp, instance_id, window, true, true);
        // The proximity-in changed the state we're about to report.
        if send_event {
            input_state |= PenInputFlags::IN_PROXIMITY;
        }
    }

    if send_event && queue::event_enabled(EventType::PEN_MOTION) {
        let _ = queue::push(Event::PenMotion(PenMotionEvent {
            timestamp,
            window_id: wid(window),
            which: instance_id,
            pen_state: input_state,
            x,
            y,
            device_type,
        }));

        if let Some(window_id) = window {
            let (pen_mouse_events, pen_touch_events) =
                mouse::with_mouse(|m| (m.pen_mouse_events, m.pen_touch_events));
            let pen_touching = PEN_TOUCHING.load(Ordering::Relaxed);
            if pen_touching == instance_id {
                if pen_mouse_events {
                    mouse::send_mouse_motion(timestamp, window, PEN_MOUSE_ID, false, x, y);
                }
                if pen_touch_events {
                    if let Some(w) = window::window(window_id) {
                        let normalized_x = x / w.w as f32;
                        let normalized_y = y / w.h as f32;
                        touch::send_touch_motion(
                            timestamp,
                            PEN_TOUCH_ID,
                            BUTTON_LEFT as touch::FingerID,
                            window,
                            normalized_x,
                            normalized_y,
                            pressure,
                        );
                    }
                }
            } else if pen_touching == 0 {
                // send mouse motion (without a pressed button) for pens that aren't touching.
                // this might cause a little chaos if you have multiple pens hovering at the same time, but this seems unlikely in the real world, and also something you did to yourself.  :)
                if pen_mouse_events {
                    mouse::send_mouse_motion(timestamp, window, PEN_MOUSE_ID, false, x, y);
                }
            }
        }
    }
}

/// Translation of `SendPenButtonEvents()`.
#[allow(clippy::too_many_arguments)]
fn send_pen_button_events(
    timestamp: Duration,
    instance_id: PenID,
    window: Option<WindowID>,
    button: u8,
    down: bool,
    input_state: PenInputFlags,
    x: f32,
    y: f32,
    device_type: PenDeviceType,
) {
    let evtype = if down {
        EventType::PEN_BUTTON_DOWN
    } else {
        EventType::PEN_BUTTON_UP
    };
    if queue::event_enabled(evtype) {
        let _ = queue::push(Event::PenButton(PenButtonEvent {
            timestamp,
            window_id: wid(window),
            which: instance_id,
            pen_state: input_state,
            x,
            y,
            button,
            down,
            device_type,
        }));

        let pen_touching = PEN_TOUCHING.load(Ordering::Relaxed);
        if window.is_some()
            && (pen_touching == 0 || pen_touching == instance_id)
            && mouse::with_mouse(|m| m.pen_mouse_events)
        {
            const MOUSE_BUTTONS: [u8; 5] = [
                BUTTON_LEFT,
                BUTTON_RIGHT,
                BUTTON_MIDDLE,
                BUTTON_X1,
                BUTTON_X2,
            ];
            if let Some(&mb) = MOUSE_BUTTONS.get(button as usize) {
                mouse::send_mouse_button(timestamp, window, PEN_MOUSE_ID, mb, down);
            }
        }
    }
}

/// Report a pen barrel button press or release (buttons 1..=5).
/// Translation of `SDL_SendPenButton()`.
pub fn send_pen_button(
    timestamp: Duration,
    instance_id: PenID,
    window: Option<WindowID>,
    button: u8,
    down: bool,
) {
    let mut send_event = false;
    let mut input_state = PenInputFlags::NONE;
    let mut device_type = PenDeviceType::Unknown;
    let mut x = 0.0f32;
    let mut y = 0.0f32;
    let mut need_proximity = false;

    if !(1..=5).contains(&button) {
        return; // clamp for now.
    }

    {
        let mut pens = write_pens();
        if let Some(i) = find_pen_by_instance_id(&pens, instance_id) {
            let pen = &mut pens[i];
            need_proximity = ensure_pen_proximity(pen);
            input_state = pen.input_state;
            device_type = pen.info.device_type;
            let flag = PenInputFlags::button(button);
            let current = input_state.contains(flag);
            x = pen.x;
            y = pen.y;
            if down && !current {
                input_state |= flag;
                send_event = true;
            } else if !down && current {
                input_state &= !flag;
                send_event = true;
            }
            pen.input_state = input_state;
        }
    }
    if need_proximity {
        send_pen_proximity(timestamp, instance_id, window, true, true);
        input_state |= PenInputFlags::IN_PROXIMITY;
    }

    if send_event {
        send_pen_button_events(
            timestamp,
            instance_id,
            window,
            button,
            down,
            input_state,
            x,
            y,
            device_type,
        );
    }
}

/// Report the pen entering or leaving proximity. A proximity-out that is not
/// `immediate` is deferred to the next event pump (so a quick re-entry cancels it).
/// Translation of `SDL_SendPenProximity()`.
pub fn send_pen_proximity(
    timestamp: Duration,
    instance_id: PenID,
    window: Option<WindowID>,
    in_proximity: bool,
    immediate: bool,
) {
    let mut send_event = false;
    let mut input_state = PenInputFlags::NONE;
    let mut orig_input_state = PenInputFlags::NONE;
    let mut device_type = PenDeviceType::Unknown;
    let mut x = 0.0f32;
    let mut y = 0.0f32;

    {
        let mut pens = write_pens();
        if let Some(i) = find_pen_by_instance_id(&pens, instance_id) {
            let pen = &mut pens[i];
            device_type = pen.info.device_type;
            if in_proximity || immediate {
                input_state = pen.input_state;
                orig_input_state = input_state;
                let currently_in = input_state.contains(PenInputFlags::IN_PROXIMITY);
                if currently_in != in_proximity {
                    if in_proximity {
                        input_state |= PenInputFlags::IN_PROXIMITY;
                    } else {
                        input_state &= !PenInputFlags::IN_PROXIMITY;
                        // drop all still-pressed buttons, too.
                        for button in 1..=5u8 {
                            input_state &= !PenInputFlags::button(button);
                        }
                    }
                    send_event = true;
                    pen.input_state = input_state;
                    x = pen.x;
                    y = pen.y;
                }
                pen.pending_proximity_out = false;
            } else {
                pen.pending_proximity_out = true;
                pen.pending_proximity_window_id = wid(window);
                PENDING_PROXIMITY_OUT.store(true, Ordering::Release);
            }
        }
    }

    if send_event {
        // report any currently-pressed buttons as released if we're leaving proximity.
        if !in_proximity {
            for button in 1..=5u8 {
                let flag = PenInputFlags::button(button);
                if orig_input_state.contains(flag) {
                    // orig_input_state continues to report we're in proximity.
                    orig_input_state &= !flag; // drop the button we're reporting on.
                    send_pen_button_events(
                        timestamp,
                        instance_id,
                        window,
                        button,
                        false,
                        orig_input_state,
                        x,
                        y,
                        device_type,
                    );
                }
            }
        }

        let event_type = if in_proximity {
            EventType::PEN_PROXIMITY_IN
        } else {
            EventType::PEN_PROXIMITY_OUT
        };
        if queue::event_enabled(event_type) {
            let _ = queue::push(Event::PenProximity(PenProximityEvent {
                event_type,
                timestamp,
                window_id: wid(window),
                which: instance_id,
                pen_state: input_state,
                device_type,
            }));
        }
    }
}

/// Deliver deferred proximity-out events. Translation of `SDL_SendPendingPenProximity()`.
pub(crate) fn send_pending_pen_proximity() {
    if PENDING_PROXIMITY_OUT
        .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        let pending: Vec<(PenID, WindowID)> = {
            let mut pens = write_pens();
            pens.iter_mut()
                .filter(|p| p.pending_proximity_out)
                .map(|p| {
                    p.pending_proximity_out = false;
                    (p.instance_id, p.pending_proximity_window_id)
                })
                .collect()
        };
        for (instance_id, window_id) in pending {
            let window = if window_id != 0 {
                if window::window(window_id).is_none() {
                    // The window is already gone, ignore this event
                    continue;
                }
                Some(window_id)
            } else {
                None
            };
            send_pen_proximity(Duration::ZERO, instance_id, window, false, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::window::tests::with_video;

    fn drain() -> Vec<Event> {
        queue::get_events(EventType::FIRST, EventType::LAST, 1000).unwrap()
    }

    #[test]
    fn flags_and_axes() {
        assert_eq!(PenInputFlags::button(1), PenInputFlags::BUTTON_1);
        assert_eq!(PenInputFlags::button(5), PenInputFlags::BUTTON_5);
        assert_eq!(PenInputFlags::IN_PROXIMITY.0, 0x8000_0000);
        assert_eq!(PenAxis::from_index(6), Some(PenAxis::TangentialPressure));
        assert_eq!(PenAxis::from_index(7), None);
        assert_eq!(PenAxis::TangentialPressure.name(), "TANGENTIAL_PRESSURE");
        assert_eq!(
            PenCapabilityFlags::from_axis(PenAxis::Slider),
            PenCapabilityFlags::SLIDER
        );
        assert_eq!(
            PenCapabilityFlags::from_axis(PenAxis::TangentialPressure).0,
            0
        );
    }

    #[test]
    fn pen_lifecycle_and_emulation() {
        with_video(&[1], |_| {
            mouse::pre_init_mouse().unwrap();
            init_pen().unwrap();
            let handle: Arc<dyn Any + Send + Sync> = Arc::new(42u8);
            let info = PenInfo {
                num_buttons: 2,
                device_type: PenDeviceType::Direct,
                ..Default::default()
            };
            let id = add_pen_device(
                Duration::ZERO,
                Some("Stylus"),
                Some(1),
                Some(&info),
                handle.clone(),
                true,
            );
            assert_ne!(id, 0);
            assert_eq!(find_pen_by_handle(&handle), id);
            assert_eq!(
                find_pen_by_callback(|h| h.downcast_ref::<u8>() == Some(&42)),
                id
            );
            assert_eq!(pen_device_type(id), PenDeviceType::Direct);
            assert_eq!(pen_device_type(999), PenDeviceType::Invalid);
            // Pen touch emulation device appears when a pen exists.
            assert!(touch::touch_devices().contains(&PEN_TOUCH_ID));
            let types: Vec<_> = drain().iter().map(|e| e.event_type()).collect();
            assert_eq!(types, vec![EventType::PEN_PROXIMITY_IN]);

            send_pen_motion(Duration::ZERO, id, Some(1), 100.0, 50.0);
            send_pen_motion(Duration::ZERO, id, Some(1), 100.0, 50.0); // no change
            send_pen_axis(Duration::ZERO, id, Some(1), PenAxis::Pressure, 0.5);
            send_pen_touch(Duration::ZERO, id, Some(1), false, true);
            let types: Vec<_> = drain().iter().map(|e| e.event_type()).collect();
            // hovering pen moves the mouse; touching presses the emulated button and finger
            assert!(types.starts_with(&[EventType::PEN_MOTION]), "{types:?}");
            assert!(types.contains(&EventType::MOUSE_MOTION), "{types:?}");
            assert!(types.contains(&EventType::PEN_AXIS), "{types:?}");
            assert!(types.contains(&EventType::PEN_DOWN), "{types:?}");
            assert!(types.contains(&EventType::MOUSE_BUTTON_DOWN), "{types:?}");
            assert!(types.contains(&EventType::FINGER_DOWN), "{types:?}");
            let (state, axes) = pen_status(id);
            assert!(state.contains(PenInputFlags::DOWN | PenInputFlags::IN_PROXIMITY));
            assert_eq!(axes[0], 0.5);

            // Buttons map to mouse buttons; button 0/6 are ignored.
            send_pen_button(Duration::ZERO, id, Some(1), 2, true);
            send_pen_button(Duration::ZERO, id, Some(1), 6, true);
            let events = drain();
            assert!(matches!(
                events[0],
                Event::PenButton(PenButtonEvent {
                    button: 2,
                    down: true,
                    ..
                })
            ));
            assert!(matches!(
                events[1],
                Event::MouseButton(super::super::MouseButtonEvent {
                    button: BUTTON_MIDDLE,
                    ..
                })
            ));
            assert_eq!(events.len(), 2);

            // Deferred proximity out: nothing until the pump, then buttons release first.
            send_pen_touch(Duration::ZERO, id, Some(1), false, false);
            drain();
            send_pen_proximity(Duration::ZERO, id, Some(1), false, false);
            assert!(drain().is_empty());
            send_pending_pen_proximity();
            let types: Vec<_> = drain().iter().map(|e| e.event_type()).collect();
            assert_eq!(types[0], EventType::PEN_BUTTON_UP);
            assert!(types.contains(&EventType::PEN_PROXIMITY_OUT));
            assert!(!pen_status(id)
                .0
                .intersects(PenInputFlags::IN_PROXIMITY | PenInputFlags::BUTTON_2));

            // A deferred out cancelled by motion sends no event.
            send_pen_proximity(Duration::ZERO, id, Some(1), true, true);
            drain();
            send_pen_proximity(Duration::ZERO, id, Some(1), false, false);
            send_pen_motion(Duration::ZERO, id, Some(1), 10.0, 10.0);
            send_pending_pen_proximity();
            let types: Vec<_> = drain().iter().map(|e| e.event_type()).collect();
            assert!(!types.contains(&EventType::PEN_PROXIMITY_OUT), "{types:?}");

            remove_pen_device(Duration::ZERO, Some(1), id);
            assert_eq!(find_pen_by_handle(&handle), 0);
            assert!(!touch::touch_devices().contains(&PEN_TOUCH_ID));
            assert!(drain()
                .iter()
                .any(|e| e.event_type() == EventType::PEN_PROXIMITY_OUT));
            quit_pen();
            mouse::quit_mouse();
        });
    }
}
