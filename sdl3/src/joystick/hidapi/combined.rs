// Rust translation of src/joystick/hidapi/SDL_hidapi_combined.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! This driver supports the Nintendo Switch Joy-Cons pair controllers: a
//! device with two children, whose drivers do the work.

use super::{DeviceCtx, DriverContext, DriverImpl, HidapiDevice, JoystickCaps};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::joystick::gamepad::GamepadType;
use crate::joystick::JoystickData;

/// The combined driver's static functions.
pub(crate) struct CombinedDriver;

impl DriverImpl for CombinedDriver {
    /// Translation of `HIDAPI_DriverCombined_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[]
    }

    /// Translation of `HIDAPI_DriverCombined_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        true
    }

    /// Translation of `HIDAPI_DriverCombined_IsSupportedDevice()`.
    fn is_supported_device(
        &self,
        _device: Option<&HidapiDevice>,
        _name: &str,
        _gamepad_type: GamepadType,
        _vendor_id: u16,
        _product_id: u16,
        _version: u16,
        _interface_number: i32,
        _interface_class: i32,
        _interface_subclass: i32,
        _interface_protocol: i32,
    ) -> bool {
        // This is always explicitly created for combined devices
        false
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(CombinedContext)
    }
}

/// Ok if any child succeeded; otherwise the error the last child set
/// (upstream returns false, with the error of the last failure).
fn any_succeeded(results: Vec<Result<()>>) -> Result<()> {
    let mut error = Error::new("");
    for result in results {
        match result {
            Ok(()) => return Ok(()),
            Err(e) => error = e,
        }
    }
    Err(error)
}

/// The combined driver has no state of its own.
struct CombinedContext;

impl DriverContext for CombinedContext {
    /// Translation of `HIDAPI_DriverCombined_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        device.joystick_connected();
        Ok(())
    }

    /// Translation of `HIDAPI_DriverCombined_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        let mut result = true;

        for child in device.children() {
            if !device
                .with_child(&child, |context, child| context.update_device(child))
                .unwrap_or(false)
            {
                result = false;
            }
        }
        result
    }

    /// Translation of `HIDAPI_DriverCombined_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        let children = device.children();
        let mut serial: Option<String> = None;

        for (i, child) in children.iter().enumerate() {
            let result = device
                .with_child(child, |context, child| {
                    context.open_joystick(child, joystick)
                })
                .unwrap_or_else(|| Err(Error::new("HIDAPI device is busy")));
            if let Err(e) = result {
                child.state().broken = true;

                for child in children[..i].iter().rev() {
                    device.with_child(child, |context, child| {
                        context.close_joystick(child, joystick.instance_id)
                    });
                }
                return Err(e);
            }

            // Extend the serial number with the child serial number
            if let Some(child_serial) = joystick.serial.take() {
                serial = Some(match serial {
                    Some(serial) => format!("{serial},{child_serial}"),
                    None => child_serial,
                });
            }
        }

        // Update the joystick with the combined serial numbers
        joystick.serial = serial;

        Ok(())
    }

    /// Translation of `HIDAPI_DriverCombined_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        let results: Vec<Result<()>> = device
            .children()
            .iter()
            .filter_map(|child| {
                device.with_child(child, |context, child| {
                    context.rumble_joystick(
                        child,
                        joystick,
                        low_frequency_rumble,
                        high_frequency_rumble,
                    )
                })
            })
            .collect();
        any_succeeded(results)
    }

    /// Translation of `HIDAPI_DriverCombined_RumbleJoystickTriggers()`.
    fn rumble_joystick_triggers(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        left_rumble: u16,
        right_rumble: u16,
    ) -> Result<()> {
        let results: Vec<Result<()>> = device
            .children()
            .iter()
            .filter_map(|child| {
                device.with_child(child, |context, child| {
                    context.rumble_joystick_triggers(child, joystick, left_rumble, right_rumble)
                })
            })
            .collect();
        any_succeeded(results)
    }

    /// Translation of `HIDAPI_DriverCombined_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
    ) -> JoystickCaps {
        let mut caps = JoystickCaps(0);

        for child in device.children() {
            if let Some(child_caps) = device.with_child(&child, |context, child| {
                context.get_joystick_capabilities(child, joystick)
            }) {
                caps |= child_caps;
            }
        }
        caps
    }

    /// Translation of `HIDAPI_DriverCombined_SetJoystickLED()`.
    fn set_joystick_led(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        red: u8,
        green: u8,
        blue: u8,
    ) -> Result<()> {
        let results: Vec<Result<()>> = device
            .children()
            .iter()
            .filter_map(|child| {
                device.with_child(child, |context, child| {
                    context.set_joystick_led(child, joystick, red, green, blue)
                })
            })
            .collect();
        any_succeeded(results)
    }

    /// Translation of `HIDAPI_DriverCombined_SetJoystickSensorsEnabled()`.
    fn set_joystick_sensors_enabled(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        enabled: bool,
    ) -> Result<()> {
        let results: Vec<Result<()>> = device
            .children()
            .iter()
            .filter_map(|child| {
                device.with_child(child, |context, child| {
                    context.set_joystick_sensors_enabled(child, joystick, enabled)
                })
            })
            .collect();
        any_succeeded(results)
    }

    /// Translation of `HIDAPI_DriverCombined_CloseJoystick()`.
    fn close_joystick(&mut self, device: &mut DeviceCtx<'_>, joystick: JoystickID) {
        for child in device.children() {
            device.with_child(&child, |context, child| {
                context.close_joystick(child, joystick)
            });
        }
    }
}
