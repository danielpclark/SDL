// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// HIDDeviceBLESteamController.java from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `class HIDDeviceBLESteamController extends BluetoothGattCallback
//! implements HIDDevice`: a Steam Controller (the original one, "D0G", or
//! the second generation, "Triton") over Bluetooth LE. Its GATT client
//! (`connectGatt()`, LE transport) discovers Valve's service once
//! connected, finds the input characteristic (the product and its report
//! ID tell which) and the Triton's output report characteristics (their
//! UUIDs' report IDs), enables the input's notifications (a GATT
//! operation, with a retry), enters "Valve mode" (a write of the report
//! characteristic; a Triton has none to do) and then registers the device
//! with hid.cpp (`HIDDeviceConnected()`). The input characteristic's
//! notifications are its input reports; feature reports are the report
//! characteristic's reads and writes, output reports its writes (a
//! Triton's to the report's own characteristic). The GATT operations are
//! queued, one at a time, each run on the main thread and finished by its
//! callback.
//!
//! The class is the DEX file's (`super::super::classes`), a subclass of
//! the framework's abstract `BluetoothGattCallback` whose overrides are
//! native; its constructor is `BluetoothGattCallback`'s (Note: upstream's
//! takes the manager and the device; here the rest of it runs in Rust, on
//! the new object, [`HidDeviceBleSteamController::new`]). Its fields are
//! Rust; the callbacks find the device of their object among those
//! made (a device shut down gets none). UUIDs are kept as their strings
//! (`UUID.toString()`'s, lowercase), which compare as the UUIDs do.
//! Upstream's anonymous `Runnable`s are `SDLNativeRunnable`s
//! (`super::anonymous`).

use std::collections::{HashMap, VecDeque};
use std::ffi::CStr;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use super::super::classes::HID_DEVICE_BLE_STEAM_CONTROLLER;
use super::super::env::{self, Arg, Env, Global, JniResult, Local, Obj};
use super::super::jni::*;
use super::super::ndk::LogPriority;
use super::hid_device::HidDevice;
use super::hid_device_manager::{self as manager, log, HidDeviceManager};
use crate::hidapi::android as hid;

const D0G_BLE2_PID: jint = 0x1106;
const TRITON_BLE_PID: jint = 0x1303;

#[allow(dead_code)]
const TRANSPORT_AUTO: jint = 0;
#[allow(dead_code)]
const TRANSPORT_BREDR: jint = 1;
const TRANSPORT_LE: jint = 2;

#[allow(dead_code)]
const CHROMEBOOK_CONNECTION_CHECK_INTERVAL: jint = 10000;

const STEAM_CONTROLLER_SERVICE: &str = "100f6c32-1735-4313-b402-38567131e5f3";
const INPUT_CHARACTERISTIC_D0G: &str = "100f6c33-1735-4313-b402-38567131e5f3";
const INPUT_CHARACTERISTIC_TRITON_0X45: &str = "100f6c7a-1735-4313-b402-38567131e5f3";
const INPUT_CHARACTERISTIC_TRITON_0X47: &str = "100f6c7c-1735-4313-b402-38567131e5f3";
const REPORT_CHARACTERISTIC: &str = "100f6c34-1735-4313-b402-38567131e5f3";
const ENTER_VALVE_MODE: [u8; 6] = [0xC0, 0x87, 0x03, 0x08, 0x07, 0x00];
/// The Client Characteristic Configuration descriptor.
const CCCD: &str = "00002902-0000-1000-8000-00805f9b34fb";

/// `BluetoothProfile.STATE_DISCONNECTED`, `STATE_CONNECTING`,
/// `STATE_CONNECTED`.
const STATE_DISCONNECTED: jint = 0;
#[allow(dead_code)]
const STATE_CONNECTING: jint = 1;
const STATE_CONNECTED: jint = 2;
/// `BluetoothGattCharacteristic.PROPERTY_NOTIFY`, `PROPERTY_INDICATE`.
const PROPERTY_NOTIFY: jint = 0x10;
const PROPERTY_INDICATE: jint = 0x20;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// `UUID.fromString(uuid)`.
fn uuid<'a>(env: Env<'a>, uuid: &str) -> JniResult<'a, Local<'a>> {
    let uuid_class = env.find_class(c"java/util/UUID")?;
    let string = env.new_string(uuid)?;
    Ok(env
        .call_static_method(
            uuid_class.obj(),
            c"fromString",
            c"(Ljava/lang/String;)Ljava/util/UUID;",
            &[Arg::Object(string.obj())],
        )?
        .object()
        .expect("a UUID"))
}

/// `attribute.getUuid().toString()` (of a characteristic, a descriptor or
/// a service).
fn uuid_of<'a>(env: Env<'a>, attribute: Obj<'_>) -> JniResult<'a, String> {
    let attribute = manager::non_null(env, attribute, c"getUuid() on null")?;
    let uuid = env
        .call_method(attribute, c"getUuid", c"()Ljava/util/UUID;", &[])?
        .object();
    manager::string_of(env, uuid.as_ref().map_or(Obj::NULL, Local::obj))
}

/// `characteristic.getValue()`.
fn value_of<'a>(env: Env<'a>, characteristic: Obj<'_>) -> JniResult<'a, Vec<u8>> {
    let characteristic = manager::non_null(env, characteristic, c"getValue() on null")?;
    let value = env
        .call_method(characteristic, c"getValue", c"()[B", &[])?
        .object();
    match value {
        Some(value) => env.get_byte_array(value.obj()),
        None => Ok(Vec::new()),
    }
}

/// `Arrays.copyOfRange(report, from, to)` (the
/// `IllegalArgumentException` of `from > to`).
fn copy_of_range<'a>(
    env: Env<'a>,
    report: &[u8],
    from: usize,
    to: isize,
) -> JniResult<'a, Vec<u8>> {
    if (from as isize) > to {
        let message = std::ffi::CString::new(format!("{from} > {to}")).expect("a message");
        return manager::throw(env, c"java/lang/IllegalArgumentException", &message);
    }
    let to = to as usize;
    let mut range = report[from.min(report.len())..to.min(report.len())].to_vec();
    range.resize(to - from, 0);
    Ok(range)
}

/// The report ID of a characteristic's UUID (`100F6C` and two characters,
/// hexadecimal), less 0x35: a Triton's output report characteristic when
/// it's 0x80 or more. Translation of `probeService()`'s pattern
/// `100F6C([0-9A-Z]{2})` (case-insensitive, found anywhere) and
/// `Integer.parseInt(group, 16)`: `None` without a match, an error for a
/// group that isn't hexadecimal.
fn report_id_of_uuid(uuid: &str) -> Option<Result<jint, String>> {
    let bytes = uuid.as_bytes();
    let prefix = b"100f6c";
    (0..bytes.len().saturating_sub(prefix.len() + 1)).find_map(|i| {
        let candidate = &bytes[i..i + prefix.len() + 2];
        let (head, group) = candidate.split_at(prefix.len());
        if head.eq_ignore_ascii_case(prefix) && group.iter().all(u8::is_ascii_alphanumeric) {
            let group = std::str::from_utf8(group).expect("ASCII");
            Some(
                i32::from_str_radix(group, 16)
                    .map(|report_id| report_id - 0x35)
                    .map_err(|_| {
                        format!("java.lang.NumberFormatException: For input string: \"{group}\" under radix 16")
                    }),
            )
        } else {
            None
        }
    })
}

/// Translation of `GattOperation.Operation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    ChrRead,
    ChrWrite,
    EnableNotification,
}

/// Translation of `static class GattOperation`.
#[derive(Debug)]
struct GattOperation {
    /// `mOp`
    op: Operation,
    /// `mUuid`
    uuid: String,
    /// `mValue`
    value: Option<Vec<u8>>,
    /// `mGatt`
    gatt: Option<Arc<Global>>,
    /// `mResult`
    result: AtomicBool,
    /// `mDelayMs`
    delay_ms: jint,
}

impl GattOperation {
    fn new(
        gatt: Option<Arc<Global>>,
        operation: Operation,
        uuid: &str,
        value: Option<Vec<u8>>,
        delay_ms: jint,
    ) -> GattOperation {
        GattOperation {
            op: operation,
            uuid: uuid.to_owned(),
            value,
            gatt,
            result: AtomicBool::new(true),
            delay_ms,
        }
    }

    fn gatt<'a>(&self, env: Env<'a>) -> JniResult<'a, Obj<'_>> {
        match &self.gatt {
            Some(gatt) => Ok(gatt.obj()),
            None => manager::null_pointer(env, c"a call on a null BluetoothGatt"),
        }
    }

    /// Translation of `run()`: this is executed in main thread.
    fn run<'a>(&self, env: Env<'a>) -> JniResult<'a, ()> {
        match self.op {
            Operation::ChrRead => {
                let chr = self.get_characteristic(env, &self.uuid)?;
                // (Log.v("Reading characteristic " + chr.getUuid()), commented out)
                if !env
                    .call_method(
                        self.gatt(env)?,
                        c"readCharacteristic",
                        c"(Landroid/bluetooth/BluetoothGattCharacteristic;)Z",
                        &[Arg::Object(chr.as_ref().map_or(Obj::NULL, Local::obj))],
                    )?
                    .boolean()
                {
                    log(
                        LogPriority::Error,
                        &format!("Unable to read characteristic {}", self.uuid),
                    );
                    self.result.store(false, Ordering::SeqCst);
                    return Ok(());
                }
                self.result.store(true, Ordering::SeqCst);
            }
            Operation::ChrWrite => {
                let chr = self.get_characteristic(env, &self.uuid)?;
                // (Log.v("Writing characteristic " + chr.getUuid() + " value=" + ...), commented out)
                let Some(chr) = chr else {
                    return manager::null_pointer(env, c"setValue() on a null characteristic");
                };
                let value = env.new_byte_array(self.value.as_deref().unwrap_or_default())?;
                env.call_method(
                    chr.obj(),
                    c"setValue",
                    c"([B)Z",
                    &[Arg::Object(value.obj())],
                )?;
                if !env
                    .call_method(
                        self.gatt(env)?,
                        c"writeCharacteristic",
                        c"(Landroid/bluetooth/BluetoothGattCharacteristic;)Z",
                        &[Arg::Object(chr.obj())],
                    )?
                    .boolean()
                {
                    log(
                        LogPriority::Error,
                        &format!("Unable to write characteristic {}", self.uuid),
                    );
                    self.result.store(false, Ordering::SeqCst);
                    return Ok(());
                }
                self.result.store(true, Ordering::SeqCst);
            }
            Operation::EnableNotification => {
                let chr = self.get_characteristic(env, &self.uuid)?;
                // (Log.v("Writing descriptor of " + chr.getUuid()), commented out)
                if let Some(chr) = chr {
                    let cccd_uuid = uuid(env, CCCD)?;
                    let cccd = env
                        .call_method(
                            chr.obj(),
                            c"getDescriptor",
                            c"(Ljava/util/UUID;)Landroid/bluetooth/BluetoothGattDescriptor;",
                            &[Arg::Object(cccd_uuid.obj())],
                        )?
                        .object();
                    if let Some(cccd) = cccd {
                        let properties = env
                            .call_method(chr.obj(), c"getProperties", c"()I", &[])?
                            .int();
                        let field: &CStr = if (properties & PROPERTY_NOTIFY) == PROPERTY_NOTIFY {
                            c"ENABLE_NOTIFICATION_VALUE"
                        } else if (properties & PROPERTY_INDICATE) == PROPERTY_INDICATE {
                            c"ENABLE_INDICATION_VALUE"
                        } else {
                            log(
                                LogPriority::Error,
                                "Unable to start notifications on input characteristic",
                            );
                            self.result.store(false, Ordering::SeqCst);
                            return Ok(());
                        };
                        let descriptor_class =
                            env.find_class(c"android/bluetooth/BluetoothGattDescriptor")?;
                        let value = env
                            .get_static_field(descriptor_class.obj(), field, c"[B")?
                            .object();

                        env.call_method(
                            self.gatt(env)?,
                            c"setCharacteristicNotification",
                            c"(Landroid/bluetooth/BluetoothGattCharacteristic;Z)Z",
                            &[Arg::Object(chr.obj()), Arg::Boolean(true)],
                        )?;
                        env.call_method(
                            cccd.obj(),
                            c"setValue",
                            c"([B)Z",
                            &[Arg::Object(value.as_ref().map_or(Obj::NULL, Local::obj))],
                        )?;
                        if !env
                            .call_method(
                                self.gatt(env)?,
                                c"writeDescriptor",
                                c"(Landroid/bluetooth/BluetoothGattDescriptor;)Z",
                                &[Arg::Object(cccd.obj())],
                            )?
                            .boolean()
                        {
                            log(
                                LogPriority::Error,
                                &format!("Unable to write descriptor {}", self.uuid),
                            );
                            self.result.store(false, Ordering::SeqCst);
                            return Ok(());
                        }
                        self.result.store(true, Ordering::SeqCst);
                    }
                }
            }
        }
        Ok(())
    }

    /// Translation of `finish()`.
    fn finish(&self) -> bool {
        self.result.load(Ordering::SeqCst)
    }

    /// Translation of `getDelayMs()`.
    fn get_delay_ms(&self) -> jint {
        self.delay_ms
    }

    /// Translation of `getCharacteristic()`.
    fn get_characteristic<'a>(&self, env: Env<'a>, uuid: &str) -> JniResult<'a, Option<Local<'a>>> {
        get_characteristic(env, self.gatt(env)?, uuid)
    }

    /// Translation of `readCharacteristic()`.
    fn read_characteristic(gatt: Option<Arc<Global>>, uuid: &str) -> GattOperation {
        GattOperation::new(gatt, Operation::ChrRead, uuid, None, 0)
    }

    /// Translation of `writeCharacteristic()`.
    fn write_characteristic(
        gatt: Option<Arc<Global>>,
        uuid: &str,
        value: Vec<u8>,
    ) -> GattOperation {
        GattOperation::new(gatt, Operation::ChrWrite, uuid, Some(value), 0)
    }

    /// Translation of `enableNotification()` (with a delay).
    fn enable_notification(gatt: Option<Arc<Global>>, uuid: &str, delay_ms: jint) -> GattOperation {
        GattOperation::new(gatt, Operation::EnableNotification, uuid, None, delay_ms)
    }
}

/// Valve's service's characteristic of `uuid` (`gatt.getService(...)
/// .getCharacteristic(uuid)`), `None` without the service.
fn get_characteristic<'a>(
    env: Env<'a>,
    gatt: Obj<'_>,
    uuid_str: &str,
) -> JniResult<'a, Option<Local<'a>>> {
    let Some(valve_service) = get_valve_service(env, gatt)? else {
        return Ok(None);
    };
    let uuid = uuid(env, uuid_str)?;
    Ok(env
        .call_method(
            valve_service.obj(),
            c"getCharacteristic",
            c"(Ljava/util/UUID;)Landroid/bluetooth/BluetoothGattCharacteristic;",
            &[Arg::Object(uuid.obj())],
        )?
        .object())
}

/// `gatt.getService(steamControllerService)`.
fn get_valve_service<'a>(env: Env<'a>, gatt: Obj<'_>) -> JniResult<'a, Option<Local<'a>>> {
    let service = uuid(env, STEAM_CONTROLLER_SERVICE)?;
    Ok(env
        .call_method(
            gatt,
            c"getService",
            c"(Ljava/util/UUID;)Landroid/bluetooth/BluetoothGattService;",
            &[Arg::Object(service.obj())],
        )?
        .object())
}

/// `gatt.getServices()`.
fn get_services<'a>(env: Env<'a>, gatt: Obj<'_>) -> JniResult<'a, Vec<Local<'a>>> {
    let services = env
        .call_method(gatt, c"getServices", c"()Ljava/util/List;", &[])?
        .object();
    manager::collection(env, services.as_ref().map_or(Obj::NULL, Local::obj))
}

/// `mOperations` and `mCurrentOperation`, with the monitor of
/// `synchronized (mOperations)`.
#[derive(Debug, Default)]
struct Operations {
    queue: VecDeque<Arc<GattOperation>>,
    current: Option<Arc<GattOperation>>,
}

/// Translation of `class HIDDeviceBLESteamController`'s fields.
#[derive(Debug)]
pub(crate) struct HidDeviceBleSteamController {
    /// The Java object (the `BluetoothGattCallback`).
    this: Global,
    /// `mManager`
    manager: Mutex<Option<Weak<HidDeviceManager>>>,
    /// `mDevice`
    device: Global,
    /// `mDeviceId`
    device_id: jint,
    /// `mGatt`
    gatt: Mutex<Option<Arc<Global>>>,
    /// `mIsRegistered`
    is_registered: AtomicBool,
    /// `mIsConnected`
    is_connected: AtomicBool,
    /// `mIsChromebook`
    is_chromebook: bool,
    /// `mIsReconnecting`
    is_reconnecting: AtomicBool,
    /// `mHasEnabledNotifications`
    has_enabled_notifications: AtomicBool,
    /// `mHasSeenInputUpdate`
    has_seen_input_update: AtomicBool,
    /// `mFrozen`
    frozen: AtomicBool,
    /// `mOperations`, `mCurrentOperation`
    operations: Mutex<Operations>,
    /// `mHandler`
    handler: Global,
    /// `mProductId`
    product_id: AtomicI32,
    /// `mReportId`
    report_id: AtomicI32,
    /// `mInputCharacteristic`
    input_characteristic: Mutex<Option<String>>,
    /// `mOutputReportChars`
    output_report_chars: Mutex<HashMap<jint, Global>>,
    weak_self: Weak<HidDeviceBleSteamController>,
}

/// The devices made, for their callbacks.
static DEVICES: Mutex<Vec<Weak<HidDeviceBleSteamController>>> = Mutex::new(Vec::new());

/// The device of a callback object.
fn device_of(env: Env<'_>, this: Obj<'_>) -> Option<Arc<HidDeviceBleSteamController>> {
    let mut devices = lock(&DEVICES);
    devices.retain(|device| device.strong_count() > 0);
    devices
        .iter()
        .filter_map(Weak::upgrade)
        .find(|device| env.is_same_object(device.this.obj(), this))
}

impl HidDeviceBleSteamController {
    /// Translation of the constructor
    /// `HIDDeviceBLESteamController(HIDDeviceManager, BluetoothDevice)`.
    pub(super) fn new<'a>(
        env: Env<'a>,
        manager: &HidDeviceManager,
        device: Obj<'_>,
    ) -> JniResult<'a, Arc<HidDeviceBleSteamController>> {
        let class = env::app_class(HID_DEVICE_BLE_STEAM_CONTROLLER)
            .expect("JNI_OnLoad() found HIDDeviceBLESteamController");
        let this = env.new_object(class.obj(), c"()V", &[])?;
        let identifier = identifier(env, device)?;
        let device_id = manager.get_device_id_for_identifier(env, &identifier)?;
        let is_chromebook = super::activity::is_chromebook(env)?;
        let handler = manager::main_handler(env)?;
        let global = |obj: Obj<'_>| Global::new(env, obj).expect("a global reference");

        let controller = Arc::new_cyclic(|weak_self| HidDeviceBleSteamController {
            this: global(this.obj()),
            manager: Mutex::new(Some(manager.weak())),
            device: global(device),
            device_id,
            gatt: Mutex::new(None),
            is_registered: AtomicBool::new(false),
            is_connected: AtomicBool::new(false),
            is_chromebook,
            is_reconnecting: AtomicBool::new(false),
            has_enabled_notifications: AtomicBool::new(false),
            has_seen_input_update: AtomicBool::new(false),
            frozen: AtomicBool::new(false),
            operations: Mutex::new(Operations::default()),
            handler: global(handler.obj()),
            product_id: AtomicI32::new(-1),
            report_id: AtomicI32::new(0),
            input_characteristic: Mutex::new(None),
            output_report_chars: Mutex::new(HashMap::new()),
            weak_self: weak_self.clone(),
        });
        lock(&DEVICES).push(Arc::downgrade(&controller));

        let gatt = controller.connect_gatt(env)?;
        controller.set_gatt(gatt);
        controller
            .has_enabled_notifications
            .store(false, Ordering::SeqCst);
        controller
            .has_seen_input_update
            .store(false, Ordering::SeqCst);
        // (a commented-out postDelayed() runs checkConnectionForChromebookIssue() in 10 s)
        Ok(controller)
    }

    /// `mManager`, the `NullPointerException` of a call on it once it's
    /// shut down.
    fn manager<'a>(&self, env: Env<'a>) -> JniResult<'a, Arc<HidDeviceManager>> {
        match lock(&self.manager).as_ref().and_then(Weak::upgrade) {
            Some(manager) => Ok(manager),
            None => manager::null_pointer(env, c"a call on a null HIDDeviceManager"),
        }
    }

    /// Translation of `getIdentifier()`.
    fn get_identifier<'a>(&self, env: Env<'a>) -> JniResult<'a, String> {
        identifier(env, self.device.obj())
    }

    /// Translation of `getGatt()`.
    #[allow(dead_code)]
    fn get_gatt(&self) -> Option<Arc<Global>> {
        lock(&self.gatt).clone()
    }

    fn set_gatt(&self, gatt: Option<Local<'_>>) {
        let gatt = gatt.and_then(|g| {
            let env = env::get_env()?;
            Global::new(env, g.obj()).map(Arc::new)
        });
        *lock(&self.gatt) = gatt;
    }

    /// `mGatt`, the `NullPointerException` of a call on it when it's null.
    fn gatt<'a>(&self, env: Env<'a>) -> JniResult<'a, Arc<Global>> {
        match lock(&self.gatt).clone() {
            Some(gatt) => Ok(gatt),
            None => manager::null_pointer(env, c"a call on a null BluetoothGatt"),
        }
    }

    /// Because on Chromebooks we show up as a dual-mode device, it will attempt to connect TRANSPORT_AUTO, which will use TRANSPORT_BREDR instead
    /// of TRANSPORT_LE.  Let's force ourselves to connect low energy.
    /// Translation of `connectGatt(boolean)`.
    fn connect_gatt_managed<'a>(
        &self,
        env: Env<'a>,
        managed: bool,
    ) -> JniResult<'a, Option<Local<'a>>> {
        let manager = self.manager(env)?;
        let context = manager.get_context();
        let connect = |transport: Option<jint>| -> JniResult<'a, Option<Local<'a>>> {
            let mut args = vec![
                Arg::Object(context),
                Arg::Boolean(managed),
                Arg::Object(self.this.obj()),
            ];
            let descriptor: &CStr = match transport {
                Some(transport) => {
                    args.push(Arg::Int(transport));
                    c"(Landroid/content/Context;ZLandroid/bluetooth/BluetoothGattCallback;I)Landroid/bluetooth/BluetoothGatt;"
                }
                None => c"(Landroid/content/Context;ZLandroid/bluetooth/BluetoothGattCallback;)Landroid/bluetooth/BluetoothGatt;",
            };
            Ok(env
                .call_method(self.device.obj(), c"connectGatt", descriptor, &args)?
                .object())
        };
        if super::super::sdk_int(env) >= 23
        /* Android 6.0 (M) */
        {
            match connect(Some(TRANSPORT_LE)) {
                Ok(gatt) => Ok(gatt),
                Err(thrown) => {
                    drop(thrown.catch_exception()?);
                    connect(None)
                }
            }
        } else {
            connect(None)
        }
    }

    /// Translation of `connectGatt()`.
    fn connect_gatt<'a>(&self, env: Env<'a>) -> JniResult<'a, Option<Local<'a>>> {
        self.connect_gatt_managed(env, false)
    }

    /// Translation of `getConnectionState()`.
    fn get_connection_state<'a>(&self, env: Env<'a>) -> JniResult<'a, jint> {
        let manager = self.manager(env)?;
        let context = manager.get_context();
        if context.is_null() {
            // We are lacking any context to get our Bluetooth information.  We'll just assume disconnected.
            return Ok(STATE_DISCONNECTED);
        }

        let bt_manager = manager::system_service(env, context, manager::BLUETOOTH_SERVICE)?;
        let Some(bt_manager) = bt_manager else {
            // This device doesn't support Bluetooth.  We should never be here, because how did
            // we instantiate a device to start with?
            return Ok(STATE_DISCONNECTED);
        };

        Ok(env
            .call_method(
                bt_manager.obj(),
                c"getConnectionState",
                c"(Landroid/bluetooth/BluetoothDevice;I)I",
                &[Arg::Object(self.device.obj()), Arg::Int(manager::GATT)],
            )?
            .int())
    }

    /// `mGatt.disconnect(); mGatt = connectGatt(false);`.
    fn reconnect_gatt<'a>(&self, env: Env<'a>) -> JniResult<'a, ()> {
        let gatt = self.gatt(env)?;
        env.call_method(gatt.obj(), c"disconnect", c"()V", &[])?;
        let gatt = self.connect_gatt_managed(env, false)?;
        self.set_gatt(gatt);
        Ok(())
    }

    /// Translation of `reconnect()`.
    pub(super) fn reconnect<'a>(&self, env: Env<'a>) -> JniResult<'a, ()> {
        if self.get_connection_state(env)? != STATE_CONNECTED {
            let gatt = self.gatt(env)?;
            env.call_method(gatt.obj(), c"disconnect", c"()V", &[])?;
            let gatt = self.connect_gatt(env)?;
            self.set_gatt(gatt);
        }
        Ok(())
    }

    /// Translation of `checkConnectionForChromebookIssue()` (Note
    /// (upstream): never called, its first call is commented out).
    #[allow(dead_code)]
    fn check_connection_for_chromebook_issue<'a>(&self, env: Env<'a>) -> JniResult<'a, ()> {
        if !self.is_chromebook {
            // We only do this on Chromebooks, because otherwise it's really annoying to just attempt
            // over and over.
            return Ok(());
        }

        let connection_state = self.get_connection_state(env)?;

        match connection_state {
            STATE_CONNECTED => {
                if !self.is_connected.load(Ordering::SeqCst) {
                    // We are in the Bad Chromebook Place.  We can force a disconnect
                    // to try to recover.
                    log(LogPriority::Verbose, "Chromebook: We are in a very bad state; the controller shows as connected in the underlying Bluetooth layer, but we never received a callback.  Forcing a reconnect.");
                    self.is_reconnecting.store(true, Ordering::SeqCst);
                    self.reconnect_gatt(env)?;
                } else if !self.is_registered() {
                    let gatt = self.gatt(env)?;
                    if !get_services(env, gatt.obj())?.is_empty() {
                        log(LogPriority::Verbose, "Chromebook: We are connected to a controller, but never got our registration.  Trying to recover.");
                        self.probe_service(env)?;
                    } else {
                        log(LogPriority::Verbose, "Chromebook: We are connected to a controller, but never discovered services.  Trying to recover.");
                        self.is_reconnecting.store(true, Ordering::SeqCst);
                        self.reconnect_gatt(env)?;
                    }
                } else {
                    log(
                        LogPriority::Verbose,
                        "Chromebook: We are connected, and registered.  Everything's good!",
                    );
                    return Ok(());
                }
            }

            STATE_DISCONNECTED => {
                log(LogPriority::Verbose, "Chromebook: We have either been disconnected, or the Chromebook BtGatt.ContextMap bug has bitten us.  Attempting a disconnect/reconnect, but we may not be able to recover.");

                self.is_reconnecting.store(true, Ordering::SeqCst);
                self.reconnect_gatt(env)?;
            }

            STATE_CONNECTING => {
                log(
                    LogPriority::Verbose,
                    "Chromebook: We're still trying to connect.  Waiting a bit longer.",
                );
            }

            _ => {}
        }

        let final_this = self.weak_self.clone();
        manager::post_delayed(
            env,
            self.handler.obj(),
            CHROMEBOOK_CONNECTION_CHECK_INTERVAL,
            move |env| {
                if let Some(final_this) = final_this.upgrade() {
                    if let Err(thrown) = final_this.check_connection_for_chromebook_issue(env) {
                        // (thrown in the main thread's Java)
                        drop(thrown);
                    }
                }
            },
        )
    }

    /// Translation of `isRegistered()`.
    fn is_registered(&self) -> bool {
        self.is_registered.load(Ordering::SeqCst)
    }

    /// Translation of `setRegistered()`.
    fn set_registered(&self) {
        self.is_registered.store(true, Ordering::SeqCst);
    }

    /// Translation of `probeService()` (of itself, its only caller's
    /// argument).
    fn probe_service<'a>(&self, env: Env<'a>) -> JniResult<'a, bool> {
        if self.is_registered() {
            return Ok(true);
        }

        if !self.is_connected.load(Ordering::SeqCst) {
            return Ok(false);
        }

        log(
            LogPriority::Verbose,
            &format!(
                "probeService controller={}",
                manager::string_of(env, self.this.obj())?
            ),
        );

        let gatt = self.gatt(env)?;
        let services = get_services(env, gatt.obj())?;
        for service in &services {
            let service_uuid = uuid_of(env, service.obj())?;
            if service_uuid == STEAM_CONTROLLER_SERVICE {
                log(
                    LogPriority::Verbose,
                    &format!("Found Valve steam controller service {service_uuid}"),
                );

                let characteristics = env
                    .call_method(
                        service.obj(),
                        c"getCharacteristics",
                        c"()Ljava/util/List;",
                        &[],
                    )?
                    .object();
                let characteristics = manager::collection(
                    env,
                    characteristics.as_ref().map_or(Obj::NULL, Local::obj),
                )?;
                for chr in &characteristics {
                    let chr_uuid = uuid_of(env, chr.obj())?;
                    if chr_uuid == INPUT_CHARACTERISTIC_TRITON_0X45 {
                        log(
                            LogPriority::Verbose,
                            "Found Triton input characteristic 0x45",
                        );
                        self.product_id.store(TRITON_BLE_PID, Ordering::SeqCst);
                        self.report_id.store(0x45, Ordering::SeqCst);
                        *lock(&self.input_characteristic) = Some(chr_uuid);
                    } else if chr_uuid == INPUT_CHARACTERISTIC_TRITON_0X47 {
                        log(
                            LogPriority::Verbose,
                            "Found Triton input characteristic 0x47",
                        );
                        self.product_id.store(TRITON_BLE_PID, Ordering::SeqCst);
                        self.report_id.store(0x47, Ordering::SeqCst);
                        *lock(&self.input_characteristic) = Some(chr_uuid);
                    } else if chr_uuid == INPUT_CHARACTERISTIC_D0G {
                        log(LogPriority::Verbose, "Found D0G input characteristic");
                        self.product_id.store(D0G_BLE2_PID, Ordering::SeqCst);
                        self.report_id.store(0x03, Ordering::SeqCst);
                        *lock(&self.input_characteristic) = Some(chr_uuid);
                    } else {
                        match report_id_of_uuid(&chr_uuid) {
                            Some(Ok(report_id)) => {
                                if report_id >= 0x80 {
                                    // This is a Triton output report characteristic that we need to care about.
                                    log(
                                        LogPriority::Verbose,
                                        &format!("Found Triton output report 0x{report_id:x}"),
                                    );
                                    if let Some(chr) = Global::new(env, chr.obj()) {
                                        lock(&self.output_report_chars).insert(report_id, chr);
                                    }
                                }
                            }
                            Some(Err(nfe)) => {
                                log(
                                    LogPriority::Warn,
                                    &format!(
                                        "Could not parse report characteristic {chr_uuid}: {nfe}"
                                    ),
                                );
                            }
                            None => {}
                        }
                    }
                }

                let input_characteristic = lock(&self.input_characteristic).clone();
                for chr in &characteristics {
                    if Some(uuid_of(env, chr.obj())?) == input_characteristic {
                        // Start notifications
                        let cccd_uuid = uuid(env, CCCD)?;
                        let cccd = env
                            .call_method(
                                chr.obj(),
                                c"getDescriptor",
                                c"(Ljava/util/UUID;)Landroid/bluetooth/BluetoothGattDescriptor;",
                                &[Arg::Object(cccd_uuid.obj())],
                            )?
                            .object();
                        if cccd.is_some() {
                            self.enable_notification(env, &uuid_of(env, chr.obj())?)?;
                        }
                    }
                }
                return Ok(true);
            }
        }

        if services.is_empty() && self.is_chromebook && !self.is_reconnecting.load(Ordering::SeqCst)
        {
            log(LogPriority::Error, "Chromebook: Discovered services were empty; this almost certainly means the BtGatt.ContextMap bug has bitten us.");
            self.is_connected.store(false, Ordering::SeqCst);
            self.is_reconnecting.store(true, Ordering::SeqCst);
            self.reconnect_gatt(env)?;
        }

        Ok(false)
    }

    /// Translation of `finishCurrentGattOperation()`.
    fn finish_current_gatt_operation<'a>(&self, env: Env<'a>) -> JniResult<'a, ()> {
        let op = lock(&self.operations).current.take();
        if let Some(op) = op {
            let result = op.finish(); // TODO: Maybe in main thread as well?

            // Our operation failed, let's add it back to the beginning of our queue.
            if !result {
                // (Note: with the queue's monitor here)
                lock(&self.operations).queue.push_front(op);
            }
        }
        self.execute_next_gatt_operation(env)
    }

    /// Translation of `executeNextGattOperation()`.
    fn execute_next_gatt_operation<'a>(&self, env: Env<'a>) -> JniResult<'a, ()> {
        let delay_ms = {
            let mut operations = lock(&self.operations);
            if operations.current.is_some() {
                return Ok(());
            }

            let Some(next) = operations.queue.pop_front() else {
                return Ok(());
            };
            let delay_ms = next.get_delay_ms();
            operations.current = Some(next);
            delay_ms
        };

        let this = self.weak_self.clone();
        let gatt_operation_runnable = move |env: Env<'_>| {
            let Some(this) = this.upgrade() else {
                return;
            };
            let operations = lock(&this.operations);
            let Some(current) = &operations.current else {
                log(LogPriority::Error, "Current operation null in executor?");
                return;
            };

            if let Err(thrown) = current.run(env) {
                // (thrown in the main thread's Java)
                drop(thrown);
            }
            // now wait for the GATT callback and when it comes, finish this operation
        };

        // (Run in main thread; if we have a delay on this operation, wait before we post it.)
        manager::post_delayed(env, self.handler.obj(), delay_ms, gatt_operation_runnable)
    }

    /// Translation of `queueGattOperation()`.
    fn queue_gatt_operation<'a>(&self, env: Env<'a>, op: GattOperation) -> JniResult<'a, ()> {
        lock(&self.operations).queue.push_back(Arc::new(op));
        self.execute_next_gatt_operation(env)
    }

    /// Translation of `enableNotification()`.
    fn enable_notification<'a>(&self, env: Env<'a>, chr_uuid: &str) -> JniResult<'a, ()> {
        // Add a 500ms delay to notification write for Amazon Fire TV devices, as otherwise if we do this too quickly after connecting
        // it will return success and then silently drop the operation on the floor.
        let op = GattOperation::enable_notification(lock(&self.gatt).clone(), chr_uuid, 500);
        self.queue_gatt_operation(env, op)?;

        // Amazon Fire devices can also silently timeout on writeDescriptor, so
        // set up a little delayed check that will attempt to write a second time.
        //
        // While this only seems to be needed on Amazon Fire TV devices at present, it
        // doesn't hurt to have a retry on other devices as well.
        //
        let final_this = self.weak_self.clone();
        let final_uuid = chr_uuid.to_owned();
        manager::post_delayed(env, self.handler.obj(), 1000, move |env| {
            let Some(final_this) = final_this.upgrade() else {
                return;
            };
            if !final_this.has_enabled_notifications.load(Ordering::SeqCst) {
                if final_this.has_seen_input_update.load(Ordering::SeqCst) {
                    // Amazon Five devices may have enabled notifications on the input characteristic and not given us a callback. If we've seen
                    // input reports, though, somewhat by definition notifications are enabled.
                    log(LogPriority::Warn, "WriteDescriptor has never returned, but we've seen input reports. Moving on with controller initialization.");
                    final_this
                        .has_enabled_notifications
                        .store(true, Ordering::SeqCst);
                    if let Err(thrown) = final_this.enable_valve_mode(env) {
                        drop(thrown);
                    }
                    return;
                }

                // Give one more try.
                let retry = GattOperation::enable_notification(
                    lock(&final_this.gatt).clone(),
                    &final_uuid,
                    500,
                );
                if let Err(thrown) = final_this.queue_gatt_operation(env, retry) {
                    drop(thrown);
                }
            }
        })
    }

    /// Translation of `writeCharacteristic()`.
    fn write_characteristic<'a>(
        &self,
        env: Env<'a>,
        uuid: &str,
        value: Vec<u8>,
    ) -> JniResult<'a, ()> {
        let op = GattOperation::write_characteristic(lock(&self.gatt).clone(), uuid, value);
        self.queue_gatt_operation(env, op)
    }

    /// Translation of `readCharacteristic()`.
    fn read_characteristic<'a>(&self, env: Env<'a>, uuid: &str) -> JniResult<'a, ()> {
        let op = GattOperation::read_characteristic(lock(&self.gatt).clone(), uuid);
        self.queue_gatt_operation(env, op)
    }

    /// `HIDDeviceConnected()` of the controller, once configured.
    fn register<'a>(&self, env: Env<'a>) -> JniResult<'a, ()> {
        self.manager(env)?;
        let identifier = self.get_identifier(env)?;
        let serial_number = self.get_serial_number(env)?;
        let manufacturer = self.get_manufacturer_name(env)?;
        let product = self.get_product_name(env)?;
        hid::hid_device_connected(&hid::ConnectedDevice {
            device_id: self.get_id(),
            identifier: &identifier,
            vendor_id: self.get_vendor_id(env)?,
            product_id: self.get_product_id(env)?,
            serial_number: &serial_number,
            release_number: self.get_version(),
            manufacturer: &manufacturer,
            product: &product,
            interface: 0,
            interface_class: 0,
            interface_subclass: 0,
            interface_protocol: 0,
            bluetooth: true,
            report_id: self.report_id.load(Ordering::SeqCst),
        });
        Ok(())
    }

    // BluetoothGattCallback overridden methods

    /// Translation of `onConnectionStateChange()`.
    fn on_connection_state_change<'a>(
        &self,
        env: Env<'a>,
        _status: jint,
        new_state: jint,
    ) -> JniResult<'a, ()> {
        // (Log.v("onConnectionStateChange status=" + status + " newState=" + newState), commented out)
        self.is_reconnecting.store(false, Ordering::SeqCst);
        if new_state == 2 {
            self.is_connected.store(true, Ordering::SeqCst);
            // Run directly, without GattOperation
            if !self.is_registered() {
                let this = self.weak_self.clone();
                manager::post_delayed(env, self.handler.obj(), 0, move |env| {
                    let Some(this) = this.upgrade() else {
                        return;
                    };
                    let result = this.gatt(env).and_then(|gatt| {
                        env.call_method(gatt.obj(), c"discoverServices", c"()Z", &[])
                    });
                    if let Err(thrown) = result {
                        drop(thrown);
                    }
                })?;
            }
        } else if new_state == 0 {
            self.is_connected.store(false, Ordering::SeqCst);
        }

        // Disconnection is handled in SteamLink using the ACTION_ACL_DISCONNECTED Intent.
        Ok(())
    }

    /// Translation of `onServicesDiscovered()`.
    fn on_services_discovered<'a>(
        &self,
        env: Env<'a>,
        gatt: Obj<'_>,
        status: jint,
    ) -> JniResult<'a, ()> {
        // (Log.v("onServicesDiscovered status=" + status), commented out)
        if status == 0 {
            let gatt = manager::non_null(env, gatt, c"getServices() on a null BluetoothGatt")?;
            if get_services(env, gatt)?.is_empty() {
                log(LogPriority::Verbose, "onServicesDiscovered returned zero services; something has gone horribly wrong down in Android's Bluetooth stack.");
                self.is_reconnecting.store(true, Ordering::SeqCst);
                self.is_connected.store(false, Ordering::SeqCst);
                env.call_method(gatt, c"disconnect", c"()V", &[])?;
                let gatt = self.connect_gatt_managed(env, false)?;
                self.set_gatt(gatt);
            } else {
                if self.get_product_id(env)? == TRITON_BLE_PID {
                    // Android will not properly play well with Data Length Extensions without manually requesting a large MTU,
                    // and Triton controllers require DLE support.
                    //
                    // 517 is basically a "magic number" as far as Android's bluetooth code is concerned, so do not change
                    // this value. It is functionally "please enable data length extensions" on some Android builds.
                    let gatt = self.gatt(env)?;
                    env.call_method(gatt.obj(), c"requestMtu", c"(I)Z", &[Arg::Int(517)])?;
                }

                self.probe_service(env)?;
            }
        }
        Ok(())
    }

    /// Translation of `onCharacteristicRead()`.
    fn on_characteristic_read<'a>(
        &self,
        env: Env<'a>,
        characteristic: Obj<'_>,
    ) -> JniResult<'a, ()> {
        // (Log.v("onCharacteristicRead status=" + status + " uuid=" + characteristic.getUuid()), commented out)

        if uuid_of(env, characteristic)? == REPORT_CHARACTERISTIC
            && !self.frozen.load(Ordering::SeqCst)
        {
            self.manager(env)?;
            hid::hid_device_report_response(self.get_id(), &value_of(env, characteristic)?);
        }

        self.finish_current_gatt_operation(env)
    }

    /// Translation of `onCharacteristicWrite()`.
    fn on_characteristic_write<'a>(
        &self,
        env: Env<'a>,
        characteristic: Obj<'_>,
    ) -> JniResult<'a, ()> {
        // (Log.v("onCharacteristicWrite status=" + status + " uuid=" + characteristic.getUuid()), commented out)

        if uuid_of(env, characteristic)? == REPORT_CHARACTERISTIC {
            // Only register controller with the native side once it has been fully configured
            if !self.is_registered() {
                log(
                    LogPriority::Verbose,
                    &format!("Registering Steam Controller with ID: {}", self.get_id()),
                );
                self.register(env)?;
                self.set_registered();
            }
        }

        self.finish_current_gatt_operation(env)
    }

    /// Translation of `onCharacteristicChanged()`.
    fn on_characteristic_changed<'a>(
        &self,
        env: Env<'a>,
        characteristic: Obj<'_>,
    ) -> JniResult<'a, ()> {
        // Enable this for verbose logging of controller input reports
        // (Log.v("onCharacteristicChanged uuid=" + characteristic.getUuid() + " data=" + ...), commented out)

        let input_characteristic = lock(&self.input_characteristic).clone();
        if Some(uuid_of(env, characteristic)?) == input_characteristic
            && !self.frozen.load(Ordering::SeqCst)
        {
            self.has_seen_input_update.store(true, Ordering::SeqCst);
            self.manager(env)?;
            hid::hid_device_input_report(self.get_id(), &value_of(env, characteristic)?);
        }
        Ok(())
    }

    /// Translation of `enableValveMode()`.
    fn enable_valve_mode<'a>(&self, env: Env<'a>) -> JniResult<'a, ()> {
        let gatt = self.gatt(env)?;
        let Some(valve_service) = get_valve_service(env, gatt.obj())? else {
            return Ok(());
        };

        let report_uuid = uuid(env, REPORT_CHARACTERISTIC)?;
        let report_chr = env
            .call_method(
                valve_service.obj(),
                c"getCharacteristic",
                c"(Ljava/util/UUID;)Landroid/bluetooth/BluetoothGattCharacteristic;",
                &[Arg::Object(report_uuid.obj())],
            )?
            .object();
        if let Some(report_chr) = report_chr {
            if self.get_product_id(env)? == TRITON_BLE_PID {
                // For Triton we just mark things registered.
                log(
                    LogPriority::Verbose,
                    &format!(
                        "Registering Triton Steam Controller with ID: {}",
                        self.get_id()
                    ),
                );
                self.register(env)?;
                self.set_registered();
            } else {
                // For the original controller, we need to manually enter Valve mode.
                log(
                    LogPriority::Verbose,
                    "Writing report characteristic to enter valve mode",
                );
                let value = env.new_byte_array(&ENTER_VALVE_MODE)?;
                env.call_method(
                    report_chr.obj(),
                    c"setValue",
                    c"([B)Z",
                    &[Arg::Object(value.obj())],
                )?;
                env.call_method(
                    gatt.obj(),
                    c"writeCharacteristic",
                    c"(Landroid/bluetooth/BluetoothGattCharacteristic;)Z",
                    &[Arg::Object(report_chr.obj())],
                )?;
            }
        }
        Ok(())
    }

    /// Translation of `onDescriptorWrite()`.
    fn on_descriptor_write<'a>(&self, env: Env<'a>, descriptor: Obj<'_>) -> JniResult<'a, ()> {
        let descriptor =
            manager::non_null(env, descriptor, c"getCharacteristic() on a null descriptor")?;
        let chr = env
            .call_method(
                descriptor,
                c"getCharacteristic",
                c"()Landroid/bluetooth/BluetoothGattCharacteristic;",
                &[],
            )?
            .object();
        // (Log.v("onDescriptorWrite status=" + status + " uuid=" + chr.getUuid() + " descriptor=" + ...), commented out)

        let input_characteristic = lock(&self.input_characteristic).clone();
        if Some(uuid_of(env, chr.as_ref().map_or(Obj::NULL, Local::obj))?) == input_characteristic {
            self.has_enabled_notifications.store(true, Ordering::SeqCst);
            self.enable_valve_mode(env)?;
        }

        self.finish_current_gatt_operation(env)
    }
}

/// `String.format("SteamController.%s", device.getAddress())`.
fn identifier<'a>(env: Env<'a>, device: Obj<'_>) -> JniResult<'a, String> {
    let device = manager::non_null(env, device, c"getAddress() on a null device")?;
    let address = manager::call_string(env, device, c"getAddress")?;
    Ok(format!(
        "SteamController.{}",
        address.as_deref().unwrap_or("null")
    ))
}

// Public API

impl HidDevice for HidDeviceBleSteamController {
    fn get_id(&self) -> jint {
        self.device_id
    }

    fn get_vendor_id<'a>(&self, _env: Env<'a>) -> JniResult<'a, jint> {
        // Valve Corporation
        const VALVE_USB_VID: jint = 0x28DE;
        Ok(VALVE_USB_VID)
    }

    fn get_product_id<'a>(&self, env: Env<'a>) -> JniResult<'a, jint> {
        let product_id = self.product_id.load(Ordering::SeqCst);
        if product_id > 0 {
            // We've already set a product ID.
            return Ok(product_id);
        }

        let name = manager::call_string(env, self.device.obj(), c"getName")?;
        let Some(name) = name else {
            return manager::null_pointer(env, c"startsWith() on a null name");
        };
        let product_id = if name.starts_with("Steam Ctrl") {
            // We're a newer Triton device
            TRITON_BLE_PID
        } else {
            // We're an OG Steam Controller
            D0G_BLE2_PID
        };
        self.product_id.store(product_id, Ordering::SeqCst);

        Ok(product_id)
    }

    fn get_serial_number<'a>(&self, _env: Env<'a>) -> JniResult<'a, String> {
        // This will be read later via feature report by Steam
        Ok("12345".into())
    }

    fn get_version(&self) -> jint {
        0
    }

    fn get_manufacturer_name<'a>(&self, _env: Env<'a>) -> JniResult<'a, String> {
        Ok("Valve Corporation".into())
    }

    fn get_product_name<'a>(&self, _env: Env<'a>) -> JniResult<'a, String> {
        Ok("Steam Controller".into())
    }

    fn get_device<'a>(&self, _env: Env<'a>) -> Option<Local<'a>> {
        None
    }

    fn open<'a>(&self, _env: Env<'a>) -> JniResult<'a, bool> {
        Ok(true)
    }

    fn write_report<'a>(&self, env: Env<'a>, report: &[u8], feature: bool) -> JniResult<'a, jint> {
        if !self.is_registered() {
            log(
                LogPriority::Error,
                "Attempted writeReport before Steam Controller is registered!",
            );
            if self.is_connected.load(Ordering::SeqCst) {
                self.probe_service(env)?;
            }
            return Ok(-1);
        }

        if feature {
            // We need to skip the first byte, as that doesn't go over the air
            // (Note (upstream): and the last one: copyOfRange()'s end is
            // exclusive)
            let actual_report = copy_of_range(env, report, 1, report.len() as isize - 1)?;
            // (Log.v("writeFeatureReport " + ...), commented out)
            self.write_characteristic(env, REPORT_CHARACTERISTIC, actual_report)?;
            return Ok(report.len() as jint);
        } else {
            // If we're an original-recipe Steam Controller we just write to the characteristic directly.
            if self.get_product_id(env)? == D0G_BLE2_PID {
                // (Log.v("writeOutputReport " + ...), commented out)
                self.write_characteristic(env, REPORT_CHARACTERISTIC, report.to_vec())?;
                return Ok(report.len() as jint);
            }

            // If we're a Triton, we need to find the correct report characteristic.
            if !report.is_empty() {
                let report_id = jint::from(report[0]);
                let targeted_uuid = match lock(&self.output_report_chars).get(&report_id) {
                    Some(chr) => Some(uuid_of(env, chr.obj())?),
                    None => None,
                };
                if let Some(targeted_uuid) = targeted_uuid {
                    // (Note (upstream): without the last byte too, as above)
                    let actual_report = copy_of_range(env, report, 1, report.len() as isize - 1)?;
                    // (Log.v("writeOutputReport 0x" + ...), commented out)
                    self.write_characteristic(env, &targeted_uuid, actual_report)?;
                    return Ok(report.len() as jint);
                } else {
                    log(
                        LogPriority::Warn,
                        &format!(
                            "Got report write request for unknown report type 0x{report_id:x}"
                        ),
                    );
                }
            }
        }

        Ok(-1)
    }

    fn read_report<'a>(&self, env: Env<'a>, _report: &[u8], feature: bool) -> JniResult<'a, bool> {
        if !self.is_registered() {
            log(
                LogPriority::Error,
                "Attempted readReport before Steam Controller is registered!",
            );
            if self.is_connected.load(Ordering::SeqCst) {
                self.probe_service(env)?;
            }
            return Ok(false);
        }

        if feature {
            self.read_characteristic(env, REPORT_CHARACTERISTIC)?;
            Ok(true)
        } else {
            // Not implemented
            Ok(false)
        }
    }

    fn close<'a>(&self, _env: Env<'a>) -> JniResult<'a, ()> {
        Ok(())
    }

    fn set_frozen<'a>(&self, _env: Env<'a>, frozen: bool) -> JniResult<'a, ()> {
        self.frozen.store(frozen, Ordering::SeqCst);
        Ok(())
    }

    fn shutdown<'a>(&self, env: Env<'a>) -> JniResult<'a, ()> {
        self.close(env)?;

        let g = lock(&self.gatt).take();
        if let Some(g) = g {
            env.call_method(g.obj(), c"disconnect", c"()V", &[])?;
            env.call_method(g.obj(), c"close", c"()V", &[])?;
        }
        *lock(&self.manager) = None;
        self.is_registered.store(false, Ordering::SeqCst);
        self.is_connected.store(false, Ordering::SeqCst);
        lock(&self.operations).queue.clear();
        // (its callbacks have no device from now on)
        lock(&DEVICES).retain(|device| !std::ptr::eq(device.as_ptr(), self));
        Ok(())
    }
}

// The native overrides of the DEX file's class.

/// A callback's body, on the device of its object (none for a device shut
/// down); an exception is left pending, for the framework.
fn callback<'a>(
    env: *mut JNIEnv,
    this: jobject,
    body: impl FnOnce(Env<'a>, &HidDeviceBleSteamController) -> JniResult<'a, ()>,
) {
    // SAFETY: a native method's environment and object.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    if let Some(device) = device_of(env, this) {
        if let Err(thrown) = body(env, &device) {
            // (pending, for the framework)
            drop(thrown);
        }
    }
}

pub(crate) extern "system" fn on_connection_state_change(
    env: *mut JNIEnv,
    this: jobject,
    _g: jobject,
    status: jint,
    new_state: jint,
) {
    callback(env, this, |env, device| {
        device.on_connection_state_change(env, status, new_state)
    });
}

pub(crate) extern "system" fn on_services_discovered(
    env: *mut JNIEnv,
    this: jobject,
    gatt: jobject,
    status: jint,
) {
    // SAFETY: a native method's argument.
    let gatt = unsafe { Obj::from_raw(gatt) };
    callback(env, this, |env, device| {
        device.on_services_discovered(env, gatt, status)
    });
}

pub(crate) extern "system" fn on_characteristic_read(
    env: *mut JNIEnv,
    this: jobject,
    _gatt: jobject,
    characteristic: jobject,
    _status: jint,
) {
    // SAFETY: a native method's argument.
    let characteristic = unsafe { Obj::from_raw(characteristic) };
    callback(env, this, |env, device| {
        device.on_characteristic_read(env, characteristic)
    });
}

pub(crate) extern "system" fn on_characteristic_write(
    env: *mut JNIEnv,
    this: jobject,
    _gatt: jobject,
    characteristic: jobject,
    _status: jint,
) {
    // SAFETY: a native method's argument.
    let characteristic = unsafe { Obj::from_raw(characteristic) };
    callback(env, this, |env, device| {
        device.on_characteristic_write(env, characteristic)
    });
}

pub(crate) extern "system" fn on_characteristic_changed(
    env: *mut JNIEnv,
    this: jobject,
    _gatt: jobject,
    characteristic: jobject,
) {
    // SAFETY: a native method's argument.
    let characteristic = unsafe { Obj::from_raw(characteristic) };
    callback(env, this, |env, device| {
        device.on_characteristic_changed(env, characteristic)
    });
}

/// Translation of `onDescriptorRead()`.
pub(crate) extern "system" fn on_descriptor_read(
    _env: *mut JNIEnv,
    _this: jobject,
    _gatt: jobject,
    _descriptor: jobject,
    _status: jint,
) {
    // (Log.v("onDescriptorRead status=" + status), commented out)
}

pub(crate) extern "system" fn on_descriptor_write(
    env: *mut JNIEnv,
    this: jobject,
    _gatt: jobject,
    descriptor: jobject,
    _status: jint,
) {
    // SAFETY: a native method's argument.
    let descriptor = unsafe { Obj::from_raw(descriptor) };
    callback(env, this, |env, device| {
        device.on_descriptor_write(env, descriptor)
    });
}

/// Translation of `onReliableWriteCompleted()`.
pub(crate) extern "system" fn on_reliable_write_completed(
    _env: *mut JNIEnv,
    _this: jobject,
    _gatt: jobject,
    _status: jint,
) {
    // (Log.v("onReliableWriteCompleted status=" + status), commented out)
}

/// Translation of `onReadRemoteRssi()`.
pub(crate) extern "system" fn on_read_remote_rssi(
    _env: *mut JNIEnv,
    _this: jobject,
    _gatt: jobject,
    _rssi: jint,
    _status: jint,
) {
    // (Log.v("onReadRemoteRssi status=" + status), commented out)
}

/// Translation of `onMtuChanged()`.
pub(crate) extern "system" fn on_mtu_changed(
    _env: *mut JNIEnv,
    _this: jobject,
    _gatt: jobject,
    _mtu: jint,
    _status: jint,
) {
    // (Log.v("onMtuChanged status=" + status), commented out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_ids_of_uuids() {
        // The characteristics of the service: report IDs below 0x80 (the
        // input and report characteristics)...
        assert_eq!(
            report_id_of_uuid(INPUT_CHARACTERISTIC_D0G),
            Some(Ok(0x33 - 0x35))
        );
        assert_eq!(report_id_of_uuid(REPORT_CHARACTERISTIC), Some(Ok(-1)));
        assert_eq!(
            report_id_of_uuid(INPUT_CHARACTERISTIC_TRITON_0X45),
            Some(Ok(0x45))
        );
        // ...and a Triton's output reports.
        assert_eq!(
            report_id_of_uuid("100f6cb5-1735-4313-b402-38567131e5f3"),
            Some(Ok(0x80))
        );
        assert_eq!(
            report_id_of_uuid("100F6CB7-1735-4313-B402-38567131E5F3"),
            Some(Ok(0x82))
        );
        // Found anywhere.
        assert_eq!(
            report_id_of_uuid("00002902-100f-6cb6-8000-100f6cc000fb"),
            Some(Ok(0xc0 - 0x35))
        );
        // Not hexadecimal.
        assert!(matches!(report_id_of_uuid("100f6czz"), Some(Err(_))));
        // None.
        assert_eq!(
            report_id_of_uuid("00002902-0000-1000-8000-00805f9b34fb"),
            None
        );
        assert_eq!(report_id_of_uuid("100f6c"), None);
    }
}
