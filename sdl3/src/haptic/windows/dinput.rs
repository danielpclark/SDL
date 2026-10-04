// Rust translation of src/haptic/windows/SDL_dinputhaptic.c and
// SDL_dinputhaptic_c.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! DirectInput force feedback for the Windows haptic driver: the devices
//! (attached, force feedback capable), their actuator axes and supported
//! effects, and the conversion of SDL effects to `DIEFFECT`s.

use std::ffi::c_void;
use std::sync::{Mutex, MutexGuard};

use windows_sys::core::GUID;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

use super::super::{
    HapticData, HapticDirection, HapticDirectionType, HapticEffect, HapticEffectSlot,
    HapticFeatures, HAPTIC_INFINITY,
};
use super::{add_haptic_device, remove_haptic_device, with_list, HapticHwData, HapticlistItem};
use crate::core::windows::directx::*;
use crate::core::windows::{
    co_initialize, co_uninitialize, helper_window, is_equal_guid, wide_to_utf8,
};
use crate::error::{Error, Result};
use crate::hints;
use crate::init::{was_init, InitFlags};
use crate::joystick::windows::dinput::C_DF_DIJOYSTICK2;

/// Internal stuff: `coinitialized` and `dinput`.
struct DinputState {
    coinitialized: bool,
    dinput: Option<DirectInput>,
}

static DINPUT: Mutex<DinputState> = Mutex::new(DinputState {
    coinitialized: false,
    dinput: None,
});

fn lock() -> MutexGuard<'static, DinputState> {
    DINPUT.lock().unwrap_or_else(|e| e.into_inner())
}

/// The DirectInput object, if initialized (a new reference, so the lock
/// isn't held while it calls back).
fn dinput() -> Option<DirectInput> {
    lock().dinput.clone()
}

/// Whether DirectInput is in use (for the tests).
#[cfg(test)]
pub(super) fn dinput_in_use() -> bool {
    lock().dinput.is_some()
}

/// Like `SDL_SetError` but for DX error codes. Translation of
/// `DI_SetError()` (which doesn't show the code).
fn di_set_error(str: &str) -> Error {
    Error::new(format!("Haptic error {str}"))
}

/// Translation of `SDL_DINPUT_HapticInit()`; on failure the caller cleans
/// up with `SDL_SYS_HapticQuit()`.
pub(super) fn haptic_init() -> Result<()> {
    if lock().dinput.is_some() {
        // Already open.
        return Err(Error::new("Haptic: SubSystem already open."));
    }

    if !hints::get_bool(hints::JOYSTICK_DIRECTINPUT, true) {
        // In some environments, IDirectInput8_Initialize / _EnumDevices can take a minute even with no controllers.
        return Ok(());
    }

    let ret = co_initialize();
    if ret < 0 {
        return Err(di_set_error("Coinitialize"));
    }

    lock().coinitialized = true;

    let dinput = DirectInput::create().map_err(|_| di_set_error("CoCreateInstance"))?;

    // Because we used CoCreateInstance, we need to Initialize it, first.
    // SAFETY: GetModuleHandleW(NULL) is the executable's handle.
    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    if instance.is_null() {
        return Err(crate::core::windows::set_error("GetModuleHandle() failed"));
    }
    dinput
        .initialize(instance, DIRECTINPUT_VERSION)
        .map_err(|_| di_set_error("Initializing DirectInput device"))?;
    lock().dinput = Some(dinput.clone());

    // Look for haptic devices.
    for dev_class in DI8DEVCLASS_DEVICE..=DI8DEVCLASS_GAMECTRL {
        if dev_class == DI8DEVCLASS_GAMECTRL && !was_init(InitFlags::JOYSTICK).is_empty() {
            // The joystick subsystem will manage adding DInput joystick haptic devices
            continue;
        }

        // Callback to find the haptic devices (EnumHapticsCallback()).
        dinput
            .enum_devices(
                dev_class,
                DIEDFL_FORCEFEEDBACK | DIEDFL_ATTACHEDONLY,
                |instance| {
                    haptic_maybe_add_device(instance);
                    true // continue enumerating
                },
            )
            .map_err(|_| di_set_error("Enumerating DirectInput devices"))?;
    }

    Ok(())
}

/// Add a device to the haptic list if it is an attached force feedback
/// device that isn't there yet. Translation of
/// `SDL_DINPUT_HapticMaybeAddDevice()`.
pub(crate) fn haptic_maybe_add_device(pdid_instance: &DIDEVICEINSTANCEW) -> bool {
    const NEEDFLAGS: u32 = DIDC_ATTACHED | DIDC_FORCEFEEDBACK;

    let Some(dinput) = dinput() else {
        return false; // not initialized. We'll pick these up on enumeration if we init later.
    };

    // Make sure we don't already have it
    if with_list(|l| {
        l.iter()
            .any(|item| item.instance.as_bytes() == pdid_instance.as_bytes())
    }) {
        return false; // Already added
    }

    // Open the device
    let Ok(device) = dinput.create_device(&pdid_instance.guidInstance) else {
        // DI_SetError("Creating DirectInput device",ret);
        return false;
    };

    // Get capabilities.
    let mut capabilities = DIDEVCAPS {
        dwSize: size_of::<DIDEVCAPS>() as u32,
        ..Default::default()
    };
    let ret = device.get_capabilities(&mut capabilities);
    drop(device);
    if ret.is_err() {
        // DI_SetError("Getting device capabilities",ret);
        return false;
    }

    if capabilities.dwFlags & NEEDFLAGS != NEEDFLAGS {
        return false; // not a device we can use.
    }

    add_haptic_device(HapticlistItem {
        instance_id: crate::utils::next_object_id(),
        name: wide_to_utf8(&pdid_instance.tszProductName),
        // Copy the instance over, useful for creating devices.
        instance: *pdid_instance,
        capabilities,
    })
}

/// Remove a device from the haptic list. Translation of
/// `SDL_DINPUT_HapticMaybeRemoveDevice()`.
pub(crate) fn haptic_maybe_remove_device(pdid_instance: &DIDEVICEINSTANCEW) -> bool {
    if lock().dinput.is_none() {
        return false; // not initialized, ignore this.
    }

    let index = with_list(|l| {
        l.iter()
            .position(|item| item.instance.as_bytes() == pdid_instance.as_bytes())
    });
    match index {
        // found it, remove it.
        Some(index) => remove_haptic_device(index),
        None => false,
    }
}

/// The `DIJOFS_*` offset of a force feedback axis, by its type.
fn axis_offset(guid: &GUID) -> Option<u32> {
    let offset = if is_equal_guid(guid, &GUID_XAXIS) {
        DIJOFS_X
    } else if is_equal_guid(guid, &GUID_YAXIS) {
        DIJOFS_Y
    } else if is_equal_guid(guid, &GUID_ZAXIS) {
        DIJOFS_Z
    } else if is_equal_guid(guid, &GUID_RXAXIS) {
        DIJOFS_RX
    } else if is_equal_guid(guid, &GUID_RYAXIS) {
        DIJOFS_RY
    } else if is_equal_guid(guid, &GUID_RZAXIS) {
        DIJOFS_RZ
    } else {
        return None;
    };
    Some(offset)
}

/// Callback to get supported axes: records the actuator axes, up to 3.
/// Returns whether to go on. Translation of `DI_DeviceObjectCallback()`.
pub(super) fn di_device_object_callback(
    dev: &DIDEVICEOBJECTINSTANCEW,
    axes: &mut [u32; 3],
    naxes: &mut i32,
) -> bool {
    if dev.dwType & DIDFT_AXIS != 0 && dev.dwFlags & DIDOI_FFACTUATOR != 0 {
        let Some(offset) = axis_offset(&dev.guidType) else {
            return true; // can't use this, go on.
        };

        // (naxes starts at 0 and stops at 3, so this stays in bounds)
        axes[*naxes as usize] = offset;
        *naxes += 1;

        // Currently using the artificial limit of 3 axes.
        if *naxes >= 3 {
            return false;
        }
    }

    true
}

/// The effects and the DirectInput GUIDs of their types, in the order of
/// `DI_EffectCallback()` (and `SDL_SYS_HapticEffectType()`).
const EFFECT_GUIDS: [(GUID, HapticFeatures); 12] = [
    (GUID_SPRING, HapticFeatures::SPRING),
    (GUID_DAMPER, HapticFeatures::DAMPER),
    (GUID_INERTIA, HapticFeatures::INERTIA),
    (GUID_FRICTION, HapticFeatures::FRICTION),
    (GUID_CONSTANTFORCE, HapticFeatures::CONSTANT),
    (GUID_CUSTOMFORCE, HapticFeatures::CUSTOM),
    (GUID_SINE, HapticFeatures::SINE),
    (GUID_SQUARE, HapticFeatures::SQUARE),
    (GUID_TRIANGLE, HapticFeatures::TRIANGLE),
    (GUID_SAWTOOTHUP, HapticFeatures::SAWTOOTHUP),
    (GUID_SAWTOOTHDOWN, HapticFeatures::SAWTOOTHDOWN),
    (GUID_RAMPFORCE, HapticFeatures::RAMP),
];

/// Callback to get all supported effects: the feature of an effect GUID.
/// Translation of `DI_EffectCallback()` (its `EFFECT_TEST()`s).
pub(super) fn di_effect_feature(guid: &GUID) -> HapticFeatures {
    EFFECT_GUIDS
        .iter()
        .filter(|(g, _)| is_equal_guid(guid, g))
        .fold(HapticFeatures::NONE, |supported, (_, s)| supported | *s)
}

/// Opens the haptic device.
///
///    Steps:
///       - Set cooperative level.
///       - Set data format.
///       - Acquire exclusiveness.
///       - Reset actuators.
///       - Get supported features.
///
/// Translation of `SDL_DINPUT_HapticOpenFromDevice()`.
fn haptic_open_from_device(
    haptic: &mut HapticData,
    device8: InputDevice,
    is_joystick: bool,
) -> Result<()> {
    // We'll use the device8 from now on.
    let device = &device8;
    let mut axes = [0u32; 3];

    /* !!! FIXME: opening a haptic device here first will make an attempt to
    !!! FIXME:  SDL_OpenJoystick() that same device fail later, since we
    !!! FIXME:  have it open in exclusive mode. But this will allow
    !!! FIXME:  SDL_OpenJoystick() followed by SDL_OpenHapticFromJoystick()
    !!! FIXME:  to work, and that's probably the common case. Still,
    !!! FIXME:  ideally, We need to unify the opening code. */

    let result = (|| -> Result<()> {
        if !is_joystick {
            // if is_joystick, we already set this up elsewhere.
            let helper_window = helper_window::hwnd().unwrap_or(std::ptr::null_mut());
            // Grab it exclusively to use force feedback stuff.
            device
                .set_cooperative_level(helper_window, DISCL_EXCLUSIVE | DISCL_BACKGROUND)
                .map_err(|_| di_set_error("Setting cooperative level to exclusive"))?;

            // Set data format.
            device
                .set_data_format(&C_DF_DIJOYSTICK2.0)
                .map_err(|_| di_set_error("Setting data format"))?;

            // Acquire the device.
            if device.acquire() < 0 {
                return Err(di_set_error("Acquiring DirectInput device"));
            }
        }

        // Get number of axes.
        let mut naxes = haptic.naxes;
        let ret = device.enum_objects(
            |object| di_device_object_callback(object, &mut axes, &mut naxes),
            DIDFT_AXIS,
        );
        haptic.naxes = naxes;
        ret.map_err(|_| di_set_error("Getting device axes"))?;

        // Reset all actuators - just in case.
        if device.send_force_feedback_command(DISFFC_RESET) < 0 {
            return Err(di_set_error("Resetting device"));
        }

        // Enabling actuators.
        if device.send_force_feedback_command(DISFFC_SETACTUATORSON) < 0 {
            return Err(di_set_error("Enabling actuators"));
        }

        // Get supported effects.
        let mut supported = haptic.supported;
        let ret = device.enum_effects(
            |pei| {
                supported |= di_effect_feature(&pei.guid);
                true // Check for more.
            },
            DIEFT_ALL,
        );
        haptic.supported = supported;
        ret.map_err(|_| di_set_error("Enumerating supported effects"))?;
        if haptic.supported.is_empty() {
            // Error since device supports nothing.
            return Err(Error::new(
                "Haptic: Internal error on finding supported effects.",
            ));
        }

        // Check autogain and autocenter.
        if device
            .set_property_dword(DIPROP_FFGAIN, 0, DIPH_DEVICE, 10000)
            .is_ok()
        {
            // Gain is supported.
            haptic.supported |= HapticFeatures::GAIN;
        }
        if device
            .set_property_dword(DIPROP_AUTOCENTER, 0, DIPH_DEVICE, DIPROPAUTOCENTER_OFF)
            .is_ok()
        {
            // Autocenter is supported.
            haptic.supported |= HapticFeatures::AUTOCENTER;
        }

        // Status is always supported.
        haptic.supported |= HapticFeatures::STATUS | HapticFeatures::PAUSE;
        Ok(())
    })();

    if let Err(e) = result {
        // Error handling
        device.unacquire();
        return Err(e);
    }

    // Check maximum effects.
    /* This is not actually supported as thus under windows,
    there is no way to tell the number of EFFECTS that a
    device can hold, so we'll just use a "random" number
    instead and put warnings in SDL_haptic.h */
    haptic.effects = (0..128).map(|_| HapticEffectSlot::default()).collect();
    haptic.nplaying = 128; // Even more impossible to get this then neffects.

    haptic.hwdata = Some(Box::new(HapticHwData {
        device: device8,
        axes,
        is_joystick,
    }));
    Ok(())
}

/// Translation of `SDL_DINPUT_HapticOpen()`.
pub(super) fn haptic_open(haptic: &mut HapticData, item: &DIDEVICEINSTANCEW) -> Result<()> {
    let dinput = dinput().ok_or_else(|| di_set_error("Creating DirectInput device"))?;

    // Open the device
    let device = dinput
        .create_device(&item.guidInstance)
        .map_err(|_| di_set_error("Creating DirectInput device"))?;

    // (on failure the device is released with the error)
    haptic_open_from_device(haptic, device, false)
}

/// Whether the haptic device and the joystick device are the same.
/// Translation of `SDL_DINPUT_JoystickSameHaptic()`.
pub(super) fn joystick_same_haptic(hwdata: &HapticHwData, joystick_device: &InputDevice) -> bool {
    // Get the device instances.
    let Ok(hap_instance) = hwdata.device.get_device_info() else {
        return false;
    };
    let Ok(joy_instance) = joystick_device.get_device_info() else {
        return false;
    };

    is_equal_guid(&hap_instance.guidInstance, &joy_instance.guidInstance)
}

/// Translation of `SDL_DINPUT_HapticOpenFromJoystick()`.
pub(super) fn haptic_open_from_joystick(
    haptic: &mut HapticData,
    joystick_device: &InputDevice,
) -> Result<()> {
    let joy_instance = joystick_device
        .get_device_info()
        .map_err(|_| Error::new("Haptic: GetDeviceInfo failed"))?;

    // Since it comes from a joystick we have to try to match it with a haptic device on our haptic list.
    let item = with_list(|l| {
        l.iter()
            .find(|item| is_equal_guid(&item.instance.guidInstance, &joy_instance.guidInstance))
            .map(|item| (item.instance_id, item.name.clone()))
    });
    let Some((instance_id, name)) = item else {
        return Err(Error::new("Couldn't find joystick in haptic device list"));
    };

    haptic.instance_id = instance_id;
    haptic.name = Some(name);
    // FIXME (upstream): the haptic device uses the joystick's device without
    // a reference of its own, so closing the joystick first leaves it
    // dangling; here it takes a reference (released on close, where
    // upstream skips the release).
    haptic_open_from_device(haptic, joystick_device.clone(), true)
}

/// Translation of `SDL_DINPUT_HapticClose()`.
pub(super) fn haptic_close(hwdata: HapticHwData) {
    hwdata.device.unacquire();

    // (dropping hwdata releases the device: upstream's when it isn't
    // grabbed by a joystick, the reference taken in
    // haptic_open_from_joystick() when it is)
}

/// Translation of `SDL_DINPUT_HapticQuit()`.
pub(super) fn haptic_quit() {
    let mut s = lock();
    s.dinput = None;

    if s.coinitialized {
        co_uninitialize();
        s.coinitialized = false;
    }
}

/// Converts an SDL trigger button to an DIEFFECT trigger button.
/// Translation of `DIGetTriggerButton()`.
pub(super) fn di_get_trigger_button(button: u16) -> u32 {
    let mut dw_trigger_button = DIEB_NOTRIGGER;

    if button != 0 {
        dw_trigger_button = dijofs_button(button as u32 - 1);
    }

    dw_trigger_button
}

/// Clamps and converts. Translation of `CCONVERT()`.
pub(super) fn cconvert(x: u16) -> u32 {
    if x > 0x7FFF {
        10000
    } else {
        (x as u32 * 10000) / 0x7FFF
    }
}

/// Just converts. Translation of `CONVERT()`.
pub(super) fn convert(x: i32) -> i32 {
    (x * 10000) / 0x7FFF
}

/// The type specific parameters of a [`DiEffect`]
/// (`lpvTypeSpecificParams`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TypeSpecificParams {
    Constant(DICONSTANTFORCE),
    Periodic(DIPERIODIC),
    /// One per axis.
    Condition(Vec<DICONDITION>),
    Ramp(DIRAMPFORCE),
    /// A `DICUSTOMFORCE`: its channel count, sample period and sample
    /// count, and the samples.
    Custom {
        c_channels: u32,
        dw_sample_period: u32,
        c_samples: u32,
        rgl_force_data: Vec<i32>,
    },
}

impl TypeSpecificParams {
    /// `cbTypeSpecificParams`
    fn size(&self) -> u32 {
        (match self {
            TypeSpecificParams::Constant(_) => size_of::<DICONSTANTFORCE>(),
            TypeSpecificParams::Periodic(_) => size_of::<DIPERIODIC>(),
            TypeSpecificParams::Condition(c) => size_of::<DICONDITION>() * c.len(),
            TypeSpecificParams::Ramp(_) => size_of::<DIRAMPFORCE>(),
            TypeSpecificParams::Custom { .. } => size_of::<DICUSTOMFORCE>(),
        }) as u32
    }
}

/// A `DIEFFECT` and what it points to. Translation of what
/// `SDL_SYS_ToDIEFFECT()` allocates (and `SDL_SYS_HapticFreeDIEFFECT()`
/// frees, here on drop).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DiEffect {
    pub(super) dw_flags: u32,
    pub(super) dw_duration: u32,
    pub(super) dw_gain: u32,
    pub(super) dw_trigger_button: u32,
    pub(super) dw_trigger_repeat_interval: u32,
    pub(super) dw_start_delay: u32,
    /// `cAxes`; `rgdwAxes` holds that many when it isn't zero.
    pub(super) c_axes: u32,
    pub(super) rgdw_axes: Vec<u32>,
    /// `rglDirection` (NULL without axes).
    pub(super) rgl_direction: Option<Vec<i32>>,
    /// `lpEnvelope` (NULL without an envelope).
    pub(super) lp_envelope: Option<DIENVELOPE>,
    pub(super) params: TypeSpecificParams,
}

impl DiEffect {
    /// Run `f` with the `DIEFFECT`, pointing into `self`.
    pub(super) fn with_dieffect<R>(&mut self, f: impl FnOnce(&DIEFFECT) -> R) -> R {
        let mut custom;
        let cb_type_specific_params = self.params.size();
        let lpv_type_specific_params: *mut c_void = match &mut self.params {
            TypeSpecificParams::Constant(constant) => (constant as *mut DICONSTANTFORCE).cast(),
            TypeSpecificParams::Periodic(periodic) => (periodic as *mut DIPERIODIC).cast(),
            TypeSpecificParams::Condition(condition) => condition.as_mut_ptr().cast(),
            TypeSpecificParams::Ramp(ramp) => (ramp as *mut DIRAMPFORCE).cast(),
            TypeSpecificParams::Custom {
                c_channels,
                dw_sample_period,
                c_samples,
                rgl_force_data,
            } => {
                custom = DICUSTOMFORCE {
                    cChannels: *c_channels,
                    dwSamplePeriod: *dw_sample_period,
                    cSamples: *c_samples,
                    rglForceData: rgl_force_data.as_mut_ptr(),
                };
                (&mut custom as *mut DICUSTOMFORCE).cast()
            }
        };
        let effect = DIEFFECT {
            dwSize: size_of::<DIEFFECT>() as u32, // Set the structure size.
            dwFlags: self.dw_flags,
            dwDuration: self.dw_duration,
            dwSamplePeriod: 0, // Not used by us.
            dwGain: self.dw_gain,
            dwTriggerButton: self.dw_trigger_button,
            dwTriggerRepeatInterval: self.dw_trigger_repeat_interval,
            cAxes: self.c_axes,
            rgdwAxes: if self.c_axes > 0 {
                self.rgdw_axes.as_mut_ptr()
            } else {
                std::ptr::null_mut()
            },
            rglDirection: self
                .rgl_direction
                .as_mut()
                .map_or(std::ptr::null_mut(), |d| d.as_mut_ptr()),
            lpEnvelope: self
                .lp_envelope
                .as_mut()
                .map_or(std::ptr::null_mut(), |e| e as *mut DIENVELOPE),
            cbTypeSpecificParams: cb_type_specific_params,
            lpvTypeSpecificParams: lpv_type_specific_params,
            dwStartDelay: self.dw_start_delay,
        };
        f(&effect)
    }
}

/// Sets the direction: `dwFlags` and `rglDirection` for `naxes` axes.
/// Translation of `SDL_SYS_SetDirection()`.
pub(super) fn set_direction(
    dw_flags: &mut u32,
    dir: &HapticDirection,
    naxes: u32,
) -> Option<Vec<i32>> {
    // Handle no axes a part.
    if naxes == 0 {
        *dw_flags |= DIEFF_SPHERICAL; // Set as default.
        return None;
    }

    // Has axes.
    let mut rgl_dir = vec![0i32; naxes as usize];

    match dir.kind {
        HapticDirectionType::Polar => {
            *dw_flags |= DIEFF_POLAR;
            rgl_dir[0] = dir.dir[0];
        }
        HapticDirectionType::Cartesian | HapticDirectionType::Spherical => {
            *dw_flags |= if dir.kind == HapticDirectionType::Cartesian {
                DIEFF_CARTESIAN
            } else {
                DIEFF_SPHERICAL
            };
            rgl_dir[0] = dir.dir[0];
            if naxes > 1 {
                rgl_dir[1] = dir.dir[1];
            }
            if naxes > 2 {
                rgl_dir[2] = dir.dir[2];
            }
        }
        HapticDirectionType::SteeringAxis => {
            *dw_flags |= DIEFF_CARTESIAN;
            rgl_dir[0] = 0;
        }
    }
    Some(rgl_dir)
}

/// The envelope of an effect: none when it has no attack and no fade.
fn envelope(
    attack_length: u16,
    attack_level: u16,
    fade_length: u16,
    fade_level: u16,
) -> Option<DIENVELOPE> {
    if attack_length == 0 && fade_length == 0 {
        return None;
    }
    Some(DIENVELOPE {
        dwSize: size_of::<DIENVELOPE>() as u32, // Always should be this.
        dwAttackLevel: cconvert(attack_level),
        dwAttackTime: attack_length as u32 * 1000,
        dwFadeLevel: cconvert(fade_level),
        dwFadeTime: fade_length as u32 * 1000,
    })
}

/// The generic part of an effect: its direction, duration (ms), delay (ms),
/// trigger button and interval.
struct Generics<'a> {
    direction: &'a HapticDirection,
    length: u32,
    delay: u16,
    button: u16,
    interval: u16,
}

/// Creates the DIEFFECT from a SDL_HapticEffect, for a device with `naxes`
/// axes, `hw_axes` being their offsets. Translation of
/// `SDL_SYS_ToDIEFFECT()`.
pub(super) fn to_dieffect(naxes: i32, hw_axes: &[u32; 3], src: &HapticEffect) -> Result<DiEffect> {
    let generics = match src {
        HapticEffect::Constant(c) => Generics {
            direction: &c.direction,
            length: c.length,
            delay: c.delay,
            button: c.button,
            interval: c.interval,
        },
        HapticEffect::Periodic(p) => Generics {
            direction: &p.direction,
            length: p.length,
            delay: p.delay,
            button: p.button,
            interval: p.interval,
        },
        HapticEffect::Condition(c) => Generics {
            direction: &c.direction,
            length: c.length,
            delay: c.delay,
            button: c.button,
            interval: c.interval,
        },
        HapticEffect::Ramp(r) => Generics {
            direction: &r.direction,
            length: r.length,
            delay: r.delay,
            button: r.button,
            interval: r.interval,
        },
        HapticEffect::Custom(c) => Generics {
            direction: &c.direction,
            length: c.length,
            delay: c.delay,
            button: c.button,
            interval: c.interval,
        },
        HapticEffect::LeftRight(_) => return Err(Error::new("Haptic: Unknown effect type.")),
    };

    // Set global stuff.
    let mut dw_flags = DIEFF_OBJECTOFFSETS; // Seems obligatory.

    // Axes.
    let c_axes = if generics.direction.kind == HapticDirectionType::SteeringAxis {
        1
    } else {
        naxes as u32
    };
    // FIXME (upstream): with more than 3 axes (the haptic axes hint can set
    // that), the axes after the third are left uninitialized, and condition
    // effects read past their 3-entry arrays; here those read as 0.
    let rgdw_axes: Vec<u32> = (0..c_axes as usize)
        .map(|i| hw_axes.get(i).copied().unwrap_or(0)) // Always at least one axis.
        .collect();

    // Generics
    // FIXME (upstream): `length * 1000UL` wraps (unsigned long is 32 bits
    // on Windows), so long and infinite lengths don't become DirectInput's
    // INFINITE; and the trigger repeat interval is given in milliseconds
    // where DirectInput wants microseconds.
    let dw_duration = generics.length.wrapping_mul(1000); // In microseconds.
    let dw_trigger_button = di_get_trigger_button(generics.button);
    let dw_trigger_repeat_interval = generics.interval as u32;
    let dw_start_delay = generics.delay as u32 * 1000; // In microseconds.

    // Direction.
    let rgl_direction = set_direction(&mut dw_flags, generics.direction, c_axes);

    // The big type handling switch, even bigger than Linux's version.
    let (params, lp_envelope) = match src {
        HapticEffect::Constant(hap_constant) => (
            // Specifics
            TypeSpecificParams::Constant(DICONSTANTFORCE {
                lMagnitude: convert(hap_constant.level as i32),
            }),
            envelope(
                hap_constant.attack_length,
                hap_constant.attack_level,
                hap_constant.fade_length,
                hap_constant.fade_level,
            ),
        ),

        HapticEffect::Periodic(hap_periodic) => (
            // Specifics
            TypeSpecificParams::Periodic(DIPERIODIC {
                dwMagnitude: convert((hap_periodic.magnitude as i32).abs()) as u32,
                lOffset: convert(hap_periodic.offset as i32),
                dwPhase: (hap_periodic.phase as u32
                    + if hap_periodic.magnitude < 0 { 18000 } else { 0 })
                    % 36000,
                dwPeriod: hap_periodic.period as u32 * 1000,
            }),
            envelope(
                hap_periodic.attack_length,
                hap_periodic.attack_level,
                hap_periodic.fade_length,
                hap_periodic.fade_level,
            ),
        ),

        HapticEffect::Condition(hap_condition) => {
            if c_axes == 0 {
                // (SDL_calloc(0, ...) fails)
                return Err(Error::out_of_memory());
            }
            // Specifics
            let at = |values: &[i16; 3], i: usize| values.get(i).copied().unwrap_or(0) as i32;
            let atu = |values: &[u16; 3], i: usize| values.get(i).copied().unwrap_or(0);
            let condition = (0..c_axes as usize)
                .map(|i| DICONDITION {
                    lOffset: convert(at(&hap_condition.center, i)),
                    lPositiveCoefficient: convert(at(&hap_condition.right_coeff, i)),
                    lNegativeCoefficient: convert(at(&hap_condition.left_coeff, i)),
                    dwPositiveSaturation: cconvert(atu(&hap_condition.right_sat, i) / 2),
                    dwNegativeSaturation: cconvert(atu(&hap_condition.left_sat, i) / 2),
                    lDeadBand: cconvert(atu(&hap_condition.deadband, i) / 2) as i32,
                })
                .collect();
            // Envelope - Not actually supported by most CONDITION implementations.
            (TypeSpecificParams::Condition(condition), None)
        }

        HapticEffect::Ramp(hap_ramp) => (
            // Specifics
            TypeSpecificParams::Ramp(DIRAMPFORCE {
                lStart: convert(hap_ramp.start as i32),
                lEnd: convert(hap_ramp.end as i32),
            }),
            envelope(
                hap_ramp.attack_length,
                hap_ramp.attack_level,
                hap_ramp.fade_length,
                hap_ramp.fade_level,
            ),
        ),

        HapticEffect::Custom(hap_custom) => {
            // Specifics
            // FIXME (upstream): the samples are read for channels * samples
            // entries whatever the size of the data array; missing ones read
            // as 0 here.
            let count = hap_custom.samples as usize * hap_custom.channels as usize;
            let rgl_force_data = (0..count)
                .map(|i| cconvert(hap_custom.data.get(i).copied().unwrap_or(0)) as i32)
                .collect();
            (
                TypeSpecificParams::Custom {
                    c_channels: hap_custom.channels as u32,
                    dw_sample_period: hap_custom.period as u32 * 1000,
                    c_samples: hap_custom.samples as u32,
                    rgl_force_data,
                },
                envelope(
                    hap_custom.attack_length,
                    hap_custom.attack_level,
                    hap_custom.fade_length,
                    hap_custom.fade_level,
                ),
            )
        }

        HapticEffect::LeftRight(_) => return Err(Error::new("Haptic: Unknown effect type.")),
    };

    Ok(DiEffect {
        dw_flags,
        dw_duration,
        dw_gain: 10000, // Gain is set globally, not locally.
        dw_trigger_button,
        dw_trigger_repeat_interval,
        dw_start_delay,
        c_axes,
        rgdw_axes,
        rgl_direction,
        lp_envelope,
        params,
    })
}

/// Gets the effect type from the generic SDL haptic effect wrapper.
/// Translation of `SDL_SYS_HapticEffectType()`.
pub(super) fn haptic_effect_type(effect: &HapticEffect) -> Option<GUID> {
    // (the type of each effect but the left/right one has its GUID)
    let feature = effect.effect_type();
    EFFECT_GUIDS
        .iter()
        .find(|(_, f)| *f == feature)
        .map(|(guid, _)| *guid)
}

/// Translation of `SDL_DINPUT_HapticNewEffect()`: the effect and the
/// DirectInput object made of it.
pub(super) fn haptic_new_effect(
    hwdata: &HapticHwData,
    naxes: i32,
    base: &HapticEffect,
) -> Result<(DiEffect, Effect)> {
    let Some(effect_type) = haptic_effect_type(base) else {
        return Err(Error::new("Haptic: Unknown effect type."));
    };

    // Get the effect.
    let mut effect = to_dieffect(naxes, &hwdata.axes, base)?;

    // Create the actual effect.
    let reference = effect
        .with_dieffect(|dieffect| hwdata.device.create_effect(&effect_type, dieffect))
        .map_err(|_| di_set_error("Unable to create effect"))?;

    Ok((effect, reference))
}

/// Translation of `DIGetDirectionUpdateFlag()`.
pub(super) fn di_get_direction_update_flag(before: &DiEffect, after: &DiEffect) -> bool {
    if before.c_axes != after.c_axes {
        return true;
    }
    // rglDirection must be non-null for DIEP_DIRECTION to be a valid flag
    let Some(after_direction) = &after.rgl_direction else {
        return false;
    };
    let Some(before_direction) = &before.rgl_direction else {
        return true;
    };
    before_direction != after_direction
}

/// Translation of `DIGetEnvelopeUpdateFlag()`.
pub(super) fn di_get_envelope_update_flag(before: &DiEffect, after: &DiEffect) -> bool {
    match (&before.lp_envelope, &after.lp_envelope) {
        (None, None) => false,
        // A null lpEnvelope is valid for DIEP_ENVELOPE (clears the envelope from the effect)
        (None, _) | (_, None) => true,
        (Some(b), Some(a)) => b != a,
    }
}

/// Translation of `DIGetTypeSpecificParamsUpdateFlag()`.
pub(super) fn di_get_type_specific_params_update_flag(before: &DiEffect, after: &DiEffect) -> bool {
    // Shouldn't happen since this implies an effect's type somehow changed, but need to check to avoid an out-of-bounds memcmp
    if before.params.size() != after.params.size() {
        return true;
    }
    // (lpvTypeSpecificParams is never NULL)
    match (&before.params, &after.params) {
        // FIXME (upstream): the memcmp() compares the DICUSTOMFORCE
        // structures, whose sample pointers always differ, rather than the
        // samples: custom effects always count as changed.
        (TypeSpecificParams::Custom { .. }, TypeSpecificParams::Custom { .. }) => true,
        (b, a) => b != a,
    }
}

/// Calculate the exact flags needed when updating an existing
/// DirectInput haptic effect. Translation of `DICalculateUpdateFlags()`.
pub(super) fn di_calculate_update_flags(before: &DiEffect, after: &DiEffect) -> u32 {
    let mut flags = 0;

    if di_get_direction_update_flag(before, after) {
        flags |= DIEP_DIRECTION;
    }

    if before.dw_duration != after.dw_duration {
        flags |= DIEP_DURATION;
    }

    if di_get_envelope_update_flag(before, after) {
        flags |= DIEP_ENVELOPE;
    }

    if before.dw_start_delay != after.dw_start_delay {
        flags |= DIEP_STARTDELAY;
    }

    if before.dw_trigger_button != after.dw_trigger_button {
        flags |= DIEP_TRIGGERBUTTON;
    }

    if before.dw_trigger_repeat_interval != after.dw_trigger_repeat_interval {
        flags |= DIEP_TRIGGERREPEATINTERVAL;
    }

    if di_get_type_specific_params_update_flag(before, after) {
        flags |= DIEP_TYPESPECIFICPARAMS;
    }

    if flags == 0 {
        /* Awkward: SDL_UpdateHapticEffect was called, but nothing was changed.
         * Calling IDirectInputEffect_SetParameters with no flags is nonsense,
         * so our options are to either send all the flags, or exit early.
         * Sending all the flags seems like the safer option: The programmer may be trying
         * to force an update for some reason (e.g. driver bug workaround?). Conversely,
         * if the programmer doesn't want IDirectInputEffect_SetParameters to be called, they
         * can just avoid calling SDL_UpdateHapticEffect when there's no changes. */
        flags = DIEP_DIRECTION
            | DIEP_DURATION
            | DIEP_ENVELOPE
            | DIEP_STARTDELAY
            | DIEP_TRIGGERBUTTON
            | DIEP_TRIGGERREPEATINTERVAL
            | DIEP_TYPESPECIFICPARAMS;
    }

    flags
}

/// Translation of `SDL_DINPUT_HapticUpdateEffect()`: the new effect, to
/// replace `effect`, once the device has it.
pub(super) fn haptic_update_effect(
    hwdata: &HapticHwData,
    naxes: i32,
    effect: &DiEffect,
    reference: &Effect,
    data: &HapticEffect,
) -> Result<DiEffect> {
    // Get the effect.
    let mut temp = to_dieffect(naxes, &hwdata.axes, data)?;

    let flags = di_calculate_update_flags(effect, &temp);

    let device = &hwdata.device;
    let ret = temp.with_dieffect(|dieffect| {
        // Create the actual effect.
        let mut ret = reference.set_parameters(dieffect, flags);
        if ret == DIERR_NOTEXCLUSIVEACQUIRED {
            device.unacquire();
            let helper_window = helper_window::hwnd().unwrap_or(std::ptr::null_mut());
            if device
                .set_cooperative_level(helper_window, DISCL_EXCLUSIVE | DISCL_BACKGROUND)
                .is_ok()
            {
                ret = DIERR_NOTACQUIRED;
            }
        }
        if ret == DIERR_INPUTLOST || ret == DIERR_NOTACQUIRED {
            ret = device.acquire();
            if ret >= 0 {
                ret = reference.set_parameters(dieffect, flags);
            }
        }
        ret
    });
    if ret < 0 {
        return Err(di_set_error("Unable to update effect"));
    }

    // Copy it over.
    Ok(temp)
}

/// Translation of `SDL_DINPUT_HapticRunEffect()`.
pub(super) fn haptic_run_effect(reference: &Effect, iterations: u32) -> Result<()> {
    // Check if it's infinite.
    let iter = if iterations == HAPTIC_INFINITY {
        windows_sys::Win32::System::Threading::INFINITE
    } else {
        iterations
    };

    // Run the effect.
    if reference.start(iter, 0) < 0 {
        return Err(di_set_error("Running the effect"));
    }
    Ok(())
}

/// Translation of `SDL_DINPUT_HapticStopEffect()`.
pub(super) fn haptic_stop_effect(reference: &Effect) -> Result<()> {
    if reference.stop() < 0 {
        return Err(di_set_error("Unable to stop effect"));
    }
    Ok(())
}

/// Translation of `SDL_DINPUT_HapticDestroyEffect()`.
pub(super) fn haptic_destroy_effect(reference: Effect) {
    if reference.unload() < 0 {
        // ("Haptic error Removing effect from the device", which nobody reads)
        let _ = di_set_error("Removing effect from the device");
    }
    // FIXME (upstream): the effect object is unloaded but never released,
    // which leaks it; here dropping it releases it.
}

/// Translation of `SDL_DINPUT_HapticGetEffectStatus()`.
pub(super) fn haptic_get_effect_status(reference: &Effect) -> Result<bool> {
    let status = reference
        .get_effect_status()
        .map_err(|_| di_set_error("Getting effect status"))?;

    Ok(status != 0)
}

/// Translation of `SDL_DINPUT_HapticSetGain()`.
pub(super) fn haptic_set_gain(hwdata: &HapticHwData, gain: i32) -> Result<()> {
    // Create the weird structure thingy.
    let dw_data = (gain as u32).wrapping_mul(100); // 0 to 10,000

    // Try to set the autocenter.
    hwdata
        .device
        .set_property_dword(DIPROP_FFGAIN, 0, DIPH_DEVICE, dw_data)
        .map_err(|_| di_set_error("Setting gain"))?;
    Ok(())
}

/// Translation of `SDL_DINPUT_HapticSetAutocenter()`.
pub(super) fn haptic_set_autocenter(hwdata: &HapticHwData, autocenter: i32) -> Result<()> {
    // Create the weird structure thingy.
    let dw_data = if autocenter == 0 {
        DIPROPAUTOCENTER_OFF
    } else {
        DIPROPAUTOCENTER_ON
    };

    // Try to set the autocenter.
    hwdata
        .device
        .set_property_dword(DIPROP_AUTOCENTER, 0, DIPH_DEVICE, dw_data)
        .map_err(|_| di_set_error("Setting autocenter"))?;
    Ok(())
}

/// Send a force feedback command, failing with `what`.
fn send_command(hwdata: &HapticHwData, command: u32, what: &str) -> Result<()> {
    if hwdata.device.send_force_feedback_command(command) < 0 {
        return Err(di_set_error(what));
    }
    Ok(())
}

/// Translation of `SDL_DINPUT_HapticPause()`.
pub(super) fn haptic_pause(hwdata: &HapticHwData) -> Result<()> {
    // Pause the device.
    send_command(hwdata, DISFFC_PAUSE, "Pausing the device")
}

/// Translation of `SDL_DINPUT_HapticResume()`.
pub(super) fn haptic_resume(hwdata: &HapticHwData) -> Result<()> {
    // Unpause the device.
    // FIXME (upstream): the error message says "Pausing" (copied from
    // SDL_DINPUT_HapticPause()).
    send_command(hwdata, DISFFC_CONTINUE, "Pausing the device")
}

/// Translation of `SDL_DINPUT_HapticStopAll()`.
pub(super) fn haptic_stop_all(hwdata: &HapticHwData) -> Result<()> {
    // Try to stop the effects.
    send_command(hwdata, DISFFC_STOPALL, "Stopping the device")
}
