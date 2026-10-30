// Rust translation of src/video/android/SDL_androidpen.c and
// SDL_androidpen.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The pens: a stylus's (or its eraser's) events from the surface's touch
//! events and its motion listener's hovering (`onNativePen()`), a pen for
//! each pointer ID, added when it's first seen, with its position,
//! pressure, buttons, touches and proximity.

use std::sync::Arc;
use std::time::Duration;

use crate::events::pen::{
    add_pen_device, find_pen_by_callback, pen_status, send_pen_axis, send_pen_button,
    send_pen_motion, send_pen_proximity, send_pen_touch, PenAxis, PenCapabilityFlags,
    PenDeviceType, PenInfo, PenInputFlags, PenSubtype,
};
use crate::events::WindowID;

const ACTION_DOWN: i32 = 0;
const ACTION_UP: i32 = 1;
const ACTION_CANCEL: i32 = 3;
const ACTION_POINTER_DOWN: i32 = 5;
const ACTION_POINTER_UP: i32 = 6;
const ACTION_HOVER_ENTER: i32 = 9;
const ACTION_HOVER_EXIT: i32 = 10;

/// `SDL_PenDeviceType` of `onNativePen()`'s `device_type`.
fn pen_device_type(device_type: i32) -> PenDeviceType {
    match device_type {
        1 => PenDeviceType::Direct,
        2 => PenDeviceType::Indirect,
        -1 => PenDeviceType::Invalid,
        _ => PenDeviceType::Unknown,
    }
}

/// A pen's event: its pointer ID, its device's type, its buttons (the
/// `SDL_PenInputFlags` of the buttons pressed, with the tip's or the
/// eraser's flag), the motion event's action, its position and pressure.
/// Translation of `Android_OnPen()`.
#[allow(clippy::too_many_arguments)] // (upstream's)
pub(crate) fn on_pen(
    window: Option<WindowID>,
    pen_id_in: i32,
    device_type: i32,
    button: i32,
    action: i32,
    x: f32,
    y: f32,
    p: f32,
) {
    let Some(window) = window else {
        return;
    };

    // pointer index starts from zero.
    let pen_id_in = pen_id_in.wrapping_add(1);

    // (the handle is the pointer ID, as upstream's pointer-sized number:
    // found by its value)
    let handle = pen_id_in as isize as usize;
    let mut pen = find_pen_by_callback(|h| h.downcast_ref::<usize>() == Some(&handle));
    if pen == 0 {
        // TODO: Query JNI for pen device info
        let peninfo = PenInfo {
            capabilities: PenCapabilityFlags(
                PenCapabilityFlags::PRESSURE.0 | PenCapabilityFlags::ERASER.0,
            ),
            num_buttons: 2,
            subtype: PenSubtype::Pen,
            device_type: pen_device_type(device_type),
            ..PenInfo::default()
        };
        pen = add_pen_device(
            Duration::ZERO,
            None,
            Some(window),
            Some(&peninfo),
            Arc::new(handle),
            true,
        );
        if pen == 0 {
            crate::log::log(
                crate::log::Category::Application,
                crate::log::Priority::Info,
                &format!("error: can't add a pen device {pen_id_in}"),
            );
            return;
        }
    }

    send_pen_motion(Duration::ZERO, pen, Some(window), x, y);
    send_pen_axis(Duration::ZERO, pen, Some(window), PenAxis::Pressure, p);
    // TODO: add more axis

    let (current, _) = pen_status(pen);
    let diff = current.0 as i32 ^ button;
    if diff != 0 {
        for i in 1..=5u8 {
            let mask = 1i32 << i;
            if diff & mask != 0 {
                send_pen_button(Duration::ZERO, pen, Some(window), i, (button & mask) != 0);
            }
        }
    }

    // button contains DOWN/ERASER_TIP on DOWN/UP regardless of pressed state, use action to distinguish
    // we don't compare tip flags above because MotionEvent.getButtonState doesn't return stylus tip/eraser state.
    let eraser = (button & PenInputFlags::ERASER_TIP.0 as i32) != 0;
    match action {
        ACTION_HOVER_ENTER => send_pen_proximity(Duration::ZERO, pen, Some(window), true, true),

        // strictly speaking, this can mean both "proximity out" and "left the View" but close enough.
        ACTION_CANCEL | ACTION_HOVER_EXIT => {
            send_pen_proximity(Duration::ZERO, pen, Some(window), false, false)
        }

        ACTION_DOWN | ACTION_POINTER_DOWN => {
            send_pen_touch(Duration::ZERO, pen, Some(window), eraser, true)
        }

        ACTION_UP | ACTION_POINTER_UP => {
            send_pen_touch(Duration::ZERO, pen, Some(window), eraser, false)
        }

        _ => {}
    }
}
