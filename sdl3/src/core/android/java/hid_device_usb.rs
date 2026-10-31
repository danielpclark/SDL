// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// HIDDeviceUSB.java from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `class HIDDeviceUSB implements HIDDevice`: an interface of a USB device
//! (`UsbDevice`), opened through the `UsbManager` (`openDevice()`, its
//! interface claimed, forced), its input reports read from its first IN
//! endpoint by a thread of its own (`bulkTransfer()`, a second's timeout:
//! upstream's `InputThread`, a Java `Thread`, is a Rust thread attached to
//! the VM here, stopped through its `mRunning` without Java's interrupt,
//! which a transfer doesn't heed), its output reports written to its first
//! OUT endpoint, and its feature reports (and input reports asked for) sent
//! and read with HID's class requests (`controlTransfer()`). While frozen
//! (the app not in front) its interface is released, for other apps, and
//! its reports dropped.
//!
//! It isn't a framework class: its fields are Rust (the connection and the
//! endpoints shared with the input thread).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::thread::JoinHandle;

use super::super::env::{self, Arg, Env, Global, JniResult, Local, Obj};
use super::super::jni::*;
use super::super::ndk::LogPriority;
use super::hid_device::HidDevice;
use super::hid_device_manager::{self as manager, log, HidDeviceManager};
use crate::hidapi::android as hid;

/// `UsbConstants.USB_DIR_OUT`, `USB_DIR_IN`, `USB_TYPE_CLASS`.
const USB_DIR_OUT: jint = 0;
const USB_DIR_IN: jint = 0x80;
const USB_TYPE_CLASS: jint = 0x20;
/// RECIPIENT_INTERFACE
const RECIPIENT_INTERFACE: jint = 0x01;
/// HID set_report, get_report
const HID_SET_REPORT: jint = 0x09;
const HID_GET_REPORT: jint = 0x01;
/// HID feature, HID Input
const HID_FEATURE: jint = 3;
const HID_INPUT: jint = 1;
/// timeout millis
const TIMEOUT: jint = 1000;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The fields the input thread shares: `mConnection`, `mInputEndpoint`,
/// `mRunning`, `mFrozen`.
#[derive(Debug, Default)]
struct Shared {
    running: AtomicBool,
    frozen: AtomicBool,
}

/// Translation of `class HIDDeviceUSB`'s fields.
#[derive(Debug)]
pub(crate) struct HidDeviceUsb {
    /// `mManager`
    manager: Mutex<Option<Weak<HidDeviceManager>>>,
    /// `mDevice`
    device: Global,
    /// `mInterfaceIndex`
    interface_index: jint,
    /// `mInterface`
    interface: jint,
    /// `mDeviceId`
    device_id: jint,
    /// `mConnection`
    connection: Mutex<Option<Arc<Global>>>,
    /// `mInputEndpoint`
    input_endpoint: Mutex<Option<Arc<Global>>>,
    /// `mOutputEndpoint`
    output_endpoint: Mutex<Option<Arc<Global>>>,
    /// `mInputThread`
    input_thread: Mutex<Option<JoinHandle<()>>>,
    /// `mRunning`, `mFrozen`
    shared: Arc<Shared>,
    /// `mClaimed`
    claimed: AtomicBool,
}

/// `String.format("%x", value)`.
fn hex(value: jint) -> String {
    format!("{:x}", value as u32)
}

impl HidDeviceUsb {
    /// Translation of the constructor `HIDDeviceUSB(HIDDeviceManager,
    /// UsbDevice, int)`.
    pub(super) fn new<'a>(
        env: Env<'a>,
        manager: &HidDeviceManager,
        usb_device: Obj<'_>,
        interface_index: jint,
    ) -> JniResult<'a, Arc<HidDeviceUsb>> {
        let usb_interface = usb_interface(env, usb_device, interface_index)?;
        let interface = env
            .call_method(usb_interface.obj(), c"getId", c"()I", &[])?
            .int();
        let mut device = HidDeviceUsb {
            manager: Mutex::new(Some(manager.weak())),
            device: Global::new(env, usb_device).expect("a global reference"),
            interface_index,
            interface,
            device_id: 0,
            connection: Mutex::new(None),
            input_endpoint: Mutex::new(None),
            output_endpoint: Mutex::new(None),
            input_thread: Mutex::new(None),
            shared: Arc::new(Shared::default()),
            claimed: AtomicBool::new(false),
        };
        device.device_id =
            manager.get_device_id_for_identifier(env, &device.get_identifier(env)?)?;
        Ok(Arc::new(device))
    }

    /// `mManager`, the `NullPointerException` of a call on it once it's
    /// shut down.
    fn manager<'a>(&self, env: Env<'a>) -> JniResult<'a, Arc<HidDeviceManager>> {
        match lock(&self.manager).as_ref().and_then(Weak::upgrade) {
            Some(manager) => Ok(manager),
            None => manager::null_pointer(env, c"a call on a null HIDDeviceManager"),
        }
    }

    fn device_int<'a>(&self, env: Env<'a>, name: &std::ffi::CStr) -> JniResult<'a, jint> {
        Ok(env.call_method(self.device.obj(), name, c"()I", &[])?.int())
    }

    /// Translation of `getIdentifier()`.
    pub(super) fn get_identifier<'a>(&self, env: Env<'a>) -> JniResult<'a, String> {
        let name = manager::call_string(env, self.device.obj(), c"getDeviceName")?;
        Ok(format!(
            "{}/{}/{}/{}",
            name.as_deref().unwrap_or("null"),
            hex(self.device_int(env, c"getVendorId")?),
            hex(self.device_int(env, c"getProductId")?),
            self.interface_index
        ))
    }

    /// Translation of `getDeviceName()`.
    fn get_device_name<'a>(&self, env: Env<'a>) -> JniResult<'a, String> {
        Ok(format!(
            "{} {}(0x{}/0x{})",
            self.get_manufacturer_name(env)?,
            self.get_product_name(env)?,
            hex(self.get_vendor_id(env)?),
            hex(self.get_product_id(env)?)
        ))
    }

    /// `mDevice.getInterface(mInterfaceIndex)`.
    fn interface<'a>(&self, env: Env<'a>) -> JniResult<'a, Local<'a>> {
        usb_interface(env, self.device.obj(), self.interface_index)
    }

    fn connection(&self) -> Option<Arc<Global>> {
        lock(&self.connection).clone()
    }
}

/// `usbDevice.getInterface(index)`.
fn usb_interface<'a>(env: Env<'a>, usb_device: Obj<'_>, index: jint) -> JniResult<'a, Local<'a>> {
    match env
        .call_method(
            usb_device,
            c"getInterface",
            c"(I)Landroid/hardware/usb/UsbInterface;",
            &[Arg::Int(index)],
        )?
        .object()
    {
        Some(usb_interface) => Ok(usb_interface),
        None => manager::null_pointer(env, c"a null interface"),
    }
}

/// `report[0]`, the `ArrayIndexOutOfBoundsException` of an empty report.
fn report_number<'a>(env: Env<'a>, report: &[u8]) -> JniResult<'a, i8> {
    match report.first() {
        Some(&number) => Ok(number as i8),
        None => manager::throw(
            env,
            c"java/lang/ArrayIndexOutOfBoundsException",
            c"length=0; index=0",
        ),
    }
}

/// A class request's `wValue`: the report type and number. (Note
/// (upstream): the report number, a Java `byte`, is sign-extended: a number
/// of 0x80 or more is ORed in as a negative value.)
fn report_value(report_type: jint, report_number: i8) -> jint {
    (report_type << 8) | jint::from(report_number)
}

/// `connection.controlTransfer(requestType, request, value, index, buffer,
/// offset, length, timeout)`.
#[allow(clippy::too_many_arguments)]
fn control_transfer<'a>(
    env: Env<'a>,
    connection: Obj<'_>,
    request_type: jint,
    request: jint,
    value: jint,
    index: jint,
    buffer: Obj<'_>,
    offset: jint,
    length: jint,
) -> JniResult<'a, jint> {
    Ok(env
        .call_method(
            connection,
            c"controlTransfer",
            c"(IIII[BIII)I",
            &[
                Arg::Int(request_type),
                Arg::Int(request),
                Arg::Int(value),
                Arg::Int(index),
                Arg::Object(buffer),
                Arg::Int(offset),
                Arg::Int(length),
                Arg::Int(TIMEOUT),
            ],
        )?
        .int())
}

/// `connection.bulkTransfer(endpoint, buffer, length, timeout)`.
fn bulk_transfer<'a>(
    env: Env<'a>,
    connection: Obj<'_>,
    endpoint: Obj<'_>,
    buffer: Obj<'_>,
    length: jint,
) -> JniResult<'a, jint> {
    Ok(env
        .call_method(
            connection,
            c"bulkTransfer",
            c"(Landroid/hardware/usb/UsbEndpoint;[BII)I",
            &[
                Arg::Object(endpoint),
                Arg::Object(buffer),
                Arg::Int(length),
                Arg::Int(TIMEOUT),
            ],
        )?
        .int())
}

/// `connection.claimInterface(iface, true)`.
fn claim_interface<'a>(env: Env<'a>, connection: Obj<'_>, iface: Obj<'_>) -> JniResult<'a, bool> {
    Ok(env
        .call_method(
            connection,
            c"claimInterface",
            c"(Landroid/hardware/usb/UsbInterface;Z)Z",
            &[Arg::Object(iface), Arg::Boolean(true)],
        )?
        .boolean())
}

/// `connection.releaseInterface(iface)`.
fn release_interface<'a>(env: Env<'a>, connection: Obj<'_>, iface: Obj<'_>) -> JniResult<'a, bool> {
    Ok(env
        .call_method(
            connection,
            c"releaseInterface",
            c"(Landroid/hardware/usb/UsbInterface;)Z",
            &[Arg::Object(iface)],
        )?
        .boolean())
}

impl HidDevice for HidDeviceUsb {
    fn get_id(&self) -> jint {
        self.device_id
    }

    fn get_vendor_id<'a>(&self, env: Env<'a>) -> JniResult<'a, jint> {
        self.device_int(env, c"getVendorId")
    }

    fn get_product_id<'a>(&self, env: Env<'a>) -> JniResult<'a, jint> {
        self.device_int(env, c"getProductId")
    }

    fn get_serial_number<'a>(&self, env: Env<'a>) -> JniResult<'a, String> {
        let result = match manager::call_string(env, self.device.obj(), c"getSerialNumber") {
            Ok(result) => result,
            Err(thrown) => {
                // (Log.w("App permissions mean we cannot get serial number for device " + ...), commented out)
                drop(thrown.catch_exception()?);
                None
            }
        };
        Ok(result.unwrap_or_default())
    }

    fn get_version(&self) -> jint {
        0
    }

    fn get_manufacturer_name<'a>(&self, env: Env<'a>) -> JniResult<'a, String> {
        let result = manager::call_string(env, self.device.obj(), c"getManufacturerName")?;
        match result {
            Some(result) => Ok(result),
            None => Ok(hex(self.get_vendor_id(env)?)),
        }
    }

    fn get_product_name<'a>(&self, env: Env<'a>) -> JniResult<'a, String> {
        let result = manager::call_string(env, self.device.obj(), c"getProductName")?;
        match result {
            Some(result) => Ok(result),
            None => Ok(hex(self.get_product_id(env)?)),
        }
    }

    fn get_device<'a>(&self, env: Env<'a>) -> Option<Local<'a>> {
        env.new_local_ref(self.device.obj())
    }

    fn open<'a>(&self, env: Env<'a>) -> JniResult<'a, bool> {
        let usb_manager = self.manager(env)?.get_usb_manager(env);
        let Some(usb_manager) = usb_manager else {
            return manager::null_pointer(env, c"openDevice() on a null UsbManager");
        };
        let connection = env
            .call_method(
                usb_manager.obj(),
                c"openDevice",
                c"(Landroid/hardware/usb/UsbDevice;)Landroid/hardware/usb/UsbDeviceConnection;",
                &[Arg::Object(self.device.obj())],
            )?
            .object();
        let connection =
            connection.map(|c| Arc::new(Global::new(env, c.obj()).expect("a global reference")));
        *lock(&self.connection) = connection.clone();
        let Some(connection) = connection else {
            log(
                LogPriority::Warn,
                &format!("Unable to open USB device {}", self.get_device_name(env)?),
            );
            return Ok(false);
        };

        // Force claim our interface
        let iface = self.interface(env)?;
        if !claim_interface(env, connection.obj(), iface.obj())? {
            log(
                LogPriority::Warn,
                &format!(
                    "Failed to claim interfaces on USB device {}",
                    self.get_device_name(env)?
                ),
            );
            self.close(env)?;
            return Ok(false);
        }
        self.claimed.store(true, Ordering::SeqCst);

        // Find the endpoints
        let endpoint_count = env
            .call_method(iface.obj(), c"getEndpointCount", c"()I", &[])?
            .int();
        for j in 0..endpoint_count {
            let endpt = env
                .call_method(
                    iface.obj(),
                    c"getEndpoint",
                    c"(I)Landroid/hardware/usb/UsbEndpoint;",
                    &[Arg::Int(j)],
                )?
                .object();
            let Some(endpt) = endpt else {
                return manager::null_pointer(env, c"getDirection() on a null endpoint");
            };
            match env
                .call_method(endpt.obj(), c"getDirection", c"()I", &[])?
                .int()
            {
                USB_DIR_IN => {
                    let mut input = lock(&self.input_endpoint);
                    if input.is_none() {
                        *input = Global::new(env, endpt.obj()).map(Arc::new);
                    }
                }
                USB_DIR_OUT => {
                    let mut output = lock(&self.output_endpoint);
                    if output.is_none() {
                        *output = Global::new(env, endpt.obj()).map(Arc::new);
                    }
                }
                _ => {}
            }
        }

        // Make sure the required endpoints were present. The original Steam Controller and the wireless dongle for it do NOT
        // actually have -- or require -- output endpoints, so we need to accept only an input one for them or else we'll fall
        // back to the Android system gamepad functionality (and lose our paddles et al).
        let input_endpoint = lock(&self.input_endpoint).clone();
        let Some(input_endpoint) = input_endpoint else {
            log(
                LogPriority::Warn,
                &format!(
                    "Missing required endpoint on USB device {}",
                    self.get_device_name(env)?
                ),
            );
            release_interface(env, connection.obj(), iface.obj())?;
            self.close(env)?;
            return Ok(false);
        };

        // Start listening for input
        self.shared.running.store(true, Ordering::SeqCst);
        let shared = self.shared.clone();
        let device_id = self.device_id;
        let thread = std::thread::Builder::new()
            .name("HIDDeviceUSB.InputThread".into())
            .spawn(move || input_thread_run(&shared, &connection, &input_endpoint, device_id));
        match thread {
            Ok(thread) => *lock(&self.input_thread) = Some(thread),
            Err(_) => {
                return manager::throw(env, c"java/lang/OutOfMemoryError", c"pthread_create failed")
            }
        }

        Ok(true)
    }

    fn write_report<'a>(&self, env: Env<'a>, report: &[u8], feature: bool) -> JniResult<'a, jint> {
        let Some(connection) = self.connection() else {
            log(
                LogPriority::Warn,
                "writeReport() called with no device connection",
            );
            return Ok(-1);
        };

        if !self.claimed.load(Ordering::SeqCst) {
            log(
                LogPriority::Warn,
                "writeReport() called but some other process currently owns the USB device",
            );
            return Ok(-1);
        }

        let buffer = env.new_byte_array(report)?;
        if feature {
            let mut offset = 0;
            let mut length = report.len() as jint;
            let mut skipped_report_id = false;
            let report_number = report_number(env, report)?;

            if report_number == 0x0 {
                offset += 1;
                length -= 1;
                skipped_report_id = true;
            }

            let res = control_transfer(
                env,
                connection.obj(),
                USB_TYPE_CLASS | RECIPIENT_INTERFACE | USB_DIR_OUT,
                HID_SET_REPORT,
                report_value(HID_FEATURE, report_number),
                self.interface,
                buffer.obj(),
                offset,
                length,
            )?;

            if res < 0 {
                log(
                    LogPriority::Warn,
                    &format!(
                        "writeFeatureReport() returned {res} on device {}",
                        self.get_device_name(env)?
                    ),
                );
                return Ok(-1);
            }

            if skipped_report_id {
                length += 1;
            }
            Ok(length)
        } else {
            let Some(output_endpoint) = lock(&self.output_endpoint).clone() else {
                log(
                    LogPriority::Error,
                    "Tried to write an output report to an interface with no output endpoint!",
                );
                return Ok(-1);
            };
            let res = bulk_transfer(
                env,
                connection.obj(),
                output_endpoint.obj(),
                buffer.obj(),
                report.len() as jint,
            )?;
            if res != report.len() as jint {
                log(
                    LogPriority::Warn,
                    &format!(
                        "writeOutputReport() returned {res} on device {}",
                        self.get_device_name(env)?
                    ),
                );
            }
            Ok(res)
        }
    }

    fn read_report<'a>(&self, env: Env<'a>, report: &[u8], feature: bool) -> JniResult<'a, bool> {
        let mut offset = 0;
        let mut length = report.len() as jint;
        let mut skipped_report_id = false;
        let report_number = report_number(env, report)?;

        let Some(connection) = self.connection() else {
            log(
                LogPriority::Warn,
                "readReport() called with no device connection",
            );
            return Ok(false);
        };
        if !self.claimed.load(Ordering::SeqCst) {
            // (Note (upstream): an input report is said to be read, and
            // never answered)
            return Ok(!feature);
        }

        if report_number == 0x0 {
            /* Offset the return buffer by 1, so that the report ID
            will remain in byte 0. */
            offset += 1;
            length -= 1;
            skipped_report_id = true;
        }

        let buffer = env.new_byte_array(report)?;
        let mut res = control_transfer(
            env,
            connection.obj(),
            USB_TYPE_CLASS | RECIPIENT_INTERFACE | USB_DIR_IN,
            HID_GET_REPORT,
            report_value(if feature { HID_FEATURE } else { HID_INPUT }, report_number),
            self.interface,
            buffer.obj(),
            offset,
            length,
        )?;

        if res < 0 {
            log(
                LogPriority::Warn,
                &format!(
                    "getFeatureReport() returned {res} on device {}",
                    self.get_device_name(env)?
                ),
            );
            return Ok(false);
        }

        if skipped_report_id {
            res += 1;
            length += 1;
        }

        let mut data = env.get_byte_array(buffer.obj())?;
        if res != length {
            data.truncate(res.max(0) as usize);
        }
        // (mManager.HIDDeviceReportResponse(), the native's body)
        self.manager(env)?;
        hid::hid_device_report_response(self.device_id, &data);

        Ok(true)
    }

    fn close<'a>(&self, env: Env<'a>) -> JniResult<'a, ()> {
        self.shared.running.store(false, Ordering::SeqCst);
        let thread = lock(&self.input_thread).take();
        if let Some(thread) = thread {
            // (Java interrupts it, as long as it's alive, and joins it)
            let _ = thread.join();
        }
        let connection = lock(&self.connection).take();
        if let Some(connection) = connection {
            if self.claimed.load(Ordering::SeqCst) {
                let iface = self.interface(env)?;
                release_interface(env, connection.obj(), iface.obj())?;
            }
            env.call_method(connection.obj(), c"close", c"()V", &[])?;
            self.claimed.store(false, Ordering::SeqCst);
        }
        Ok(())
    }

    fn shutdown<'a>(&self, env: Env<'a>) -> JniResult<'a, ()> {
        self.close(env)?;
        *lock(&self.manager) = None;
        Ok(())
    }

    fn set_frozen<'a>(&self, env: Env<'a>, frozen: bool) -> JniResult<'a, ()> {
        self.shared.frozen.store(frozen, Ordering::SeqCst);

        /* If we have a valid device connection and the claim state doesn't match what we want, try to correct that. */
        if let Some(connection) = self.connection() {
            if self.claimed.load(Ordering::SeqCst) == frozen {
                let iface = self.interface(env)?;
                if frozen {
                    let claimed = !release_interface(env, connection.obj(), iface.obj())?;
                    self.claimed.store(claimed, Ordering::SeqCst);
                    if claimed {
                        log(
                            LogPriority::Error,
                            "Tried to release claim on USB device, but failed!",
                        );
                    }
                } else {
                    let claimed = claim_interface(env, connection.obj(), iface.obj())?;
                    self.claimed.store(claimed, Ordering::SeqCst);
                    if !claimed {
                        log(
                            LogPriority::Error,
                            "Tried to regain claim on USB device, but failed!",
                        );
                    }
                }
            }
        }
        Ok(())
    }
}

/// Translation of `InputThread.run()`: the input endpoint's reports, passed
/// on until the device closes.
fn input_thread_run(
    shared: &Shared,
    connection: &Global,
    input_endpoint: &Global,
    device_id: jint,
) {
    let Some(env) = env::get_env() else {
        return;
    };
    if let Err(thrown) = input_thread_loop(env, shared, connection, input_endpoint, device_id) {
        // (an uncaught exception ends Java's thread)
        thrown.describe();
    }
}

fn input_thread_loop<'a>(
    env: Env<'a>,
    shared: &Shared,
    connection: &Global,
    input_endpoint: &Global,
    device_id: jint,
) -> JniResult<'a, ()> {
    let packet_size = env
        .call_method(input_endpoint.obj(), c"getMaxPacketSize", c"()I", &[])?
        .int();
    let packet = env.new_byte_array(&vec![0; packet_size.max(0) as usize])?;
    while shared.running.load(Ordering::SeqCst) {
        let r = match bulk_transfer(
            env,
            connection.obj(),
            input_endpoint.obj(),
            packet.obj(),
            packet_size,
        ) {
            Ok(r) => r,
            Err(thrown) => {
                let e = thrown.catch_exception()?;
                log(
                    LogPriority::Verbose,
                    &format!(
                        "Exception in UsbDeviceConnection bulktransfer: {}",
                        manager::string_of(env, e.obj())?
                    ),
                );
                break;
            }
        };
        if r < 0 {
            // Could be a timeout or an I/O error
        }
        if r > 0 {
            let mut data = env.get_byte_array(packet.obj())?;
            if r != packet_size {
                data.truncate(r as usize);
            }

            if !shared.frozen.load(Ordering::SeqCst) {
                hid::hid_device_input_report(device_id, &data);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_values() {
        assert_eq!(report_value(HID_FEATURE, 0), 0x300);
        assert_eq!(report_value(HID_FEATURE, 5), 0x305);
        assert_eq!(report_value(HID_INPUT, 0x7f), 0x17f);
        // (sign-extended, as Java's byte)
        assert_eq!(
            report_value(HID_FEATURE, 0x85u8 as i8),
            0x85u8 as i8 as jint
        );
        assert_eq!(
            USB_TYPE_CLASS | RECIPIENT_INTERFACE | USB_DIR_IN,
            0xa1,
            "a class request to an interface, device to host"
        );
        assert_eq!(USB_TYPE_CLASS | RECIPIENT_INTERFACE | USB_DIR_OUT, 0x21);
    }
}
