// Rust translation of src/hidapi/android/hid.cpp (and hid.h) from Simple
// DirectMedia Layer.
// Copyright (C) 2022 Valve Corporation
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Android backend: a wrapper implementing the "HID" API for Android,
//! which glues the hidapi API to Android's USB and BLE stack.
//!
//! The devices are those `HIDDeviceManager` (upstream's Java, translated in
//! [`crate::core::android::java::hid_device_manager`]) finds and passes on
//! through its natives: the HID interfaces of the USB devices (and the
//! vendor-specific ones of Xbox 360 and Xbox One controllers), through the
//! `UsbManager`, and Valve's Steam Controllers over Bluetooth LE, once
//! configured. The manager is made by the activity's `onCreate()`, which
//! registers it here (`HIDDeviceRegisterCallback()`); `hid_init()` starts
//! its USB part, and its Bluetooth part when `SDL_HINT_JOYSTICK_HIDAPI_STEAM`
//! is set (Bluetooth is only used for Steam Controllers; on API 31 and
//! later that asks for the `BLUETOOTH_CONNECT` permission first, which the
//! app's manifest must declare). Paths are the manager's identifiers of the
//! devices (`DEVICE_NAME/VID/PID/INTERFACE_INDEX` for USB,
//! `SteamController.ADDRESS` for Bluetooth).
//!
//! Each device keeps the input reports the manager passes on in a queue
//! (the 16 latest), which reads take from (a Bluetooth Steam Controller's
//! reports with their report ID put first); a feature report is asked for
//! and its answer waited for (2 s). Opening a USB device may need the
//! user's permission: the open fails ("Waiting for permission") and the
//! device is left out of the enumeration until the user has answered (and
//! is left out again if they refused, so that the application doesn't keep
//! asking).
//!
//! The manager's methods that upstream calls through JNI (`initialize()`,
//! `openDevice()`, ...) are Rust functions here, and the natives are
//! registered by `JNI_OnLoad()` (the DEX file's `HIDDeviceManager`,
//! `crate::core::android::classes`), which the manager's Rust calls
//! directly. The devices are shared (`Arc`, for upstream's intrusive
//! `hid_device_ref`), and their open and report state is locked (Note:
//! upstream reads and writes some of it from the manager's and the
//! caller's threads without a lock).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::{BusType, DeviceInfo};
use crate::core::android::env::{self, Env, Global, JniResult, Local, Obj};
use crate::core::android::java::hid_device_manager;
use crate::core::android::jni::*;
use crate::core::android::ndk::{self, LogPriority};
use crate::error::{Error, Result};
use crate::hints;

const TAG: &str = "hidapi";

/// `DEBUG`: the verbose logs (`LOGV()`, `LOGD()`), in a debug build as
/// upstream's (its Gradle and CMake debug builds define `DEBUG`; here a
/// build with debug assertions).
const DEBUG: bool = cfg!(debug_assertions);

/// Have error log always available. Translation of `LOGE()`.
fn loge(message: &str) {
    ndk::log_write(LogPriority::Error, TAG, message);
}

/// Translation of `LOGV()`.
fn logv(message: impl FnOnce() -> String) {
    if DEBUG {
        ndk::log_write(LogPriority::Verbose, TAG, &message());
    }
}

/// Translation of `LOGD()`.
fn logd(message: impl FnOnce() -> String) {
    if DEBUG {
        ndk::log_write(LogPriority::Debug, TAG, &message());
    }
}

const D0G_BLE2_PID: u16 = 0x1106;
const TRITON_BLE_PID: u16 = 0x1303;

/// `ETIMEDOUT`, `ECONNRESET`
const ETIMEDOUT: i32 = libc::ETIMEDOUT;
const ECONNRESET: i32 = libc::ECONNRESET;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// An open device, shared by the opens of its path. Translation of
/// `struct hid_device_`.
#[derive(Debug)]
pub(crate) struct DeviceHandle {
    /// `m_nId`
    id: i32,
    /// `m_nDeviceRefCount` (changed with [`DEVICES_REF_COUNT`] held)
    device_ref_count: AtomicI32,
}

/// The size of the report queue (`MAX_REPORT_QUEUE_SIZE`).
const MAX_REPORT_QUEUE_SIZE: usize = 16;

/// A queue of reports. Translation of `class hid_buffer_pool` (Note: its
/// free list of entries, kept for reuse, is the allocator's here).
#[derive(Debug, Default)]
struct HidBufferPool {
    queue: VecDeque<Vec<u8>>,
}

impl HidBufferPool {
    fn size(&self) -> usize {
        self.queue.len()
    }

    fn front(&self) -> &[u8] {
        self.queue.front().map_or(&[], Vec::as_slice)
    }

    fn pop_front(&mut self) {
        self.queue.pop_front();
    }

    fn emplace_back(&mut self, data: &[u8]) {
        self.queue.push_back(data.to_vec());
    }

    fn clear(&mut self) {
        while self.size() > 0 {
            self.pop_front();
        }
    }
}

/// The state behind `m_cvLock`: the open handshake and the report being
/// waited for.
#[derive(Debug, Default)]
struct CvState {
    /// `m_bIsWaitingForOpen`
    is_waiting_for_open: bool,
    /// `m_bWasOpenPending`
    was_open_pending: bool,
    /// `m_bOpenResult`
    open_result: bool,
    /// `m_bIsWaitingForReportResponse`
    is_waiting_for_report_response: bool,
    /// `m_nReportResponseError`
    report_response_error: i32,
    /// `m_reportResponse`
    report_response: Vec<u8>,
}

/// A device the manager connected. Translation of `class CHIDDevice`.
#[derive(Debug)]
struct ChidDevice {
    /// `m_nId`
    id: i32,
    /// `m_nReportID`
    report_id: i32,
    /// `m_pInfo`
    info: DeviceInfo,
    /// `m_pDevice`: the open device, while it is (Note that it isn't
    /// deleted with the device, as the app may still have a reference to
    /// it)
    device: Mutex<Option<Arc<DeviceHandle>>>,
    /// `m_bIsBLESteamController`
    is_ble_steam_controller: bool,

    /// `m_vecData`, with `m_dataLock`, which has to be held to access it
    data: Mutex<HidBufferPool>,

    /// For handling get_feature_report: `m_cvLock`, which has to be held
    /// to access any variables below (Note: here the open state's too),
    /// and `m_cv`
    cv: Mutex<CvState>,
    cond: Condvar,
}

impl ChidDevice {
    fn new(device_id: i32, info: DeviceInfo, report_id: i32) -> ChidDevice {
        // The Bluetooth Steam Controller needs special handling
        const VALVE_USB_VID: u16 = 0x28DE;
        let is_ble_steam_controller = info.vendor_id == VALVE_USB_VID
            && (info.product_id == D0G_BLE2_PID || info.product_id == TRITON_BLE_PID);
        ChidDevice {
            id: device_id,
            report_id,
            info,
            device: Mutex::new(None),
            is_ble_steam_controller,
            data: Mutex::new(HidBufferPool::default()),
            cv: Mutex::new(CvState::default()),
            cond: Condvar::new(),
        }
    }

    fn get_id(&self) -> i32 {
        self.id
    }

    fn get_device_info(&self) -> &DeviceInfo {
        &self.info
    }

    fn get_device(&self) -> Option<Arc<DeviceHandle>> {
        lock(&self.device).clone()
    }

    fn cv(&self) -> MutexGuard<'_, CvState> {
        lock(&self.cv)
    }

    fn exception_check<'a, T>(
        env: Env<'a>,
        method_name: &str,
        result: JniResult<'a, T>,
    ) -> Option<T> {
        exception_check(env, Some("CHIDDevice"), method_name, result)
    }

    /// Translation of `BOpen()`.
    fn b_open(&self) -> Result<()> {
        let Some(handler) = callback_handler() else {
            logv(|| "Device open without callback handler".into());
            // Note: upstream fails without an error message.
            return Err(Error::new("Device open without callback handler"));
        };
        let env = jni_env()?;
        let handler = local(env, &handler);

        if self.cv().is_waiting_for_open {
            return Err(Error::new("Waiting for permission"));
        }

        if !self.cv().open_result {
            let result = hid_device_manager::open_device(env, handler.obj(), self.id);
            let result = Self::exception_check(env, "BOpen", result).unwrap_or(false);
            let mut cv = self.cv();
            cv.open_result = result;

            if cv.is_waiting_for_open {
                logv(|| "Device open waiting for permission".into());
                cv.was_open_pending = true;
                return Err(Error::new("Waiting for permission"));
            }

            if !cv.open_result {
                logv(|| "Device open failed".into());
                return Err(Error::new("Device open failed"));
            }
        }

        let device = Arc::new(DeviceHandle {
            id: self.id,
            device_ref_count: AtomicI32::new(1),
        });
        logd(|| {
            format!(
                "Creating device {} ({:p}), refCount = 1\n",
                device.id, device
            )
        });
        *lock(&self.device) = Some(device);

        Ok(())
    }

    /// Translation of `SetOpenPending()`.
    fn set_open_pending(&self) {
        self.cv().is_waiting_for_open = true;
    }

    /// Translation of `BOpenPending()`.
    fn b_open_pending(&self) -> bool {
        self.cv().is_waiting_for_open
    }

    /// Translation of `SetWasOpenPending()`.
    fn set_was_open_pending(&self, state: bool) {
        self.cv().was_open_pending = state;
    }

    /// Translation of `BWasOpenPending()`.
    fn b_was_open_pending(&self) -> bool {
        self.cv().was_open_pending
    }

    /// Translation of `SetOpenResult()`.
    fn set_open_result(&self, result: bool) {
        let mut cv = self.cv();
        if cv.is_waiting_for_open {
            cv.open_result = result;
            cv.is_waiting_for_open = false;

            if cv.open_result {
                logv(|| "Device open succeeded".into());
            } else {
                logv(|| "Device open failed".into());
            }
        }
    }

    /// Translation of `BOpenResult()`.
    fn b_open_result(&self) -> bool {
        self.cv().open_result
    }

    /// Translation of `ProcessInput()`.
    fn process_input(&self, buf: &[u8]) {
        let mut data = lock(&self.data);

        if data.size() >= MAX_REPORT_QUEUE_SIZE {
            data.pop_front();
        }
        data.emplace_back(buf);
    }

    /// Translation of `GetInput()`: the oldest report, 0 bytes when there's
    /// none.
    fn get_input(&self, data: &mut [u8]) -> usize {
        let mut vec_data = lock(&self.data);

        if vec_data.size() == 0 {
            // (LOGV("hid_read_timeout no data available"), commented out)
            return 0;
        }

        let buffer = vec_data.front();
        let data_len = get_input_report(
            buffer,
            data,
            self.is_ble_steam_controller.then_some(self.report_id as u8),
        );
        vec_data.pop_front();

        // (LOGV() of the bytes read, commented out)

        data_len
    }

    /// Translation of `WriteReport()`: the bytes written, -1 when it
    /// failed.
    fn write_report(&self, data: &[u8], feature: bool) -> i32 {
        let Some(handler) = callback_handler() else {
            logv(|| "WriteReport without callback handler".into());
            return -1;
        };
        let Ok(env) = jni_env() else {
            return -1;
        };
        let handler = local(env, &handler);

        let ret = hid_device_manager::write_report(env, handler.obj(), self.id, data, feature);
        // (0 when it threw, as JNI's CallIntMethod())
        Self::exception_check(env, "WriteReport", ret).unwrap_or(0)
    }

    /// Translation of `ProcessReportResponse()`.
    fn process_report_response(&self, buf: &[u8]) {
        let mut cv = self.cv();
        if cv.is_waiting_for_report_response {
            cv.report_response = buf.to_vec();

            cv.is_waiting_for_report_response = false;
            cv.report_response_error = 0;
            self.cond.notify_one();
        }
    }

    /// Translation of `ReadReport()`: ask the manager for a feature (or
    /// input) report and wait for its answer; the bytes read (including
    /// the report number), or a negative value when it failed.
    fn read_report(&self, data: &mut [u8], feature: bool) -> i32 {
        let Some(handler) = callback_handler() else {
            logv(|| "ReadReport without callback handler".into());
            return -1;
        };
        let Ok(env) = jni_env() else {
            return -1;
        };
        let handler = local(env, &handler);

        {
            let mut cv = self.cv();
            if cv.is_waiting_for_report_response {
                logv(|| "Get feature report already ongoing... bail".into());
                return -1; // Read already ongoing, we currently do not serialize, TODO
            }
            cv.is_waiting_for_report_response = true;
        }

        let ret = hid_device_manager::read_report(env, handler.obj(), self.id, data, feature);
        // (false when it threw, as JNI's CallBooleanMethod())
        let ret = if Self::exception_check(env, "ReadReport", ret).unwrap_or(false) {
            0
        } else {
            -1
        };
        if ret < 0 {
            logv(|| "ReadReport failed".into());
            self.cv().is_waiting_for_report_response = false;
            return -1;
        }

        let mut cv = self.cv();
        if cv.is_waiting_for_report_response {
            logv(|| "=== Going to sleep".into());
            // Wait in CV until we are no longer waiting for a feature report.
            const FEATURE_REPORT_TIMEOUT_SECONDS: u64 = 2;
            cv = self
                .cond
                .wait_timeout_while(
                    cv,
                    Duration::from_secs(FEATURE_REPORT_TIMEOUT_SECONDS),
                    |cv| cv.is_waiting_for_report_response,
                )
                .unwrap_or_else(|e| e.into_inner())
                .0;

            // We are back
            if cv.is_waiting_for_report_response {
                cv.report_response_error = -ETIMEDOUT;
                cv.is_waiting_for_report_response = false;
            }
            let error = cv.report_response_error;
            logv(|| format!("=== Got feature report err={error}"));
            if cv.report_response_error != 0 {
                return cv.report_response_error;
            }
        }

        let mut bytes_to_copy = 0;

        if feature && !cv.report_response.is_empty() {
            bytes_to_copy = copy_feature_report(&cv.report_response, data);
            cv.report_response.clear();
        }

        logv(|| format!("=== Got {bytes_to_copy} bytes"));

        bytes_to_copy as i32
    }

    /// Translation of `Close()`: the manager's device closed, its reports
    /// dropped and a pending feature report's wait ended, and with
    /// `delete_device`, the open device forgotten.
    fn close(&self, delete_device: bool) {
        if let Some(handler) = callback_handler() {
            let (is_waiting_for_open, open_result) = {
                let cv = self.cv();
                (cv.is_waiting_for_open, cv.open_result)
            };
            if !is_waiting_for_open && open_result {
                if let Ok(env) = jni_env() {
                    let handler = local(env, &handler);
                    let result = hid_device_manager::close_device(env, handler.obj(), self.id);
                    Self::exception_check(env, "Close", result);
                }
            }
        }

        let mut data = lock(&self.data);
        data.clear();

        // Clean and release pending feature report reads
        let mut cv = self.cv();
        cv.report_response.clear();
        cv.is_waiting_for_report_response = false;
        cv.report_response_error = -ECONNRESET;
        self.cond.notify_all();

        cv.open_result = false;

        if delete_device {
            *lock(&self.device) = None;
        }
    }
}

/// [`ChidDevice::get_input`]'s copy of the report `buffer` into `data`:
/// the bytes copied, after the report ID when one is given (a Bluetooth
/// Steam Controller's).
fn get_input_report(buffer: &[u8], data: &mut [u8], report_id: Option<u8>) -> usize {
    let mut data_len = buffer.len().min(data.len());
    match report_id {
        Some(report_id) => {
            // FIXME (upstream): the report ID is put before `length` bytes
            // of the report, one more than the buffer has room for when
            // the report is as long as it (or longer); here the report is
            // cut to fit.
            if data.is_empty() {
                return 0;
            }
            data[0] = report_id;
            data_len = data_len.min(data.len() - 1);
            data[1..1 + data_len].copy_from_slice(&buffer[..data_len]);
            data_len += 1;
        }
        None => data[..data_len].copy_from_slice(&buffer[..data_len]),
    }
    data_len
}

/// [`ChidDevice::read_report`]'s copy of a feature report's `response` into
/// `data`, whose first byte is the report number asked for: the bytes the
/// caller gets, the report number included.
fn copy_feature_report(response: &[u8], data: &mut [u8]) -> usize {
    // Make sure we preserve the report value if it isn't already in the report.
    // (Note (upstream): an empty buffer's first byte is read; here it has
    // none.)
    let has_report_already = data.first() == response.first();

    // Make sure we only copy as much as will fit, deducting one byte for the report if we need to leave it intact.
    let offset = if has_report_already { 0 } else { 1 };
    let safe_data_len = data.len().saturating_sub(offset);
    let mut bytes_to_copy = response.len().min(safe_data_len);

    data[offset..offset + bytes_to_copy].copy_from_slice(&response[..bytes_to_copy]);

    if !has_report_already {
        // Add the report byte back on to return the real length.
        bytes_to_copy += 1;
    }
    bytes_to_copy
}

/// The manager that registered itself (`g_HIDDeviceManagerCallbackHandler`,
/// shared, as its lock isn't held while it's used) and its class
/// (`g_HIDDeviceManagerCallbackClass`). (The method IDs upstream finds
/// there, `g_midHIDDeviceManagerInitialize` and the others, aren't needed:
/// those methods are Rust functions here.)
struct CallbackHandler {
    handler: Arc<Global>,
    _class: Option<Global>,
}

static CALLBACK_HANDLER: Mutex<Option<CallbackHandler>> = Mutex::new(None);
/// `g_initialized`
static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// `g_HIDDeviceManagerCallbackHandler`, when there's one.
fn callback_handler() -> Option<Arc<Global>> {
    lock(&CALLBACK_HANDLER)
        .as_ref()
        .map(|callback| callback.handler.clone())
}

/// A local reference to a global one's object.
fn local<'a>(env: Env<'a>, global: &Global) -> Local<'a> {
    env.new_local_ref(global.obj())
        .expect("a global reference's object")
}

/// `(JNIEnv *)SDL_GetAndroidJNIEnv()`.
fn jni_env() -> Result<Env<'static>> {
    env::get_env().ok_or_else(|| Error::new("No JNI environment"))
}

/// The exception a call threw, if any, logged and cleared: the call's
/// result, else `None`. Translation of `ExceptionCheck()`.
fn exception_check<'a, T>(
    env: Env<'a>,
    class_name: Option<&str>,
    method_name: &str,
    result: JniResult<'a, T>,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(thrown) => {
            // Get our exception
            // Clear the exception so we can call JNI again
            let except = thrown.catch();

            // Get our exception message
            let message = env
                .call_method(except.obj(), c"getMessage", c"()Ljava/lang/String;", &[])
                .map(env::Value::object);
            let message = match message {
                Ok(Some(message)) => env.get_string(message.obj()).unwrap_or_else(|thrown| {
                    thrown.clear();
                    String::new()
                }),
                Ok(None) => "null".into(),
                Err(thrown) => {
                    thrown.clear();
                    String::new()
                }
            };

            // ...and log it.
            loge(&format!(
                "{}{}{} threw an exception: {}",
                class_name.unwrap_or(""),
                if class_name.is_some() { "::" } else { "" },
                method_name,
                message
            ));

            // Cleanup
            None
        }
    }
}

/// `g_Devices` (in the order the devices were connected), with
/// `g_DevicesMutex`.
static DEVICES: Mutex<Vec<Arc<ChidDevice>>> = Mutex::new(Vec::new());
/// `g_DevicesRefCountMutex`
static DEVICES_REF_COUNT: Mutex<()> = Mutex::new(());

/// Translation of `FindDevice()`.
fn find_device(device_id: i32) -> Option<Arc<ChidDevice>> {
    lock(&DEVICES)
        .iter()
        .find(|device| device.get_id() == device_id)
        .cloned()
}

// The natives of HIDDeviceManager (`HIDDeviceManager_tab`), and their Rust
// bodies, which the manager's Rust calls.

/// Translation of `HIDDeviceRegisterCallback()`: the manager that calls
/// back.
pub(crate) fn hid_device_register_callback(env: Env<'_>, thiz: Obj<'_>) {
    logv(|| "HIDDeviceRegisterCallback()".into());

    let mut callback = lock(&CALLBACK_HANDLER);
    if callback.is_some() {
        *callback = None;
    }

    let Some(handler) = Global::new(env, thiz) else {
        return;
    };
    let class = env
        .get_object_class(thiz)
        .and_then(|class| Global::new(env, class.obj()));
    *callback = Some(CallbackHandler {
        handler: Arc::new(handler),
        _class: class,
    });
}

pub(crate) extern "system" fn native_hid_device_register_callback(env: *mut JNIEnv, thiz: jobject) {
    // SAFETY: a native method's environment and object.
    let (env, thiz) = unsafe { (Env::from_raw(env), Obj::from_raw(thiz)) };
    hid_device_register_callback(env, thiz);
}

/// Translation of `HIDDeviceReleaseCallback()`.
pub(crate) fn hid_device_release_callback(env: Env<'_>, thiz: Obj<'_>) {
    logv(|| "HIDDeviceReleaseCallback".into());
    let mut callback = lock(&CALLBACK_HANDLER);
    if callback
        .as_ref()
        .is_some_and(|callback| env.is_same_object(thiz, callback.handler.obj()))
    {
        *callback = None;
        INITIALIZED.store(false, Ordering::SeqCst);
    }
}

pub(crate) extern "system" fn native_hid_device_release_callback(env: *mut JNIEnv, thiz: jobject) {
    // SAFETY: a native method's environment and object.
    let (env, thiz) = unsafe { (Env::from_raw(env), Obj::from_raw(thiz)) };
    hid_device_release_callback(env, thiz);
}

/// What `HIDDeviceConnected()` is told of a device.
#[derive(Debug)]
pub(crate) struct ConnectedDevice<'s> {
    pub(crate) device_id: i32,
    pub(crate) identifier: &'s str,
    pub(crate) vendor_id: i32,
    pub(crate) product_id: i32,
    pub(crate) serial_number: &'s str,
    pub(crate) release_number: i32,
    pub(crate) manufacturer: &'s str,
    pub(crate) product: &'s str,
    pub(crate) interface: i32,
    pub(crate) interface_class: i32,
    pub(crate) interface_subclass: i32,
    pub(crate) interface_protocol: i32,
    pub(crate) bluetooth: bool,
    pub(crate) report_id: i32,
}

/// Translation of `HIDDeviceConnected()`: a device added, last.
pub(crate) fn hid_device_connected(device: &ConnectedDevice<'_>) {
    logv(|| {
        format!(
            "HIDDeviceConnected() id={} VID/PID = {:04x}/{:04x}, interface {}\n",
            device.device_id, device.vendor_id, device.product_id, device.interface
        )
    });

    // (the integers are truncated to the fields' types, as C's assignments)
    let info = DeviceInfo {
        path: Some(device.identifier.to_owned()),
        vendor_id: device.vendor_id as u16,
        product_id: device.product_id as u16,
        serial_number: Some(device.serial_number.to_owned()),
        release_number: device.release_number as u16,
        manufacturer_string: Some(device.manufacturer.to_owned()),
        product_string: Some(device.product.to_owned()),
        interface_number: device.interface,
        interface_class: device.interface_class,
        interface_subclass: device.interface_subclass,
        interface_protocol: device.interface_protocol,
        bus_type: if device.bluetooth {
            BusType::Bluetooth
        } else {
            BusType::Usb
        },
        // (the rest is zero, from a memset)
        ..DeviceInfo::default()
    };

    let device = Arc::new(ChidDevice::new(device.device_id, info, device.report_id));

    lock(&DEVICES).push(device);
}

/// A Java string's characters, as upstream copies them (`""` for null,
/// which upstream doesn't expect).
fn java_string(env: Env<'_>, string: Obj<'_>, utf8: bool) -> String {
    if string.is_null() {
        return String::new();
    }
    let result = if utf8 {
        // CreateStringFromJString(): its Modified UTF-8
        env.get_string_utf_chars(string)
            .map(|s| s.to_string_lossy().into_owned())
    } else {
        // CreateWStringFromJString(): its UTF-16 (Note: decoded here;
        // upstream copies each unit to a wchar_t)
        env.get_string(string)
    };
    result.unwrap_or_else(|thrown| {
        thrown.clear();
        String::new()
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) extern "system" fn native_hid_device_connected(
    env: *mut JNIEnv,
    _thiz: jobject,
    device_id: jint,
    identifier: jstring,
    vendor_id: jint,
    product_id: jint,
    serial_number: jstring,
    release_number: jint,
    manufacturer: jstring,
    product: jstring,
    interface: jint,
    interface_class: jint,
    interface_subclass: jint,
    interface_protocol: jint,
    bluetooth: jboolean,
    report_id: jint,
) {
    // SAFETY: a native method's environment and arguments.
    let (env, identifier, serial_number, manufacturer, product) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(identifier),
            Obj::from_raw(serial_number),
            Obj::from_raw(manufacturer),
            Obj::from_raw(product),
        )
    };
    let identifier = java_string(env, identifier, true);
    let serial_number = java_string(env, serial_number, false);
    let manufacturer = java_string(env, manufacturer, false);
    let product = java_string(env, product, false);
    hid_device_connected(&ConnectedDevice {
        device_id,
        identifier: &identifier,
        vendor_id,
        product_id,
        serial_number: &serial_number,
        release_number,
        manufacturer: &manufacturer,
        product: &product,
        interface,
        interface_class,
        interface_subclass,
        interface_protocol,
        bluetooth: bluetooth != JNI_FALSE,
        report_id,
    });
}

/// Translation of `HIDDeviceOpenPending()`.
pub(crate) fn hid_device_open_pending(device_id: i32) {
    logv(|| format!("HIDDeviceOpenPending() id={device_id}\n"));
    if let Some(device) = find_device(device_id) {
        device.set_open_pending();
    }
}

pub(crate) extern "system" fn native_hid_device_open_pending(
    _env: *mut JNIEnv,
    _thiz: jobject,
    device_id: jint,
) {
    hid_device_open_pending(device_id);
}

/// Translation of `HIDDeviceOpenResult()`.
pub(crate) fn hid_device_open_result(device_id: i32, opened: bool) {
    logv(|| format!("HIDDeviceOpenResult() id={device_id}, result={opened}\n"));
    if let Some(device) = find_device(device_id) {
        device.set_open_result(opened);
    }
}

pub(crate) extern "system" fn native_hid_device_open_result(
    _env: *mut JNIEnv,
    _thiz: jobject,
    device_id: jint,
    opened: jboolean,
) {
    hid_device_open_result(device_id, opened != JNI_FALSE);
}

/// Translation of `HIDDeviceDisconnected()`: the device removed (and its
/// open closed, the open device kept for its holders).
pub(crate) fn hid_device_disconnected(device_id: i32) {
    logv(|| format!("HIDDeviceDisconnected() id={device_id}\n"));
    let device = {
        let mut devices = lock(&DEVICES);
        // (every device of the ID, as upstream's walk)
        let mut found = None;
        devices.retain(|curr| {
            if curr.get_id() == device_id {
                found = Some(curr.clone());
                false
            } else {
                true
            }
        });
        found
    };
    if let Some(device) = device {
        device.close(false);
    }
}

pub(crate) extern "system" fn native_hid_device_disconnected(
    _env: *mut JNIEnv,
    _thiz: jobject,
    device_id: jint,
) {
    hid_device_disconnected(device_id);
}

/// Translation of `HIDDeviceInputReport()`: an input report, queued.
pub(crate) fn hid_device_input_report(device_id: i32, value: &[u8]) {
    // (LOGV("HIDDeviceInput() id=%d len=%u\n"), commented out)
    if let Some(device) = find_device(device_id) {
        device.process_input(value);
    }
}

/// A Java `byte[]`'s bytes (none for null, which upstream doesn't expect).
fn java_bytes(env: Env<'_>, value: Obj<'_>) -> Vec<u8> {
    if value.is_null() {
        return Vec::new();
    }
    env.get_byte_array(value).unwrap_or_else(|thrown| {
        thrown.clear();
        Vec::new()
    })
}

pub(crate) extern "system" fn native_hid_device_input_report(
    env: *mut JNIEnv,
    _thiz: jobject,
    device_id: jint,
    value: jbyteArray,
) {
    // SAFETY: a native method's environment and arguments.
    let (env, value) = unsafe { (Env::from_raw(env), Obj::from_raw(value)) };
    hid_device_input_report(device_id, &java_bytes(env, value));
}

/// Translation of `HIDDeviceReportResponse()`: the answer to a feature
/// (or input) report asked for.
pub(crate) fn hid_device_report_response(device_id: i32, value: &[u8]) {
    logv(|| {
        format!(
            "HIDDeviceReportResponse() id={device_id} len={}\n",
            value.len()
        )
    });
    if let Some(device) = find_device(device_id) {
        device.process_report_response(value);
    }
}

pub(crate) extern "system" fn native_hid_device_report_response(
    env: *mut JNIEnv,
    _thiz: jobject,
    device_id: jint,
    value: jbyteArray,
) {
    // SAFETY: a native method's environment and arguments.
    let (env, value) = unsafe { (Env::from_raw(env), Obj::from_raw(value)) };
    hid_device_report_response(device_id, &java_bytes(env, value));
}

// The hid_* API

/// Translation of `RequestBluetoothPermissionCallback()`.
fn request_bluetooth_permission_callback(_permission: &str, granted: bool) {
    crate::log!(
        "Bluetooth permission {}",
        if granted { "granted" } else { "denied" }
    );

    if granted {
        if let Some(handler) = callback_handler() {
            if let Some(env) = env::get_env() {
                let handler = local(env, &handler);
                let result = hid_device_manager::initialize(env, handler.obj(), false, true);
                // Note: the exception is logged and cleared here (upstream
                // leaves it pending: for hid_init()'s check when the
                // permission was granted already, else thrown in Java's
                // main thread).
                exception_check(env, None, "RequestBluetoothPermissionCallback", result);
            }
        }
    }
}

/// Translation of `hid_init()`: the manager's USB part started, and its
/// Bluetooth part for Steam Controllers when the hint asks for them (once:
/// later calls do nothing, whatever the result).
pub(super) fn hid_init() -> Result<()> {
    if !INITIALIZED.load(Ordering::SeqCst) {
        if let Some(handler) = callback_handler() {
            // HIDAPI doesn't work well with Android < 4.3
            if crate::android::sdk_version() >= 18 {
                let env = jni_env()?;
                let handler = local(env, &handler);

                let result = hid_device_manager::initialize(env, handler.obj(), true, false);
                // (Note: each call's exception is checked, upstream's once
                // after them: JNI can't be called with one pending)
                exception_check(env, None, "hid_init", result);

                // Bluetooth is currently only used for Steam Controllers, so check that hint
                // before initializing Bluetooth, which will prompt the user for permission.
                if hints::get_bool(hints::JOYSTICK_HIDAPI_STEAM, false) {
                    if crate::android::sdk_version() < 31 {
                        let result =
                            hid_device_manager::initialize(env, handler.obj(), false, true);
                        exception_check(env, None, "hid_init", result);
                    } else {
                        crate::log!("Requesting Bluetooth permission");
                        let _ = crate::core::android::request_android_permission(
                            "android.permission.BLUETOOTH_CONNECT",
                            Box::new(request_bluetooth_permission_callback),
                        );
                    }
                }
            }
            INITIALIZED.store(true, Ordering::SeqCst); // Regardless of result, so it's only called once
        }
    }
    Ok(())
}

/// Translation of `hid_enumerate()`: the devices connected, but those being
/// opened, in the reverse of their order (each one is put first).
pub(super) fn hid_enumerate(vendor_id: u16, product_id: u16) -> Vec<DeviceInfo> {
    let mut root = Vec::new();

    let devices = lock(&DEVICES);
    for device in devices.iter() {
        // Don't enumerate devices that are currently being opened, we'll re-enumerate them when we're done
        // Make sure we skip them at least once, so they get removed and then re-added to the caller's device list
        if device.b_was_open_pending() {
            // Don't enumerate devices that failed to open, otherwise the application might try to keep prompting for access
            if !device.b_open_pending() && device.b_open_result() {
                device.set_was_open_pending(false);
            }
            continue;
        }

        let info = device.get_device_info();

        /* See if there are any devices we should skip in enumeration */
        if super::should_ignore_device(
            BusType::Unknown,
            info.vendor_id,
            info.product_id,
            0,
            0,
            false,
            false,
        ) {
            continue;
        }

        if (vendor_id == 0x0 || info.vendor_id == vendor_id)
            && (product_id == 0x0 || info.product_id == product_id)
        {
            root.insert(0, info.clone());
        }
    }
    root
}

/// Translation of `hid_open()`: not implemented.
pub(super) fn open(
    _vendor_id: u16,
    _product_id: u16,
    _serial_number: Option<&str>,
) -> Result<Device> {
    // TODO: Implement
    // (Note: upstream returns NULL without an error message.)
    Err(Error::unsupported())
}

/// Translation of `hid_open_path()`: the device of the path opened (the
/// manager asked to open it, unless it is open already, whose open is
/// shared).
pub(super) fn open_path(path: &str) -> Result<Device> {
    logv(|| format!("hid_open_path( {path} )"));

    let device = {
        let _r = lock(&DEVICES_REF_COUNT);
        let devices = lock(&DEVICES);
        let mut found = None;
        for curr in devices.iter() {
            if curr.get_device_info().path.as_deref() == Some(path) {
                if let Some(value) = curr.get_device() {
                    let count = value.device_ref_count.fetch_add(1, Ordering::SeqCst) + 1;
                    logd(|| {
                        format!(
                            "Incrementing device {} ({:p}), refCount = {count}\n",
                            value.id, value
                        )
                    });
                    return Ok(Device { handle: value });
                }

                // Hold a shared pointer to the controller for the duration
                found = Some(curr.clone());
                break;
            }
        }
        found
    };
    let Some(device) = device else {
        return Err(Error::new(format!("Couldn't find device with path {path}")));
    };
    device.b_open()?;
    device
        .get_device()
        .map(|handle| Device { handle })
        .ok_or_else(|| Error::new("Device open failed"))
}

/// An open device. Translation of `hid_device *`.
#[derive(Debug)]
pub(super) struct Device {
    handle: Arc<DeviceHandle>,
}

/// "Controller was disconnected" (Note: upstream returns -1 without an
/// error message).
fn disconnected() -> Error {
    Error::new("Controller was disconnected")
}

/// A report's result: the bytes, or the failure (Note: upstream returns -1,
/// or `-ETIMEDOUT` or `-ECONNRESET` for a feature report it waited for,
/// without an error message).
fn report_result(result: i32, what: &str) -> Result<usize> {
    match result {
        n if n >= 0 => Ok(n as usize),
        n if n == -ETIMEDOUT => Err(Error::new(format!("Timed out waiting for the {what}"))),
        n if n == -ECONNRESET => Err(Error::new(format!(
            "The device was closed while waiting for the {what}"
        ))),
        _ => Err(Error::new(format!("Couldn't get the {what}"))),
    }
}

/// `getms()`: milliseconds (Note: of a monotonic clock here, upstream's
/// are the time of day's).
fn getms(start: Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

/// Translation of `delayms()`.
fn delayms(ms: u32) {
    std::thread::sleep(Duration::from_millis(u64::from(ms)));
}

impl Device {
    fn find(&self) -> Option<Arc<ChidDevice>> {
        find_device(self.handle.id)
    }

    /// Translation of `hid_write()`.
    pub(super) fn write(&self, data: &[u8]) -> Result<usize> {
        // (LOGV("hid_write id=%d length=%zu"), commented out)
        match self.find() {
            Some(device) => match device.write_report(data, false) {
                n if n >= 0 => Ok(n as usize),
                // Note: upstream returns -1 without an error message.
                _ => Err(Error::new("Couldn't write the output report")),
            },
            None => Err(disconnected()), // Controller was disconnected
        }
    }

    /// Translation of `hid_read_timeout()`: a report, waited for while
    /// `milliseconds` is positive (Note (upstream): a negative timeout
    /// doesn't wait).
    pub(super) fn read_timeout(&self, data: &mut [u8], milliseconds: i32) -> Result<usize> {
        // (LOGV("hid_read_timeout id=%d length=%u timeout=%d"), commented out)
        match self.find() {
            Some(device) => {
                let mut result = device.get_input(data);
                if result == 0 && milliseconds > 0 {
                    let start = Instant::now();
                    loop {
                        delayms(1);
                        result = device.get_input(data);
                        if !(result == 0 && getms(start) < milliseconds as u64) {
                            break;
                        }
                    }
                }
                Ok(result)
            }
            None => {
                logv(|| "controller was disconnected".into());
                Err(disconnected()) // Controller was disconnected
            }
        }
    }

    /// Translation of `hid_read()`.
    pub(super) fn read(&self, data: &mut [u8]) -> Result<usize> {
        // TODO: Implement blocking
        // (LOGV("hid_read id=%d length=%zu"), commented out)
        self.read_timeout(data, 0)
    }

    /// Translation of `hid_set_nonblocking()`: not implemented.
    pub(super) fn set_nonblocking(&self, _nonblock: bool) -> Result<()> {
        // TODO: Implement?
        // (Note: upstream returns -1 without an error message.)
        Err(Error::unsupported())
    }

    /// Translation of `hid_send_feature_report()`.
    pub(super) fn send_feature_report(&self, data: &[u8]) -> Result<usize> {
        logv(|| {
            format!(
                "hid_send_feature_report id={} length={}",
                self.handle.id,
                data.len()
            )
        });
        match self.find() {
            Some(device) => match device.write_report(data, true) {
                n if n >= 0 => Ok(n as usize),
                // Note: upstream returns -1 without an error message.
                _ => Err(Error::new("Couldn't send the feature report")),
            },
            None => Err(disconnected()), // Controller was disconnected
        }
    }

    /// Translation of `hid_get_feature_report()`. Synchronous operation.
    /// Will block until completed.
    pub(super) fn get_feature_report(&self, data: &mut [u8]) -> Result<usize> {
        logv(|| {
            format!(
                "hid_get_feature_report id={} length={}",
                self.handle.id,
                data.len()
            )
        });
        match self.find() {
            Some(device) => report_result(device.read_report(data, true), "feature report"),
            None => Err(disconnected()), // Controller was disconnected
        }
    }

    /// Translation of `hid_get_input_report()`. Synchronous operation.
    /// Will block until completed. (Note (upstream): the report isn't
    /// copied: 0 bytes once it came.)
    pub(super) fn get_input_report(&self, data: &mut [u8]) -> Result<usize> {
        logv(|| {
            format!(
                "hid_get_input_report id={} length={}",
                self.handle.id,
                data.len()
            )
        });
        match self.find() {
            Some(device) => report_result(device.read_report(data, false), "input report"),
            None => Err(disconnected()), // Controller was disconnected
        }
    }

    /// Translation of `hid_get_manufacturer_string()`.
    pub(super) fn get_manufacturer_string(&self) -> Result<String> {
        match self.find() {
            Some(device) => Ok(device
                .get_device_info()
                .manufacturer_string
                .clone()
                .unwrap_or_default()),
            None => Err(disconnected()),
        }
    }

    /// Translation of `hid_get_product_string()`.
    pub(super) fn get_product_string(&self) -> Result<String> {
        match self.find() {
            Some(device) => Ok(device
                .get_device_info()
                .product_string
                .clone()
                .unwrap_or_default()),
            None => Err(disconnected()),
        }
    }

    /// Translation of `hid_get_serial_number_string()`.
    pub(super) fn get_serial_number_string(&self) -> Result<String> {
        match self.find() {
            Some(device) => Ok(device
                .get_device_info()
                .serial_number
                .clone()
                .unwrap_or_default()),
            None => Err(disconnected()),
        }
    }

    /// Translation of `hid_get_indexed_string()`: not implemented.
    pub(super) fn get_indexed_string(&self, _string_index: i32) -> Result<String> {
        Err(Error::unsupported())
    }

    /// Translation of `hid_get_device_info()`.
    pub(super) fn get_device_info(&self) -> Result<DeviceInfo> {
        match self.find() {
            Some(device) => Ok(device.get_device_info().clone()),
            None => Err(disconnected()),
        }
    }

    /// Translation of `hid_get_report_descriptor()`: not implemented.
    pub(super) fn get_report_descriptor(&self, _buf: &mut [u8]) -> Result<usize> {
        // Not implemented
        Err(Error::unsupported())
    }
}

impl Drop for Device {
    /// Translation of `hid_close()`: the last open of the device closes it
    /// (Note (upstream): the device of its ID, which a device connected
    /// again under the same ID has).
    fn drop(&mut self) {
        let device = &self.handle;
        logv(|| format!("hid_close id={}", device.id));
        let _r = lock(&DEVICES_REF_COUNT);
        logd(|| {
            format!(
                "Decrementing device {} ({:p}), refCount = {}\n",
                device.id,
                Arc::as_ptr(device),
                device.device_ref_count.load(Ordering::SeqCst) - 1
            )
        });
        if device.device_ref_count.fetch_sub(1, Ordering::SeqCst) - 1 == 0 {
            if let Some(chid) = find_device(device.id) {
                chid.close(true);
            }
            // (else the device is deleted: its last reference is this)
            logd(|| format!("Deleted device {:p}\n", Arc::as_ptr(device)));
        }
    }
}

/// Translation of `hid_exit()`.
pub(super) fn hid_exit() -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_queue_keeps_the_latest() {
        let device = ChidDevice::new(7, DeviceInfo::default(), 0);
        for i in 0..20u8 {
            device.process_input(&[i, i + 1]);
        }
        let mut data = [0u8; 8];
        for i in 4..20u8 {
            assert_eq!(device.get_input(&mut data), 2);
            assert_eq!(&data[..2], &[i, i + 1]);
        }
        assert_eq!(device.get_input(&mut data), 0);
    }

    #[test]
    fn steam_controller_reports_get_their_id() {
        let info = DeviceInfo {
            vendor_id: 0x28de,
            product_id: D0G_BLE2_PID,
            ..DeviceInfo::default()
        };
        let device = ChidDevice::new(1, info, 3);
        assert!(device.is_ble_steam_controller);
        device.process_input(&[1, 2, 3]);
        let mut data = [0u8; 8];
        assert_eq!(device.get_input(&mut data), 4);
        assert_eq!(&data[..4], &[3, 1, 2, 3]);

        // (cut to fit)
        device.process_input(&[1, 2, 3]);
        let mut small = [0u8; 3];
        assert_eq!(device.get_input(&mut small), 3);
        assert_eq!(small, [3, 1, 2]);

        // The other devices' reports are as they come, cut to fit.
        let other = ChidDevice::new(2, DeviceInfo::default(), 3);
        assert!(!other.is_ble_steam_controller);
        other.process_input(&[9, 8, 7]);
        assert_eq!(other.get_input(&mut small[..2]), 2);
        assert_eq!(&small[..2], &[9, 8]);
    }

    #[test]
    fn feature_reports_keep_their_number() {
        // The answer starts with the report number asked for.
        let mut data = [5u8, 0, 0, 0];
        assert_eq!(copy_feature_report(&[5, 1, 2], &mut data), 3);
        assert_eq!(data, [5, 1, 2, 0]);

        // It doesn't: the number is kept, the answer after it.
        let mut data = [5u8, 0, 0, 0];
        assert_eq!(copy_feature_report(&[1, 2], &mut data), 3);
        assert_eq!(data, [5, 1, 2, 0]);

        // Cut to fit.
        let mut data = [5u8, 0, 0];
        assert_eq!(copy_feature_report(&[1, 2, 3, 4], &mut data), 3);
        assert_eq!(data, [5, 1, 2]);
        let mut data = [5u8, 0];
        assert_eq!(copy_feature_report(&[5, 1, 2, 3], &mut data), 2);
        assert_eq!(data, [5, 1]);
    }

    #[test]
    fn report_responses_are_only_taken_when_waited_for() {
        let device = ChidDevice::new(3, DeviceInfo::default(), 0);
        device.process_report_response(&[1, 2]);
        assert!(device.cv().report_response.is_empty());
        device.cv().is_waiting_for_report_response = true;
        device.process_report_response(&[1, 2]);
        let cv = device.cv();
        assert!(!cv.is_waiting_for_report_response);
        assert_eq!(cv.report_response, [1, 2]);
        assert_eq!(cv.report_response_error, 0);
    }

    #[test]
    fn open_handshake() {
        let device = ChidDevice::new(4, DeviceInfo::default(), 0);
        // A result without a pending open is ignored.
        device.set_open_result(true);
        assert!(!device.b_open_result());
        device.set_open_pending();
        assert!(device.b_open_pending());
        device.set_open_result(true);
        assert!(!device.b_open_pending());
        assert!(device.b_open_result());
        device.close(false);
        assert!(!device.b_open_result());
        assert_eq!(device.cv().report_response_error, -ECONNRESET);
    }
}
