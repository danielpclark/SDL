// Rust translation of src/joystick/windows/SDL_dinputjoystick.c and
// SDL_dinputjoystick_c.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! DirectInput 8 joysticks for the Windows joystick driver: the HID game
//! controllers DirectInput enumerates that XInput, RawInput and the
//! drivers before this one don't handle, with their axes, buttons and POV
//! hats read through the `DIJOYSTATE2` data format (buffered, or polled
//! when the device can't buffer), and a sine force feedback effect for
//! rumble.
//!
//! DirectInput is created through COM, which loads `dinput8.dll` at run
//! time; with the `SDL_JOYSTICK_DIRECTINPUT` hint off it isn't used at all.

use std::sync::{Mutex, MutexGuard};

use windows_sys::core::{GUID, HRESULT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

use super::super::gamepad::GamepadType;
use super::super::usb_ids::{USB_PRODUCT_STEAM_VIRTUAL_GAMEPAD, USB_VENDOR_VALVE};
use super::super::{
    create_joystick_guid, create_joystick_name, gamepad_type_from_vidpid,
    joystick_handled_by_another_driver, should_ignore_joystick, JoystickData,
    HARDWARE_BUS_BLUETOOTH, HARDWARE_BUS_USB, HAT_CENTERED, HAT_DOWN, HAT_LEFT, HAT_RIGHT, HAT_UP,
    JOYSTICK_AXIS_MAX, JOYSTICK_AXIS_MIN, MAX_RUMBLE_DURATION_MS, PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN,
};
use super::xinput::Input;
use super::{add_joystick_device, JoyStickDeviceData};
use crate::core::windows::directx::*;
use crate::core::windows::{
    co_initialize, co_uninitialize, has_broken_ezfrd64_dll, helper_window, is_equal_guid,
    wide_to_utf8,
};
use crate::error::{Error, Result};
use crate::hints;

/// Buffer up to 128 input messages.
const INPUT_QSIZE: usize = 128;

/// Each joystick can have up to 256 inputs. Translation of `MAX_INPUTS`.
const MAX_INPUTS: usize = 256;

/// Translation of `CONVERT_MAGNITUDE()`.
pub(super) fn convert_magnitude(x: i16) -> u32 {
    ((x as i32 * 10000) / 0x7FFF) as u32
}

/// The local variables, `coinitialized` and `dinput`.
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

/// The DirectInput object, if DirectInput is in use (a new reference, so
/// the lock isn't held while it calls back).
fn dinput() -> Option<DirectInput> {
    lock().dinput.clone()
}

/// Whether DirectInput is in use (for the tests).
#[cfg(test)]
pub(super) fn dinput_in_use() -> bool {
    lock().dinput.is_some()
}

/// The `DIOBJECTDATAFORMAT` array of [`C_DF_DIJOYSTICK2`] (raw pointers to
/// the static GUIDs, so it is declared `Sync` by hand).
pub(crate) struct ObjectDataFormats([DIOBJECTDATAFORMAT; 164]);

// SAFETY: the pointers point to immutable statics; nothing writes them.
unsafe impl Sync for ObjectDataFormats {}

/// A `DIDATAFORMAT` pointing to static object formats.
pub(crate) struct DataFormat(pub(crate) DIDATAFORMAT);

// SAFETY: as for ObjectDataFormats.
unsafe impl Sync for DataFormat {}

/// Taken from Wine - Thanks! Translation of `dfDIJoystick2`.
static DF_DIJOYSTICK2: ObjectDataFormats = ObjectDataFormats({
    const AXIS: u32 = DIDFT_OPTIONAL | DIDFT_AXIS | DIDFT_ANYINSTANCE;
    const POV: u32 = DIDFT_OPTIONAL | DIDFT_POV | DIDFT_ANYINSTANCE;
    const BUTTON: u32 = DIDFT_OPTIONAL | DIDFT_BUTTON | DIDFT_ANYINSTANCE;
    const fn object(
        pguid: *const GUID,
        dw_ofs: u32,
        dw_type: u32,
        dw_flags: u32,
    ) -> DIOBJECTDATAFORMAT {
        DIOBJECTDATAFORMAT {
            pguid,
            dwOfs: dw_ofs,
            dwType: dw_type,
            dwFlags: dw_flags,
        }
    }
    let axes: [*const GUID; 6] = [
        &GUID_XAXIS,
        &GUID_YAXIS,
        &GUID_ZAXIS,
        &GUID_RXAXIS,
        &GUID_RYAXIS,
        &GUID_RZAXIS,
    ];
    let mut df = [object(std::ptr::null(), 0, 0, 0); 164];
    let mut n = 0;
    // The positions, velocities, accelerations and forces of the six axes
    // and two sliders; the position block has the POVs and buttons after it.
    let aspects = [
        (DIDOI_ASPECTPOSITION, DIJOFS_X),
        (DIDOI_ASPECTVELOCITY, 176), // FIELD_OFFSET(DIJOYSTATE2, lVX)
        (DIDOI_ASPECTACCEL, 208),    // FIELD_OFFSET(DIJOYSTATE2, lAX)
        (DIDOI_ASPECTFORCE, 240),    // FIELD_OFFSET(DIJOYSTATE2, lFX)
    ];
    let mut a = 0;
    while a < aspects.len() {
        let (aspect, base) = aspects[a];
        let mut i = 0;
        while i < 6 {
            df[n] = object(axes[i], base + 4 * i as u32, AXIS, aspect);
            n += 1;
            i += 1;
        }
        // note: dwOfs value matches Windows (DIJOFS_SLIDER() for every aspect)
        df[n] = object(&GUID_SLIDER, dijofs_slider(0), AXIS, aspect);
        df[n + 1] = object(&GUID_SLIDER, dijofs_slider(1), AXIS, aspect);
        n += 2;
        if a == 0 {
            let mut pov = 0;
            while pov < 4 {
                df[n] = object(&GUID_POV, dijofs_pov(pov), POV, 0);
                n += 1;
                pov += 1;
            }
            let mut button = 0;
            while button < 128 {
                df[n] = object(std::ptr::null(), dijofs_button(button), BUTTON, 0);
                n += 1;
                button += 1;
            }
        }
        a += 1;
    }
    assert!(n == 164);
    df
});

/// The `DIJOYSTATE2` data format. Translation of `SDL_c_dfDIJoystick2`.
pub(crate) static C_DF_DIJOYSTICK2: DataFormat = DataFormat(DIDATAFORMAT {
    dwSize: size_of::<DIDATAFORMAT>() as u32,
    dwObjSize: size_of::<DIOBJECTDATAFORMAT>() as u32,
    dwFlags: DIDF_ABSAXIS,
    dwDataSize: size_of::<DIJOYSTATE2>() as u32,
    dwNumObjs: 164,
    rgodf: &DF_DIJOYSTICK2.0 as *const [DIOBJECTDATAFORMAT; 164] as *const DIOBJECTDATAFORMAT,
});

/// Convert a DirectInput return code to a text message. Translation of
/// `SetDIerror()`.
fn set_di_error(function: &str, code: HRESULT) -> Error {
    Error::new(format!("{function}() DirectX error 0x{:08x}", code as u32))
}

/// Translation of `SDL_IsXInputDevice()`.
fn is_xinput_device(vendor_id: u16, product_id: u16, hid_path: &str) -> bool {
    // Some other backends will pick up XInput-compatible devices
    // (SDL_UsingGameInputForXInputControllers() is false without GameInput)
    if !super::xinput::xinput_enabled() && !super::rawinput::is_enabled() {
        return false;
    }

    // If device path contains "IG_" then its an XInput device
    // See: https://docs.microsoft.com/windows/win32/xinput/xinput-and-directinput
    if hid_path.contains("IG_") {
        return true;
    }

    let gamepad_type = gamepad_type_from_vidpid(vendor_id, product_id, None, false);
    gamepad_type == GamepadType::Xbox360
        || gamepad_type == GamepadType::XboxOne
        || (vendor_id == USB_VENDOR_VALVE && product_id == USB_PRODUCT_STEAM_VIRTUAL_GAMEPAD)
}

/// The manufacturer and product names the HIDAPI driver knows for a
/// device (`HIDAPI_GetDeviceManufacturerName()` and
/// `HIDAPI_GetDeviceProductName()`).
fn hidapi_device_names(vendor_id: u16, product_id: u16) -> (Option<String>, Option<String>) {
    use crate::joystick::hidapi;
    (
        hidapi::get_device_manufacturer_name(vendor_id, product_id),
        hidapi::get_device_product_name(vendor_id, product_id),
    )
}

/// The manufacturer and product names of a device. Translation of
/// `QueryDeviceName()`.
fn query_device_name(
    device: &InputDevice,
    vendor_id: u16,
    product_id: u16,
) -> Option<(Option<String>, Option<String>)> {
    let (manufacturer_string, product_string) = hidapi_device_names(vendor_id, product_id);
    if product_string.is_some() {
        return Some((manufacturer_string, product_string));
    }

    let wsz = device.get_property_string(DIPROP_PRODUCTNAME).ok()?;
    Some((None, Some(wide_to_utf8(&wsz))))
}

/// The device's path, upper case. Translation of `QueryDevicePath()`.
fn query_device_path(device: &InputDevice) -> Option<String> {
    let path = device.get_guid_and_path().ok()?;
    // Normalize path to upper case.
    Some(wide_to_utf8(&path).to_ascii_uppercase())
}

/// The vendor and product IDs. Translation of `QueryDeviceInfo()`.
fn query_device_info(device: &InputDevice) -> Option<(u16, u16)> {
    let vidpid = device.get_property_dword(DIPROP_VIDPID).ok()?;
    Some((vidpid as u16, (vidpid >> 16) as u16))
}

/// The rumble effect: its axes, direction and periodic parameters, which
/// the `DIEFFECT` [`RumbleEffect::dieffect`] makes points to. Translation
/// of the `DIEFFECT` `CreateRumbleEffectData()` allocates (and
/// `FreeRumbleEffectData()` frees, here on drop).
#[derive(Debug)]
pub(super) struct RumbleEffect {
    axes: [u32; 2],
    direction: [i32; 2],
    pub(super) periodic: DIPERIODIC,
}

impl RumbleEffect {
    /// Translation of `CreateRumbleEffectData()`.
    pub(super) fn new(magnitude: i16) -> Box<RumbleEffect> {
        Box::new(RumbleEffect {
            axes: [0; 2],
            direction: [0; 2],
            periodic: DIPERIODIC {
                dwMagnitude: convert_magnitude(magnitude),
                lOffset: 0,
                dwPhase: 0,
                dwPeriod: 1000000,
            },
        })
    }

    /// The effect, pointing into `self`.
    pub(super) fn dieffect(&mut self) -> DIEFFECT {
        DIEFFECT {
            dwSize: size_of::<DIEFFECT>() as u32,
            dwFlags: DIEFF_OBJECTOFFSETS | DIEFF_CARTESIAN,
            dwDuration: MAX_RUMBLE_DURATION_MS * 1000, // In microseconds.
            dwSamplePeriod: 0,
            dwGain: 10000,
            dwTriggerButton: DIEB_NOTRIGGER,
            dwTriggerRepeatInterval: 0,
            cAxes: 2,
            rgdwAxes: self.axes.as_mut_ptr(),
            rglDirection: self.direction.as_mut_ptr(),
            lpEnvelope: std::ptr::null_mut(),
            cbTypeSpecificParams: size_of::<DIPERIODIC>() as u32,
            lpvTypeSpecificParams: (&mut self.periodic as *mut DIPERIODIC).cast(),
            dwStartDelay: 0,
        }
    }
}

/// Translation of `SDL_DINPUT_JoystickInit()`.
pub(super) fn joystick_init() -> Result<()> {
    if !hints::get_bool(hints::JOYSTICK_DIRECTINPUT, true) {
        // In some environments, IDirectInput8_Initialize / _EnumDevices can take a minute even with no controllers.
        lock().dinput = None;
        return Ok(());
    }

    let result = co_initialize();
    if result < 0 {
        return Err(set_di_error("CoInitialize", result));
    }

    lock().coinitialized = true;

    let dinput = DirectInput::create().map_err(|hr| set_di_error("CoCreateInstance", hr))?;

    // Because we used CoCreateInstance, we need to Initialize it, first.
    // SAFETY: GetModuleHandleW(NULL) is the executable's handle.
    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    if instance.is_null() {
        return Err(crate::core::windows::set_error("GetModuleHandle() failed"));
    }
    dinput
        .initialize(instance, DIRECTINPUT_VERSION)
        .map_err(|hr| set_di_error("IDirectInput::Initialize", hr))?;
    lock().dinput = Some(dinput);
    Ok(())
}

/// The Steam virtual gamepad slot in a device path, or -1. Translation of
/// `GetSteamVirtualGamepadSlot()` (its `SDL_sscanf()`).
pub(super) fn get_steam_virtual_gamepad_slot(
    vendor_id: u16,
    product_id: u16,
    device_path: &str,
) -> i32 {
    if vendor_id == USB_VENDOR_VALVE && product_id == USB_PRODUCT_STEAM_VIRTUAL_GAMEPAD {
        use super::Scan::*;
        if let Some(slot) = super::scan_int(
            device_path,
            &[Lit("\\\\?\\HID#VID_28DE&PID_11FF&IG_0"), Int],
        ) {
            return slot;
        }
    }
    -1
}

/// Helper function for direct input, gets called for each connected
/// joystick: `context` is the device list of the previous detection,
/// `sys_joystick` the one being built. Translation of
/// `EnumJoystickDetectCallback()`.
fn enum_joystick_detect_callback(
    dinput: &DirectInput,
    device_instance: &DIDEVICEINSTANCEW,
    context: &mut Vec<JoyStickDeviceData>,
    sys_joystick: &mut Vec<JoyStickDeviceData>,
) {
    let version = 0;

    // We are only supporting HID devices.
    if device_instance.dwDevType & DIDEVTYPE_HID == 0 {
        return;
    }

    // (the device is released when it goes out of scope)
    let Ok(device) = dinput.create_device(&device_instance.guidInstance) else {
        return;
    };
    let Some(hid_path) = query_device_path(&device) else {
        return;
    };
    let Some((vendor, product)) = query_device_info(&device) else {
        return;
    };
    let Some((manufacturer_string, product_string)) = query_device_name(&device, vendor, product)
    else {
        return;
    };

    if is_xinput_device(vendor, product, &hid_path)
        || should_ignore_joystick(vendor, product, version, product_string.as_deref())
        || joystick_handled_by_another_driver(
            super::super::WINDOWS_DRIVER_INDEX,
            vendor,
            product,
            version,
            product_string.as_deref(),
        )
    {
        return;
    }

    // update GUIDs of joysticks with matching paths, in case they're not open yet
    if let Some(i) = context.iter().position(|j| j.path == hid_path) {
        let mut joystick = context.remove(i);
        // Update with new guid/etc, if it has changed
        joystick.dxdevice = *device_instance;
        sys_joystick.insert(0, joystick);
        return;
    }

    let Some(joystickname) = create_joystick_name(
        vendor,
        product,
        manufacturer_string.as_deref(),
        product_string.as_deref(),
    ) else {
        return;
    };

    let bus = if vendor != 0 && product != 0 {
        HARDWARE_BUS_USB
    } else {
        HARDWARE_BUS_BLUETOOTH
    };
    let new_joystick = JoyStickDeviceData {
        guid: create_joystick_guid(
            bus,
            vendor,
            product,
            version,
            manufacturer_string.as_deref(),
            product_string.as_deref(),
            0,
            0,
        ),
        joystickname,
        send_add_event: false,
        n_instance_id: 0,
        b_xinput_device: false,
        sub_type: 0,
        xinput_user_id: 0,
        dxdevice: *device_instance,
        steam_virtual_gamepad_slot: get_steam_virtual_gamepad_slot(vendor, product, &hid_path),
        // (SDL_strlcpy() into a MAX_PATH buffer)
        path: crate::stdlib::string::utf8_truncate(&hid_path, MAX_PATH - 1).to_string(),
    };

    add_joystick_device(sys_joystick, new_joystick);
}

/// Translation of `SDL_DINPUT_JoystickDetect()`.
pub(super) fn joystick_detect(
    context: &mut Vec<JoyStickDeviceData>,
    sys_joystick: &mut Vec<JoyStickDeviceData>,
) {
    let Some(dinput) = dinput() else {
        return;
    };

    let _ = dinput.enum_devices(DI8DEVCLASS_GAMECTRL, DIEDFL_ATTACHEDONLY, |instance| {
        enum_joystick_detect_callback(&dinput, instance, context, sys_joystick);
        true // get next device, please
    });
}

/// Whether DirectInput has a device with this vendor and product.
/// Translation of `SDL_DINPUT_JoystickPresent()` (and
/// `EnumJoystickPresentCallback()`).
pub(super) fn joystick_present(vendor_id: u16, product_id: u16, _version_number: u16) -> bool {
    let Some(dinput) = dinput() else {
        return false;
    };

    let mut present = false;
    let _ = dinput.enum_devices(DI8DEVCLASS_GAMECTRL, DIEDFL_ATTACHEDONLY, |instance| {
        // We are only supporting HID devices.
        if instance.dwDevType & DIDEVTYPE_HID == 0 {
            return true;
        }
        let Ok(device) = dinput.create_device(&instance.guidInstance) else {
            return true;
        };
        if query_device_info(&device) == Some((vendor_id, product_id)) {
            present = true;
            return false; // found it
        }
        true
    });
    present
}

/// Button, axis or hat. Translation of `Type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InputType {
    Button,
    Axis,
    Hat,
}

/// An input of a device. Translation of `input_t`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DeviceInput {
    /// DirectInput offset for this input type
    pub(super) ofs: u32,
    /// Button, axis or hat
    pub(super) kind: InputType,
    /// SDL input offset
    pub(super) num: u8,
}

/// The DirectInput half of `struct joystick_hwdata`.
pub(crate) struct DinputHwData {
    pub(crate) input_device: InputDevice,
    pub(crate) capabilities: DIDEVCAPS,
    buffered: bool,
    first_update: bool,
    inputs: Vec<DeviceInput>,
    num_sliders: u32,
    ff_initialized: bool,
    ffeffect: Option<Box<RumbleEffect>>,
    ffeffect_ref: Option<Effect>,
}

/// The counts `EnumDevObjectsCallback()` builds up, of `joystick` and its
/// hwdata.
#[derive(Default)]
pub(super) struct ObjectCounts {
    pub(super) nbuttons: usize,
    pub(super) nhats: usize,
    pub(super) naxes: usize,
    pub(super) inputs: Vec<DeviceInput>,
    pub(super) num_sliders: u32,
}

/// The offset an axis object maps to (`None`: not an axis we can grok).
fn axis_offset(guid_type: &GUID, num_sliders: &mut u32) -> Option<u32> {
    let ofs = if is_equal_guid(guid_type, &GUID_XAXIS) {
        DIJOFS_X
    } else if is_equal_guid(guid_type, &GUID_YAXIS) {
        DIJOFS_Y
    } else if is_equal_guid(guid_type, &GUID_ZAXIS) {
        DIJOFS_Z
    } else if is_equal_guid(guid_type, &GUID_RXAXIS) {
        DIJOFS_RX
    } else if is_equal_guid(guid_type, &GUID_RYAXIS) {
        DIJOFS_RY
    } else if is_equal_guid(guid_type, &GUID_RZAXIS) {
        DIJOFS_RZ
    } else if is_equal_guid(guid_type, &GUID_SLIDER) {
        // FIXME (upstream): a third slider gets DIJOFS_SLIDER(2), the
        // offset of the first POV, which DIJOYSTATE2 has no slider at: it
        // never reports a value.
        let ofs = dijofs_slider(*num_sliders);
        *num_sliders += 1;
        ofs
    } else {
        return None; // not an axis we can grok
    };
    Some(ofs)
}

/// Add an object of the device to `counts`; `configure_axis` sets up an
/// axis (its range and dead zone), failing if it can't be used. Returns
/// whether to go on (`DIENUM_CONTINUE`). Translation of
/// `EnumDevObjectsCallback()`.
pub(super) fn enum_dev_objects_callback(
    device_object: &DIDEVICEOBJECTINSTANCEW,
    counts: &mut ObjectCounts,
    configure_axis: impl FnOnce(u32) -> bool,
) -> bool {
    let input = if device_object.dwType & DIDFT_BUTTON != 0 {
        let num = counts.nbuttons as u8;
        counts.nbuttons += 1;
        DeviceInput {
            ofs: dijofs_button(num as u32),
            kind: InputType::Button,
            num,
        }
    } else if device_object.dwType & DIDFT_POV != 0 {
        // DIJOYSTATE2.rgdwPOV only has room for 4 POVs, ignore any beyond that.
        if counts.nhats >= 4 {
            return true;
        }
        let num = counts.nhats as u8;
        counts.nhats += 1;
        DeviceInput {
            ofs: dijofs_pov(num as u32),
            kind: InputType::Hat,
            num,
        }
    } else if device_object.dwType & DIDFT_AXIS != 0 {
        let num = counts.naxes as u8;
        let Some(ofs) = axis_offset(&device_object.guidType, &mut counts.num_sliders) else {
            return true; // not an axis we can grok
        };
        if !configure_axis(device_object.dwType) {
            return true; // don't use this axis
        }
        counts.naxes += 1;
        DeviceInput {
            ofs,
            kind: InputType::Axis,
            num,
        }
    } else {
        // not supported at this time
        return true;
    };

    counts.inputs.push(input);

    if counts.inputs.len() == MAX_INPUTS {
        return false; // too many
    }

    true
}

/// Set up an axis object: its range and a zero dead zone. Translation of
/// that part of `EnumDevObjectsCallback()`.
fn configure_axis(device: &InputDevice, dw_type: u32) -> bool {
    if device
        .set_property_range(
            DIPROP_RANGE,
            dw_type,
            DIPH_BYID,
            (JOYSTICK_AXIS_MIN as i32, JOYSTICK_AXIS_MAX as i32),
        )
        .is_err()
    {
        return false; // don't use this axis
    }

    // Set dead zone to 0.
    device
        .set_property_dword(DIPROP_DEADZONE, dw_type, DIPH_BYID, 0)
        .is_ok()
}

/// Sort the input objects by their data offset into the DInput struct
/// (this gives a reasonable ordering for the inputs), and recalculate the
/// indices for each input. Translation of `SortDevObjects()` (and
/// `SortDevFunc()`).
pub(super) fn sort_dev_objects(inputs: &mut [DeviceInput]) {
    let mut n_buttons = 0u8;
    let mut n_hats = 0u8;
    let mut n_axis = 0u8;

    // (SDL_qsort() isn't stable; inputs with the same offset only come from
    // duplicated objects, whose order doesn't matter)
    inputs.sort_by_key(|input| input.ofs);

    for input in inputs {
        let counter = match input.kind {
            InputType::Button => &mut n_buttons,
            InputType::Hat => &mut n_hats,
            InputType::Axis => &mut n_axis,
        };
        input.num = *counter;
        *counter = counter.wrapping_add(1);
    }
}

/// Translation of `SDL_DINPUT_JoystickOpen()`: the DirectInput state of
/// the joystick, which is released (unacquired) on failure.
pub(super) fn joystick_open(
    joystick: &mut JoystickData,
    joystickdevice: &JoyStickDeviceData,
) -> Result<DinputHwData> {
    let dinput = dinput().ok_or_else(|| set_di_error("IDirectInput::CreateDevice", -1))?;

    // FIXME (upstream): on any failure below, the device created here stays
    // in joystick->hwdata, which the caller frees without releasing it; here
    // it is released when the error drops it.
    let input_device = dinput
        .create_device(&joystickdevice.dxdevice.guidInstance)
        .map_err(|hr| set_di_error("IDirectInput::CreateDevice", hr))?;
    let mut hwdata = DinputHwData {
        input_device,
        capabilities: DIDEVCAPS {
            dwSize: size_of::<DIDEVCAPS>() as u32,
            ..Default::default()
        },
        buffered: true,
        first_update: false,
        inputs: Vec::new(),
        num_sliders: 0,
        ff_initialized: false,
        ffeffect: None,
        ffeffect_ref: None,
    };
    let device = hwdata.input_device.clone();

    /* Acquire shared access. Exclusive access is required for forces,
     * though. */
    let helper_window = helper_window::hwnd().unwrap_or(std::ptr::null_mut());
    device
        .set_cooperative_level(helper_window, DISCL_EXCLUSIVE | DISCL_BACKGROUND)
        .map_err(|hr| set_di_error("IDirectInputDevice8::SetCooperativeLevel", hr))?;

    // Use the extended data structure: DIJOYSTATE2.
    device
        .set_data_format(&C_DF_DIJOYSTICK2.0)
        .map_err(|hr| set_di_error("IDirectInputDevice8::SetDataFormat", hr))?;

    if !has_broken_ezfrd64_dll() {
        // Get device capabilities to see if we are force feedback capable
        device
            .get_capabilities(&mut hwdata.capabilities)
            .map_err(|hr| set_di_error("IDirectInputDevice8::GetCapabilities", hr))?;
    }

    // Force capable?
    if hwdata.capabilities.dwFlags & DIDC_FORCEFEEDBACK != 0 {
        let result = device.acquire();
        if result < 0 {
            return Err(set_di_error("IDirectInputDevice8::Acquire", result));
        }

        // reset all actuators.
        // (Not necessarily supported, ignore if not supported.)
        let _ = device.send_force_feedback_command(DISFFC_RESET);

        let result = device.unacquire();
        if result < 0 {
            return Err(set_di_error("IDirectInputDevice8::Unacquire", result));
        }

        /* Turn on auto-centering for a ForceFeedback device (until told
         * otherwise). */
        // (Not necessarily supported, ignore if not supported.)
        let _ = device.set_property_dword(DIPROP_AUTOCENTER, 0, DIPH_DEVICE, DIPROPAUTOCENTER_ON);

        let _ = joystick
            .properties()
            .set(PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN, true);
    }

    // What buttons and axes does it have?
    let mut counts = ObjectCounts {
        nbuttons: joystick.nbuttons,
        nhats: joystick.nhats,
        naxes: joystick.naxes,
        ..Default::default()
    };
    let _ = device.enum_objects(
        |object| {
            enum_dev_objects_callback(object, &mut counts, |dw_type| {
                configure_axis(&device, dw_type)
            })
        },
        DIDFT_BUTTON | DIDFT_AXIS | DIDFT_POV,
    );
    joystick.nbuttons = counts.nbuttons;
    joystick.nhats = counts.nhats;
    joystick.naxes = counts.naxes;
    hwdata.inputs = counts.inputs;
    hwdata.num_sliders = counts.num_sliders;

    /* Reorder the input objects. Some devices do not report the X axis as
     * the first axis, for example. */
    sort_dev_objects(&mut hwdata.inputs);

    // Set the buffer size
    let result = device.set_property_dword(DIPROP_BUFFERSIZE, 0, DIPH_DEVICE, INPUT_QSIZE as u32);

    match result {
        Ok(DI_POLLEDDEVICE) => {
            /* This device doesn't support buffering, so we're forced
             * to use less reliable polling. */
            hwdata.buffered = false;
        }
        Err(hr) => return Err(set_di_error("IDirectInputDevice8::SetProperty", hr)),
        Ok(_) => {}
    }
    hwdata.first_update = true;

    // Poll and wait for initial device state to be populated
    let result = device.poll();
    if result == DIERR_INPUTLOST || result == DIERR_NOTACQUIRED {
        device.acquire();
        device.poll();
    }
    std::thread::sleep(std::time::Duration::from_millis(50));

    Ok(hwdata)
}

/// Translation of `SDL_DINPUT_JoystickInitRumble()`.
fn joystick_init_rumble(hwdata: &mut DinputHwData, magnitude: i16) -> Result<()> {
    let device = &hwdata.input_device;

    // Reset and then enable actuators
    let mut result = device.send_force_feedback_command(DISFFC_RESET);
    if result == DIERR_INPUTLOST || result == DIERR_NOTEXCLUSIVEACQUIRED {
        result = device.acquire();
        if result >= 0 {
            result = device.send_force_feedback_command(DISFFC_RESET);
        }
    }
    if result < 0 {
        return Err(set_di_error(
            "IDirectInputDevice8::SendForceFeedbackCommand(DISFFC_RESET)",
            result,
        ));
    }

    let result = device.send_force_feedback_command(DISFFC_SETACTUATORSON);
    if result < 0 {
        return Err(set_di_error(
            "IDirectInputDevice8::SendForceFeedbackCommand(DISFFC_SETACTUATORSON)",
            result,
        ));
    }

    // Create the effect
    // FIXME (upstream): when CreateEffect() fails, the effect data stays
    // allocated while ff_initialized stays false, so the next rumble
    // allocates another and leaks this one; here it is replaced (and freed).
    let ffeffect = hwdata.ffeffect.insert(RumbleEffect::new(magnitude));
    let effect = ffeffect.dieffect();

    let effect_ref = device
        .create_effect(&GUID_SINE, &effect)
        .map_err(|hr| set_di_error("IDirectInputDevice8::CreateEffect", hr))?;
    hwdata.ffeffect_ref = Some(effect_ref);
    Ok(())
}

/// Scale and average the two rumble strengths (the magnitude of
/// `SDL_DINPUT_JoystickRumble()`).
pub(super) fn rumble_magnitude(low_frequency_rumble: u16, high_frequency_rumble: u16) -> i16 {
    (((low_frequency_rumble / 2) as i32 + (high_frequency_rumble / 2) as i32) / 2) as i16
}

/// Translation of `SDL_DINPUT_JoystickRumble()`.
pub(super) fn joystick_rumble(
    hwdata: &mut DinputHwData,
    low_frequency_rumble: u16,
    high_frequency_rumble: u16,
) -> Result<()> {
    // Scale and average the two rumble strengths
    let magnitude = rumble_magnitude(low_frequency_rumble, high_frequency_rumble);

    if hwdata.capabilities.dwFlags & DIDC_FORCEFEEDBACK == 0 {
        return Err(Error::unsupported());
    }

    if hwdata.ff_initialized {
        // (ff_initialized means both exist)
        let (Some(ffeffect), Some(effect_ref)) = (&mut hwdata.ffeffect, &hwdata.ffeffect_ref)
        else {
            return Err(Error::unsupported());
        };
        ffeffect.periodic.dwMagnitude = convert_magnitude(magnitude);
        let effect = ffeffect.dieffect();

        let flags = DIEP_DURATION | DIEP_TYPESPECIFICPARAMS;
        let mut result = effect_ref.set_parameters(&effect, flags);
        if result == DIERR_INPUTLOST {
            result = hwdata.input_device.acquire();
            if result >= 0 {
                result = effect_ref.set_parameters(&effect, flags);
            }
        }
        if result < 0 {
            return Err(set_di_error("IDirectInputDevice8::SetParameters", result));
        }
    } else {
        joystick_init_rumble(hwdata, magnitude)?;
        hwdata.ff_initialized = true;
    }
    let Some(effect_ref) = &hwdata.ffeffect_ref else {
        return Err(Error::unsupported());
    };

    let mut result = effect_ref.start(1, 0);
    if result == DIERR_INPUTLOST || result == DIERR_NOTEXCLUSIVEACQUIRED {
        result = hwdata.input_device.acquire();
        if result >= 0 {
            result = effect_ref.start(1, 0);
        }
    }
    if result < 0 {
        return Err(set_di_error("IDirectInputDevice8::Start", result));
    }
    Ok(())
}

/// The hat position of a POV value (hundredths of degrees clockwise from
/// north, or centered when the low word is 0xFFFF). Translation of
/// `TranslatePOV()`.
pub(super) fn translate_pov(value: u32) -> u8 {
    const HAT_VALS: [u8; 8] = [
        HAT_UP,
        HAT_UP | HAT_RIGHT,
        HAT_RIGHT,
        HAT_DOWN | HAT_RIGHT,
        HAT_DOWN,
        HAT_DOWN | HAT_LEFT,
        HAT_LEFT,
        HAT_UP | HAT_LEFT,
    ];

    if value & 0xFFFF == 0xFFFF {
        return HAT_CENTERED;
    }

    // Round the value up:
    let value = (value.wrapping_add(4500 / 2) % 36000) / 4500;

    // (8 or more shouldn't happen)
    HAT_VALS
        .get(value as usize)
        .copied()
        .unwrap_or(HAT_CENTERED)
}

/// The events of a polled state: each known axis, button and POV.
/// Translation of the loop of `UpdateDINPUTJoystickState_Polled()`.
pub(super) fn polled_state_inputs(inputs: &[DeviceInput], state: &DIJOYSTATE2) -> Vec<Input> {
    let mut out = Vec::with_capacity(inputs.len());
    for input in inputs {
        match input.kind {
            InputType::Axis => {
                let value = match input.ofs {
                    DIJOFS_X => state.lX,
                    DIJOFS_Y => state.lY,
                    DIJOFS_Z => state.lZ,
                    DIJOFS_RX => state.lRx,
                    DIJOFS_RY => state.lRy,
                    DIJOFS_RZ => state.lRz,
                    ofs if ofs == dijofs_slider(0) => state.rglSlider[0],
                    ofs if ofs == dijofs_slider(1) => state.rglSlider[1],
                    _ => continue,
                };
                out.push(Input::Axis(input.num, value as i16));
            }
            InputType::Button => {
                // FIXME (upstream): a device with more than 128 buttons
                // gets offsets past rgbButtons, which the C code reads out
                // of bounds; those buttons read as released here.
                let index = input.ofs.wrapping_sub(DIJOFS_BUTTON0) as usize;
                let down = state.rgbButtons.get(index).is_some_and(|&b| b != 0);
                out.push(Input::Button(input.num, down));
            }
            InputType::Hat => {
                let pos = translate_pov(state.rgdwPOV[input.num as usize % 4]);
                out.push(Input::Hat(input.num, pos));
            }
        }
    }
    out
}

/// The events of buffered device data. Translation of the loop of
/// `UpdateDINPUTJoystickState_Buffered()`.
pub(super) fn buffered_inputs(inputs: &[DeviceInput], events: &[DIDEVICEOBJECTDATA]) -> Vec<Input> {
    let mut out = Vec::new();
    for event in events {
        for input in inputs.iter().filter(|input| event.dwOfs == input.ofs) {
            out.push(match input.kind {
                InputType::Axis => Input::Axis(input.num, event.dwData as i16),
                InputType::Button => Input::Button(input.num, event.dwData != 0),
                InputType::Hat => Input::Hat(input.num, translate_pov(event.dwData)),
            });
        }
    }
    out
}

/// Function to update the state of a joystick - called as a device poll.
/// Translation of `UpdateDINPUTJoystickState_Polled()`: the events to send.
fn update_state_polled(hwdata: &DinputHwData) -> Vec<Input> {
    let device = &hwdata.input_device;
    let mut state = DIJOYSTATE2::default();

    let mut result = device.get_device_state(&mut state);
    if result == DIERR_INPUTLOST || result == DIERR_NOTACQUIRED {
        device.acquire();
        result = device.get_device_state(&mut state);
    }

    if result != DI_OK {
        return Vec::new();
    }

    // Set each known axis, button and POV.
    polled_state_inputs(&hwdata.inputs, &state)
}

/// Translation of `UpdateDINPUTJoystickState_Buffered()`: the events to
/// send.
fn update_state_buffered(hwdata: &DinputHwData) -> Vec<Input> {
    let device = &hwdata.input_device;
    let mut evtbuf = [DIDEVICEOBJECTDATA::default(); INPUT_QSIZE];

    let (mut result, mut numevents) = device.get_device_data(&mut evtbuf);
    if result == DIERR_INPUTLOST || result == DIERR_NOTACQUIRED {
        device.acquire();
        (result, numevents) = device.get_device_data(&mut evtbuf);
    }

    // Handle the events or punt
    if result < 0 {
        return Vec::new();
    }

    let mut out = buffered_inputs(&hwdata.inputs, &evtbuf[..numevents]);

    if result == DI_BUFFEROVERFLOW {
        /* Our buffer wasn't big enough to hold all the queued events,
         * so poll the device to make sure we have the complete state.
         */
        out.extend(update_state_polled(hwdata));
    }
    out
}

/// Translation of `SDL_DINPUT_JoystickUpdate()`: the events to send (the
/// caller sends them, without the device list borrowed).
pub(super) fn joystick_update(hwdata: &mut DinputHwData) -> Vec<Input> {
    let device = &hwdata.input_device;
    let result = device.poll();
    if result == DIERR_INPUTLOST || result == DIERR_NOTACQUIRED {
        device.acquire();
        device.poll();
    }

    if hwdata.first_update {
        // Poll to get the initial state of the joystick
        hwdata.first_update = false;
        return update_state_polled(hwdata);
    }

    if hwdata.buffered {
        update_state_buffered(hwdata)
    } else {
        update_state_polled(hwdata)
    }
}

/// Translation of `SDL_DINPUT_JoystickClose()`.
pub(super) fn joystick_close(mut hwdata: DinputHwData) {
    if let Some(effect_ref) = hwdata.ffeffect_ref.take() {
        effect_ref.unload();
        // FIXME (upstream): the effect is unloaded but never released, which
        // leaks it; here dropping it releases it.
    }
    hwdata.ffeffect = None;
    hwdata.input_device.unacquire();
    // (the device is released when hwdata is dropped)
}

/// Translation of `SDL_DINPUT_JoystickQuit()`.
pub(super) fn joystick_quit() {
    let mut s = lock();
    s.dinput = None;

    if s.coinitialized {
        co_uninitialize();
        s.coinitialized = false;
    }
}
