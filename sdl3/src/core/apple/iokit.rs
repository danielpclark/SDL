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
    pub(crate) fn IOIteratorNext(iterator: IoIterator) -> IoObject;
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
