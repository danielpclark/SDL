// Rust translation of src/haptic/windows/SDL_windowshaptic.c and
// SDL_windowshaptic_c.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Windows haptic driver: DirectInput force feedback devices
//! ([`dinput`]), including the joysticks of the Windows joystick driver,
//! which adds and removes their haptic devices as it detects them.
//!
//! SDL3 has no XInput haptic support (XInput controllers rumble through the
//! joystick API); the `thread`, `mutex`, `stopTicks` and `stopThread`
//! fields of upstream's `struct haptic_hwdata` are left over from it and
//! unused, so they aren't here.

mod dinput;

#[cfg(test)]
mod tests;

use std::sync::Mutex;

use super::{HapticData, HapticDriver, HapticEffect, HapticID};
use crate::core::windows::directx::{
    Effect, InputDevice, DI8DEVCLASS_POINTER, DIDC_FORCEFEEDBACK, DIDEVCAPS, DIDEVICEINSTANCEW,
};
use crate::error::{Error, Result};
use crate::joystick::Joystick;

/// Haptic system hardware data. Translation of `struct haptic_hwdata`.
struct HapticHwData {
    device: InputDevice,
    /// Axes to use.
    axes: [u32; 3],
    /// Device is loaded as joystick.
    #[allow(dead_code)] // (the reference is released either way, see haptic_close())
    is_joystick: bool,
}

/// Haptic system effect data. Translation of `struct haptic_hweffect`.
struct HapticHwEffect {
    effect: dinput::DiEffect,
    reference: Effect,
}

/// An available haptic device. Translation of `SDL_hapticlist_item`
/// (whose `haptic`, never set, isn't kept).
struct HapticlistItem {
    instance_id: HapticID,
    name: String,
    instance: DIDEVICEINSTANCEW,
    capabilities: DIDEVCAPS,
}

/// List of available haptic devices, in the order they were added.
/// Translation of `SDL_hapticlist` (with `SDL_hapticlist_tail` and
/// `numhaptics`, its end and length).
static HAPTICLIST: Mutex<Vec<HapticlistItem>> = Mutex::new(Vec::new());

fn with_list<R>(f: impl FnOnce(&mut Vec<HapticlistItem>) -> R) -> R {
    f(&mut HAPTICLIST.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Translation of `SDL_SYS_AddHapticDevice()`.
fn add_haptic_device(item: HapticlistItem) -> bool {
    // Device has been added.
    with_list(|l| l.push(item));
    true
}

/// Remove the device at `index`; whether it was open (upstream's
/// `item->haptic`, which is never set, so this is always false).
/// Translation of `SDL_SYS_RemoveHapticDevice()`.
fn remove_haptic_device(index: usize) -> bool {
    with_list(|l| {
        l.remove(index);
    });
    // !!! TODO: Send a haptic remove event?
    false
}

/// Add a DirectInput device the joystick driver found. Translation of
/// `SDL_DINPUT_HapticMaybeAddDevice()`, as the joystick driver calls it.
pub(crate) fn dinput_haptic_maybe_add_device(instance: &DIDEVICEINSTANCEW) -> bool {
    dinput::haptic_maybe_add_device(instance)
}

/// Remove a DirectInput device the joystick driver lost. Translation of
/// `SDL_DINPUT_HapticMaybeRemoveDevice()`, as the joystick driver calls it.
pub(crate) fn dinput_haptic_maybe_remove_device(instance: &DIDEVICEINSTANCEW) -> bool {
    dinput::haptic_maybe_remove_device(instance)
}

/// The driver data of an open device.
fn hwdata(haptic: &HapticData) -> Result<&HapticHwData> {
    haptic
        .hwdata
        .as_ref()
        .and_then(|h| h.downcast_ref::<HapticHwData>())
        .ok_or_else(|| Error::invalid_param("haptic"))
}

/// The driver data of an effect slot (created by `new_effect`).
fn hweffect(haptic: &mut HapticData, effect: usize) -> Result<&mut HapticHwEffect> {
    haptic
        .effects
        .get_mut(effect)
        .and_then(|slot| slot.hweffect.as_mut())
        .and_then(|h| h.downcast_mut::<HapticHwEffect>())
        .ok_or_else(|| Error::invalid_param("effect"))
}

/// Translation of the Windows `SDL_SYS_Haptic*` functions.
pub(super) struct WindowsHapticDriver;

pub(super) static WINDOWS_HAPTIC_DRIVER: WindowsHapticDriver = WindowsHapticDriver;

impl HapticDriver for WindowsHapticDriver {
    /// Initializes the haptic subsystem. Translation of
    /// `SDL_SYS_HapticInit()`.
    fn init(&self) -> Result<()> {
        if let Err(e) = dinput::haptic_init() {
            self.quit();
            return Err(e);
        }

        /* The joystick subsystem will usually be initialized before haptics,
         * so the initial HapticMaybeAddDevice() calls from the joystick
         * subsystem will arrive too early to create haptic devices. We will
         * invoke those callbacks again here to pick up any joysticks that
         * were added prior to haptics initialization. */
        for instance in crate::joystick::windows::device_instances() {
            dinput::haptic_maybe_add_device(&instance);
        }

        Ok(())
    }

    /// Translation of `SDL_SYS_NumHaptics()`.
    fn count(&self) -> usize {
        with_list(|l| l.len())
    }

    /// Translation of `SDL_SYS_HapticInstanceID()` (and `HapticByDevIndex()`).
    fn instance_id(&self, index: usize) -> HapticID {
        with_list(|l| l.get(index).map_or(0, |item| item.instance_id))
    }

    /// Return the name of a haptic device, does not need to be opened.
    /// Translation of `SDL_SYS_HapticName()`.
    fn name(&self, index: usize) -> Option<String> {
        with_list(|l| l.get(index).map(|item| item.name.clone()))
    }

    /// Opens a haptic device for usage. Translation of `SDL_SYS_HapticOpen()`
    /// (and `HapticByInstanceID()`).
    fn open(&self, haptic: &mut HapticData) -> Result<()> {
        let instance = with_list(|l| {
            l.iter()
                .find(|item| item.instance_id == haptic.instance_id)
                .map(|item| item.instance)
        })
        .ok_or_else(|| Error::new(format!("Haptic device {} not found", haptic.instance_id)))?;
        dinput::haptic_open(haptic, &instance)
    }

    /// Opens a haptic device from first mouse it finds for usage.
    /// Translation of `SDL_SYS_HapticMouse()`.
    fn mouse(&self) -> Option<usize> {
        // Grab the first mouse haptic device we find.
        // FIXME (upstream): this compares the device type (DI8DEVTYPE_MOUSE
        // and its subtype) with a device class (DI8DEVCLASS_POINTER), so it
        // never finds one.
        with_list(|l| {
            l.iter()
                .position(|item| item.capabilities.dwDevType == DI8DEVCLASS_POINTER)
        })
    }

    /// Checks to see if a joystick has haptic features.
    /// Translation of `SDL_SYS_JoystickIsHaptic()`.
    fn joystick_is_haptic(&self, joystick: &Joystick) -> bool {
        let Some((_, capabilities)) = crate::joystick::windows::joystick_dinput_device(joystick)
        else {
            return false;
        };
        capabilities.dwFlags & DIDC_FORCEFEEDBACK != 0
    }

    /// Opens a SDL_Haptic from a SDL_Joystick.
    /// Translation of `SDL_SYS_HapticOpenFromJoystick()`.
    fn open_from_joystick(&self, haptic: &mut HapticData, joystick: &Joystick) -> Result<()> {
        let Some((Some(device), _)) = crate::joystick::windows::joystick_dinput_device(joystick)
        else {
            return Err(Error::new("Haptic: joystick isn't a DirectInput joystick"));
        };
        dinput::haptic_open_from_joystick(haptic, &device)
    }

    /// Checks to see if the haptic device and joystick are in reality the
    /// same. Translation of `SDL_SYS_JoystickSameHaptic()`.
    fn joystick_same_haptic(&self, haptic: &HapticData, joystick: &Joystick) -> bool {
        let Some((Some(device), _)) = crate::joystick::windows::joystick_dinput_device(joystick)
        else {
            return false;
        };
        let Ok(hwdata) = hwdata(haptic) else {
            return false;
        };
        dinput::joystick_same_haptic(hwdata, &device)
    }

    /// Closes the haptic device. Translation of `SDL_SYS_HapticClose()`.
    fn close(&self, haptic: &mut HapticData) {
        if let Some(hwdata) = haptic.hwdata.take() {
            // Free effects.
            haptic.effects.clear();

            // Clean up
            if let Ok(hwdata) = hwdata.downcast::<HapticHwData>() {
                dinput::haptic_close(*hwdata);
            }
        }
    }

    /// Clean up after system specific haptic stuff.
    /// Translation of `SDL_SYS_HapticQuit()`.
    fn quit(&self) {
        /* Opened and not closed haptics are leaked, this is on purpose.
         * Close your haptic devices after usage. */
        // !!! FIXME: (...is leaking on purpose a good idea?) - No, of course not.
        with_list(|l| l.clear());

        dinput::haptic_quit();
    }

    /// Creates a new haptic effect. Translation of
    /// `SDL_SYS_HapticNewEffect()`.
    fn new_effect(
        &self,
        haptic: &mut HapticData,
        effect: usize,
        base: &HapticEffect,
    ) -> Result<()> {
        let naxes = haptic.naxes;
        let (effect_data, reference) = dinput::haptic_new_effect(hwdata(haptic)?, naxes, base)?;
        let slot = haptic
            .effects
            .get_mut(effect)
            .ok_or_else(|| Error::invalid_param("effect"))?;
        slot.hweffect = Some(Box::new(HapticHwEffect {
            effect: effect_data,
            reference,
        }));
        Ok(())
    }

    /// Updates an effect. Translation of `SDL_SYS_HapticUpdateEffect()`.
    fn update_effect(
        &self,
        haptic: &mut HapticData,
        effect: usize,
        data: &HapticEffect,
    ) -> Result<()> {
        let naxes = haptic.naxes;
        let new_effect = {
            let hwdata = hwdata(haptic)?;
            let slot = haptic
                .effects
                .get(effect)
                .and_then(|slot| slot.hweffect.as_ref())
                .and_then(|h| h.downcast_ref::<HapticHwEffect>())
                .ok_or_else(|| Error::invalid_param("effect"))?;
            dinput::haptic_update_effect(hwdata, naxes, &slot.effect, &slot.reference, data)?
        };
        hweffect(haptic, effect)?.effect = new_effect;
        Ok(())
    }

    /// Runs an effect. Translation of `SDL_SYS_HapticRunEffect()`.
    fn run_effect(&self, haptic: &mut HapticData, effect: usize, iterations: u32) -> Result<()> {
        dinput::haptic_run_effect(&hweffect(haptic, effect)?.reference, iterations)
    }

    /// Stops an effect. Translation of `SDL_SYS_HapticStopEffect()`.
    fn stop_effect(&self, haptic: &mut HapticData, effect: usize) -> Result<()> {
        dinput::haptic_stop_effect(&hweffect(haptic, effect)?.reference)
    }

    /// Frees the effect. Translation of `SDL_SYS_HapticDestroyEffect()`.
    fn destroy_effect(&self, haptic: &mut HapticData, effect: usize) {
        let Some(slot) = haptic.effects.get_mut(effect) else {
            return;
        };
        if let Some(hweffect) = slot.hweffect.take() {
            if let Ok(hweffect) = hweffect.downcast::<HapticHwEffect>() {
                dinput::haptic_destroy_effect(hweffect.reference);
            }
        }
    }

    /// Gets the status of a haptic effect. Translation of
    /// `SDL_SYS_HapticGetEffectStatus()`.
    fn effect_status(&self, haptic: &mut HapticData, effect: usize) -> Result<bool> {
        dinput::haptic_get_effect_status(&hweffect(haptic, effect)?.reference)
    }

    /// Sets the gain. Translation of `SDL_SYS_HapticSetGain()`.
    fn set_gain(&self, haptic: &mut HapticData, gain: i32) -> Result<()> {
        dinput::haptic_set_gain(hwdata(haptic)?, gain)
    }

    /// Sets the autocentering. Translation of `SDL_SYS_HapticSetAutocenter()`.
    fn set_autocenter(&self, haptic: &mut HapticData, autocenter: i32) -> Result<()> {
        dinput::haptic_set_autocenter(hwdata(haptic)?, autocenter)
    }

    /// Pauses the device. Translation of `SDL_SYS_HapticPause()`.
    fn pause(&self, haptic: &mut HapticData) -> Result<()> {
        dinput::haptic_pause(hwdata(haptic)?)
    }

    /// Unpauses the device. Translation of `SDL_SYS_HapticResume()`.
    fn resume(&self, haptic: &mut HapticData) -> Result<()> {
        dinput::haptic_resume(hwdata(haptic)?)
    }

    /// Stops all the playing effects on the device. Translation of
    /// `SDL_SYS_HapticStopAll()`.
    fn stop_all(&self, haptic: &mut HapticData) -> Result<()> {
        dinput::haptic_stop_all(hwdata(haptic)?)
    }
}
