// Rust translation of src/hidapi/linux/hid.c and src/hidapi/SDL_hidapi_linux.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.
//
// HIDAPI - Multi-Platform library for communication with HID devices.
// Alan Ott, Signal 11 Software; libusb/hidapi Team. Copyright 2022, All
// Rights Reserved. At the discretion of the user of this library, this
// software may be licensed under the terms of the GNU General Public
// License v3, a BSD-Style license, or the original HIDAPI license as
// outlined in the LICENSE.txt, LICENSE-gpl3.txt, LICENSE-bsd.txt, and
// LICENSE-orig.txt files located at the root of the source distribution.
// These files may also be found in the public source code repository
// located at: https://github.com/libusb/hidapi .

//! The Linux `hidraw` backend: devices are found through libudev (the
//! `hidraw` subsystem, with their USB or Bluetooth parents for the strings)
//! and used through the hidraw character devices and their ioctls.
//!
//! Upstream's `hid_init()` sets the C locale so that `mbstowcs()` decodes
//! the UTF-8 strings udev reports; here the strings stay UTF-8. Errors are
//! returned (upstream, built into SDL, sets them with `SDL_SetError()`), so
//! the error strings `hid_error()` keeps don't exist.

use std::ffi::{c_char, c_int, c_ulong, c_void, CStr, CString};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use super::{BusType, DeviceInfo};
use crate::core::linux::input::{errno_string, ioc_read_write, ioctl_ptr, ior, strerror};
use crate::core::linux::udev::{self, udev_device, UdevSymbols};
use crate::error::{Error, Result};
use crate::stdlib::string::strtol;

// <linux/input.h> bus types

/// `BUS_USB`
const BUS_USB: u32 = 0x03;
/// `BUS_BLUETOOTH`
const BUS_BLUETOOTH: u32 = 0x05;
/// `BUS_I2C`
const BUS_I2C: u32 = 0x18;
/// `BUS_SPI`
const BUS_SPI: u32 = 0x1C;

// <linux/hidraw.h>

/// `HID_MAX_DESCRIPTOR_SIZE`
const HID_MAX_DESCRIPTOR_SIZE: usize = 4096;

/// `struct hidraw_report_descriptor`
#[repr(C)]
#[derive(Clone, Copy)]
struct HidrawReportDescriptor {
    size: u32,
    value: [u8; HID_MAX_DESCRIPTOR_SIZE],
}

impl HidrawReportDescriptor {
    /// The bytes of the descriptor (`value[..size]`, kept in bounds).
    fn bytes(&self) -> &[u8] {
        &self.value[..(self.size as usize).min(HID_MAX_DESCRIPTOR_SIZE)]
    }
}

/// `HIDIOCGRDESCSIZE`
const HIDIOCGRDESCSIZE: c_ulong = ior(b'H', 0x01, size_of::<c_int>());
/// `HIDIOCGRDESC`
const HIDIOCGRDESC: c_ulong = ior(b'H', 0x02, size_of::<HidrawReportDescriptor>());

/// `HIDIOCSFEATURE(len)`
const fn hidiocsfeature(len: usize) -> c_ulong {
    ioc_read_write(b'H', 0x06, len)
}

/// `HIDIOCGFEATURE(len)`
const fn hidiocgfeature(len: usize) -> c_ulong {
    ioc_read_write(b'H', 0x07, len)
}

/// `HIDIOCGINPUT(len)`: not defined in Linux kernel headers < 5.11; this
/// definition is from hidraw.h in Linux >= 5.11.
/// <https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/commit/?id=f43d3870cafa2a0f3854c1819c8385733db8f9ae>
const fn hidiocginput(len: usize) -> c_ulong {
    ioc_read_write(b'H', 0x0A, len)
}

// The udev functions (`udev_ctx` of SDL_hidapi_linux.h)

/// Translation of `udev_ctx`: the libudev functions, while `SDL_hid_init()`
/// holds a reference to them.
static UDEV_CTX: Mutex<Option<UdevSymbols>> = Mutex::new(None);

fn udev_ctx() -> Option<UdevSymbols> {
    *UDEV_CTX.lock().unwrap_or_else(|e| e.into_inner())
}

/// A string from libudev (NULL is `None`; invalid UTF-8 is replaced).
///
/// # Safety
///
/// `p` must be NULL or point to a NUL-terminated string.
unsafe fn c_string(p: *const c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        // SAFETY: the caller guarantees a NUL-terminated string.
        Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
    }
}

/// A `struct udev` context, unreferenced on drop.
struct Udev<'a> {
    syms: &'a UdevSymbols,
    udev: NonNull<udev::udev>,
}

impl<'a> Udev<'a> {
    /// `udev_new()`
    fn new(syms: &'a UdevSymbols) -> Option<Udev<'a>> {
        // SAFETY: udev_new has no preconditions.
        NonNull::new(unsafe { (syms.udev_new)() }).map(|udev| Udev { syms, udev })
    }

    /// `udev_device_new_from_devnum(udev, 'c', devnum)`
    fn device_from_devnum(&self, devnum: libc::dev_t) -> Option<UdevDevice<'a>> {
        // SAFETY: the context is live.
        let dev = unsafe {
            (self.syms.udev_device_new_from_devnum)(self.udev.as_ptr(), b'c' as c_char, devnum)
        };
        NonNull::new(dev).map(|dev| UdevDevice {
            syms: self.syms,
            dev,
        })
    }

    /// `udev_device_new_from_syspath()`
    fn device_from_syspath(&self, syspath: &CStr) -> Option<UdevDevice<'a>> {
        // SAFETY: the context is live and the path NUL-terminated.
        let dev = unsafe {
            (self.syms.udev_device_new_from_syspath)(self.udev.as_ptr(), syspath.as_ptr())
        };
        NonNull::new(dev).map(|dev| UdevDevice {
            syms: self.syms,
            dev,
        })
    }
}

impl Drop for Udev<'_> {
    fn drop(&mut self) {
        // SAFETY: we own this reference.
        unsafe {
            (self.syms.udev_unref)(self.udev.as_ptr());
        }
    }
}

/// A referenced `struct udev_device`, unreferenced on drop.
struct UdevDevice<'a> {
    syms: &'a UdevSymbols,
    dev: NonNull<udev_device>,
}

impl Drop for UdevDevice<'_> {
    fn drop(&mut self) {
        // SAFETY: we own this reference.
        unsafe {
            (self.syms.udev_device_unref)(self.dev.as_ptr());
        }
    }
}

/// A device node the functions below read: an owned [`UdevDevice`] or a
/// parent, which belongs to its child.
#[derive(Clone, Copy)]
struct DeviceRef<'a> {
    syms: &'a UdevSymbols,
    dev: NonNull<udev_device>,
}

impl<'a> UdevDevice<'a> {
    fn as_ref(&self) -> DeviceRef<'a> {
        DeviceRef {
            syms: self.syms,
            dev: self.dev,
        }
    }
}

impl<'a> DeviceRef<'a> {
    /// `udev_device_get_syspath()`
    fn syspath(self) -> Option<String> {
        // SAFETY: the device is live; the string is copied.
        unsafe { c_string((self.syms.udev_device_get_syspath)(self.dev.as_ptr())) }
    }

    /// `udev_device_get_devnode()`
    fn devnode(self) -> Option<String> {
        // SAFETY: as above.
        unsafe { c_string((self.syms.udev_device_get_devnode)(self.dev.as_ptr())) }
    }

    /// `udev_device_get_sysattr_value()`, as bytes.
    fn sysattr_bytes(self, name: &CStr) -> Option<Vec<u8>> {
        // SAFETY: as above; the name is NUL-terminated.
        let p =
            unsafe { (self.syms.udev_device_get_sysattr_value)(self.dev.as_ptr(), name.as_ptr()) };
        // SAFETY: libudev returns NULL or a NUL-terminated string, copied
        // here.
        (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_bytes().to_vec())
    }

    /// `udev_device_get_sysattr_value()` (invalid UTF-8 is replaced).
    fn sysattr(self, name: &CStr) -> Option<String> {
        self.sysattr_bytes(name)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    }

    /// `udev_device_get_parent_with_subsystem_devtype()`: the parent
    /// belongs to this device (it "doesn't need to be (and can't be)
    /// unref'd").
    fn parent_with_subsystem_devtype(
        self,
        subsystem: &CStr,
        devtype: Option<&CStr>,
    ) -> Option<Self> {
        // SAFETY: the device is live; the strings are NUL-terminated.
        let parent = unsafe {
            (self.syms.udev_device_get_parent_with_subsystem_devtype)(
                self.dev.as_ptr(),
                subsystem.as_ptr(),
                devtype.map_or(std::ptr::null(), CStr::as_ptr),
            )
        };
        NonNull::new(parent).map(|dev| DeviceRef {
            syms: self.syms,
            dev,
        })
    }
}

// Upstream's string helpers

/// Translation of `utf8_to_wchar_t()`: upstream decodes with `mbstowcs()`,
/// which gives an empty string for bytes that don't decode; that is what
/// invalid UTF-8 gives here.
fn utf8_to_wchar_t(utf8: Option<&[u8]>) -> Option<String> {
    utf8.map(|bytes| String::from_utf8(bytes.to_vec()).unwrap_or_default())
}

/// Translation of `copy_udev_string()`.
fn copy_udev_string(dev: DeviceRef<'_>, udev_name: &CStr) -> Option<String> {
    utf8_to_wchar_t(dev.sysattr_bytes(udev_name).as_deref())
}

// Report descriptor parsing

/// Translation of `get_hid_item_size()`: the size of the HID item at
/// `pos`, as `(data_len, key_size)`, or `None` for an invalid key.
fn get_hid_item_size(report_descriptor: &[u8], pos: usize) -> Option<(usize, usize)> {
    let key = report_descriptor[pos];

    /*
     * This is a Long Item. The next byte contains the
     * length of the data section (value) for this key.
     * See the HID specification, version 1.11, section
     * 6.2.2.3, titled "Long Items."
     */
    // (for a malformed report, with no length byte, upstream sets data_len
    // and key_size to 0, then falls through to the short item sizes)
    if key & 0xf0 == 0xf0 && pos + 1 < report_descriptor.len() {
        return Some((usize::from(report_descriptor[pos + 1]), 3));
    }

    /*
     * This is a Short Item. The bottom two bits of the
     * key contain the size code for the data section
     * (value) for this key. Refer to the HID
     * specification, version 1.11, section 6.2.2.2,
     * titled "Short Items."
     */
    let size_code = key & 0x3;
    match size_code {
        0..=2 => Some((usize::from(size_code), 1)),
        _ => Some((4, 1)),
    }
}

/// Translation of `get_hid_report_bytes()`: `num_bytes` (0, 1, 2 or 4)
/// bytes of the item at `cur`, little endian.
fn get_hid_report_bytes(rpt: &[u8], num_bytes: usize, cur: usize) -> u32 {
    // Return if there aren't enough bytes.
    if cur + num_bytes >= rpt.len() {
        return 0;
    }

    match num_bytes {
        1 => u32::from(rpt[cur + 1]),
        2 => u32::from(rpt[cur + 2]) * 256 + u32::from(rpt[cur + 1]),
        4 => u32::from_le_bytes([rpt[cur + 1], rpt[cur + 2], rpt[cur + 3], rpt[cur + 4]]),
        _ => 0,
    }
}

/// Translation of `hid_iterate_over_collection()`: iterates until the end
/// of a Collection. Assumes that `pos` is exactly at the beginning of a
/// Collection. Skips all nested Collection, i.e. iterates until the end of
/// current level Collection.
///
/// Returns the item size `(data_len, key_size)` at the end of the current
/// Collection (with `pos` on it), or `None` when an error occurred (broken
/// Descriptor, end of a Collection is found before its begin, or no
/// Collection is found at all).
fn hid_iterate_over_collection(
    report_descriptor: &[u8],
    pos: &mut usize,
) -> Option<(usize, usize)> {
    let mut collection_level = 0;

    while *pos < report_descriptor.len() {
        let key = report_descriptor[*pos];
        let key_cmd = key & 0xfc;

        // Determine data_len and key_size
        let (data_len, key_size) = get_hid_item_size(report_descriptor, *pos)?;

        match key_cmd {
            0xa0 => collection_level += 1, // Collection 6.2.2.4 (Main)
            0xc0 => collection_level -= 1, // End Collection 6.2.2.4 (Main)
            _ => {}
        }

        if collection_level < 0 {
            /* Broken descriptor or someone is using this function wrong,
             * i.e. should be called exactly at the collection start */
            return None;
        }

        if collection_level == 0 {
            /* Found it!
             * Also possible when called not at the collection start, but should not happen if used correctly */
            return Some((data_len, key_size));
        }

        *pos += data_len + key_size;
    }

    None // Did not find the end of a Collection
}

/// Translation of `struct hid_usage_iterator`.
#[derive(Clone, Copy, Debug, Default)]
struct HidUsageIterator {
    pos: usize,
    usage_page_found: bool,
    usage_page: u16,
}

/// What [`get_next_hid_usage`] found (its return value 0, 1 or -1).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum NextUsage {
    /// A Usage Page/Usage pair (0)
    Found(u16, u16),
    /// Finished processing the descriptor (1)
    Finished,
    /// A malformed report (-1)
    Malformed,
}

/// Retrieves the device's Usage Page and Usage from the report descriptor.
/// The algorithm returns the current Usage Page/Usage pair whenever a new
/// Collection is found and a Usage Local Item is currently in scope.
/// Usage Local Items are consumed by each Main Item (See. 6.2.2.8).
/// The algorithm should give similar results as Apple's:
///   <https://developer.apple.com/documentation/iokit/kiohiddeviceusagepairskey?language=objc>
/// Physical Collections are also matched (macOS does the same).
///
/// This function can be called repeatedly until it doesn't find a pair;
/// `ctx.pos` is the starting point (initially 0) and is updated to the next
/// search position. Translation of `get_next_hid_usage()`.
fn get_next_hid_usage(report_descriptor: &[u8], ctx: &mut HidUsageIterator) -> NextUsage {
    let size = report_descriptor.len();
    let initial = ctx.pos == 0; // Used to handle case where no top-level application collection is defined
    let mut usage_found = false;
    let mut usage = 0u16;

    while ctx.pos < size {
        let key = report_descriptor[ctx.pos];
        let key_cmd = key & 0xfc;

        // Determine data_len and key_size
        let Some((mut data_len, mut key_size)) = get_hid_item_size(report_descriptor, ctx.pos)
        else {
            return NextUsage::Malformed; // malformed report
        };

        match key_cmd {
            0x4 => {
                // Usage Page 6.2.2.7 (Global)
                ctx.usage_page = get_hid_report_bytes(report_descriptor, data_len, ctx.pos) as u16;
                ctx.usage_page_found = true;
            }
            0x8 => {
                // Usage 6.2.2.8 (Local)
                if data_len == 4 {
                    // Usages 5.5 / Usage Page 6.2.2.7
                    ctx.usage_page = get_hid_report_bytes(report_descriptor, 2, ctx.pos + 2) as u16;
                    ctx.usage_page_found = true;
                    usage = get_hid_report_bytes(report_descriptor, 2, ctx.pos) as u16;
                } else {
                    usage = get_hid_report_bytes(report_descriptor, data_len, ctx.pos) as u16;
                }
                usage_found = true;
            }
            0xa0 => {
                // Collection 6.2.2.4 (Main)
                match hid_iterate_over_collection(report_descriptor, &mut ctx.pos) {
                    Some(sizes) => (data_len, key_size) = sizes,
                    None => return NextUsage::Malformed,
                }

                // A pair is valid - to be reported when Collection is found
                if usage_found && ctx.usage_page_found {
                    return NextUsage::Found(ctx.usage_page, usage);
                }
            }
            _ => {}
        }

        // Skip over this key and its associated data
        ctx.pos += data_len + key_size;
    }

    /* If no top-level application collection is found and usage page/usage pair is found, pair is valid
    https://docs.microsoft.com/en-us/windows-hardware/drivers/hid/top-level-collections */
    if initial && usage_found && ctx.usage_page_found {
        return NextUsage::Found(ctx.usage_page, usage); // success
    }

    NextUsage::Finished // finished processing
}

/// The Usage Page/Usage pairs of a report descriptor, in order (the loop
/// of `create_device_info_for_device()`).
fn hid_usages(report_descriptor: &[u8]) -> Vec<(u16, u16)> {
    let mut usages = Vec::new();
    let mut usage_iterator = HidUsageIterator::default();
    while let NextUsage::Found(page, usage) =
        get_next_hid_usage(report_descriptor, &mut usage_iterator)
    {
        usages.push((page, usage));
    }
    usages
}

/// Translation of `get_hid_report_descriptor()`: retrieves the hidraw
/// report descriptor from a file. When using this form,
/// `<sysfs_path>/device/report_descriptor`, elevated privileges are not
/// required.
fn get_hid_report_descriptor(rpt_path: &str) -> Result<Vec<u8>> {
    /*
     * Read in the Report Descriptor
     * The sysfs file has a maximum size of 4096 (which is the same as HID_MAX_DESCRIPTOR_SIZE) so we should always
     * be ok when reading the descriptor.
     * In practice if the HID descriptor is any larger I suspect many other things will break.
     */
    use std::io::Read;

    let mut file = std::fs::File::open(rpt_path)
        .map_err(|e| Error::new(format!("open failed ({rpt_path}): {}", io_error_string(&e))))?;
    let mut value = vec![0u8; HID_MAX_DESCRIPTOR_SIZE];
    let res = file
        .read(&mut value)
        .map_err(|e| Error::new(format!("read failed ({rpt_path}): {}", io_error_string(&e))))?;
    value.truncate(res);
    Ok(value)
}

/// `strerror()` of an I/O error.
fn io_error_string(e: &std::io::Error) -> String {
    errno_string(e.raw_os_error().unwrap_or(0))
}

/// Translation of `get_hid_report_descriptor_from_sysfs()`.
fn get_hid_report_descriptor_from_sysfs(sysfs_path: &str) -> Result<Vec<u8>> {
    // Construct <sysfs_path>/device/report_descriptor
    get_hid_report_descriptor(&format!("{sysfs_path}/device/report_descriptor"))
}

/// The keys of a uevent file's `KEY=value` lines (`strtok_r()` on "\n",
/// which skips empty lines, then `strchr()` for the '=').
fn uevent_lines(uevent: &str) -> impl Iterator<Item = (&str, &str)> {
    // (upstream copies at most 1023 bytes of the uevent before splitting it)
    let mut end = uevent.len().min(1023);
    while !uevent.is_char_boundary(end) {
        end -= 1;
    }
    uevent[..end]
        .split('\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| line.split_once('='))
}

/// The `HID_ID=0003:000005AC:00008242` value (type vendor product), parsed
/// with `sscanf("%x:%hx:%hx")`.
fn parse_hid_id(value: &str) -> Option<(u32, u16, u16)> {
    let scan_hex = |s: &str| -> Option<(u64, usize)> {
        let (value, len) = crate::stdlib::string::strtoul(s, 16);
        (len > 0).then_some((value, len))
    };
    let (bus_type, len) = scan_hex(value)?;
    let rest = value[len..].strip_prefix(':')?;
    let (vendor_id, len) = scan_hex(rest)?;
    let rest = rest[len..].strip_prefix(':')?;
    let (product_id, _) = scan_hex(rest)?;
    Some((bus_type as u32, vendor_id as u16, product_id as u16))
}

/// Translation of `parse_hid_vid_pid_from_uevent()`.
fn parse_hid_vid_pid_from_uevent(uevent: &str) -> Result<(u32, u16, u16)> {
    for (key, value) in uevent_lines(uevent) {
        if key == "HID_ID" {
            /*
             *        type vendor   product
             * HID_ID=0003:000005AC:00008242
             */
            if let Some(id) = parse_hid_id(value) {
                return Ok(id);
            }
        }
    }

    Err(Error::new("Couldn't find/parse HID_ID"))
}

/// Translation of `parse_hid_vid_pid_from_uevent_path()`.
fn parse_hid_vid_pid_from_uevent_path(uevent_path: &str) -> Result<(u32, u16, u16)> {
    use std::io::Read;

    let mut file = std::fs::File::open(uevent_path).map_err(|e| {
        Error::new(format!(
            "open failed ({uevent_path}): {}",
            io_error_string(&e)
        ))
    })?;
    let mut buf = [0u8; 1023]; // (1024 with the '\0' at the end)
    let res = file.read(&mut buf).map_err(|e| {
        Error::new(format!(
            "read failed ({uevent_path}): {}",
            io_error_string(&e)
        ))
    })?;

    let text = &buf[..res];
    let text = &text[..text.iter().position(|&b| b == 0).unwrap_or(text.len())];
    parse_hid_vid_pid_from_uevent(&String::from_utf8_lossy(text))
}

/// Translation of `parse_hid_vid_pid_from_sysfs()`.
fn parse_hid_vid_pid_from_sysfs(sysfs_path: &str) -> Result<(u32, u16, u16)> {
    // Construct <sysfs_path>/device/uevent
    parse_hid_vid_pid_from_uevent_path(&format!("{sysfs_path}/device/uevent"))
}

/// What [`parse_uevent_info`] found: the bus type, vendor and product IDs
/// (zero unless found), the serial number and the product name.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct UeventInfo {
    bus_type: u32,
    vendor_id: u16,
    product_id: u16,
    serial_number_utf8: Option<String>,
    product_name_utf8: Option<String>,
    /// Whether the ID, the name and the serial number were all found
    /// (upstream's return value).
    complete: bool,
}

/// Translation of `parse_uevent_info()`.
fn parse_uevent_info(uevent: Option<&str>) -> UeventInfo {
    let mut info = UeventInfo::default();
    let Some(uevent) = uevent else {
        return info;
    };

    let mut found_id = false;
    let mut found_serial = false;
    let mut found_name = false;

    for (key, value) in uevent_lines(uevent) {
        match key {
            "HID_ID" => {
                /*
                 *        type vendor   product
                 * HID_ID=0003:000005AC:00008242
                 */
                if let Some((bus_type, vendor_id, product_id)) = parse_hid_id(value) {
                    info.bus_type = bus_type;
                    info.vendor_id = vendor_id;
                    info.product_id = product_id;
                    found_id = true;
                }
            }
            "HID_NAME" => {
                info.product_name_utf8 = Some(value.to_owned());
                found_name = true;
            }
            "HID_UNIQ" => {
                info.serial_number_utf8 = Some(value.to_owned());
                found_serial = true;
            }
            _ => {}
        }
    }

    info.complete = found_id && found_name && found_serial;
    info
}

/// The `uevent` of a device's HID parent, if it has one.
fn hid_parent_uevent(dev: DeviceRef<'_>) -> Option<UeventInfo> {
    let hid_dev = dev.parent_with_subsystem_devtype(c"hid", None)?;
    Some(parse_uevent_info(hid_dev.sysattr(c"uevent").as_deref()))
}

/// Translation of `create_device_info_for_device()`: one record per
/// Usage Page/Usage pair of the device.
fn create_device_info_for_device(raw_dev: DeviceRef<'_>) -> Vec<DeviceInfo> {
    let sysfs_path = raw_dev.syspath();
    let dev_path = raw_dev.devnode();

    let Some(uevent) = hid_parent_uevent(raw_dev) else {
        // Unable to find parent hid device.
        return Vec::new();
    };

    if !uevent.complete {
        // parse_uevent_info() failed for at least one field.
        return Vec::new();
    }

    // Filter out unhandled devices right away
    match uevent.bus_type {
        BUS_BLUETOOTH | BUS_I2C | BUS_USB | BUS_SPI => {}
        _ => return Vec::new(),
    }

    // Create the record.
    let mut cur_dev = DeviceInfo {
        path: dev_path.clone(),
        vendor_id: uevent.vendor_id,
        product_id: uevent.product_id,
        serial_number: utf8_to_wchar_t(uevent.serial_number_utf8.as_deref().map(str::as_bytes)),
        release_number: 0x0,
        interface_number: -1,
        ..DeviceInfo::default()
    };

    match uevent.bus_type {
        BUS_USB => {
            /* The device pointed to by raw_dev contains information about
            the hidraw device. In order to get information about the
            USB device, get the parent device with the
            subsystem/devtype pair of "usb"/"usb_device". This will
            be several levels up the tree, but the function will find
            it. */
            let usb_dev = raw_dev.parent_with_subsystem_devtype(c"usb", Some(c"usb_device"));

            /* uhid USB devices
             * Since this is a virtual hid interface, no USB information will
             * be available. */
            match usb_dev {
                None => {
                    // Manufacturer and Product strings
                    cur_dev.manufacturer_string = Some(String::new());
                    cur_dev.product_string =
                        utf8_to_wchar_t(uevent.product_name_utf8.as_deref().map(str::as_bytes));
                }
                Some(usb_dev) => {
                    cur_dev.manufacturer_string = copy_udev_string(usb_dev, c"manufacturer");
                    cur_dev.product_string = copy_udev_string(usb_dev, c"product");

                    cur_dev.bus_type = BusType::Usb;

                    cur_dev.release_number = usb_dev
                        .sysattr(c"bcdDevice")
                        .map_or(0x0, |s| strtol(&s, 16).0 as u16);

                    // Get a handle to the interface's udev node.
                    if let Some(intf_dev) =
                        raw_dev.parent_with_subsystem_devtype(c"usb", Some(c"usb_interface"))
                    {
                        cur_dev.interface_number = intf_dev
                            .sysattr(c"bInterfaceNumber")
                            .map_or(-1, |s| strtol(&s, 16).0 as i32);
                    }
                }
            }
        }
        BUS_BLUETOOTH | BUS_I2C | BUS_SPI => {
            cur_dev.manufacturer_string = Some(String::new());
            cur_dev.product_string =
                utf8_to_wchar_t(uevent.product_name_utf8.as_deref().map(str::as_bytes));

            cur_dev.bus_type = match uevent.bus_type {
                BUS_BLUETOOTH => BusType::Bluetooth,
                BUS_I2C => BusType::I2c,
                _ => BusType::Spi,
            };
        }
        _ => {
            /* Unknown device type - this should never happen, as we
             * check for USB and Bluetooth devices above */
        }
    }

    let mut devices = Vec::new();

    // Usage Page and Usage
    let report_desc = sysfs_path
        .as_deref()
        .map(get_hid_report_descriptor_from_sysfs);
    match report_desc {
        Some(Ok(report_desc)) => {
            /*
             * Parse the first usage and usage page
             * out of the report descriptor, then any additional usage and
             * usage pages, with a new record for each additional pair.
             */
            let mut usages = hid_usages(&report_desc).into_iter();
            if let Some((page, usage)) = usages.next() {
                cur_dev.usage_page = page;
                cur_dev.usage = usage;
            }
            devices.push(cur_dev);
            for (page, usage) in usages {
                // Create new record for additional usage pairs
                let prev_dev = devices.last().expect("a record");
                let tmp = DeviceInfo {
                    path: dev_path.clone(),
                    vendor_id: uevent.vendor_id,
                    product_id: uevent.product_id,
                    serial_number: prev_dev.serial_number.clone(),
                    release_number: prev_dev.release_number,
                    interface_number: prev_dev.interface_number,
                    manufacturer_string: prev_dev.manufacturer_string.clone(),
                    product_string: prev_dev.product_string.clone(),
                    usage_page: page,
                    usage,
                    bus_type: prev_dev.bus_type,
                    ..DeviceInfo::default()
                };
                devices.push(tmp);
            }
        }
        _ => devices.push(cur_dev),
    }

    // (HIDAPI_IGNORE_DEVICE)
    devices.retain(|dev| {
        !super::should_ignore_device(
            dev.bus_type,
            dev.vendor_id,
            dev.product_id,
            dev.usage_page,
            dev.usage,
            false,
            false,
        )
    });

    devices
}

// The library

/// Translation of `hid_init()` with the `udev_ctx` setup of
/// `SDL_hid_init()`.
pub(super) fn hid_init() -> Result<()> {
    let syms = udev::get_udev_syms().ok();
    *UDEV_CTX.lock().unwrap_or_else(|e| e.into_inner()) = syms;
    if syms.is_none() {
        return Err(Error::new("Could not initialize UDEV"));
    }

    // (upstream sets the C locale here, for mbstowcs())
    Ok(())
}

/// Translation of `hid_exit()` with the `udev_ctx` cleanup of
/// `SDL_hid_exit()`.
pub(super) fn hid_exit() -> Result<()> {
    *UDEV_CTX.lock().unwrap_or_else(|e| e.into_inner()) = None;
    udev::release_udev_syms();
    Ok(())
}

/// Translation of `hid_enumerate()`.
pub(super) fn hid_enumerate(vendor_id: u16, product_id: u16) -> Vec<DeviceInfo> {
    let Some(syms) = udev_ctx() else {
        return Vec::new();
    };
    let syms = &syms;

    // Create the udev object
    let Some(udev) = Udev::new(syms) else {
        // "Couldn't create udev context"
        return Vec::new();
    };

    let mut root = Vec::new();

    // Create a list of the devices in the 'hidraw' subsystem.
    let mut sysfs_paths = Vec::new();
    // SAFETY: the context is live; the list entries belong to the
    // enumerator, which is unreferenced after their names are copied.
    unsafe {
        let enumerate = (syms.udev_enumerate_new)(udev.udev.as_ptr());
        if !enumerate.is_null() {
            (syms.udev_enumerate_add_match_subsystem)(enumerate, c"hidraw".as_ptr());
            (syms.udev_enumerate_scan_devices)(enumerate);
            let mut entry = (syms.udev_enumerate_get_list_entry)(enumerate);
            while !entry.is_null() {
                let name = (syms.udev_list_entry_get_name)(entry);
                if !name.is_null() {
                    sysfs_paths.push(CStr::from_ptr(name).to_owned());
                }
                entry = (syms.udev_list_entry_get_next)(entry);
            }
            (syms.udev_enumerate_unref)(enumerate);
        }
    }

    /* For each item, see if it matches the vid/pid, and if so
    create a udev_device record for it */
    for sysfs_path in &sysfs_paths {
        if vendor_id != 0 || product_id != 0 {
            let Ok((_, dev_vid, dev_pid)) =
                parse_hid_vid_pid_from_sysfs(&sysfs_path.to_string_lossy())
            else {
                continue;
            };

            if vendor_id != 0 && vendor_id != dev_vid {
                continue;
            }
            if product_id != 0 && product_id != dev_pid {
                continue;
            }
        }

        let Some(raw_dev) = udev.device_from_syspath(sysfs_path) else {
            continue;
        };

        let mut tmp = create_device_info_for_device(raw_dev.as_ref());

        if let Some(first) = tmp.first_mut() {
            if first.manufacturer_string.is_none() {
                if let Some(manufacturer_string) = hwdb_vendor(syms, &udev, vendor_id) {
                    first.manufacturer_string = utf8_to_wchar_t(Some(&manufacturer_string));
                }
            }
        }

        // (move to the tail of the returned list)
        root.append(&mut tmp);
    }

    // (with no devices, upstream sets "No HID devices found in the system."
    // or "No HID devices with requested VID/PID found in the system.")
    root
}

/// The `ID_VENDOR_FROM_DATABASE` of a vendor in the hardware database, if
/// libudev has one (the hwdb lookup of `hid_enumerate()`).
fn hwdb_vendor(syms: &UdevSymbols, udev: &Udev<'_>, vendor_id: u16) -> Option<Vec<u8>> {
    let (Some(hwdb_new), Some(hwdb_unref), Some(get_properties)) = (
        syms.udev_hwdb_new,
        syms.udev_hwdb_unref,
        syms.udev_hwdb_get_properties_list_entry,
    ) else {
        return None;
    };
    const KEY: &[u8] = b"ID_VENDOR_FROM_DATABASE";

    // FIXME (upstream): the modalias is built from the vendor_id argument of
    // hid_enumerate() (0 when enumerating all devices), not from the
    // device's own vendor ID.
    let modalias = CString::new(format!("usb:v{vendor_id:04X}*")).expect("no NUL");
    let mut result = None;
    // SAFETY: the context is live; the hwdb and its list entries are used
    // before the hwdb is unreferenced, and the strings are copied.
    unsafe {
        let hwdb = hwdb_new(udev.udev.as_ptr());
        if !hwdb.is_null() {
            let mut entry = get_properties(hwdb, modalias.as_ptr(), 0);
            while !entry.is_null() {
                let name = (syms.udev_list_entry_get_name)(entry);
                if !name.is_null() && CStr::from_ptr(name).to_bytes() == KEY {
                    let value = (syms.udev_list_entry_get_value)(entry);
                    if !value.is_null() {
                        result = Some(CStr::from_ptr(value).to_bytes().to_vec());
                    }
                    break;
                }
                entry = (syms.udev_list_entry_get_next)(entry);
            }
            hwdb_unref(hwdb);
        }
    }
    result
}

/// Translation of `hid_open()`.
pub(super) fn open(vendor_id: u16, product_id: u16, serial_number: Option<&str>) -> Result<Device> {
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
        .map(|cur_dev| cur_dev.path.clone());

    match path_to_open {
        // Open the device
        Some(Some(path)) => open_path(&path),
        // (a NULL path counts as not found)
        _ => Err(Error::new(
            "Device with requested VID/PID/(SerialNumber) not found",
        )),
    }
}

/// Translation of `hid_open_path()`.
pub(super) fn open_path(path: &str) -> Result<Device> {
    const MAX_ATTEMPTS: i32 = 50;
    let c_path = CString::new(path).map_err(|_| {
        Error::new(format!(
            "Failed to open a device with path '{path}': Invalid argument"
        ))
    })?;

    let mut device_handle = -1;
    let mut error = 0;
    for _attempt in 1..=MAX_ATTEMPTS {
        // SAFETY: the path is NUL-terminated.
        device_handle = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if device_handle < 0 {
            error = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if error == libc::EACCES {
                // udev might be setting up permissions, wait a bit and try again
                std::thread::sleep(std::time::Duration::from_millis(1));
                continue;
            }
        }
        break;
    }

    if device_handle < 0 {
        // Unable to open a device.
        return Err(Error::new(format!(
            "Failed to open a device with path '{path}': {}",
            errno_string(error)
        )));
    }

    // SAFETY: the fd was just opened and is owned by nothing else.
    let fd = unsafe { OwnedFd::from_raw_fd(device_handle) };

    // Make sure this is a HIDRAW device - responds to HIDIOCGRDESCSIZE
    let mut desc_size: c_int = 0;
    // SAFETY: the request writes an int.
    let res = unsafe {
        ioctl_ptr(
            fd.as_raw_fd(),
            HIDIOCGRDESCSIZE,
            (&mut desc_size as *mut c_int).cast(),
        )
    };
    if res < 0 {
        return Err(Error::new(format!(
            "ioctl(GRDESCSIZE) error for '{path}', not a HIDRAW device?: {}",
            strerror()
        )));
    }

    let mut dev = Device {
        fd,
        blocking: AtomicBool::new(true),
        needs_ble_hack: false,
        device_info: Mutex::new(None),
    };
    dev.needs_ble_hack = dev.is_ble() == 1;

    Ok(dev)
}

/// An open hidraw device. Translation of `struct hid_device_`.
#[derive(Debug)]
pub(super) struct Device {
    /// `device_handle`
    fd: OwnedFd,
    blocking: AtomicBool,
    needs_ble_hack: bool,
    /// The cached `hid_get_device_info()`.
    device_info: Mutex<Option<Vec<DeviceInfo>>>,
}

impl Device {
    fn raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    /// The `st_rdev` of the device node.
    fn devnum(&self) -> Option<libc::dev_t> {
        // SAFETY: stat is plain data that fstat fills in.
        let mut s: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: the fd is open and s is writable.
        if unsafe { libc::fstat(self.raw_fd(), &mut s) } < 0 {
            return None;
        }
        Some(s.st_rdev)
    }

    /// Translation of `is_BLE()`: 1 for a Bluetooth device that needs the
    /// feature report fix, 0 if not, -1 on error.
    fn is_ble(&self) -> i32 {
        let Some(syms) = udev_ctx() else {
            return -1;
        };

        // Create the udev object
        let Some(udev) = Udev::new(&syms) else {
            crate::log::debug!(crate::log::Category::Input, "Can't create udev");
            return -1;
        };

        // Get the dev_t (major/minor numbers) from the file handle.
        let Some(devnum) = self.devnum() else {
            return -1;
        };

        // Open a udev device from the dev_t. 'c' means character device.
        let mut ret = 0;
        if let Some(udev_dev) = udev.device_from_devnum(devnum) {
            if let Some(uevent) = hid_parent_uevent(udev_dev.as_ref()) {
                if uevent.bus_type == BUS_BLUETOOTH {
                    // Right now the Steam Controller is the only BLE device that we send feature reports to
                    if uevent.vendor_id == 0x28de
                    /* Valve */
                    {
                        ret = 1;
                    }
                }
            }
        }
        ret
    }

    /// Translation of `create_device_info_for_hid_device()`.
    fn create_device_info(&self) -> Result<Vec<DeviceInfo>> {
        // Get the dev_t (major/minor numbers) from the file handle.
        let devnum = self
            .devnum()
            .ok_or_else(|| Error::new("Failed to stat device handle"))?;

        // Create the udev object
        let syms = udev_ctx().ok_or_else(|| Error::new("Couldn't create udev context"))?;
        let udev = Udev::new(&syms).ok_or_else(|| Error::new("Couldn't create udev context"))?;

        // Open a udev device from the dev_t. 'c' means character device.
        let root = udev
            .device_from_devnum(devnum)
            .map(|udev_dev| create_device_info_for_device(udev_dev.as_ref()))
            .unwrap_or_default();

        if root.is_empty() {
            // TODO: have a better error reporting via create_device_info_for_device
            return Err(Error::new("Couldn't create hid_device_info"));
        }
        Ok(root)
    }

    /// Translation of `hid_write()`.
    pub(super) fn write(&self, data: &[u8]) -> Result<usize> {
        if data.is_empty() {
            return Err(Error::new(errno_string(libc::EINVAL)));
        }

        // SAFETY: data is readable for its length.
        let bytes_written = unsafe { libc::write(self.raw_fd(), data.as_ptr().cast(), data.len()) };
        if bytes_written < 0 {
            return Err(Error::new(strerror()));
        }
        Ok(bytes_written as usize)
    }

    /// Translation of `hid_read_timeout()`.
    pub(super) fn read_timeout(&self, data: &mut [u8], milliseconds: i32) -> Result<usize> {
        if milliseconds >= 0 {
            /* Milliseconds is either 0 (non-blocking) or > 0 (contains
            a valid timeout). In both cases we want to call poll()
            and wait for data to arrive.  Don't rely on non-blocking
            operation (O_NONBLOCK) since some kernels don't seem to
            properly report device disconnection through read() when
            in non-blocking mode.  */
            let mut fds = libc::pollfd {
                fd: self.raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one valid pollfd.
            let ret = unsafe { libc::poll(&mut fds, 1, milliseconds) };
            if ret == 0 {
                // Timeout
                return Ok(0);
            }
            if ret == -1 {
                // Error
                return Err(Error::new(strerror()));
            }
            /* Check for errors on the file descriptor. This will
            indicate a device disconnection. */
            if fds.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                // We cannot use strerror() here as no -1 was returned from poll().
                return Err(Error::new(
                    "hid_read_timeout: unexpected poll error (device disconnected)",
                ));
            }
        }

        // SAFETY: data is writable for its length.
        let bytes_read = unsafe { libc::read(self.raw_fd(), data.as_mut_ptr().cast(), data.len()) };
        if bytes_read < 0 {
            let error = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if error == libc::EAGAIN || error == libc::EINPROGRESS {
                return Ok(0);
            }
            return Err(Error::new(errno_string(error)));
        }
        Ok(bytes_read as usize)
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
        /* Do all non-blocking in userspace using poll(), since it looks
        like there's a bug in the kernel in some versions where
        read() will not return -1 on disconnection of the USB device */
        self.blocking.store(!nonblock, Ordering::Relaxed);
        Ok(()) // Success
    }

    /// Translation of `hid_send_feature_report()`.
    pub(super) fn send_feature_report(&self, data: &[u8]) -> Result<usize> {
        const MAX_RETRIES: i32 = 50;

        for _retry in 0..MAX_RETRIES {
            // SAFETY: the request reads data.len() bytes (it is _IOWR, but
            // the kernel only writes the report back for the read
            // direction of GFEATURE).
            let res = unsafe {
                ioctl_ptr(
                    self.raw_fd(),
                    hidiocsfeature(data.len()),
                    data.as_ptr() as *mut c_void,
                )
            };
            if res < 0 {
                let error = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                if error == libc::EPIPE {
                    // Try again...
                    continue;
                }
                return Err(Error::new(format!(
                    "ioctl (SFEATURE): {}",
                    errno_string(error)
                )));
            }
            return Ok(res as usize);
        }
        // FIXME (upstream): after MAX_RETRIES broken pipes the failure is
        // returned with the error cleared, so it has no message.
        Err(Error::new(""))
    }

    /// Translation of `hid_get_feature_report()`.
    pub(super) fn get_feature_report(&self, data: &mut [u8]) -> Result<usize> {
        let length = data.len();
        let Some(&report) = data.first() else {
            return Err(Error::new(format!(
                "ioctl (GFEATURE): {}",
                errno_string(libc::EINVAL)
            )));
        };

        // SAFETY: the request reads and writes length bytes of data.
        let res = unsafe {
            ioctl_ptr(
                self.raw_fd(),
                hidiocgfeature(length),
                data.as_mut_ptr().cast(),
            )
        };
        if res < 0 {
            return Err(Error::new(format!("ioctl (GFEATURE): {}", strerror())));
        }
        let mut res = res as usize;
        if self.needs_ble_hack {
            /* Versions of BlueZ before 5.56 don't include the report in the data,
             * and versions of BlueZ >= 5.56 include 2 copies of the report.
             * We'll fix it so that there is a single copy of the report in both cases
             */
            // FIXME (upstream): both moves copy `res` bytes, one past the
            // end of the buffer when the report fills it; here they stop
            // at its end.
            if length > 1 && data[0] == report && data[1] == report {
                let n = res.min(length - 1);
                data.copy_within(1..1 + n, 0);
            } else if data[0] != report {
                let n = res.min(length - 1);
                data.copy_within(0..n, 1);
                data[0] = report;
                res = (res + 1).min(length);
            }
        }

        Ok(res)
    }

    /// Translation of `hid_get_input_report()`.
    pub(super) fn get_input_report(&self, data: &mut [u8]) -> Result<usize> {
        // SAFETY: the request reads and writes data.len() bytes of data.
        let res = unsafe {
            ioctl_ptr(
                self.raw_fd(),
                hidiocginput(data.len()),
                data.as_mut_ptr().cast(),
            )
        };
        if res < 0 {
            return Err(Error::new(format!("ioctl (GINPUT): {}", strerror())));
        }
        Ok(res as usize)
    }

    /// Translation of `hid_get_device_info()`: the device's records,
    /// created on first use (the first one is the device info).
    fn device_info_list(&self) -> Result<Vec<DeviceInfo>> {
        let mut cached = self.device_info.lock().unwrap_or_else(|e| e.into_inner());
        if cached.is_none() {
            // Lazy initialize device_info
            *cached = Some(self.create_device_info()?);
        }
        Ok(cached.clone().unwrap_or_default())
    }

    /// Translation of `hid_get_device_info()`.
    pub(super) fn get_device_info(&self) -> Result<DeviceInfo> {
        self.device_info_list()?
            .into_iter()
            .next()
            .ok_or_else(|| Error::new("Couldn't create hid_device_info"))
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

    /// Translation of `hid_get_indexed_string()`.
    pub(super) fn get_indexed_string(&self, _string_index: i32) -> Result<String> {
        Err(Error::new(
            "hid_get_indexed_string: not supported by hidraw",
        ))
    }

    /// Translation of `get_hid_report_descriptor_from_hidraw()`.
    fn get_hid_report_descriptor_from_hidraw(&self) -> Result<Box<HidrawReportDescriptor>> {
        let mut desc_size: c_int = 0;

        // Get Report Descriptor Size
        // SAFETY: the request writes an int.
        let res = unsafe {
            ioctl_ptr(
                self.raw_fd(),
                HIDIOCGRDESCSIZE,
                (&mut desc_size as *mut c_int).cast(),
            )
        };
        if res < 0 {
            return Err(Error::new(format!("ioctl(GRDESCSIZE): {}", strerror())));
        }

        // Get Report Descriptor
        let mut rpt_desc = Box::new(HidrawReportDescriptor {
            size: desc_size as u32,
            value: [0; HID_MAX_DESCRIPTOR_SIZE],
        });
        // SAFETY: the request reads the size and writes the whole structure.
        let res = unsafe {
            ioctl_ptr(
                self.raw_fd(),
                HIDIOCGRDESC,
                (&mut *rpt_desc as *mut HidrawReportDescriptor).cast(),
            )
        };
        if res < 0 {
            return Err(Error::new(format!("ioctl(GRDESC): {}", strerror())));
        }

        Ok(rpt_desc)
    }

    /// Translation of `hid_get_report_descriptor()`.
    pub(super) fn get_report_descriptor(&self, buf: &mut [u8]) -> Result<usize> {
        let rpt_desc = self.get_hid_report_descriptor_from_hidraw()?;
        let bytes = rpt_desc.bytes();
        let buf_size = buf.len().min(bytes.len());
        buf[..buf_size].copy_from_slice(&bytes[..buf_size]);
        Ok(buf_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_sizes() {
        // Short items: the size code is the low two bits, 3 means 4 bytes
        assert_eq!(get_hid_item_size(&[0x05, 0x01], 0), Some((1, 1)));
        assert_eq!(get_hid_item_size(&[0x06, 0x00, 0xff], 0), Some((2, 1)));
        assert_eq!(get_hid_item_size(&[0x07], 0), Some((4, 1)));
        assert_eq!(get_hid_item_size(&[0xc0], 0), Some((0, 1)));
        // A long item, and a malformed one at the end (short item sizes)
        assert_eq!(get_hid_item_size(&[0xfe, 0x10], 0), Some((16, 3)));
        assert_eq!(get_hid_item_size(&[0xfe], 0), Some((2, 1)));
        assert_eq!(get_hid_item_size(&[0xff], 0), Some((4, 1)));
    }

    #[test]
    fn report_bytes() {
        let rpt = [0x27, 0xff, 0xff, 0x00, 0x00, 0x99];
        assert_eq!(get_hid_report_bytes(&rpt, 0, 0), 0);
        assert_eq!(get_hid_report_bytes(&rpt, 1, 0), 0xff);
        assert_eq!(get_hid_report_bytes(&rpt, 2, 0), 0xffff);
        assert_eq!(get_hid_report_bytes(&rpt, 4, 0), 0x0000ffff);
        // Not enough bytes
        assert_eq!(get_hid_report_bytes(&rpt, 4, 1), 0x990000ff);
        assert_eq!(get_hid_report_bytes(&rpt, 4, 2), 0);
        assert_eq!(get_hid_report_bytes(&rpt, 3, 0), 0);
    }

    #[test]
    fn uevent_parsing() {
        let uevent = "DRIVER=sony\nHID_ID=0005:0000054C:000009CC\nHID_NAME=Wireless Controller\n\
                      HID_PHYS=00:11:22:33:44:55\nHID_UNIQ=aa:bb:cc:dd:ee:ff\nMODALIAS=hid:b0005g0000v0000054Cp000009CC\n";
        let info = parse_uevent_info(Some(uevent));
        assert_eq!(
            info,
            UeventInfo {
                bus_type: 5,
                vendor_id: 0x054c,
                product_id: 0x09cc,
                serial_number_utf8: Some("aa:bb:cc:dd:ee:ff".into()),
                product_name_utf8: Some("Wireless Controller".into()),
                complete: true,
            }
        );
        assert_eq!(
            parse_hid_vid_pid_from_uevent(uevent).unwrap(),
            (5, 0x054c, 0x09cc)
        );

        // Without a serial number the information isn't complete
        let info = parse_uevent_info(Some("HID_ID=0003:0000045E:0000028E\nHID_NAME=X\n"));
        assert!(!info.complete);
        assert_eq!((info.vendor_id, info.product_id), (0x045e, 0x028e));
        assert!(!parse_uevent_info(None).complete);
        assert!(parse_hid_vid_pid_from_uevent("HID_NAME=X\n").is_err());
        // Malformed IDs aren't found
        assert!(parse_hid_vid_pid_from_uevent("HID_ID=0003-0000045E\n").is_err());
        // "%hx" keeps the low 16 bits
        assert_eq!(parse_hid_id("3:1234ABCD:0x12"), Some((3, 0xabcd, 0x12)));
    }

    /// The pairs and final result of the usage iterator on a descriptor.
    fn usages_and_end(descriptor: &[u8]) -> (Vec<(u16, u16)>, i32) {
        let mut ctx = HidUsageIterator::default();
        let mut pairs = Vec::new();
        loop {
            match get_next_hid_usage(descriptor, &mut ctx) {
                NextUsage::Found(page, usage) => pairs.push((page, usage)),
                NextUsage::Finished => return (pairs, 1),
                NextUsage::Malformed => return (pairs, -1),
            }
        }
    }

    #[test]
    fn usages_of_real_descriptors() {
        // (the descriptors of the Windows reconstructor tests; the expected
        // pairs come from upstream's get_next_hid_usage() compiled from C)
        let xbox = include_bytes!("testdata/045E_02FF_0005_0001.rpt_desc");
        let mouse = include_bytes!("testdata/046D_C534_0002_0001.rpt_desc");
        let headset = include_bytes!("testdata/047F_C056_0003_FFA0.rpt_desc");
        assert_eq!(usages_and_end(xbox), (vec![(0x0001, 0x0005)], 1));
        assert_eq!(usages_and_end(mouse), (vec![(0x0001, 0x0002)], 1));
        let all = [xbox.as_slice(), mouse, headset].concat();
        assert_eq!(
            usages_and_end(&all),
            (
                vec![(0x0001, 0x0005), (0x0001, 0x0002), (0xffa0, 0x0003)],
                1
            )
        );
        assert_eq!(hid_usages(&all).len(), 3);
    }

    #[test]
    fn usages_of_random_descriptors() {
        // Random descriptors made of common item bytes; the expected results
        // come from upstream's get_next_hid_usage() compiled from C with the
        // same generator.
        type Case = (usize, &'static [(u16, u16)], i32);
        const EXPECTED: [Case; 64] = [
            (38, &[], 1),
            (127, &[(0x000b, 0x00a1)], -1),
            (194, &[(0x00fe, 0x0100)], -1),
            (51, &[], -1),
            (31, &[], -1),
            (38, &[], -1),
            (113, &[], -1),
            (122, &[], -1),
            (199, &[(0xa101, 0x30fe)], -1),
            (91, &[], -1),
            (78, &[], -1),
            (49, &[], 1),
            (66, &[], -1),
            (57, &[(0xc0ff, 0xc001)], 1),
            (149, &[(0x0205, 0x30a1)], 1),
            (105, &[], -1),
            (113, &[(0x0a75, 0xfeff)], 1),
            (157, &[(0x0a15, 0x0007)], 1),
            (198, &[], -1),
            (142, &[], -1),
            (27, &[], -1),
            (63, &[], 1),
            (84, &[], -1),
            (152, &[], -1),
            (22, &[], -1),
            (108, &[], -1),
            (138, &[], -1),
            (78, &[(0x0007, 0xc0fe)], 1),
            (104, &[(0x0681, 0x0030)], 1),
            (57, &[], -1),
            (121, &[], -1),
            (66, &[(0x30a0, 0x0007)], 1),
            (149, &[], -1),
            (14, &[(0x0000, 0x00a0)], 1),
            (169, &[], -1),
            (29, &[], -1),
            (79, &[], 1),
            (135, &[], -1),
            (30, &[], -1),
            (24, &[], -1),
            (157, &[], -1),
            (52, &[], -1),
            (167, &[], -1),
            (124, &[], -1),
            (114, &[], -1),
            (52, &[], 1),
            (174, &[(0xc009, 0x0007)], 1),
            (58, &[(0x8181, 0xfe00)], 1),
            (158, &[], -1),
            (66, &[], -1),
            (21, &[], 1),
            (187, &[], -1),
            (54, &[], -1),
            (36, &[], -1),
            (188, &[], -1),
            (41, &[], -1),
            (180, &[], 1),
            (112, &[], -1),
            (21, &[(0x0000, 0x1575)], 1),
            (172, &[], -1),
            (145, &[], -1),
            (40, &[], -1),
            (43, &[], -1),
            (147, &[(0x0007, 0x0075)], 1),
        ];
        const ALPHABET: [u8; 20] = [
            0x05, 0x06, 0x07, 0x09, 0x0a, 0x0b, 0xa1, 0xa1, 0xc0, 0xc0, 0x15, 0x75, 0xfe, 0x01,
            0x00, 0x02, 0x30, 0x81, 0xff, 0xa0,
        ];
        let mut state: u32 = 0x12345678;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        for (len, pairs, end) in EXPECTED {
            let n = 1 + next() as usize % 200;
            assert_eq!(n, len);
            let descriptor: Vec<u8> = (0..n)
                .map(|_| ALPHABET[next() as usize % ALPHABET.len()])
                .collect();
            assert_eq!(
                usages_and_end(&descriptor),
                (pairs.to_vec(), end),
                "{descriptor:02x?}"
            );
        }
    }

    #[test]
    fn hidraw_node_names() {
        assert!(super::super::is_hidraw_node(b"hidraw0"));
        assert!(super::super::is_hidraw_node(b"hidraw12"));
        assert!(!super::super::is_hidraw_node(b"hidraw"));
        assert!(!super::super::is_hidraw_node(b"hidraw1a"));
        assert!(!super::super::is_hidraw_node(b"input0"));
    }

    #[test]
    fn ioctl_requests() {
        // (from <linux/hidraw.h> on x86-64)
        if cfg!(target_arch = "x86_64") {
            assert_eq!(HIDIOCGRDESCSIZE, 0x80044801);
            assert_eq!(HIDIOCGRDESC, 0x90044802);
            assert_eq!(hidiocsfeature(64), 0xC0404806);
            assert_eq!(hidiocgfeature(64), 0xC0404807);
            assert_eq!(hidiocginput(64), 0xC040480A);
        }
    }
}
