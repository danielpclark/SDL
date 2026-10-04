// Rust translation of src/haptic/hidapi/SDL_hidapihaptic.c,
// SDL_hidapihaptic.h and SDL_hidapihaptic_c.h from Simple DirectMedia Layer.
// Copyright (C) 2025 Katharine Chui <katharine.chui@gmail.com>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The HIDAPI haptic drivers: force feedback for the joysticks of the
//! HIDAPI joystick driver, sent through the joystick
//! ([`Joystick::send_effect`]). The front end tries them before the
//! platform driver; so far there is one, for the Logitech wheels
//! ([`lg4ff`]).
//!
//! All hid command sent and effect rendering are ported from
//! <https://github.com/berarma/new-lg4ff>
//!
//! Upstream keeps a list of the haptics this layer opened, guarded by a
//! mutex, to tell them from the platform driver's
//! (`SDL_HIDAPI_HapticIsHidapi()`); here a HIDAPI haptic is one whose
//! `hwdata` is a [`HidapiHapticDevice`]. That list and its mutex are all
//! that `SDL_HIDAPI_HapticInit()` and `SDL_HIDAPI_HapticQuit()` set up and
//! destroy, so there is nothing left for them to do.

pub(crate) mod lg4ff;

use std::sync::Arc;

use super::{HapticData, HapticEffect, HapticEffectID, HapticEffectSlot, HapticFeatures};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::joystick::{assert_joysticks_locked, Joystick};

/// What a HIDAPI haptic driver uses of its joystick (the `SDL_Joystick *`
/// it is opened from); [`Joystick`] in the library, a fake in the tests.
pub(crate) trait HapticJoystick: Send + Sync {
    /// `SDL_GetJoystickID()`
    fn id(&self) -> JoystickID;
    /// `SDL_GetJoystickVendor()`
    fn vendor(&self) -> u16;
    /// `SDL_GetJoystickProduct()`
    fn product(&self) -> u16;
    /// `SDL_GetJoystickProductVersion()`
    fn product_version(&self) -> u16;
    /// `SDL_SendJoystickEffect()`
    fn send_effect(&self, data: &[u8]) -> Result<()>;
}

impl HapticJoystick for Joystick {
    fn id(&self) -> JoystickID {
        Joystick::id(self)
    }
    fn vendor(&self) -> u16 {
        Joystick::vendor(self)
    }
    fn product(&self) -> u16 {
        Joystick::product(self)
    }
    fn product_version(&self) -> u16 {
        Joystick::product_version(self)
    }
    fn send_effect(&self, data: &[u8]) -> Result<()> {
        Joystick::send_effect(self, data)
    }
}

/// A HIDAPI haptic driver: the `JoystickSupported` and `Open` functions
/// of `SDL_HIDAPI_HapticDriver`; the others are the
/// [`HidapiHapticContext`]'s that `open` returns.
pub(crate) trait HidapiHapticDriver: Sync {
    /// Whether the haptic can be opened from the joystick
    /// (`JoystickSupported`).
    fn joystick_supported(&self, joystick: &dyn HapticJoystick) -> bool;

    /// Open the haptic of a joystick (`Open`); the driver sets the error.
    fn open(&self, joystick: Arc<dyn HapticJoystick>) -> Result<Box<dyn HidapiHapticContext>>;
}

/// An open HIDAPI haptic: the driver context and the effect functions of
/// `SDL_HIDAPI_HapticDriver` (they mirror the effect interface of
/// `SDL_haptic.h`).
///
/// The joystick stays open while the context exists (upstream's "functions
/// below need to handle the possibility of a null joystick instance" can't
/// happen), though it may be disconnected.
pub(crate) trait HidapiHapticContext: Send {
    /// Clean up what `open` set up (`Close`); the context is dropped after.
    fn close(&mut self);
    /// The number of effects the device can store (`NumEffects`).
    fn num_effects(&self) -> i32;
    /// The number of effects the device can play concurrently
    /// (`NumEffectsPlaying`).
    fn num_effects_playing(&self) -> i32;
    /// The supported effects (`GetFeatures`).
    fn features(&self) -> HapticFeatures;
    /// The number of haptic axes (`NumAxes`).
    fn num_axes(&self) -> i32;
    /// Create an effect, returning its ID (`CreateEffect`).
    fn create_effect(&mut self, data: &HapticEffect) -> Result<HapticEffectID>;
    /// `UpdateEffect`
    fn update_effect(&mut self, id: HapticEffectID, data: &HapticEffect) -> Result<()>;
    /// `RunEffect`
    fn run_effect(&mut self, id: HapticEffectID, iterations: u32) -> Result<()>;
    /// `StopEffect`
    fn stop_effect(&mut self, id: HapticEffectID) -> Result<()>;
    /// `DestroyEffect`
    fn destroy_effect(&mut self, id: HapticEffectID);
    /// Whether the effect is playing; `false` if not, or on error
    /// (`GetEffectStatus`).
    fn effect_status(&mut self, id: HapticEffectID) -> bool;
    /// Set the gain, 0 to 100 (`SetGain`).
    fn set_gain(&mut self, gain: i32) -> Result<()>;
    /// Set the autocenter, 0 to 100 (`SetAutocenter`).
    fn set_autocenter(&mut self, autocenter: i32) -> Result<()>;
    /// `Pause`
    fn pause(&mut self) -> Result<()>;
    /// `Resume`
    fn resume(&mut self) -> Result<()>;
    /// `StopEffects`
    fn stop_effects(&mut self) -> Result<()>;
}

/// The `hwdata` of a HIDAPI haptic. Translation of
/// `SDL_HIDAPI_HapticDevice` (its `haptic` back reference isn't needed).
pub(crate) struct HidapiHapticDevice {
    /// The driver context (`ctx`), declared first so that it is dropped
    /// (after being closed) before the joystick reference.
    ctx: Box<dyn HidapiHapticContext>,
    /// The related HIDAPI joystick: a reference of its own, so that the
    /// joystick isn't fully destroyed before the haptic is closed.
    joystick: Arc<dyn HapticJoystick>,
}

/// The HIDAPI haptic drivers. Translation of `drivers`.
static DRIVERS: &[&dyn HidapiHapticDriver] = &[&lg4ff::LG4FF_HAPTIC_DRIVER];

/// The device of a HIDAPI haptic.
fn device(haptic: &mut HapticData) -> Result<&mut HidapiHapticDevice> {
    haptic
        .hwdata
        .as_mut()
        .and_then(|h| h.downcast_mut::<HidapiHapticDevice>())
        .ok_or_else(|| Error::invalid_param("haptic"))
}

/// Whether a haptic was opened by this layer. Translation of
/// `SDL_HIDAPI_HapticIsHidapi()`.
pub(crate) fn is_hidapi(haptic: &HapticData) -> bool {
    haptic
        .hwdata
        .as_ref()
        .is_some_and(|h| h.is::<HidapiHapticDevice>())
}

/// Whether a HIDAPI haptic driver supports the joystick. Translation of
/// `SDL_HIDAPI_JoystickIsHaptic()`.
pub(crate) fn joystick_is_haptic(joystick: &Joystick) -> bool {
    assert_joysticks_locked();

    if !crate::joystick::hidapi::is_hidapi_joystick(joystick) {
        return false;
    }

    DRIVERS.iter().any(|d| d.joystick_supported(joystick))
}

/// Open the haptic of a HIDAPI joystick. Translation of
/// `SDL_HIDAPI_HapticOpenFromJoystick()`.
pub(crate) fn open_from_joystick(haptic: &mut HapticData, joystick: &Joystick) -> Result<()> {
    assert_joysticks_locked();

    if !crate::joystick::hidapi::is_hidapi_joystick(joystick) {
        return Err(Error::new(
            "Cannot open hidapi haptic from non hidapi joystick",
        ));
    }

    for driver in DRIVERS {
        if driver.joystick_supported(joystick) {
            // grab a joystick ref so that it doesn't get fully destroyed before the haptic is closed
            // (upstream grabs it after the driver's Open succeeded; the driver context holds it
            // here, and an Open that fails just drops it again)
            let reference: Arc<dyn HapticJoystick> = Arc::new(Joystick::open(joystick.id())?);

            // the driver is responsible for setting the error
            let ctx = driver.open(reference.clone())?;
            let device = HidapiHapticDevice {
                ctx,
                joystick: reference,
            };

            // this is outside of the syshaptic driver

            let neffects = usize::try_from(device.ctx.num_effects()).unwrap_or(0);
            haptic.nplaying = device.ctx.num_effects_playing();
            haptic.supported = device.ctx.features();
            haptic.naxes = device.ctx.num_axes();
            haptic.effects = (0..neffects).map(|_| HapticEffectSlot::default()).collect();

            // outside of SYS_HAPTIC
            // FIXME (upstream): every HIDAPI haptic gets instance ID 255,
            // which a platform driver's device (whose IDs come from the
            // object ID counter) can have too.
            haptic.instance_id = 255;

            haptic.hwdata = Some(Box::new(device));
            return Ok(());
        }
    }

    Err(Error::new(
        "No supported HIDAPI haptic driver found for joystick",
    ))
}

/// Whether a haptic is the one of the joystick. Translation of
/// `SDL_HIDAPI_JoystickSameHaptic()`.
pub(crate) fn joystick_same_haptic(haptic: &HapticData, joystick: &Joystick) -> bool {
    assert_joysticks_locked();
    if !crate::joystick::hidapi::is_hidapi_joystick(joystick) {
        return false;
    }

    // Note (upstream): upstream takes the hwdata of any haptic in the list
    // for a HIDAPI device, the platform driver's too.
    let Some(device) = haptic
        .hwdata
        .as_ref()
        .and_then(|h| h.downcast_ref::<HidapiHapticDevice>())
    else {
        return false;
    };

    // (the same open joystick: a joystick ID has one while it's open)
    device.joystick.id() == joystick.id()
}

/// Close a HIDAPI haptic. Translation of `SDL_HIDAPI_HapticClose()`.
pub(crate) fn close(haptic: &mut HapticData) {
    let Some(hwdata) = haptic.hwdata.take() else {
        return;
    };
    match hwdata.downcast::<HidapiHapticDevice>() {
        Ok(mut device) => {
            device.ctx.close();

            // a reference was grabbed during open, now release it
            // (with the context, which holds it too)
            drop(device);
            haptic.effects.clear();
        }
        Err(hwdata) => haptic.hwdata = Some(hwdata),
    }
}

/// Create an effect. Translation of `SDL_HIDAPI_HapticNewEffect()`.
pub(crate) fn new_effect(haptic: &mut HapticData, base: &HapticEffect) -> Result<HapticEffectID> {
    let new_id = device(haptic)?.ctx.create_effect(base)?;
    if let Some(slot) = usize::try_from(new_id)
        .ok()
        .and_then(|i| haptic.effects.get_mut(i))
    {
        slot.effect = Some(base.clone());
    }
    Ok(new_id)
}

/// Translation of `SDL_HIDAPI_HapticUpdateEffect()` (which, unlike the
/// platform drivers' path, doesn't store the new effect in the front end's
/// slot).
pub(crate) fn update_effect(
    haptic: &mut HapticData,
    id: HapticEffectID,
    data: &HapticEffect,
) -> Result<()> {
    device(haptic)?.ctx.update_effect(id, data)
}

/// Translation of `SDL_HIDAPI_HapticRunEffect()`.
pub(crate) fn run_effect(
    haptic: &mut HapticData,
    id: HapticEffectID,
    iterations: u32,
) -> Result<()> {
    device(haptic)?.ctx.run_effect(id, iterations)
}

/// Translation of `SDL_HIDAPI_HapticStopEffect()`.
pub(crate) fn stop_effect(haptic: &mut HapticData, id: HapticEffectID) -> Result<()> {
    device(haptic)?.ctx.stop_effect(id)
}

/// Translation of `SDL_HIDAPI_HapticDestroyEffect()`.
pub(crate) fn destroy_effect(haptic: &mut HapticData, id: HapticEffectID) {
    if let Ok(device) = device(haptic) {
        device.ctx.destroy_effect(id);
    }
}

/// Translation of `SDL_HIDAPI_HapticGetEffectStatus()`.
pub(crate) fn effect_status(haptic: &mut HapticData, id: HapticEffectID) -> bool {
    device(haptic).is_ok_and(|device| device.ctx.effect_status(id))
}

/// Translation of `SDL_HIDAPI_HapticSetGain()`.
pub(crate) fn set_gain(haptic: &mut HapticData, gain: i32) -> Result<()> {
    device(haptic)?.ctx.set_gain(gain)
}

/// Translation of `SDL_HIDAPI_HapticSetAutocenter()`.
pub(crate) fn set_autocenter(haptic: &mut HapticData, autocenter: i32) -> Result<()> {
    device(haptic)?.ctx.set_autocenter(autocenter)
}

/// Translation of `SDL_HIDAPI_HapticPause()`.
pub(crate) fn pause(haptic: &mut HapticData) -> Result<()> {
    device(haptic)?.ctx.pause()
}

/// Translation of `SDL_HIDAPI_HapticResume()`.
pub(crate) fn resume(haptic: &mut HapticData) -> Result<()> {
    device(haptic)?.ctx.resume()
}

/// Translation of `SDL_HIDAPI_HapticStopAll()`.
pub(crate) fn stop_all(haptic: &mut HapticData) -> Result<()> {
    device(haptic)?.ctx.stop_effects()
}

#[cfg(test)]
mod tests;
