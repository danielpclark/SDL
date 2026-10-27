// Rust translation of src/video/uikit/SDL_uikitpen.m and SDL_uikitpen.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Apple Pencil: one pen ("Apple Pencil", a direct pencil with
//! pressure and tilt, with distance from iOS 16.1 and rotation from
//! iOS 17.5), added when it first touches or hovers over a view (UIKit
//! only tells of a paired Pencil through its events) and removed when the
//! video subsystem quits. Its touches ([`handle_pen_press`],
//! [`handle_pen_motion`], [`handle_pen_release`]) and its hovering
//! ([`handle_pen_hover`], iOS 13, with the distance from iOS 16.1 and the
//! angles from iOS 16.4) send its position, pressure, tilt (from the
//! azimuth and altitude angles), rotation (the roll angle) and distance.
//! Not on tvOS.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::core::apple::foundation::{available, CGFloat, CGPoint, NSTimeInterval};
use crate::core::apple::objc::{msg_send, Id};
use crate::events::pen::{
    add_pen_device, pen_status, remove_pen_device, send_pen_axis, send_pen_motion, send_pen_touch,
    PenAxis, PenCapabilityFlags, PenDeviceType, PenID, PenInfo, PenInputFlags, PenSubtype,
};
use crate::events::WindowID;

use super::events::get_event_timestamp;
use super::view::get_sdl_window;

/// Translation of `apple_pencil_id` (0: none yet).
static APPLE_PENCIL_ID: AtomicU32 = AtomicU32::new(0);

/// Translation of `UIKit_InitPen()` (which upstream doesn't call).
#[allow(dead_code)]
pub(crate) fn init_pen() -> bool {
    true
}

/// The Pencil's ID, adding it first if it isn't there yet. Translation of
/// `UIKit_AddPenIfNecesary()`.
// we only have one Apple Pencil at a time, and it must be paired to the iOS device.
// We only know about its existence when it first sends an event, so add an single SDL pen
// device here if we haven't already.
fn add_pen_if_necessary(window: Option<WindowID>) -> PenID {
    let id = APPLE_PENCIL_ID.load(Ordering::Relaxed);
    if id != 0 {
        return id;
    }
    let mut capabilities =
        PenCapabilityFlags::PRESSURE.0 | PenCapabilityFlags::XTILT.0 | PenCapabilityFlags::YTILT.0;

    if available!(ios(17, 5)) {
        // need rollAngle method.
        capabilities |= PenCapabilityFlags::ROTATION.0;
    }

    if available!(ios(16, 1)) {
        // need zOffset method.
        capabilities |= PenCapabilityFlags::DISTANCE.0;
    }

    // Apple Pencil and iOS can report when the pencil is being "squeezed" but it's a boolean thing,
    // so we can't use it for tangential pressure.

    let info = PenInfo {
        capabilities: PenCapabilityFlags(capabilities),
        max_tilt: 90,
        num_buttons: 0,
        subtype: PenSubtype::Pencil,
        device_type: PenDeviceType::Direct, // Apple Pencil on iOS is always a direct device; it works on the tablet's screen.
        ..PenInfo::default()
    };

    // There's only ever one Apple Pencil at most, so we just pass a non-zero value for the handle.
    let id = add_pen_device(
        Duration::ZERO,
        Some("Apple Pencil"),
        window,
        Some(&info),
        Arc::new(0x1usize),
        true,
    );
    APPLE_PENCIL_ID.store(id, Ordering::Relaxed);
    id
}

/// Translation of `UIKit_HandlePenAxes()`.
#[allow(clippy::too_many_arguments)]
fn handle_pen_axes(
    window: Option<WindowID>,
    nstimestamp: NSTimeInterval,
    z_offset: f32,
    point: &CGPoint,
    force: f32,
    maximum_possible_force: f32,
    azimuth_angle_in_view: f32,
    altitude_angle: f32,
    roll_angle: f32,
) {
    let pen_id = add_pen_if_necessary(window);
    if pen_id != 0 {
        let timestamp = get_event_timestamp(nstimestamp);
        let radians_to_degrees = 180.0f32 / std::f32::consts::PI;

        // Normalize force to 0.0f ... 1.0f range.
        let pressure = force / maximum_possible_force;

        // azimuthAngleInView is in radians, with 0 being the pen's back end pointing due east on the screen when the
        // tip is touching the screen, and negative when heading north from there, positive to the south.
        // So convert to degrees, 0 being due east, etc.
        let azimuth_angle = azimuth_angle_in_view * radians_to_degrees;

        // altitudeAngle is in radians, with 0 being the pen laying flat on (parallel to) the device
        //  screen and PI/2 being it pointing straight up from (perpendicular to) the device screen.
        // So convert to degrees, 0 being flat and 90 being straight up.
        let altitude_angle = altitude_angle * radians_to_degrees;

        // the azimuth_angle goes from -180 to 180 (with abs(angle) moving from 180 to 0, left to right), but SDL wants
        // it from -90 (back facing west) to 90 (back facing east).
        let xtilt = (180.0 - azimuth_angle.abs()) - 90.0;

        // the altitude_angle goes from 0 to 90 regardless of which direction the pen is lifting off the device, but SDL wants
        // it from -90 (flat facing north) to 90 (flat facing south).
        let ytilt = if azimuth_angle < 0.0 {
            -(90.0 - altitude_angle)
        } else {
            90.0 - altitude_angle
        };

        // rotation is in radians, and only available on a later iOS.
        let rotation = roll_angle * radians_to_degrees; // !!! FIXME: this might need adjustment, I don't have a pencil that supports it.

        if force == 0.0 && pen_status(pen_id).0.contains(PenInputFlags::DOWN) {
            // The first hover as the pen is being released has a stale position, so ignore it
            return;
        }

        send_pen_motion(timestamp, pen_id, window, point.x as f32, point.y as f32);
        send_pen_axis(timestamp, pen_id, window, PenAxis::Pressure, pressure);
        send_pen_axis(timestamp, pen_id, window, PenAxis::XTilt, xtilt);
        send_pen_axis(timestamp, pen_id, window, PenAxis::YTilt, ytilt);
        send_pen_axis(timestamp, pen_id, window, PenAxis::Rotation, rotation);
        send_pen_axis(timestamp, pen_id, window, PenAxis::Distance, z_offset);
    }
}

/// The Pencil hovering over a view (its `UIHoverGestureRecognizer` began
/// or changed). Translation of `UIKit_HandlePenHover()`.
#[cfg(not(target_os = "tvos"))]
pub(crate) fn handle_pen_hover(view: Id, recognizer: Id) {
    // SAFETY: UIHoverGestureRecognizer's properties and methods, each
    // asked for only on the version that has it.
    unsafe {
        let mut z_offset = 0.0f32;
        if available!(ios(16, 1)) {
            let value: CGFloat = msg_send![recognizer, zOffset];
            z_offset = value as f32;
        }

        let mut azimuth_angle_in_view = 0.0f32;
        if available!(ios(16, 4)) {
            let value: CGFloat = msg_send![recognizer, azimuthAngleInView: view];
            azimuth_angle_in_view = value as f32;
        }

        let mut altitude_angle = 0.0f32;
        if available!(ios(16, 4)) {
            let value: CGFloat = msg_send![recognizer, altitudeAngle];
            altitude_angle = value as f32;
        }

        let mut roll_angle = 0.0f32;
        if available!(ios(17, 5)) {
            let value: CGFloat = msg_send![recognizer, rollAngle];
            roll_angle = value as f32;
        }

        let window = get_sdl_window(view);
        let point: CGPoint = msg_send![recognizer, locationInView: view];

        // force is zero here; if you're here, you're not touching.
        // !!! FIXME: no timestamp on these...?
        handle_pen_axes(
            window,
            0.0,
            z_offset,
            &point,
            0.0,
            1.0,
            azimuth_angle_in_view,
            altitude_angle,
            roll_angle,
        );
    }
}

/// Translation of `UIKit_HandlePenAxesFromUITouch()`.
fn handle_pen_axes_from_uitouch(view: Id, pencil: Id) {
    // SAFETY: UITouch's properties and methods, each asked for only on the
    // version that has it.
    unsafe {
        #[allow(unused_mut)]
        let mut roll_angle = 0.0f32;
        #[cfg(not(target_os = "tvos"))]
        if available!(ios(17, 5)) {
            let value: CGFloat = msg_send![pencil, rollAngle];
            roll_angle = value as f32;
        }

        let window = get_sdl_window(view);
        let point: CGPoint = msg_send![pencil, locationInView: view];
        let timestamp: NSTimeInterval = msg_send![pencil, timestamp];
        let force: CGFloat = msg_send![pencil, force];
        let maximum_possible_force: CGFloat = msg_send![pencil, maximumPossibleForce];
        let azimuth_angle_in_view: CGFloat = msg_send![pencil, azimuthAngleInView: view];
        let altitude_angle: CGFloat = msg_send![pencil, altitudeAngle];

        // zOffset is zero here; if you're here, you're touching.
        handle_pen_axes(
            window,
            timestamp,
            0.0,
            &point,
            force as f32,
            maximum_possible_force as f32,
            azimuth_angle_in_view as f32,
            altitude_angle as f32,
            roll_angle,
        );
    }
}

/// The Pencil moving on a view. Translation of `UIKit_HandlePenMotion()`.
pub(crate) fn handle_pen_motion(view: Id, pencil: Id) {
    handle_pen_axes_from_uitouch(view, pencil);
}

/// The Pencil touching a view. Translation of `UIKit_HandlePenPress()`.
pub(crate) fn handle_pen_press(view: Id, pencil: Id) {
    let window = get_sdl_window(view);
    let pen_id = add_pen_if_necessary(window);
    if pen_id != 0 {
        handle_pen_axes_from_uitouch(view, pencil);
        // SAFETY: a UITouch property.
        let timestamp: NSTimeInterval = unsafe { msg_send![pencil, timestamp] };
        send_pen_touch(get_event_timestamp(timestamp), pen_id, window, false, true);
    }
}

/// The Pencil leaving a view (or its touch cancelled). Translation of
/// `UIKit_HandlePenRelease()`.
pub(crate) fn handle_pen_release(view: Id, pencil: Id) {
    let window = get_sdl_window(view);
    let pen_id = add_pen_if_necessary(window);
    if pen_id != 0 {
        // SAFETY: a UITouch property.
        let timestamp: NSTimeInterval = unsafe { msg_send![pencil, timestamp] };
        send_pen_touch(get_event_timestamp(timestamp), pen_id, window, false, false);
        handle_pen_axes_from_uitouch(view, pencil);
    }
}

/// Remove the Pencil (if it came). Translation of `UIKit_QuitPen()`.
pub(crate) fn quit_pen() {
    let id = APPLE_PENCIL_ID.swap(0, Ordering::Relaxed);
    if id != 0 {
        remove_pen_device(Duration::ZERO, None, id);
    }
}
