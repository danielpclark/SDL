// Rust translation of src/haptic/android/SDL_syshaptic.c and
// src/haptic/android/SDL_syshaptic_c.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Android haptic driver: the vibrators that the Java side's haptic
//! handler (`SDLControllerManager.java`'s `SDLHapticHandler`,
//! `crate::core::android::java::controller_manager`) finds: the system's
//! vibrator (`VIBRATOR_SERVICE`), when the device has one. The handler
//! looks for it when the driver initializes
//! (`Android_JNI_DetectHapticDevices()`), and tells of it through
//! `nativeAddHaptic()` and `nativeRemoveHaptic()`.
//!
//! A device has one effect, a left/right one, played as a vibration of the
//! effect's length at the motors' mean intensity (60% of the large one's,
//! 40% of the small one's), through the handler's `hapticRun()`, and
//! stopped by its `hapticStop()`. Nothing else is supported; the gain,
//! autocenter, pause and resume succeed doing nothing.
//!
//! Note (upstream), kept as it is:
//!
//! * the effect is never allocated (its `hweffect` stays null): every new
//!   effect takes the first slot, and destroying one does nothing;
//! * the magnitudes are divided by 32767 (an intensity above 1 for one
//!   above that, which Java takes as 1);
//! * the list isn't cleared when the subsystem quits (there's no way to
//!   scan for the vibrators but at init).
//!
//! Where upstream keeps `haptic->hwdata` pointing at the device's list item
//! (whose `device_id` its effects read, after the item is freed when the
//! device is removed while it's open), the open device keeps its device
//! ID, and the item is found by its instance ID.

use std::sync::{Mutex, MutexGuard};

use super::{HapticData, HapticDriver, HapticEffect, HapticEffectSlot, HapticFeatures, HapticID};
use crate::core::android as jni;
use crate::error::{Error, Result};
use crate::joystick::Joystick;

/// A vibrator. Translation of `SDL_hapticlist_item`.
#[derive(Debug)]
struct HapticlistItem {
    instance_id: HapticID,
    device_id: i32,
    name: String,
    /// Whether it's open (`haptic`, the `SDL_Haptic *`).
    haptic: bool,
}

/// Translation of `SDL_hapticlist` (and `SDL_hapticlist_tail`;
/// `numhaptics` is its length).
static HAPTICLIST: Mutex<Vec<HapticlistItem>> = Mutex::new(Vec::new());

fn hapticlist() -> MutexGuard<'static, Vec<HapticlistItem>> {
    HAPTICLIST.lock().unwrap_or_else(|e| e.into_inner())
}

/// The open device's device ID (its `hwdata`).
fn device_id(haptic: &HapticData) -> i32 {
    haptic
        .hwdata
        .as_ref()
        .and_then(|hwdata| hwdata.downcast_ref::<i32>())
        .copied()
        .unwrap_or(-1)
}

pub(super) struct AndroidHapticDriver;

pub(super) static ANDROID_HAPTIC_DRIVER: AndroidHapticDriver = AndroidHapticDriver;

impl HapticDriver for AndroidHapticDriver {
    /// Translation of `SDL_SYS_HapticInit()`.
    fn init(&self) -> Result<()> {
        jni::detect_haptic_devices();
        Ok(())
    }

    /// Translation of `SDL_SYS_NumHaptics()`.
    fn count(&self) -> usize {
        hapticlist().len()
    }

    /// Translation of `SDL_SYS_HapticInstanceID()` (`HapticByOrder()`).
    fn instance_id(&self, index: usize) -> HapticID {
        hapticlist().get(index).map_or(0, |item| item.instance_id)
    }

    /// Translation of `SDL_SYS_HapticName()` (upstream also sets "No such
    /// device" for an index without a device).
    fn name(&self, index: usize) -> Option<String> {
        hapticlist().get(index).map(|item| item.name.clone())
    }

    /// Translation of `SDL_SYS_HapticOpen()` (`OpenHapticByInstanceID()`,
    /// `OpenHaptic()`).
    fn open(&self, haptic: &mut HapticData) -> Result<()> {
        let mut list = hapticlist();
        let item = list
            .iter_mut()
            .find(|item| item.instance_id == haptic.instance_id)
            .ok_or_else(|| Error::new("No such device"))?;
        if item.haptic {
            return Err(Error::new("Haptic already opened"));
        }

        haptic.hwdata = Some(Box::new(item.device_id));
        item.haptic = true;

        haptic.instance_id = item.instance_id;
        haptic.name = Some(item.name.clone());
        haptic.supported = HapticFeatures::LEFTRIGHT;
        let neffects = 1;
        haptic.nplaying = neffects;
        haptic.effects = (0..neffects).map(|_| HapticEffectSlot::default()).collect();
        Ok(())
    }

    /// Translation of `SDL_SYS_HapticMouse()`.
    fn mouse(&self) -> Option<usize> {
        None
    }

    /// Translation of `SDL_SYS_JoystickIsHaptic()`.
    fn joystick_is_haptic(&self, _joystick: &Joystick) -> bool {
        false
    }

    /// Translation of `SDL_SYS_HapticOpenFromJoystick()`.
    fn open_from_joystick(&self, _haptic: &mut HapticData, _joystick: &Joystick) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `SDL_SYS_JoystickSameHaptic()`.
    fn joystick_same_haptic(&self, _haptic: &HapticData, _joystick: &Joystick) -> bool {
        false
    }

    /// Translation of `SDL_SYS_HapticClose()`.
    fn close(&self, haptic: &mut HapticData) {
        if let Some(item) = hapticlist()
            .iter_mut()
            .find(|item| item.instance_id == haptic.instance_id)
        {
            item.haptic = false;
        }
        haptic.hwdata = None;
    }

    /// Translation of `SDL_SYS_HapticQuit()`.
    fn quit(&self) {
        // We don't have any way to scan for joysticks (and their vibrators) at init, so don't wipe the list
        // of joysticks here in case this is a reinit.
    }

    /// Translation of `SDL_SYS_HapticNewEffect()`.
    fn new_effect(
        &self,
        _haptic: &mut HapticData,
        _effect: usize,
        _base: &HapticEffect,
    ) -> Result<()> {
        Ok(())
    }

    /// Translation of `SDL_SYS_HapticUpdateEffect()`.
    fn update_effect(
        &self,
        _haptic: &mut HapticData,
        _effect: usize,
        _data: &HapticEffect,
    ) -> Result<()> {
        Ok(())
    }

    /// Translation of `SDL_SYS_HapticRunEffect()`.
    fn run_effect(&self, haptic: &mut HapticData, effect: usize, _iterations: u32) -> Result<()> {
        // (the effect's union read as a left/right one: the only kind the
        // device supports)
        let leftright = match &haptic.effects[effect].effect {
            Some(HapticEffect::LeftRight(leftright)) => *leftright,
            _ => Default::default(),
        };
        let large = leftright.large_magnitude as f32 / 32767.0;
        let small = leftright.small_magnitude as f32 / 32767.0;

        let total = (large * 0.6) + (small * 0.4);

        jni::haptic_run(device_id(haptic), total, leftright.length as i32);
        Ok(())
    }

    /// Translation of `SDL_SYS_HapticStopEffect()`.
    fn stop_effect(&self, haptic: &mut HapticData, _effect: usize) -> Result<()> {
        jni::haptic_stop(device_id(haptic));
        Ok(())
    }

    /// Translation of `SDL_SYS_HapticDestroyEffect()`.
    fn destroy_effect(&self, _haptic: &mut HapticData, _effect: usize) {}

    /// Translation of `SDL_SYS_HapticGetEffectStatus()`.
    fn effect_status(&self, _haptic: &mut HapticData, _effect: usize) -> Result<bool> {
        Ok(false)
    }

    /// Translation of `SDL_SYS_HapticSetGain()`.
    fn set_gain(&self, _haptic: &mut HapticData, _gain: i32) -> Result<()> {
        Ok(())
    }

    /// Translation of `SDL_SYS_HapticSetAutocenter()`.
    fn set_autocenter(&self, _haptic: &mut HapticData, _autocenter: i32) -> Result<()> {
        Ok(())
    }

    /// Translation of `SDL_SYS_HapticPause()`.
    fn pause(&self, _haptic: &mut HapticData) -> Result<()> {
        Ok(())
    }

    /// Translation of `SDL_SYS_HapticResume()`.
    fn resume(&self, _haptic: &mut HapticData) -> Result<()> {
        Ok(())
    }

    /// Translation of `SDL_SYS_HapticStopAll()`.
    fn stop_all(&self, _haptic: &mut HapticData) -> Result<()> {
        Ok(())
    }
}

/// A vibrator the haptic handler found. Translation of
/// `Android_AddHaptic()`.
pub(crate) fn add_haptic(device_id: i32, name: &str) -> bool {
    hapticlist().push(HapticlistItem {
        instance_id: crate::utils::next_object_id(),
        device_id,
        name: name.to_owned(),
        haptic: false,
    });
    true
}

/// A vibrator the haptic handler lost: whether it was open. Translation
/// of `Android_RemoveHaptic()`.
pub(crate) fn remove_haptic(device_id: i32) -> bool {
    let mut list = hapticlist();
    // found it, remove it.
    if let Some(i) = list.iter().position(|item| item.device_id == device_id) {
        let item = list.remove(i);
        // Need to decrement the haptic count
        // !!! TODO: Send a haptic remove event?
        return item.haptic;
    }
    false
}

#[cfg(test)]
mod tests;
