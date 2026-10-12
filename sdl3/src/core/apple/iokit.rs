// IOKit (the HID manager, the I/O Registry and its notifications) and
// ForceFeedback, declared by hand for the Apple input backends.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The parts of `<IOKit/IOKitLib.h>`, `<IOKit/hid/IOHIDManager.h>`,
//! `<IOKit/hid/IOHIDKeys.h>`, `<IOKit/usb/USBSpec.h>`, `<mach/mach.h>` and
//! `<ForceFeedback/ForceFeedback.h>` that HIDAPI's macOS backend, the IOKit
//! joystick driver and the Darwin haptic driver use, with the few
//! CoreFoundation functions only they need (`CFSet`, `CFString`
//! comparisons, run loop sources).
//!
//! IOKit and ForceFeedback are linked, as upstream links them.

// (the SDK's names)
#![allow(clippy::upper_case_acronyms)]

use std::ffi::{c_char, c_void};

use super::foundation::{
    Boolean, CFAllocatorRef, CFArrayRef, CFDictionaryRef, CFIndex, CFMutableDictionaryRef,
    CFOptionFlags, CFRange, CFRunLoopRef, CFRunLoopSourceRef, CFStringEncoding, CFStringRef,
    CFTypeID, CFTypeRef,
};

/// `mach_port_t`.
pub(crate) type MachPort = u32;
/// `kern_return_t`.
pub(crate) type KernReturn = i32;
/// `IOReturn`.
pub(crate) type IOReturn = KernReturn;
/// `IOOptionBits`.
pub(crate) type IOOptionBits = u32;
/// `io_object_t` (and `io_service_t`, `io_iterator_t`, `io_registry_entry_t`).
pub(crate) type IoObject = MachPort;
/// `io_service_t`.
pub(crate) type IoService = IoObject;
/// `io_iterator_t`.
pub(crate) type IoIterator = IoObject;
/// `io_registry_entry_t`.
pub(crate) type IoRegistryEntry = IoObject;
/// `IOHIDManagerRef`.
pub(crate) type IOHIDManagerRef = *mut c_void;
/// `IOHIDDeviceRef`.
pub(crate) type IOHIDDeviceRef = *mut c_void;
/// `IOHIDElementRef`.
pub(crate) type IOHIDElementRef = *mut c_void;
/// `IOHIDValueRef`.
pub(crate) type IOHIDValueRef = *mut c_void;
/// `IONotificationPortRef`.
pub(crate) type IONotificationPortRef = *mut c_void;
/// `IOHIDReportType`.
pub(crate) type IOHIDReportType = u32;
/// `IOHIDElementType`.
pub(crate) type IOHIDElementType = u32;
/// `IOHIDElementCookie`.
pub(crate) type IOHIDElementCookie = u32;
/// `CFSetRef`.
pub(crate) type CFSetRef = *const c_void;
/// `CFComparisonResult`.
pub(crate) type CFComparisonResult = CFIndex;
/// `CFStringCompareFlags`.
pub(crate) type CFStringCompareFlags = CFOptionFlags;

/// `MACH_PORT_NULL` / `IO_OBJECT_NULL`.
pub(crate) const MACH_PORT_NULL: MachPort = 0;
/// `IO_OBJECT_NULL`.
pub(crate) const IO_OBJECT_NULL: IoObject = 0;
/// `kIOMainPortDefault` (`kIOMasterPortDefault`): the default main port,
/// `MACH_PORT_NULL`.
pub(crate) const K_IO_MAIN_PORT_DEFAULT: MachPort = MACH_PORT_NULL;
/// `KERN_SUCCESS`.
pub(crate) const KERN_SUCCESS: KernReturn = 0;
/// `KERN_INVALID_ARGUMENT`.
pub(crate) const KERN_INVALID_ARGUMENT: KernReturn = 4;
/// `kIOReturnSuccess`.
pub(crate) const K_IO_RETURN_SUCCESS: IOReturn = 0;
/// `kIOReturnInvalid` (`iokit_common_err(1)`).
pub(crate) const K_IO_RETURN_INVALID: IOReturn = 0xe000_0001_u32 as i32;
/// `kIOReturnNotPermitted` (`iokit_common_err(0x2e2)`).
pub(crate) const K_IO_RETURN_NOT_PERMITTED: IOReturn = 0xe000_02e2_u32 as i32;
/// `kIOReturnExclusiveAccess` (`iokit_common_err(0x2c5)`).
pub(crate) const K_IO_RETURN_EXCLUSIVE_ACCESS: IOReturn = 0xe000_02c5_u32 as i32;

/// `kIOHIDOptionsTypeNone`.
pub(crate) const K_IOHID_OPTIONS_TYPE_NONE: IOOptionBits = 0x00;
/// `kIOHIDOptionsTypeSeizeDevice`.
pub(crate) const K_IOHID_OPTIONS_TYPE_SEIZE_DEVICE: IOOptionBits = 0x01;

/// `kIOHIDReportTypeInput`.
pub(crate) const K_IOHID_REPORT_TYPE_INPUT: IOHIDReportType = 0;
/// `kIOHIDReportTypeOutput`.
pub(crate) const K_IOHID_REPORT_TYPE_OUTPUT: IOHIDReportType = 1;
/// `kIOHIDReportTypeFeature`.
pub(crate) const K_IOHID_REPORT_TYPE_FEATURE: IOHIDReportType = 2;

/// `kIOHIDElementTypeInput_Misc`.
pub(crate) const K_IOHID_ELEMENT_TYPE_INPUT_MISC: IOHIDElementType = 1;
/// `kIOHIDElementTypeInput_Button`.
pub(crate) const K_IOHID_ELEMENT_TYPE_INPUT_BUTTON: IOHIDElementType = 2;
/// `kIOHIDElementTypeInput_Axis`.
pub(crate) const K_IOHID_ELEMENT_TYPE_INPUT_AXIS: IOHIDElementType = 3;
/// `kIOHIDElementTypeInput_ScanCodes`.
pub(crate) const K_IOHID_ELEMENT_TYPE_INPUT_SCAN_CODES: IOHIDElementType = 4;
/// `kIOHIDElementTypeOutput`.
pub(crate) const K_IOHID_ELEMENT_TYPE_OUTPUT: IOHIDElementType = 129;
/// `kIOHIDElementTypeFeature`.
pub(crate) const K_IOHID_ELEMENT_TYPE_FEATURE: IOHIDElementType = 257;
/// `kIOHIDElementTypeCollection`.
pub(crate) const K_IOHID_ELEMENT_TYPE_COLLECTION: IOHIDElementType = 513;

/// `kIOHIDValueScaleTypeCalibrated`.
pub(crate) const K_IOHID_VALUE_SCALE_TYPE_CALIBRATED: u32 = 0;
/// `kIOHIDValueScaleTypePhysical`.
pub(crate) const K_IOHID_VALUE_SCALE_TYPE_PHYSICAL: u32 = 1;

/// `kCFCompareEqualTo`.
pub(crate) const K_CF_COMPARE_EQUAL_TO: CFComparisonResult = 0;
/// `kCFStringEncodingUTF32LE`.
pub(crate) const K_CF_STRING_ENCODING_UTF32LE: CFStringEncoding = 0x1c00_0100;
/// `kCFNumberShortType`.
pub(crate) const K_CF_NUMBER_SHORT_TYPE: CFIndex = 8;

/// `kIOServicePlane`.
pub(crate) const K_IO_SERVICE_PLANE: &std::ffi::CStr = c"IOService";
/// `kIOFirstMatchNotification`.
pub(crate) const K_IO_FIRST_MATCH_NOTIFICATION: &std::ffi::CStr = c"IOServiceFirstMatch";
/// `kIOTerminatedNotification`.
pub(crate) const K_IO_TERMINATED_NOTIFICATION: &std::ffi::CStr = c"IOServiceTerminate";
/// `kIOHIDDeviceKey`: the class of HID device services.
pub(crate) const K_IOHID_DEVICE_KEY: &std::ffi::CStr = c"IOHIDDevice";

// The IOHIDKeys.h / USBSpec.h property keys (used with CFSTR() upstream).

/// `kIOHIDTransportKey`.
pub(crate) const K_IOHID_TRANSPORT_KEY: &str = "Transport";
/// `kIOHIDVendorIDKey`.
pub(crate) const K_IOHID_VENDOR_ID_KEY: &str = "VendorID";
/// `kIOHIDVendorIDSourceKey`.
pub(crate) const K_IOHID_VENDOR_ID_SOURCE_KEY: &str = "VendorIDSource";
/// `kIOHIDProductIDKey`.
pub(crate) const K_IOHID_PRODUCT_ID_KEY: &str = "ProductID";
/// `kIOHIDVersionNumberKey`.
pub(crate) const K_IOHID_VERSION_NUMBER_KEY: &str = "VersionNumber";
/// `kIOHIDManufacturerKey`.
pub(crate) const K_IOHID_MANUFACTURER_KEY: &str = "Manufacturer";
/// `kIOHIDProductKey`.
pub(crate) const K_IOHID_PRODUCT_KEY: &str = "Product";
/// `kIOHIDSerialNumberKey`.
pub(crate) const K_IOHID_SERIAL_NUMBER_KEY: &str = "SerialNumber";
/// `kIOHIDLocationIDKey`.
pub(crate) const K_IOHID_LOCATION_ID_KEY: &str = "LocationID";
/// `kIOHIDPrimaryUsageKey`.
pub(crate) const K_IOHID_PRIMARY_USAGE_KEY: &str = "PrimaryUsage";
/// `kIOHIDPrimaryUsagePageKey`.
pub(crate) const K_IOHID_PRIMARY_USAGE_PAGE_KEY: &str = "PrimaryUsagePage";
/// `kIOHIDDeviceUsagePairsKey`.
pub(crate) const K_IOHID_DEVICE_USAGE_PAIRS_KEY: &str = "DeviceUsagePairs";
/// `kIOHIDDeviceUsageKey`.
pub(crate) const K_IOHID_DEVICE_USAGE_KEY: &str = "DeviceUsage";
/// `kIOHIDDeviceUsagePageKey`.
pub(crate) const K_IOHID_DEVICE_USAGE_PAGE_KEY: &str = "DeviceUsagePage";
/// `kIOHIDMaxInputReportSizeKey`.
pub(crate) const K_IOHID_MAX_INPUT_REPORT_SIZE_KEY: &str = "MaxInputReportSize";
/// `kIOHIDReportDescriptorKey`.
pub(crate) const K_IOHID_REPORT_DESCRIPTOR_KEY: &str = "ReportDescriptor";
/// `kIOHIDVirtualHIDevice`.
pub(crate) const K_IOHID_VIRTUAL_HIDEVICE: &str = "HIDVirtualDevice";
/// `kIOHIDTransportUSBValue`.
pub(crate) const K_IOHID_TRANSPORT_USB_VALUE: &str = "USB";
/// `kIOHIDTransportBluetoothValue`.
pub(crate) const K_IOHID_TRANSPORT_BLUETOOTH_VALUE: &str = "Bluetooth";
/// `kIOHIDTransportBluetoothLowEnergyValue`.
pub(crate) const K_IOHID_TRANSPORT_BLUETOOTH_LOW_ENERGY_VALUE: &str = "BluetoothLowEnergy";
/// `kIOHIDTransportI2CValue`.
pub(crate) const K_IOHID_TRANSPORT_I2C_VALUE: &str = "I2C";
/// `kIOHIDTransportSPIValue`.
pub(crate) const K_IOHID_TRANSPORT_SPI_VALUE: &str = "SPI";
/// `kUSBInterfaceNumber`.
pub(crate) const K_USB_INTERFACE_NUMBER: &str = "bInterfaceNumber";

/// `kCFRunLoopRunFinished` and the other `CFRunLoopRunInMode()` results are
/// in [`super::foundation`].
///
/// `CFRunLoopSourceContext` (version 0).
#[repr(C)]
pub(crate) struct CFRunLoopSourceContext {
    pub(crate) version: CFIndex,
    pub(crate) info: *mut c_void,
    pub(crate) retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    pub(crate) release: Option<unsafe extern "C" fn(*const c_void)>,
    pub(crate) copy_description: Option<unsafe extern "C" fn(*const c_void) -> CFStringRef>,
    pub(crate) equal: Option<unsafe extern "C" fn(*const c_void, *const c_void) -> Boolean>,
    pub(crate) hash: Option<unsafe extern "C" fn(*const c_void) -> usize>,
    pub(crate) schedule: Option<unsafe extern "C" fn(*mut c_void, CFRunLoopRef, CFStringRef)>,
    pub(crate) cancel: Option<unsafe extern "C" fn(*mut c_void, CFRunLoopRef, CFStringRef)>,
    pub(crate) perform: Option<unsafe extern "C" fn(*mut c_void)>,
}

/// `IOHIDCallback`.
pub(crate) type IOHIDCallback =
    Option<unsafe extern "C" fn(context: *mut c_void, result: IOReturn, sender: *mut c_void)>;
/// `IOHIDReportCallback`.
pub(crate) type IOHIDReportCallback = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        result: IOReturn,
        sender: *mut c_void,
        report_type: IOHIDReportType,
        report_id: u32,
        report: *mut u8,
        report_length: CFIndex,
    ),
>;
/// `IOHIDDeviceCallback`.
pub(crate) type IOHIDDeviceCallback = Option<
    unsafe extern "C" fn(
        context: *mut c_void,
        result: IOReturn,
        sender: *mut c_void,
        device: IOHIDDeviceRef,
    ),
>;
/// `IOServiceMatchingCallback`.
pub(crate) type IOServiceMatchingCallback =
    Option<unsafe extern "C" fn(refcon: *mut c_void, iterator: IoIterator)>;

/// `mach_msg_header_t`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct MachMsgHeader {
    pub(crate) msgh_bits: u32,
    pub(crate) msgh_size: u32,
    pub(crate) msgh_remote_port: MachPort,
    pub(crate) msgh_local_port: MachPort,
    pub(crate) msgh_voucher_port: MachPort,
    pub(crate) msgh_id: i32,
}

/// `MACH_RCV_MSG`.
pub(crate) const MACH_RCV_MSG: i32 = 0x0000_0002;
/// `MACH_RCV_TIMEOUT`.
pub(crate) const MACH_RCV_TIMEOUT: i32 = 0x0000_0100;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    pub(crate) fn IOObjectRelease(object: IoObject) -> KernReturn;
    pub(crate) fn IOObjectRetain(object: IoObject) -> KernReturn;
    pub(crate) fn IOObjectConformsTo(object: IoObject, class_name: *const c_char) -> Boolean;
    pub(crate) fn IOObjectIsEqualTo(object: IoObject, an_object: IoObject) -> Boolean;
    pub(crate) fn IOIteratorNext(iterator: IoIterator) -> IoObject;
    pub(crate) fn IOIteratorIsValid(iterator: IoIterator) -> Boolean;
    pub(crate) fn IOServiceMatching(name: *const c_char) -> CFMutableDictionaryRef;
    pub(crate) fn IOServiceGetMatchingService(
        main_port: MachPort,
        matching: CFDictionaryRef,
    ) -> IoService;
    pub(crate) fn IOServiceGetMatchingServices(
        main_port: MachPort,
        matching: CFDictionaryRef,
        existing: *mut IoIterator,
    ) -> KernReturn;
    pub(crate) fn IOServiceAddMatchingNotification(
        notify_port: IONotificationPortRef,
        notification_type: *const c_char,
        matching: CFDictionaryRef,
        callback: IOServiceMatchingCallback,
        refcon: *mut c_void,
        notification: *mut IoIterator,
    ) -> KernReturn;
    pub(crate) fn IONotificationPortCreate(main_port: MachPort) -> IONotificationPortRef;
    pub(crate) fn IONotificationPortDestroy(notify: IONotificationPortRef);
    pub(crate) fn IONotificationPortGetMachPort(notify: IONotificationPortRef) -> MachPort;
    pub(crate) fn IONotificationPortGetRunLoopSource(
        notify: IONotificationPortRef,
    ) -> CFRunLoopSourceRef;
    pub(crate) fn IODispatchCalloutFromMessage(
        unused: *mut c_void,
        msg: *mut MachMsgHeader,
        reference: IONotificationPortRef,
    );
    pub(crate) fn IORegistryEntryIDMatching(entry_id: u64) -> CFMutableDictionaryRef;
    pub(crate) fn IORegistryEntryFromPath(
        main_port: MachPort,
        path: *const c_char,
    ) -> IoRegistryEntry;
    pub(crate) fn IORegistryEntryGetRegistryEntryID(
        entry: IoRegistryEntry,
        entry_id: *mut u64,
    ) -> KernReturn;
    pub(crate) fn IORegistryEntryGetParentEntry(
        entry: IoRegistryEntry,
        plane: *const c_char,
        parent: *mut IoRegistryEntry,
    ) -> KernReturn;
    pub(crate) fn IORegistryEntryCreateCFProperty(
        entry: IoRegistryEntry,
        key: CFStringRef,
        allocator: CFAllocatorRef,
        options: IOOptionBits,
    ) -> CFTypeRef;
    pub(crate) fn IORegistryEntryCreateCFProperties(
        entry: IoRegistryEntry,
        properties: *mut CFMutableDictionaryRef,
        allocator: CFAllocatorRef,
        options: IOOptionBits,
    ) -> KernReturn;
    pub(crate) fn IORegistryEntrySearchCFProperty(
        entry: IoRegistryEntry,
        plane: *const c_char,
        key: CFStringRef,
        allocator: CFAllocatorRef,
        options: IOOptionBits,
    ) -> CFTypeRef;

    pub(crate) fn IOHIDManagerCreate(
        allocator: CFAllocatorRef,
        options: IOOptionBits,
    ) -> IOHIDManagerRef;
    pub(crate) fn IOHIDManagerOpen(manager: IOHIDManagerRef, options: IOOptionBits) -> IOReturn;
    pub(crate) fn IOHIDManagerClose(manager: IOHIDManagerRef, options: IOOptionBits) -> IOReturn;
    pub(crate) fn IOHIDManagerSetDeviceMatching(
        manager: IOHIDManagerRef,
        matching: CFDictionaryRef,
    );
    pub(crate) fn IOHIDManagerSetDeviceMatchingMultiple(
        manager: IOHIDManagerRef,
        multiple: CFArrayRef,
    );
    pub(crate) fn IOHIDManagerCopyDevices(manager: IOHIDManagerRef) -> CFSetRef;
    pub(crate) fn IOHIDManagerScheduleWithRunLoop(
        manager: IOHIDManagerRef,
        run_loop: CFRunLoopRef,
        run_loop_mode: CFStringRef,
    );
    pub(crate) fn IOHIDManagerUnscheduleFromRunLoop(
        manager: IOHIDManagerRef,
        run_loop: CFRunLoopRef,
        run_loop_mode: CFStringRef,
    );
    pub(crate) fn IOHIDManagerRegisterDeviceMatchingCallback(
        manager: IOHIDManagerRef,
        callback: IOHIDDeviceCallback,
        context: *mut c_void,
    );

    pub(crate) fn IOHIDDeviceGetTypeID() -> CFTypeID;
    pub(crate) fn IOHIDDeviceCreate(
        allocator: CFAllocatorRef,
        service: IoService,
    ) -> IOHIDDeviceRef;
    pub(crate) fn IOHIDDeviceGetService(device: IOHIDDeviceRef) -> IoService;
    pub(crate) fn IOHIDDeviceOpen(device: IOHIDDeviceRef, options: IOOptionBits) -> IOReturn;
    pub(crate) fn IOHIDDeviceClose(device: IOHIDDeviceRef, options: IOOptionBits) -> IOReturn;
    pub(crate) fn IOHIDDeviceConformsTo(
        device: IOHIDDeviceRef,
        usage_page: u32,
        usage: u32,
    ) -> Boolean;
    pub(crate) fn IOHIDDeviceGetProperty(device: IOHIDDeviceRef, key: CFStringRef) -> CFTypeRef;
    pub(crate) fn IOHIDDeviceCopyMatchingElements(
        device: IOHIDDeviceRef,
        matching: CFDictionaryRef,
        options: IOOptionBits,
    ) -> CFArrayRef;
    pub(crate) fn IOHIDDeviceScheduleWithRunLoop(
        device: IOHIDDeviceRef,
        run_loop: CFRunLoopRef,
        run_loop_mode: CFStringRef,
    );
    pub(crate) fn IOHIDDeviceUnscheduleFromRunLoop(
        device: IOHIDDeviceRef,
        run_loop: CFRunLoopRef,
        run_loop_mode: CFStringRef,
    );
    pub(crate) fn IOHIDDeviceRegisterRemovalCallback(
        device: IOHIDDeviceRef,
        callback: IOHIDCallback,
        context: *mut c_void,
    );
    pub(crate) fn IOHIDDeviceRegisterInputReportCallback(
        device: IOHIDDeviceRef,
        report: *mut u8,
        report_length: CFIndex,
        callback: IOHIDReportCallback,
        context: *mut c_void,
    );
    pub(crate) fn IOHIDDeviceSetReport(
        device: IOHIDDeviceRef,
        report_type: IOHIDReportType,
        report_id: CFIndex,
        report: *const u8,
        report_length: CFIndex,
    ) -> IOReturn;
    pub(crate) fn IOHIDDeviceGetReport(
        device: IOHIDDeviceRef,
        report_type: IOHIDReportType,
        report_id: CFIndex,
        report: *mut u8,
        report_length: *mut CFIndex,
    ) -> IOReturn;
    pub(crate) fn IOHIDDeviceGetValue(
        device: IOHIDDeviceRef,
        element: IOHIDElementRef,
        value: *mut IOHIDValueRef,
    ) -> IOReturn;

    pub(crate) fn IOHIDElementGetTypeID() -> CFTypeID;
    pub(crate) fn IOHIDElementGetType(element: IOHIDElementRef) -> IOHIDElementType;
    pub(crate) fn IOHIDElementGetUsagePage(element: IOHIDElementRef) -> u32;
    pub(crate) fn IOHIDElementGetUsage(element: IOHIDElementRef) -> u32;
    pub(crate) fn IOHIDElementGetCookie(element: IOHIDElementRef) -> IOHIDElementCookie;
    pub(crate) fn IOHIDElementGetLogicalMin(element: IOHIDElementRef) -> CFIndex;
    pub(crate) fn IOHIDElementGetLogicalMax(element: IOHIDElementRef) -> CFIndex;
    pub(crate) fn IOHIDElementGetChildren(element: IOHIDElementRef) -> CFArrayRef;

    pub(crate) fn IOHIDValueGetIntegerValue(value: IOHIDValueRef) -> CFIndex;
    pub(crate) fn IOHIDValueGetScaledValue(value: IOHIDValueRef, scale_type: u32) -> f64;
}

// CoreFoundation functions only the IOKit backends use.
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub(crate) static kCFCoreFoundationVersionNumber: f64;
    pub(crate) fn CFSetGetCount(set: CFSetRef) -> CFIndex;
    pub(crate) fn CFSetGetValues(set: CFSetRef, values: *mut *const c_void);
    pub(crate) fn CFStringCompare(
        s1: CFStringRef,
        s2: CFStringRef,
        options: CFStringCompareFlags,
    ) -> CFComparisonResult;
    pub(crate) fn CFStringHasPrefix(s: CFStringRef, prefix: CFStringRef) -> Boolean;
    pub(crate) fn CFStringGetBytes(
        s: CFStringRef,
        range: CFRange,
        encoding: CFStringEncoding,
        loss_byte: u8,
        is_external_representation: Boolean,
        buffer: *mut u8,
        max_buf_len: CFIndex,
        used_buf_len: *mut CFIndex,
    ) -> CFIndex;
    pub(crate) fn CFRunLoopSourceCreate(
        allocator: CFAllocatorRef,
        order: CFIndex,
        context: *mut CFRunLoopSourceContext,
    ) -> CFRunLoopSourceRef;
    pub(crate) fn CFRunLoopSourceSignal(source: CFRunLoopSourceRef);
    pub(crate) fn CFStringGetSystemEncoding() -> CFStringEncoding;
}

#[link(name = "System", kind = "dylib")]
unsafe extern "C" {
    pub(crate) fn mach_msg(
        msg: *mut MachMsgHeader,
        option: i32,
        send_size: u32,
        rcv_size: u32,
        rcv_name: MachPort,
        timeout: u32,
        notify: MachPort,
    ) -> KernReturn;
    pub(crate) fn mach_error_string(error_value: KernReturn) -> *const c_char;
}

/// `mach_error_string()`.
pub(crate) fn mach_error(error_value: KernReturn) -> String {
    // SAFETY: returns a static string for any value.
    let p = unsafe { mach_error_string(error_value) };
    if p.is_null() {
        return String::new();
    }
    // SAFETY: a NUL-terminated static string.
    unsafe { std::ffi::CStr::from_ptr(p) }
        .to_string_lossy()
        .into_owned()
}

/// `CFStringGetBytes()` of a whole string as UTF-32LE, at most `max_chars`
/// characters (UTF-16 units counted as `CFStringGetLength()` does, as the
/// range is), decoded: what HIDAPI copies into its `wchar_t` buffers.
///
/// # Safety
///
/// `s` must be a valid `CFString`.
pub(crate) unsafe fn cf_string_utf32(s: CFStringRef, max_chars: usize) -> String {
    // SAFETY: the caller's contract; the buffer has room for max_chars
    // UTF-32 units, as CFStringGetBytes() is told.
    unsafe {
        let str_len = super::foundation::CFStringGetLength(s);
        let range = CFRange {
            location: 0,
            length: if str_len as usize > max_chars {
                max_chars as CFIndex
            } else {
                str_len
            },
        };
        let mut buf = vec![0u32; max_chars];
        let mut used_buf_len: CFIndex = 0;
        let chars_copied = CFStringGetBytes(
            s,
            range,
            K_CF_STRING_ENCODING_UTF32LE,
            b'?',
            0,
            buf.as_mut_ptr() as *mut u8,
            (max_chars * 4) as CFIndex,
            &mut used_buf_len,
        );
        if chars_copied <= 0 {
            return String::new();
        }
        // Note (upstream): the terminator goes at `chars_copied`, the
        // number of UTF-16 units converted, which is past the characters
        // written when the string has surrogate pairs; the units written
        // are what `used_buf_len` counts.
        buf[..(used_buf_len as usize / 4).min(max_chars)]
            .iter()
            .map(|&c| char::from_u32(u32::from_le(c)).unwrap_or('\u{fffd}'))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// ForceFeedback (<ForceFeedback/ForceFeedback.h>, ForceFeedbackConstants.h);
// the values and layouts checked against the SDK with a C harness.

/// `HRESULT`.
pub(crate) type HResult = i32;
/// `FFDeviceObjectReference`.
pub(crate) type FFDeviceObjectReference = *mut c_void;
/// `FFEffectObjectReference`.
pub(crate) type FFEffectObjectReference = *mut c_void;
/// `CFUUIDRef`.
pub(crate) type CFUUIDRef = *const c_void;

/// `FF_OK`.
pub(crate) const FF_OK: HResult = 0;
/// `FF_INFINITE`.
pub(crate) const FF_INFINITE: u32 = 0xFFFF_FFFF;
/// `FFEB_NOTRIGGER`.
pub(crate) const FFEB_NOTRIGGER: u32 = 0xFFFF_FFFF;

/// `FFJOFS_BUTTON(n)`: the offset of button `n` in `FFJOYSTATE`.
pub(crate) const fn ffjofs_button(n: u32) -> u32 {
    48 + n
}

/// `FFERR_DEVICEFULL`.
pub(crate) const FFERR_DEVICEFULL: HResult = 0x8004_0201_u32 as i32;
/// `FFERR_DEVICEPAUSED`.
pub(crate) const FFERR_DEVICEPAUSED: HResult = 0x8004_0301_u32 as i32;
/// `FFERR_DEVICERELEASED`.
pub(crate) const FFERR_DEVICERELEASED: HResult = 0x8004_0307_u32 as i32;
/// `FFERR_EFFECTPLAYING`.
pub(crate) const FFERR_EFFECTPLAYING: HResult = 0x8004_0208_u32 as i32;
/// `FFERR_EFFECTTYPEMISMATCH`.
pub(crate) const FFERR_EFFECTTYPEMISMATCH: HResult = 0x8004_0303_u32 as i32;
/// `FFERR_EFFECTTYPENOTSUPPORTED`.
pub(crate) const FFERR_EFFECTTYPENOTSUPPORTED: HResult = 0x8004_0306_u32 as i32;
/// `FFERR_GENERIC`.
pub(crate) const FFERR_GENERIC: HResult = 0x8000_0008_u32 as i32;
/// `FFERR_HASEFFECTS`.
pub(crate) const FFERR_HASEFFECTS: HResult = 0x8004_0204_u32 as i32;
/// `FFERR_INCOMPLETEEFFECT`.
pub(crate) const FFERR_INCOMPLETEEFFECT: HResult = 0x8004_0206_u32 as i32;
/// `FFERR_INTERNAL`.
pub(crate) const FFERR_INTERNAL: HResult = 0x8004_0302_u32 as i32;
/// `FFERR_INVALIDDOWNLOADID`.
pub(crate) const FFERR_INVALIDDOWNLOADID: HResult = 0x8004_0300_u32 as i32;
/// `FFERR_INVALIDPARAM`.
pub(crate) const FFERR_INVALIDPARAM: HResult = 0x8000_0003_u32 as i32;
/// `FFERR_MOREDATA`.
pub(crate) const FFERR_MOREDATA: HResult = 0x8004_0202_u32 as i32;
/// `FFERR_NOINTERFACE`.
pub(crate) const FFERR_NOINTERFACE: HResult = 0x8000_0004_u32 as i32;
/// `FFERR_NOTDOWNLOADED`.
pub(crate) const FFERR_NOTDOWNLOADED: HResult = 0x8004_0203_u32 as i32;
/// `FFERR_NOTINITIALIZED`.
pub(crate) const FFERR_NOTINITIALIZED: HResult = 0x8004_0305_u32 as i32;
/// `FFERR_OUTOFMEMORY`.
pub(crate) const FFERR_OUTOFMEMORY: HResult = 0x8000_0002_u32 as i32;
/// `FFERR_UNPLUGGED`.
pub(crate) const FFERR_UNPLUGGED: HResult = 0x8004_0209_u32 as i32;
/// `FFERR_UNSUPPORTED`.
pub(crate) const FFERR_UNSUPPORTED: HResult = 0x8000_0001_u32 as i32;
/// `FFERR_UNSUPPORTEDAXIS`.
pub(crate) const FFERR_UNSUPPORTEDAXIS: HResult = 0x8004_0304_u32 as i32;

/// `FFSFFC_RESET`.
pub(crate) const FFSFFC_RESET: u32 = 0x01;
/// `FFSFFC_STOPALL`.
pub(crate) const FFSFFC_STOPALL: u32 = 0x02;
/// `FFSFFC_PAUSE`.
pub(crate) const FFSFFC_PAUSE: u32 = 0x04;
/// `FFSFFC_CONTINUE`.
pub(crate) const FFSFFC_CONTINUE: u32 = 0x08;
/// `FFSFFC_SETACTUATORSON`.
pub(crate) const FFSFFC_SETACTUATORSON: u32 = 0x10;

/// `FFEFF_OBJECTOFFSETS`.
pub(crate) const FFEFF_OBJECTOFFSETS: u32 = 0x02;
/// `FFEFF_CARTESIAN`.
pub(crate) const FFEFF_CARTESIAN: u32 = 0x10;
/// `FFEFF_POLAR`.
pub(crate) const FFEFF_POLAR: u32 = 0x20;
/// `FFEFF_SPHERICAL`.
pub(crate) const FFEFF_SPHERICAL: u32 = 0x40;

/// `FFEP_DURATION`.
pub(crate) const FFEP_DURATION: u32 = 0x01;
/// `FFEP_TRIGGERBUTTON`.
pub(crate) const FFEP_TRIGGERBUTTON: u32 = 0x08;
/// `FFEP_TRIGGERREPEATINTERVAL`.
pub(crate) const FFEP_TRIGGERREPEATINTERVAL: u32 = 0x10;
/// `FFEP_DIRECTION`.
pub(crate) const FFEP_DIRECTION: u32 = 0x40;
/// `FFEP_ENVELOPE`.
pub(crate) const FFEP_ENVELOPE: u32 = 0x80;
/// `FFEP_TYPESPECIFICPARAMS`.
pub(crate) const FFEP_TYPESPECIFICPARAMS: u32 = 0x100;
/// `FFEP_STARTDELAY`.
pub(crate) const FFEP_STARTDELAY: u32 = 0x200;

/// `FFPROP_FFGAIN`.
pub(crate) const FFPROP_FFGAIN: u32 = 1;
/// `FFPROP_AUTOCENTER`.
pub(crate) const FFPROP_AUTOCENTER: u32 = 3;

/// `FFCAP_ET_CONSTANTFORCE`.
pub(crate) const FFCAP_ET_CONSTANTFORCE: u32 = 0x001;
/// `FFCAP_ET_RAMPFORCE`.
pub(crate) const FFCAP_ET_RAMPFORCE: u32 = 0x002;
/// `FFCAP_ET_SQUARE`.
pub(crate) const FFCAP_ET_SQUARE: u32 = 0x004;
/// `FFCAP_ET_SINE`.
pub(crate) const FFCAP_ET_SINE: u32 = 0x008;
/// `FFCAP_ET_TRIANGLE`.
pub(crate) const FFCAP_ET_TRIANGLE: u32 = 0x010;
/// `FFCAP_ET_SAWTOOTHUP`.
pub(crate) const FFCAP_ET_SAWTOOTHUP: u32 = 0x020;
/// `FFCAP_ET_SAWTOOTHDOWN`.
pub(crate) const FFCAP_ET_SAWTOOTHDOWN: u32 = 0x040;
/// `FFCAP_ET_SPRING`.
pub(crate) const FFCAP_ET_SPRING: u32 = 0x080;
/// `FFCAP_ET_DAMPER`.
pub(crate) const FFCAP_ET_DAMPER: u32 = 0x100;
/// `FFCAP_ET_INERTIA`.
pub(crate) const FFCAP_ET_INERTIA: u32 = 0x200;
/// `FFCAP_ET_FRICTION`.
pub(crate) const FFCAP_ET_FRICTION: u32 = 0x400;
/// `FFCAP_ET_CUSTOMFORCE`.
pub(crate) const FFCAP_ET_CUSTOMFORCE: u32 = 0x800;

/// `FFENVELOPE`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FFENVELOPE {
    pub(crate) dw_size: u32,
    pub(crate) dw_attack_level: u32,
    pub(crate) dw_attack_time: u32,
    pub(crate) dw_fade_level: u32,
    pub(crate) dw_fade_time: u32,
}

/// `FFCONSTANTFORCE`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FFCONSTANTFORCE {
    pub(crate) l_magnitude: i32,
}

/// `FFPERIODIC`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FFPERIODIC {
    pub(crate) dw_magnitude: u32,
    pub(crate) l_offset: i32,
    pub(crate) dw_phase: u32,
    pub(crate) dw_period: u32,
}

/// `FFCONDITION`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FFCONDITION {
    pub(crate) l_offset: i32,
    pub(crate) l_positive_coefficient: i32,
    pub(crate) l_negative_coefficient: i32,
    pub(crate) dw_positive_saturation: u32,
    pub(crate) dw_negative_saturation: u32,
    pub(crate) l_dead_band: i32,
}

/// `FFRAMPFORCE`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FFRAMPFORCE {
    pub(crate) l_start: i32,
    pub(crate) l_end: i32,
}

/// `FFCUSTOMFORCE`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct FFCUSTOMFORCE {
    pub(crate) c_channels: u32,
    pub(crate) dw_sample_period: u32,
    pub(crate) c_samples: u32,
    pub(crate) rgl_force_data: *mut i32,
}

/// `FFEFFECT`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct FFEFFECT {
    pub(crate) dw_size: u32,
    pub(crate) dw_flags: u32,
    pub(crate) dw_duration: u32,
    pub(crate) dw_sample_period: u32,
    pub(crate) dw_gain: u32,
    pub(crate) dw_trigger_button: u32,
    pub(crate) dw_trigger_repeat_interval: u32,
    pub(crate) c_axes: u32,
    pub(crate) rgdw_axes: *mut u32,
    pub(crate) rgl_direction: *mut i32,
    pub(crate) lp_envelope: *mut FFENVELOPE,
    pub(crate) cb_type_specific_params: u32,
    pub(crate) lpv_type_specific_params: *mut c_void,
    pub(crate) dw_start_delay: u32,
}

impl Default for FFEFFECT {
    /// All zeros (`SDL_zero()`).
    fn default() -> FFEFFECT {
        FFEFFECT {
            dw_size: 0,
            dw_flags: 0,
            dw_duration: 0,
            dw_sample_period: 0,
            dw_gain: 0,
            dw_trigger_button: 0,
            dw_trigger_repeat_interval: 0,
            c_axes: 0,
            rgdw_axes: std::ptr::null_mut(),
            rgl_direction: std::ptr::null_mut(),
            lp_envelope: std::ptr::null_mut(),
            cb_type_specific_params: 0,
            lpv_type_specific_params: std::ptr::null_mut(),
            dw_start_delay: 0,
        }
    }
}

/// `FFCAPABILITIES`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FFCAPABILITIES {
    pub(crate) ff_spec_ver: [u8; 4],
    pub(crate) supported_effects: u32,
    pub(crate) emulated_effects: u32,
    pub(crate) sub_type: u32,
    pub(crate) num_ff_axes: u32,
    pub(crate) ff_axes: [u8; 32],
    pub(crate) storage_capacity: u32,
    pub(crate) playback_capacity: u32,
    pub(crate) firmware_ver: [u8; 4],
    pub(crate) hardware_ver: [u8; 4],
    pub(crate) driver_ver: [u8; 4],
}

#[link(name = "ForceFeedback", kind = "framework")]
unsafe extern "C" {
    pub(crate) fn FFIsForceFeedback(hid_device: IoService) -> HResult;
    pub(crate) fn FFCreateDevice(
        hid_device: IoService,
        device: *mut FFDeviceObjectReference,
    ) -> HResult;
    pub(crate) fn FFReleaseDevice(device: FFDeviceObjectReference) -> HResult;
    pub(crate) fn FFDeviceCreateEffect(
        device: FFDeviceObjectReference,
        uuid: CFUUIDRef,
        effect_definition: *mut FFEFFECT,
        effect: *mut FFEffectObjectReference,
    ) -> HResult;
    pub(crate) fn FFDeviceReleaseEffect(
        device: FFDeviceObjectReference,
        effect: FFEffectObjectReference,
    ) -> HResult;
    pub(crate) fn FFDeviceSendForceFeedbackCommand(
        device: FFDeviceObjectReference,
        command: u32,
    ) -> HResult;
    pub(crate) fn FFDeviceGetForceFeedbackCapabilities(
        device: FFDeviceObjectReference,
        capabilities: *mut FFCAPABILITIES,
    ) -> HResult;
    pub(crate) fn FFDeviceGetForceFeedbackProperty(
        device: FFDeviceObjectReference,
        property: u32,
        value: *mut c_void,
        value_size: u64,
    ) -> HResult;
    pub(crate) fn FFDeviceSetForceFeedbackProperty(
        device: FFDeviceObjectReference,
        property: u32,
        value: *mut c_void,
    ) -> HResult;
    pub(crate) fn FFEffectStart(
        effect: FFEffectObjectReference,
        iterations: u32,
        flags: u32,
    ) -> HResult;
    pub(crate) fn FFEffectStop(effect: FFEffectObjectReference) -> HResult;
    pub(crate) fn FFEffectSetParameters(
        effect: FFEffectObjectReference,
        effect_definition: *mut FFEFFECT,
        flags: u32,
    ) -> HResult;
    pub(crate) fn FFEffectGetEffectStatus(
        effect: FFEffectObjectReference,
        status: *mut u32,
    ) -> HResult;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFUUIDGetConstantUUIDWithBytes(
        alloc: CFAllocatorRef,
        byte0: u8,
        byte1: u8,
        byte2: u8,
        byte3: u8,
        byte4: u8,
        byte5: u8,
        byte6: u8,
        byte7: u8,
        byte8: u8,
        byte9: u8,
        byte10: u8,
        byte11: u8,
        byte12: u8,
        byte13: u8,
        byte14: u8,
        byte15: u8,
    ) -> CFUUIDRef;
}

/// The `kFFEffectType_*_ID` UUIDs, which differ in their fourth byte
/// (`0x60` for `ConstantForce` to `0x6b` for `CustomForce`).
pub(crate) fn ff_effect_type_uuid(fourth_byte: u8) -> CFUUIDRef {
    // SAFETY: returns a constant UUID object the runtime keeps.
    unsafe {
        CFUUIDGetConstantUUIDWithBytes(
            std::ptr::null(),
            0xE5,
            0x59,
            0xC4,
            fourth_byte,
            0xC5,
            0xCD,
            0x11,
            0xD6,
            0x8A,
            0x1C,
            0x00,
            0x03,
            0x93,
            0x53,
            0xBD,
            0x00,
        )
    }
}

/// `kFFEffectType_ConstantForce_ID`.
pub(crate) fn k_ff_effect_type_constant_force_id() -> CFUUIDRef {
    ff_effect_type_uuid(0x60)
}
/// `kFFEffectType_RampForce_ID`.
pub(crate) fn k_ff_effect_type_ramp_force_id() -> CFUUIDRef {
    ff_effect_type_uuid(0x61)
}
/// `kFFEffectType_Square_ID`.
pub(crate) fn k_ff_effect_type_square_id() -> CFUUIDRef {
    ff_effect_type_uuid(0x62)
}
/// `kFFEffectType_Sine_ID`.
pub(crate) fn k_ff_effect_type_sine_id() -> CFUUIDRef {
    ff_effect_type_uuid(0x63)
}
/// `kFFEffectType_Triangle_ID`.
pub(crate) fn k_ff_effect_type_triangle_id() -> CFUUIDRef {
    ff_effect_type_uuid(0x64)
}
/// `kFFEffectType_SawtoothUp_ID`.
pub(crate) fn k_ff_effect_type_sawtooth_up_id() -> CFUUIDRef {
    ff_effect_type_uuid(0x65)
}
/// `kFFEffectType_SawtoothDown_ID`.
pub(crate) fn k_ff_effect_type_sawtooth_down_id() -> CFUUIDRef {
    ff_effect_type_uuid(0x66)
}
/// `kFFEffectType_Spring_ID`.
pub(crate) fn k_ff_effect_type_spring_id() -> CFUUIDRef {
    ff_effect_type_uuid(0x67)
}
/// `kFFEffectType_Damper_ID`.
pub(crate) fn k_ff_effect_type_damper_id() -> CFUUIDRef {
    ff_effect_type_uuid(0x68)
}
/// `kFFEffectType_Inertia_ID`.
pub(crate) fn k_ff_effect_type_inertia_id() -> CFUUIDRef {
    ff_effect_type_uuid(0x69)
}
/// `kFFEffectType_Friction_ID`.
pub(crate) fn k_ff_effect_type_friction_id() -> CFUUIDRef {
    ff_effect_type_uuid(0x6A)
}
/// `kFFEffectType_CustomForce_ID`.
pub(crate) fn k_ff_effect_type_custom_force_id() -> CFUUIDRef {
    ff_effect_type_uuid(0x6B)
}

/// Like strerror but for force feedback errors. Translation of
/// `FFStrError()` (of the IOKit joystick and Darwin haptic drivers).
pub(crate) fn ff_str_error(err: HResult) -> &'static str {
    match err {
        FFERR_DEVICEFULL => "device full",
        // This should be valid, but for some reason isn't defined...
        /* case FFERR_DEVICENOTREG:
        return "device not registered"; */
        FFERR_DEVICEPAUSED => "device paused",
        FFERR_DEVICERELEASED => "device released",
        FFERR_EFFECTPLAYING => "effect playing",
        FFERR_EFFECTTYPEMISMATCH => "effect type mismatch",
        FFERR_EFFECTTYPENOTSUPPORTED => "effect type not supported",
        FFERR_GENERIC => "undetermined error",
        FFERR_HASEFFECTS => "device has effects",
        FFERR_INCOMPLETEEFFECT => "incomplete effect",
        FFERR_INTERNAL => "internal fault",
        FFERR_INVALIDDOWNLOADID => "invalid download id",
        FFERR_INVALIDPARAM => "invalid parameter",
        FFERR_MOREDATA => "more data",
        FFERR_NOINTERFACE => "interface not supported",
        FFERR_NOTDOWNLOADED => "effect is not downloaded",
        FFERR_NOTINITIALIZED => "object has not been initialized",
        FFERR_OUTOFMEMORY => "out of memory",
        FFERR_UNPLUGGED => "device is unplugged",
        FFERR_UNSUPPORTED => "function call unsupported",
        FFERR_UNSUPPORTEDAXIS => "axis unsupported",

        _ => "unknown error",
    }
}

// The HID usage tables (<IOKit/hid/IOHIDUsageTables.h>) the drivers look at.

/// `kHIDPage_GenericDesktop`.
pub(crate) const K_HID_PAGE_GENERIC_DESKTOP: u32 = 0x01;
/// `kHIDPage_Simulation`.
pub(crate) const K_HID_PAGE_SIMULATION: u32 = 0x02;
/// `kHIDPage_Button`.
pub(crate) const K_HID_PAGE_BUTTON: u32 = 0x09;
/// `kHIDPage_Consumer`.
pub(crate) const K_HID_PAGE_CONSUMER: u32 = 0x0C;
/// `kHIDUsage_GD_Mouse`.
pub(crate) const K_HID_USAGE_GD_MOUSE: u32 = 0x02;
/// `kHIDUsage_GD_Joystick`.
pub(crate) const K_HID_USAGE_GD_JOYSTICK: u32 = 0x04;
/// `kHIDUsage_GD_GamePad`.
pub(crate) const K_HID_USAGE_GD_GAMEPAD: u32 = 0x05;
/// `kHIDUsage_GD_MultiAxisController`.
pub(crate) const K_HID_USAGE_GD_MULTI_AXIS_CONTROLLER: u32 = 0x08;
/// `kHIDUsage_GD_X`.
pub(crate) const K_HID_USAGE_GD_X: u32 = 0x30;
/// `kHIDUsage_GD_Y`.
pub(crate) const K_HID_USAGE_GD_Y: u32 = 0x31;
/// `kHIDUsage_GD_Z`.
pub(crate) const K_HID_USAGE_GD_Z: u32 = 0x32;
/// `kHIDUsage_GD_Rx`.
pub(crate) const K_HID_USAGE_GD_RX: u32 = 0x33;
/// `kHIDUsage_GD_Ry`.
pub(crate) const K_HID_USAGE_GD_RY: u32 = 0x34;
/// `kHIDUsage_GD_Rz`.
pub(crate) const K_HID_USAGE_GD_RZ: u32 = 0x35;
/// `kHIDUsage_GD_Slider`.
pub(crate) const K_HID_USAGE_GD_SLIDER: u32 = 0x36;
/// `kHIDUsage_GD_Dial`.
pub(crate) const K_HID_USAGE_GD_DIAL: u32 = 0x37;
/// `kHIDUsage_GD_Wheel`.
pub(crate) const K_HID_USAGE_GD_WHEEL: u32 = 0x38;
/// `kHIDUsage_GD_Hatswitch`.
pub(crate) const K_HID_USAGE_GD_HATSWITCH: u32 = 0x39;
/// `kHIDUsage_GD_Start`.
pub(crate) const K_HID_USAGE_GD_START: u32 = 0x3D;
/// `kHIDUsage_GD_Select`.
pub(crate) const K_HID_USAGE_GD_SELECT: u32 = 0x3E;
/// `kHIDUsage_GD_SystemMainMenu`.
pub(crate) const K_HID_USAGE_GD_SYSTEM_MAIN_MENU: u32 = 0x85;
/// `kHIDUsage_GD_DPadUp`.
pub(crate) const K_HID_USAGE_GD_DPAD_UP: u32 = 0x90;
/// `kHIDUsage_GD_DPadDown`.
pub(crate) const K_HID_USAGE_GD_DPAD_DOWN: u32 = 0x91;
/// `kHIDUsage_GD_DPadRight`.
pub(crate) const K_HID_USAGE_GD_DPAD_RIGHT: u32 = 0x92;
/// `kHIDUsage_GD_DPadLeft`.
pub(crate) const K_HID_USAGE_GD_DPAD_LEFT: u32 = 0x93;
/// `kHIDUsage_Sim_Rudder`.
pub(crate) const K_HID_USAGE_SIM_RUDDER: u32 = 0xBA;
/// `kHIDUsage_Sim_Throttle`.
pub(crate) const K_HID_USAGE_SIM_THROTTLE: u32 = 0xBB;
/// `kHIDUsage_Sim_Accelerator`.
pub(crate) const K_HID_USAGE_SIM_ACCELERATOR: u32 = 0xC4;
/// `kHIDUsage_Sim_Brake`.
pub(crate) const K_HID_USAGE_SIM_BRAKE: u32 = 0xC5;

#[cfg(test)]
mod tests {
    use super::*;

    /// The layouts the C harness printed for the SDK's structures.
    #[test]
    fn force_feedback_layouts() {
        assert_eq!(size_of::<FFEFFECT>(), 80);
        assert_eq!(std::mem::offset_of!(FFEFFECT, rgdw_axes), 32);
        assert_eq!(std::mem::offset_of!(FFEFFECT, lpv_type_specific_params), 64);
        assert_eq!(std::mem::offset_of!(FFEFFECT, dw_start_delay), 72);
        assert_eq!(size_of::<FFENVELOPE>(), 20);
        assert_eq!(size_of::<FFPERIODIC>(), 16);
        assert_eq!(size_of::<FFCONDITION>(), 24);
        assert_eq!(size_of::<FFRAMPFORCE>(), 8);
        assert_eq!(size_of::<FFCUSTOMFORCE>(), 24);
        assert_eq!(std::mem::offset_of!(FFCUSTOMFORCE, rgl_force_data), 16);
        assert_eq!(size_of::<FFCAPABILITIES>(), 72);
        assert_eq!(std::mem::offset_of!(FFCAPABILITIES, ff_axes), 20);
        assert_eq!(std::mem::offset_of!(FFCAPABILITIES, storage_capacity), 52);
        assert_eq!(std::mem::offset_of!(FFCAPABILITIES, driver_ver), 68);
        assert_eq!(size_of::<CFRunLoopSourceContext>(), 80);
        assert_eq!(size_of::<MachMsgHeader>(), 24);
        assert_eq!(ffjofs_button(3), 51);
    }

    #[test]
    fn effect_type_uuids_are_constants() {
        // The same object each time, and the twelve are distinct.
        assert_eq!(k_ff_effect_type_sine_id(), k_ff_effect_type_sine_id());
        let all = [
            k_ff_effect_type_constant_force_id(),
            k_ff_effect_type_ramp_force_id(),
            k_ff_effect_type_square_id(),
            k_ff_effect_type_sine_id(),
            k_ff_effect_type_triangle_id(),
            k_ff_effect_type_sawtooth_up_id(),
            k_ff_effect_type_sawtooth_down_id(),
            k_ff_effect_type_spring_id(),
            k_ff_effect_type_damper_id(),
            k_ff_effect_type_inertia_id(),
            k_ff_effect_type_friction_id(),
            k_ff_effect_type_custom_force_id(),
        ];
        for (i, a) in all.iter().enumerate() {
            assert!(!a.is_null());
            for b in &all[i + 1..] {
                assert_ne!(a, b);
            }
        }
        assert_eq!(ff_str_error(FFERR_UNPLUGGED), "device is unplugged");
        assert_eq!(ff_str_error(FFERR_GENERIC), "undetermined error");
        assert_eq!(ff_str_error(12345), "unknown error");
    }
}
