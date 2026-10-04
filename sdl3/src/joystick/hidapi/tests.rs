// Tests for the HIDAPI joystick driver framework.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use super::*;

/// A device that isn't opened, for driver tests.
pub(super) fn test_device(info: &DeviceInfo) -> Arc<HidapiDevice> {
    let name = create_joystick_name(
        info.vendor_id,
        info.product_id,
        info.manufacturer_string.as_deref(),
        info.product_string.as_deref(),
    )
    .unwrap_or_default();
    let gamepad_type = get_joystick_game_controller_protocol(
        Some(&name),
        info.vendor_id,
        info.product_id,
        info.interface_number,
        info.interface_class,
        info.interface_subclass,
        info.interface_protocol,
    );
    Arc::new(HidapiDevice {
        manufacturer_string: info.manufacturer_string.clone(),
        product_string: info.product_string.clone(),
        path: info.path.clone().unwrap_or_default(),
        vendor_id: info.vendor_id,
        product_id: info.product_id,
        version: info.release_number,
        interface_number: info.interface_number,
        interface_class: info.interface_class,
        interface_subclass: info.interface_subclass,
        interface_protocol: info.interface_protocol,
        usage_page: info.usage_page,
        usage: info.usage,
        is_bluetooth: info.bus_type == BusType::Bluetooth,
        state: Mutex::new(DeviceState {
            name,
            serial: info.serial_number.clone(),
            guid: create_joystick_guid(
                HARDWARE_BUS_USB,
                info.vendor_id,
                info.product_id,
                info.release_number,
                info.manufacturer_string.as_deref(),
                info.product_string.as_deref(),
                b'h',
                0,
            ),
            joystick_type: JoystickType::Gamepad,
            gamepad_type,
            steam_virtual_gamepad_slot: -1,
            driver: None,
            joysticks: Vec::new(),
            seen: true,
            broken: false,
            parent: Weak::new(),
            children: Vec::new(),
        }),
        context: Mutex::new(None),
        dev: Mutex::new(None),
        rumble_pending: AtomicI32::new(0),
        valid: AtomicBool::new(true),
    })
}

/// The events a driver function reported, as text, for comparisons.
pub(super) fn describe(pending: &[Pending]) -> Vec<String> {
    pending
        .iter()
        .map(|p| match p {
            Pending::Axis(_, _, axis, value) => format!("axis {axis} {value}"),
            Pending::Button(_, _, button, down) => format!("button {button} {}", u8::from(*down)),
            Pending::Hat(_, _, hat, value) => format!("hat {hat} {value}"),
            Pending::Touchpad(_, _, touchpad, finger, down, x, y, pressure) => {
                format!(
                    "touchpad {touchpad} {finger} {} {x} {y} {pressure}",
                    u8::from(*down)
                )
            }
            Pending::Sensor(_, _, sensor_type, sensor_timestamp, data, n) => {
                format!(
                    "sensor {sensor_type:?} {sensor_timestamp} {:?}",
                    &data[..*n]
                )
            }
            Pending::PowerInfo(_, state, percent) => format!("power {state:?} {percent}"),
            Pending::Added(_) => "added".to_owned(),
            Pending::Removed(_, _) => "removed".to_owned(),
            Pending::UpdateProperties(_) => "properties".to_owned(),
        })
        .collect()
}

/// Run `f` with a context for `device`, returning what it reported.
pub(super) fn run<R>(
    device: &Arc<HidapiDevice>,
    f: impl FnOnce(&mut DeviceCtx<'_>) -> R,
) -> (R, Vec<String>) {
    let mut pending = Vec::new();
    let result = {
        let mut dctx = DeviceCtx {
            device,
            pending: &mut pending,
        };
        f(&mut dctx)
    };
    (result, describe(&pending))
}

#[test]
fn game_controller_protocol() {
    // Xbox 360 and Xbox One interfaces of known vendors
    assert_eq!(
        get_joystick_game_controller_protocol(None, 0x045e, 0x1234, 0, 0xff, 93, 1),
        GamepadType::Xbox360
    );
    assert_eq!(
        get_joystick_game_controller_protocol(None, 0x045e, 0x1234, 0, 0xff, 71, 208),
        GamepadType::XboxOne
    );
    // ... on another interface, or of an unknown vendor, the IDs decide
    assert_eq!(
        get_joystick_game_controller_protocol(None, 0x045e, 0x1234, 1, 0xff, 71, 208),
        GamepadType::Standard
    );
    assert_eq!(
        get_joystick_game_controller_protocol(None, 0xabcd, 0x1234, 0, 0xff, 93, 1),
        GamepadType::Standard
    );
    assert_eq!(
        get_joystick_game_controller_protocol(None, 0x054c, 0x09cc, -1, 0, 0, 0),
        GamepadType::Ps4
    );
}

#[test]
fn playstation_detection() {
    assert!(supports_playstation_detection(USB_VENDOR_HORI, 0x0001));
    assert!(!supports_playstation_detection(USB_VENDOR_LOGITECH, 0x0001));
    assert!(!supports_playstation_detection(
        USB_VENDOR_MADCATZ,
        USB_PRODUCT_MADCATZ_SAITEK_SIDE_PANEL_CONTROL_DECK
    ));
    assert!(supports_playstation_detection(0x7545, 0x0001));
    // A known HORI Switch controller isn't probed
    assert!(!supports_playstation_detection(USB_VENDOR_HORI, 0x00c1));
    assert_eq!(remap_val(0.5, 0.0, 1.0, -10.0, 10.0), 0.0);
    assert_eq!(load16(0x34, 0x12), 0x1234);
    assert_eq!(load16(0x00, 0x80), i16::MIN);
    assert_eq!(load32(0x78, 0x56, 0x34, 0x12), 0x12345678);
}

#[test]
fn equivalent_devices() {
    let device = test_device(&DeviceInfo {
        vendor_id: USB_VENDOR_MICROSOFT,
        product_id: USB_PRODUCT_XBOX360_WIRELESS_RECEIVER,
        interface_number: -1,
        ..DeviceInfo::default()
    });
    assert!(is_equivalent_to_device(
        USB_VENDOR_MICROSOFT,
        USB_PRODUCT_XBOX360_WIRELESS_RECEIVER,
        &device
    ));
    assert!(is_equivalent_to_device(
        USB_VENDOR_MICROSOFT,
        USB_PRODUCT_XBOX360_XUSB_CONTROLLER,
        &device
    ));
    assert!(!is_equivalent_to_device(0x054c, 0x09cc, &device));
}

#[test]
fn connections_are_delivered_later() {
    let _l = crate::test_support::test_lock();
    let _lock = lock_joysticks();

    let device = test_device(&DeviceInfo {
        vendor_id: 0x045e,
        product_id: 0x028e,
        interface_number: 0,
        ..DeviceInfo::default()
    });
    let (joystick, events) = run(&device, |d| {
        let id = d.joystick_connected();
        d.send_button(1, id, 0, true);
        d.joystick_disconnected(id);
        id
    });
    assert_ne!(joystick, 0);
    // The joystick left the device right away; the events wait
    assert_eq!(device.num_joysticks(), 0);
    assert_eq!(events, ["added", "button 0 1", "removed"]);
    NUMJOYSTICKS.store(0, Ordering::Relaxed);
    CHANGE_COUNT.store(0, Ordering::Relaxed);
}

#[test]
fn device_names_and_serials() {
    let _l = crate::test_support::test_lock();
    let _lock = lock_joysticks();

    let device = test_device(&DeviceInfo {
        vendor_id: 0x054c,
        product_id: 0x09cc,
        interface_number: 0,
        ..DeviceInfo::default()
    });
    let guid = device.guid();
    run(&device, |d| {
        d.set_device_name("My Pad");
        d.set_device_serial("aa-bb");
        // Empty values don't change anything
        d.set_device_name("");
        d.set_device_serial("");
    });
    assert_eq!(device.name(), "My Pad");
    assert_eq!(device.serial().as_deref(), Some("aa-bb"));
    let mut expected = guid;
    set_joystick_guid_crc(&mut expected, crate::stdlib::crc16(0, b"My Pad"));
    assert_eq!(device.guid(), expected);
    assert!(!serial_is_empty(&device));
    device.state().serial = Some("0000".to_owned());
    assert!(serial_is_empty(&device));
}
