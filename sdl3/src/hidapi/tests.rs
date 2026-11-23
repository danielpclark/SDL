// Tests for the HIDAPI layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use super::*;

#[test]
fn libusb_required_devices() {
    assert!(requires_libusb(
        USB_VENDOR_NINTENDO,
        USB_PRODUCT_NINTENDO_GAMECUBE_ADAPTER,
        false
    ));
    assert!(requires_libusb(
        USB_VENDOR_MICROSOFT,
        USB_PRODUCT_XBOX360_BIGBUTTON_RECEIVER,
        false
    ));
    assert!(!requires_libusb(USB_VENDOR_MICROSOFT, 0x028e, false));
    assert_eq!(
        requires_libusb(USB_VENDOR_MICROSOFT, 0x028e, true),
        cfg!(target_os = "macos")
    );
}

#[test]
fn device_filtering() {
    let _l = crate::test_support::test_lock();

    init().ok();
    hints::set(hints::HIDAPI_ENUMERATE_ONLY_CONTROLLERS, "1").unwrap();
    hints::set(hints::HIDAPI_IGNORE_DEVICES, "0x1234/0x5678,0xABCD/0x0000").unwrap();
    // (the hint callbacks only exist while the library is initialized)
    if lock_state().refcount > 0 {
        let ignore = |bus, vid, pid, page, usage| {
            should_ignore_device(bus, vid, pid, page, usage, false, false)
        };

        // Devices that need libusb aren't platform devices
        assert!(ignore(
            BusType::Usb,
            USB_VENDOR_NINTENDO,
            USB_PRODUCT_NINTENDO_GAMECUBE_ADAPTER,
            0,
            0
        ));
        // Only controllers, when the usage page is known
        assert!(!ignore(
            BusType::Usb,
            USB_VENDOR_MICROSOFT,
            0x028e,
            USB_USAGEPAGE_GENERIC_DESKTOP,
            USB_USAGE_GENERIC_GAMEPAD
        ));
        assert!(ignore(
            BusType::Usb,
            USB_VENDOR_MICROSOFT,
            0x028e,
            USB_USAGEPAGE_GENERIC_DESKTOP,
            USB_USAGE_GENERIC_KEYBOARD
        ));
        assert!(!ignore(BusType::Usb, USB_VENDOR_MICROSOFT, 0x028e, 0, 0));
        // The Steam Controller's keyboard and mouse
        assert!(ignore(
            BusType::Usb,
            USB_VENDOR_VALVE,
            0x1102,
            USB_USAGEPAGE_GENERIC_DESKTOP,
            USB_USAGE_GENERIC_MOUSE
        ));
        assert_eq!(
            ignore(
                BusType::Bluetooth,
                USB_VENDOR_VALVE,
                0x1106,
                USB_USAGEPAGE_GENERIC_DESKTOP,
                USB_USAGE_GENERIC_MOUSE
            ),
            cfg!(windows)
        );
        assert!(!ignore(BusType::Usb, USB_VENDOR_VALVE, 0x1102, 0xff00, 1));
        // Flydigi controllers by their vendor usage page
        assert!(!ignore(
            BusType::Usb,
            USB_VENDOR_FLYDIGI_V2,
            USB_PRODUCT_FLYDIGI_V2_APEX,
            USB_USAGEPAGE_VENDOR_FLYDIGI,
            1
        ));
        assert!(ignore(
            BusType::Usb,
            USB_VENDOR_FLYDIGI_V2,
            USB_PRODUCT_FLYDIGI_V2_APEX,
            USB_USAGEPAGE_GENERIC_DESKTOP,
            USB_USAGE_GENERIC_GAMEPAD
        ));
        // The ignore list (vendor wildcards with a 0x0000 product)
        assert!(ignore(BusType::Usb, 0x1234, 0x5678, 0, 0));
        assert!(!ignore(BusType::Usb, 0x1234, 0x5679, 0, 0));
        assert!(ignore(BusType::Usb, 0xabcd, 0x0001, 0, 0));

        hints::set(hints::HIDAPI_ENUMERATE_ONLY_CONTROLLERS, "0").unwrap();
        assert!(!ignore(
            BusType::Usb,
            USB_VENDOR_MICROSOFT,
            0x028e,
            USB_USAGEPAGE_GENERIC_DESKTOP,
            USB_USAGE_GENERIC_KEYBOARD
        ));
        exit().unwrap();
    } else {
        crate::test_support::skip("hidapi", "hidapi couldn't be initialized");
    }
    hints::reset(hints::HIDAPI_ENUMERATE_ONLY_CONTROLLERS);
    hints::reset(hints::HIDAPI_IGNORE_DEVICES);
}

#[test]
fn init_exit_and_enumerate() {
    let _l = crate::test_support::test_lock();

    if let Err(e) = init() {
        crate::test_support::skip(
            "hidapi",
            format_args!("hidapi couldn't be initialized ({})", e.message()),
        );
        return;
    }
    // Counted
    init().unwrap();
    assert_eq!(lock_state().refcount, 2);
    assert_ne!(device_change_count(), 0);

    let devices = enumerate(0, 0).unwrap();
    if devices.is_empty() {
        println!("note: no HID devices, skipping the device checks");
    }
    for info in &devices {
        // Filtering by the device's own IDs finds it again
        let same = enumerate(info.vendor_id, info.product_id).unwrap();
        assert!(same.iter().any(|d| d.path == info.path));

        // Opening may be refused (permissions); when it works the device
        // describes itself the same way
        if let Some(path) = &info.path {
            if let Ok(device) = HidDevice::open_path(path) {
                device.set_nonblocking(true).unwrap();
                let mut buf = [0u8; 64];
                let _ = device.read(&mut buf);
                let _ = device.read_timeout(&mut buf, Some(Duration::ZERO));
                if let Ok(own) = device.device_info() {
                    assert_eq!(own.vendor_id, info.vendor_id);
                    assert_eq!(own.product_id, info.product_id);
                }
                let mut descriptor = [0u8; MAX_REPORT_DESCRIPTOR_SIZE];
                if let Ok(len) = device.report_descriptor(&mut descriptor) {
                    assert!(len <= MAX_REPORT_DESCRIPTOR_SIZE);
                }
                assert_eq!(device.properties(), device.properties());
            }
        }
    }
    assert!(HidDevice::open_path("/nonexistent/hidraw").is_err());
    assert!(HidDevice::open(0xfff0, 0xfff0, None).is_err());

    exit().unwrap();
    assert_eq!(lock_state().refcount, 1);
    exit().unwrap();
    assert_eq!(lock_state().refcount, 0);
    // Exiting more often than initializing is harmless
    exit().unwrap();
}

/// The IOHIDManager backend with every device enumerated (not only
/// controllers): the paths are registry entry IDs, the strings are never
/// missing, IOKit notifications drive the change counter, and devices on a
/// vendor usage page (never keyboards, which would need the Input Monitoring
/// permission) open with their read thread, read nothing in time and close.
#[cfg(target_os = "macos")]
#[test]
fn macos_enumerate_all_and_open_vendor_devices() {
    let _l = crate::test_support::test_lock();

    hints::set(hints::HIDAPI_ENUMERATE_ONLY_CONTROLLERS, "0").unwrap();
    if let Err(e) = init() {
        crate::test_support::skip(
            "hidapi",
            format_args!("hidapi couldn't be initialized ({})", e.message()),
        );
        hints::reset(hints::HIDAPI_ENUMERATE_ONLY_CONTROLLERS);
        return;
    }
    assert_ne!(device_change_count(), 0);
    assert!(lock_state().discovery.can_get_notifications);
    // The devices aren't seized: SDL opens them shared
    assert!(!mac::hid_darwin_get_open_exclusive());

    let devices = enumerate(0, 0).unwrap();
    println!("note: {} HID device interfaces", devices.len());
    for info in &devices {
        let path = info.path.as_deref().unwrap();
        assert!(path.starts_with("DevSrvsID:"), "{path}");
        assert!(info.serial_number.is_some());
        assert!(info.manufacturer_string.is_some());
        assert!(info.product_string.is_some());
    }
    let mut opened = 0;
    for info in devices.iter().filter(|d| d.usage_page >= 0xff00).take(3) {
        let Ok(device) = HidDevice::open_path(info.path.as_deref().unwrap()) else {
            continue;
        };
        opened += 1;
        let mut buf = [0u8; 64];
        // (vendor interfaces rarely send anything unprompted)
        let _ = device.read_timeout(&mut buf, Some(Duration::from_millis(10)));
        device.set_nonblocking(true).unwrap();
        assert_eq!(device.read(&mut buf).unwrap_or(0), 0);
        let own = device.device_info().unwrap();
        assert_eq!(own.path, info.path);
        let mut descriptor = [0u8; MAX_REPORT_DESCRIPTOR_SIZE];
        assert!(device.report_descriptor(&mut descriptor).is_ok());
        assert_eq!(
            device.indexed_string(1).unwrap_err().message(),
            "hid_get_indexed_string: not available on this platform"
        );
        assert_eq!(device.write(&[]).unwrap_err().message(), "Invalid argument");
    }
    println!("note: opened {opened} vendor device interfaces");
    assert_eq!(
        HidDevice::open_path("DevSrvsID:1").unwrap_err().message(),
        "hid_open_path: device mach entry not found with the given path"
    );

    exit().unwrap();
    hints::reset(hints::HIDAPI_ENUMERATE_ONLY_CONTROLLERS);
}

/// The Steam Controllers' characteristics, as the iOS backend tells them
/// apart (`-peripheral:didDiscoverCharacteristicsForService:error:`).
#[test]
fn ble_characteristics() {
    use ble::*;

    assert_eq!(
        input_characteristic(VALVE_INPUT_CHAR_0X1106),
        Some(InputCharacteristic {
            pid: D0G_BLE2_PID,
            report_id: 0x03,
            report_length: 19
        })
    );
    assert_eq!(
        input_characteristic(VALVE_INPUT_CHAR_0X1303_0X45),
        Some(InputCharacteristic {
            pid: TRITON_BLE_PID,
            report_id: 0x45,
            report_length: 45
        })
    );
    // (a UUID is the same in lower case)
    assert_eq!(
        input_characteristic(&VALVE_INPUT_CHAR_0X1303_0X47.to_ascii_lowercase()),
        Some(InputCharacteristic {
            pid: TRITON_BLE_PID,
            report_id: 0x47,
            report_length: 45
        })
    );
    assert_eq!(input_characteristic(VALVE_REPORT_CHAR), None);
    assert_eq!(input_characteristic(VALVE_SERVICE), None);
    assert!(uuid_eq(
        VALVE_REPORT_CHAR,
        &VALVE_REPORT_CHAR.to_ascii_lowercase()
    ));
    assert!(!uuid_eq(VALVE_REPORT_CHAR, VALVE_SERVICE));
    assert_eq!(VALVE_USB_VID, USB_VENDOR_VALVE);

    // Output reports: "100F6C%x" past 0x35, less 0x35, from 0x80
    assert_eq!(
        output_report_id("100F6CB5-1735-4313-B402-38567131E5F3"),
        Some(0x80)
    );
    assert_eq!(
        output_report_id("100F6CFF-1735-4313-B402-38567131E5F3"),
        Some(0xca)
    );
    assert_eq!(
        output_report_id("100F6CB4-1735-4313-B402-38567131E5F3"),
        None
    );
    assert_eq!(
        output_report_id("100F6C36-1735-4313-B402-38567131E5F3"),
        None
    );
    // (the input and report characteristics are below 0x80 or 0x35)
    for uuid in [
        VALVE_INPUT_CHAR_0X1106,
        VALVE_INPUT_CHAR_0X1303_0X45,
        VALVE_INPUT_CHAR_0X1303_0X47,
        VALVE_REPORT_CHAR,
        VALVE_SERVICE,
    ] {
        assert_eq!(output_report_id(uuid), None, "{uuid}");
    }
    // (sscanf's literal prefix is case sensitive; `%x` takes as many
    // digits as there are, after spaces, a sign or 0x)
    assert_eq!(
        output_report_id("100f6cb5-1735-4313-B402-38567131E5F3"),
        None
    );
    assert_eq!(output_report_id("2902"), None);
    assert_eq!(output_report_id("100F6C-1735"), None);
    assert_eq!(output_report_id("100F6C0xB5-1735"), Some(0x80));
    assert_eq!(output_report_id("100F6C 1B5"), Some(0x180));
    assert_eq!(output_report_id("100F6C-B5"), None);
}

/// The input reports' ring buffer (`RingBuffer`): first in, first out,
/// 255 reports at most (the 256th write drops the oldest and says so).
#[test]
fn ble_ring_buffer() {
    use ble::*;

    let mut ring = RingBuffer::new(3);
    assert_eq!(ring.elem_size(), 3);
    let mut out = [0u8; 3];
    assert!(!ring.read(&mut out));
    assert!(ring.write(&[1, 2, 3]));
    // (a short report's rest is zeros, a long one is cut)
    assert!(ring.write(&[4]));
    assert!(ring.write(&[7, 8, 9, 10]));
    assert!(ring.read(&mut out));
    assert_eq!(out, [1, 2, 3]);
    assert!(ring.read(&mut out));
    assert_eq!(out, [4, 0, 0]);
    assert!(ring.read(&mut out));
    assert_eq!(out, [7, 8, 9]);
    assert!(!ring.read(&mut out));

    let mut ring = RingBuffer::new(1);
    for i in 0..255u8 {
        assert!(ring.write(&[i]), "{i}");
    }
    assert!(!ring.write(&[255]));
    let mut out = [0u8; 1];
    for i in 1..=255u8 {
        assert!(ring.read(&mut out));
        assert_eq!(out[0], i);
    }
    assert!(!ring.read(&mut out));

    // Without a report length (upstream divides by zero there)
    let mut ring = RingBuffer::new(0);
    assert!(!ring.write(&[1]));
    assert!(ring.read(&mut []));
}

/// Input reports read from the ring buffer (`read_input_report:`): the
/// report ID, then the report.
#[test]
fn ble_input_reports() {
    use ble::*;

    let mut dst = [0xaau8; 21];
    assert_eq!(read_input_report(None, 3, &mut dst), 0);
    let mut ring = RingBuffer::new(19);
    assert_eq!(read_input_report(Some(&mut ring), 3, &mut dst), 0);
    let report: Vec<u8> = (1..=19).collect();
    ring.write(&report);
    assert_eq!(read_input_report(Some(&mut ring), 3, &mut dst), 20);
    assert_eq!(dst[0], 3);
    assert_eq!(&dst[1..20], &report[..]);
    assert_eq!(dst[20], 0xaa);
    // (a short buffer gets what fits, and the report's length)
    ring.write(&report);
    let mut short = [0u8; 5];
    assert_eq!(read_input_report(Some(&mut ring), 0x45, &mut short), 20);
    assert_eq!(short, [0x45, 1, 2, 3, 4]);
}

/// Feature reports (`send_feature_report:length:`,
/// `get_feature_report:into:length:` and `hid_get_feature_report()`).
#[test]
fn ble_feature_reports() {
    use ble::*;

    // Sent: the bytes after the report ID, as many as the report's length
    // (so the last one is past the report: 0 here), at most 64
    let packet: Vec<u8> = (0..20).map(|i| 0x10 + i).collect();
    let segment = feature_report_segment(&packet);
    assert_eq!(segment.len(), 20);
    assert_eq!(&segment[..19], &packet[1..]);
    assert_eq!(segment[19], 0);
    let usb: Vec<u8> = (0..65).collect();
    assert_eq!(feature_report_segment(&usb), &usb[1..65]);
    assert!(feature_report_segment(&[]).is_empty());
    assert_eq!(feature_report_segment(&[3]), [0]);

    // Received: after the report ID asked for, unless the value starts
    // with it
    let value: Vec<u8> = (0..19).map(|i| 0xc0 + i).collect();
    let mut buffer = [0u8; 20];
    buffer[0] = 0x03;
    assert_eq!(copy_feature_report(&value, &mut buffer), 20);
    assert_eq!(buffer[0], 0x03);
    assert_eq!(&buffer[1..], &value[..]);
    let mut buffer = [0u8; 4];
    buffer[0] = 0xc0;
    assert_eq!(copy_feature_report(&value, &mut buffer), 4);
    assert_eq!(buffer, [0xc0, 0xc1, 0xc2, 0xc3]);
    let mut buffer = [3u8; 8];
    assert_eq!(copy_feature_report(&[], &mut buffer), 0);
    assert_eq!(copy_feature_report(&value, &mut []), 0);

    // What hid_get_feature_report() returns: the length when one byte
    // less was copied
    assert_eq!(feature_report_result(19, 20), 20);
    assert_eq!(feature_report_result(20, 20), 20);
    assert_eq!(feature_report_result(5, 20), 5);
    assert_eq!(feature_report_result(-1, 20), -1);
    assert_eq!(feature_report_result(-1, 0), 0);
}

/// On iOS (a process of the simulator, without Bluetooth: the hint isn't
/// set) the backend's manager exists, as upstream's `HIDBLEManager`
/// class, without a central manager, and there are no devices.
#[cfg(any(target_os = "ios", target_os = "tvos"))]
#[test]
fn ios_without_bluetooth() {
    use crate::core::apple::objc::{class, msg_send, Id, NSUInteger};

    let _l = crate::test_support::test_lock();
    if hints::get(hints::JOYSTICK_HIDAPI_STEAM).is_some() {
        println!("note: SDL_JOYSTICK_HIDAPI_STEAM is set, not checking");
        return;
    }
    init().unwrap();
    // SAFETY: the backend's class and its getters.
    unsafe {
        let manager: Id = msg_send![class!(HIDBLEManager), sharedInstance];
        assert!(!manager.is_null());
        let central: Id = msg_send![manager, centralManager];
        assert!(central.is_null());
        let map: Id = msg_send![manager, deviceMap];
        let count: NSUInteger = msg_send![map, count];
        assert_eq!(count, 0);
        ble_scan(true);
        let scans: i32 = msg_send![manager, nPendingScans];
        assert_eq!(scans, 0);
        ble_scan(false);
        let pairs: i32 = msg_send![manager, nPendingPairs];
        assert_eq!(pairs, 0);
    }
    assert!(enumerate(0, 0).unwrap().is_empty());
    assert!(HidDevice::open_path("00000000-0000-0000-0000-000000000000").is_err());
    assert!(HidDevice::open(0x28de, 0x1106, None).is_err());
    exit().unwrap();
}
