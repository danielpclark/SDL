// Rust translation of src/haptic/darwin/SDL_syshaptic.c and
// SDL_syshaptic_c.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Darwin haptic driver: the HID devices with force feedback, through
//! the ForceFeedback framework. The IOKit joystick driver adds and removes
//! its joysticks' force feedback services as it finds and loses them
//! (`MacHaptic_MaybeAddDevice()` / `MacHaptic_MaybeRemoveDevice()`).

#[cfg(test)]
mod tests;

use std::ffi::c_void;
use std::sync::{Mutex, MutexGuard};

use super::{
    HapticData, HapticDirection, HapticDirectionType, HapticDriver, HapticEffect, HapticEffectSlot,
    HapticFeatures, HapticID, HAPTIC_INFINITY,
};
use crate::core::apple::foundation::{
    cf_dictionary_get, kCFAllocatorDefault, CFGetTypeID, CFNumberGetValue, CFStringGetTypeID,
    CfRef, K_CF_NUMBER_LONG_TYPE,
};
use crate::core::apple::iokit::*;
use crate::error::{Error, Result};
use crate::joystick::Joystick;

/// An available haptic device. Translation of `SDL_hapticlist_item`
/// (whose `haptic`, never set, isn't kept).
struct HapticlistItem {
    instance_id: HapticID,
    /// Name of the device.
    name: String,

    /// Node we use to create the device.
    dev: IoService,

    // Usage pages for determining if it's a mouse or not.
    usage: i64,
    usage_page: i64,
}

/// The device list: `SDL_hapticlist` (with `SDL_hapticlist_tail`, its end)
/// and `numhaptics` (-1 before the subsystem is initialized, as `None`).
struct HapticList {
    items: Option<Vec<HapticlistItem>>,
}

static HAPTICLIST: Mutex<HapticList> = Mutex::new(HapticList { items: None });

fn lock_list() -> MutexGuard<'static, HapticList> {
    HAPTICLIST.lock().unwrap_or_else(|e| e.into_inner())
}

/// Haptic system hardware data. Translation of `struct haptic_hwdata`.
struct HapticHwData {
    /// Hardware device.
    device: FFDeviceObjectReference,
    axes: [u8; 3],
}

// SAFETY: the ForceFeedback device is used with the haptic device locked,
// from one thread at a time.
unsafe impl Send for HapticHwData {}

/// The type specific parameters of an [`FfEffect`] (`lpvTypeSpecificParams`).
#[derive(Clone, Debug)]
pub(crate) enum TypeSpecificParams {
    Constant(FFCONSTANTFORCE),
    Periodic(FFPERIODIC),
    /// One per axis (none, and a null pointer, without axes).
    Condition(Vec<FFCONDITION>),
    Ramp(FFRAMPFORCE),
    /// An `FFCUSTOMFORCE`: its channel count, sample period and sample
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
            TypeSpecificParams::Constant(_) => size_of::<FFCONSTANTFORCE>(),
            TypeSpecificParams::Periodic(_) => size_of::<FFPERIODIC>(),
            TypeSpecificParams::Condition(c) => size_of::<FFCONDITION>() * c.len(),
            TypeSpecificParams::Ramp(_) => size_of::<FFRAMPFORCE>(),
            TypeSpecificParams::Custom { .. } => size_of::<FFCUSTOMFORCE>(),
        }) as u32
    }
}

/// An `FFEFFECT` and what it points to. Translation of what
/// `SDL_SYS_ToFFEFFECT()` allocates (and `SDL_SYS_HapticFreeFFEFFECT()`
/// frees, here on drop).
#[derive(Clone, Debug)]
pub(crate) struct FfEffect {
    pub(crate) dw_flags: u32,
    pub(crate) dw_duration: u32,
    pub(crate) dw_trigger_button: u32,
    pub(crate) dw_trigger_repeat_interval: u32,
    pub(crate) dw_start_delay: u32,
    /// `cAxes`; `rgdwAxes` holds that many when it isn't zero.
    pub(crate) c_axes: u32,
    pub(crate) rgdw_axes: Vec<u32>,
    /// `rglDirection` (NULL without axes).
    pub(crate) rgl_direction: Option<Vec<i32>>,
    /// `lpEnvelope` (NULL without an envelope).
    pub(crate) lp_envelope: Option<FFENVELOPE>,
    pub(crate) params: TypeSpecificParams,
}

impl FfEffect {
    /// Run `f` with the `FFEFFECT`, pointing into `self`.
    fn with_ffeffect<R>(&mut self, f: impl FnOnce(&mut FFEFFECT) -> R) -> R {
        let mut custom;
        let cb_type_specific_params = self.params.size();
        let lpv_type_specific_params: *mut c_void = match &mut self.params {
            TypeSpecificParams::Constant(constant) => (constant as *mut FFCONSTANTFORCE).cast(),
            TypeSpecificParams::Periodic(periodic) => (periodic as *mut FFPERIODIC).cast(),
            TypeSpecificParams::Condition(condition) => {
                if condition.is_empty() {
                    std::ptr::null_mut()
                } else {
                    condition.as_mut_ptr().cast()
                }
            }
            TypeSpecificParams::Ramp(ramp) => (ramp as *mut FFRAMPFORCE).cast(),
            TypeSpecificParams::Custom {
                c_channels,
                dw_sample_period,
                c_samples,
                rgl_force_data,
            } => {
                custom = FFCUSTOMFORCE {
                    c_channels: *c_channels,
                    dw_sample_period: *dw_sample_period,
                    c_samples: *c_samples,
                    rgl_force_data: rgl_force_data.as_mut_ptr(),
                };
                (&mut custom as *mut FFCUSTOMFORCE).cast()
            }
        };
        let mut effect = FFEFFECT {
            dw_size: size_of::<FFEFFECT>() as u32, // Set the structure size.
            dw_flags: self.dw_flags,
            dw_duration: self.dw_duration,
            dw_sample_period: 0, // Not used by us.
            dw_gain: 10000,      // Gain is set globally, not locally.
            dw_trigger_button: self.dw_trigger_button,
            dw_trigger_repeat_interval: self.dw_trigger_repeat_interval,
            c_axes: self.c_axes,
            rgdw_axes: if self.c_axes > 0 {
                self.rgdw_axes.as_mut_ptr()
            } else {
                std::ptr::null_mut()
            },
            rgl_direction: self
                .rgl_direction
                .as_mut()
                .map_or(std::ptr::null_mut(), |d| d.as_mut_ptr()),
            lp_envelope: self
                .lp_envelope
                .as_mut()
                .map_or(std::ptr::null_mut(), |e| e as *mut FFENVELOPE),
            cb_type_specific_params,
            lpv_type_specific_params,
            dw_start_delay: self.dw_start_delay,
        };
        f(&mut effect)
    }
}

/// Haptic system effect data. Translation of `struct haptic_hweffect`.
struct HapticHwEffect {
    /// Reference.
    reference: FFEffectObjectReference,
    /// Hardware effect.
    effect: FfEffect,
}

// SAFETY: as HapticHwData.
unsafe impl Send for HapticHwEffect {}

/// Initializes the haptic subsystem. Translation of `SDL_SYS_HapticInit()`.
fn haptic_init() -> Result<()> {
    {
        let mut list = lock_list();
        if list.items.is_some() {
            return Err(Error::new("Haptic subsystem already initialized!"));
        }
        list.items = Some(Vec::new());
    }

    // Get HID devices.
    // SAFETY: a NUL-terminated class name.
    let matching = unsafe { IOServiceMatching(K_IOHID_DEVICE_KEY.as_ptr()) };
    if matching.is_null() {
        return Err(Error::new("Haptic: Failed to get IOServiceMatching."));
    }

    // Now search I/O Registry for matching devices.
    let mut iter: IoIterator = 0;
    // SAFETY: the dictionary is consumed by the call.
    let result =
        unsafe { IOServiceGetMatchingServices(K_IO_MAIN_PORT_DEFAULT, matching, &mut iter) };
    if result != K_IO_RETURN_SUCCESS {
        return Err(Error::new("Haptic: Couldn't create a HID object iterator."));
    }
    // IOServiceGetMatchingServices consumes dictionary.

    // SAFETY: the iterator is ours; each entry is released once (and
    // retained by mac_haptic_maybe_add_device() if it keeps it).
    unsafe {
        if IOIteratorIsValid(iter) == 0 {
            // No iterator.
            return Ok(());
        }

        loop {
            let device = IOIteratorNext(iter);
            if device == IO_OBJECT_NULL {
                break;
            }
            mac_haptic_maybe_add_device(device);
            // always release as the AddDevice will retain IF it's a forcefeedback device
            IOObjectRelease(device);
        }
        IOObjectRelease(iter);
    }

    Ok(())
}

/// Add a HID service with force feedback, unless it is there already.
/// Translation of `MacHaptic_MaybeAddDevice()`.
pub(crate) fn mac_haptic_maybe_add_device(device: IoService) -> bool {
    if lock_list().items.is_none() {
        return false; // not initialized. We'll pick these up on enumeration if we init later.
    }

    // Check for force feedback.
    // SAFETY: a valid service.
    if unsafe { FFIsForceFeedback(device) } != FF_OK {
        return false;
    }

    // Make sure we don't already have it
    {
        let list = lock_list();
        let items = list.items.as_ref().map_or(&[][..], Vec::as_slice);
        // SAFETY: valid services.
        if items
            .iter()
            .any(|item| unsafe { IOObjectIsEqualTo(item.dev, device) } != 0)
        {
            // Already added
            return false;
        }
    }

    let instance_id = crate::utils::next_object_id();

    // retain it as we are going to keep it around a while
    // SAFETY: a valid service; released when the item is.
    unsafe { IOObjectRetain(device) };

    // Set basic device data.
    // (the errors HIDGetDeviceProduct() and the usage lookups set are
    // ignored by upstream)
    let name = hid_get_device_product(device).unwrap_or_default();

    // Set usage pages.
    let (mut usage, mut usage_page) = (0i64, 0i64);
    let mut hid_properties = std::ptr::null_mut();
    // SAFETY: a valid service; the properties are ours to release.
    let result = unsafe {
        IORegistryEntryCreateCFProperties(device, &mut hid_properties, kCFAllocatorDefault, 0)
    };
    // SAFETY: the dictionary IORegistryEntryCreateCFProperties() created.
    if let (KERN_SUCCESS, Some(hid_properties)) =
        (result, unsafe { CfRef::from_owned(hid_properties) })
    {
        // SAFETY: a dictionary of properties; the numbers are read as the
        // type named.
        unsafe {
            let r = cf_dictionary_get(hid_properties.get(), K_IOHID_PRIMARY_USAGE_PAGE_KEY);
            if !r.is_null() {
                // (on failure upstream sets "Haptic: Receiving device's usage page.")
                CFNumberGetValue(
                    r,
                    K_CF_NUMBER_LONG_TYPE,
                    (&mut usage_page as *mut i64).cast(),
                );
                let r = cf_dictionary_get(hid_properties.get(), K_IOHID_PRIMARY_USAGE_KEY);
                if !r.is_null() {
                    // (on failure upstream sets "Haptic: Receiving device's usage.")
                    CFNumberGetValue(r, K_CF_NUMBER_LONG_TYPE, (&mut usage as *mut i64).cast());
                }
            }
        }
    }

    let mut list = lock_list();
    let Some(items) = list.items.as_mut() else {
        // (the subsystem quit meanwhile)
        // SAFETY: the reference taken above.
        unsafe { IOObjectRelease(device) };
        return false;
    };
    items.push(HapticlistItem {
        instance_id,
        name,
        dev: device,
        usage,
        usage_page,
    });

    // Device has been added.
    true
}

/// Remove a HID service's device; whether it was open (upstream's
/// `item->haptic`, which is never set, so this is always false).
/// Translation of `MacHaptic_MaybeRemoveDevice()`.
pub(crate) fn mac_haptic_maybe_remove_device(device: IoService) -> bool {
    let mut list = lock_list();
    let Some(items) = list.items.as_mut() else {
        return false; // not initialized. ignore this.
    };

    // SAFETY: valid services.
    if let Some(i) = items
        .iter()
        .position(|item| unsafe { IOObjectIsEqualTo(item.dev, device) } != 0)
    {
        // found it, remove it.
        let item = items.remove(i);
        // !!! TODO: Send a haptic remove event?

        // SAFETY: the reference mac_haptic_maybe_add_device() took.
        unsafe { IOObjectRelease(item.dev) };
        return false;
    }

    false
}

/// Gets the device's product name. Translation of `HIDGetDeviceProduct()`.
fn hid_get_device_product(dev: IoService) -> Result<String> {
    let mut hid_properties = std::ptr::null_mut();
    // SAFETY: a valid service; the dictionary is ours to release.
    let ret = unsafe {
        IORegistryEntryCreateCFProperties(dev, &mut hid_properties, kCFAllocatorDefault, 0)
    };
    // Note (upstream): the HID properties are never released; here they
    // are, as are the USB properties and the parents on the error paths.
    // SAFETY: the dictionary created above, if any.
    let hid_properties = match (ret, unsafe { CfRef::from_owned(hid_properties) }) {
        (KERN_SUCCESS, Some(p)) => p,
        _ => return Err(Error::new("Haptic: Unable to create CFProperties.")),
    };

    /* macOS currently is not mirroring all USB properties to HID page so need to look at USB device page also
     * get dictionary for USB properties: step up two levels and get CF dictionary for USB properties
     */
    let mut parent1: IoRegistryEntry = 0;
    let mut parent2: IoRegistryEntry = 0;
    let mut usb_properties = std::ptr::null_mut();
    // SAFETY: valid registry entries; each parent is released below.
    let found = unsafe {
        KERN_SUCCESS
            == IORegistryEntryGetParentEntry(dev, K_IO_SERVICE_PLANE.as_ptr(), &mut parent1)
            && KERN_SUCCESS
                == IORegistryEntryGetParentEntry(parent1, K_IO_SERVICE_PLANE.as_ptr(), &mut parent2)
            && KERN_SUCCESS
                == IORegistryEntryCreateCFProperties(
                    parent2,
                    &mut usb_properties,
                    kCFAllocatorDefault,
                    0,
                )
    };
    let release_parents = || {
        // SAFETY: the parents IORegistryEntryGetParentEntry() returned.
        unsafe {
            if parent2 != 0 && K_IO_RETURN_SUCCESS != IOObjectRelease(parent2) {
                // ("Haptic: IOObjectRelease error with parent2.", which nobody reads)
            }
            if parent1 != 0 && K_IO_RETURN_SUCCESS != IOObjectRelease(parent1) {
                // ("Haptic: IOObjectRelease error with parent1.")
            }
        }
    };
    if !found {
        release_parents();
        return Err(Error::new("Haptic: Error getting registry entries."));
    }
    // SAFETY: the dictionary created above, if any.
    let Some(usb_properties) = (unsafe { CfRef::from_owned(usb_properties) }) else {
        release_parents();
        return Err(Error::new(
            "Haptic: IORegistryEntryCreateCFProperties failed to create usbProperties.",
        ));
    };

    /* get device info
     * try hid dictionary first, if fail then go to USB dictionary
     */

    // Get product name
    let mut name = String::new();
    // SAFETY: dictionaries of properties.
    let mut ref_cf = unsafe { cf_dictionary_get(hid_properties.get(), K_IOHID_PRODUCT_KEY) };
    if ref_cf.is_null() {
        // SAFETY: as above.
        ref_cf = unsafe { cf_dictionary_get(usb_properties.get(), "USB Product Name") };
    }
    if !ref_cf.is_null() {
        // SAFETY: a CF object from the dictionaries.
        match unsafe { c_string_system_encoding(ref_cf, 256) } {
            Some(s) => name = s,
            None => {
                release_parents();
                return Err(Error::new(
                    "Haptic: CFStringGetCString error retrieving pDevice->product.",
                ));
            }
        }
    }

    drop(usb_properties);

    // Release stuff.
    release_parents();

    Ok(name)
}

/// `CFStringGetCString(ref, buf, size, CFStringGetSystemEncoding())`, the
/// bytes read as UTF-8 (as SDL then uses them); `None` where it fails (not
/// a string, or longer than the buffer).
///
/// # Safety
///
/// `s` must be a valid CF object.
unsafe fn c_string_system_encoding(s: *const c_void, size: usize) -> Option<String> {
    use crate::core::apple::foundation::CFStringGetCString;

    // SAFETY: the caller's contract; the buffer is as large as said.
    unsafe {
        if CFGetTypeID(s) != CFStringGetTypeID() {
            return None;
        }
        let mut buf = vec![0u8; size];
        if CFStringGetCString(
            s,
            buf.as_mut_ptr().cast(),
            size as isize,
            CFStringGetSystemEncoding(),
        ) == 0
        {
            return None;
        }
        let len = buf.iter().position(|&b| b == 0).unwrap_or(size);
        Some(String::from_utf8_lossy(&buf[..len]).into_owned())
    }
}

/// `FF_TEST()`: the SDL features of the ForceFeedback effects supported.
pub(crate) fn supported_effects_features(supported_effects: u32) -> HapticFeatures {
    let mut supported = HapticFeatures::NONE;
    for (ff, s) in [
        (FFCAP_ET_CONSTANTFORCE, HapticFeatures::CONSTANT),
        (FFCAP_ET_RAMPFORCE, HapticFeatures::RAMP),
        (FFCAP_ET_SQUARE, HapticFeatures::SQUARE),
        (FFCAP_ET_SINE, HapticFeatures::SINE),
        (FFCAP_ET_TRIANGLE, HapticFeatures::TRIANGLE),
        (FFCAP_ET_SAWTOOTHUP, HapticFeatures::SAWTOOTHUP),
        (FFCAP_ET_SAWTOOTHDOWN, HapticFeatures::SAWTOOTHDOWN),
        (FFCAP_ET_SPRING, HapticFeatures::SPRING),
        (FFCAP_ET_DAMPER, HapticFeatures::DAMPER),
        (FFCAP_ET_INERTIA, HapticFeatures::INERTIA),
        (FFCAP_ET_FRICTION, HapticFeatures::FRICTION),
        (FFCAP_ET_CUSTOMFORCE, HapticFeatures::CUSTOM),
    ] {
        if supported_effects & ff != 0 {
            supported |= s;
        }
    }
    supported
}

/// Gets supported features. Translation of `GetSupportedFeatures()`.
fn get_supported_features(haptic: &mut HapticData, hwdata: &mut HapticHwData) -> Result<()> {
    let device = hwdata.device;
    let mut features = FFCAPABILITIES::default();

    // SAFETY: an open force feedback device; the outputs are as large as
    // said.
    unsafe {
        let ret = FFDeviceGetForceFeedbackCapabilities(device, &mut features);
        if ret != FF_OK {
            return Err(Error::new(
                "Haptic: Unable to get device's supported features.",
            ));
        }

        // Get maximum effects.
        let neffects = features.storage_capacity as usize;
        haptic.nplaying = features.playback_capacity as i32;
        haptic.effects = (0..neffects).map(|_| HapticEffectSlot::default()).collect();

        // Test for effects.
        let mut supported = supported_effects_features(features.supported_effects);

        // Check if supports gain.
        let mut val = 0u32;
        let ret = FFDeviceGetForceFeedbackProperty(
            device,
            FFPROP_FFGAIN,
            (&mut val as *mut u32).cast(),
            size_of::<u32>() as u64,
        );
        if ret == FF_OK {
            supported |= HapticFeatures::GAIN;
        } else if ret != FFERR_UNSUPPORTED {
            return Err(Error::new(format!(
                "Haptic: Unable to get if device supports gain: {}.",
                ff_str_error(ret)
            )));
        }

        // Checks if supports autocenter.
        let ret = FFDeviceGetForceFeedbackProperty(
            device,
            FFPROP_AUTOCENTER,
            (&mut val as *mut u32).cast(),
            size_of::<u32>() as u64,
        );
        if ret == FF_OK {
            supported |= HapticFeatures::AUTOCENTER;
        } else if ret != FFERR_UNSUPPORTED {
            return Err(Error::new(format!(
                "Haptic: Unable to get if device supports autocenter: {}.",
                ff_str_error(ret)
            )));
        }

        // Check for axes, we have an artificial limit on axes
        haptic.naxes = features.num_ff_axes.min(3) as i32;
        // Actually store the axes we want to use
        let n = haptic.naxes as usize;
        hwdata.axes[..n].copy_from_slice(&features.ff_axes[..n]);

        // Always supported features.
        supported |= HapticFeatures::STATUS | HapticFeatures::PAUSE;

        haptic.supported = supported;
    }
    Ok(())
}

/// Opens the haptic device from the file descriptor. Translation of
/// `SDL_SYS_HapticOpenFromService()`.
fn haptic_open_from_service(haptic: &mut HapticData, service: IoService) -> Result<()> {
    // Allocate the hwdata
    let mut hwdata = HapticHwData {
        device: std::ptr::null_mut(),
        axes: [0; 3],
    };

    // Open the device
    // SAFETY: a valid service; the device is released on failure or close.
    let ret = unsafe { FFCreateDevice(service, &mut hwdata.device) };
    if ret != FF_OK {
        return Err(Error::new(format!(
            "Haptic: Unable to create device from service: {}.",
            ff_str_error(ret)
        )));
    }

    let result = (|| {
        // Get supported features.
        get_supported_features(haptic, &mut hwdata)?;

        // Reset and then enable actuators.
        // SAFETY: the device created above.
        let ret = unsafe { FFDeviceSendForceFeedbackCommand(hwdata.device, FFSFFC_RESET) };
        if ret != FF_OK {
            return Err(Error::new(format!(
                "Haptic: Unable to reset device: {}.",
                ff_str_error(ret)
            )));
        }
        // SAFETY: as above.
        let ret = unsafe { FFDeviceSendForceFeedbackCommand(hwdata.device, FFSFFC_SETACTUATORSON) };
        if ret != FF_OK {
            return Err(Error::new(format!(
                "Haptic: Unable to enable actuators: {}.",
                ff_str_error(ret)
            )));
        }

        // Allocate effects memory.
        // (done by get_supported_features(), the slots cleared)
        Ok(())
    })();

    if let Err(e) = result {
        // Error handling
        // SAFETY: the device created above, released once.
        unsafe { FFReleaseDevice(hwdata.device) };
        haptic.effects.clear();
        return Err(e);
    }

    haptic.hwdata = Some(Box::new(hwdata));
    Ok(())
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

/// Converts an SDL trigger button to an FFEFFECT trigger button.
/// Translation of `FFGetTriggerButton()`.
pub(crate) fn ff_get_trigger_button(button: u16) -> u32 {
    let mut dw_trigger_button = FFEB_NOTRIGGER;

    if button != 0 {
        dw_trigger_button = ffjofs_button(u32::from(button) - 1);
    }

    dw_trigger_button
}

/// Sets the direction: `dwFlags` and `rglDirection` for `naxes` axes.
/// Translation of `SDL_SYS_SetDirection()` (whose "Unknown direction type"
/// can't happen here).
pub(crate) fn set_direction(
    dw_flags: &mut u32,
    dir: &HapticDirection,
    naxes: u32,
) -> Option<Vec<i32>> {
    // Handle no axes a part.
    if naxes == 0 {
        *dw_flags |= FFEFF_SPHERICAL; // Set as default.
        return None;
    }

    // Has axes.
    let mut rgl_dir = vec![0i32; naxes as usize];

    match dir.kind {
        HapticDirectionType::Polar => {
            *dw_flags |= FFEFF_POLAR;
            rgl_dir[0] = dir.dir[0];
        }
        HapticDirectionType::Cartesian | HapticDirectionType::Spherical => {
            *dw_flags |= if dir.kind == HapticDirectionType::Cartesian {
                FFEFF_CARTESIAN
            } else {
                FFEFF_SPHERICAL
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
            *dw_flags |= FFEFF_CARTESIAN;
            rgl_dir[0] = 0;
        }
    }
    Some(rgl_dir)
}

/// Clamps and converts. Translation of `CCONVERT()`.
pub(crate) fn cconvert(x: u16) -> u32 {
    if x > 0x7FFF {
        10000
    } else {
        (u32::from(x) * 10000) / 0x7FFF
    }
}

/// Just converts. Translation of `CONVERT()`.
pub(crate) fn convert(x: i32) -> i32 {
    (x * 10000) / 0x7FFF
}

/// The envelope of an effect: none when it has no attack and no fade.
fn envelope(
    attack_length: u16,
    attack_level: u16,
    fade_length: u16,
    fade_level: u16,
) -> Option<FFENVELOPE> {
    if attack_length == 0 && fade_length == 0 {
        return None;
    }
    Some(FFENVELOPE {
        dw_size: size_of::<FFENVELOPE>() as u32, // Always should be this.
        dw_attack_level: cconvert(attack_level),
        dw_attack_time: u32::from(attack_length) * 1000,
        dw_fade_level: cconvert(fade_level),
        dw_fade_time: u32::from(fade_length) * 1000,
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

/// Creates the FFEFFECT from a SDL_HapticEffect, for a device with `naxes`
/// axes, `hw_axes` being their ForceFeedback object offsets. Translation of
/// `SDL_SYS_ToFFEFFECT()`.
pub(crate) fn to_ffeffect(naxes: i32, hw_axes: &[u8; 3], src: &HapticEffect) -> Result<FfEffect> {
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
    let mut dw_flags = FFEFF_OBJECTOFFSETS; // Seems obligatory.

    // Axes.
    let c_axes = if generics.direction.kind == HapticDirectionType::SteeringAxis {
        1
    } else {
        naxes as u32
    };
    // Note (upstream): with more than 3 axes (the haptic axes hint can set
    // that), the axes after the third are left uninitialized, and condition
    // effects read past their 3-entry arrays; here those read as 0.
    let rgdw_axes: Vec<u32> = (0..c_axes as usize)
        .map(|i| hw_axes.get(i).copied().map_or(0, u32::from)) // Always at least one axis.
        .collect();

    // Generics
    // FIXME (upstream): `length * 1000` wraps (a 32-bit DWORD), so long and
    // infinite lengths don't become FF_INFINITE; and the trigger repeat
    // interval is given in milliseconds where ForceFeedback wants
    // microseconds.
    let dw_duration = generics.length.wrapping_mul(1000); // In microseconds.
    let dw_trigger_button = ff_get_trigger_button(generics.button);
    let dw_trigger_repeat_interval = u32::from(generics.interval);
    let dw_start_delay = u32::from(generics.delay) * 1000; // In microseconds.

    // Direction.
    let rgl_direction = set_direction(&mut dw_flags, generics.direction, c_axes);

    // The big type handling switch, even bigger then Linux's version.
    let (params, lp_envelope) = match src {
        HapticEffect::Constant(hap_constant) => (
            // Specifics
            TypeSpecificParams::Constant(FFCONSTANTFORCE {
                l_magnitude: convert(i32::from(hap_constant.level)),
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
            TypeSpecificParams::Periodic(FFPERIODIC {
                dw_magnitude: convert(i32::from(hap_periodic.magnitude).abs()) as u32,
                l_offset: convert(i32::from(hap_periodic.offset)),
                dw_phase: (u32::from(hap_periodic.phase)
                    + if hap_periodic.magnitude < 0 { 18000 } else { 0 })
                    % 36000,
                dw_period: u32::from(hap_periodic.period) * 1000,
            }),
            envelope(
                hap_periodic.attack_length,
                hap_periodic.attack_level,
                hap_periodic.fade_length,
                hap_periodic.fade_level,
            ),
        ),

        HapticEffect::Condition(hap_condition) => {
            // Specifics
            // (with no axes, there are no conditions and a null pointer)
            let at = |values: &[i16; 3], i: usize| values.get(i).copied().map_or(0, i32::from);
            let atu = |values: &[u16; 3], i: usize| values.get(i).copied().unwrap_or(0);
            let condition = (0..c_axes as usize)
                .map(|i| FFCONDITION {
                    l_offset: convert(at(&hap_condition.center, i)),
                    l_positive_coefficient: convert(at(&hap_condition.right_coeff, i)),
                    l_negative_coefficient: convert(at(&hap_condition.left_coeff, i)),
                    dw_positive_saturation: cconvert(atu(&hap_condition.right_sat, i) / 2),
                    dw_negative_saturation: cconvert(atu(&hap_condition.left_sat, i) / 2),
                    l_dead_band: cconvert(atu(&hap_condition.deadband, i) / 2) as i32,
                })
                .collect();
            // Envelope - Not actually supported by most CONDITION implementations.
            (TypeSpecificParams::Condition(condition), None)
        }

        HapticEffect::Ramp(hap_ramp) => (
            // Specifics
            TypeSpecificParams::Ramp(FFRAMPFORCE {
                l_start: convert(i32::from(hap_ramp.start)),
                l_end: convert(i32::from(hap_ramp.end)),
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
            // Note (upstream): the samples are read for channels * samples
            // entries whatever the size of the data array; missing ones read
            // as 0 here.
            let count = usize::from(hap_custom.samples) * usize::from(hap_custom.channels);
            let rgl_force_data = (0..count)
                .map(|i| cconvert(hap_custom.data.get(i).copied().unwrap_or(0)) as i32)
                .collect();
            (
                TypeSpecificParams::Custom {
                    c_channels: u32::from(hap_custom.channels),
                    dw_sample_period: u32::from(hap_custom.period) * 1000,
                    c_samples: u32::from(hap_custom.samples),
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

    Ok(FfEffect {
        dw_flags,
        dw_duration,
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
pub(crate) fn haptic_effect_type(effect: &HapticEffect) -> Option<CFUUIDRef> {
    let feature = effect.effect_type();
    Some(match feature {
        f if f == HapticFeatures::CONSTANT => k_ff_effect_type_constant_force_id(),
        f if f == HapticFeatures::RAMP => k_ff_effect_type_ramp_force_id(),
        f if f == HapticFeatures::SQUARE => k_ff_effect_type_square_id(),
        f if f == HapticFeatures::SINE => k_ff_effect_type_sine_id(),
        f if f == HapticFeatures::TRIANGLE => k_ff_effect_type_triangle_id(),
        f if f == HapticFeatures::SAWTOOTHUP => k_ff_effect_type_sawtooth_up_id(),
        f if f == HapticFeatures::SAWTOOTHDOWN => k_ff_effect_type_sawtooth_down_id(),
        f if f == HapticFeatures::SPRING => k_ff_effect_type_spring_id(),
        f if f == HapticFeatures::DAMPER => k_ff_effect_type_damper_id(),
        f if f == HapticFeatures::INERTIA => k_ff_effect_type_inertia_id(),
        f if f == HapticFeatures::FRICTION => k_ff_effect_type_friction_id(),
        f if f == HapticFeatures::CUSTOM => k_ff_effect_type_custom_force_id(),
        _ => return None,
    })
}

/// The flags of an effect update: all of them (upstream doesn't compare
/// the old and new effects). Translation of the flags of
/// `SDL_SYS_HapticUpdateEffect()`.
pub(crate) const UPDATE_FLAGS: u32 = FFEP_DIRECTION
    | FFEP_DURATION
    | FFEP_ENVELOPE
    | FFEP_STARTDELAY
    | FFEP_TRIGGERBUTTON
    | FFEP_TRIGGERREPEATINTERVAL
    | FFEP_TYPESPECIFICPARAMS;

/// Send a ForceFeedback command to an open device.
fn send_command(haptic: &HapticData, command: u32, what: &str) -> Result<()> {
    let device = hwdata(haptic)?.device;
    // SAFETY: an open device.
    let ret = unsafe { FFDeviceSendForceFeedbackCommand(device, command) };
    if ret != FF_OK {
        return Err(Error::new(format!(
            "Haptic: {what}: {}.",
            ff_str_error(ret)
        )));
    }
    Ok(())
}

/// Translation of the Darwin `SDL_SYS_Haptic*` functions.
pub(crate) struct DarwinHapticDriver;

pub(crate) static DARWIN_HAPTIC_DRIVER: DarwinHapticDriver = DarwinHapticDriver;

impl HapticDriver for DarwinHapticDriver {
    /// Initializes the haptic subsystem. Translation of
    /// `SDL_SYS_HapticInit()`.
    fn init(&self) -> Result<()> {
        haptic_init()
    }

    /// Translation of `SDL_SYS_NumHaptics()` (-1 before the subsystem is
    /// initialized, which is no devices here).
    fn count(&self) -> usize {
        lock_list().items.as_ref().map_or(0, Vec::len)
    }

    /// Translation of `SDL_SYS_HapticInstanceID()` (and `HapticByDevIndex()`).
    fn instance_id(&self, index: usize) -> HapticID {
        lock_list()
            .items
            .as_ref()
            .and_then(|l| l.get(index))
            .map_or(0, |item| item.instance_id)
    }

    /// Return the name of a haptic device, does not need to be opened.
    /// Translation of `SDL_SYS_HapticName()`.
    fn name(&self, index: usize) -> Option<String> {
        lock_list()
            .items
            .as_ref()
            .and_then(|l| l.get(index))
            .map(|item| item.name.clone())
    }

    /// Opens a haptic device for usage. Translation of `SDL_SYS_HapticOpen()`
    /// (and `HapticByInstanceID()`).
    fn open(&self, haptic: &mut HapticData) -> Result<()> {
        let dev = lock_list()
            .items
            .as_ref()
            .and_then(|l| l.iter().find(|item| item.instance_id == haptic.instance_id))
            .map(|item| item.dev)
            // (upstream dereferences the item without checking it)
            .ok_or_else(|| Error::new(format!("Haptic device {} not found", haptic.instance_id)))?;

        haptic_open_from_service(haptic, dev)
    }

    /// Opens a haptic device from first mouse it finds for usage.
    /// Translation of `SDL_SYS_HapticMouse()`.
    fn mouse(&self) -> Option<usize> {
        // Note (upstream): without a mouse this returns device index 0,
        // which SDL_OpenHapticFromMouse() passes to SDL_OpenHaptic() as an
        // instance ID, never a valid one; the open fails, as `None` makes
        // it fail here.
        lock_list().items.as_ref()?.iter().position(|item| {
            item.usage_page == i64::from(K_HID_PAGE_GENERIC_DESKTOP)
                && item.usage == i64::from(K_HID_USAGE_GD_MOUSE)
        })
    }

    /// Checks to see if a joystick has haptic features.
    /// Translation of `SDL_SYS_JoystickIsHaptic()`.
    fn joystick_is_haptic(&self, joystick: &Joystick) -> bool {
        // (a joystick of another driver, or whose device is gone, has none)
        crate::joystick::darwin::joystick_ffservice(joystick.id()).is_some_and(|s| s != 0)
    }

    /// Opens a SDL_Haptic from a SDL_Joystick.
    /// Translation of `SDL_SYS_HapticOpenFromJoystick()`.
    fn open_from_joystick(&self, haptic: &mut HapticData, joystick: &Joystick) -> Result<()> {
        let Some(ffservice) = crate::joystick::darwin::joystick_ffservice(joystick.id()) else {
            return Err(Error::new("Haptic: joystick isn't an IOKit joystick"));
        };
        if let Some(instance_id) = lock_list().items.as_ref().and_then(|l| {
            l.iter()
                // SAFETY: valid services.
                .find(|item| unsafe { IOObjectIsEqualTo(item.dev, ffservice) } != 0)
                .map(|item| item.instance_id)
        }) {
            haptic.instance_id = instance_id;
        }

        if let Ok(Some(name)) = joystick.name() {
            haptic.name = Some(name);
        }

        haptic_open_from_service(haptic, ffservice)
    }

    /// Checks to see if the haptic device and joystick are in reality the
    /// same. Translation of `SDL_SYS_JoystickSameHaptic()`.
    fn joystick_same_haptic(&self, haptic: &HapticData, joystick: &Joystick) -> bool {
        let Some(ffservice) = crate::joystick::darwin::joystick_ffservice(joystick.id()) else {
            return false;
        };
        let Ok(hwdata) = hwdata(haptic) else {
            return false;
        };
        // FIXME (upstream): this compares the ForceFeedback device
        // reference, a pointer cut to an io_object_t, with the joystick's
        // service, so an open haptic device is never found again by its
        // joystick.
        // SAFETY: IOObjectIsEqualTo() compares the two object names.
        unsafe { IOObjectIsEqualTo(hwdata.device as usize as IoObject, ffservice) != 0 }
    }

    /// Closes the haptic device. Translation of `SDL_SYS_HapticClose()`.
    fn close(&self, haptic: &mut HapticData) {
        if let Some(hwdata) = haptic.hwdata.take() {
            // Free Effects.
            haptic.effects.clear();

            // Clean up
            if let Ok(hwdata) = hwdata.downcast::<HapticHwData>() {
                // SAFETY: the device created when opening, released once.
                unsafe { FFReleaseDevice(hwdata.device) };
            }

            // Free
        }
    }

    /// Clean up after system specific haptic stuff.
    /// Translation of `SDL_SYS_HapticQuit()`.
    fn quit(&self) {
        let items = lock_list().items.take().unwrap_or_default();
        for item in items {
            /* Opened and not closed haptics are leaked, this is on purpose.
             * Close your haptic devices after usage. */

            // Free the io_service_t
            // SAFETY: the reference mac_haptic_maybe_add_device() took.
            unsafe { IOObjectRelease(item.dev) };
        }
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
        let (device, axes) = {
            let hw = hwdata(haptic)?;
            (hw.device, hw.axes)
        };

        // Get the type.
        let Some(effect_type) = haptic_effect_type(base) else {
            return Err(Error::new("Haptic: Unknown effect type."));
        };

        // Get the effect.
        let mut effect_data = to_ffeffect(naxes, &axes, base)?;

        // Create the actual effect.
        let mut reference: FFEffectObjectReference = std::ptr::null_mut();
        let ret = effect_data.with_ffeffect(|ffeffect| {
            // SAFETY: an open device and an effect pointing into effect_data.
            unsafe { FFDeviceCreateEffect(device, effect_type, ffeffect, &mut reference) }
        });
        if ret != FF_OK {
            return Err(Error::new(format!(
                "Haptic: Unable to create effect: {}.",
                ff_str_error(ret)
            )));
        }

        let slot = haptic
            .effects
            .get_mut(effect)
            .ok_or_else(|| Error::invalid_param("effect"))?;
        slot.hweffect = Some(Box::new(HapticHwEffect {
            reference,
            effect: effect_data,
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
        let axes = hwdata(haptic)?.axes;

        // Get the effect.
        let mut temp = to_ffeffect(naxes, &axes, data)?;

        /* Set the flags.  Might be worthwhile to diff temp with loaded effect and
         *  only change those parameters. */
        let flags = UPDATE_FLAGS;

        // Create the actual effect.
        let hw = hweffect(haptic, effect)?;
        let reference = hw.reference;
        // SAFETY: the effect object and a definition pointing into temp.
        let ret = temp
            .with_ffeffect(|ffeffect| unsafe { FFEffectSetParameters(reference, ffeffect, flags) });
        if ret != FF_OK {
            return Err(Error::new(format!(
                "Haptic: Unable to update effect: {}.",
                ff_str_error(ret)
            )));
        }

        // Copy it over.
        hw.effect = temp;

        Ok(())
    }

    /// Runs an effect. Translation of `SDL_SYS_HapticRunEffect()`.
    fn run_effect(&self, haptic: &mut HapticData, effect: usize, iterations: u32) -> Result<()> {
        // Check if it's infinite.
        let iter = if iterations == HAPTIC_INFINITY {
            FF_INFINITE
        } else {
            iterations
        };

        // Run the effect.
        let reference = hweffect(haptic, effect)?.reference;
        // SAFETY: the effect object.
        let ret = unsafe { FFEffectStart(reference, iter, 0) };
        if ret != FF_OK {
            return Err(Error::new(format!(
                "Haptic: Unable to run the effect: {}.",
                ff_str_error(ret)
            )));
        }

        Ok(())
    }

    /// Stops an effect. Translation of `SDL_SYS_HapticStopEffect()`.
    fn stop_effect(&self, haptic: &mut HapticData, effect: usize) -> Result<()> {
        let reference = hweffect(haptic, effect)?.reference;
        // SAFETY: the effect object.
        let ret = unsafe { FFEffectStop(reference) };
        if ret != FF_OK {
            return Err(Error::new(format!(
                "Haptic: Unable to stop the effect: {}.",
                ff_str_error(ret)
            )));
        }

        Ok(())
    }

    /// Frees the effect. Translation of `SDL_SYS_HapticDestroyEffect()`.
    fn destroy_effect(&self, haptic: &mut HapticData, effect: usize) {
        let Ok(device) = hwdata(haptic).map(|h| h.device) else {
            return;
        };
        let Some(slot) = haptic.effects.get_mut(effect) else {
            return;
        };
        if let Some(hweffect) = slot.hweffect.take() {
            if let Ok(hweffect) = hweffect.downcast::<HapticHwEffect>() {
                // SAFETY: the effect object of the device, released once.
                let ret = unsafe { FFDeviceReleaseEffect(device, hweffect.reference) };
                if ret != FF_OK {
                    // ("Haptic: Error removing the effect from the device: %s.",
                    // which nobody reads)
                }
                // (the effect's data is freed with it)
            }
        }
    }

    /// Gets the status of a haptic effect. Translation of
    /// `SDL_SYS_HapticGetEffectStatus()`.
    fn effect_status(&self, haptic: &mut HapticData, effect: usize) -> Result<bool> {
        let reference = hweffect(haptic, effect)?.reference;
        let mut status = 0u32;
        // SAFETY: the effect object; a writable status.
        let ret = unsafe { FFEffectGetEffectStatus(reference, &mut status) };
        if ret != FF_OK {
            return Err(Error::new(format!(
                "Haptic: Unable to get effect status: {}.",
                ff_str_error(ret)
            )));
        }

        if status == 0 {
            return Ok(false);
        }
        Ok(true) // Assume it's playing or emulated.
    }

    /// Sets the gain. Translation of `SDL_SYS_HapticSetGain()`.
    fn set_gain(&self, haptic: &mut HapticData, gain: i32) -> Result<()> {
        let device = hwdata(haptic)?.device;
        let mut val = (gain * 100) as u32; // macOS uses 0 to 10,000
                                           // SAFETY: an open device; the value is the property's type.
        let ret = unsafe {
            FFDeviceSetForceFeedbackProperty(device, FFPROP_FFGAIN, (&mut val as *mut u32).cast())
        };
        if ret != FF_OK {
            return Err(Error::new(format!(
                "Haptic: Error setting gain: {}.",
                ff_str_error(ret)
            )));
        }

        Ok(())
    }

    /// Sets the autocentering. Translation of `SDL_SYS_HapticSetAutocenter()`.
    fn set_autocenter(&self, haptic: &mut HapticData, autocenter: i32) -> Result<()> {
        let device = hwdata(haptic)?.device;
        // macOS only has 0 (off) and 1 (on)
        let mut val: u32 = if autocenter == 0 { 0 } else { 1 };

        // SAFETY: as in set_gain().
        let ret = unsafe {
            FFDeviceSetForceFeedbackProperty(
                device,
                FFPROP_AUTOCENTER,
                (&mut val as *mut u32).cast(),
            )
        };
        if ret != FF_OK {
            return Err(Error::new(format!(
                "Haptic: Error setting autocenter: {}.",
                ff_str_error(ret)
            )));
        }

        Ok(())
    }

    /// Pauses the device. Translation of `SDL_SYS_HapticPause()`.
    fn pause(&self, haptic: &mut HapticData) -> Result<()> {
        send_command(haptic, FFSFFC_PAUSE, "Error pausing device")
    }

    /// Unpauses the device. Translation of `SDL_SYS_HapticResume()`.
    fn resume(&self, haptic: &mut HapticData) -> Result<()> {
        send_command(haptic, FFSFFC_CONTINUE, "Error resuming device")
    }

    /// Stops all currently playing effects. Translation of
    /// `SDL_SYS_HapticStopAll()`.
    fn stop_all(&self, haptic: &mut HapticData) -> Result<()> {
        send_command(haptic, FFSFFC_STOPALL, "Error stopping device")
    }
}

/// Whether the device list exists (`numhaptics != -1`), for the tests.
#[cfg(test)]
fn initialized() -> bool {
    lock_list().items.is_some()
}
