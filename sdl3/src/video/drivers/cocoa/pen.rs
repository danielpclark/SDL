// Rust translation of src/video/cocoa/SDL_cocoapen.m and SDL_cocoapen.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Pens (tablets): proximity events add the pens (by device and tool ID,
//! never removed until shutdown, since Apple gives no indication when they
//! are gone) and point events carry their position, buttons, pressure,
//! rotation and tilt.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::events::get_event_timestamp;
use super::sys::*;
use super::window::WindowData;
use crate::core::apple::foundation::NSPoint;
use crate::core::apple::objc::{msg_send, Bool, Id, NSUInteger};
use crate::error::Result;
use crate::events::pen::{
    add_pen_device, find_pen_by_callback, remove_all_pen_devices, send_pen_axis, send_pen_button,
    send_pen_motion, send_pen_proximity, send_pen_touch, PenAxis, PenCapabilityFlags, PenID,
    PenInfo, PenSubtype,
};

/// Translation of `Cocoa_InitPen()`.
pub(crate) fn init_pen() -> Result<()> {
    Ok(())
}

/// Translation of `Cocoa_PenHandle`.
struct PenHandle {
    deviceid: NSUInteger,
    toolid: NSUInteger,
    pen: std::sync::atomic::AtomicU32,
    is_eraser: AtomicBool,
}

/// Translation of `Cocoa_FindPenByDeviceID()` (with
/// `FindPenByDeviceAndToolID()`): the pen, after `f` saw its handle.
fn find_pen_by_device_id(
    deviceid: NSUInteger,
    toolid: NSUInteger,
    f: impl Fn(&PenHandle),
) -> Option<PenID> {
    let pen = find_pen_by_callback(|handle| {
        let Some(cocoa_handle) = handle.downcast_ref::<PenHandle>() else {
            return false;
        };
        if cocoa_handle.deviceid != deviceid || cocoa_handle.toolid != toolid {
            return false;
        }
        f(cocoa_handle);
        true
    });
    (pen != 0).then_some(pen)
}

/// Translation of `Cocoa_HandlePenProximityEvent()`.
fn handle_pen_proximity_event(data: &WindowData, event: Id) {
    // SAFETY: a tablet proximity NSEvent.
    let (devid, toolid, entering, timestamp): (NSUInteger, NSUInteger, Bool, f64) = unsafe {
        (
            msg_send![event, deviceID],
            msg_send![event, pointingDeviceID],
            msg_send![event, isEnteringProximity],
            msg_send![event, timestamp],
        )
    };
    let timestamp = get_event_timestamp(timestamp);

    if entering.is_true() {
        // new pen coming!
        // SAFETY: as above.
        let devtype: NSUInteger = unsafe { msg_send![event, pointingDeviceType] };
        let is_eraser = devtype == NSPointingDeviceTypeEraser;
        let is_pen = devtype == NSPointingDeviceTypePen;
        if !is_eraser && !is_pen {
            return; // we ignore other things, which hopefully is right.
        }

        if let Some(pen) = find_pen_by_device_id(devid, toolid, |handle| {
            handle.is_eraser.store(is_eraser, Ordering::Relaxed); // in case this changed.
        }) {
            send_pen_proximity(timestamp, pen, Some(data.window), true, true);
            return; // already have this one.
        }

        // Cocoa offers almost none of this information as specifics, but can without warning offer any of these specific things.
        let capabilities = PenCapabilityFlags(
            PenCapabilityFlags::PRESSURE.0
                | PenCapabilityFlags::ROTATION.0
                | PenCapabilityFlags::XTILT.0
                | PenCapabilityFlags::YTILT.0
                | PenCapabilityFlags::TANGENTIAL_PRESSURE.0
                | if is_eraser {
                    PenCapabilityFlags::ERASER.0
                } else {
                    0
                },
        );
        let peninfo = PenInfo {
            capabilities,
            max_tilt: 90,
            num_buttons: 2,
            subtype: if is_eraser {
                PenSubtype::Eraser
            } else {
                PenSubtype::Pen
            },
            ..PenInfo::default()
        };

        let handle = Arc::new(PenHandle {
            deviceid: devid,
            toolid,
            pen: std::sync::atomic::AtomicU32::new(0),
            is_eraser: AtomicBool::new(is_eraser),
        });
        let pen = add_pen_device(
            timestamp,
            None,
            Some(data.window),
            Some(&peninfo),
            handle.clone(),
            true,
        );
        handle.pen.store(pen, Ordering::Relaxed);
        // (a pen that couldn't be added is dropped with its handle)
    } else {
        // old pen leaving!
        if let Some(pen) = find_pen_by_device_id(devid, toolid, |_| {}) {
            // We never remove pens (until shutdown), since Apple gives no indication when they are actually gone.
            // But unless you are plugging and unplugging a tablet millions of times, generating new device IDs, this shouldn't be a massive memory drain.
            send_pen_proximity(timestamp, pen, Some(data.window), false, false);
        }
    }
}

/// Translation of `Cocoa_HandlePenPointEvent()`.
fn handle_pen_point_event(data: &WindowData, event: Id) {
    // SAFETY: a tablet NSEvent.
    let (timestamp, deviceid, toolid) = unsafe {
        let t: f64 = msg_send![event, timestamp];
        let d: NSUInteger = msg_send![event, deviceID];
        let p: NSUInteger = msg_send![event, pointingDeviceID];
        (get_event_timestamp(t), d, p)
    };
    let is_eraser = AtomicBool::new(false);
    let Some(pen) = find_pen_by_device_id(deviceid, toolid, |h| {
        is_eraser.store(h.is_eraser.load(Ordering::Relaxed), Ordering::Relaxed)
    }) else {
        return;
    };
    let is_eraser = is_eraser.load(Ordering::Relaxed);

    // SAFETY: as above.
    let (buttons, tilt, point, pressure, rotation, tangential): (
        NSUInteger,
        NSPoint,
        NSPoint,
        f32,
        f32,
        f32,
    ) = unsafe {
        (
            msg_send![event, buttonMask],
            msg_send![event, tilt],
            msg_send![event, locationInWindow],
            msg_send![event, pressure],
            msg_send![event, rotation],
            msg_send![event, tangentialPressure],
        )
    };
    let is_touching = (buttons & NSEventButtonMaskPenTip) != 0;
    let window = Some(data.window);
    let h = crate::video::core::with_window(data.window, |w| w.core.h).unwrap_or(0);

    send_pen_touch(timestamp, pen, window, is_eraser, is_touching);
    send_pen_motion(
        timestamp,
        pen,
        window,
        point.x as f32,
        (h as f64 - point.y) as f32,
    );
    send_pen_button(
        timestamp,
        pen,
        window,
        1,
        (buttons & NSEventButtonMaskPenLowerSide) != 0,
    );
    send_pen_button(
        timestamp,
        pen,
        window,
        2,
        (buttons & NSEventButtonMaskPenUpperSide) != 0,
    );
    send_pen_axis(timestamp, pen, window, PenAxis::Pressure, pressure);
    send_pen_axis(timestamp, pen, window, PenAxis::Rotation, rotation);
    send_pen_axis(
        timestamp,
        pen,
        window,
        PenAxis::XTilt,
        (tilt.x as f32) * 90.0,
    );
    send_pen_axis(
        timestamp,
        pen,
        window,
        PenAxis::YTilt,
        (-tilt.y as f32) * 90.0,
    );
    send_pen_axis(
        timestamp,
        pen,
        window,
        PenAxis::TangentialPressure,
        tangential,
    );
}

/// Handle an event if it is a tablet one. Translation of
/// `Cocoa_HandlePenEvent()`.
pub(crate) fn handle_pen_event(data: &WindowData, event: Id) -> bool {
    // SAFETY: an NSEvent.
    let mut ty: NSUInteger = unsafe { msg_send![event, type] };

    if ty != NSEventTypeTabletPoint && ty != NSEventTypeTabletProximity {
        // SAFETY: as above (mouse events have a subtype).
        let subtype: i16 = unsafe { msg_send![event, subtype] };
        if subtype == NSEventSubtypeTabletPoint {
            ty = NSEventTypeTabletPoint;
        } else if subtype == NSEventSubtypeTabletProximity {
            ty = NSEventTypeTabletProximity;
        } else {
            return false; // not a tablet event.
        }
    }

    if ty == NSEventTypeTabletPoint {
        handle_pen_point_event(data, event);
    } else if ty == NSEventTypeTabletProximity {
        handle_pen_proximity_event(data, event);
    } else {
        return false; // not a tablet event.
    }

    true
}

/// Translation of `Cocoa_QuitPen()` (the handles are freed with the pens).
pub(crate) fn quit_pen() {
    remove_all_pen_devices(|_, _| {});
}
