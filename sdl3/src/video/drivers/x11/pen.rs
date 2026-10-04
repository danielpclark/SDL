// Rust translation of src/video/x11/SDL_x11pen.c and SDL_x11pen.h from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Pressure-sensitive pen support for X11 (through XInput2).

use std::cell::RefCell;
use std::ffi::{c_int, c_uchar, c_ulong, CStr};
use std::sync::Arc;
use std::time::Duration;

use super::sys::*;
use super::video::{atom_name, intern_atom, X11Video};
use super::xinput2::x11_xinput2_is_initialized;
use crate::events::pen::{
    add_pen_device, find_pen_by_callback, remove_all_pen_devices, remove_pen_device,
    send_pen_proximity, PenAxis, PenCapabilityFlags, PenID, PenInfo, PenSubtype,
};
use crate::events::WindowID;

pub(crate) const SDL_X11_PEN_AXIS_VALUATOR_MISSING: i32 = -1;

/// Translation of `X11_PenHandle` (the pen's `SDL_PenID` comes from the
/// pen registry, which holds the handle).
#[derive(Clone, Debug)]
pub(crate) struct X11PenHandle {
    pub(crate) is_eraser: bool,
    pub(crate) x11_deviceid: c_int,
    pub(crate) valuator_for_axis: [i32; PenAxis::COUNT],
    /// shift value to add to PEN_AXIS_SLIDER (before normalisation)
    pub(crate) slider_bias: f32,
    /// rotation to add to PEN_AXIS_ROTATION  (after normalisation)
    pub(crate) rotation_bias: f32,
    pub(crate) axis_min: [f32; PenAxis::COUNT],
    pub(crate) axis_max: [f32; PenAxis::COUNT],
}

/// The device's classes.
///
/// # Safety
///
/// `dev` must be a device info from `XIQueryDevice()`.
unsafe fn device_classes(dev: &XIDeviceInfo) -> &[*mut XIAnyClassInfo] {
    // SAFETY: the caller's contract; `classes` has `num_classes` entries.
    unsafe { std::slice::from_raw_parts(dev.classes, dev.num_classes.max(0) as usize) }
}

impl X11Video {
    /// Does this device have a valuator for pressure sensitivity?
    /// Translation of `X11_XInput2DeviceIsPen()`.
    fn x11_xinput2_device_is_pen(&self, dev: &XIDeviceInfo) -> bool {
        let pen_atom_abs_pressure = self.atoms().pen_atom_abs_pressure;
        // SAFETY: dev comes from XIQueryDevice; each class pointer is valid
        // and a valuator class is an XIValuatorClassInfo.
        unsafe {
            for &classinfo in device_classes(dev) {
                if (*classinfo).type_ == XIValuatorClass {
                    let val_classinfo = &*(classinfo as *const XIValuatorClassInfo);
                    if val_classinfo.label == pen_atom_abs_pressure {
                        return true;
                    }
                }
            }
        }

        false
    }

    /// Heuristically determines if device is an eraser.
    /// Translation of `X11_XInput2PenIsEraser()`.
    fn x11_xinput2_pen_is_eraser(&self, deviceid: c_int, devicename: &str) -> bool {
        const PEN_ERASER_NAME_TAG: &str = "eraser"; // String constant to identify erasers
        let atoms = self.atoms();

        if let (true, Some(xi)) = (atoms.pen_atom_wacom_tool_type != None, &self.x.xinput2) {
            let mut type_return: Atom = 0;
            let mut format_return: c_int = 0;
            let mut num_items_return: c_ulong = 0;
            let mut bytes_after_return: c_ulong = 0;
            let mut tooltype_name_info: *mut c_uchar = std::ptr::null_mut();

            // Try Wacom-specific method
            // SAFETY: the display is open; the out-parameters are valid;
            // the property data is freed below.
            let status = unsafe {
                (xi.XIGetProperty)(
                    self.display,
                    deviceid,
                    atoms.pen_atom_wacom_tool_type,
                    0,
                    32,
                    False,
                    AnyPropertyType,
                    &mut type_return,
                    &mut format_return,
                    &mut num_items_return,
                    &mut bytes_after_return,
                    &mut tooltype_name_info,
                )
            };
            if status == Success && !tooltype_name_info.is_null() && num_items_return > 0 {
                let mut tooltype_name: Option<String> = Option::None;

                if type_return == XA_ATOM {
                    // Atom instead of string?  Un-intern
                    // SAFETY: an XA_ATOM property holds atoms.
                    let atom = unsafe { *(tooltype_name_info as *const Atom) };
                    if atom != None {
                        tooltype_name = atom_name(&self.x, self.display, atom);
                    }
                } else if type_return == XA_STRING && format_return == 8 {
                    // SAFETY: property data is always NUL-terminated.
                    tooltype_name = Some(unsafe {
                        CStr::from_ptr(tooltype_name_info.cast())
                            .to_string_lossy()
                            .into_owned()
                    });
                }

                if let Some(tooltype_name) = tooltype_name {
                    let result = tooltype_name.eq_ignore_ascii_case(PEN_ERASER_NAME_TAG);
                    // SAFETY: the property data from XIGetProperty, freed
                    // once (the atom name was copied and freed by
                    // atom_name()).
                    unsafe {
                        (self.x.XFree)(tooltype_name_info.cast());
                    }

                    return result;
                }
                // FIXME (upstream): the property data leaks when it is
                // neither an atom nor a string.
            }
        }

        // Non-Wacom device?

        /* We assume that a device is an eraser if its name contains the string "eraser".
         * Unfortunately there doesn't seem to be a clean way to distinguish these cases (as of 2022-03). */
        crate::stdlib::string::strcasestr(devicename, PEN_ERASER_NAME_TAG).is_some()
    }

    /// Read out an integer property, extending 8 and 16 bit values suitably.
    /// Returns the values read (at most `max_words`), or none on error.
    /// Translation of `X11_XInput2PenGetIntProperty()`.
    fn x11_xinput2_pen_get_int_property(
        &self,
        deviceid: c_int,
        property: Atom,
        max_words: usize,
    ) -> Vec<i32> {
        let mut type_return: Atom = 0;
        let mut format_return: c_int = 0;
        let mut num_items_return: c_ulong = 0;
        let mut bytes_after_return: c_ulong = 0;
        let mut output: *mut c_uchar = std::ptr::null_mut();

        let Some(xi) = &self.x.xinput2 else {
            return Vec::new();
        };
        if property == None {
            return Vec::new();
        }

        // SAFETY: the display is open; the out-parameters are valid.
        let status = unsafe {
            (xi.XIGetProperty)(
                self.display,
                deviceid,
                property,
                0,
                max_words as std::ffi::c_long,
                False,
                XA_INTEGER,
                &mut type_return,
                &mut format_return,
                &mut num_items_return,
                &mut bytes_after_return,
                &mut output,
            )
        };
        if status != Success || num_items_return == 0 || output.is_null() {
            return Vec::new();
        }

        if type_return == XA_INTEGER {
            let to_copy = max_words.min(num_items_return as usize);
            // SAFETY: the property holds `num_items_return` items of
            // `format_return` bits (32-bit items as longs); freed here.
            unsafe {
                let dest: Vec<i32> = match format_return {
                    8 => (0..to_copy)
                        .map(|k| *(output as *const i8).add(k) as i32)
                        .collect(),
                    16 => (0..to_copy)
                        .map(|k| *(output as *const i16).add(k) as i32)
                        .collect(),
                    // (XIGetProperty() hands out format-32 data packed,
                    // unlike XGetWindowProperty())
                    _ => std::slice::from_raw_parts(output as *const i32, to_copy).to_vec(),
                };
                (self.x.XFree)(output.cast());
                return dest;
            }
        }

        // FIXME (upstream): the property data leaks on a type mismatch.
        Vec::new() // type mismatch
    }

    /// Identify Wacom devices and extract their (device type, serial) IDs.
    /// Translation of `X11_XInput2PenWacomDeviceID()`.
    fn x11_xinput2_pen_wacom_device_id(&self, deviceid: c_int) -> Option<(u32, u32)> {
        let serial_id_buf = self.x11_xinput2_pen_get_int_property(
            deviceid,
            self.atoms().pen_atom_wacom_serial_ids,
            3,
        );
        if serial_id_buf.len() == 3 {
            return Some((serial_id_buf[2] as u32, serial_id_buf[1] as u32));
        }

        Option::None
    }

    /// Check if a Wacom device is in proximity of the tablet.
    /// Translation of `X11_XInput2PenIsInProximity()`.
    fn x11_xinput2_pen_is_in_proximity(&self, deviceid: c_int) -> Option<bool> {
        let serial_id_buf = self.x11_xinput2_pen_get_int_property(
            deviceid,
            self.atoms().pen_atom_wacom_serial_ids,
            5,
        );
        if serial_id_buf.len() == 5 {
            return Some(serial_id_buf[4] != 0 || serial_id_buf[3] != 0);
        }
        Option::None
    }

    /// Translation of `X11_MaybeAddPen()`.
    fn x11_maybe_add_pen(&self, dev: &XIDeviceInfo) -> Option<(PenID, X11PenHandle)> {
        let atoms = self.atoms();
        let mut capabilities = PenCapabilityFlags(0);

        if (dev.use_ != XISlavePointer && dev.use_ != XIFloatingSlave)
            || dev.enabled == 0
            || !self.x11_xinput2_device_is_pen(dev)
        {
            return Option::None; // Only track physical devices that are enabled and look like pens
        } else if let Some(found) = x11_find_pen_by_device_id(dev.deviceid) {
            return Some(found); // already have this pen, skip it.
        }

        let mut handle = X11PenHandle {
            is_eraser: false,
            x11_deviceid: 0,
            // until proven otherwise
            valuator_for_axis: [SDL_X11_PEN_AXIS_VALUATOR_MISSING; PenAxis::COUNT],
            slider_bias: 0.0,
            rotation_bias: 0.0,
            axis_min: [0.0; PenAxis::COUNT],
            axis_max: [0.0; PenAxis::COUNT],
        };

        let mut total_buttons = 0;
        // SAFETY: dev comes from XIQueryDevice; each class pointer is valid
        // and its type says which class structure it is.
        unsafe {
            for &classinfo in device_classes(dev) {
                if (*classinfo).type_ == XIButtonClass {
                    let button_classinfo = &*(classinfo as *const XIButtonClassInfo);
                    total_buttons += button_classinfo.num_buttons;
                } else if (*classinfo).type_ == XIValuatorClass {
                    let val_classinfo = &*(classinfo as *const XIValuatorClassInfo);
                    let valuator_nr = val_classinfo.number as i8;
                    let vname = val_classinfo.label;
                    let min = val_classinfo.min as f32;
                    let max = val_classinfo.max as f32;

                    // afaict, SDL_PEN_AXIS_DISTANCE is never reported by XInput2 (Wayland can offer it, though)
                    let axis = if vname == atoms.pen_atom_abs_pressure {
                        Some(PenAxis::Pressure)
                    } else if vname == atoms.pen_atom_abs_tilt_x {
                        Some(PenAxis::XTilt)
                    } else if vname == atoms.pen_atom_abs_tilt_y {
                        Some(PenAxis::YTilt)
                    } else {
                        Option::None
                    };

                    // !!! FIXME: there are wacom-specific hacks for getting SDL_PEN_AXIS_(ROTATION|SLIDER) on some devices, but for simplicity, we're skipping all that for now.

                    if let Some(axis) = axis {
                        capabilities.0 |= PenCapabilityFlags::from_axis(axis).0;
                        handle.valuator_for_axis[axis as usize] = valuator_nr as i32;
                        handle.axis_min[axis as usize] = min;
                        handle.axis_max[axis as usize] = max;
                    }
                }
            }
        }

        // We have a pen if and only if the device measures pressure.
        // We checked this in X11_XInput2DeviceIsPen, so just assert it here.
        crate::sdl_assert!(capabilities.0 & PenCapabilityFlags::PRESSURE.0 != 0);

        // SAFETY: the device name is a NUL-terminated string.
        let name = unsafe { CStr::from_ptr(dev.name).to_string_lossy().into_owned() };
        let is_eraser = self.x11_xinput2_pen_is_eraser(dev.deviceid, &name);
        let (wacom_devicetype_id, _wacom_serial) = self
            .x11_xinput2_pen_wacom_device_id(dev.deviceid)
            .unwrap_or((0, 0));

        let mut peninfo = PenInfo {
            capabilities,
            max_tilt: -1,
            wacom_id: wacom_devicetype_id,
            num_buttons: total_buttons,
            subtype: if is_eraser {
                PenSubtype::Eraser
            } else {
                PenSubtype::Pen
            },
            ..Default::default()
        };
        if is_eraser {
            peninfo.capabilities.0 |= PenCapabilityFlags::ERASER.0;
        }

        handle.is_eraser = is_eraser;
        handle.x11_deviceid = dev.deviceid;

        // just say it's in proximity if we can't detect this state.
        let in_proximity = self
            .x11_xinput2_pen_is_in_proximity(dev.deviceid)
            .unwrap_or(true);

        let pen = add_pen_device(
            Duration::ZERO,
            Some(&name),
            Option::None,
            Some(&peninfo),
            Arc::new(handle.clone()),
            in_proximity,
        );
        if pen == 0 {
            return Option::None;
        }

        Some((pen, handle))
    }

    /// Add a pen (if this function's further checks validate it).
    /// Translation of `X11_MaybeAddPenByDeviceID()`.
    pub(crate) fn x11_maybe_add_pen_by_device_id(
        &self,
        deviceid: c_int,
    ) -> Option<(PenID, X11PenHandle)> {
        if x11_xinput2_is_initialized() {
            let xi = self.x.xinput2.as_ref()?;
            let mut num_device_info = 0;
            // SAFETY: the display is open; the device info is freed here.
            unsafe {
                let device_info = (xi.XIQueryDevice)(self.display, deviceid, &mut num_device_info);
                if !device_info.is_null() {
                    crate::sdl_assert!(num_device_info == 1);
                    let handle = self.x11_maybe_add_pen(&*device_info);
                    (xi.XIFreeDeviceInfo)(device_info);
                    return handle;
                }
            }
        }
        Option::None
    }

    /// Notify that the pen has entered/left proximity.
    /// Translation of `X11_NotifyPenProximityChange()`.
    pub(crate) fn x11_notify_pen_proximity_change(
        &self,
        window: Option<WindowID>,
        deviceid: c_int,
    ) {
        if let Some((pen, _)) = x11_find_pen_by_device_id(deviceid) {
            if let Some(in_proximity) = self.x11_xinput2_pen_is_in_proximity(deviceid) {
                send_pen_proximity(Duration::ZERO, pen, window, in_proximity, in_proximity);
            }
        }
    }

    /// Prep pen support (never fails; pens simply won't be added if there's a problem).
    /// Translation of `X11_InitPen()`.
    pub(crate) fn x11_init_pen(&self) {
        if !x11_xinput2_is_initialized() {
            return; // we need XIQueryDevice() for this.
        }
        let Some(xi) = &self.x.xinput2 else {
            return;
        };

        {
            let lookup_pen_atom = |name: &str| intern_atom(&self.x, self.display, name, false);
            let mut atoms = self.atoms.write().unwrap_or_else(|e| e.into_inner());
            atoms.pen_atom_device_product_id = lookup_pen_atom("Device Product ID");
            atoms.pen_atom_wacom_serial_ids = lookup_pen_atom("Wacom Serial IDs");
            atoms.pen_atom_wacom_tool_type = lookup_pen_atom("Wacom Tool Type");
            atoms.pen_atom_abs_pressure = lookup_pen_atom("Abs Pressure");
            atoms.pen_atom_abs_tilt_x = lookup_pen_atom("Abs Tilt X");
            atoms.pen_atom_abs_tilt_y = lookup_pen_atom("Abs Tilt Y");
        }

        // Do an initial check on devices. After this, we'll add/remove individual pens when XI_HierarchyChanged events alert us.
        let mut num_device_info = 0;
        // SAFETY: the display is open; the device info has
        // `num_device_info` entries and is freed here.
        unsafe {
            let device_info = (xi.XIQueryDevice)(self.display, XIAllDevices, &mut num_device_info);
            if !device_info.is_null() {
                for i in 0..num_device_info.max(0) as usize {
                    self.x11_maybe_add_pen(&*device_info.add(i));
                }
                (xi.XIFreeDeviceInfo)(device_info);
            }
        }
    }

    /// Clean up pen support. Translation of `X11_QuitPen()`.
    pub(crate) fn x11_quit_pen(&self) {
        // (X11_FreePenHandle(): the handles are dropped with the pens)
        remove_all_pen_devices(|_, _| {});
    }
}

/// Map X11 device ID to pen ID (and the pen's handle).
/// Translation of `X11_FindPenByDeviceID()` (with `FindPenByDeviceID()`).
pub(crate) fn x11_find_pen_by_device_id(deviceid: c_int) -> Option<(PenID, X11PenHandle)> {
    let handle: RefCell<Option<X11PenHandle>> = RefCell::new(Option::None);
    let pen = find_pen_by_callback(|h| {
        let Some(x11_handle) = h.downcast_ref::<X11PenHandle>() else {
            return false;
        };
        if x11_handle.x11_deviceid != deviceid {
            return false;
        }
        *handle.borrow_mut() = Some(x11_handle.clone());
        true
    });
    let handle = handle.into_inner()?;
    (pen != 0).then_some((pen, handle))
}

/// Remove a pen. It's okay if deviceid is bogus or not a pen, we'll check it.
/// Translation of `X11_RemovePenByDeviceID()`.
pub(crate) fn x11_remove_pen_by_device_id(deviceid: c_int) {
    if let Some((pen, _)) = x11_find_pen_by_device_id(deviceid) {
        remove_pen_device(Duration::ZERO, Option::None, pen);
    }
}

/// Translation of `X11_XInput2NormalizePenAxes()`.
#[allow(clippy::needless_range_loop)] // (the axis indexes several arrays, as upstream)
fn x11_xinput2_normalize_pen_axes(pen: &X11PenHandle, coords: &mut [f32; PenAxis::COUNT]) {
    // Normalise axes
    for axis in 0..PenAxis::COUNT {
        let valuator = pen.valuator_for_axis[axis];
        if valuator == SDL_X11_PEN_AXIS_VALUATOR_MISSING {
            continue;
        }

        let mut value = coords[axis];
        let min = pen.axis_min[axis];
        let max = pen.axis_max[axis];

        if axis == PenAxis::Slider as usize {
            value += pen.slider_bias;
        }

        // min ... 0 ... max
        if min < 0.0 {
            // Normalise so that 0 remains 0.0
            if value < 0.0 {
                value /= -min;
            } else if max == 0.0 {
                value = 0.0;
            } else {
                value /= max;
            }
        } else {
            // 0 ... min ... max
            // including 0.0 = min
            if max == 0.0 {
                value = 0.0;
            } else {
                value = (value - min) / max;
            }
        }

        if axis == PenAxis::XTilt as usize || axis == PenAxis::YTilt as usize {
            //if (peninfo->info.max_tilt > 0.0f) {
            //    value *= peninfo->info.max_tilt; // normalize to physical max
            //}
        } else if axis == PenAxis::Rotation as usize {
            // normalised to -1..1, so let's convert to degrees
            value *= 180.0;
            value += pen.rotation_bias;

            // handle simple over/underflow
            if value >= 180.0 {
                value -= 360.0;
            } else if value < -180.0 {
                value += 360.0;
            }
        }

        coords[axis] = value;
    }
}

/// Converts XINPUT2 valuators into pen axis information, including normalisation.
/// Translation of `X11_PenAxesFromValuators()`.
///
/// `input_values` are the event's valuator values, `mask` its valuator mask.
pub(crate) fn x11_pen_axes_from_valuators(
    pen: &X11PenHandle,
    input_values: &[f64],
    mask: &[u8],
) -> [f32; PenAxis::COUNT] {
    let mut axis_values = [0.0f32; PenAxis::COUNT];
    for (i, value) in axis_values.iter_mut().enumerate() {
        let valuator = pen.valuator_for_axis[i];
        if valuator == SDL_X11_PEN_AXIS_VALUATOR_MISSING
            || valuator as usize >= mask.len() * 8
            || !XIMaskIsSet(mask, valuator)
        {
            *value = 0.0;
        } else {
            // FIXME (upstream): the values are packed (one per set mask
            // bit), but they are indexed by valuator number, which can
            // read the wrong value or past the array (0.0 here).
            *value = input_values.get(valuator as usize).copied().unwrap_or(0.0) as f32;
        }
    }
    x11_xinput2_normalize_pen_axes(pen, &mut axis_values);
    axis_values
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pen() -> X11PenHandle {
        let mut p = X11PenHandle {
            is_eraser: false,
            x11_deviceid: 9,
            valuator_for_axis: [SDL_X11_PEN_AXIS_VALUATOR_MISSING; PenAxis::COUNT],
            slider_bias: 0.0,
            rotation_bias: 0.0,
            axis_min: [0.0; PenAxis::COUNT],
            axis_max: [0.0; PenAxis::COUNT],
        };
        p.valuator_for_axis[PenAxis::Pressure as usize] = 2;
        p.axis_max[PenAxis::Pressure as usize] = 2048.0;
        p.valuator_for_axis[PenAxis::XTilt as usize] = 3;
        p.axis_min[PenAxis::XTilt as usize] = -64.0;
        p.axis_max[PenAxis::XTilt as usize] = 63.0;
        p
    }

    #[test]
    fn normalizes_axes() {
        let p = pen();
        let mut mask = [0u8; 1];
        XISetMask(&mut mask, 2);
        XISetMask(&mut mask, 3);
        let values = [0.0, 0.0, 1024.0, -32.0];
        let axes = x11_pen_axes_from_valuators(&p, &values, &mask);
        assert_eq!(axes[PenAxis::Pressure as usize], 0.5);
        assert_eq!(axes[PenAxis::XTilt as usize], -0.5);
        assert_eq!(axes[PenAxis::YTilt as usize], 0.0);

        // A valuator outside the mask reads as 0.
        let axes = x11_pen_axes_from_valuators(&p, &values, &[0u8]);
        assert_eq!(axes[PenAxis::Pressure as usize], 0.0);
    }
}
