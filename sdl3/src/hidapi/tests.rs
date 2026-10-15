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
