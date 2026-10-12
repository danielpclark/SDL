// Rust translation of src/hidapi/mac/hid.c (and hidapi_darwin.h) from
// Simple DirectMedia Layer's copy of HIDAPI.
// HIDAPI - Multi-Platform library for communication with HID devices.
// Alan Ott, Signal 11 Software; libusb/hidapi Team.
// Copyright 2022, All Rights Reserved.
// Used under the original HIDAPI license / BSD-style license (see LICENSE.txt).
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The macOS backend: devices through the IOHIDManager (see Apple Technical
//! Note TN2187), each open device with a thread running its own run loop to
//! collect input reports.
//!
//! Paths are `DevSrvsID:<RegistryEntryID>` (as enumerated); an I/O Registry
//! path in the IOService plane is accepted too, as older HIDAPIs returned.

use std::collections::VecDeque;
use std::ffi::{c_void, CString};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};
use std::sync::{Arc, Barrier, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::{BusType, DeviceInfo};
use crate::core::apple::foundation::{
    cf_string, kCFAllocatorDefault, kCFRunLoopDefaultMode, kCFTypeDictionaryKeyCallBacks,
    kCFTypeDictionaryValueCallBacks, string_from_cf, CFArrayGetCount, CFArrayGetTypeID,
    CFArrayGetValueAtIndex, CFArrayRef, CFDataGetBytePtr, CFDataGetLength, CFDataGetTypeID,
    CFDictionaryCreateMutable, CFDictionaryGetTypeID, CFDictionaryGetValueIfPresent,
    CFDictionarySetValue, CFGetTypeID, CFIndex, CFNumberCreate, CFNumberGetTypeID,
    CFNumberGetValue, CFRelease, CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopGetMain,
    CFRunLoopRef, CFRunLoopRunInMode, CFRunLoopSourceRef, CFRunLoopStop, CFRunLoopWakeUp,
    CFStringCreateWithCString, CFStringGetTypeID, CFTypeRef, CfRef, K_CF_NUMBER_SINT32_TYPE,
    K_CF_RUN_LOOP_RUN_FINISHED, K_CF_RUN_LOOP_RUN_HANDLED_SOURCE, K_CF_RUN_LOOP_RUN_STOPPED,
    K_CF_RUN_LOOP_RUN_TIMED_OUT, K_CF_STRING_ENCODING_ASCII,
};
use crate::core::apple::iokit::*;
use crate::error::{Error, Result};

// - Run context -

/// `hid_mgr`.
struct HidManager(IOHIDManagerRef);

// SAFETY: the manager is only used with the mutex held.
unsafe impl Send for HidManager {}

static HID_MGR: Mutex<Option<HidManager>> = Mutex::new(None);
/// `is_macos_10_10_or_greater`.
static IS_MACOS_10_10_OR_GREATER: AtomicBool = AtomicBool::new(false);
/// `device_open_options`.
static DEVICE_OPEN_OPTIONS: AtomicU32 = AtomicU32::new(0);

fn lock_mgr() -> MutexGuard<'static, Option<HidManager>> {
    HID_MGR.lock().unwrap_or_else(|e| e.into_inner())
}

/// The `BUF_LEN` of the strings' `wchar_t` buffers (one is the terminator).
const BUF_LEN: usize = 256;

/// Translation of `get_array_property()`.
///
/// # Safety
///
/// `device` must be a valid device.
unsafe fn get_array_property(device: IOHIDDeviceRef, key: &str) -> CFArrayRef {
    let key = cf_string(key);
    // SAFETY: the caller's contract; the property is owned by the device.
    unsafe {
        let r = IOHIDDeviceGetProperty(device, key.get());
        if !r.is_null() && CFGetTypeID(r) == CFArrayGetTypeID() {
            r
        } else {
            std::ptr::null()
        }
    }
}

/// Translation of `get_int_property()`.
///
/// # Safety
///
/// `device` must be a valid device.
unsafe fn get_int_property(device: IOHIDDeviceRef, key: &str) -> i32 {
    // (the value CFNumberGetValue() stores, even when it reports a lossy
    // conversion)
    // SAFETY: the caller's contract.
    unsafe { int_property(device, key) }.map_or(0, |(value, _)| value)
}

/// Translation of `try_get_int_property()`.
///
/// # Safety
///
/// `device` must be a valid device.
unsafe fn try_get_int_property(device: IOHIDDeviceRef, key: &str) -> Option<i32> {
    // SAFETY: the caller's contract.
    match unsafe { int_property(device, key) } {
        Some((value, true)) => Some(value),
        _ => None,
    }
}

/// A number property as `kCFNumberSInt32Type`, with whether
/// `CFNumberGetValue()` converted it exactly.
///
/// # Safety
///
/// `device` must be a valid device.
unsafe fn int_property(device: IOHIDDeviceRef, key: &str) -> Option<(i32, bool)> {
    let key = cf_string(key);
    // SAFETY: the caller's contract; the out pointer is an i32 as the
    // number type says.
    unsafe {
        let r = IOHIDDeviceGetProperty(device, key.get());
        if !r.is_null() && CFGetTypeID(r) == CFNumberGetTypeID() {
            let mut value = 0i32;
            let ok = CFNumberGetValue(
                r,
                K_CF_NUMBER_SINT32_TYPE,
                &mut value as *mut i32 as *mut c_void,
            );
            return Some((value, ok != 0));
        }
        None
    }
}

/// Translation of `try_get_ioregistry_int_property()`.
///
/// # Safety
///
/// `service` must be a valid registry entry.
unsafe fn try_get_ioregistry_int_property(service: IoService, property: &str) -> Option<i32> {
    let key = cf_string(property);
    // SAFETY: the caller's contract; the property is ours to release.
    unsafe {
        let r = IORegistryEntryCreateCFProperty(service, key.get(), kCFAllocatorDefault, 0);
        let r = CfRef::from_owned(r)?;
        if CFGetTypeID(r.get()) == CFNumberGetTypeID() {
            let mut value = 0i32;
            if CFNumberGetValue(
                r.get(),
                K_CF_NUMBER_SINT32_TYPE,
                &mut value as *mut i32 as *mut c_void,
            ) != 0
            {
                return Some(value);
            }
        }
        None
    }
}

/// Translation of `get_usage_pairs()`.
///
/// # Safety
///
/// `device` must be a valid device.
unsafe fn get_usage_pairs(device: IOHIDDeviceRef) -> CFArrayRef {
    // SAFETY: the caller's contract.
    unsafe { get_array_property(device, K_IOHID_DEVICE_USAGE_PAIRS_KEY) }
}

/// Translation of `get_vendor_id()`.
///
/// # Safety
///
/// `device` must be a valid device.
unsafe fn get_vendor_id(device: IOHIDDeviceRef) -> u16 {
    // SAFETY: the caller's contract.
    unsafe { get_int_property(device, K_IOHID_VENDOR_ID_KEY) as u16 }
}

/// Translation of `get_product_id()`.
///
/// # Safety
///
/// `device` must be a valid device.
unsafe fn get_product_id(device: IOHIDDeviceRef) -> u16 {
    // SAFETY: the caller's contract.
    unsafe { get_int_property(device, K_IOHID_PRODUCT_ID_KEY) as u16 }
}

/// Translation of `get_max_report_length()`.
///
/// # Safety
///
/// `device` must be a valid device.
unsafe fn get_max_report_length(device: IOHIDDeviceRef) -> i32 {
    // SAFETY: the caller's contract.
    unsafe { get_int_property(device, K_IOHID_MAX_INPUT_REPORT_SIZE_KEY) }
}

/// Translation of `get_string_property()`: the string (truncated to the
/// `BUF_LEN - 1` characters of upstream's buffer), or `None` when the
/// device has no such string (where upstream leaves an empty buffer).
///
/// # Safety
///
/// `device` must be a valid device.
unsafe fn get_string_property(device: IOHIDDeviceRef, prop: &str) -> Option<String> {
    let key = cf_string(prop);
    // SAFETY: the caller's contract; the string is owned by the device.
    unsafe {
        let s = IOHIDDeviceGetProperty(device, key.get());
        if !s.is_null() && CFGetTypeID(s) == CFStringGetTypeID() {
            Some(cf_string_utf32(s, BUF_LEN - 1))
        } else {
            None
        }
    }
}

/// The string a `wchar_t` buffer holds after `get_*_string()`: empty when
/// the property is missing (`dup_wcs(buf)` of a cleared buffer).
fn dup_wcs(s: Option<String>) -> Option<String> {
    Some(s.unwrap_or_default())
}

/// Translation of `init_hid_manager()`.
fn init_hid_manager(mgr: &mut Option<HidManager>) -> Result<()> {
    // Initialize all the HID Manager Objects
    // SAFETY: creating and scheduling a new manager on this thread's run
    // loop.
    unsafe {
        let m = IOHIDManagerCreate(kCFAllocatorDefault, K_IOHID_OPTIONS_TYPE_NONE);
        if !m.is_null() {
            IOHIDManagerSetDeviceMatching(m, std::ptr::null());
            IOHIDManagerScheduleWithRunLoop(m, CFRunLoopGetCurrent(), kCFRunLoopDefaultMode);
            *mgr = Some(HidManager(m));
            return Ok(());
        }
    }

    Err(Error::new("Failed to create IOHIDManager"))
}

/// Initialize the IOHIDManager if necessary. This is the public function,
/// and it is safe to call this function repeatedly. Translation of
/// `hid_init()`.
pub(super) fn hid_init() -> Result<()> {
    hid_init_locked(&mut lock_mgr())
}

fn hid_init_locked(mgr: &mut Option<HidManager>) -> Result<()> {
    if mgr.is_none() {
        // SAFETY: a constant CoreFoundation exports.
        let version = unsafe { kCFCoreFoundationVersionNumber };
        IS_MACOS_10_10_OR_GREATER.store(version >= 1151.16, Ordering::Relaxed); // kCFCoreFoundationVersionNumber10_10
        hid_darwin_set_open_exclusive(true); // Backward compatibility
        return init_hid_manager(mgr);
    }

    // Already initialized.
    Ok(())
}

/// Translation of `hid_exit()`.
pub(super) fn hid_exit() -> Result<()> {
    if let Some(m) = lock_mgr().take() {
        // Close the HID manager.
        // SAFETY: the manager we created, released once.
        unsafe {
            IOHIDManagerClose(m.0, K_IOHID_OPTIONS_TYPE_NONE);
            CFRelease(m.0);
        }
    }

    Ok(())
}

/// Translation of `process_pending_events()`.
fn process_pending_events() {
    loop {
        // SAFETY: runs this thread's run loop briefly.
        let res = unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.001, 0) };
        if res == K_CF_RUN_LOOP_RUN_FINISHED || res == K_CF_RUN_LOOP_RUN_TIMED_OUT {
            break;
        }
    }
}

/// Translation of `read_usb_interface_from_hid_service_parent()`.
///
/// # Safety
///
/// `hid_service` must be a valid registry entry.
unsafe fn read_usb_interface_from_hid_service_parent(hid_service: IoService) -> i32 {
    let mut result = -1;
    let mut current: IoRegistryEntry = IO_OBJECT_NULL;
    let mut parent_number = 0;

    // SAFETY: the caller's contract; each entry the registry hands out is
    // released once.
    unsafe {
        let mut res =
            IORegistryEntryGetParentEntry(hid_service, K_IO_SERVICE_PLANE.as_ptr(), &mut current);
        while KERN_SUCCESS == res
            /* Only search up to 3 parent entries.
             * With the default driver - the parent-of-interest supposed to be the first one,
             * but lets assume some custom drivers or so, with deeper tree. */
            && parent_number < 3
        {
            let mut parent: IoRegistryEntry = IO_OBJECT_NULL;
            parent_number += 1;

            if let Some(interface_number) =
                try_get_ioregistry_int_property(current, K_USB_INTERFACE_NUMBER)
            {
                result = interface_number;
                break;
            }

            res = IORegistryEntryGetParentEntry(current, K_IO_SERVICE_PLANE.as_ptr(), &mut parent);
            if parent != 0 {
                IOObjectRelease(current);
                current = parent;
            }
        }

        if current != 0 {
            IOObjectRelease(current);
        }
    }

    result
}

/// The bus a transport string names (the `kIOHIDTransportKey` checks of
/// `get_bus_type()` and `create_device_info_with_usage()`).
fn bus_type_of_transport(transport: &str) -> BusType {
    if transport == K_IOHID_TRANSPORT_USB_VALUE {
        BusType::Usb
    } else if transport.starts_with(K_IOHID_TRANSPORT_BLUETOOTH_VALUE) {
        // Match "Bluetooth", "BluetoothLowEnergy" and "Bluetooth Low Energy" strings
        BusType::Bluetooth
    } else if transport == K_IOHID_TRANSPORT_I2C_VALUE {
        BusType::I2c
    } else if transport == K_IOHID_TRANSPORT_SPI_VALUE {
        BusType::Spi
    } else {
        BusType::Unknown
    }
}

/// The device's transport, when it is a string.
///
/// # Safety
///
/// `dev` must be a valid device.
unsafe fn transport(dev: IOHIDDeviceRef) -> Option<String> {
    let key = cf_string(K_IOHID_TRANSPORT_KEY);
    // SAFETY: the caller's contract; the property is owned by the device.
    unsafe {
        let transport_prop = IOHIDDeviceGetProperty(dev, key.get());
        if !transport_prop.is_null() && CFGetTypeID(transport_prop) == CFStringGetTypeID() {
            string_from_cf(transport_prop)
        } else {
            None
        }
    }
}

/// Translation of `get_bus_type()`.
///
/// # Safety
///
/// `dev` must be a valid device.
unsafe fn get_bus_type(dev: IOHIDDeviceRef) -> BusType {
    // SAFETY: the caller's contract.
    match unsafe { transport(dev) } {
        Some(t) => bus_type_of_transport(&t),
        None => BusType::Unknown,
    }
}

/// The path of a registry entry ID (`"DevSrvsID:%llu"`).
fn path_of_entry_id(entry_id: u64) -> String {
    format!("DevSrvsID:{entry_id}")
}

/// Translation of `create_device_info_with_usage()`.
///
/// # Safety
///
/// `dev` must be a valid device.
unsafe fn create_device_info_with_usage(
    dev: IOHIDDeviceRef,
    usage_page: i32,
    usage: i32,
) -> Option<DeviceInfo> {
    if dev.is_null() {
        return None;
    }

    // SAFETY: the caller's contract for every device query below.
    unsafe {
        let dev_vid = get_vendor_id(dev);
        let dev_pid = get_product_id(dev);
        let mut dev_version = get_int_property(dev, K_IOHID_VERSION_NUMBER_KEY) as u16;

        // See if there are any devices we should skip in enumeration
        if super::should_ignore_device(
            get_bus_type(dev),
            dev_vid,
            dev_pid,
            usage_page as u16,
            usage as u16,
            false,
            false,
        ) {
            return None;
        }

        if crate::joystick::is_joystick_steam_virtual_gamepad(dev_vid, dev_pid, dev_version) {
            let key = cf_string(K_IOHID_VIRTUAL_HIDEVICE);
            if IOHIDDeviceGetProperty(dev, key.get())
                != crate::core::apple::foundation::kCFBooleanTrue
            {
                // This is a real Xbox 360 controller, adjust the version so it's not detected as a Steam virtual gamepad
                dev_version = 1;
            }
        }

        let mut cur_dev = DeviceInfo {
            usage_page: usage_page as u16,
            usage: usage as u16,
            ..DeviceInfo::default()
        };

        // Fill in the path (as a unique ID of the service entry)
        let hid_service = IOHIDDeviceGetService(dev);
        let mut entry_id = 0u64;
        let res = if hid_service != MACH_PORT_NULL {
            IORegistryEntryGetRegistryEntryID(hid_service, &mut entry_id)
        } else {
            KERN_INVALID_ARGUMENT
        };

        cur_dev.path = Some(if res == KERN_SUCCESS {
            path_of_entry_id(entry_id)
        } else {
            // for whatever reason, trying to keep it a non-NULL string
            String::new()
        });

        // Serial Number
        cur_dev.serial_number = dup_wcs(get_string_property(dev, K_IOHID_SERIAL_NUMBER_KEY));

        // Manufacturer and Product strings
        cur_dev.manufacturer_string = dup_wcs(get_string_property(dev, K_IOHID_MANUFACTURER_KEY));
        cur_dev.product_string = dup_wcs(get_string_property(dev, K_IOHID_PRODUCT_KEY));

        // VID/PID
        cur_dev.vendor_id = dev_vid;
        cur_dev.product_id = dev_pid;

        // Release Number
        cur_dev.release_number = dev_version;

        /* Interface Number.
         * We can only retrieve the interface number for USB HID devices.
         * See below */
        cur_dev.interface_number = -1;

        // Bus Type
        if let Some(t) = transport(dev) {
            cur_dev.bus_type = bus_type_of_transport(&t);
            if cur_dev.bus_type == BusType::Usb {
                /* A IOHIDDeviceRef used to have this simple property,
                 * until macOS 13.3 - we will try to use it. */
                if let Some(interface_number) = try_get_int_property(dev, K_USB_INTERFACE_NUMBER) {
                    cur_dev.interface_number = interface_number;
                } else {
                    /* Otherwise fallback to io_service_t property.
                     * (of one of the parent services). */
                    cur_dev.interface_number =
                        read_usb_interface_from_hid_service_parent(hid_service);

                    /* If the above doesn't work -
                     * no (known) fallback exists at this point. */
                }
            }
        }

        Some(cur_dev)
    }
}

/// Translation of `create_device_info()`.
///
/// # Safety
///
/// `device` must be a valid device.
unsafe fn create_device_info(device: IOHIDDeviceRef) -> Vec<DeviceInfo> {
    // SAFETY: the caller's contract; the usage pairs belong to the device.
    unsafe {
        let primary_usage_page = get_int_property(device, K_IOHID_PRIMARY_USAGE_PAGE_KEY);
        let primary_usage = get_int_property(device, K_IOHID_PRIMARY_USAGE_KEY);

        // Primary should always be first, to match previous behavior.
        let mut list: Vec<DeviceInfo> =
            create_device_info_with_usage(device, primary_usage_page, primary_usage)
                .into_iter()
                .collect();

        let usage_pairs = get_usage_pairs(device);

        if !usage_pairs.is_null() {
            let page_key = cf_string(K_IOHID_DEVICE_USAGE_PAGE_KEY);
            let usage_key = cf_string(K_IOHID_DEVICE_USAGE_KEY);
            for i in 0..CFArrayGetCount(usage_pairs) {
                let dict = CFArrayGetValueAtIndex(usage_pairs, i);
                if CFGetTypeID(dict) != CFDictionaryGetTypeID() {
                    continue;
                }

                let mut usage_page_ref: CFTypeRef = std::ptr::null();
                let mut usage_ref: CFTypeRef = std::ptr::null();
                let mut usage_page = 0i32;
                let mut usage = 0i32;

                if CFDictionaryGetValueIfPresent(dict, page_key.get(), &mut usage_page_ref) == 0
                    || CFDictionaryGetValueIfPresent(dict, usage_key.get(), &mut usage_ref) == 0
                    || CFGetTypeID(usage_page_ref) != CFNumberGetTypeID()
                    || CFGetTypeID(usage_ref) != CFNumberGetTypeID()
                    || CFNumberGetValue(
                        usage_page_ref,
                        K_CF_NUMBER_SINT32_TYPE,
                        &mut usage_page as *mut i32 as *mut c_void,
                    ) == 0
                    || CFNumberGetValue(
                        usage_ref,
                        K_CF_NUMBER_SINT32_TYPE,
                        &mut usage as *mut i32 as *mut c_void,
                    ) == 0
                {
                    continue;
                }
                if usage_page == primary_usage_page && usage == primary_usage {
                    continue; // Already added.
                }

                if let Some(next) = create_device_info_with_usage(device, usage_page, usage) {
                    list.push(next);
                }
            }
        }

        list
    }
}

/// Translation of `hid_enumerate()`.
pub(super) fn hid_enumerate(vendor_id: u16, product_id: u16) -> Vec<DeviceInfo> {
    let mut mgr = lock_mgr();
    // Set up the HID Manager if it hasn't been done
    if hid_init_locked(&mut mgr).is_err() {
        return Vec::new();
    }
    let hid_mgr = mgr.as_ref().expect("initialized").0;

    // give the IOHIDManager a chance to update itself
    process_pending_events();

    let mut root = Vec::new();

    // SAFETY: the manager is live while the lock is held; the matching
    // dictionary and the device set are released after use, and the
    // devices are valid while the set is.
    unsafe {
        // Get a list of the Devices
        let mut matching = std::ptr::null_mut();
        if vendor_id != 0 || product_id != 0 {
            matching = CFDictionaryCreateMutable(
                kCFAllocatorDefault,
                K_IOHID_OPTIONS_TYPE_NONE as CFIndex,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            );

            if !matching.is_null() && vendor_id != 0 {
                let v = CFNumberCreate(
                    kCFAllocatorDefault,
                    K_CF_NUMBER_SHORT_TYPE,
                    &vendor_id as *const u16 as *const c_void,
                );
                let key = cf_string(K_IOHID_VENDOR_ID_KEY);
                CFDictionarySetValue(matching, key.get(), v);
                CFRelease(v);
            }

            if !matching.is_null() && product_id != 0 {
                let p = CFNumberCreate(
                    kCFAllocatorDefault,
                    K_CF_NUMBER_SHORT_TYPE,
                    &product_id as *const u16 as *const c_void,
                );
                let key = cf_string(K_IOHID_PRODUCT_ID_KEY);
                CFDictionarySetValue(matching, key.get(), p);
                CFRelease(p);
            }
        }
        IOHIDManagerSetDeviceMatching(hid_mgr, matching);
        if !matching.is_null() {
            CFRelease(matching);
        }

        let device_set = IOHIDManagerCopyDevices(hid_mgr);

        let mut device_array: Vec<*const c_void> = Vec::new();
        if !device_set.is_null() {
            // Convert the list into a C array so we can iterate easily.
            let num_devices = CFSetGetCount(device_set);
            device_array = vec![std::ptr::null(); num_devices as usize];
            CFSetGetValues(device_set, device_array.as_mut_ptr());
        }

        // Iterate over each device, making an entry for it.
        for &dev in &device_array {
            if dev.is_null() {
                continue;
            }

            // (move the pointer to the tail of returned list)
            root.append(&mut create_device_info(dev as IOHIDDeviceRef));
        }

        if !device_set.is_null() {
            CFRelease(device_set);
        }
    }

    // (with no devices, upstream sets "No HID devices found in the system."
    // or "No HID devices with requested VID/PID found in the system.")
    root
}

/// Translation of `hid_open()`.
pub(super) fn open(vendor_id: u16, product_id: u16, serial_number: Option<&str>) -> Result<Device> {
    // This function is identical to the Linux version. Platform independent.
    let devs = hid_enumerate(vendor_id, product_id);
    if devs.is_empty() {
        return Err(Error::new(if vendor_id == 0 && product_id == 0 {
            "No HID devices found in the system."
        } else {
            "No HID devices with requested VID/PID found in the system."
        }));
    }

    let path_to_open = devs
        .iter()
        .find(|cur_dev| {
            cur_dev.vendor_id == vendor_id
                && cur_dev.product_id == product_id
                && serial_number
                    .is_none_or(|serial| cur_dev.serial_number.as_deref() == Some(serial))
        })
        .and_then(|cur_dev| cur_dev.path.clone());

    match path_to_open {
        Some(path) => open_path(&path),
        None => Err(Error::new(
            "Device with requested VID/PID/(SerialNumber) not found",
        )),
    }
}

/// The input reports received and the thread's flags (the part of
/// `struct hid_device_` the `mutex` protects, and what the read thread and
/// the callbacks share).
struct Shared {
    device_handle: IOHIDDeviceRef,
    open_options: IOOptionBits,
    /// `input_reports`, under `mutex`.
    input_reports: Mutex<VecDeque<Vec<u8>>>,
    /// `condition`.
    condition: Condvar,
    disconnected: AtomicBool,
    shutdown_thread: AtomicBool,
    run_loop_mode: CfRef,
    run_loop: AtomicPtr<c_void>,
    source: AtomicPtr<c_void>,
    /// `input_report_buf` (written by IOKit before each report callback).
    input_report_buf: Box<[std::cell::UnsafeCell<u8>]>,
    /// Ensures correct startup sequence
    barrier: Barrier,
    /// Ensures correct shutdown sequence
    shutdown_barrier: Barrier,
}

// SAFETY: the device reference and the run loop are CF objects used from
// the threads upstream uses them on; the report buffer is only written by
// IOKit on the read thread, and the rest is synchronized.
unsafe impl Send for Shared {}
// SAFETY: as above.
unsafe impl Sync for Shared {}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, VecDeque<Vec<u8>>> {
        self.input_reports.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn buf_ptr(&self) -> *mut u8 {
        self.input_report_buf.as_ptr() as *mut u8
    }
}

/// An open device. Translation of `struct hid_device_`.
pub(super) struct Device {
    shared: Arc<Shared>,
    blocking: AtomicBool,
    thread: Option<std::thread::JoinHandle<()>>,
    /// The cached `hid_get_device_info()`.
    device_info: Mutex<Option<DeviceInfo>>,
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
            .field("device_handle", &self.shared.device_handle)
            .finish_non_exhaustive()
    }
}

/// Translation of `hid_device_removal_callback()`.
unsafe extern "C" fn hid_device_removal_callback(
    context: *mut c_void,
    _result: IOReturn,
    _sender: *mut c_void,
) {
    // Stop the Run Loop for this device.
    // SAFETY: the context is the device's shared state, alive while the
    // callback is registered.
    let d = unsafe { &*(context as *const Shared) };

    d.disconnected.store(true, Ordering::SeqCst);
    // SAFETY: the read thread's run loop.
    unsafe { CFRunLoopStop(d.run_loop.load(Ordering::SeqCst)) };
}

/// The Run Loop calls this function for each input report received. This
/// function puts the data into a linked list to be picked up by hid_read().
/// Translation of `hid_report_callback()`.
unsafe extern "C" fn hid_report_callback(
    context: *mut c_void,
    _result: IOReturn,
    _sender: *mut c_void,
    _report_type: IOHIDReportType,
    _report_id: u32,
    report: *mut u8,
    report_length: CFIndex,
) {
    // SAFETY: as in hid_device_removal_callback().
    let dev = unsafe { &*(context as *const Shared) };

    // Make a new Input Report object
    let rpt = if report.is_null() || report_length <= 0 {
        Vec::new()
    } else {
        // SAFETY: IOKit hands the report's bytes.
        unsafe { std::slice::from_raw_parts(report, report_length as usize) }.to_vec()
    };

    // Lock this section
    let mut reports = dev.lock();

    // Attach the new report object to the end of the list.
    let num_queued = reports.len().saturating_sub(1);
    let was_empty = reports.is_empty();
    reports.push_back(rpt);
    /* Pop one off if we've reached 30 in the queue. This
    way we don't grow forever if the user never reads
    anything from the device. */
    if !was_empty && num_queued > 30 {
        reports.pop_front();
    }

    // Signal a waiting thread that there is data.
    dev.condition.notify_one();

    // Unlock
}

/// This gets called when the read_thread's run loop gets signaled by
/// hid_close(), and serves to stop the read_thread's run loop. Translation
/// of `perform_signal_callback()`.
unsafe extern "C" fn perform_signal_callback(context: *mut c_void) {
    // SAFETY: as in hid_device_removal_callback().
    let dev = unsafe { &*(context as *const Shared) };
    // SAFETY: the read thread's run loop.
    unsafe { CFRunLoopStop(dev.run_loop.load(Ordering::SeqCst)) }; /*TODO: CFRunLoopGetCurrent()*/
}

/// Translation of `read_thread()`.
fn read_thread(dev: Arc<Shared>) {
    let ctx_ptr = Arc::as_ptr(&dev) as *mut c_void;
    // SAFETY: the device and the mode are live; the source is created
    // with the shared state as its info, which outlives it.
    unsafe {
        // Move the device's run loop to this thread.
        IOHIDDeviceScheduleWithRunLoop(
            dev.device_handle,
            CFRunLoopGetCurrent(),
            dev.run_loop_mode.get(),
        );

        /* Create the RunLoopSource which is used to signal the
        event loop to stop when hid_close() is called. */
        let mut ctx = CFRunLoopSourceContext {
            version: 0,
            info: ctx_ptr,
            retain: None,
            release: None,
            copy_description: None,
            equal: None,
            hash: None,
            schedule: None,
            cancel: None,
            perform: Some(perform_signal_callback),
        };
        let source = CFRunLoopSourceCreate(kCFAllocatorDefault, 0 /*order*/, &mut ctx);
        dev.source.store(source, Ordering::SeqCst);
        CFRunLoopAddSource(CFRunLoopGetCurrent(), source, dev.run_loop_mode.get());

        /* Store off the Run Loop so it can be stopped from hid_close()
        and on device disconnection. */
        dev.run_loop.store(CFRunLoopGetCurrent(), Ordering::SeqCst);
    }

    // Notify the main thread that the read thread is up and running.
    dev.barrier.wait();

    /* Run the Event Loop. CFRunLoopRunInMode() will dispatch HID input
    reports into the hid_report_callback(). */
    while !dev.shutdown_thread.load(Ordering::SeqCst) && !dev.disconnected.load(Ordering::SeqCst) {
        // SAFETY: runs this thread's run loop in the device's mode.
        let code = unsafe {
            CFRunLoopRunInMode(dev.run_loop_mode.get(), 1000.0 /*sec*/, 0)
        };
        // Return if the device has been disconnected
        if code == K_CF_RUN_LOOP_RUN_FINISHED || code == K_CF_RUN_LOOP_RUN_STOPPED {
            dev.disconnected.store(true, Ordering::SeqCst);
            break;
        }

        // Break if The Run Loop returns Finished or Stopped.
        if code != K_CF_RUN_LOOP_RUN_TIMED_OUT && code != K_CF_RUN_LOOP_RUN_HANDLED_SOURCE {
            /* There was some kind of error. Setting
            shutdown seems to make sense, but
            there may be something else more appropriate */
            dev.shutdown_thread.store(true, Ordering::SeqCst);
            break;
        }
    }

    /* Now that the read thread is stopping, Wake any threads which are
    waiting on data (in hid_read_timeout()). Do this under a mutex to
    make sure that a thread which is about to go to sleep waiting on
    the condition actually will go to sleep before the condition is
    signaled. */
    {
        let _reports = dev.lock();
        dev.condition.notify_all();
    }

    /* Wait here until hid_close() is called and makes it past
    the call to CFRunLoopWakeUp(). This thread still needs to
    be valid when that function is called on the other thread. */
    dev.shutdown_barrier.wait();
}

/// Translation of `hid_open_service_registry_from_path()`.
///
/// `path` must be one of:
/// - in format 'DevSrvsID:<RegistryEntryID>' (as returned by hid_enumerate);
/// - a valid path to an IOHIDDevice in the IOService plane (as returned by
///   IORegistryEntryGetPath, e.g.: "IOService:/AppleACPIPlatformExpert/PCI0@0/AppleACPIPCI/EHC1@1D,7/AppleUSBEHCI/PLAYSTATION(R)3 Controller@fd120000/IOUSBInterface@0/IOUSBHIDDriver");
///
/// Second format is for compatibility with paths accepted by older versions
/// of HIDAPI.
fn hid_open_service_registry_from_path(path: &str) -> IoRegistryEntry {
    // Get the IORegistry entry for the given path
    if let Some(rest) = path.strip_prefix("DevSrvsID:") {
        if let Some(entry_id) = parse_entry_id(rest) {
            // SAFETY: the matching dictionary is consumed by the call.
            return unsafe {
                IOServiceGetMatchingService(MACH_PORT_NULL, IORegistryEntryIDMatching(entry_id))
            };
        }
    } else {
        // Fallback to older format of the path
        let Ok(c_path) = CString::new(path) else {
            return MACH_PORT_NULL;
        };
        // SAFETY: a NUL-terminated path.
        return unsafe { IORegistryEntryFromPath(MACH_PORT_NULL, c_path.as_ptr()) };
    }

    MACH_PORT_NULL
}

/// The `strtoull(path + 10, &endptr, 10)` of a `DevSrvsID:` path, when it
/// consumes the whole rest (`*endptr == '\0'`).
fn parse_entry_id(rest: &str) -> Option<u64> {
    let (entry_id, len) = crate::stdlib::string::strtoull(rest, 10);
    (len == rest.len()).then_some(entry_id)
}

/// Translation of `hid_open_path()`.
pub(super) fn open_path(path: &str) -> Result<Device> {
    // Set up the HID Manager if it hasn't been done
    hid_init()?;

    let open_options = DEVICE_OPEN_OPTIONS.load(Ordering::Relaxed);

    // Get the IORegistry entry for the given path
    let entry = hid_open_service_registry_from_path(path);
    if entry == MACH_PORT_NULL {
        // Path wasn't valid (maybe device was removed?)
        return Err(Error::new(
            "hid_open_path: device mach entry not found with the given path",
        ));
    }

    // SAFETY: the entry is ours, released once below; the device reference
    // created from it is owned by the device (or released on failure).
    unsafe {
        // Create an IOHIDDevice for the entry
        let device_handle = IOHIDDeviceCreate(kCFAllocatorDefault, entry);
        if device_handle.is_null() {
            IOObjectRelease(entry);
            // Error creating the HID device
            return Err(Error::new(
                "hid_open_path: failed to create IOHIDDevice from the mach entry",
            ));
        }

        // Open the IOHIDDevice
        let ret = IOHIDDeviceOpen(device_handle, open_options);
        if ret != K_IO_RETURN_SUCCESS {
            CFRelease(device_handle);
            IOObjectRelease(entry);
            return Err(Error::new(format!(
                "hid_open_path: failed to open IOHIDDevice from mach entry: (0x{:08X}) {}",
                ret as u32,
                mach_error(ret)
            )));
        }

        // Create the buffers for receiving data
        let max_input_report_len = get_max_report_length(device_handle).max(0) as usize;
        let input_report_buf: Box<[std::cell::UnsafeCell<u8>]> = (0..max_input_report_len)
            .map(|_| std::cell::UnsafeCell::new(0))
            .collect();

        /* Create the Run Loop Mode for this device.
        printing the reference seems to work. */
        let str = CString::new(format!("HIDAPI_{device_handle:p}")).expect("no NUL");
        let run_loop_mode = CfRef::from_owned(CFStringCreateWithCString(
            std::ptr::null(),
            str.as_ptr(),
            K_CF_STRING_ENCODING_ASCII,
        ))
        .expect("CFStringCreateWithCString");

        let shared = Arc::new(Shared {
            device_handle,
            open_options,
            input_reports: Mutex::new(VecDeque::new()),
            condition: Condvar::new(),
            disconnected: AtomicBool::new(false),
            shutdown_thread: AtomicBool::new(false),
            run_loop_mode,
            run_loop: AtomicPtr::new(std::ptr::null_mut()),
            source: AtomicPtr::new(std::ptr::null_mut()),
            input_report_buf,
            barrier: Barrier::new(2),
            shutdown_barrier: Barrier::new(2),
        });
        let ctx = Arc::as_ptr(&shared) as *mut c_void;

        // Attach the device to a Run Loop
        IOHIDDeviceRegisterInputReportCallback(
            device_handle,
            shared.buf_ptr(),
            max_input_report_len as CFIndex,
            Some(hid_report_callback),
            ctx,
        );
        IOHIDDeviceRegisterRemovalCallback(device_handle, Some(hid_device_removal_callback), ctx);

        // Start the read thread
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("hidapi-mac-read".into())
            .spawn(move || read_thread(thread_shared))
            .expect("spawning the read thread");

        // Wait here for the read thread to be initialized.
        shared.barrier.wait();

        IOObjectRelease(entry);
        Ok(Device {
            shared,
            blocking: AtomicBool::new(true),
            thread: Some(thread),
            device_info: Mutex::new(None),
        })
    }
}

/// `"MAGIC0"`, the prefix of a report sent with its report ID in place.
const PASS_THROUGH_MAGIC: &[u8] = b"MAGIC0";

impl Device {
    /// Translation of `set_report()`.
    fn set_report(&self, report_type: IOHIDReportType, data: &[u8]) -> Result<usize> {
        let length = data.len();

        if data.is_empty() {
            return Err(Error::new("Invalid argument")); // strerror(EINVAL)
        }

        let mut report_id = data[0];
        let mut data_to_send = data;

        if report_id == 0x0 {
            /* Not using numbered Reports.
            Don't send the report number. */
            data_to_send = &data[1..];
        } else if length > 6 && data.starts_with(PASS_THROUGH_MAGIC) {
            report_id = data[PASS_THROUGH_MAGIC.len()];
            data_to_send = &data[PASS_THROUGH_MAGIC.len()..];
        }

        // Avoid crash if the device has been unplugged.
        if self.shared.disconnected.load(Ordering::SeqCst) {
            return Err(Error::new("Device is disconnected"));
        }

        // SAFETY: the device is open; the bytes are valid for their length.
        let res = unsafe {
            IOHIDDeviceSetReport(
                self.shared.device_handle,
                report_type,
                report_id as CFIndex,
                data_to_send.as_ptr(),
                data_to_send.len() as CFIndex,
            )
        };

        if res != K_IO_RETURN_SUCCESS {
            return Err(Error::new(format!(
                "IOHIDDeviceSetReport failed: (0x{:08X}) {}",
                res as u32,
                mach_error(res)
            )));
        }

        Ok(length)
    }

    /// Translation of `get_report()`.
    fn get_report(&self, report_type: IOHIDReportType, data: &mut [u8]) -> Result<usize> {
        // (upstream reads data[0] without checking the length)
        let Some(&report_id) = data.first() else {
            return Err(Error::new("Invalid argument"));
        };
        let offset = usize::from(report_id == 0x0);
        /* Not using numbered Reports.
        Don't send the report number. */
        let report = &mut data[offset..];
        let mut report_length = report.len() as CFIndex;

        // Avoid crash if the device has been unplugged.
        if self.shared.disconnected.load(Ordering::SeqCst) {
            return Err(Error::new("Device is disconnected"));
        }

        // SAFETY: the device is open; the buffer is writable for its length.
        let res = unsafe {
            IOHIDDeviceGetReport(
                self.shared.device_handle,
                report_type,
                report_id as CFIndex,
                report.as_mut_ptr(),
                &mut report_length,
            )
        };

        if res != K_IO_RETURN_SUCCESS {
            return Err(Error::new(format!(
                "IOHIDDeviceGetReport failed: (0x{:08X}) {}",
                res as u32,
                mach_error(res)
            )));
        }

        if report_id == 0x0 {
            // 0 report number still present at the beginning
            report_length += 1;
        }

        Ok(report_length as usize)
    }

    /// Translation of `hid_write()`.
    pub(super) fn write(&self, data: &[u8]) -> Result<usize> {
        self.set_report(K_IOHID_REPORT_TYPE_OUTPUT, data)
    }

    /// Helper function, so that this isn't duplicated in hid_read().
    /// Translation of `return_data()`: copy the first report out of the
    /// list (into `data`, truncated to its length).
    fn return_data(reports: &mut VecDeque<Vec<u8>>, data: &mut [u8]) -> usize {
        let rpt = reports.pop_front().expect("a queued report");
        let len = data.len().min(rpt.len());
        data[..len].copy_from_slice(&rpt[..len]);
        len
    }

    /// Translation of `hid_read_timeout()`.
    pub(super) fn read_timeout(&self, data: &mut [u8], milliseconds: i32) -> Result<usize> {
        let dev = &*self.shared;

        // Lock the access to the report list.
        let mut reports = dev.lock();

        // There's an input report queued up. Return it.
        if !reports.is_empty() {
            // Return the first one
            return Ok(Self::return_data(&mut reports, data));
        }

        // Return if the device has been disconnected.
        if dev.disconnected.load(Ordering::SeqCst) {
            return Err(Error::new("hid_read_timeout: device disconnected"));
        }

        if dev.shutdown_thread.load(Ordering::SeqCst) {
            /* This means the device has been closed (or there
            has been an error. An error code of -1 should
            be returned. */
            return Err(Error::new("hid_read_timeout: thread shutdown"));
        }

        // There is no data. Go to sleep and wait for data.

        if milliseconds == -1 {
            // Blocking
            // (cond_wait())
            while reports.is_empty() {
                reports = dev
                    .condition
                    .wait(reports)
                    .unwrap_or_else(|e| e.into_inner());

                /* A res of 0 means we may have been signaled or it may
                be a spurious wakeup. Check to see that there's actually
                data in the queue before returning, and if not, go back
                to sleep. See the pthread_cond_timedwait() man page for
                details. */

                if dev.shutdown_thread.load(Ordering::SeqCst)
                    || dev.disconnected.load(Ordering::SeqCst)
                {
                    // There was an error, or a device disconnection.
                    return Err(Error::new("hid_read_timeout: error waiting for more data"));
                }
            }
            Ok(Self::return_data(&mut reports, data))
        } else if milliseconds > 0 {
            // Non-blocking, but called with timeout.
            // (cond_timedwait() until the absolute time now + milliseconds)
            let deadline = Instant::now() + Duration::from_millis(milliseconds as u64);
            while reports.is_empty() {
                let now = Instant::now();
                if now >= deadline {
                    return Ok(0); // ETIMEDOUT
                }
                let (r, timeout) = dev
                    .condition
                    .wait_timeout(reports, deadline - now)
                    .unwrap_or_else(|e| e.into_inner());
                reports = r;
                if timeout.timed_out() && reports.is_empty() {
                    return Ok(0); // ETIMEDOUT
                }

                /* A res of 0 means we may have been signaled or it may
                be a spurious wakeup. Check to see that there's actually
                data in the queue before returning, and if not, go back
                to sleep. See the pthread_cond_timedwait() man page for
                details. */

                if dev.shutdown_thread.load(Ordering::SeqCst)
                    || dev.disconnected.load(Ordering::SeqCst)
                {
                    return Err(Error::new("hid_read_timeout:  error waiting for more data"));
                }
            }
            Ok(Self::return_data(&mut reports, data))
        } else {
            // Purely non-blocking
            Ok(0)
        }

        // Unlock
    }

    /// Translation of `hid_read()`.
    pub(super) fn read(&self, data: &mut [u8]) -> Result<usize> {
        let milliseconds = if self.blocking.load(Ordering::Relaxed) {
            -1
        } else {
            0
        };
        self.read_timeout(data, milliseconds)
    }

    /// Translation of `hid_set_nonblocking()`.
    pub(super) fn set_nonblocking(&self, nonblock: bool) -> Result<()> {
        // All Nonblocking operation is handled by the library.
        self.blocking.store(!nonblock, Ordering::Relaxed);

        Ok(())
    }

    /// Translation of `hid_send_feature_report()`.
    pub(super) fn send_feature_report(&self, data: &[u8]) -> Result<usize> {
        self.set_report(K_IOHID_REPORT_TYPE_FEATURE, data)
    }

    /// Translation of `hid_get_feature_report()`.
    pub(super) fn get_feature_report(&self, data: &mut [u8]) -> Result<usize> {
        self.get_report(K_IOHID_REPORT_TYPE_FEATURE, data)
    }

    /// Translation of `hid_get_input_report()`.
    pub(super) fn get_input_report(&self, data: &mut [u8]) -> Result<usize> {
        self.get_report(K_IOHID_REPORT_TYPE_INPUT, data)
    }

    /// Translation of `hid_get_manufacturer_string()`.
    pub(super) fn get_manufacturer_string(&self) -> Result<String> {
        Ok(self
            .get_device_info()?
            .manufacturer_string
            .unwrap_or_default())
    }

    /// Translation of `hid_get_product_string()`.
    pub(super) fn get_product_string(&self) -> Result<String> {
        Ok(self.get_device_info()?.product_string.unwrap_or_default())
    }

    /// Translation of `hid_get_serial_number_string()`.
    pub(super) fn get_serial_number_string(&self) -> Result<String> {
        Ok(self.get_device_info()?.serial_number.unwrap_or_default())
    }

    /// Translation of `hid_get_device_info()` (the first entry of the list
    /// `create_device_info()` makes).
    pub(super) fn get_device_info(&self) -> Result<DeviceInfo> {
        let mut cached = self.device_info.lock().unwrap_or_else(|e| e.into_inner());
        if cached.is_none() {
            // SAFETY: the device is open.
            *cached = unsafe { create_device_info(self.shared.device_handle) }
                .into_iter()
                .next();
        }
        cached
            .clone()
            .ok_or_else(|| Error::new("Failed to create hid_device_info"))
    }

    /// Translation of `hid_get_indexed_string()`.
    pub(super) fn get_indexed_string(&self, _string_index: i32) -> Result<String> {
        Err(Error::new(
            "hid_get_indexed_string: not available on this platform",
        ))
    }

    /// Translation of `hid_darwin_get_location_id()`.
    #[allow(dead_code)]
    pub(super) fn get_location_id(&self) -> Result<u32> {
        // SAFETY: the device is open.
        let res = unsafe { get_int_property(self.shared.device_handle, K_IOHID_LOCATION_ID_KEY) };
        if res != 0 {
            Ok(res as u32)
        } else {
            Err(Error::new("Failed to get IOHIDLocationID property"))
        }
    }

    /// Translation of `hid_darwin_is_device_open_exclusive()`.
    #[allow(dead_code)]
    pub(super) fn is_device_open_exclusive(&self) -> bool {
        self.shared.open_options == K_IOHID_OPTIONS_TYPE_SEIZE_DEVICE
    }

    /// Translation of `hid_get_report_descriptor()`.
    pub(super) fn get_report_descriptor(&self, buf: &mut [u8]) -> Result<usize> {
        let key = cf_string(K_IOHID_REPORT_DESCRIPTOR_KEY);
        // SAFETY: the device is open; the data is owned by it and copied.
        unsafe {
            let r = IOHIDDeviceGetProperty(self.shared.device_handle, key.get());
            if !r.is_null() && CFGetTypeID(r) == CFDataGetTypeID() {
                let descriptor_buf = CFDataGetBytePtr(r);
                let descriptor_buf_len = CFDataGetLength(r);

                if descriptor_buf.is_null() || descriptor_buf_len < 0 {
                    return Err(Error::new("Zero buffer/length"));
                }

                let copy_len = (descriptor_buf_len as usize).min(buf.len());
                buf[..copy_len]
                    .copy_from_slice(std::slice::from_raw_parts(descriptor_buf, copy_len));
                Ok(copy_len)
            } else {
                Err(Error::new(
                    "Failed to get kIOHIDReportDescriptorKey property",
                ))
            }
        }
    }
}

impl Drop for Device {
    /// Translation of `hid_close()`.
    fn drop(&mut self) {
        let dev = &*self.shared;
        let ctx = Arc::as_ptr(&self.shared) as *mut c_void;
        let is_macos_10_10_or_greater = IS_MACOS_10_10_OR_GREATER.load(Ordering::Relaxed);

        // SAFETY: the device is open and the run loop is the read thread's,
        // which waits at the shutdown barrier until this is done with it.
        unsafe {
            /* Disconnect the report callback before close.
            See comment below.
            */
            if is_macos_10_10_or_greater || !dev.disconnected.load(Ordering::SeqCst) {
                IOHIDDeviceRegisterInputReportCallback(
                    dev.device_handle,
                    dev.buf_ptr(),
                    dev.input_report_buf.len() as CFIndex,
                    None,
                    ctx,
                );
                IOHIDDeviceRegisterRemovalCallback(dev.device_handle, None, ctx);
                IOHIDDeviceUnscheduleFromRunLoop(
                    dev.device_handle,
                    dev.run_loop.load(Ordering::SeqCst),
                    dev.run_loop_mode.get(),
                );
                IOHIDDeviceScheduleWithRunLoop(
                    dev.device_handle,
                    CFRunLoopGetMain(),
                    kCFRunLoopDefaultMode,
                );
            }

            // Cause read_thread() to stop.
            dev.shutdown_thread.store(true, Ordering::SeqCst);

            // Wake up the run thread's event loop so that the thread can exit.
            CFRunLoopSourceSignal(dev.source.load(Ordering::SeqCst) as CFRunLoopSourceRef);
            CFRunLoopWakeUp(dev.run_loop.load(Ordering::SeqCst) as CFRunLoopRef);
        }

        // Notify the read thread that it can shut down now.
        dev.shutdown_barrier.wait();

        // Wait for read_thread() to end.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }

        /* Close the OS handle to the device, but only if it's not
        been unplugged. If it's been unplugged, then calling
        IOHIDDeviceClose() will crash.

        UPD: The crash part was true in/until some version of macOS.
        Starting with macOS 10.15, there is an opposite effect in some environments:
        crash happens if IOHIDDeviceClose() is not called.
        Not leaking a resource in all tested environments.
        */
        if is_macos_10_10_or_greater || !dev.disconnected.load(Ordering::SeqCst) {
            // SAFETY: the device was opened with these options.
            unsafe { IOHIDDeviceClose(dev.device_handle, dev.open_options) };
        }

        // Clear out the queue of received reports.
        dev.lock().clear();
        // SAFETY: the device reference and the source are ours, released
        // once (free_hid_device() releases the source and the mode).
        unsafe {
            CFRelease(dev.device_handle);
            let source = dev.source.load(Ordering::SeqCst);
            if !source.is_null() {
                CFRelease(source);
            }
        }
    }
}

/// Changes the behavior of all further calls to `hid_open` or
/// `hid_open_path`: devices are opened in exclusive mode
/// (`kIOHIDOptionsTypeSeizeDevice`) or not. Translation of
/// `hid_darwin_set_open_exclusive()`.
pub(super) fn hid_darwin_set_open_exclusive(open_exclusive: bool) {
    DEVICE_OPEN_OPTIONS.store(
        if !open_exclusive {
            K_IOHID_OPTIONS_TYPE_NONE
        } else {
            K_IOHID_OPTIONS_TYPE_SEIZE_DEVICE
        },
        Ordering::Relaxed,
    );
}

/// Translation of `hid_darwin_get_open_exclusive()`.
#[allow(dead_code)]
pub(super) fn hid_darwin_get_open_exclusive() -> bool {
    DEVICE_OPEN_OPTIONS.load(Ordering::Relaxed) == K_IOHID_OPTIONS_TYPE_SEIZE_DEVICE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_transports() {
        assert_eq!(path_of_entry_id(4294969191), "DevSrvsID:4294969191");
        assert_eq!(path_of_entry_id(u64::MAX), "DevSrvsID:18446744073709551615");
        assert_eq!(parse_entry_id("4294969191"), Some(4294969191));
        // strtoull(): an empty rest is entry 0, a trailing character fails
        assert_eq!(parse_entry_id(""), Some(0));
        assert_eq!(parse_entry_id("12x"), None);
        assert_eq!(bus_type_of_transport("USB"), BusType::Usb);
        assert_eq!(bus_type_of_transport("Bluetooth"), BusType::Bluetooth);
        assert_eq!(
            bus_type_of_transport("BluetoothLowEnergy"),
            BusType::Bluetooth
        );
        assert_eq!(
            bus_type_of_transport("Bluetooth Low Energy"),
            BusType::Bluetooth
        );
        assert_eq!(bus_type_of_transport("I2C"), BusType::I2c);
        assert_eq!(bus_type_of_transport("SPI"), BusType::Spi);
        assert_eq!(bus_type_of_transport("FIFO"), BusType::Unknown);
        assert_eq!(bus_type_of_transport("usb"), BusType::Unknown);
    }

    #[test]
    fn open_exclusive_option() {
        let _l = crate::test_support::test_lock();
        let before = hid_darwin_get_open_exclusive();
        hid_darwin_set_open_exclusive(true);
        assert_eq!(
            DEVICE_OPEN_OPTIONS.load(Ordering::Relaxed),
            K_IOHID_OPTIONS_TYPE_SEIZE_DEVICE
        );
        hid_darwin_set_open_exclusive(false);
        assert_eq!(
            DEVICE_OPEN_OPTIONS.load(Ordering::Relaxed),
            K_IOHID_OPTIONS_TYPE_NONE
        );
        hid_darwin_set_open_exclusive(before);
    }

    #[test]
    fn utf32_strings() {
        let s = cf_string("Pad 🎮 ✓");
        assert_eq!(unsafe { cf_string_utf32(s.get(), BUF_LEN - 1) }, "Pad 🎮 ✓");
        // At most the buffer's characters (counted in UTF-16 units)
        assert_eq!(unsafe { cf_string_utf32(s.get(), 3) }, "Pad");
        let long = cf_string(&"x".repeat(300));
        assert_eq!(
            unsafe { cf_string_utf32(long.get(), BUF_LEN - 1) }.len(),
            255
        );
    }
}
