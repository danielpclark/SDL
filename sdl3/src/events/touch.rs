// Rust translation of src/events/SDL_touch.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! General touch handling code for SDL: touch devices, fingers, and the
//! touch ⇄ mouse emulation hooks.

use std::cell::RefCell;
use std::time::Duration;

use super::mouse::{self, BUTTON_LEFT, TOUCH_MOUSE_ID};
use super::queue::{self, EVENT_LOCK};
use super::window::{self, WindowCore};
use super::{Event, EventType, PinchFingerEvent, TouchFingerEvent, WindowID};
use crate::error::{Error, Result};
use crate::thread::{RawMutexGuard, ReentrantMutex};

/// A unique id for a touch device. Translation of `SDL_TouchID`.
pub type TouchID = u64;
/// A unique id for a single finger on a touch device. Translation of `SDL_FingerID`.
pub type FingerID = u64;

/// The touch id for touch events simulated with mouse input.
/// Translation of `SDL_MOUSE_TOUCHID` (`(SDL_TouchID)-1`).
pub const MOUSE_TOUCH_ID: TouchID = u64::MAX;
/// The touch id for touch events simulated with pen input.
/// Translation of `SDL_PEN_TOUCHID` (`(SDL_TouchID)-2`).
pub const PEN_TOUCH_ID: TouchID = u64::MAX - 1;

/// An enum that describes the type of a touch device. Translation of `SDL_TouchDeviceType`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum TouchDeviceType {
    #[default]
    Invalid = -1,
    /// touch screen with window-relative coordinates
    Direct,
    /// trackpad with absolute device coordinates
    IndirectAbsolute,
    /// trackpad with screen cursor-relative coordinates
    IndirectRelative,
}

/// Data about a single finger in a multitouch event. Translation of `SDL_Finger`.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Finger {
    /// the finger ID
    pub id: FingerID,
    /// the x-axis location of the touch event, normalized (0...1)
    pub x: f32,
    /// the y-axis location of the touch event, normalized (0...1)
    pub y: f32,
    /// the quantity of pressure applied, normalized (0...1)
    pub pressure: f32,
}

/// Translation of `struct SDL_Touch`.
struct Touch {
    id: TouchID,
    device_type: TouchDeviceType,
    fingers: Vec<Finger>,
    name: String,
}

struct TouchState {
    /// Translation of `SDL_touchDevices`.
    devices: Vec<Touch>,
    // for mapping touch events to mice
    finger_touching: bool,
    track_fingerid: FingerID,
    track_touchid: TouchID,
}

/// Upstream guards this with `SDL_event_lock` (`SDL_GUARDED_BY(SDL_event_lock)`);
/// here it has its own recursive lock and the send functions additionally hold
/// [`EVENT_LOCK`] for the duration, as upstream does. The `RefCell` borrow is
/// never held across a call into the mouse or the event queue.
static TOUCH: ReentrantMutex<RefCell<TouchState>> = ReentrantMutex::new(RefCell::new(TouchState {
    devices: Vec::new(),
    finger_touching: false,
    track_fingerid: 0,
    track_touchid: 0,
}));

/// Translation of `SDL_LockTouch()`; the guard releases on drop.
fn lock_touch() -> RawMutexGuard<'static> {
    EVENT_LOCK.guard()
}

/// Run `f` on the touch state while holding the event lock.
fn with_touch<R>(f: impl FnOnce(&mut TouchState) -> R) -> R {
    let guard = TOUCH.lock();
    let mut state = guard.borrow_mut();
    f(&mut state)
}

// Public functions

/// Initialize the touch subsystem (called by the video subsystem's init).
/// Translation of `SDL_InitTouch()`.
pub fn init_touch() -> Result<()> {
    Ok(())
}

/// Whether any touch device is registered. Translation of `SDL_TouchDevicesAvailable()`.
pub fn touch_devices_available() -> bool {
    with_touch(|t| !t.devices.is_empty())
}

/// The registered touch device ids. Translation of `SDL_GetTouchDevices()`.
pub fn touch_devices() -> Vec<TouchID> {
    with_touch(|t| t.devices.iter().map(|d| d.id).collect())
}

/// Translation of `SDL_GetTouchIndex()`.
fn touch_index(state: &TouchState, id: TouchID) -> Option<usize> {
    state.devices.iter().position(|d| d.id == id)
}

/// Translation of `SDL_GetTouch()`; `Err` when the device is unknown.
fn get_touch(state: &mut TouchState, id: TouchID) -> Result<&mut Touch> {
    match touch_index(state, id) {
        Some(index) => Ok(&mut state.devices[index]),
        None => {
            if id == MOUSE_TOUCH_ID || id == PEN_TOUCH_ID {
                // this is a virtual touch device, but for some reason they aren't added to the system. Just ignore it.
                Err(Error::new(""))
            } else if window::video().is_some_and(|v| v.reset_touch()) {
                Err(crate::err!("Unknown touch id {id}, resetting"))
            } else {
                Err(crate::err!("Unknown touch device id {id}, cannot reset"))
            }
        }
    }
}

/// The name of a touch device. Translation of `SDL_GetTouchDeviceName()`.
pub fn touch_device_name(id: TouchID) -> Result<String> {
    with_touch(|t| get_touch(t, id).map(|touch| touch.name.clone()))
}

/// The type of a touch device. Translation of `SDL_GetTouchDeviceType()`.
pub fn touch_device_type(id: TouchID) -> TouchDeviceType {
    with_touch(|t| {
        get_touch(t, id)
            .map(|touch| touch.device_type)
            .unwrap_or(TouchDeviceType::Invalid)
    })
}

/// A snapshot of the active fingers on a touch device. Translation of `SDL_GetTouchFingers()`.
pub fn touch_fingers(touch_id: TouchID) -> Result<Vec<Finger>> {
    with_touch(|t| get_touch(t, touch_id).map(|touch| touch.fingers.clone()))
}

/// Register a touch device; returns its index. Translation of `SDL_AddTouch()`.
pub fn add_touch(touch_id: TouchID, device_type: TouchDeviceType, name: &str) -> usize {
    debug_assert!(touch_id != 0);

    with_touch(|t| {
        if let Some(index) = touch_index(t, touch_id) {
            return index;
        }

        // Add the touch to the list of touch
        t.devices.push(Touch {
            id: touch_id,
            device_type,
            fingers: Vec::new(),
            name: name.to_owned(),
        });
        t.devices.len() - 1
    })
}

/// Translation of `SDL_AddFinger()`.
fn add_finger(touch: &mut Touch, fingerid: FingerID, x: f32, y: f32, pressure: f32) {
    debug_assert!(fingerid != 0);
    touch.fingers.push(Finger {
        id: fingerid,
        x,
        y,
        pressure,
    });
}

/// Translation of `SDL_DelFinger()`.
fn del_finger(touch: &mut Touch, fingerid: FingerID) {
    if let Some(index) = touch.fingers.iter().position(|f| f.id == fingerid) {
        touch.fingers.remove(index);
    }
}

/// Clamp a normalized touch position into window pixel coordinates (the
/// repeated `pos_x`/`pos_y` block in `SDL_SendTouch`/`SDL_SendTouchMotion`).
fn touch_to_window_pos(window: &WindowCore, x: f32, y: f32) -> (f32, f32) {
    let mut pos_x = x * window.w as f32;
    let mut pos_y = y * window.h as f32;
    if pos_x < 0.0 {
        pos_x = 0.0;
    }
    if pos_x > (window.w - 1) as f32 {
        pos_x = (window.w - 1) as f32;
    }
    if pos_y < 0.0 {
        pos_y = 0.0;
    }
    if pos_y > (window.h - 1) as f32 {
        pos_y = (window.h - 1) as f32;
    }
    (pos_x, pos_y)
}

/// Send a finger down/up/canceled event. Translation of `SDL_SendTouch()`.
///
/// `x`, `y` and `pressure` are normalized (0...1); `timestamp` zero means "now".
#[allow(clippy::too_many_arguments)]
pub fn send_touch(
    timestamp: Duration,
    id: TouchID,
    fingerid: FingerID,
    window_id: Option<WindowID>,
    event_type: EventType,
    x: f32,
    y: f32,
    pressure: f32,
) {
    let down = event_type == EventType::FINGER_DOWN;

    // Hold the event lock across the whole operation, like upstream.
    let _lock = lock_touch();

    if with_touch(|t| get_touch(t, id).is_err()) {
        return;
    }

    let (touch_mouse_events, mouse_touch_events, pen_touch_events) = mouse::with_mouse(|m| {
        (
            m.touch_mouse_events,
            m.mouse_touch_events,
            m.pen_touch_events,
        )
    });
    let window = window_id.and_then(window::window);

    // SDL_HINT_TOUCH_MOUSE_EVENTS: controlling whether touch events should generate synthetic mouse events
    // SDL_HINT_VITA_TOUCH_MOUSE_DEVICE: controlling which touchpad should generate synthetic mouse events, PSVita-only
    {
        // FIXME: maybe we should only restrict to a few SDL_TouchDeviceType
        if id != MOUSE_TOUCH_ID && id != PEN_TOUCH_ID && touch_mouse_events {
            let (finger_touching, tracked) = with_touch(|t| {
                (
                    t.finger_touching,
                    t.track_touchid == id && t.track_fingerid == fingerid,
                )
            });
            if let Some(window) = &window {
                if down {
                    if !finger_touching {
                        let (pos_x, pos_y) = touch_to_window_pos(window, x, y);
                        mouse::send_mouse_motion(
                            timestamp,
                            window_id,
                            TOUCH_MOUSE_ID,
                            false,
                            pos_x,
                            pos_y,
                        );
                        mouse::send_mouse_button(
                            timestamp,
                            window_id,
                            TOUCH_MOUSE_ID,
                            BUTTON_LEFT,
                            true,
                        );
                    }
                } else if finger_touching && tracked {
                    mouse::send_mouse_button(
                        timestamp,
                        window_id,
                        TOUCH_MOUSE_ID,
                        BUTTON_LEFT,
                        false,
                    );
                }
            }
            with_touch(|t| {
                if down {
                    if !t.finger_touching {
                        t.finger_touching = true;
                        t.track_touchid = id;
                        t.track_fingerid = fingerid;
                    }
                } else if t.finger_touching && t.track_touchid == id && t.track_fingerid == fingerid
                {
                    t.finger_touching = false;
                }
            });
        }
    }

    // SDL_HINT_MOUSE_TOUCH_EVENTS: if not set, discard synthetic touch events coming from platform layer
    if (!mouse_touch_events && id == MOUSE_TOUCH_ID) || (!pen_touch_events && id == PEN_TOUCH_ID) {
        return;
    }

    let finger = with_touch(|t| {
        get_touch(t, id)
            .ok()
            .and_then(|touch| touch.fingers.iter().find(|f| f.id == fingerid).copied())
    });
    if down {
        if finger.is_some() {
            /* This finger is already down.
            Assume the finger-up for the previous touch was lost, and send it. */
            send_touch(
                timestamp,
                id,
                fingerid,
                window_id,
                EventType::FINGER_CANCELED,
                x,
                y,
                pressure,
            );
        }

        with_touch(|t| {
            if let Ok(touch) = get_touch(t, id) {
                add_finger(touch, fingerid, x, y, pressure);
            }
        });

        if queue::event_enabled(event_type) {
            let _ = queue::push(Event::TouchFinger(TouchFingerEvent {
                event_type,
                timestamp,
                touch_id: id,
                finger_id: fingerid,
                x,
                y,
                dx: 0.0,
                dy: 0.0,
                pressure,
                window_id: window_id.unwrap_or(0),
            }));
        }
    } else {
        let Some(finger) = finger else {
            // This finger is already up
            return;
        };

        if queue::event_enabled(event_type) {
            let _ = queue::push(Event::TouchFinger(TouchFingerEvent {
                event_type,
                timestamp,
                touch_id: id,
                finger_id: fingerid,
                // I don't trust the coordinates passed on fingerUp
                x: finger.x,
                y: finger.y,
                dx: 0.0,
                dy: 0.0,
                pressure,
                window_id: window_id.unwrap_or(0),
            }));
        }

        with_touch(|t| {
            if let Ok(touch) = get_touch(t, id) {
                del_finger(touch, fingerid);
            }
        });
    }
}

/// Send a finger motion event. Translation of `SDL_SendTouchMotion()`.
pub fn send_touch_motion(
    timestamp: Duration,
    id: TouchID,
    fingerid: FingerID,
    window_id: Option<WindowID>,
    x: f32,
    y: f32,
    pressure: f32,
) {
    let _lock = lock_touch();

    if with_touch(|t| get_touch(t, id).is_err()) {
        return;
    }

    let (touch_mouse_events, mouse_touch_events) =
        mouse::with_mouse(|m| (m.touch_mouse_events, m.mouse_touch_events));

    // SDL_HINT_TOUCH_MOUSE_EVENTS: controlling whether touch events should generate synthetic mouse events
    if id != MOUSE_TOUCH_ID && id != PEN_TOUCH_ID && touch_mouse_events {
        if let Some(window) = window_id.and_then(window::window) {
            let tracked = with_touch(|t| {
                t.finger_touching && t.track_touchid == id && t.track_fingerid == fingerid
            });
            if tracked {
                let (pos_x, pos_y) = touch_to_window_pos(&window, x, y);
                mouse::send_mouse_motion(timestamp, window_id, TOUCH_MOUSE_ID, false, pos_x, pos_y);
            }
        }
    }

    // SDL_HINT_MOUSE_TOUCH_EVENTS: if not set, discard synthetic touch events coming from platform layer
    if !mouse_touch_events && id == MOUSE_TOUCH_ID {
        return;
    }

    let update = with_touch(|t| {
        let touch = get_touch(t, id).ok()?;
        let Some(finger) = touch.fingers.iter_mut().find(|f| f.id == fingerid) else {
            return Some(None);
        };

        let xrel = x - finger.x;
        let yrel = y - finger.y;
        let prel = pressure - finger.pressure;

        // Drop events that don't change state
        if xrel == 0.0 && yrel == 0.0 && prel == 0.0 {
            return None;
        }

        // Update internal touch coordinates
        finger.x = x;
        finger.y = y;
        finger.pressure = pressure;
        Some(Some((xrel, yrel)))
    });

    match update {
        None => {}
        Some(None) => {
            send_touch(
                timestamp,
                id,
                fingerid,
                window_id,
                EventType::FINGER_DOWN,
                x,
                y,
                pressure,
            );
        }
        // Post the event, if desired
        Some(Some((xrel, yrel))) if queue::event_enabled(EventType::FINGER_MOTION) => {
            let _ = queue::push(Event::TouchFinger(TouchFingerEvent {
                event_type: EventType::FINGER_MOTION,
                timestamp,
                touch_id: id,
                finger_id: fingerid,
                x,
                y,
                dx: xrel,
                dy: yrel,
                pressure,
                window_id: window_id.unwrap_or(0),
            }));
        }
        Some(Some(_)) => {}
    }
}

/// Unregister a touch device. Translation of `SDL_DelTouch()`.
pub fn del_touch(id: TouchID) {
    with_touch(|t| {
        if t.devices.is_empty() {
            // We've already cleaned up, we won't find this device
            return;
        }

        if let Some(index) = touch_index(t, id) {
            t.devices.swap_remove(index);
        }
    });
}

/// Shut down the touch subsystem. Translation of `SDL_QuitTouch()`.
pub fn quit_touch() {
    let ids: Vec<TouchID> = with_touch(|t| t.devices.iter().rev().map(|d| d.id).collect());
    for id in ids {
        del_touch(id);
    }
    with_touch(|t| {
        debug_assert!(t.devices.is_empty());
        t.devices = Vec::new();
    });
}

/// Send a pinch gesture event; `span_*`/`focus_*` are normalized or negative
/// for "unknown". Returns `true` if posted. Translation of `SDL_SendPinch()`.
#[allow(clippy::too_many_arguments)]
pub fn send_pinch(
    event_type: EventType,
    timestamp: Duration,
    window_id: WindowID,
    scale: f32,
    span_x: f32,
    span_y: f32,
    focus_x: f32,
    focus_y: f32,
) -> bool {
    /* Post the event, if desired */
    if queue::event_enabled(event_type) {
        let Some(window) = window::window(window_id) else {
            return false;
        };
        let scaled = |v: f32, dim: i32| if v >= 0.0 { v * dim as f32 } else { -1.0 };
        return queue::push(Event::Pinch(PinchFingerEvent {
            event_type,
            timestamp,
            scale,
            window_id,
            span_x: scaled(span_x, window.w),
            span_y: scaled(span_y, window.h),
            focus_x: scaled(focus_x, window.w),
            focus_y: scaled(focus_y, window.h),
        }))
        .unwrap_or(false);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::window::tests::with_video;
    use crate::hints;

    fn drain() -> Vec<Event> {
        queue::get_events(EventType::FIRST, EventType::LAST, 1000).unwrap()
    }

    #[test]
    fn devices_and_fingers() {
        with_video(&[1], |_| {
            mouse::pre_init_mouse().unwrap();
            assert!(!touch_devices_available());
            assert_eq!(add_touch(7, TouchDeviceType::Direct, "screen"), 0);
            assert_eq!(add_touch(7, TouchDeviceType::Direct, "dup"), 0);
            assert_eq!(add_touch(8, TouchDeviceType::IndirectAbsolute, "pad"), 1);
            assert_eq!(touch_devices(), vec![7, 8]);
            assert_eq!(touch_device_name(7).unwrap(), "screen");
            assert_eq!(touch_device_type(8), TouchDeviceType::IndirectAbsolute);
            assert_eq!(touch_device_type(9), TouchDeviceType::Invalid);
            assert!(touch_device_name(9)
                .unwrap_err()
                .message()
                .starts_with("Unknown touch device id 9"));
            drain();

            // Touch-to-mouse emulation is on by default: the first finger drives the mouse.
            send_touch(
                Duration::ZERO,
                7,
                1,
                Some(1),
                EventType::FINGER_DOWN,
                0.5,
                0.5,
                1.0,
            );
            let types: Vec<_> = drain().iter().map(|e| e.event_type()).collect();
            assert!(types.contains(&EventType::FINGER_DOWN), "{types:?}");
            assert!(types.contains(&EventType::MOUSE_BUTTON_DOWN), "{types:?}");
            assert_eq!(mouse::mouse_state().0, 320.0);
            assert_eq!(
                touch_fingers(7).unwrap(),
                vec![Finger {
                    id: 1,
                    x: 0.5,
                    y: 0.5,
                    pressure: 1.0
                }]
            );

            // Motion without change is dropped; motion of an unknown finger becomes a down.
            send_touch_motion(Duration::ZERO, 7, 1, Some(1), 0.5, 0.5, 1.0);
            assert!(drain().is_empty());
            send_touch_motion(Duration::ZERO, 7, 1, Some(1), 0.75, 0.5, 1.0);
            match drain().last() {
                Some(Event::TouchFinger(f)) => {
                    assert_eq!((f.event_type, f.dx), (EventType::FINGER_MOTION, 0.25))
                }
                other => panic!("{other:?}"),
            }
            send_touch_motion(Duration::ZERO, 7, 2, Some(1), 0.1, 0.1, 0.5);
            assert_eq!(
                drain().last().map(|e| e.event_type()),
                Some(EventType::FINGER_DOWN)
            );
            assert_eq!(touch_fingers(7).unwrap().len(), 2);

            // A duplicate down cancels the previous touch first.
            send_touch(
                Duration::ZERO,
                7,
                2,
                Some(1),
                EventType::FINGER_DOWN,
                0.2,
                0.2,
                0.5,
            );
            let types: Vec<_> = drain().iter().map(|e| e.event_type()).collect();
            assert_eq!(
                types,
                vec![EventType::FINGER_CANCELED, EventType::FINGER_DOWN]
            );

            // Finger up reports the last known position and releases the emulated mouse button.
            send_touch(
                Duration::ZERO,
                7,
                1,
                Some(1),
                EventType::FINGER_UP,
                0.0,
                0.0,
                0.0,
            );
            let events = drain();
            // The emulated mouse release goes out before the finger event, as upstream.
            assert_eq!(events[0].event_type(), EventType::MOUSE_BUTTON_UP);
            match events.iter().find(|e| matches!(e, Event::TouchFinger(_))) {
                Some(Event::TouchFinger(f)) => {
                    assert_eq!((f.event_type, f.x, f.y), (EventType::FINGER_UP, 0.75, 0.5))
                }
                other => panic!("{other:?}"),
            }
            send_touch(
                Duration::ZERO,
                7,
                1,
                Some(1),
                EventType::FINGER_UP,
                0.0,
                0.0,
                0.0,
            );
            assert!(drain().is_empty(), "already up");

            // With the hint off, touches don't drive the mouse.
            hints::set(hints::TOUCH_MOUSE_EVENTS, "0").unwrap();
            send_touch(
                Duration::ZERO,
                7,
                3,
                Some(1),
                EventType::FINGER_DOWN,
                0.5,
                0.5,
                1.0,
            );
            let types: Vec<_> = drain().iter().map(|e| e.event_type()).collect();
            assert_eq!(types, vec![EventType::FINGER_DOWN]);
            hints::reset(hints::TOUCH_MOUSE_EVENTS);

            assert!(send_pinch(
                EventType::PINCH_UPDATE,
                Duration::ZERO,
                1,
                1.5,
                0.5,
                -1.0,
                0.25,
                0.25
            ));
            match drain().pop() {
                Some(Event::Pinch(p)) => assert_eq!(
                    (p.scale, p.span_x, p.span_y, p.focus_x),
                    (1.5, 320.0, -1.0, 160.0)
                ),
                other => panic!("{other:?}"),
            }

            del_touch(7);
            assert_eq!(touch_devices(), vec![8]);
            quit_touch();
            assert!(!touch_devices_available());
            mouse::quit_mouse();
        });
    }
}
