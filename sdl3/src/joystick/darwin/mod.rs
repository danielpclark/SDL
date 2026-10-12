// Rust translation of src/joystick/darwin/SDL_iokitjoystick.c and
// SDL_iokitjoystick_c.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The IOKit joystick driver (`SDL_DARWIN_JoystickDriver`): joysticks,
//! gamepads and multi-axis controllers found by an IOHIDManager scheduled
//! on the run loop of the thread that initialized the joystick subsystem
//! (hotplug callbacks fire when [`detect`](JoystickDriver::detect) runs it
//! in its own mode), read element by element, with rumble through the
//! ForceFeedback framework.
//!
//! Upstream's device list is a linked list of `recDevice`s that open
//! joysticks point into (`hwdata`); here the devices are a `Vec` and an
//! open joystick's `hwdata` is whether its instance ID is in `open`, the
//! device being found by that ID (a device's instance ID is its joystick's).

use std::ffi::c_void;
use std::sync::{Mutex, MutexGuard};

use super::{
    assert_joysticks_locked, create_joystick_guid, create_joystick_name,
    is_joystick_steam_virtual_gamepad, is_joystick_xbox_one, joystick_handled_by_another_driver,
    private_joystick_added, private_joystick_removed, send_joystick_axis, send_joystick_button,
    send_joystick_hat, should_ignore_joystick, usb_ids::*, JoystickData, JoystickDriver,
    DARWIN_DRIVER_INDEX, HARDWARE_BUS_USB, HAT_CENTERED, HAT_DOWN, HAT_LEFT, HAT_LEFTDOWN,
    HAT_LEFTUP, HAT_RIGHT, HAT_RIGHTDOWN, HAT_RIGHTUP, HAT_UP, MAX_RUMBLE_DURATION_MS,
    PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN,
};
use crate::core::apple::foundation::{
    cf_number_get_i32, cf_string, kCFAllocatorDefault, kCFBooleanTrue, kCFTypeArrayCallBacks,
    kCFTypeDictionaryKeyCallBacks, kCFTypeDictionaryValueCallBacks, string_from_cf, CFArrayCreate,
    CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef, CFDictionaryCreate, CFGetTypeID,
    CFNumberCreate, CFRelease, CFRetain, CFRunLoopGetCurrent, CFRunLoopRunInMode,
    CFStringGetTypeID, CFStringRef, CfRef, K_CF_NUMBER_INT_TYPE, K_CF_RUN_LOOP_RUN_HANDLED_SOURCE,
};
use crate::core::apple::iokit::*;
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::guid::Guid;
use crate::hints;

#[cfg(test)]
mod tests;

/// `SDL_JOYSTICK_RUNLOOP_MODE`, `CFSTR("SDLJoystick")`: created once and
/// never released, like a `CFSTR()` constant (the HID manager and devices
/// keep the mode they are scheduled in without retaining it).
fn joystick_runloop_mode() -> CFStringRef {
    static MODE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| cf_string("SDLJoystick").into_raw() as usize) as CFStringRef
}

/// `CONVERT_MAGNITUDE(x)`, the 0..10000 range of ForceFeedback (into the
/// `DWORD` it is stored in).
fn convert_magnitude(x: i16) -> u32 {
    ((i32::from(x) * 10000) / 0x7FFF) as u32
}

/// An `FFEFFECT` for rumble with the axes, directions and periodic
/// parameters it points to (upstream's separately allocated buffers,
/// freed by `FreeRumbleEffectData()` when this is dropped).
pub(crate) struct RumbleEffect {
    pub(crate) effect: FFEFFECT,
    axes: [u32; 2],
    direction: [i32; 2],
    periodic: FFPERIODIC,
}

impl RumbleEffect {
    /// The effect's periodic parameters (`lpvTypeSpecificParams`).
    fn periodic_mut(&mut self) -> &mut FFPERIODIC {
        &mut self.periodic
    }
}

/// Translation of `CreateRumbleEffectData()`.
pub(crate) fn create_rumble_effect_data(magnitude: i16) -> Box<RumbleEffect> {
    // Create the effect
    let mut effect = Box::new(RumbleEffect {
        effect: FFEFFECT::default(),
        axes: [0; 2],
        direction: [0; 2],
        periodic: FFPERIODIC::default(),
    });
    effect.effect.dw_size = size_of::<FFEFFECT>() as u32;
    effect.effect.dw_gain = 10000;
    effect.effect.dw_flags = FFEFF_OBJECTOFFSETS;
    effect.effect.dw_duration = MAX_RUMBLE_DURATION_MS * 1000; // In microseconds.
    effect.effect.dw_trigger_button = FFEB_NOTRIGGER;

    effect.effect.c_axes = 2;
    effect.effect.rgdw_axes = effect.axes.as_mut_ptr();

    effect.effect.rgl_direction = effect.direction.as_mut_ptr();
    effect.effect.dw_flags |= FFEFF_CARTESIAN;

    effect.periodic.dw_magnitude = convert_magnitude(magnitude);
    effect.periodic.dw_period = 1000000;

    effect.effect.cb_type_specific_params = size_of::<FFPERIODIC>() as u32;
    effect.effect.lpv_type_specific_params = (&mut effect.periodic as *mut FFPERIODIC).cast();

    effect
}

/// Translation of `struct recElement`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RecElement {
    element_ref: IOHIDElementRef,
    cookie: IOHIDElementCookie,
    /// HID usage
    #[allow(dead_code)] // (recorded, never read, as upstream)
    usage_page: u32,
    usage: u32,
    /// reported min value possible
    min: i32,
    /// reported max value possible
    max: i32,

    // runtime variables used for auto-calibration
    /// min returned value
    min_report: i32,
    /// max returned value
    max_report: i32,
}

/// A device of the driver. Translation of `struct joystick_hwdata`
/// (`recDevice`).
pub(crate) struct RecDevice {
    /// HIDManager device handle
    device_ref: IOHIDDeviceRef,
    /// Interface for force feedback, 0 = no ff
    pub(crate) ffservice: IoService,
    ffdevice: FFDeviceObjectReference,
    ffeffect: Option<Box<RumbleEffect>>,
    ffeffect_ref: FFEffectObjectReference,
    ff_initialized: bool,

    /// name of product
    product: String,
    /// usage page from IOUSBHID Parser.h which defines general usage
    usage: u32,
    /// usage within above page from IOUSBHID Parser.h which defines specific usage
    usage_page: u32,

    /// number of axis (calculated, not reported by device)
    axes: usize,
    /// number of buttons (calculated, not reported by device)
    buttons: usize,
    /// number of hat switches (calculated, not reported by device)
    hats: usize,
    /// number of total elements (should be total of above) (calculated, not reported by device)
    elements: usize,

    first_axis: Vec<RecElement>,
    first_button: Vec<RecElement>,
    first_hat: Vec<RecElement>,

    removed: bool,
    /// The open joystick of this device (`joystick`).
    joystick: Option<JoystickID>,
    /// is 'deviceRef' attached to a CFRunLoop?
    run_loop_attached: bool,

    instance_id: JoystickID,
    guid: Guid,
    steam_virtual_gamepad_slot: i32,
    nacon_revolution_x_unlimited: bool,
}

impl RecDevice {
    fn new() -> RecDevice {
        RecDevice {
            device_ref: std::ptr::null_mut(),
            ffservice: 0,
            ffdevice: std::ptr::null_mut(),
            ffeffect: None,
            ffeffect_ref: std::ptr::null_mut(),
            ff_initialized: false,
            product: String::new(),
            usage: 0,
            usage_page: 0,
            axes: 0,
            buttons: 0,
            hats: 0,
            elements: 0,
            first_axis: Vec::new(),
            first_button: Vec::new(),
            first_hat: Vec::new(),
            removed: false,
            joystick: None,
            run_loop_attached: false,
            instance_id: 0,
            guid: Guid::ZERO,
            steam_virtual_gamepad_slot: 0,
            nacon_revolution_x_unlimited: false,
        }
    }
}

/// The driver's state (the file-level statics of `SDL_iokitjoystick.c`).
struct DarwinState {
    /// The base object of the HID Manager API (`hidman`).
    hidman: IOHIDManagerRef,
    /// List of all available devices (`gpDeviceList`).
    devices: Vec<RecDevice>,
    /// The open joysticks whose `hwdata` is set (their device's ID).
    open: Vec<JoystickID>,
}

// SAFETY: the IOKit references are used with the mutex held, from the
// threads holding the joystick lock, as upstream uses them.
unsafe impl Send for DarwinState {}

static STATE: Mutex<DarwinState> = Mutex::new(DarwinState {
    hidman: std::ptr::null_mut(),
    devices: Vec::new(),
    open: Vec::new(),
});

fn lock_state() -> MutexGuard<'static, DarwinState> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

impl DarwinState {
    /// Translation of `GetDeviceForIndex()`.
    fn device_for_index(&mut self, device_index: usize) -> Option<&mut RecDevice> {
        self.devices
            .iter_mut()
            .filter(|d| !d.removed)
            .nth(device_index)
    }

    fn device_by_id(&mut self, instance_id: JoystickID) -> Option<&mut RecDevice> {
        self.devices
            .iter_mut()
            .find(|d| d.instance_id == instance_id)
    }

    /// The device an open joystick's `hwdata` points to.
    fn hwdata(&mut self, joystick: JoystickID) -> Option<&mut RecDevice> {
        if !self.open.contains(&joystick) {
            return None;
        }
        self.device_by_id(joystick)
    }

    /// `joystick->hwdata = NULL`.
    fn clear_hwdata(&mut self, joystick: JoystickID) {
        self.open.retain(|&j| j != joystick);
    }
}

/// Translation of `FreeDevice()`: remove the device at `index` from the
/// list.
fn free_device(state: &mut DarwinState, index: usize) {
    let remove_device = &mut state.devices[index];
    if !remove_device.device_ref.is_null() {
        if remove_device.run_loop_attached {
            /* Calling IOHIDDeviceUnscheduleFromRunLoop without a prior,
             * paired call to IOHIDDeviceScheduleWithRunLoop can lead
             * to crashes in MacOS 10.14.x and earlier.  This doesn't
             * appear to be a problem in MacOS 10.15.x, but we'll
             * do it anyways.  (Part-of fix for Bug 5034)
             */
            let mode = joystick_runloop_mode();
            // SAFETY: the device is retained and was scheduled on this
            // mode.
            unsafe {
                IOHIDDeviceUnscheduleFromRunLoop(
                    remove_device.device_ref,
                    CFRunLoopGetCurrent(),
                    mode,
                )
            };
        }
        // SAFETY: the reference GetDeviceInfo() retained, released once.
        unsafe { CFRelease(remove_device.device_ref) };
        remove_device.device_ref = std::ptr::null_mut();
    }

    /* clear out any reference to removeDevice from an associated,
     * live instance of SDL_Joystick  (Part-of fix for Bug 5034)
     */
    let joystick = remove_device.joystick;
    {
        let _lock = super::lock_joysticks();
        if let Some(joystick) = joystick {
            state.clear_hwdata(joystick);
        }
    }

    // Note (upstream): the force feedback device and effect of a device
    // that wasn't unplugged are leaked here; they are released with it.
    let mut device = state.devices.remove(index);
    release_force_feedback(&mut device);

    // (the element lists are freed with the device)
}

/// Release a device's ForceFeedback effect and device.
fn release_force_feedback(device: &mut RecDevice) {
    // SAFETY: the effect and device references are ours, released once.
    unsafe {
        if !device.ffeffect_ref.is_null() {
            FFDeviceReleaseEffect(device.ffdevice, device.ffeffect_ref);
            device.ffeffect_ref = std::ptr::null_mut();
        }
        device.ffeffect = None;
        if !device.ffdevice.is_null() {
            FFReleaseDevice(device.ffdevice);
            device.ffdevice = std::ptr::null_mut();
            device.ff_initialized = false;
        }
    }
}

/// Translation of `GetHIDElementState()`.
fn get_hid_element_state(device_ref: IOHIDDeviceRef, element: &mut RecElement) -> Option<i32> {
    if device_ref.is_null() {
        return None;
    }
    let mut value_ref: IOHIDValueRef = std::ptr::null_mut();
    // SAFETY: a retained device and one of its elements; the value is
    // owned by the device.
    if unsafe { IOHIDDeviceGetValue(device_ref, element.element_ref, &mut value_ref) }
        == K_IO_RETURN_SUCCESS
    {
        // SAFETY: the value IOHIDDeviceGetValue() returned.
        let value = unsafe { IOHIDValueGetIntegerValue(value_ref) } as i32;

        // record min and max for auto calibration
        if value < element.min_report {
            element.min_report = value;
        }
        if value > element.max_report {
            element.max_report = value;
        }
        return Some(value);
    }
    None
}

/// The scaling of `GetHIDScaledCalibratedState()`: `value` scaled from the
/// range read so far to `min`..`max` (left alone while that range is
/// empty). `read_scale` is the range's width before the read,
/// `min_report` its minimum after it, as upstream evaluates them.
pub(crate) fn scale_calibrated(
    value: i32,
    min_report: i32,
    read_scale: f32,
    min: i32,
    max: i32,
) -> i32 {
    let device_scale = max.wrapping_sub(min) as f32;
    if read_scale == 0.0 {
        value // no scaling at all
    } else {
        ((value.wrapping_sub(min_report) as f32 * device_scale / read_scale) + min as f32) as i32
    }
}

/// Translation of `GetHIDScaledCalibratedState()`.
fn get_hid_scaled_calibrated_state(
    device_ref: IOHIDDeviceRef,
    element: &mut RecElement,
    min: i32,
    max: i32,
) -> Option<i32> {
    let read_scale = element.max_report.wrapping_sub(element.min_report) as f32;
    let value = get_hid_element_state(device_ref, element)?;
    Some(scale_calibrated(
        value,
        element.min_report,
        read_scale,
        min,
        max,
    ))
}

/// The thumbstick values of the NACON Revolution X Unlimited (the second
/// half of `GetHIDScaledCalibratedState_NACON_Revolution_X_Unlimited()`).
pub(crate) fn nacon_axis_value(value: i32) -> i32 {
    if value >= 0 {
        // Negative axis values range from 32767 (at rest) to 0 (minimum)
        -32767 + value
    } else {
        // Positive axis values range from -32768 (at rest) to 0 (maximum)
        32768 + value
    }
}

/// Translation of `GetHIDScaledCalibratedState_NACON_Revolution_X_Unlimited()`.
fn get_hid_scaled_calibrated_state_nacon_revolution_x_unlimited(
    device_ref: IOHIDDeviceRef,
    element: &mut RecElement,
    min: i32,
    max: i32,
) -> Option<i32> {
    if element.min_report == 0 && element.max_report == 255 {
        return get_hid_scaled_calibrated_state(device_ref, element, min, max);
    }

    // This device thumbstick axes have an unusual axis range that
    // doesn't work with GetHIDScaledCalibratedState() above.
    //
    // See https://github.com/libsdl-org/SDL/issues/13143 for details
    get_hid_element_state(device_ref, element).map(nacon_axis_value)
}

/// Translation of `JoystickDeviceWasRemovedCallback()`.
unsafe extern "C" fn joystick_device_was_removed_callback(
    ctx: *mut c_void,
    _result: IOReturn,
    _sender: *mut c_void,
) {
    let instance_id = ctx as usize as JoystickID;
    let ffservice = {
        let mut state = lock_state();
        let Some(device) = state.device_by_id(instance_id) else {
            return;
        };
        device.removed = true;
        if !device.device_ref.is_null() {
            // deviceRef was invalidated due to the remove
            // SAFETY: the reference GetDeviceInfo() retained, released once.
            unsafe { CFRelease(device.device_ref) };
            device.device_ref = std::ptr::null_mut();
        }
        release_force_feedback(device);
        device.ffservice
    };
    crate::haptic::darwin::mac_haptic_maybe_remove_device(ffservice);

    private_joystick_removed(instance_id);
}

/// A HID element we care about, by its type and usage: which of the
/// device's lists it goes in (the switch of `AddHIDElement()`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ElementKind {
    Axis,
    Button,
    Hat,
}

/// Translation of the type and usage checks of `AddHIDElement()`.
pub(crate) fn element_kind(
    element_type: IOHIDElementType,
    usage_page: u32,
    usage: u32,
) -> Option<ElementKind> {
    // look at types of interest
    match element_type {
        K_IOHID_ELEMENT_TYPE_INPUT_MISC
        | K_IOHID_ELEMENT_TYPE_INPUT_BUTTON
        | K_IOHID_ELEMENT_TYPE_INPUT_AXIS => {
            match usage_page {
                // only interested in kHIDPage_GenericDesktop and kHIDPage_Button
                K_HID_PAGE_GENERIC_DESKTOP => match usage {
                    K_HID_USAGE_GD_X
                    | K_HID_USAGE_GD_Y
                    | K_HID_USAGE_GD_Z
                    | K_HID_USAGE_GD_RX
                    | K_HID_USAGE_GD_RY
                    | K_HID_USAGE_GD_RZ
                    | K_HID_USAGE_GD_SLIDER
                    | K_HID_USAGE_GD_DIAL
                    | K_HID_USAGE_GD_WHEEL => Some(ElementKind::Axis),

                    K_HID_USAGE_GD_HATSWITCH => Some(ElementKind::Hat),
                    K_HID_USAGE_GD_DPAD_UP
                    | K_HID_USAGE_GD_DPAD_DOWN
                    | K_HID_USAGE_GD_DPAD_RIGHT
                    | K_HID_USAGE_GD_DPAD_LEFT
                    | K_HID_USAGE_GD_START
                    | K_HID_USAGE_GD_SELECT
                    | K_HID_USAGE_GD_SYSTEM_MAIN_MENU => Some(ElementKind::Button),
                    _ => None,
                },

                K_HID_PAGE_SIMULATION => match usage {
                    K_HID_USAGE_SIM_RUDDER
                    | K_HID_USAGE_SIM_THROTTLE
                    | K_HID_USAGE_SIM_ACCELERATOR
                    | K_HID_USAGE_SIM_BRAKE => Some(ElementKind::Axis),

                    _ => None,
                },

                // e.g. 'pause' button on Steelseries MFi gamepads.
                K_HID_PAGE_BUTTON | K_HID_PAGE_CONSUMER => Some(ElementKind::Button),

                _ => None,
            }
        }
        _ => None,
    }
}

/// Add `element` to a list kept sorted by usage (after the elements of
/// the same usage), unless an element with its cookie is there (the
/// `ElementAlreadyAdded()` check and the list insertion of
/// `AddHIDElement()`). Returns whether it was added.
pub(crate) fn add_sorted(list: &mut Vec<RecElement>, element: RecElement) -> bool {
    if list.iter().any(|e| e.cookie == element.cookie) {
        return false;
    }
    let pos = list
        .iter()
        .position(|e| element.usage < e.usage)
        .unwrap_or(list.len());
    list.insert(pos, element);
    true
}

/// Call AddHIDElement() on all elements in an array of IOHIDElementRefs.
/// Translation of `AddHIDElements()`.
///
/// # Safety
///
/// `array` must be a valid array of the device's elements.
unsafe fn add_hid_elements(array: CFArrayRef, device: &mut RecDevice) {
    // SAFETY: the caller's contract.
    unsafe {
        for i in 0..CFArrayGetCount(array) {
            add_hid_element(CFArrayGetValueAtIndex(array, i) as IOHIDElementRef, device);
        }
    }
}

/// See if we care about this HID element, and if so, note it in our
/// recDevice. Translation of `AddHIDElement()`.
///
/// # Safety
///
/// `ref_element` must be null or a valid CF object.
unsafe fn add_hid_element(ref_element: IOHIDElementRef, device: &mut RecDevice) {
    // SAFETY: the caller's contract; the element queries are made on an
    // element, as its type ID says.
    unsafe {
        if ref_element.is_null() || CFGetTypeID(ref_element) != IOHIDElementGetTypeID() {
            return;
        }
        let cookie = IOHIDElementGetCookie(ref_element);
        let usage_page = IOHIDElementGetUsagePage(ref_element);
        let usage = IOHIDElementGetUsage(ref_element);
        let element_type = IOHIDElementGetType(ref_element);

        if element_type == K_IOHID_ELEMENT_TYPE_COLLECTION {
            let array = IOHIDElementGetChildren(ref_element);
            if !array.is_null() {
                add_hid_elements(array, device);
            }
            return;
        }

        let Some(kind) = element_kind(element_type, usage_page, usage) else {
            return;
        };
        let min = IOHIDElementGetLogicalMin(ref_element) as i32;
        let max = IOHIDElementGetLogicalMax(ref_element) as i32;
        let element = RecElement {
            element_ref: ref_element,
            cookie,
            usage_page,
            usage,
            min,
            max,
            min_report: min,
            max_report: max,
        };
        // add to list
        let added = match kind {
            ElementKind::Axis => add_sorted(&mut device.first_axis, element),
            ElementKind::Hat => add_sorted(&mut device.first_hat, element),
            ElementKind::Button => add_sorted(&mut device.first_button, element),
        };
        if added {
            match kind {
                ElementKind::Axis => device.axes += 1,
                ElementKind::Hat => device.hats += 1,
                ElementKind::Button => device.buttons += 1,
            }
            device.elements += 1;
        }
    }
}

/// Translation of `GetSteamVirtualGamepadSlot()`.
pub(crate) fn get_steam_virtual_gamepad_slot(
    vendor_id: u16,
    product_id: u16,
    product_string: &str,
) -> i32 {
    let mut slot = -1;

    if vendor_id == USB_VENDOR_MICROSOFT && product_id == USB_PRODUCT_XBOX360_WIRED_CONTROLLER {
        // Gamepad name is "GamePad-N", where N is slot + 1
        // (SDL_sscanf(product_string, "GamePad-%d", &slot) == 1)
        if let Some(rest) = product_string.strip_prefix("GamePad-") {
            let (n, len) = crate::stdlib::string::strtol(rest, 10);
            if len > 0 {
                slot = n as i32 - 1;
            }
        }
    }
    slot
}

/// Translation of `IsControlledBy360ControllerDriver()`.
///
/// # Safety
///
/// `hid_device` must be a valid device.
unsafe fn is_controlled_by_360_controller_driver(hid_device: IOHIDDeviceRef) -> bool {
    let mut controlled_by_360controller = false;
    // SAFETY: the caller's contract; the service belongs to the device.
    unsafe {
        let service = IOHIDDeviceGetService(hid_device);
        if service != MACH_PORT_NULL {
            controlled_by_360controller =
                IOObjectConformsTo(service, c"Xbox360ControllerClass".as_ptr()) != 0;
        }
    }
    controlled_by_360controller
}

/// A number property of a device (`CFNumberGetValue(kCFNumberSInt32Type)`
/// of `IOHIDDeviceGetProperty()`), `None` when the device doesn't have it.
///
/// # Safety
///
/// `hid_device` must be a valid device.
unsafe fn int_property(hid_device: IOHIDDeviceRef, key: &str) -> Option<i32> {
    let key = cf_string(key);
    // SAFETY: the caller's contract; the property is owned by the device.
    unsafe { cf_number_get_i32(IOHIDDeviceGetProperty(hid_device, key.get())) }
}

/// A string property of a device, as `CFStringGetCString()` copies it
/// into a 256-byte buffer: empty when it is missing or doesn't fit.
///
/// # Safety
///
/// `hid_device` must be a valid device.
unsafe fn string_property(hid_device: IOHIDDeviceRef, key: &str) -> String {
    let key = cf_string(key);
    // SAFETY: the caller's contract; the property is owned by the device.
    unsafe {
        let r = IOHIDDeviceGetProperty(hid_device, key.get());
        if r.is_null() || CFGetTypeID(r) != CFStringGetTypeID() {
            return String::new();
        }
        match string_from_cf(r) {
            Some(s) if s.len() < 256 => s,
            _ => String::new(),
        }
    }
}

/// `SDL_strlcpy(pDevice->product, name, sizeof(pDevice->product))`: at most
/// 255 bytes (cut at a character boundary here).
fn truncate_product(mut name: String) -> String {
    if name.len() > 255 {
        let mut end = 255;
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
    }
    name
}

/// Translation of `GetDeviceInfo()`.
///
/// # Safety
///
/// `hid_device` must be a valid device.
unsafe fn get_device_info(hid_device: IOHIDDeviceRef, device: &mut RecDevice) -> bool {
    // SAFETY: the caller's contract for every device query below.
    unsafe {
        // get usage page and usage
        // (upstream reads the numbers without checking their type)
        if let Some(usage_page) = int_property(hid_device, K_IOHID_PRIMARY_USAGE_PAGE_KEY) {
            device.usage_page = usage_page as u32;
        }
        if device.usage_page != K_HID_PAGE_GENERIC_DESKTOP {
            return false; // Filter device list to non-keyboard/mouse stuff
        }

        if let Some(usage) = int_property(hid_device, K_IOHID_PRIMARY_USAGE_KEY) {
            device.usage = usage as u32;
        }

        if device.usage != K_HID_USAGE_GD_JOYSTICK
            && device.usage != K_HID_USAGE_GD_GAMEPAD
            && device.usage != K_HID_USAGE_GD_MULTI_AXIS_CONTROLLER
        {
            return false; // Filter device list to non-keyboard/mouse stuff
        }

        /* Make sure we retain the use of the IOKit-provided device-object,
        lest the device get disconnected and we try to use it.  (Fixes
        SDL-Bugzilla #4961, aka. https://bugzilla.libsdl.org/show_bug.cgi?id=4961 )
        */
        CFRetain(hid_device);

        /* Now that we've CFRetain'ed the device-object (for our use), we'll
        save the reference to it.
        */
        device.device_ref = hid_device;

        let vendor = int_property(hid_device, K_IOHID_VENDOR_ID_KEY).unwrap_or(0);
        let product = int_property(hid_device, K_IOHID_PRODUCT_ID_KEY).unwrap_or(0);
        let mut version = int_property(hid_device, K_IOHID_VERSION_NUMBER_KEY).unwrap_or(0);

        if !is_controlled_by_360_controller_driver(hid_device)
            && is_joystick_xbox_one(vendor as u16, product as u16)
        {
            // We can't actually use this API for Xbox controllers without the 360Controller driver
            return false;
        }

        if is_joystick_steam_virtual_gamepad(vendor as u16, product as u16, version as u16) {
            let key = cf_string(K_IOHID_VIRTUAL_HIDEVICE);
            if IOHIDDeviceGetProperty(hid_device, key.get()) != kCFBooleanTrue {
                // This is a real Xbox 360 controller, adjust the version so it's not detected as a Steam virtual gamepad
                version = 1;
            }
        }

        // get device name
        let manufacturer_string = string_property(hid_device, K_IOHID_MANUFACTURER_KEY);
        let product_string = string_property(hid_device, K_IOHID_PRODUCT_KEY);
        if let Some(name) = create_joystick_name(
            vendor as u16,
            product as u16,
            Some(&manufacturer_string),
            Some(&product_string),
        ) {
            device.product = truncate_product(name);
        }

        if should_ignore_joystick(
            vendor as u16,
            product as u16,
            version as u16,
            Some(&device.product),
        ) {
            return false;
        }

        if joystick_handled_by_another_driver(
            DARWIN_DRIVER_INDEX,
            vendor as u16,
            product as u16,
            version as u16,
            Some(&device.product),
        ) {
            return false;
        }

        device.guid = create_joystick_guid(
            HARDWARE_BUS_USB,
            vendor as u16,
            product as u16,
            version as u16,
            Some(&manufacturer_string),
            Some(&product_string),
            0,
            0,
        );
        device.steam_virtual_gamepad_slot =
            get_steam_virtual_gamepad_slot(vendor as u16, product as u16, &product_string);

        if vendor == i32::from(USB_VENDOR_NACON_ALT)
            && product == i32::from(USB_PRODUCT_NACON_REVOLUTION_X_UNLIMITED_BT)
        {
            device.nacon_revolution_x_unlimited = true;
        }

        let array = IOHIDDeviceCopyMatchingElements(
            hid_device,
            std::ptr::null(),
            K_IOHID_OPTIONS_TYPE_NONE,
        );
        if !array.is_null() {
            add_hid_elements(array, device);
            CFRelease(array);
        }

        true
    }
}

/// Whether the GameController (MFi) driver handles a device
/// (`IOS_SupportedHIDDevice()`); the hook it fills in when it is
/// translated ([`super::JOYSTICK_MFI`]).
fn mfi_supported_hid_device(_device: IOHIDDeviceRef) -> bool {
    false
}

/// Translation of `JoystickAlreadyKnown()`.
///
/// # Safety
///
/// `io_hid_device_object` must be a valid device.
unsafe fn joystick_already_known(
    state: &DarwinState,
    io_hid_device_object: IOHIDDeviceRef,
) -> bool {
    // SAFETY: the caller's contract.
    if super::JOYSTICK_MFI
        && !unsafe { is_controlled_by_360_controller_driver(io_hid_device_object) }
        && mfi_supported_hid_device(io_hid_device_object)
    {
        return true;
    }

    state
        .devices
        .iter()
        .any(|i| i.device_ref == io_hid_device_object)
}

/// Translation of `JoystickDeviceWasAddedCallback()`.
unsafe extern "C" fn joystick_device_was_added_callback(
    _ctx: *mut c_void,
    res: IOReturn,
    _sender: *mut c_void,
    io_hid_device_object: IOHIDDeviceRef,
) {
    if res != K_IO_RETURN_SUCCESS {
        return;
    }

    // SAFETY: IOKit hands a valid device.
    if unsafe { joystick_already_known(&lock_state(), io_hid_device_object) } {
        return; // IOKit sent us a duplicate.
    }

    let mut device = RecDevice::new();

    // (with the driver state unlocked: the other drivers are asked whether
    // they handle the device)
    // SAFETY: IOKit hands a valid device.
    if !unsafe { get_device_info(io_hid_device_object, &mut device) } {
        // FreeDevice(): not a device we care about, probably.
        if !device.device_ref.is_null() {
            // SAFETY: the reference GetDeviceInfo() retained.
            unsafe { CFRelease(device.device_ref) };
        }
        return;
    }

    // Allocate an instance ID for this device
    // (before the removal callback is registered, as it identifies the
    // device to the callback)
    device.instance_id = crate::utils::next_object_id();

    // Get notified when this device is disconnected.
    let mode = joystick_runloop_mode();
    // SAFETY: the device is retained; the context is the instance ID.
    unsafe {
        IOHIDDeviceRegisterRemovalCallback(
            io_hid_device_object,
            Some(joystick_device_was_removed_callback),
            device.instance_id as usize as *mut c_void,
        );
        IOHIDDeviceScheduleWithRunLoop(io_hid_device_object, CFRunLoopGetCurrent(), mode);
    }
    device.run_loop_attached = true;

    // We have to do some storage of the io_service_t for SDL_OpenHapticFromJoystick
    // SAFETY: the service belongs to the device.
    let ioservice = unsafe { IOHIDDeviceGetService(io_hid_device_object) };
    // SAFETY: a valid service.
    if ioservice != 0 && unsafe { FFIsForceFeedback(ioservice) } == FF_OK {
        device.ffservice = ioservice;
        crate::haptic::darwin::mac_haptic_maybe_add_device(ioservice);
    }

    // Add device to the end of the list
    let instance_id = device.instance_id;
    lock_state().devices.push(device);

    private_joystick_added(instance_id);
}

/// Translation of `ConfigHIDManager()`.
///
/// # Safety
///
/// `hidman` must be a new manager and `matching_array` a valid array.
unsafe fn config_hid_manager(hidman: IOHIDManagerRef, matching_array: CFArrayRef) -> bool {
    // SAFETY: the caller's contract; the callbacks fire on this thread's
    // run loop, in the joystick mode.
    unsafe {
        let runloop = CFRunLoopGetCurrent();

        if IOHIDManagerOpen(hidman, K_IOHID_OPTIONS_TYPE_NONE) != K_IO_RETURN_SUCCESS {
            return false;
        }

        let mode = joystick_runloop_mode();
        IOHIDManagerSetDeviceMatchingMultiple(hidman, matching_array);
        IOHIDManagerRegisterDeviceMatchingCallback(
            hidman,
            Some(joystick_device_was_added_callback),
            std::ptr::null_mut(),
        );
        IOHIDManagerScheduleWithRunLoop(hidman, runloop, mode);

        while CFRunLoopRunInMode(mode, 0.0, 1) == K_CF_RUN_LOOP_RUN_HANDLED_SOURCE {
            // no-op. Callback fires once per existing device.
        }
    }

    // future hotplug events will come through SDL_JOYSTICK_RUNLOOP_MODE now.

    true // good to go.
}

/// Translation of `CreateHIDDeviceMatchDictionary()`.
fn create_hid_device_match_dictionary(page: u32, usage: u32) -> Option<CfRef> {
    // SAFETY: the numbers are created from values of the type named and
    // released after the dictionary retains them.
    unsafe {
        let page_num_ref = CfRef::from_owned(CFNumberCreate(
            kCFAllocatorDefault,
            K_CF_NUMBER_INT_TYPE,
            &page as *const u32 as *const c_void,
        ));
        let usage_num_ref = CfRef::from_owned(CFNumberCreate(
            kCFAllocatorDefault,
            K_CF_NUMBER_INT_TYPE,
            &usage as *const u32 as *const c_void,
        ));
        let page_key = cf_string(K_IOHID_DEVICE_USAGE_PAGE_KEY);
        let usage_key = cf_string(K_IOHID_DEVICE_USAGE_KEY);

        let (Some(page_num_ref), Some(usage_num_ref)) = (page_num_ref, usage_num_ref) else {
            return None;
        };
        let keys = [page_key.get(), usage_key.get()];
        let vals = [page_num_ref.get(), usage_num_ref.get()];
        CfRef::from_owned(CFDictionaryCreate(
            kCFAllocatorDefault,
            keys.as_ptr(),
            vals.as_ptr(),
            2,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        ))
    }
}

/// Translation of `CreateHIDManager()`. (The state is locked only to store
/// the manager: ConfigHIDManager() runs the run loop, whose callbacks lock
/// it.)
fn create_hid_manager() -> bool {
    let vals = [
        create_hid_device_match_dictionary(K_HID_PAGE_GENERIC_DESKTOP, K_HID_USAGE_GD_JOYSTICK),
        create_hid_device_match_dictionary(K_HID_PAGE_GENERIC_DESKTOP, K_HID_USAGE_GD_GAMEPAD),
        create_hid_device_match_dictionary(
            K_HID_PAGE_GENERIC_DESKTOP,
            K_HID_USAGE_GD_MULTI_AXIS_CONTROLLER,
        ),
    ];
    if vals.iter().any(Option::is_none) {
        return false;
    }
    let ptrs: Vec<*const c_void> = vals.iter().flatten().map(CfRef::get).collect();
    // SAFETY: an array of the dictionaries, which it retains.
    let Some(array) = (unsafe {
        CfRef::from_owned(CFArrayCreate(
            kCFAllocatorDefault,
            ptrs.as_ptr(),
            ptrs.len() as isize,
            &kCFTypeArrayCallBacks,
        ))
    }) else {
        return false;
    };

    // SAFETY: creating a manager.
    let hidman = unsafe { IOHIDManagerCreate(kCFAllocatorDefault, K_IOHID_OPTIONS_TYPE_NONE) };
    if hidman.is_null() {
        return false;
    }
    lock_state().hidman = hidman;
    // SAFETY: the new manager and the matching array.
    unsafe { config_hid_manager(hidman, array.get()) }
}

/// The hat position of a hat switch value (the `DARWIN_JoystickUpdate()`
/// hat switch decoding): 4 and 8 position switches, centered otherwise.
pub(crate) fn hat_position(value: i32, min: i32, max: i32) -> u8 {
    let range = max.wrapping_sub(min).wrapping_add(1);
    let mut value = value.wrapping_sub(min);
    if range == 4 {
        // 4 position hatswitch - scale up value
        value = value.wrapping_mul(2);
    } else if range != 8 {
        // Neither a 4 nor 8 positions - fall back to default position (centered)
        value = -1;
    }
    match value {
        0 => HAT_UP,
        1 => HAT_RIGHTUP,
        2 => HAT_RIGHT,
        3 => HAT_RIGHTDOWN,
        4 => HAT_DOWN,
        5 => HAT_LEFTDOWN,
        6 => HAT_LEFT,
        7 => HAT_LEFTUP,
        /* Every other value is mapped to center. We do that because some
         * joysticks use 8 and some 15 for this value, and apparently
         * there are even more variants out there - so we try to be generous.
         */
        _ => HAT_CENTERED,
    }
}

/// Translation of `DARWIN_JoystickInitRumble()`.
fn init_rumble(device: &mut RecDevice, magnitude: i16) -> Result<()> {
    // SAFETY: the service is the device's force feedback service; the
    // effect is kept alive (boxed) as long as the effect object.
    unsafe {
        if device.ffdevice.is_null() {
            let result = FFCreateDevice(device.ffservice, &mut device.ffdevice);
            if result != FF_OK {
                return Err(Error::new(format!(
                    "Unable to create force feedback device from service: {}",
                    ff_str_error(result)
                )));
            }
        }

        // Reset and then enable actuators
        let result = FFDeviceSendForceFeedbackCommand(device.ffdevice, FFSFFC_RESET);
        if result != FF_OK {
            return Err(Error::new(format!(
                "Unable to reset force feedback device: {}",
                ff_str_error(result)
            )));
        }

        let result = FFDeviceSendForceFeedbackCommand(device.ffdevice, FFSFFC_SETACTUATORSON);
        if result != FF_OK {
            return Err(Error::new(format!(
                "Unable to enable force feedback actuators: {}",
                ff_str_error(result)
            )));
        }

        // Create the effect
        // Note (upstream): an effect left from a failed initialization is
        // leaked when it is replaced; here it is dropped.
        let effect = device.ffeffect.insert(create_rumble_effect_data(magnitude));

        let result = FFDeviceCreateEffect(
            device.ffdevice,
            k_ff_effect_type_sine_id(),
            &mut effect.effect,
            &mut device.ffeffect_ref,
        );
        if result != FF_OK {
            return Err(Error::new(format!(
                "Haptic: Unable to create effect: {}",
                ff_str_error(result)
            )));
        }
    }
    Ok(())
}

/// What `DARWIN_JoystickUpdate()` sends, collected with the state locked.
enum Input {
    Axis(u8, i16),
    Button(u8, bool),
    Hat(u8, u8),
}

/// Translation of `SDL_DARWIN_JoystickDriver`.
pub(crate) struct DarwinJoystickDriver;

pub(crate) static DARWIN_JOYSTICK_DRIVER: DarwinJoystickDriver = DarwinJoystickDriver;

impl JoystickDriver for DarwinJoystickDriver {
    /// Translation of `DARWIN_JoystickInit()`.
    fn init(&self) -> Result<()> {
        if !hints::get_bool(hints::JOYSTICK_IOKIT, true) {
            return Ok(());
        }

        if !create_hid_manager() {
            return Err(Error::new("Joystick: Couldn't initialize HID Manager"));
        }

        Ok(())
    }

    /// Translation of `DARWIN_JoystickGetCount()`.
    fn count(&self) -> usize {
        lock_state().devices.iter().filter(|d| !d.removed).count()
    }

    /// Translation of `DARWIN_JoystickDetect()`.
    fn detect(&self) {
        let hidman = {
            let mut state = lock_state();
            let mut i = 0;
            while i < state.devices.len() {
                if state.devices[i].removed {
                    free_device(&mut state, i);
                } else {
                    i += 1;
                }
            }
            state.hidman
        };

        if !hidman.is_null() {
            /* run this after the checks above so we don't set device->removed and delete the device before
            DARWIN_JoystickUpdate can run to clean up the SDL_Joystick object that owns this device */
            let mode = joystick_runloop_mode();
            // SAFETY: runs this thread's run loop in the joystick mode; the
            // callbacks take the state lock themselves.
            while unsafe { CFRunLoopRunInMode(mode, 0.0, 1) } == K_CF_RUN_LOOP_RUN_HANDLED_SOURCE {
                // no-op. Pending callbacks will fire in CFRunLoopRunInMode().
            }
        }
    }

    /// Translation of `DARWIN_JoystickIsDevicePresent()`.
    fn is_device_present(
        &self,
        _vendor_id: u16,
        _product_id: u16,
        _version: u16,
        _name: Option<&str>,
    ) -> bool {
        // We don't override any other drivers
        false
    }

    /// Translation of `DARWIN_JoystickGetDeviceName()`.
    fn device_name(&self, device_index: usize) -> Option<String> {
        Some(
            lock_state()
                .device_for_index(device_index)
                .map_or_else(|| "UNKNOWN".to_owned(), |d| d.product.clone()),
        )
    }

    /// Translation of `DARWIN_JoystickGetDevicePath()`.
    fn device_path(&self, _device_index: usize) -> Option<String> {
        None
    }

    /// Translation of `DARWIN_JoystickGetDeviceSteamVirtualGamepadSlot()`.
    fn device_steam_virtual_gamepad_slot(&self, device_index: usize) -> i32 {
        lock_state()
            .device_for_index(device_index)
            .map_or(-1, |d| d.steam_virtual_gamepad_slot)
    }

    /// Translation of `DARWIN_JoystickGetDevicePlayerIndex()`.
    fn device_player_index(&self, _device_index: usize) -> i32 {
        -1
    }

    /// Translation of `DARWIN_JoystickSetDevicePlayerIndex()`.
    fn set_device_player_index(&self, _device_index: usize, _player_index: i32) {}

    /// Translation of `DARWIN_JoystickGetDeviceGUID()`.
    fn device_guid(&self, device_index: usize) -> Guid {
        lock_state()
            .device_for_index(device_index)
            .map_or(Guid::ZERO, |d| d.guid)
    }

    /// Translation of `DARWIN_JoystickGetDeviceInstanceID()`.
    fn device_instance_id(&self, device_index: usize) -> JoystickID {
        lock_state()
            .device_for_index(device_index)
            .map_or(0, |d| d.instance_id)
    }

    /// Translation of `DARWIN_JoystickOpen()`.
    fn open(&self, joystick: &mut JoystickData, device_index: usize) -> Result<()> {
        let mut state = lock_state();
        // (upstream dereferences the device without checking it)
        let Some(device) = state.device_for_index(device_index) else {
            return Err(Error::new("No such device"));
        };

        device.joystick = Some(joystick.instance_id);

        joystick.naxes = device.axes;
        joystick.nhats = device.hats;
        joystick.nbuttons = device.buttons;

        let rumble = device.ffservice != 0;
        let instance_id = device.instance_id;
        state.open.retain(|&j| j != joystick.instance_id);
        state.open.push(instance_id);
        drop(state);

        if rumble {
            let _ = joystick
                .properties()
                .set(PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN, true);
        }

        Ok(())
    }

    /// Translation of `DARWIN_JoystickRumble()`.
    fn rumble(
        &self,
        joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        let mut state = lock_state();
        let device = state.hwdata(joystick);

        // Scale and average the two rumble strengths
        let magnitude = (((i32::from(low_frequency_rumble) / 2)
            + (i32::from(high_frequency_rumble) / 2))
            / 2) as i16;

        let Some(device) = device else {
            return Err(Error::new("Rumble failed, device disconnected"));
        };

        if device.ffservice == 0 {
            return Err(Error::unsupported());
        }

        // SAFETY: the effect object and its definition are the device's.
        unsafe {
            if device.ff_initialized {
                let effect = device.ffeffect.as_mut().expect("an initialized effect");
                effect.periodic_mut().dw_magnitude = convert_magnitude(magnitude);

                let result = FFEffectSetParameters(
                    device.ffeffect_ref,
                    &mut effect.effect,
                    FFEP_DURATION | FFEP_TYPESPECIFICPARAMS,
                );
                if result != FF_OK {
                    return Err(Error::new(format!(
                        "Unable to update rumble effect: {}",
                        ff_str_error(result)
                    )));
                }
            } else {
                init_rumble(device, magnitude)?;
                device.ff_initialized = true;
            }

            let result = FFEffectStart(device.ffeffect_ref, 1, 0);
            if result != FF_OK {
                return Err(Error::new(format!(
                    "Unable to run the rumble effect: {}",
                    ff_str_error(result)
                )));
            }
        }
        Ok(())
    }

    /// Translation of `DARWIN_JoystickRumbleTriggers()`.
    fn rumble_triggers(
        &self,
        _joystick: JoystickID,
        _left_rumble: u16,
        _right_rumble: u16,
    ) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `DARWIN_JoystickSetLED()`.
    fn set_led(&self, _joystick: JoystickID, _red: u8, _green: u8, _blue: u8) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `DARWIN_JoystickSendEffect()`.
    fn send_effect(&self, _joystick: JoystickID, _data: &[u8]) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `DARWIN_JoystickSetSensorsEnabled()`.
    fn set_sensors_enabled(&self, _joystick: JoystickID, _enabled: bool) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `DARWIN_JoystickUpdate()`.
    fn update(&self, joystick: JoystickID) {
        let timestamp = crate::timer::ticks_ns();
        let mut inputs = Vec::new();
        {
            let mut state = lock_state();
            let Some(device) = state.hwdata(joystick) else {
                return;
            };

            if device.removed {
                // device was unplugged; ignore it.
                state.clear_hwdata(joystick);
                return;
            }

            let device_ref = device.device_ref;
            let nacon = device.nacon_revolution_x_unlimited;
            for (i, element) in device.first_axis.iter_mut().enumerate() {
                let good_read = if nacon {
                    get_hid_scaled_calibrated_state_nacon_revolution_x_unlimited(
                        device_ref, element, -32768, 32767,
                    )
                } else {
                    get_hid_scaled_calibrated_state(device_ref, element, -32768, 32767)
                };
                if let Some(value) = good_read {
                    inputs.push(Input::Axis(i as u8, value as i16));
                }
            }

            for (i, element) in device.first_button.iter_mut().enumerate() {
                if let Some(value) = get_hid_element_state(device_ref, element) {
                    inputs.push(Input::Button(i as u8, value != 0));
                }
            }

            for (i, element) in device.first_hat.iter_mut().enumerate() {
                if let Some(value) = get_hid_element_state(device_ref, element) {
                    inputs.push(Input::Hat(
                        i as u8,
                        hat_position(value, element.min, element.max),
                    ));
                }
            }
        }

        // (sent with the driver state unlocked: event watchers may call
        // back into the joystick API)
        for input in inputs {
            match input {
                Input::Axis(i, value) => send_joystick_axis(timestamp, joystick, i, value),
                Input::Button(i, down) => send_joystick_button(timestamp, joystick, i, down),
                Input::Hat(i, pos) => send_joystick_hat(timestamp, joystick, i, pos),
            }
        }
    }

    /// Translation of `DARWIN_JoystickClose()`.
    fn close(&self, joystick: &mut JoystickData) {
        let mut state = lock_state();
        if let Some(device) = state.hwdata(joystick.instance_id) {
            device.joystick = None;
        }
        state.clear_hwdata(joystick.instance_id);
    }

    /// Translation of `DARWIN_JoystickQuit()`.
    fn quit(&self) {
        let mut state = lock_state();
        while !state.devices.is_empty() {
            free_device(&mut state, 0);
        }

        if !state.hidman.is_null() {
            let mode = joystick_runloop_mode();
            // SAFETY: the manager we created, released once.
            unsafe {
                IOHIDManagerUnscheduleFromRunLoop(state.hidman, CFRunLoopGetCurrent(), mode);
                IOHIDManagerClose(state.hidman, K_IOHID_OPTIONS_TYPE_NONE);
                CFRelease(state.hidman);
            }
            state.hidman = std::ptr::null_mut();
        }
        state.open.clear();
    }

    /// Translation of `DARWIN_JoystickGetGamepadMapping()`.
    fn gamepad_mapping(&self, _device_index: usize) -> Option<super::gamepad::GamepadMapping> {
        None
    }
}

/// Whether an open joystick is one of this driver's with a force feedback
/// service, and that service (the `joystick->hwdata->ffservice` the
/// Darwin haptic driver reads).
pub(crate) fn joystick_ffservice(joystick: JoystickID) -> Option<IoService> {
    assert_joysticks_locked();
    lock_state().hwdata(joystick).map(|d| d.ffservice)
}
