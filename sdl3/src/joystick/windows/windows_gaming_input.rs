// Rust translation of src/joystick/windows/SDL_windows_gaming_input.c from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Windows.Gaming.Input joystick driver (`SDL_WGI_JoystickDriver`):
//! the WinRT raw game controllers, found through the
//! `RawGameControllerAdded`/`Removed` events (WinRT calls them on its own
//! threads), with their buttons, switches and axes, the gamepad vibration
//! of the controllers that are gamepads, and battery reports. The WinRT
//! interfaces are declared by hand in [`wgi_abi`](super::wgi_abi) and the
//! WinRT functions loaded from `combase.dll` at run time.
//!
//! The driver is off unless the `SDL_JOYSTICK_WGI` hint is set; it leaves
//! the XInput-capable devices to the XInput and RawInput drivers.

use std::cell::RefCell;
use std::ffi::{c_void, CString};

use windows_sys::core::{GUID, HRESULT};
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_Device_IDA, CM_Get_Parent, CM_Locate_DevNodeA, CM_LOCATE_DEVNODE_NORMAL, CR_SUCCESS,
    MAX_DEVICE_ID_LEN,
};
use windows_sys::Win32::Foundation::{E_POINTER, S_OK};
use windows_sys::Win32::UI::Input::{
    GetRawInputDeviceInfoA, GetRawInputDeviceList, RAWINPUTDEVICELIST, RIDI_DEVICEINFO,
    RIDI_DEVICENAME, RID_DEVICE_INFO, RIM_TYPEHID,
};

use super::super::gamepad::GamepadMapping;
use super::super::usb_ids::{
    USB_PRODUCT_STEAM_VIRTUAL_GAMEPAD, USB_PRODUCT_XBOX360_XUSB_CONTROLLER, USB_VENDOR_MICROSOFT,
    USB_VENDOR_VALVE,
};
use super::super::{
    create_joystick_guid, is_joystick_xbox_one, joystick_handled_by_another_driver,
    joysticks_initialized, joysticks_quitting, lock_joysticks, private_joystick_added,
    private_joystick_force_recentering, private_joystick_removed, send_joystick_axis,
    send_joystick_button, send_joystick_hat, send_joystick_power_info, should_ignore_joystick,
    JoystickConnectionState, JoystickData, JoystickDriver, JoystickType, HARDWARE_BUS_BLUETOOTH,
    HARDWARE_BUS_USB, HAT_CENTERED, HAT_DOWN, HAT_LEFT, HAT_LEFTDOWN, HAT_LEFTUP, HAT_RIGHT,
    HAT_RIGHTDOWN, HAT_RIGHTUP, HAT_UP, PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN,
    PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN, WGI_DRIVER_INDEX,
};
use super::wgi_abi::*;
use crate::core::windows::com::ComPtr;
use crate::core::windows::{error_from_hresult, ro_initialize, ro_uninitialize, set_error};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::guid::Guid;
use crate::hints;
use crate::power::PowerState;
use crate::thread::ReentrantMutex;

/// The private structure used to keep track of a joystick. Translation of
/// `struct joystick_hwdata`.
struct HwData {
    /// The joystick this belongs to
    instance_id: JoystickID,
    controller: ComPtr<IRawGameControllerVtbl>,
    #[allow(dead_code)] // (kept like upstream, released on close)
    game_controller: Option<ComPtr<IGameControllerVtbl>>,
    battery: Option<ComPtr<IGameControllerBatteryInfoVtbl>>,
    gamepad: Option<ComPtr<IGamepadVtbl>>,
    vibration: GamepadVibration,
    timestamp: u64,
}

/// A controller. Translation of `WindowsGamingInputControllerState`.
struct ControllerState {
    instance_id: JoystickID,
    controller: ComPtr<IRawGameControllerVtbl>,
    name: String,
    guid: Guid,
    #[allow(dead_code)] // (kept like upstream; only the GUID uses it)
    joystick_type: JoystickType,
    steam_virtual_gamepad_slot: i32,
}

/// The driver's state. Translation of `wgi` (its function pointers are
/// [`winrt()`]).
#[derive(Default)]
struct Wgi {
    ro_initialized: bool,
    controller_statics: Option<ComPtr<IRawGameControllerStaticsVtbl>>,
    arcade_stick_statics: Option<ComPtr<IInspectableVtbl>>,
    arcade_stick_statics2: Option<ComPtr<IFromGameControllerVtbl>>,
    flight_stick_statics: Option<ComPtr<IFlightStickStaticsVtbl>>,
    gamepad_statics: Option<ComPtr<IInspectableVtbl>>,
    gamepad_statics2: Option<ComPtr<IFromGameControllerVtbl>>,
    racing_wheel_statics: Option<ComPtr<IInspectableVtbl>>,
    racing_wheel_statics2: Option<ComPtr<IFromGameControllerVtbl>>,
    controller_added_token: EventRegistrationToken,
    controller_removed_token: EventRegistrationToken,
    /// (`controller_count` is its length)
    controllers: Vec<ControllerState>,
    /// The open joysticks' `joystick->hwdata`.
    open: Vec<HwData>,
}

/// Guarded by the joystick lock upstream; the `RefCell` borrow is never
/// held across an event push or a call back into the joystick API.
static WGI: ReentrantMutex<RefCell<Option<Wgi>>> = ReentrantMutex::new(RefCell::new(None));

fn with_wgi<R>(f: impl FnOnce(&mut Wgi) -> R) -> R {
    let guard = WGI.lock();
    let mut wgi = guard.borrow_mut();
    f(wgi.get_or_insert_with(Wgi::default))
}

/// The device instance ID of a raw input device interface path, as
/// `CM_Locate_DevNode()` takes it, if the path is something we know how
/// to parse (the transformation in `SDL_IsXInputDevice()`).
///
/// Example: `\\?\HID#VID_045E&PID_02FF&IG_00#9&2c203035&2&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}`
/// gives `HID\VID_045E&PID_02FF&IG_00\9&2c203035&2&0000`.
pub(super) fn device_instance_id(dev_name: &str) -> Option<String> {
    // Make sure the device interface string is something we know how to parse
    if !dev_name.starts_with("\\\\?\\") || !dev_name.contains("#{") {
        return None;
    }

    // Unescape the backslashes in the string and terminate before the GUID portion
    let mut unescaped = String::with_capacity(dev_name.len());
    let mut chars = dev_name.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '#' {
            if chars.peek() == Some(&'{') {
                break;
            }
            unescaped.push('\\');
        } else {
            unescaped.push(c);
        }
    }

    /* We'll be left with a string like this: \\?\HID\VID_045E&PID_02FF&IG_00\9&2c203035&2&0000
     * Simply skip the \\?\ prefix and we'll have a properly formed device instance ID */
    Some(unescaped[4..].to_string())
}

/// Whether the XInput or RawInput backends will pick up the device.
/// Translation of `SDL_IsXInputDevice()`.
fn is_xinput_device(vendor: u16, product: u16, name: Option<&str>) -> bool {
    let vidpid = (vendor as u32) | ((product as u32) << 16);

    // XInput and RawInput backends will pick up XInput-compatible devices
    if !super::xinput::xinput_enabled() && !super::rawinput::is_enabled() {
        return false;
    }

    // Sometimes we'll get a Windows.Gaming.Input callback before the raw input device is even in the list,
    // so try to do some checks up front to catch these cases.
    if is_joystick_xbox_one(vendor, product) || name.is_some_and(|name| name.starts_with("Xbox ")) {
        return true;
    }

    // Go through RAWINPUT (WinXP and later) to find HID devices.
    let mut raw_device_count = 0u32;
    // SAFETY: a NULL list asks for the count.
    let result = unsafe {
        GetRawInputDeviceList(
            std::ptr::null_mut(),
            &mut raw_device_count,
            size_of::<RAWINPUTDEVICELIST>() as u32,
        )
    };
    if result == u32::MAX || raw_device_count == 0 {
        return false; // oh well.
    }

    // SAFETY: RAWINPUTDEVICELIST is plain data; all zeros is valid.
    let mut raw_devices =
        vec![unsafe { std::mem::zeroed::<RAWINPUTDEVICELIST>() }; raw_device_count as usize];
    // SAFETY: the list holds raw_device_count entries.
    let raw_device_count = unsafe {
        GetRawInputDeviceList(
            raw_devices.as_mut_ptr(),
            &mut raw_device_count,
            size_of::<RAWINPUTDEVICELIST>() as u32,
        )
    };
    if raw_device_count == u32::MAX {
        return false; // oh well.
    }

    for raw_device in raw_devices.iter().take(raw_device_count as usize) {
        // SAFETY: RID_DEVICE_INFO is plain data; all zeros is valid.
        let mut rdi: RID_DEVICE_INFO = unsafe { std::mem::zeroed() };
        let mut dev_name = [0u8; 260];
        let mut rdi_size = size_of::<RID_DEVICE_INFO>() as u32;
        let mut name_size = dev_name.len() as u32;

        rdi.cbSize = size_of::<RID_DEVICE_INFO>() as u32;

        if raw_device.dwType != RIM_TYPEHID {
            continue; // Skip non-XInput devices
        }
        // SAFETY: rdi holds rdi_size bytes.
        if unsafe {
            GetRawInputDeviceInfoA(
                raw_device.hDevice,
                RIDI_DEVICEINFO,
                (&mut rdi as *mut RID_DEVICE_INFO).cast(),
                &mut rdi_size,
            )
        } == u32::MAX
        {
            continue;
        }
        // SAFETY: dev_name holds name_size bytes.
        if unsafe {
            GetRawInputDeviceInfoA(
                raw_device.hDevice,
                RIDI_DEVICENAME,
                dev_name.as_mut_ptr().cast(),
                &mut name_size,
            )
        } == u32::MAX
        {
            continue;
        }
        let len = dev_name
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(dev_name.len());
        let dev_name = String::from_utf8_lossy(&dev_name[..len]).into_owned();
        if !dev_name.contains("IG_") {
            // Skip non-XInput devices
            continue;
        }

        // SAFETY: for a HID device the union holds the hid member.
        let hid_info = unsafe { rdi.Anonymous.hid };

        // First check for a simple VID/PID match. This will work for Xbox 360 controllers.
        if (hid_info.dwVendorId & 0xFFFF) | ((hid_info.dwProductId & 0xFFFF) << 16) == vidpid {
            return true;
        }

        /* For Xbox One controllers, Microsoft doesn't propagate the VID/PID down to the HID stack.
         * We'll have to walk the device tree upwards searching for a match for our VID/PID. */
        let Some(instance_id) = device_instance_id(&dev_name) else {
            continue;
        };
        let Ok(instance_id) = CString::new(instance_id) else {
            continue;
        };

        let mut dev_node = 0u32;
        // SAFETY: the ID is NUL-terminated; dev_node is writable.
        if unsafe {
            CM_Locate_DevNodeA(
                &mut dev_node,
                instance_id.as_ptr().cast(),
                CM_LOCATE_DEVNODE_NORMAL,
            )
        } != CR_SUCCESS
        {
            continue;
        }

        let dev_vid_pid_string = format!("VID_{vendor:04X}&PID_{product:04X}");

        // SAFETY: dev_node is writable and names a device node.
        while unsafe { CM_Get_Parent(&mut dev_node, dev_node, 0) } == CR_SUCCESS {
            let mut device_id = [0u8; MAX_DEVICE_ID_LEN as usize];

            // SAFETY: the buffer holds the given length.
            if unsafe {
                CM_Get_Device_IDA(dev_node, device_id.as_mut_ptr(), device_id.len() as u32, 0)
            } == CR_SUCCESS
            {
                let len = device_id
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(device_id.len());
                if String::from_utf8_lossy(&device_id[..len]).contains(&dev_vid_pid_string) {
                    // The VID/PID matched a parent device
                    return true;
                }
            }
        }
    }

    false
}

/// The statics of a runtime class, or `None` (upstream sets an error
/// nobody reads).
fn load_statics<V>(class_name: &str, iid: &GUID) -> Option<ComPtr<V>> {
    winrt().get_activation_factory(class_name, iid).ok()
}

/// Translation of `WGI_LoadRawGameControllerStatics()`.
fn load_raw_game_controller_statics(wgi: &mut Wgi) {
    // ("Couldn't find Windows.Gaming.Input.IRawGameControllerStatics" on failure)
    wgi.controller_statics = load_statics(
        RUNTIME_CLASS_RAW_GAME_CONTROLLER,
        &IID_IRAWGAMECONTROLLERSTATICS,
    );
}

/// Translation of `WGI_LoadOtherControllerStatics()`.
fn load_other_controller_statics(wgi: &mut Wgi) {
    if wgi.arcade_stick_statics.is_none() {
        wgi.arcade_stick_statics =
            load_statics(RUNTIME_CLASS_ARCADE_STICK, &IID_IARCADESTICKSTATICS);
        if let Some(statics) = &wgi.arcade_stick_statics {
            wgi.arcade_stick_statics2 = statics.query(&IID_IARCADESTICKSTATICS2).ok();
        }
    }

    if wgi.flight_stick_statics.is_none() {
        wgi.flight_stick_statics =
            load_statics(RUNTIME_CLASS_FLIGHT_STICK, &IID_IFLIGHTSTICKSTATICS);
    }

    if wgi.gamepad_statics.is_none() {
        wgi.gamepad_statics = load_statics(RUNTIME_CLASS_GAMEPAD, &IID_IGAMEPADSTATICS);
        if let Some(statics) = &wgi.gamepad_statics {
            wgi.gamepad_statics2 = statics.query(&IID_IGAMEPADSTATICS2).ok();
        }
    }

    if wgi.racing_wheel_statics.is_none() {
        wgi.racing_wheel_statics =
            load_statics(RUNTIME_CLASS_RACING_WHEEL, &IID_IRACINGWHEELSTATICS);
        if let Some(statics) = &wgi.racing_wheel_statics {
            wgi.racing_wheel_statics2 = statics.query(&IID_IRACINGWHEELSTATICS2).ok();
        }
    }
}

/// `FromGameController()` on a statics interface: whether it gave an
/// object (released again).
fn from_game_controller(
    statics: &ComPtr<IFromGameControllerVtbl>,
    game_controller: &ComPtr<IGameControllerVtbl>,
) -> bool {
    // SAFETY: FromGameController stores an object reference (or NULL) on
    // success.
    unsafe {
        ComPtr::<IInspectableVtbl>::from_out(|out| {
            (statics.vtbl().FromGameController)(statics.as_ptr(), game_controller.as_ptr(), out)
        })
    }
    .is_ok()
}

/// Translation of `GetGameControllerType()`.
fn get_game_controller_type(
    wgi: &mut Wgi,
    game_controller: &ComPtr<IGameControllerVtbl>,
) -> JoystickType {
    /* Wait to initialize these interfaces until we need them.
     * Initializing the gamepad interface will switch Bluetooth PS4 controllers into enhanced mode, breaking DirectInput
     */
    load_other_controller_statics(wgi);

    if wgi
        .gamepad_statics2
        .as_ref()
        .is_some_and(|s| from_game_controller(s, game_controller))
    {
        return JoystickType::Gamepad;
    }

    if wgi
        .arcade_stick_statics2
        .as_ref()
        .is_some_and(|s| from_game_controller(s, game_controller))
    {
        return JoystickType::ArcadeStick;
    }

    if let Some(statics) = &wgi.flight_stick_statics {
        // SAFETY: as in from_game_controller().
        let flight_stick = unsafe {
            ComPtr::<IInspectableVtbl>::from_out(|out| {
                (statics.vtbl().FromGameController)(statics.as_ptr(), game_controller.as_ptr(), out)
            })
        };
        if flight_stick.is_ok() {
            return JoystickType::FlightStick;
        }
    }

    if wgi
        .racing_wheel_statics2
        .as_ref()
        .is_some_and(|s| from_game_controller(s, game_controller))
    {
        return JoystickType::Wheel;
    }

    JoystickType::Unknown
}

/// The Steam virtual gamepad slot in a controller's non-roamable ID, or -1.
/// Translation of `GetSteamVirtualGamepadSlot()`.
fn get_steam_virtual_gamepad_slot(
    controller: &ComPtr<IRawGameControllerVtbl>,
    vendor_id: u16,
    product_id: u16,
) -> i32 {
    let mut slot = -1;

    if vendor_id == USB_VENDOR_VALVE && product_id == USB_PRODUCT_STEAM_VIRTUAL_GAMEPAD {
        if let Ok(controller2) =
            controller.query::<IRawGameController2Vtbl>(&IID_IRAWGAMECONTROLLER2)
        {
            let mut h_string: HSTRING = std::ptr::null_mut();
            // SAFETY: get_NonRoamableId stores an owned HSTRING on success.
            let hr = unsafe {
                (controller2.vtbl().get_NonRoamableId)(controller2.as_ptr(), &mut h_string)
            };
            if hr >= 0 {
                // SAFETY: the string is ours to delete.
                if let Some(id) = unsafe { winrt().take_string(h_string) } {
                    slot = steam_virtual_gamepad_slot_of_id(&id);
                }
            }
        }
    }
    slot
}

/// The `SDL_sscanf()` of `GetSteamVirtualGamepadSlot()`: the slot in a
/// non-roamable ID, or -1.
pub(super) fn steam_virtual_gamepad_slot_of_id(id: &str) -> i32 {
    use super::Scan::*;
    super::scan_int(
        id,
        &[
            Lit("{wgi/nrid/:steam-"),
            SkipHex,
            Lit("&"),
            SkipHex,
            Lit("&"),
            SkipHex,
            Lit("#"),
            Int,
            Lit("#"),
            SkipUnsigned,
            Lit("}"),
        ],
    )
    .unwrap_or(-1)
}

/// Translation of `IEventHandler_CRawGameControllerVtbl_InvokeAdded()`.
fn invoke_added(e: &ComPtr<IRawGameControllerVtbl>) -> HRESULT {
    let _lock = lock_joysticks();

    // We can get delayed calls to InvokeAdded() after WGI_JoystickQuit()
    if joysticks_quitting() || !joysticks_initialized() {
        return S_OK;
    }

    let Ok(controller) = e.query::<IRawGameControllerVtbl>(&IID_IRAWGAMECONTROLLER) else {
        return S_OK;
    };
    let mut bus = HARDWARE_BUS_USB;
    let mut vendor = 0u16;
    let mut product = 0u16;
    let version = 0u16;
    let mut joystick_type = JoystickType::Unknown;

    // SAFETY: the getters write the IDs.
    unsafe {
        (controller.vtbl().get_HardwareVendorId)(controller.as_ptr(), &mut vendor);
        (controller.vtbl().get_HardwareProductId)(controller.as_ptr(), &mut product);
    }

    let game_controller = controller
        .query::<IGameControllerVtbl>(&IID_IGAMECONTROLLER)
        .ok();
    if let Some(game_controller) = &game_controller {
        let mut wireless = 0u8;
        // SAFETY: get_IsWireless writes the flag.
        let hr = unsafe {
            (game_controller.vtbl().get_IsWireless)(game_controller.as_ptr(), &mut wireless)
        };
        if hr >= 0 && wireless != 0 {
            bus = HARDWARE_BUS_BLUETOOTH;

            // Fixup for Wireless Xbox 360 Controller
            if product == 0 {
                vendor = USB_VENDOR_MICROSOFT;
                product = USB_PRODUCT_XBOX360_XUSB_CONTROLLER;
            }
        }
        // FIXME (upstream): game_controller is released here, and used
        // below for the controller type; here it's kept until then.
    }

    let mut name = None;
    if let Ok(controller2) = controller.query::<IRawGameController2Vtbl>(&IID_IRAWGAMECONTROLLER2) {
        let mut h_string: HSTRING = std::ptr::null_mut();
        // SAFETY: get_DisplayName stores an owned HSTRING on success.
        let hr =
            unsafe { (controller2.vtbl().get_DisplayName)(controller2.as_ptr(), &mut h_string) };
        if hr >= 0 {
            // SAFETY: the string is ours to delete.
            name = unsafe { winrt().take_string(h_string) };
        }
    }
    let name = name.unwrap_or_default();

    let ignore_joystick = should_ignore_joystick(vendor, product, version, Some(&name))
        || joystick_handled_by_another_driver(WGI_DRIVER_INDEX, vendor, product, version, Some(&name))
        // This hasn't been detected by the RAWINPUT driver yet, but it will be picked up later.
        || is_xinput_device(vendor, product, Some(&name));

    if !ignore_joystick {
        // New device, add it
        let joystick_id = crate::utils::next_object_id();

        with_wgi(|wgi| {
            if let Some(game_controller) = &game_controller {
                joystick_type = get_game_controller_type(wgi, game_controller);
            }

            let steam_virtual_gamepad_slot =
                get_steam_virtual_gamepad_slot(&controller, vendor, product);
            wgi.controllers.push(ControllerState {
                instance_id: joystick_id,
                guid: create_joystick_guid(
                    bus,
                    vendor,
                    product,
                    version,
                    None,
                    Some(&name),
                    b'w',
                    joystick_type as u8,
                ),
                name,
                joystick_type,
                steam_virtual_gamepad_slot,
                controller: controller.clone(),
            });
        });

        private_joystick_added(joystick_id);
    }

    S_OK
}

/// Translation of `IEventHandler_CRawGameControllerVtbl_InvokeRemoved()`.
fn invoke_removed(e: &ComPtr<IRawGameControllerVtbl>) -> HRESULT {
    let _lock = lock_joysticks();

    // Can we get delayed calls to InvokeRemoved() after WGI_JoystickQuit()?
    if !joysticks_initialized() {
        return S_OK;
    }

    let Ok(controller) = e.query::<IRawGameControllerVtbl>(&IID_IRAWGAMECONTROLLER) else {
        return S_OK;
    };

    let removed = with_wgi(|wgi| {
        let i = wgi
            .controllers
            .iter()
            .position(|c| c.controller.same(&controller))?;
        // (the state's reference to the controller is released with it)
        Some(wgi.controllers.remove(i).instance_id)
    });
    if let Some(joystick_id) = removed {
        private_joystick_removed(joystick_id);
    }

    S_OK
}

/// `IEventHandler_CRawGameControllerVtbl_QueryInterface()`
unsafe extern "system" fn controller_query_interface(
    this: *mut IEventHandlerRawGameController,
    riid: *const GUID,
    ppv_object: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: WinRT calls this on our delegate with valid arguments.
    unsafe {
        delegate_query_interface(this, riid, ppv_object, &IID_IEVENTHANDLER_RAWGAMECONTROLLER)
    }
}

/// A new reference to an event's controller (`None` for NULL).
///
/// # Safety
///
/// `e` must be NULL or a live raw game controller.
unsafe fn event_controller(e: *mut IRawGameController) -> Option<ComPtr<IRawGameControllerVtbl>> {
    if e.is_null() {
        return None;
    }
    // SAFETY: we take a reference of our own (AddRef) to the borrowed one.
    let borrowed = std::mem::ManuallyDrop::new(unsafe { ComPtr::from_raw(e) }?);
    Some((*borrowed).clone())
}

/// The added event: `IEventHandler_CRawGameControllerVtbl_InvokeAdded()`.
unsafe extern "system" fn controller_invoke_added(
    _this: *mut IEventHandlerRawGameController,
    _sender: *mut IInspectable,
    e: *mut IRawGameController,
) -> HRESULT {
    // SAFETY: WinRT passes the added controller.
    match unsafe { event_controller(e) } {
        Some(e) => invoke_added(&e),
        None => E_POINTER,
    }
}

/// The removed event: `IEventHandler_CRawGameControllerVtbl_InvokeRemoved()`.
unsafe extern "system" fn controller_invoke_removed(
    _this: *mut IEventHandlerRawGameController,
    _sender: *mut IInspectable,
    e: *mut IRawGameController,
) -> HRESULT {
    // SAFETY: WinRT passes the removed controller.
    match unsafe { event_controller(e) } {
        Some(e) => invoke_removed(&e),
        None => E_POINTER,
    }
}

static CONTROLLER_ADDED_VTBL: IEventHandlerVtbl<IRawGameControllerVtbl> = IEventHandlerVtbl {
    QueryInterface: controller_query_interface,
    AddRef: delegate_add_ref::<IRawGameControllerVtbl>,
    Release: delegate_release::<IRawGameControllerVtbl>,
    Invoke: controller_invoke_added,
};
static CONTROLLER_ADDED: Delegate<IRawGameControllerVtbl> = Delegate::new(&CONTROLLER_ADDED_VTBL);

static CONTROLLER_REMOVED_VTBL: IEventHandlerVtbl<IRawGameControllerVtbl> = IEventHandlerVtbl {
    QueryInterface: controller_query_interface,
    AddRef: delegate_add_ref::<IRawGameControllerVtbl>,
    Release: delegate_release::<IRawGameControllerVtbl>,
    Invoke: controller_invoke_removed,
};
static CONTROLLER_REMOVED: Delegate<IRawGameControllerVtbl> =
    Delegate::new(&CONTROLLER_REMOVED_VTBL);

/// `CO_MTA_USAGE_COOKIE` of `WGI_JoystickInit()` (never released).
static MTA_COOKIE: std::sync::Mutex<usize> = std::sync::Mutex::new(0);

/// The hat position of a switch position. Translation of `ConvertHatValue()`.
pub(super) fn convert_hat_value(value: i32) -> u8 {
    match value {
        switch_position::UP => HAT_UP,
        switch_position::UP_RIGHT => HAT_RIGHTUP,
        switch_position::RIGHT => HAT_RIGHT,
        switch_position::DOWN_RIGHT => HAT_RIGHTDOWN,
        switch_position::DOWN => HAT_DOWN,
        switch_position::DOWN_LEFT => HAT_LEFTDOWN,
        switch_position::LEFT => HAT_LEFT,
        switch_position::UP_LEFT => HAT_LEFTUP,
        _ => HAT_CENTERED,
    }
}

/// An axis value of a reading. (The conversion of `WGI_JoystickUpdate()`.)
pub(super) fn convert_axis_value(value: f64) -> i16 {
    ((value * 65535.0) as i32 - 32768) as i16
}

/// The power state of a battery status. (The switch of
/// `WGI_JoystickUpdate()`.)
pub(super) fn power_state_of_battery_status(status: i32) -> PowerState {
    match status {
        battery_status::NOT_PRESENT => PowerState::NoBattery,
        battery_status::DISCHARGING => PowerState::OnBattery,
        battery_status::IDLE => PowerState::Charged,
        battery_status::CHARGING => PowerState::Charging,
        _ => PowerState::Unknown,
    }
}

/// The battery percentage of a report's capacities. (The rounding of
/// `WGI_JoystickUpdate()`.)
pub(super) fn battery_percent(full_capacity: i32, curr_capacity: i32) -> i32 {
    if full_capacity > 0 {
        ((curr_capacity as f32 / full_capacity as f32) * 100.0).round() as i32
    } else {
        0
    }
}

/// An `IReference<int>` battery capacity.
fn capacity(
    report: &ComPtr<IBatteryReportVtbl>,
    getter: unsafe extern "system" fn(*mut IBatteryReport, *mut *mut IReferenceInt32) -> HRESULT,
) -> i32 {
    let mut value = 0;
    // SAFETY: the getter stores an IReference<int> on success.
    if let Ok(reference) =
        unsafe { ComPtr::<IReferenceInt32Vtbl>::from_out(|out| getter(report.as_ptr(), out)) }
    {
        // SAFETY: get_Value writes the value.
        unsafe {
            (reference.vtbl().get_Value)(reference.as_ptr(), &mut value);
        }
    }
    value
}

/// Translation of `SDL_WGI_JoystickDriver`.
pub(in crate::joystick) struct WgiJoystickDriver;

pub(in crate::joystick) static WGI_JOYSTICK_DRIVER: WgiJoystickDriver = WgiJoystickDriver;

impl JoystickDriver for WgiJoystickDriver {
    /// Translation of `WGI_JoystickInit()`.
    fn init(&self) -> Result<()> {
        // (SDL_UsingGameInputForXInputControllers() is false without GameInput)
        if !hints::get_bool(hints::JOYSTICK_WGI, false) {
            return Ok(());
        }

        if ro_initialize() < 0 {
            return Err(Error::new("RoInitialize() failed"));
        }
        with_wgi(|wgi| wgi.ro_initialized = true);

        let winrt = winrt();
        if let Err(name) = winrt.resolve_all() {
            return Err(set_error(&format!("GetProcAddress failed for {name}")));
        }

        {
            /* There seems to be a bug in Windows where a dependency of WGI can be unloaded from memory prior to WGI itself.
             * This results in Windows_Gaming_Input!GameController::~GameController() invoking an unloaded DLL and crashing.
             * As a workaround, we will keep a reference to the MTA to prevent COM from unloading DLLs later.
             * See https://github.com/libsdl-org/SDL/issues/5552 for more details.
             */
            let mut cookie = MTA_COOKIE.lock().unwrap_or_else(|e| e.into_inner());
            if *cookie == 0 {
                if let Some(co_increment_mta_usage) = winrt.co_increment_mta_usage {
                    let mut handle: *mut c_void = std::ptr::null_mut();
                    // SAFETY: the cookie is writable.
                    let hr = unsafe { co_increment_mta_usage(&mut handle) };
                    if hr < 0 {
                        return Err(error_from_hresult(Some("CoIncrementMTAUsage() failed"), hr));
                    }
                    *cookie = handle as usize;
                }
            }
        }

        let statics = with_wgi(|wgi| {
            load_raw_game_controller_statics(wgi);
            wgi.controller_statics.clone()
        });

        if let Some(statics) = statics {
            let mut token = EventRegistrationToken::default();
            // (a failure sets "Windows.Gaming.Input.IRawGameControllerStatics.add_RawGameControllerAdded failed"
            // and so on upstream, which nobody reads)
            // SAFETY: the delegates are static COM objects.
            unsafe {
                (statics.vtbl().add_RawGameControllerAdded)(
                    statics.as_ptr(),
                    CONTROLLER_ADDED.iface(),
                    &mut token,
                );
            }
            with_wgi(|wgi| wgi.controller_added_token = token);

            // SAFETY: as above.
            unsafe {
                (statics.vtbl().add_RawGameControllerRemoved)(
                    statics.as_ptr(),
                    CONTROLLER_REMOVED.iface(),
                    &mut token,
                );
            }
            with_wgi(|wgi| wgi.controller_removed_token = token);

            // SAFETY: get_RawGameControllers stores a vector view on success.
            let controllers = unsafe {
                ComPtr::from_out(|out| {
                    (statics.vtbl().get_RawGameControllers)(statics.as_ptr(), out)
                })
            };
            if let Ok(controllers) = controllers {
                let mut count = 0;
                // SAFETY: get_Size writes the count.
                let hr = unsafe { (controllers.vtbl().get_Size)(controllers.as_ptr(), &mut count) };
                if hr >= 0 {
                    for i in 0..count {
                        // SAFETY: GetAt stores the controller on success.
                        let controller = unsafe {
                            ComPtr::from_out(|out| {
                                (controllers.vtbl().GetAt)(controllers.as_ptr(), i, out)
                            })
                        };
                        if let Ok(controller) = controller {
                            invoke_added(&controller);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Translation of `WGI_JoystickGetCount()`.
    fn count(&self) -> usize {
        with_wgi(|wgi| wgi.controllers.len())
    }

    /// Translation of `WGI_JoystickDetect()`.
    fn detect(&self) {}

    /// Translation of `WGI_JoystickIsDevicePresent()`.
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

    /// Translation of `WGI_JoystickGetDeviceName()`.
    fn device_name(&self, device_index: usize) -> Option<String> {
        with_wgi(|wgi| wgi.controllers.get(device_index).map(|c| c.name.clone()))
    }

    /// Translation of `WGI_JoystickGetDevicePath()`.
    fn device_path(&self, _device_index: usize) -> Option<String> {
        None
    }

    /// Translation of `WGI_JoystickGetDeviceSteamVirtualGamepadSlot()`.
    fn device_steam_virtual_gamepad_slot(&self, device_index: usize) -> i32 {
        with_wgi(|wgi| {
            wgi.controllers
                .get(device_index)
                .map_or(-1, |c| c.steam_virtual_gamepad_slot)
        })
    }

    /// Translation of `WGI_JoystickGetDevicePlayerIndex()`.
    fn device_player_index(&self, _device_index: usize) -> i32 {
        // FIXME (upstream): this returns false, i.e. player index 0, rather
        // than -1 (no player index).
        0
    }

    /// Translation of `WGI_JoystickSetDevicePlayerIndex()`.
    fn set_device_player_index(&self, _device_index: usize, _player_index: i32) {}

    /// Translation of `WGI_JoystickGetDeviceGUID()`.
    fn device_guid(&self, device_index: usize) -> Guid {
        with_wgi(|wgi| {
            wgi.controllers
                .get(device_index)
                .map_or(Guid::ZERO, |c| c.guid)
        })
    }

    /// Translation of `WGI_JoystickGetDeviceInstanceID()`.
    fn device_instance_id(&self, device_index: usize) -> JoystickID {
        with_wgi(|wgi| {
            wgi.controllers
                .get(device_index)
                .map_or(0, |c| c.instance_id)
        })
    }

    /// Translation of `WGI_JoystickOpen()`.
    fn open(&self, joystick: &mut JoystickData, device_index: usize) -> Result<()> {
        let Some((controller, gamepad_statics2)) = with_wgi(|wgi| {
            wgi.controllers
                .get(device_index)
                .map(|c| (c.controller.clone(), wgi.gamepad_statics2.clone()))
        }) else {
            return Err(Error::new("No such device"));
        };

        let game_controller = controller
            .query::<IGameControllerVtbl>(&IID_IGAMECONTROLLER)
            .ok();
        let battery = controller
            .query::<IGameControllerBatteryInfoVtbl>(&IID_IGAMECONTROLLERBATTERYINFO)
            .ok();

        let mut gamepad = None;
        if let (Some(statics2), Some(game_controller)) = (&gamepad_statics2, &game_controller) {
            // SAFETY: FromGameController stores the gamepad (or NULL) on
            // success; the gamepad is an IGamepad.
            gamepad = unsafe {
                ComPtr::from_out(|out: *mut *mut IGamepad| {
                    (statics2.vtbl().FromGameController)(
                        statics2.as_ptr(),
                        game_controller.as_ptr(),
                        out.cast(),
                    )
                })
            }
            .ok();
        }

        let mut wireless = 0u8;
        if let Some(game_controller) = &game_controller {
            // SAFETY: get_IsWireless writes the flag.
            unsafe {
                (game_controller.vtbl().get_IsWireless)(game_controller.as_ptr(), &mut wireless);
            }
        }

        // Initialize the joystick capabilities
        joystick.connection_state = if wireless != 0 {
            JoystickConnectionState::Wireless
        } else {
            JoystickConnectionState::Wired
        };
        let (mut nbuttons, mut naxes, mut nhats) = (0i32, 0i32, 0i32);
        // SAFETY: the getters write the counts.
        unsafe {
            (controller.vtbl().get_ButtonCount)(controller.as_ptr(), &mut nbuttons);
            (controller.vtbl().get_AxisCount)(controller.as_ptr(), &mut naxes);
            (controller.vtbl().get_SwitchCount)(controller.as_ptr(), &mut nhats);
        }
        joystick.nbuttons = nbuttons.max(0) as usize;
        joystick.naxes = naxes.max(0) as usize;
        joystick.nhats = nhats.max(0) as usize;

        if gamepad.is_some() {
            // FIXME: Can WGI even tell us if trigger rumble is supported?
            let props = joystick.properties();
            let _ = props.set(PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN, true);
            let _ = props.set(PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN, true);
        }

        let hwdata = HwData {
            instance_id: joystick.instance_id,
            controller,
            game_controller,
            battery,
            gamepad,
            vibration: GamepadVibration::default(),
            timestamp: 0,
        };
        with_wgi(|wgi| wgi.open.push(hwdata));
        Ok(())
    }

    /// Translation of `WGI_JoystickRumble()`.
    fn rumble(
        &self,
        joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        with_wgi(|wgi| {
            let hwdata = wgi
                .open
                .iter_mut()
                .find(|h| h.instance_id == joystick)
                .ok_or_else(|| Error::invalid_param("joystick"))?;

            let Some(gamepad) = &hwdata.gamepad else {
                return Err(Error::unsupported());
            };
            // Note: reusing partially filled vibration data struct
            hwdata.vibration.LeftMotor = low_frequency_rumble as f64 / u16::MAX as f64;
            hwdata.vibration.RightMotor = high_frequency_rumble as f64 / u16::MAX as f64;
            // SAFETY: the gamepad is alive; the vibration is passed by value.
            let hr = unsafe { (gamepad.vtbl().put_Vibration)(gamepad.as_ptr(), hwdata.vibration) };
            if hr >= 0 {
                Ok(())
            } else {
                Err(error_from_hresult(
                    Some("Windows.Gaming.Input.IGamepad.put_Vibration failed"),
                    hr,
                ))
            }
        })
    }

    /// Translation of `WGI_JoystickRumbleTriggers()`.
    fn rumble_triggers(
        &self,
        joystick: JoystickID,
        left_rumble: u16,
        right_rumble: u16,
    ) -> Result<()> {
        with_wgi(|wgi| {
            let hwdata = wgi
                .open
                .iter_mut()
                .find(|h| h.instance_id == joystick)
                .ok_or_else(|| Error::invalid_param("joystick"))?;

            let Some(gamepad) = &hwdata.gamepad else {
                return Err(Error::unsupported());
            };
            // Note: reusing partially filled vibration data struct
            hwdata.vibration.LeftTrigger = left_rumble as f64 / u16::MAX as f64;
            hwdata.vibration.RightTrigger = right_rumble as f64 / u16::MAX as f64;
            // SAFETY: the gamepad is alive; the vibration is passed by value.
            let hr = unsafe { (gamepad.vtbl().put_Vibration)(gamepad.as_ptr(), hwdata.vibration) };
            if hr >= 0 {
                Ok(())
            } else {
                Err(error_from_hresult(
                    Some("Windows.Gaming.Input.IGamepad.put_Vibration failed"),
                    hr,
                ))
            }
        })
    }

    /// Translation of `WGI_JoystickSetLED()`.
    fn set_led(&self, _joystick: JoystickID, _red: u8, _green: u8, _blue: u8) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `WGI_JoystickSendEffect()`.
    fn send_effect(&self, _joystick: JoystickID, _data: &[u8]) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `WGI_JoystickSetSensorsEnabled()`.
    fn set_sensors_enabled(&self, _joystick: JoystickID, _enabled: bool) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `WGI_JoystickUpdate()`.
    fn update(&self, joystick: JoystickID) {
        let Some((controller, battery, last_timestamp)) = with_wgi(|wgi| {
            wgi.open
                .iter()
                .find(|h| h.instance_id == joystick)
                .map(|h| (h.controller.clone(), h.battery.clone(), h.timestamp))
        }) else {
            return;
        };
        let Some((nbuttons, nhats, naxes)) = super::super::with_joystick(joystick, |j| {
            (
                j.nbuttons.min(u8::MAX as usize),
                j.nhats.min(u8::MAX as usize),
                j.naxes.min(u8::MAX as usize),
            )
        }) else {
            return;
        };
        let mut buttons = vec![0u8; nbuttons];
        let mut hats = vec![0i32; nhats];
        let mut axes = vec![0f64; naxes];
        let mut timestamp = 0u64;

        // SAFETY: each buffer holds the count given.
        let hr = unsafe {
            (controller.vtbl().GetCurrentReading)(
                controller.as_ptr(),
                nbuttons as u32,
                buttons.as_mut_ptr(),
                nhats as u32,
                hats.as_mut_ptr(),
                naxes as u32,
                axes.as_mut_ptr(),
                &mut timestamp,
            )
        };
        if hr >= 0 && (timestamp == 0 || timestamp != last_timestamp) {
            with_wgi(|wgi| {
                if let Some(h) = wgi.open.iter_mut().find(|h| h.instance_id == joystick) {
                    h.timestamp = timestamp;
                }
            });

            // The axes are all zero when the application loses focus
            let all_zero = naxes > 0 && axes.iter().all(|&axis| axis == 0.0);
            if all_zero {
                private_joystick_force_recentering(joystick);
            } else {
                // FIXME: What units are the timestamp we get from GetCurrentReading()?
                let timestamp = crate::timer::ticks_ns();
                for (i, &button) in buttons.iter().enumerate() {
                    send_joystick_button(timestamp, joystick, i as u8, button != 0);
                }
                for (i, &hat) in hats.iter().enumerate() {
                    send_joystick_hat(timestamp, joystick, i as u8, convert_hat_value(hat));
                }
                for (i, &axis) in axes.iter().enumerate() {
                    send_joystick_axis(timestamp, joystick, i as u8, convert_axis_value(axis));
                }
            }
        }

        if let Some(battery) = battery {
            // SAFETY: TryGetBatteryReport stores a report (or NULL) on success.
            let report = unsafe {
                ComPtr::<IBatteryReportVtbl>::from_out(|out| {
                    (battery.vtbl().TryGetBatteryReport)(battery.as_ptr(), out)
                })
            };
            if let Ok(report) = report {
                let mut state = PowerState::Unknown;
                let mut status = 0;

                // SAFETY: get_Status writes the status.
                let hr = unsafe { (report.vtbl().get_Status)(report.as_ptr(), &mut status) };
                if hr >= 0 {
                    state = power_state_of_battery_status(status);
                }

                let full_capacity = capacity(
                    &report,
                    report.vtbl().get_FullChargeCapacityInMilliwattHours,
                );
                let curr_capacity =
                    capacity(&report, report.vtbl().get_RemainingCapacityInMilliwattHours);

                let percent = battery_percent(full_capacity, curr_capacity);

                send_joystick_power_info(joystick, state, percent);
            }
        }
    }

    /// Translation of `WGI_JoystickClose()`.
    fn close(&self, joystick: &mut JoystickData) {
        // (the hwdata's references are released when it drops)
        let hwdata = with_wgi(|wgi| {
            let i = wgi
                .open
                .iter()
                .position(|h| h.instance_id == joystick.instance_id)?;
            Some(wgi.open.remove(i))
        });
        drop(hwdata);
    }

    /// Translation of `WGI_JoystickQuit()`.
    fn quit(&self) {
        let statics = with_wgi(|wgi| wgi.controller_statics.clone());
        if let Some(statics) = statics {
            while let Some(controller) =
                with_wgi(|wgi| wgi.controllers.last().map(|c| c.controller.clone()))
            {
                invoke_removed(&controller);
                // FIXME (upstream): if InvokeRemoved() can't remove the
                // controller (its QueryInterface() fails), this loops
                // forever; here the controller is dropped anyway.
                with_wgi(|wgi| {
                    if wgi
                        .controllers
                        .last()
                        .is_some_and(|c| c.controller.same(&controller))
                    {
                        wgi.controllers.pop();
                    }
                });
            }

            with_wgi(|wgi| {
                wgi.arcade_stick_statics = None;
                wgi.arcade_stick_statics2 = None;
                wgi.flight_stick_statics = None;
                wgi.gamepad_statics = None;
                wgi.gamepad_statics2 = None;
                wgi.racing_wheel_statics = None;
                wgi.racing_wheel_statics2 = None;
            });

            let (added, removed) =
                with_wgi(|wgi| (wgi.controller_added_token, wgi.controller_removed_token));
            // SAFETY: the tokens came from the add_* calls on statics.
            unsafe {
                (statics.vtbl().remove_RawGameControllerAdded)(statics.as_ptr(), added);
                (statics.vtbl().remove_RawGameControllerRemoved)(statics.as_ptr(), removed);
            }
        }

        let ro_initialized = with_wgi(|wgi| {
            let ro_initialized = wgi.ro_initialized;
            *wgi = Wgi::default();
            ro_initialized
        });
        if ro_initialized {
            ro_uninitialize();
        }
    }

    /// Translation of `WGI_JoystickGetGamepadMapping()`.
    fn gamepad_mapping(&self, _device_index: usize) -> Option<GamepadMapping> {
        None
    }
}
