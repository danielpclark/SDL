// Rust translation of src/video/x11/SDL_x11xinput2.c and
// SDL_x11xinput2.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! XInput2: multiple mice and keyboards, raw (relative) motion, smooth
//! scrolling, multitouch, pinch gestures and pens.
//!
//! The XInput2 state is file-level static state upstream; it is a
//! `static` here too (it outlives a video device, like upstream's).

use std::ffi::{c_int, c_uchar};
use std::sync::Mutex;
use std::time::Duration;

use super::pen::{
    x11_find_pen_by_device_id, x11_pen_axes_from_valuators, x11_remove_pen_by_device_id,
    SDL_X11_PEN_AXIS_VALUATOR_MISSING,
};
use super::sys::*;
use super::video::X11Video;
use super::window::with_x11_window;
use crate::events::mouse::{self, MouseID, MouseWheelDirection};
use crate::events::pen::{
    send_pen_axis, send_pen_button, send_pen_motion, send_pen_touch, PenAxis,
};
use crate::events::touch::{self, TouchDeviceType, TouchID};
use crate::events::{keyboard, EventType, KeyboardID, WindowID};
use crate::hints;
use crate::stdlib::math::fabs;
use crate::video::core::with_window;

#[allow(dead_code)] // (defined but unused upstream)
const MAX_AXIS: usize = 16;

/// Translation of `SDL_XInput2ScrollInfo`.
#[derive(Clone, Copy, Default, Debug)]
struct XInput2ScrollInfo {
    number: c_int,
    scroll_type: c_int,
    prev_value: f64,
    increment: f64,
    prev_value_valid: bool,
}

/// Translation of `SDL_XInput2ScrollableDevice`.
#[derive(Clone, Default, Debug)]
struct XInput2ScrollableDevice {
    scroll_info: Vec<XInput2ScrollInfo>,
}

/// Translation of `SDL_XInput2RelativeDevice`.
#[derive(Clone, Copy, Default, Debug)]
struct XInput2RelativeDevice {
    number: [c_int; 2],
    relative: [bool; 2],
    prev_coord_valid: [bool; 2],
    minval: [f64; 2],
    maxval: [f64; 2],
    prev_coords: [f64; 2],
}

/// Translation of `SDL_XInput2DeviceInfo` (the linked list is a `Vec`,
/// most recently used first).
#[derive(Clone, Default, Debug)]
struct XInput2DeviceInfo {
    device_id: c_int,
    relative: XInput2RelativeDevice,
    scroll: XInput2ScrollableDevice,
}

/// The file-level statics of `SDL_x11xinput2.c`.
struct XInput2State {
    initialized: bool,
    grabbed_touch_raised: bool,
    active_touch_count: c_int,
    scrolling_supported: bool,
    multitouch_supported: bool,
    gesture_supported: bool,

    /// Opcode returned X11_XQueryExtension
    /// It will be used in event processing
    /// to know that the event came from
    /// this extension
    opcode: c_int,

    rel_x_atom: Atom,
    rel_y_atom: Atom,
    abs_x_atom: Atom,
    abs_y_atom: Atom,

    /// Pointer button remapping table
    pointer_button_map: Vec<c_uchar>,

    device_info: Vec<XInput2DeviceInfo>,
}

static XINPUT2: Mutex<XInput2State> = Mutex::new(XInput2State {
    initialized: false,
    grabbed_touch_raised: false,
    active_touch_count: 0,
    scrolling_supported: false,
    multitouch_supported: false,
    gesture_supported: false,
    opcode: 0,
    rel_x_atom: 0,
    rel_y_atom: 0,
    abs_x_atom: 0,
    abs_y_atom: 0,
    pointer_button_map: Vec::new(),
    device_info: Vec::new(),
});

fn state() -> std::sync::MutexGuard<'static, XInput2State> {
    XINPUT2.lock().unwrap_or_else(|e| e.into_inner())
}

/// The valuator mask and values of an event.
///
/// # Safety
///
/// `v` must come from an XInput2 event whose data is still valid.
unsafe fn valuators<'a>(v: &XIValuatorState) -> (&'a [u8], &'a [f64]) {
    // SAFETY: the caller's contract: `mask` has `mask_len` bytes and
    // `values` one entry per set bit.
    unsafe {
        let mask = if v.mask.is_null() {
            &[][..]
        } else {
            std::slice::from_raw_parts(v.mask, v.mask_len.max(0) as usize)
        };
        let count = mask.iter().map(|b| b.count_ones() as usize).sum();
        let values = if v.values.is_null() {
            &[][..]
        } else {
            std::slice::from_raw_parts(v.values, count)
        };
        (mask, values)
    }
}

/// Translation of `parse_relative_valuators()`.
///
/// # Safety
///
/// `rawev` must be an `XI_RawMotion` event whose data is still valid.
#[allow(clippy::needless_range_loop)] // (the index is a valuator number too, as upstream)
unsafe fn parse_relative_valuators(rel_dev: &mut XInput2RelativeDevice, rawev: &XIRawEvent) {
    let mut processed_coords = [0.0f64; 2];
    let mut values_i = 0;
    let mut found = 0;

    // Use the raw values if a custom transform function is set, or the relative system scale hint is unset.
    let use_raw_vals =
        mouse::with_mouse(|m| m.input_transform.is_some() || !m.enable_relative_system_scale);

    // SAFETY: the caller's contract.
    let (mask, values) = unsafe { valuators(&rawev.valuators) };
    let raw_values = if rawev.raw_values.is_null() {
        &[][..]
    } else {
        // SAFETY: as for `values`: one raw value per set mask bit.
        unsafe { std::slice::from_raw_parts(rawev.raw_values, values.len()) }
    };

    let mut i = 0;
    while i < (rawev.valuators.mask_len.max(0) as usize) * 8 && found < 2 {
        if !XIMaskIsSet(mask, i as c_int) {
            i += 1;
            continue;
        }

        for j in 0..2 {
            if rel_dev.number[j] == i as c_int {
                let current_val = if use_raw_vals {
                    raw_values.get(values_i).copied().unwrap_or(0.0)
                } else {
                    values.get(values_i).copied().unwrap_or(0.0)
                };

                if rel_dev.relative[j] {
                    processed_coords[j] = current_val;
                } else {
                    // The first absolute value is meaningless by itself and must be ignored, as it only establishes a baseline for future deltas.
                    if rel_dev.prev_coord_valid[j] {
                        processed_coords[j] = current_val - rel_dev.prev_coords[j];
                        // convert absolute to relative
                    }
                    rel_dev.prev_coords[j] = current_val;
                    rel_dev.prev_coord_valid[j] = true;
                }
                found += 1;

                break;
            }
        }

        values_i += 1;
        i += 1;
    }

    // Relative mouse motion is delivered to the window with keyboard focus
    let (relative_mode, focus) = mouse::with_mouse(|m| (m.relative_mode, m.focus));
    if relative_mode && keyboard::keyboard_focus().is_some() {
        // FIXME (upstream): the X server time (milliseconds) is passed as
        // an SDL timestamp (nanoseconds).
        mouse::send_mouse_motion(
            Duration::from_nanos(rawev.time),
            focus,
            rawev.sourceid as MouseID,
            true,
            processed_coords[0] as f32,
            processed_coords[1] as f32,
        );
    }
}

/// Translation of `xinput2_version_atleast()`.
fn xinput2_version_atleast(version: c_int, wantmajor: c_int, wantminor: c_int) -> bool {
    version >= ((wantmajor * 1000) + wantminor)
}

/// Translation of `xinput2_parse_scrollable_valuators()`.
///
/// # Safety
///
/// `xev` must be an XInput2 device event whose data is still valid.
unsafe fn xinput2_parse_scrollable_valuators(
    scroll_dev: &mut XInput2ScrollableDevice,
    xev: &XIDeviceEvent,
) {
    // SAFETY: the caller's contract.
    let (mask, values) = unsafe { valuators(&xev.valuators) };
    let mut wheel = Vec::new();
    let mut values_i = 0;
    for j in 0..(xev.valuators.mask_len.max(0) as usize) * 8 {
        if !XIMaskIsSet(mask, j as c_int) {
            continue;
        }

        for info in scroll_dev.scroll_info.iter_mut() {
            if info.number == j as c_int {
                let current_val = values.get(values_i).copied().unwrap_or(0.0);
                let delta = (info.prev_value - current_val) / info.increment;
                /* Ignore very large jumps that can happen as a result of overflowing
                 * the maximum range, as the driver will reset the position to zero
                 * at "something that's close to 2^32".
                 *
                 * The first scroll event is meaningless by itself and must be discarded,
                 * as it is only useful for establishing a baseline for future deltas.
                 * This is a known deficiency of the XInput2 scroll protocol, and,
                 * unfortunately, there is nothing we can do about it.
                 *
                 * http://who-t.blogspot.com/2012/06/xi-21-protocol-design-issues.html
                 */
                if info.prev_value_valid && fabs(delta) < i32::MAX as f64 * 0.95 {
                    let x = if info.scroll_type == XIScrollTypeHorizontal {
                        delta
                    } else {
                        0.0
                    };
                    let y = if info.scroll_type == XIScrollTypeVertical {
                        delta
                    } else {
                        0.0
                    };

                    wheel.push((x, y));
                }
                info.prev_value = current_val;
                info.prev_value_valid = true;
            }
        }

        values_i += 1;
    }

    // (sent once the state is no longer borrowed)
    for (x, y) in wheel {
        let focus = mouse::mouse_focus();
        // FIXME (upstream): as in parse_relative_valuators(), the X time is
        // taken for nanoseconds.
        mouse::send_mouse_wheel(
            Duration::from_nanos(xev.time),
            focus,
            xev.sourceid as MouseID,
            -x as f32,
            y as f32,
            MouseWheelDirection::Normal,
        );
    }
}

/// Translation of `xinput2_normalize_touch_coordinates()`.
fn xinput2_normalize_touch_coordinates(
    window: Option<WindowID>,
    in_x: f64,
    in_y: f64,
) -> (f32, f32) {
    let size = window.and_then(|w| with_window(w, |w| (w.core.w, w.core.h)).ok());
    if let Some((w, h)) = size {
        let out_x = if w == 1 {
            0.5
        } else {
            in_x as f32 / (w - 1) as f32
        };
        let out_y = if h == 1 {
            0.5
        } else {
            in_y as f32 / (h - 1) as f32
        };
        (out_x, out_y)
    } else {
        // couldn't find the window...
        (in_x as f32, in_y as f32)
    }
}

/// xi2 device went away? take it out of the list.
/// Translation of `xinput2_remove_device_info()`.
fn xinput2_remove_device_info(device_id: c_int) {
    let mut st = state();
    if let Some(i) = st.device_info.iter().position(|d| d.device_id == device_id) {
        st.device_info.remove(i);
    }
}

/// Translation of `xinput2_reset_device_valuators()`.
fn xinput2_reset_device_valuators() {
    for devinfo in state().device_info.iter_mut() {
        for info in devinfo.scroll.scroll_info.iter_mut() {
            info.prev_value_valid = false;
        }
        devinfo.relative.prev_coord_valid[0] = false;
        devinfo.relative.prev_coord_valid[1] = false;
    }
}

/// Translation of `xinput2_update_device_info()`.
///
/// # Safety
///
/// `classes` must be the class list of a device from XInput2.
unsafe fn xinput2_update_device_info(
    st: &XInput2State,
    devinfo: &mut XInput2DeviceInfo,
    classes: &[*mut XIAnyClassInfo],
) {
    let mut have_rel_x = false;
    let mut have_rel_y = false;
    let mut have_abs_x = false;
    let mut have_abs_y = false;
    let mut rel_axis_index = 0;

    devinfo.relative = XInput2RelativeDevice::default();

    devinfo.scroll.scroll_info.clear();

    for &class in classes {
        // SAFETY: the caller's contract; the type says which class
        // structure it is.
        unsafe {
            if (*class).type_ == XIValuatorClass {
                /* Search for relative axes with the following priority:
                 *  - Labelled 'Rel X'/'Rel Y'
                 *   - Labelled 'Abs X'/'Abs Y'
                 *    - The first two axes found
                 */
                let v = &*(class as *const XIValuatorClassInfo);
                if v.label == st.rel_x_atom
                    || (v.label == st.abs_x_atom && !have_rel_x)
                    || (rel_axis_index == 0 && !have_rel_x && !have_abs_x)
                {
                    devinfo.relative.number[0] = v.number;
                    devinfo.relative.relative[0] = v.mode == XIModeRelative;
                    devinfo.relative.minval[0] = v.min;
                    devinfo.relative.maxval[0] = v.max;

                    if v.label == st.rel_x_atom {
                        have_rel_x = true;
                    } else if v.label == st.abs_x_atom {
                        have_abs_x = true;
                    }
                } else if v.label == st.rel_y_atom
                    || (v.label == st.abs_y_atom && !have_rel_y)
                    || (rel_axis_index == 1 && !have_rel_y && !have_abs_y)
                {
                    devinfo.relative.number[1] = v.number;
                    devinfo.relative.relative[1] = v.mode == XIModeRelative;
                    devinfo.relative.minval[1] = v.min;
                    devinfo.relative.maxval[1] = v.max;

                    if v.label == st.rel_y_atom {
                        have_rel_y = true;
                    } else if v.label == st.abs_y_atom {
                        have_abs_y = true;
                    }
                }

                rel_axis_index += 1;
            } else if (*class).type_ == XIScrollClass {
                let s = &*(class as *const XIScrollClassInfo);

                // (upstream allocates the scroll infos two at a time)
                devinfo.scroll.scroll_info.push(XInput2ScrollInfo {
                    number: s.number,
                    scroll_type: s.scroll_type,
                    increment: s.increment,
                    ..Default::default()
                });
            }
        }
    }
}

/// Translation of `xinput2_get_cached_device_info()` (the index of the
/// device, moved to the front of the list).
fn xinput2_get_cached_device_info(st: &mut XInput2State, device_id: c_int) -> Option<usize> {
    let i = st
        .device_info
        .iter()
        .position(|d| d.device_id == device_id)?;
    if i != 0 {
        // move this to the front of the list, assuming we'll get more from this one.
        let devinfo = st.device_info.remove(i);
        st.device_info.insert(0, devinfo);
    }
    Some(0)
}

/// The class list of a device or event.
///
/// # Safety
///
/// `classes` must have `num_classes` valid entries.
unsafe fn class_list<'a>(
    classes: *mut *mut XIAnyClassInfo,
    num_classes: c_int,
) -> &'a [*mut XIAnyClassInfo] {
    if classes.is_null() {
        return &[];
    }
    // SAFETY: the caller's contract.
    unsafe { std::slice::from_raw_parts(classes, num_classes.max(0) as usize) }
}

impl X11Video {
    /// Translation of `query_xinput2_version()`.
    fn query_xinput2_version(&self, mut major: c_int, mut minor: c_int) -> c_int {
        if let Some(xi) = &self.x.xinput2 {
            // We don't care if this fails, so long as it sets major/minor on it's way out the door.
            // SAFETY: the display is open; the out-parameters are valid.
            unsafe {
                (xi.XIQueryVersion)(self.display, &mut major, &mut minor);
            }
        }
        (major * 1000) + minor
    }

    /// Translation of `xinput2_get_sdlwindow()`.
    fn xinput2_get_sdlwindow(&self, window: Window) -> Option<WindowID> {
        self.x11_find_window(window)
    }

    /// Translation of `xinput2_get_device_info()`: the index of the device
    /// in the (locked) state.
    fn xinput2_get_device_info(&self, st: &mut XInput2State, device_id: c_int) -> Option<usize> {
        // Cache device info as we see new devices.
        if let Some(i) = xinput2_get_cached_device_info(st, device_id) {
            return Some(i);
        }

        // Don't know about this device yet, query and cache it.
        let xi = self.x.xinput2.as_ref()?;
        let mut devinfo = XInput2DeviceInfo::default();

        let mut i = 0;
        // SAFETY: the display is open; the device info is freed here.
        unsafe {
            let xidevinfo = (xi.XIQueryDevice)(self.display, device_id, &mut i);
            if xidevinfo.is_null() {
                return Option::None;
            }

            xinput2_update_device_info(
                st,
                &mut devinfo,
                class_list((*xidevinfo).classes, (*xidevinfo).num_classes),
            );
            (xi.XIFreeDeviceInfo)(xidevinfo);
        }

        devinfo.device_id = device_id;
        st.device_info.insert(0, devinfo);

        Some(0)
    }

    /// Translation of `X11_InitXinput2()`.
    pub(crate) fn x11_init_xinput2(&self) -> bool {
        let mut event = 0;
        let mut err = 0;
        let mut opcode = 0;

        // XInput2 is required for relative mouse mode, so you probably want to leave this enabled
        if !hints::get_bool("SDL_VIDEO_X11_XINPUT2", true) {
            return false;
        }

        /*
         * Initialize XInput 2
         * According to http://who-t.blogspot.com/2009/05/xi2-recipes-part-1.html its better
         * to inform Xserver what version of Xinput we support.The server will store the version we support.
         * "As XI2 progresses it becomes important that you use this call as the server may treat the client
         * differently depending on the supported version".
         *
         * FIXME:event and err are not needed but if not passed X11_XQueryExtension returns SegmentationFault
         */
        let Some(xi) = &self.x.xinput2 else {
            return false;
        };
        // SAFETY: the display is open; the out-parameters are valid.
        if unsafe {
            (self.x.XQueryExtension)(
                self.display,
                c"XInputExtension".as_ptr(),
                &mut opcode,
                &mut event,
                &mut err,
            )
        } == 0
        {
            return false; // X server does not have XInput at all
        }
        state().opcode = opcode;

        // We need at least 2.4 for Gesture, 2.2 for Multitouch, 2.0 otherwise.
        let version = self.query_xinput2_version(2, 4);
        if !xinput2_version_atleast(version, 2, 0) {
            return false; // X server does not support the version we want at all.
        }

        let (scrolling_supported, multitouch_supported) = {
            let mut st = state();
            st.initialized = true;

            // Smooth scrolling needs XInput 2.1
            st.scrolling_supported = xinput2_version_atleast(version, 2, 1);
            // Multitouch needs XInput 2.2
            st.multitouch_supported = xinput2_version_atleast(version, 2, 2);
            // Gesture needs XInput 2.4
            st.gesture_supported = xinput2_version_atleast(version, 2, 4);

            // Populate the atoms for finding relative axes
            st.rel_x_atom = self.intern_atom("Rel X", false);
            st.rel_y_atom = self.intern_atom("Rel Y", false);
            st.abs_x_atom = self.intern_atom("Abs X", false);
            st.abs_y_atom = self.intern_atom("Abs Y", false);
            (st.scrolling_supported, st.multitouch_supported)
        };

        // Enable raw motion events for this display
        let mut mask = [0u8; 5];
        XISetMask(&mut mask, XI_RawMotion);
        XISetMask(&mut mask, XI_RawButtonPress);
        XISetMask(&mut mask, XI_RawButtonRelease);

        if scrolling_supported {
            XISetMask(&mut mask, XI_Motion);
        }

        // Enable raw touch events if supported
        if multitouch_supported {
            XISetMask(&mut mask, XI_RawTouchBegin);
            XISetMask(&mut mask, XI_RawTouchUpdate);
            XISetMask(&mut mask, XI_RawTouchEnd);
        }

        let mut eventmask = XIEventMask {
            deviceid: XIAllMasterDevices,
            mask_len: mask.len() as c_int,
            mask: mask.as_mut_ptr(),
        };
        // SAFETY: the display is open; the mask outlives the call.
        unsafe {
            (xi.XISelectEvents)(
                self.display,
                DefaultRootWindow(self.display),
                &mut eventmask,
                1,
            );
        }

        let mut mask = [0u8; 5];

        // If not using the full keyboard handling, register for keypresses to get the event source devices.
        // (USE_XINPUT2_KEYBOARD is not defined)
        XISetMask(&mut mask, XI_KeyPress);
        XISetMask(&mut mask, XI_KeyRelease);

        XISetMask(&mut mask, XI_HierarchyChanged);
        let mut eventmask = XIEventMask {
            deviceid: XIAllDevices,
            mask_len: mask.len() as c_int,
            mask: mask.as_mut_ptr(),
        };
        // SAFETY: as above.
        unsafe {
            (xi.XISelectEvents)(
                self.display,
                DefaultRootWindow(self.display),
                &mut eventmask,
                1,
            );
        }

        self.x11_xinput2_update_devices();
        self.x11_xinput2_update_pointer_mapping();

        true
    }

    /// Translation of `X11_QuitXinput2()`.
    pub(crate) fn x11_quit_xinput2(&self) {
        let mut st = state();
        st.device_info.clear();

        st.pointer_button_map.clear();
    }

    /// Translation of `X11_Xinput2UpdatePointerMapping()`.
    pub(crate) fn x11_xinput2_update_pointer_mapping(&self) {
        if x11_xinput2_is_initialized() {
            let mut map = Vec::new();

            // SAFETY: the display is open; the map buffer has the size
            // passed.
            unsafe {
                let size = (self.x.XGetPointerMapping)(self.display, std::ptr::null_mut(), 0);
                if size > 0 {
                    map = vec![0u8; size as usize];
                    let size =
                        (self.x.XGetPointerMapping)(self.display, map.as_mut_ptr(), size as u32);
                    map.truncate(size.max(0) as usize);
                }
            }
            state().pointer_button_map = map;
        }
    }

    /// Translation of `X11_HandleXinput2Event()`.
    pub(crate) fn x11_handle_xinput2_event(&self, cookie: &XGenericEventCookie) {
        let opcode = state().opcode;
        if cookie.extension != opcode {
            return;
        }

        match cookie.evtype {
            XI_HierarchyChanged => {
                // SAFETY: the data of an XI_HierarchyChanged cookie.
                let hierev = unsafe { &*(cookie.data as *const XIHierarchyEvent) };
                let info: &[XIHierarchyInfo] = if hierev.info.is_null() {
                    &[]
                } else {
                    // SAFETY: `info` has `num_info` entries.
                    unsafe {
                        std::slice::from_raw_parts(hierev.info, hierev.num_info.max(0) as usize)
                    }
                };
                for info in info {
                    // pen stuff...
                    if (info.flags & (XISlaveRemoved | XIDeviceDisabled)) != 0 {
                        x11_remove_pen_by_device_id(info.deviceid); // it's okay if this thing isn't actually a pen, it'll handle it.
                    } else if (info.flags & (XISlaveAdded | XIDeviceEnabled)) != 0 {
                        self.x11_maybe_add_pen_by_device_id(info.deviceid); // this will do more checks to make sure this is valid.
                    }

                    // not pen stuff...
                    if info.flags & (XIMasterRemoved | XISlaveRemoved) != 0 {
                        xinput2_remove_device_info(info.deviceid);
                    }
                }
                self.with_data(|d| d.xinput_hierarchy_changed = true);
            }

            // !!! FIXME: XI_DeviceChanged fires when device valuator mappings need to be updated. Is XI_PropertyEvent needed for anything?
            // case XI_PropertyEvent:
            XI_DeviceChanged => {
                // SAFETY: the data of an XI_DeviceChanged cookie.
                let dcev = unsafe { &*(cookie.data as *const XIDeviceChangedEvent) };
                let mut st = state();
                if let Some(i) = xinput2_get_cached_device_info(&mut st, dcev.deviceid) {
                    let mut devinfo = std::mem::take(&mut st.device_info[i]);
                    // SAFETY: the event's class list.
                    unsafe {
                        xinput2_update_device_info(
                            &st,
                            &mut devinfo,
                            class_list(dcev.classes, dcev.num_classes),
                        );
                    }
                    st.device_info[i] = devinfo;
                }
            }

            XI_PropertyEvent => {
                // SAFETY: the data of an XI_PropertyEvent cookie.
                let proev = unsafe { &*(cookie.data as *const XIPropertyEvent) };
                // Handle pen proximity enter/leave
                if proev.what == XIPropertyModified
                    && proev.property == self.atoms().pen_atom_wacom_serial_ids
                {
                    // FIXME (upstream): the property event is read as an
                    // XIDeviceEvent for its `event` window, past the end of
                    // the XIPropertyEvent; no window is passed here.
                    self.x11_notify_pen_proximity_change(Option::None, proev.deviceid);
                }
            }

            XI_RawMotion => {
                // SAFETY: the data of an XI_RawMotion cookie.
                let rawev = unsafe { &*(cookie.data as *const XIRawEvent) };
                let is_pen = x11_find_pen_by_device_id(rawev.sourceid).is_some();

                self.with_data(|d| d.global_mouse_changed = true);
                if is_pen {
                    return; // Pens check for XI_Motion instead
                }

                let devinfo = {
                    let mut st = state();
                    self.xinput2_get_device_info(&mut st, rawev.deviceid)
                        .map(|i| (i, st.device_info[i].relative))
                };
                let Some((i, mut relative)) = devinfo else {
                    return; // oh well.
                };

                // SAFETY: the event's data is valid until XFreeEventData.
                unsafe {
                    parse_relative_valuators(&mut relative, rawev);
                }
                let mut st = state();
                if let Some(devinfo) = st.device_info.get_mut(i) {
                    if devinfo.device_id == rawev.deviceid {
                        devinfo.relative = relative;
                    }
                }
            }

            XI_KeyPress | XI_KeyRelease => {
                // SAFETY: the data of a key cookie.
                let xev = unsafe { &*(cookie.data as *const XIDeviceEvent) };

                /* Keys are handled through core X events, however, note the device ID and
                 * associated serial, so that the source device ID can be passed through.
                 */
                self.with_data(|d| {
                    d.xinput_last_key_serial = xev.serial;
                    d.xinput_last_keyboard_device = xev.sourceid as KeyboardID;
                });
            }

            XI_RawButtonPress | XI_RawButtonRelease | XI_RawTouchBegin | XI_RawTouchUpdate
            | XI_RawTouchEnd => {
                self.with_data(|d| d.global_mouse_changed = true);
            }

            XI_ButtonPress | XI_ButtonRelease => {
                // SAFETY: the data of a button cookie.
                let xev = unsafe { &*(cookie.data as *const XIDeviceEvent) };
                let pen = x11_find_pen_by_device_id(xev.sourceid);
                let mut button = xev.detail;
                let down = cookie.evtype == XI_ButtonPress;
                let pointer_emulated = (xev.flags & XIPointerEmulated) != 0;

                // Store the button serial to filter out redundant core button events.
                let master = self.with_data(|d| {
                    d.xinput_last_button_serial = xev.serial;
                    d.xinput_master_pointer_device
                });

                if let Some((pen, handle)) = pen {
                    if xev.deviceid != xev.sourceid {
                        // Discard events from "Master" devices to avoid duplicates.
                        return;
                    }
                    // Only report button event; if there was also pen movement / pressure changes, we expect an XI_Motion event first anyway.
                    let window = self.xinput2_get_sdlwindow(xev.event);
                    if button == 1 {
                        // button 1 is the pen tip
                        send_pen_touch(Duration::ZERO, pen, window, handle.is_eraser, down);
                    } else {
                        send_pen_button(Duration::ZERO, pen, window, (button - 1) as u8, down);
                    }
                } else if !pointer_emulated {
                    // Otherwise assume a regular mouse
                    let windowdata = self.x11_find_window(xev.event);
                    let mut x_ticks = 0;
                    let mut y_ticks = 0;

                    if xev.deviceid != master {
                        /* Ignore slave button events on non-focused windows, as they can arrive before FocusIn events,
                         * or result in focus being incorrectly set while a grab is active.
                         */
                        // FIXME (upstream): windowdata can be NULL here
                        // (dereferenced upstream).
                        if windowdata.is_none() || mouse::mouse_focus() != windowdata {
                            return;
                        }

                        // Slave pointer devices don't have button remapping applied automatically, so do it manually.
                        let st = state();
                        if button as usize <= st.pointer_button_map.len() {
                            // FIXME (upstream): button 0 reads the map at -1.
                            if let Some(&mapped) = st.pointer_button_map.get((button - 1) as usize)
                            {
                                button = mapped as c_int;
                            }
                        }
                    }

                    /* Discard wheel events from "Master" devices to avoid duplicates,
                     * as coarse wheel events are stateless and can't be deduplicated.
                     */
                    if xev.deviceid != xev.sourceid
                        && super::events::x11_is_wheel_event(button, &mut x_ticks, &mut y_ticks)
                    {
                        return;
                    }

                    // FIXME (upstream): the handlers dereference windowdata,
                    // which is NULL for an event on an unknown window.
                    let Some(windowdata) = windowdata else {
                        return;
                    };
                    if down {
                        self.x11_handle_button_press(
                            windowdata,
                            xev.sourceid as MouseID,
                            button,
                            xev.event_x as f32,
                            xev.event_y as f32,
                            xev.time,
                            xev.serial,
                        );
                    } else {
                        self.x11_handle_button_release(
                            windowdata,
                            xev.sourceid as MouseID,
                            button,
                            xev.time,
                        );
                    }
                }
            }

            XI_Enter => {
                xinput2_reset_device_valuators();
            }

            /* Register to receive XI_Motion (which deactivates MotionNotify), so that we can distinguish
            real mouse motions from synthetic ones, for multitouch and pen support. */
            XI_Motion => {
                // SAFETY: the data of a motion cookie.
                let xev = unsafe { &*(cookie.data as *const XIDeviceEvent) };
                let pointer_emulated = (xev.flags & XIPointerEmulated) != 0;

                let master = self.with_data(|d| {
                    d.global_mouse_changed = true;
                    d.xinput_master_pointer_device
                });

                let pen = x11_find_pen_by_device_id(xev.sourceid);

                if let Some((pen, handle)) = pen {
                    if xev.deviceid != xev.sourceid {
                        // Discard events from "Master" devices to avoid duplicates.
                        return;
                    }

                    let window = self.xinput2_get_sdlwindow(xev.event);
                    send_pen_motion(
                        Duration::ZERO,
                        pen,
                        window,
                        xev.event_x as f32,
                        xev.event_y as f32,
                    );

                    // SAFETY: the event's data is valid until XFreeEventData.
                    let (mask, values) = unsafe { valuators(&xev.valuators) };
                    let axes = x11_pen_axes_from_valuators(&handle, values, mask);

                    for (i, &value) in axes.iter().enumerate() {
                        if handle.valuator_for_axis[i] != SDL_X11_PEN_AXIS_VALUATOR_MISSING {
                            send_pen_axis(Duration::ZERO, pen, window, PenAxis::ALL[i], value);
                        }
                    }
                } else if !pointer_emulated {
                    if xev.deviceid == xev.sourceid {
                        let scroll = {
                            let mut st = state();
                            self.xinput2_get_device_info(&mut st, xev.deviceid)
                                .map(|i| (i, std::mem::take(&mut st.device_info[i].scroll)))
                        };
                        if let Some((i, mut scroll)) = scroll {
                            // SAFETY: the event's data is valid until
                            // XFreeEventData.
                            unsafe {
                                xinput2_parse_scrollable_valuators(&mut scroll, xev);
                            }
                            let mut st = state();
                            if let Some(devinfo) = st.device_info.get_mut(i) {
                                if devinfo.device_id == xev.deviceid {
                                    devinfo.scroll = scroll;
                                }
                            }
                        }
                    }

                    /* Use the master device for non-relative motion, as the slave devices can seemingly lag behind,
                     * except when the mouse is grabbed and touches are active, as core input events are used for
                     * absolute motion while the mouse is grabbed, and core events don't have the XIPointerEmulated
                     * flag to filter out pointer events emulated from touch events.
                     */
                    let window = self.xinput2_get_sdlwindow(xev.event);
                    if let Some(window) = window {
                        let active_touch_count = state().active_touch_count;
                        let mouse_grabbed =
                            with_x11_window(window, |_, d| d.mouse_grabbed).unwrap_or(false);
                        if (xev.deviceid == master || (active_touch_count != 0 && mouse_grabbed))
                            && !mouse::with_mouse(|m| m.relative_mode)
                        {
                            self.x11_process_hit_test(
                                window,
                                xev.event_x as f32,
                                xev.event_y as f32,
                                false,
                            );
                            mouse::send_mouse_motion(
                                Duration::ZERO,
                                Some(window),
                                mouse::GLOBAL_MOUSE_ID,
                                false,
                                xev.event_x as f32,
                                xev.event_y as f32,
                            );
                        }
                    }
                }
            }

            XI_TouchBegin => {
                // SAFETY: the data of a touch cookie.
                let xev = unsafe { &*(cookie.data as *const XIDeviceEvent) };
                state().active_touch_count += 1;
                let window = self.xinput2_get_sdlwindow(xev.event);
                let (x, y) = xinput2_normalize_touch_coordinates(window, xev.event_x, xev.event_y);
                touch::send_touch(
                    Duration::ZERO,
                    xev.sourceid as TouchID,
                    xev.detail as u64,
                    window,
                    EventType::FINGER_DOWN,
                    x,
                    y,
                    1.0,
                );
            }

            XI_TouchEnd => {
                // SAFETY: the data of a touch cookie.
                let xev = unsafe { &*(cookie.data as *const XIDeviceEvent) };
                let window = self.xinput2_get_sdlwindow(xev.event);
                let mouse_grabbed = window
                    .and_then(|w| with_x11_window(w, |_, d| d.mouse_grabbed).ok())
                    .unwrap_or(false);
                {
                    let mut st = state();
                    st.active_touch_count -= 1;
                    if st.active_touch_count == 0 && window.is_some() && mouse_grabbed {
                        st.grabbed_touch_raised = true;
                    }
                }
                let (x, y) = xinput2_normalize_touch_coordinates(window, xev.event_x, xev.event_y);
                touch::send_touch(
                    Duration::ZERO,
                    xev.sourceid as TouchID,
                    xev.detail as u64,
                    window,
                    EventType::FINGER_UP,
                    x,
                    y,
                    1.0,
                );
            }

            XI_TouchUpdate => {
                // SAFETY: the data of a touch cookie.
                let xev = unsafe { &*(cookie.data as *const XIDeviceEvent) };
                let window = self.xinput2_get_sdlwindow(xev.event);
                let (x, y) = xinput2_normalize_touch_coordinates(window, xev.event_x, xev.event_y);
                touch::send_touch_motion(
                    Duration::ZERO,
                    xev.sourceid as TouchID,
                    xev.detail as u64,
                    window,
                    x,
                    y,
                    1.0,
                );
            }

            XI_GesturePinchBegin | XI_GesturePinchUpdate | XI_GesturePinchEnd => {
                // SAFETY: the data of a pinch cookie.
                let xev = unsafe { &*(cookie.data as *const XIGesturePinchEvent) };
                let window = self.xinput2_get_sdlwindow(xev.event);
                let _ = xinput2_normalize_touch_coordinates(window, xev.event_x, xev.event_y);
                let window_id = window.unwrap_or(0);

                if cookie.evtype == XI_GesturePinchBegin {
                    touch::send_pinch(
                        EventType::PINCH_BEGIN,
                        Duration::ZERO,
                        window_id,
                        0.0,
                        -1.0,
                        -1.0,
                        -1.0,
                        -1.0,
                    );
                } else if cookie.evtype == XI_GesturePinchUpdate {
                    touch::send_pinch(
                        EventType::PINCH_UPDATE,
                        Duration::ZERO,
                        window_id,
                        xev.scale as f32,
                        -1.0,
                        -1.0,
                        -1.0,
                        -1.0,
                    );
                } else {
                    touch::send_pinch(
                        EventType::PINCH_END,
                        Duration::ZERO,
                        window_id,
                        0.0,
                        -1.0,
                        -1.0,
                        -1.0,
                        -1.0,
                    );
                }
            }

            _ => {}
        }
    }

    /// Translation of `X11_InitXinput2Multitouch()`.
    pub(crate) fn x11_init_xinput2_multitouch(&self) {
        let mut st = state();
        st.grabbed_touch_raised = false;
        st.active_touch_count = 0;
    }

    /// Translation of `X11_Xinput2Select()`.
    pub(crate) fn x11_xinput2_select(&self, window: WindowID) {
        let (scrolling_supported, multitouch_supported, gesture_supported) = {
            let st = state();
            (
                st.scrolling_supported,
                st.multitouch_supported,
                st.gesture_supported,
            )
        };
        let mut mask = [0u8; 5];

        if !scrolling_supported && !multitouch_supported {
            return;
        }
        let Some(xi) = &self.x.xinput2 else {
            return;
        };

        if scrolling_supported {
            /* Track enter events that inform us that we need to update
             * the previous scroll coordinates since we cannot track
             * them outside our window.
             */
            XISetMask(&mut mask, XI_Enter);
        }

        if multitouch_supported {
            XISetMask(&mut mask, XI_TouchBegin);
            XISetMask(&mut mask, XI_TouchUpdate);
            XISetMask(&mut mask, XI_TouchEnd);
            XISetMask(&mut mask, XI_Motion);
        }

        if gesture_supported {
            XISetMask(&mut mask, XI_GesturePinchBegin);
            XISetMask(&mut mask, XI_GesturePinchUpdate);
            XISetMask(&mut mask, XI_GesturePinchEnd);
        }

        let mut eventmask = XIEventMask {
            deviceid: XIAllMasterDevices,
            mask_len: mask.len() as c_int,
            mask: mask.as_mut_ptr(),
        };
        if let Ok(xwindow) = with_x11_window(window, |_, d| d.xwindow) {
            // SAFETY: the display is open and the window ours; the mask
            // outlives the call.
            unsafe {
                (xi.XISelectEvents)(self.display, xwindow, &mut eventmask, 1);
            }
        }
    }

    /// Translation of `X11_Xinput2SelectMouseAndKeyboard()`.
    pub(crate) fn x11_xinput2_select_mouse_and_keyboard(&self, window: WindowID) -> bool {
        if x11_xinput2_is_initialized() {
            if let Some(xi) = &self.x.xinput2 {
                let mut mask = [0u8; 4];

                // This is not enabled by default because these events are only delivered to the window with mouse focus, not keyboard focus
                // (USE_XINPUT2_KEYBOARD is not defined)

                XISetMask(&mut mask, XI_ButtonPress);
                XISetMask(&mut mask, XI_ButtonRelease);
                XISetMask(&mut mask, XI_Motion);
                let _ = with_x11_window(window, |_, d| d.xinput2_mouse_enabled = true);

                XISetMask(&mut mask, XI_Enter);
                XISetMask(&mut mask, XI_Leave);

                // Hotplugging:
                XISetMask(&mut mask, XI_DeviceChanged);
                XISetMask(&mut mask, XI_HierarchyChanged);
                XISetMask(&mut mask, XI_PropertyEvent); // E.g., when swapping tablet pens

                let mut eventmask = XIEventMask {
                    deviceid: XIAllDevices,
                    mask_len: mask.len() as c_int,
                    mask: mask.as_mut_ptr(),
                };
                let xwindow = with_x11_window(window, |_, d| d.xwindow).unwrap_or(0);
                // SAFETY: the display is open and the window ours; the
                // mask outlives the call.
                if unsafe { (xi.XISelectEvents)(self.display, xwindow, &mut eventmask, 1) }
                    != Success
                {
                    crate::warn!(
                        crate::log::Category::Input,
                        "Could not enable XInput2 event handling"
                    );
                    let _ = with_x11_window(window, |_, d| {
                        d.xinput2_keyboard_enabled = false;
                        d.xinput2_mouse_enabled = false;
                    });
                }
            }
        }

        with_x11_window(window, |_, d| {
            d.xinput2_keyboard_enabled || d.xinput2_mouse_enabled
        })
        .unwrap_or(false)
    }

    /// Translation of `X11_Xinput2GrabTouch()`.
    pub(crate) fn x11_xinput2_grab_touch(&self, window: WindowID) {
        let mut mask = [0u8; 4];

        if !state().multitouch_supported {
            return;
        }
        let Some(xi) = &self.x.xinput2 else {
            return;
        };

        let mut mods = XIGrabModifiers {
            modifiers: XIAnyModifier,
            status: 0,
        };

        XISetMask(&mut mask, XI_TouchBegin);
        XISetMask(&mut mask, XI_TouchUpdate);
        XISetMask(&mut mask, XI_TouchEnd);
        XISetMask(&mut mask, XI_Motion);

        let mut eventmask = XIEventMask {
            deviceid: XIAllDevices,
            mask_len: mask.len() as c_int,
            mask: mask.as_mut_ptr(),
        };

        if let Ok(xwindow) = with_x11_window(window, |_, d| d.xwindow) {
            // SAFETY: the display is open and the window ours; the mask
            // and modifiers outlive the call.
            unsafe {
                (xi.XIGrabTouchBegin)(
                    self.display,
                    XIAllDevices,
                    xwindow,
                    True,
                    &mut eventmask,
                    1,
                    &mut mods,
                );
            }
        }
    }

    /// Translation of `X11_Xinput2UngrabTouch()`.
    pub(crate) fn x11_xinput2_ungrab_touch(&self, window: WindowID) {
        {
            let mut st = state();
            if !st.multitouch_supported {
                return;
            }

            st.grabbed_touch_raised = false;
        }
        let Some(xi) = &self.x.xinput2 else {
            return;
        };

        let mut mods = XIGrabModifiers {
            modifiers: XIAnyModifier,
            status: 0,
        };

        if let Ok(xwindow) = with_x11_window(window, |_, d| d.xwindow) {
            // SAFETY: the display is open and the window ours.
            unsafe {
                (xi.XIUngrabTouchBegin)(self.display, XIAllDevices, xwindow, 1, &mut mods);
            }
        }
    }

    /// Translation of `X11_Xinput2UpdateDevices()`.
    pub(crate) fn x11_xinput2_update_devices(&self) {
        let mut new_keyboards: Vec<KeyboardID> = Vec::new();
        let mut new_mice: Vec<MouseID> = Vec::new();
        let mut new_touch_devices: Vec<TouchID> = Vec::new();

        crate::sdl_assert!(x11_xinput2_is_initialized());
        let Some(xi) = &self.x.xinput2 else {
            return;
        };

        let mut ndevices = 0;
        // SAFETY: the display is open; the device info is freed below.
        let info = unsafe { (xi.XIQueryDevice)(self.display, XIAllDevices, &mut ndevices) };

        let old_keyboards = keyboard::keyboards();
        let old_mice = mouse::mice();
        let old_touch_devices = touch::touch_devices();

        let devices: &[XIDeviceInfo] = if info.is_null() {
            &[]
        } else {
            // SAFETY: `info` has `ndevices` entries.
            unsafe { std::slice::from_raw_parts(info, ndevices.max(0) as usize) }
        };
        for dev in devices {
            // SAFETY: the device name is NUL-terminated.
            let name = unsafe {
                std::ffi::CStr::from_ptr(dev.name)
                    .to_string_lossy()
                    .into_owned()
            };

            match dev.use_ {
                XIMasterKeyboard | XISlaveKeyboard => {
                    let keyboard_id = dev.deviceid as KeyboardID;
                    new_keyboards.push(keyboard_id);
                    if !old_keyboards.contains(&keyboard_id) {
                        keyboard::add_keyboard(keyboard_id, Some(&name));
                    }
                }
                XIMasterPointer | XISlavePointer => {
                    if dev.use_ == XIMasterPointer {
                        self.with_data(|d| d.xinput_master_pointer_device = dev.deviceid);
                    }
                    let mouse_id = dev.deviceid as MouseID;
                    new_mice.push(mouse_id);
                    if !old_mice.contains(&mouse_id) {
                        mouse::add_mouse(mouse_id, Some(&name));
                    }
                }
                _ => {}
            }

            // SAFETY: the device's class list.
            let classes = unsafe { class_list(dev.classes, dev.num_classes) };

            // If a device info entry already exists for this device, update it.
            {
                let mut st = state();
                if let Some(i) = xinput2_get_cached_device_info(&mut st, dev.deviceid) {
                    let mut devinfo = std::mem::take(&mut st.device_info[i]);
                    // SAFETY: as above.
                    unsafe {
                        xinput2_update_device_info(&st, &mut devinfo, classes);
                    }
                    st.device_info[i] = devinfo;
                }
            }

            for &class in classes {
                // SAFETY: a class pointer of the device; a touch class is
                // an XITouchClassInfo.
                let t = unsafe {
                    // Only touch devices
                    if (*class).type_ != XITouchClass {
                        continue;
                    }
                    &*(class as *const XITouchClassInfo)
                };

                let touch_id = t.sourceid as TouchID;
                new_touch_devices.push(touch_id);
                if !old_touch_devices.contains(&touch_id) {
                    let touch_type = if t.mode == XIDependentTouch {
                        TouchDeviceType::IndirectRelative
                    } else {
                        // XIDirectTouch
                        TouchDeviceType::Direct
                    };
                    touch::add_touch(touch_id, touch_type, &name);
                }
            }
        }

        for &id in old_keyboards.iter().rev() {
            if !new_keyboards.contains(&id) {
                keyboard::remove_keyboard(id);
            }
        }

        for &id in old_mice.iter().rev() {
            if !new_mice.contains(&id) {
                mouse::remove_mouse(id);
            }
        }

        for &id in old_touch_devices.iter().rev() {
            if !new_touch_devices.contains(&id) {
                touch::del_touch(id);
            }
        }

        if !info.is_null() {
            // SAFETY: the device info from XIQueryDevice, freed once.
            unsafe {
                (xi.XIFreeDeviceInfo)(info);
            }
        }
    }
}

/// Translation of `X11_Xinput2HandlesMotionForWindow()`.
pub(crate) fn x11_xinput2_handles_motion_for_window(
    xinput2_mouse_enabled: bool,
    mouse_grabbed: bool,
) -> bool {
    /* Send the active flag once more after the touch count is zero, to suppress the
     * emulated motion event when the last touch is raised.
     */
    let mut st = state();
    let ret = xinput2_mouse_enabled
        && (!mouse_grabbed || st.active_touch_count != 0 || st.grabbed_touch_raised);
    st.grabbed_touch_raised = false;

    ret
}

/// Translation of `X11_Xinput2IsInitialized()`.
pub(crate) fn x11_xinput2_is_initialized() -> bool {
    state().initialized
}
