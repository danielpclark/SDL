// Rust translation of src/hidapi/ios/hid.m (and SDL_hidapi_ios.h) from
// Simple DirectMedia Layer.
// Copyright (C) 2021 Valve Corporation
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The iOS and tvOS backend: Valve's Steam Controllers over Bluetooth LE,
//! through CoreBluetooth (on those systems apps can't reach other HID
//! devices).
//!
//! The Bluetooth manager is a singleton `HIDBLEManager` (a
//! `CBCentralManagerDelegate`, declared at run time under upstream's name),
//! made on first use. It only starts Bluetooth (its `CBCentralManager`,
//! which asks the user for permission, and needs the app's Info.plist to
//! say why with `NSBluetoothAlwaysUsageDescription`) when
//! `SDL_HINT_JOYSTICK_HIDAPI_STEAM` is set then; otherwise there are no
//! devices. With Bluetooth on, it scans for 20 seconds when Bluetooth is
//! powered on and when the app becomes active (with `SDL_hid_ble_scan()`
//! scanning in between), connects to the peripherals whose names start
//! with "Steam", and drops them when the app resigns active. Each one is a
//! `HIDBLEDevice` (a `CBPeripheralDelegate`), which finds the controller's
//! service and characteristics and keeps the input reports it is notified
//! of in a ring buffer ([`super::ble`]). A device is enumerated (and can
//! be opened) once it is connected and has sent a report, which tells
//! that the user accepted the pairing. Paths are the peripherals'
//! identifiers (UUID strings).
//!
//! CoreBluetooth is loaded at run time (Note: upstream's CMake build links
//! it; loading it keeps a program that never starts Bluetooth free of it,
//! upstream's reason for `SDL_HIDAPI_DISABLED` on iOS, and as a weakly
//! linked framework's would, its classes are nil without it). The device
//! map and the devices' state are locked (Note: upstream reads and writes
//! them from Bluetooth's queue and the caller's thread without a lock).

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{
    AtomicBool, AtomicI32, AtomicPtr, AtomicU16, AtomicU64, AtomicU8, Ordering,
};
use std::sync::{Mutex, MutexGuard, Once, OnceLock};

use super::ble::{self, RingBuffer, D0G_BLE2_PID, VALVE_USB_VID};
use super::{BusType, DeviceInfo};
use crate::core::apple::block::{
    dispatch_get_global_queue, dispatch_queue_create, dispatch_time, main_queue, DispatchFunction,
    DispatchQueue, DispatchTime, DISPATCH_QUEUE_SERIAL, DISPATCH_TIME_NOW,
};
use crate::core::apple::foundation::{
    kCFRunLoopDefaultMode, ns_string, string_from_ns, CFRunLoopRunInMode,
    K_CF_RUN_LOOP_RUN_FINISHED, K_CF_RUN_LOOP_RUN_TIMED_OUT,
};
use crate::core::apple::objc::{
    autoreleasepool, class, imp, ivar_ptr, msg_send, msg_send_super, sel, Bool, Class,
    ClassBuilder, DeclaredClass, Id, NSInteger, NSUInteger, Sel, StrongPtr, NIL,
};
use crate::error::{Error, Result};
use crate::hints;
use crate::loadso::SharedObject;
use crate::log::ns_log;

#[link(name = "System", kind = "dylib")]
unsafe extern "C" {
    fn dispatch_after_f(
        when: DispatchTime,
        queue: DispatchQueue,
        context: *mut c_void,
        work: DispatchFunction,
    );
    fn dispatch_set_target_queue(object: *mut c_void, queue: DispatchQueue);
    fn mach_approximate_time() -> u64;
    fn mach_timebase_info(info: *mut MachTimebaseInfo) -> i32;
}

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    static UIApplicationWillResignActiveNotification: Id;
    static UIApplicationDidBecomeActiveNotification: Id;
}

/// `mach_timebase_info_data_t`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct MachTimebaseInfo {
    numer: u32,
    denom: u32,
}

/// `NSEC_PER_SEC`
const NSEC_PER_SEC: u64 = 1_000_000_000;

/// `DISPATCH_QUEUE_PRIORITY_HIGH`
const DISPATCH_QUEUE_PRIORITY_HIGH: isize = 2;

/// `CBManagerState` values.
const CB_MANAGER_STATE_UNKNOWN: NSInteger = 0;
const CB_MANAGER_STATE_RESETTING: NSInteger = 1;
const CB_MANAGER_STATE_UNSUPPORTED: NSInteger = 2;
const CB_MANAGER_STATE_UNAUTHORIZED: NSInteger = 3;
const CB_MANAGER_STATE_POWERED_OFF: NSInteger = 4;
const CB_MANAGER_STATE_POWERED_ON: NSInteger = 5;

/// `CBPeripheralStateConnected`
const CB_PERIPHERAL_STATE_CONNECTED: NSInteger = 2;

/// `CBCharacteristicWriteWithResponse`
const CB_CHARACTERISTIC_WRITE_WITH_RESPONSE: NSInteger = 0;

/// `NSMapTableStrongMemory`, `NSMapTableWeakMemory`
const NS_MAP_TABLE_STRONG_MEMORY: NSUInteger = 0;
const NS_MAP_TABLE_WEAK_MEMORY: NSUInteger = 5;

/// `BLEDeviceWaitState`
const BLE_DEVICE_WAIT_STATE_NONE: u8 = 0;
const BLE_DEVICE_WAIT_STATE_WAITING: u8 = 1;
const BLE_DEVICE_WAIT_STATE_COMPLETE: u8 = 2;
const BLE_DEVICE_WAIT_STATE_ERROR: u8 = 3;

// - CoreBluetooth -

const CORE_BLUETOOTH_PATH: &str =
    "/System/Library/Frameworks/CoreBluetooth.framework/CoreBluetooth";

/// CoreBluetooth, loaded for the life of the process (`None` without it).
fn core_bluetooth() -> Option<&'static SharedObject> {
    static LIB: OnceLock<Option<SharedObject>> = OnceLock::new();
    LIB.get_or_init(|| SharedObject::load(CORE_BLUETOOTH_PATH).ok())
        .as_ref()
}

/// `CBAdvertisementDataLocalNameKey` (nil without CoreBluetooth).
fn advertisement_data_local_name_key() -> Id {
    match core_bluetooth().map(|lib| lib.symbol("CBAdvertisementDataLocalNameKey")) {
        // SAFETY: the symbol is an `NSString *const`.
        Some(Ok(p)) => unsafe { *(p.as_ptr() as *const Id) },
        _ => NIL,
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// `NSLog(@"%@", text)`.
fn log(text: &str) {
    ns_log("", text);
}

/// What `%@` prints of an object (its `description`, `(null)` for nil).
fn describe(obj: Id) -> String {
    if obj.is_null() {
        return "(null)".into();
    }
    // SAFETY: a valid object; its description is an NSString.
    unsafe {
        let description: Id = msg_send![obj, description];
        string_from_ns(description).unwrap_or_default()
    }
}

/// An object's `UUID.UUIDString` (a CBAttribute's), empty for nil.
fn uuid_string(attribute: Id) -> String {
    // SAFETY: nil or a CBAttribute, whose UUID is a CBUUID.
    unsafe {
        let uuid: Id = msg_send![attribute, UUID];
        let s: Id = msg_send![uuid, UUIDString];
        string_from_ns(s).unwrap_or_default()
    }
}

/// A peripheral's `identifier.UUIDString`.
fn identifier_string(peripheral: Id) -> String {
    // SAFETY: nil or a CBPeripheral, whose identifier is an NSUUID.
    unsafe {
        let identifier: Id = msg_send![peripheral, identifier];
        let s: Id = msg_send![identifier, UUIDString];
        string_from_ns(s).unwrap_or_default()
    }
}

/// `[NSData dataWithBytes:length:]` (autoreleased).
fn ns_data(bytes: &[u8]) -> Id {
    // SAFETY: the bytes are valid for their length (copied).
    unsafe {
        msg_send![class!(NSData), dataWithBytes: bytes.as_ptr() as *const c_void
                                         length: bytes.len() as NSUInteger]
    }
}

/// The bytes of an NSData (empty for nil).
fn data_bytes(data: Id) -> Vec<u8> {
    // SAFETY: nil or an NSData, whose bytes are valid for its length.
    unsafe {
        let len: NSUInteger = msg_send![data, length];
        let p: *const u8 = msg_send![data, bytes];
        if len == 0 || p.is_null() {
            return Vec::new();
        }
        std::slice::from_raw_parts(p, len).to_vec()
    }
}

/// The objects of an enumerator (`for (id x in ...)`), retained.
///
/// # Safety
///
/// `enumerator` must be nil or an NSEnumerator.
unsafe fn enumerate_objects(enumerator: Id) -> Vec<StrongPtr> {
    let mut objects = Vec::new();
    loop {
        // SAFETY: the caller's contract; the objects are retained.
        let obj: Id = unsafe { msg_send![enumerator, nextObject] };
        if obj.is_null() {
            break;
        }
        // SAFETY: as above.
        objects.push(unsafe { StrongPtr::retain(obj) });
    }
    objects
}

/// The objects of an NSArray, retained.
///
/// # Safety
///
/// `array` must be nil or an NSArray.
unsafe fn array_objects(array: Id) -> Vec<StrongPtr> {
    // SAFETY: the caller's contract.
    unsafe { enumerate_objects(msg_send![array, objectEnumerator]) }
}

/// Translation of `process_pending_events()`: Core Bluetooth devices
/// calling back on event boundaries of their run-loops. so annoying.
fn process_pending_events() {
    loop {
        // SAFETY: runs this thread's run loop briefly.
        let res = unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.001, 0) };
        if res == K_CF_RUN_LOOP_RUN_FINISHED || res == K_CF_RUN_LOOP_RUN_TIMED_OUT {
            break;
        }
    }
}

/// `dispatch_after(dispatch_time(DISPATCH_TIME_NOW, ns), queue, ^{ f })`.
fn run_after(ns: i64, queue: DispatchQueue, f: impl FnOnce() + Send + 'static) {
    type Work = Box<dyn FnOnce() + Send>;
    unsafe extern "C" fn work(ctx: *mut c_void) {
        // SAFETY: the box leaked below, reclaimed once.
        let f = unsafe { Box::from_raw(ctx as *mut Work) };
        f();
    }
    let ctx: Box<Work> = Box::new(Box::new(f));
    // SAFETY: a valid queue; `work` takes the context back.
    unsafe {
        dispatch_after_f(
            dispatch_time(DISPATCH_TIME_NOW, ns),
            queue,
            Box::into_raw(ctx) as *mut c_void,
            work,
        );
    }
}

/// `mach_timebase_info()`, asked once.
fn timebase() -> MachTimebaseInfo {
    static TIMEBASE: OnceLock<MachTimebaseInfo> = OnceLock::new();
    *TIMEBASE.get_or_init(|| {
        let mut info = MachTimebaseInfo::default();
        // SAFETY: a writable structure.
        unsafe { mach_timebase_info(&mut info) };
        info
    })
}

// - HIDBLEDevice -

/// The connection's feature report characteristic, its input report
/// characteristic, and the output report characteristics by report ID.
#[derive(Default)]
struct Characteristics {
    /// `bleCharacteristicInput`
    input: StrongPtr,
    /// `bleCharacteristicReport`
    report: StrongPtr,
    /// `_outputReports`
    output_reports: HashMap<i32, StrongPtr>,
}

/// The state of an `HIDBLEDevice` (its instance variables and
/// properties), in its `_state` instance variable.
struct DeviceState {
    /// `bleSteamController` (set once, by `initWithPeripheral:`)
    peripheral: StrongPtr,
    pid: AtomicU16,
    report_id: AtomicU8,
    report_length: AtomicI32,
    connected: AtomicBool,
    ready: AtomicBool,
    characteristics: Mutex<Characteristics>,
    /// `_inputReports`
    input_reports: Mutex<Option<RingBuffer>>,
    /// `_featureReport`
    feature_report: Mutex<Vec<u8>>,
    /// `_waitStateForReadFeatureReport`
    wait_state_for_read_feature_report: AtomicU8,
    /// `_waitStateForWriteFeatureReport`
    wait_state_for_write_feature_report: AtomicU8,
}

static HID_BLE_DEVICE: DeclaredClass = DeclaredClass::new();

/// The `HIDBLEDevice` class (`@interface HIDBLEDevice : NSObject
/// <CBPeripheralDelegate>`). The methods upstream's manager and hid_*
/// functions call are the functions below.
fn device_class() -> Class {
    HID_BLE_DEVICE.get(|| {
        let mut b = ClassBuilder::new(c"HIDBLEDevice", class!(NSObject)).expect("HIDBLEDevice");
        b.add_ivar::<*const DeviceState>(c"_state", c"^v");
        b.add_protocol(c"CBPeripheralDelegate");
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_method(
                sel!(dealloc),
                imp!(device_dealloc as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(peripheral:didDiscoverServices:),
                imp!(device_did_discover_services as extern "C" fn(Id, Sel, Id, Id)),
                c"v@:@@",
            );
            b.add_method(
                sel!(peripheral:didDiscoverDescriptorsForCharacteristic:error:),
                imp!(device_did_discover_descriptors as extern "C" fn(Id, Sel, Id, Id, Id)),
                c"v@:@@@",
            );
            b.add_method(
                sel!(peripheral:didDiscoverCharacteristicsForService:error:),
                imp!(device_did_discover_characteristics as extern "C" fn(Id, Sel, Id, Id, Id)),
                c"v@:@@@",
            );
            b.add_method(
                sel!(peripheral:didUpdateValueForCharacteristic:error:),
                imp!(device_did_update_value as extern "C" fn(Id, Sel, Id, Id, Id)),
                c"v@:@@@",
            );
            b.add_method(
                sel!(peripheral:didWriteValueForCharacteristic:error:),
                imp!(device_did_write_value as extern "C" fn(Id, Sel, Id, Id, Id)),
                c"v@:@@@",
            );
            b.add_method(
                sel!(peripheral:didUpdateNotificationStateForCharacteristic:error:),
                imp!(device_did_update_notification_state as extern "C" fn(Id, Sel, Id, Id, Id)),
                c"v@:@@@",
            );
        }
        b.register()
    })
}

/// The state of a device.
///
/// # Safety
///
/// `device` must be an `HIDBLEDevice` made by [`new_device`], alive for
/// `'a`.
unsafe fn device_state<'a>(device: Id) -> &'a DeviceState {
    // SAFETY: the caller's contract; the state lives as long as the object.
    unsafe { &**ivar_ptr::<*const DeviceState>(device, device_class(), c"_state") }
}

/// Translation of `-[HIDBLEDevice initWithPeripheral:]` (with `+new`).
fn new_device(peripheral: Id) -> StrongPtr {
    // SAFETY: a new object of our class, given its state; a peripheral's
    // delegate is weak, the map keeps the device.
    unsafe {
        let device = StrongPtr::from_owned(msg_send![device_class(), new]);
        let state = Box::new(DeviceState {
            peripheral: StrongPtr::retain(peripheral),
            pid: AtomicU16::new(0),
            report_id: AtomicU8::new(0),
            report_length: AtomicI32::new(0),
            connected: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            characteristics: Mutex::new(Characteristics::default()),
            input_reports: Mutex::new(None),
            feature_report: Mutex::new(Vec::new()),
            wait_state_for_read_feature_report: AtomicU8::new(BLE_DEVICE_WAIT_STATE_NONE),
            wait_state_for_write_feature_report: AtomicU8::new(BLE_DEVICE_WAIT_STATE_NONE),
        });
        *ivar_ptr::<*const DeviceState>(device.get(), device_class(), c"_state") =
            Box::into_raw(state);
        if !peripheral.is_null() {
            let () = msg_send![peripheral, setDelegate: device.get()];
        }
        device
    }
}

extern "C" fn device_dealloc(this: Id, _sel: Sel) {
    // SAFETY: our device being freed: its state is reclaimed once, then
    // NSObject's dealloc runs.
    unsafe {
        let slot = ivar_ptr::<*const DeviceState>(this, device_class(), c"_state");
        if !(*slot).is_null() {
            drop(Box::from_raw(*slot as *mut DeviceState));
            *slot = std::ptr::null();
        }
        let () = msg_send_super![this, device_class(), dealloc];
    }
}

impl DeviceState {
    /// Translation of `-[HIDBLEDevice onDisconnect]`.
    fn on_disconnect(&self) {
        self.set_connected(false);
        self.ready.store(false, Ordering::SeqCst);

        *lock(&self.input_reports) = None;
    }

    /// Translation of `-[HIDBLEDevice setConnected:]` (the `connected`
    /// property's setter).
    fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::SeqCst);
        if connected {
            // SAFETY: the device's peripheral.
            let () = unsafe { msg_send![self.peripheral, discoverServices: NIL] };
        } else {
            log("Disconnected");
        }
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    /// `bleCharacteristicInput`, retained.
    fn input_characteristic(&self) -> StrongPtr {
        lock(&self.characteristics).input.clone()
    }

    /// `[bleSteamController setNotifyValue:value forCharacteristic:characteristic]`.
    fn set_notify_value(&self, value: bool, characteristic: &StrongPtr) {
        // SAFETY: the peripheral and one of its characteristics (or nil).
        let () = unsafe {
            msg_send![self.peripheral, setNotifyValue: Bool::from(value)
                                     forCharacteristic: characteristic.get()]
        };
    }

    /// `[bleSteamController writeValue:data forCharacteristic:characteristic
    /// type:CBCharacteristicWriteWithResponse]`.
    fn write_value(&self, data: &[u8], characteristic: Id) {
        autoreleasepool(|| {
            // SAFETY: the peripheral, an NSData and one of its
            // characteristics.
            let () = unsafe {
                msg_send![self.peripheral, writeValue: ns_data(data)
                                    forCharacteristic: characteristic
                                                 type: CB_CHARACTERISTIC_WRITE_WITH_RESPONSE]
            };
        });
    }

    /// Translation of `-[HIDBLEDevice read_input_report:]`.
    fn read_input_report(&self, dst: &mut [u8]) -> usize {
        ble::read_input_report(
            lock(&self.input_reports).as_mut(),
            self.report_id.load(Ordering::SeqCst),
            dst,
        )
    }

    /// Translation of `-[HIDBLEDevice send_report:length:]`.
    fn send_report(&self, data: &[u8]) -> Result<usize> {
        if self.pid.load(Ordering::SeqCst) == D0G_BLE2_PID {
            let report = lock(&self.characteristics).report.clone();
            self.write_value(data, report.get());
            return Ok(data.len());
        }

        // We need to look up the correct characteristic for this output report
        if let Some(&id) = data.first() {
            let characteristic = lock(&self.characteristics)
                .output_reports
                .get(&i32::from(id))
                .cloned();
            if let Some(characteristic) = characteristic {
                self.write_value(&data[1..], characteristic.get());
                return Ok(data.len());
            }
        }
        // Note: upstream returns -1 without an error message.
        Err(Error::new("No characteristic for the output report"))
    }

    /// Translation of `-[HIDBLEDevice send_feature_report:length:]`.
    fn send_feature_report(&self, data: &[u8]) -> usize {
        // (FEATURE_REPORT_LOGGING, off upstream, would NSLog the report)

        // fire-and-forget - we are going to not wait for the response here because all Steam Controller BLE send_feature_report's are ignored,
        //  except errors.
        let report = lock(&self.characteristics).report.clone();
        self.write_value(&ble::feature_report_segment(data), report.get());

        // pretend we received a result anybody cares about
        data.len()

        // (upstream's `#else` branch, "technically the correct
        // send_feature_report logic if you want to make sure it gets
        // through and is acknowledged or errors out", waits for
        // `_waitStateForWriteFeatureReport` as `get_feature_report` does;
        // it isn't compiled)
    }

    /// Translation of `-[HIDBLEDevice get_feature_report:into:length:]`:
    /// the bytes of `buffer` filled, -1 for an error.
    fn get_feature_report(&self, buffer: &mut [u8]) -> isize {
        self.wait_state_for_read_feature_report
            .store(BLE_DEVICE_WAIT_STATE_WAITING, Ordering::SeqCst);
        let report = lock(&self.characteristics).report.clone();
        // SAFETY: the peripheral and its characteristic (or nil).
        let () = unsafe { msg_send![self.peripheral, readValueForCharacteristic: report.get()] };

        while self.is_connected()
            && self
                .wait_state_for_read_feature_report
                .load(Ordering::SeqCst)
                == BLE_DEVICE_WAIT_STATE_WAITING
        {
            process_pending_events();
        }

        if !self.is_connected()
            || self
                .wait_state_for_read_feature_report
                .load(Ordering::SeqCst)
                == BLE_DEVICE_WAIT_STATE_ERROR
        {
            self.wait_state_for_read_feature_report
                .store(BLE_DEVICE_WAIT_STATE_NONE, Ordering::SeqCst);
            return -1;
        }

        let amount = ble::copy_feature_report(&lock(&self.feature_report), buffer);

        self.wait_state_for_read_feature_report
            .store(BLE_DEVICE_WAIT_STATE_NONE, Ordering::SeqCst);

        // (FEATURE_REPORT_LOGGING, off upstream, would NSLog the report)

        amount as isize
    }
}

// CBPeripheralDelegate Implementation

/// `-peripheral:didDiscoverServices:`
extern "C" fn device_did_discover_services(_this: Id, _sel: Sel, peripheral: Id, _error: Id) {
    // SAFETY: CoreBluetooth's peripheral and its services.
    unsafe {
        for service in array_objects(msg_send![peripheral, services]) {
            log(&format!("Found Service: {}", describe(service.get())));
            if ble::uuid_eq(&uuid_string(service.get()), ble::VALVE_SERVICE) {
                let () = msg_send![peripheral, discoverCharacteristics: NIL
                                                         forService: service.get()];
            }
        }
    }
}

/// `-peripheral:didDiscoverDescriptorsForCharacteristic:error:`
extern "C" fn device_did_discover_descriptors(
    _this: Id,
    _sel: Sel,
    _peripheral: Id,
    _characteristic: Id,
    _error: Id,
) {
    // nothing yet needed here, enable for logging
    // (upstream's disabled code NSLogs " - Descriptor '%@'" for each)
}

/// `-peripheral:didDiscoverCharacteristicsForService:error:`
extern "C" fn device_did_discover_characteristics(
    this: Id,
    _sel: Sel,
    _peripheral: Id,
    service: Id,
    _error: Id,
) {
    if !ble::uuid_eq(&uuid_string(service), ble::VALVE_SERVICE) {
        return;
    }
    // SAFETY: our device; CoreBluetooth's service and its characteristics.
    unsafe {
        let state = device_state(this);
        for characteristic in array_objects(msg_send![service, characteristics]) {
            log(&format!(
                "Found Characteristic {}",
                describe(characteristic.get())
            ));

            let uuid = uuid_string(characteristic.get());
            if let Some(input) = ble::input_characteristic(&uuid) {
                state.pid.store(input.pid, Ordering::SeqCst);
                state.report_id.store(input.report_id, Ordering::SeqCst);
                state
                    .report_length
                    .store(input.report_length, Ordering::SeqCst);
                lock(&state.characteristics).input = characteristic;
            } else if ble::uuid_eq(&uuid, ble::VALVE_REPORT_CHAR) {
                lock(&state.characteristics).report = characteristic.clone();
                let () = msg_send![state.peripheral,
                                   discoverDescriptorsForCharacteristic: characteristic.get()];
            } else if let Some(report_id) = ble::output_report_id(&uuid) {
                lock(&state.characteristics)
                    .output_reports
                    .insert(report_id, characteristic);
            }
        }
    }
}

/// `s_ticksLastOverflowReport`
static TICKS_LAST_OVERFLOW_REPORT: AtomicU64 = AtomicU64::new(0);

/// `-peripheral:didUpdateValueForCharacteristic:error:`
extern "C" fn device_did_update_value(
    this: Id,
    _sel: Sel,
    _peripheral: Id,
    characteristic: Id,
    error: Id,
) {
    // SAFETY: our device; CoreBluetooth's characteristic, its UUID and
    // value.
    unsafe {
        let state = device_state(this);

        // receiving an input report is the final indicator that the user accepted a pairing
        // request and that we successfully established notification. CoreBluetooth has no
        // notification of the pairing acknowledgement, which is a bad oversight.
        if !state.is_ready() {
            state.ready.store(true, Ordering::SeqCst);
            {
                let mut input_reports = lock(&state.input_reports);
                if input_reports.is_none() {
                    let length = state.report_length.load(Ordering::SeqCst);
                    *input_reports = Some(RingBuffer::new(usize::try_from(length).unwrap_or(0)));
                }
            }
            manager().pending_pairs.fetch_sub(1, Ordering::SeqCst);
        }

        let uuid: Id = msg_send![characteristic, UUID];
        let (input, report) = {
            let c = lock(&state.characteristics);
            (c.input.clone(), c.report.clone())
        };
        let input_uuid: Id = msg_send![input, UUID];
        let report_uuid: Id = msg_send![report, UUID];
        let is_input: Bool = msg_send![uuid, isEqual: input_uuid];
        let is_report: Bool = msg_send![uuid, isEqual: report_uuid];
        if is_input.is_true() {
            let data = data_bytes(msg_send![characteristic, value]);
            let written = {
                let mut input_reports = lock(&state.input_reports);
                if let Some(ring) = input_reports.as_ref() {
                    if data.len() != ring.elem_size() {
                        log(&format!(
                            "HIDBLE: incoming data is {} bytes should be exactly {}",
                            data.len(),
                            ring.elem_size()
                        ));
                    }
                }
                input_reports.as_mut().is_some_and(|ring| ring.write(&data))
            };
            if !written {
                let ticks_now = mach_approximate_time();
                // Note (upstream): ticks are compared with nanoseconds.
                if ticks_now.wrapping_sub(TICKS_LAST_OVERFLOW_REPORT.load(Ordering::Relaxed))
                    > (5 * NSEC_PER_SEC / 10)
                {
                    log("HIDBLE: input report buffer overflow");
                    TICKS_LAST_OVERFLOW_REPORT.store(ticks_now, Ordering::Relaxed);
                }
            }
        } else if is_report.is_true() {
            if !error.is_null() {
                log(&format!(
                    "HIDBLE: get_feature_report error: {}",
                    describe(error)
                ));
                state
                    .wait_state_for_read_feature_report
                    .store(BLE_DEVICE_WAIT_STATE_ERROR, Ordering::SeqCst);
            } else {
                *lock(&state.feature_report) = data_bytes(msg_send![characteristic, value]);
                state
                    .wait_state_for_read_feature_report
                    .store(BLE_DEVICE_WAIT_STATE_COMPLETE, Ordering::SeqCst);
            }
        }
    }
}

/// `-peripheral:didWriteValueForCharacteristic:error:`
extern "C" fn device_did_write_value(
    this: Id,
    _sel: Sel,
    _peripheral: Id,
    characteristic: Id,
    error: Id,
) {
    if ble::uuid_eq(&uuid_string(characteristic), ble::VALVE_REPORT_CHAR) {
        // SAFETY: our device.
        let state = unsafe { device_state(this) };
        if !error.is_null() {
            log(&format!(
                "HIDBLE: write_feature_report error: {}",
                describe(error)
            ));
            state
                .wait_state_for_write_feature_report
                .store(BLE_DEVICE_WAIT_STATE_ERROR, Ordering::SeqCst);
        } else {
            state
                .wait_state_for_write_feature_report
                .store(BLE_DEVICE_WAIT_STATE_COMPLETE, Ordering::SeqCst);
        }
    }
}

/// `-peripheral:didUpdateNotificationStateForCharacteristic:error:`
extern "C" fn device_did_update_notification_state(
    _this: Id,
    _sel: Sel,
    _peripheral: Id,
    _characteristic: Id,
    _error: Id,
) {
    //NSLog( @"didUpdateNotificationStateForCharacteristic %@ (%@)", characteristic, error );
}

// - HIDBLEManager -

/// The singleton `HIDBLEManager` (`+[HIDBLEManager sharedInstance]`).
struct Manager {
    /// The `HIDBLEManager` object.
    object: Id,
    /// `centralManager` (unset unless the hint was set; set once made)
    central_manager: OnceLock<StrongPtr>,
    /// `bleSerialQueue` (null unless the hint was set)
    ble_serial_queue: AtomicPtr<c_void>,
    /// `deviceMap`: an `NSMapTable` from the peripherals (weak) to their
    /// `HIDBLEDevice`s.
    device_map: StrongPtr,
    /// `nPendingScans` (`@synchronized (self)` in `startScan:` and
    /// `stopScan`).
    pending_scans: Mutex<i32>,
    /// `nPendingPairs`
    pending_pairs: AtomicI32,
    /// The device map's lock.
    map_lock: Mutex<()>,
    /// `s_unLastUpdateTick`
    last_update_tick: AtomicU64,
}

// SAFETY: the objects are used as upstream uses them, from the caller's
// thread and Bluetooth's queue; the map is used under its lock.
unsafe impl Send for Manager {}
// SAFETY: as above.
unsafe impl Sync for Manager {}

static MANAGER: OnceLock<Manager> = OnceLock::new();

static HID_BLE_MANAGER: DeclaredClass = DeclaredClass::new();

/// The `HIDBLEManager` class (`@interface HIDBLEManager : NSObject
/// <CBCentralManagerDelegate>`), with `+sharedInstance`, its properties'
/// getters, the delegate methods and the notification handlers.
/// `startScan:`, `stopScan` and `updateConnectedSteamControllers:` are the
/// [`Manager`]'s.
fn manager_class() -> Class {
    HID_BLE_MANAGER.get(|| {
        let mut b = ClassBuilder::new(c"HIDBLEManager", class!(NSObject)).expect("HIDBLEManager");
        b.add_protocol(c"CBCentralManagerDelegate");
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_class_method(
                sel!(sharedInstance),
                imp!(manager_shared_instance as extern "C" fn(Id, Sel) -> Id),
                c"@#:",
            );
            b.add_method(
                sel!(nPendingScans),
                imp!(manager_n_pending_scans as extern "C" fn(Id, Sel) -> i32),
                c"i@:",
            );
            b.add_method(
                sel!(nPendingPairs),
                imp!(manager_n_pending_pairs as extern "C" fn(Id, Sel) -> i32),
                c"i@:",
            );
            b.add_method(
                sel!(centralManager),
                imp!(manager_central_manager as extern "C" fn(Id, Sel) -> Id),
                c"@@:",
            );
            b.add_method(
                sel!(deviceMap),
                imp!(manager_device_map as extern "C" fn(Id, Sel) -> Id),
                c"@@:",
            );
            b.add_method(
                sel!(bleSerialQueue),
                imp!(manager_ble_serial_queue as extern "C" fn(Id, Sel) -> Id),
                c"@@:",
            );
            b.add_method(
                sel!(appWillResignActiveNotification:),
                imp!(manager_app_will_resign_active as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(appDidBecomeActiveNotification:),
                imp!(manager_app_did_become_active as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(centralManagerDidUpdateState:),
                imp!(manager_did_update_state as extern "C" fn(Id, Sel, Id)),
                c"v@:@",
            );
            b.add_method(
                sel!(centralManager:didConnectPeripheral:),
                imp!(manager_did_connect_peripheral as extern "C" fn(Id, Sel, Id, Id)),
                c"v@:@@",
            );
            b.add_method(
                sel!(centralManager:didFailToConnectPeripheral:error:),
                imp!(manager_did_fail_to_connect_peripheral as extern "C" fn(Id, Sel, Id, Id, Id)),
                c"v@:@@@",
            );
            b.add_method(
                sel!(centralManager:didDiscoverPeripheral:advertisementData:RSSI:),
                imp!(manager_did_discover_peripheral as extern "C" fn(Id, Sel, Id, Id, Id, Id)),
                c"v@:@@@@",
            );
            b.add_method(
                sel!(centralManager:didDisconnectPeripheral:error:),
                imp!(manager_did_disconnect_peripheral as extern "C" fn(Id, Sel, Id, Id, Id)),
                c"v@:@@@",
            );
        }
        b.register()
    })
}

/// Translation of `+[HIDBLEManager sharedInstance]` (singleton class -
/// access using HIDBLEManager.sharedInstance): the manager, made on first
/// use, which then starts Bluetooth if the hint says so.
fn shared_instance() -> &'static Manager {
    static BLUETOOTH: Once = Once::new();
    let manager = MANAGER.get_or_init(|| {
        autoreleasepool(|| {
            // (the delegate protocol is adopted when the class is declared)
            let _ = core_bluetooth();
            // SAFETY: a new object of our class (kept for the process'
            // life) and a new map table.
            unsafe {
                let object: Id = msg_send![manager_class(), new];
                let map: Id = msg_send![class!(NSMapTable), alloc];
                let device_map = StrongPtr::from_owned(msg_send![map,
                    initWithKeyOptions: NS_MAP_TABLE_WEAK_MEMORY
                          valueOptions: NS_MAP_TABLE_STRONG_MEMORY
                              capacity: 4 as NSUInteger]);
                Manager {
                    object,
                    central_manager: OnceLock::new(),
                    ble_serial_queue: AtomicPtr::new(std::ptr::null_mut()),
                    device_map,
                    pending_scans: Mutex::new(0),
                    pending_pairs: AtomicI32::new(0),
                    map_lock: Mutex::new(()),
                    last_update_tick: AtomicU64::new(0),
                }
            }
        })
    });
    BLUETOOTH.call_once(|| manager.start_bluetooth());
    manager
}

/// The manager [`shared_instance`] made (for the methods of its object,
/// and the delegate methods, which may run while it starts Bluetooth).
fn manager() -> &'static Manager {
    MANAGER.get().expect("the HIDBLEManager")
}

extern "C" fn manager_shared_instance(_cls: Id, _sel: Sel) -> Id {
    shared_instance().object
}

extern "C" fn manager_n_pending_scans(_this: Id, _sel: Sel) -> i32 {
    *lock(&manager().pending_scans)
}

extern "C" fn manager_n_pending_pairs(_this: Id, _sel: Sel) -> i32 {
    manager().pending_pairs.load(Ordering::SeqCst)
}

extern "C" fn manager_central_manager(_this: Id, _sel: Sel) -> Id {
    manager().central()
}

extern "C" fn manager_device_map(_this: Id, _sel: Sel) -> Id {
    manager().device_map.get()
}

extern "C" fn manager_ble_serial_queue(_this: Id, _sel: Sel) -> Id {
    manager().ble_serial_queue.load(Ordering::SeqCst) as Id
}

/// Translation of `-[HIDBLEManager appWillResignActiveNotification:]`
/// (called for NSNotification UIApplicationWillResignActiveNotification).
extern "C" fn manager_app_will_resign_active(_this: Id, _sel: Sel, _note: Id) {
    let manager = manager();
    // we'll get resign-active notification if pairing is happening.
    if manager.pending_pairs.load(Ordering::SeqCst) > 0 {
        return;
    }

    let _map = lock(&manager.map_lock);
    // SAFETY: the map table, its peripherals and their devices.
    unsafe {
        for peripheral in enumerate_objects(msg_send![manager.device_map, keyEnumerator]) {
            let steam_controller: Id =
                msg_send![manager.device_map, objectForKey: peripheral.get()];
            if !steam_controller.is_null() {
                device_state(steam_controller).on_disconnect();
                let () = msg_send![manager.central(),
                                   cancelPeripheralConnection: peripheral.get()];
            }
        }
        let () = msg_send![manager.device_map, removeAllObjects];
    }
}

/// Translation of `-[HIDBLEManager appDidBecomeActiveNotification:]`
/// (called for NSNotification UIApplicationDidBecomeActiveNotification):
/// whenever the application comes back from being inactive, trigger a 20s
/// pairing scan and reconnect any devices that may have paired while we
/// were inactive.
extern "C" fn manager_app_did_become_active(_this: Id, _sel: Sel, _note: Id) {
    let manager = manager();
    manager.update_connected_steam_controllers(true);
    manager.start_scan(20);
}

impl Manager {
    /// The rest of `+[HIDBLEManager sharedInstance]`: Bluetooth, if the
    /// hint is set.
    fn start_bluetooth(&self) {
        // Bluetooth is currently only used for Steam Controllers, so check that hint
        // before initializing Bluetooth, which will prompt the user for permission.
        if !hints::get_bool(hints::JOYSTICK_HIDAPI_STEAM, false) {
            return;
        }
        autoreleasepool(|| {
            // SAFETY: the notification center's registration of the
            // object's methods, a new serial queue and a central manager
            // with the object as its delegate on it.
            unsafe {
                let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];
                let () = msg_send![center, addObserver: self.object
                                              selector: sel!(appWillResignActiveNotification:)
                                                  name: UIApplicationWillResignActiveNotification
                                                object: NIL];
                let () = msg_send![center, addObserver: self.object
                                              selector: sel!(appDidBecomeActiveNotification:)
                                                  name: UIApplicationDidBecomeActiveNotification
                                                object: NIL];

                // receive reports on a high-priority serial-queue. optionally put writes on the serial queue to avoid logical
                // race conditions talking to the controller from multiple threads, although BLE fragmentation/assembly means
                // that we can still screw this up.
                // most importantly we need to consume reports at a high priority to avoid the OS thinking we aren't really
                // listening to the BLE device, as iOS on slower devices may stop delivery of packets to the app WITHOUT ACTUALLY
                // DISCONNECTING FROM THE DEVICE if we don't react quickly enough to their delivery.
                // see also the error-handling states in the peripheral delegate to re-open the device if it gets closed
                let ble_serial_queue = dispatch_queue_create(
                    c"com.valvesoftware.steamcontroller.ble".as_ptr(),
                    DISPATCH_QUEUE_SERIAL,
                );
                dispatch_set_target_queue(
                    ble_serial_queue,
                    dispatch_get_global_queue(DISPATCH_QUEUE_PRIORITY_HIGH, 0),
                );
                self.ble_serial_queue
                    .store(ble_serial_queue, Ordering::SeqCst);

                // creating a CBCentralManager will always trigger a future centralManagerDidUpdateState:
                // where any scanning gets started or connecting to existing peripherals happens, it's never already in a
                // powered-on state for a newly launched application.
                // (until it is made, the manager's centralManager is nil,
                // as upstream's property is)
                let central: Id = msg_send![class!(CBCentralManager), alloc];
                let central = StrongPtr::from_owned(
                    msg_send![central, initWithDelegate: self.object queue: ble_serial_queue],
                );
                let _ = self.central_manager.set(central);
            }
        });
    }

    /// `self.centralManager` (nil without Bluetooth).
    fn central(&self) -> Id {
        self.central_manager.get().map_or(NIL, StrongPtr::get)
    }

    /// `self.deviceMap.count`
    fn device_count(&self) -> i32 {
        let _map = lock(&self.map_lock);
        // SAFETY: the map table.
        let count: NSUInteger = unsafe { msg_send![self.device_map, count] };
        count as i32
    }

    /// The devices of the map (`[deviceMap objectEnumerator]`), retained.
    fn devices(&self) -> Vec<StrongPtr> {
        let _map = lock(&self.map_lock);
        // SAFETY: the map table.
        unsafe { enumerate_objects(msg_send![self.device_map, objectEnumerator]) }
    }

    /// `[deviceMap objectForKey:peripheral]`, retained.
    fn device_for(&self, peripheral: Id) -> StrongPtr {
        let _map = lock(&self.map_lock);
        // SAFETY: the map table and a peripheral.
        unsafe { StrongPtr::retain(msg_send![self.device_map, objectForKey: peripheral]) }
    }

    /// `[deviceMap setObject:device forKey:peripheral]`.
    fn set_device(&self, peripheral: Id, device: &StrongPtr) {
        let _map = lock(&self.map_lock);
        // SAFETY: the map table, a peripheral and its device.
        let () = unsafe { msg_send![self.device_map, setObject: device.get() forKey: peripheral] };
    }

    /// `[deviceMap removeObjectForKey:peripheral]`.
    fn remove_device(&self, peripheral: Id) {
        let _map = lock(&self.map_lock);
        // SAFETY: the map table and a peripheral.
        let () = unsafe { msg_send![self.device_map, removeObjectForKey: peripheral] };
    }

    /// `self.centralManager.state`
    fn central_state(&self) -> NSInteger {
        // SAFETY: the central manager.
        unsafe { msg_send![self.central(), state] }
    }

    /// A peripheral found: a new device for it, connected to. (From
    /// `updateConnectedSteamControllers:` and
    /// `centralManager:didDiscoverPeripheral:advertisementData:RSSI:`.)
    fn connect(&self, peripheral: Id) {
        self.pending_pairs.fetch_add(1, Ordering::SeqCst);
        let steam_controller = new_device(peripheral);
        self.set_device(peripheral, &steam_controller);
        // SAFETY: the central manager and a peripheral it found.
        let () = unsafe { msg_send![self.central(), connectPeripheral: peripheral options: NIL] };
    }

    /// Translation of `-[HIDBLEManager updateConnectedSteamControllers:]`.
    fn update_connected_steam_controllers(&self, force: bool) -> i32 {
        if self.central().is_null() {
            return 0;
        }

        let timebase = timebase();

        // SAFETY: no preconditions.
        let ticks_now = unsafe { mach_approximate_time() };
        if !force
            && (ticks_now
                .wrapping_sub(self.last_update_tick.load(Ordering::SeqCst))
                .wrapping_mul(u64::from(timebase.numer))
                / u64::from(timebase.denom.max(1)))
                < (5 * NSEC_PER_SEC)
        {
            return self.device_count();
        }

        // we can see previously connected BLE peripherals but can't connect until the CBCentralManager
        // is fully powered up - only do work when we are in that state
        if self.central_state() != CB_MANAGER_STATE_POWERED_ON {
            return self.device_count();
        }

        // only update our last-check-time if we actually did work, otherwise there can be a long delay during initial power-up
        // SAFETY: no preconditions.
        self.last_update_tick
            .store(unsafe { mach_approximate_time() }, Ordering::SeqCst);

        // if a pair is in-flight, the central manager may still give it back via retrieveConnected... and
        // cause the SDL layer to attempt to initialize it while some of its endpoints haven't yet been established
        if self.pending_pairs.load(Ordering::SeqCst) > 0 {
            return self.device_count();
        }

        autoreleasepool(|| {
            // SAFETY: CBUUID's and NSArray's constructors, the central
            // manager and the peripherals it gives.
            unsafe {
                let uuid: Id = msg_send![class!(CBUUID), UUIDWithString: ns_string("180A").get()];
                let services: Id = msg_send![class!(NSArray), arrayWithObject: uuid];
                let peripherals: Id = msg_send![self.central(),
                                                retrieveConnectedPeripheralsWithServices: services];
                for peripheral in array_objects(peripherals) {
                    // we already know this peripheral
                    if !self.device_for(peripheral.get()).is_nil() {
                        continue;
                    }

                    log(&format!(
                        "connected peripheral: {}",
                        describe(peripheral.get())
                    ));
                    let name: Id = msg_send![peripheral, name];
                    let steam: Bool = msg_send![name, hasPrefix: ns_string("Steam").get()];
                    if steam.is_true() {
                        self.connect(peripheral.get());
                    }
                }
            }
        });

        self.device_count()
    }

    /// Translation of `-[HIDBLEManager startScan:]` (manual API for folks
    /// to start & stop scanning).
    fn start_scan(&'static self, duration: i32) {
        if self.central().is_null() {
            return;
        }

        log(&format!("BLE: requesting scan for {duration} seconds"));
        {
            let mut pending_scans = lock(&self.pending_scans);
            let first = *pending_scans == 0;
            *pending_scans += 1;
            if first {
                // SAFETY: the central manager.
                let () = unsafe {
                    msg_send![self.central(), scanForPeripheralsWithServices: NIL
                                                                            options: NIL]
                };
            }
        }

        if duration != 0 {
            run_after(
                i64::from(duration) * NSEC_PER_SEC as i64,
                main_queue(),
                move || self.stop_scan(),
            );
        }
    }

    /// Translation of `-[HIDBLEManager stopScan]`.
    fn stop_scan(&self) {
        if self.central().is_null() {
            return;
        }

        log("BLE: stopping scan");
        let mut pending_scans = lock(&self.pending_scans);
        *pending_scans -= 1;
        if *pending_scans <= 0 {
            *pending_scans = 0;
            // SAFETY: the central manager.
            let () = unsafe { msg_send![self.central(), stopScan] };
        }
    }
}

// CBCentralManagerDelegate Implementation

/// `-centralManagerDidUpdateState:` (called whenever the BLE hardware
/// state changes).
extern "C" fn manager_did_update_state(_this: Id, _sel: Sel, central: Id) {
    // SAFETY: CoreBluetooth's central manager.
    let state: NSInteger = unsafe { msg_send![central, state] };
    match state {
        CB_MANAGER_STATE_POWERED_ON => {
            log("CoreBluetooth BLE hardware is powered on and ready");

            // at startup, if we have no already attached peripherals, do a 20s scan for new unpaired devices,
            // otherwise callers should occasionally do additional scans. we don't want to continuously be
            // scanning because it drains battery, causes other nearby people to have a hard time pairing their
            // Steam Controllers, and may also trigger firmware weirdness when a device attempts to start
            // the pairing sequence multiple times concurrently
            let manager = manager();
            if manager.update_connected_steam_controllers(false) == 0 {
                // TODO: we could limit our scan to only peripherals supporting the SteamController service, but
                //  that service doesn't currently fit in the base advertising packet, we'd need to put it into an
                //  extended scan packet. Useful optimization downstream, but not currently necessary
                //	NSArray *services = @[[CBUUID UUIDWithString:VALVE_SERVICE]];
                manager.start_scan(20);
            }
        }
        CB_MANAGER_STATE_POWERED_OFF => log("CoreBluetooth BLE hardware is powered off"),
        CB_MANAGER_STATE_UNAUTHORIZED => log("CoreBluetooth BLE state is unauthorized"),
        CB_MANAGER_STATE_UNKNOWN => log("CoreBluetooth BLE state is unknown"),
        CB_MANAGER_STATE_UNSUPPORTED => {
            log("CoreBluetooth BLE hardware is unsupported on this platform")
        }
        CB_MANAGER_STATE_RESETTING => log("CoreBluetooth BLE manager is resetting"),
        _ => {}
    }
}

/// `-centralManager:didConnectPeripheral:`
extern "C" fn manager_did_connect_peripheral(_this: Id, _sel: Sel, _central: Id, peripheral: Id) {
    let manager = manager();
    let steam_controller = manager.device_for(peripheral);
    if !steam_controller.is_nil() {
        // SAFETY: a device of the map.
        unsafe { device_state(steam_controller.get()) }.set_connected(true);
    }
    manager.pending_pairs.fetch_sub(1, Ordering::SeqCst);
}

/// `-centralManager:didFailToConnectPeripheral:error:`
extern "C" fn manager_did_fail_to_connect_peripheral(
    _this: Id,
    _sel: Sel,
    _central: Id,
    peripheral: Id,
    error: Id,
) {
    log(&format!("Failed to connect: {}", describe(error)));
    let manager = manager();
    manager.remove_device(peripheral);
    manager.pending_pairs.fetch_sub(1, Ordering::SeqCst);
}

/// `-centralManager:didDiscoverPeripheral:advertisementData:RSSI:`
extern "C" fn manager_did_discover_peripheral(
    _this: Id,
    _sel: Sel,
    _central: Id,
    peripheral: Id,
    advertisement_data: Id,
    _rssi: Id,
) {
    autoreleasepool(|| {
        // SAFETY: CoreBluetooth's advertisement data (an NSDictionary) and
        // its local name (an NSString or nil).
        unsafe {
            let local_name: Id =
                msg_send![advertisement_data, objectForKey: advertisement_data_local_name_key()];
            let log_line = format!("Found '{}'", describe(local_name));

            let steam: Bool = msg_send![local_name, hasPrefix: ns_string("Steam").get()];
            if steam.is_true() {
                log(&format!(
                    "{log_line} : {} - {}",
                    describe(peripheral),
                    describe(advertisement_data)
                ));
                manager().connect(peripheral);
            }
        }
    });
}

/// `-centralManager:didDisconnectPeripheral:error:`
extern "C" fn manager_did_disconnect_peripheral(
    _this: Id,
    _sel: Sel,
    _central: Id,
    peripheral: Id,
    _error: Id,
) {
    let manager = manager();
    let steam_controller = manager.device_for(peripheral);
    if !steam_controller.is_nil() {
        // SAFETY: a device of the map.
        unsafe { device_state(steam_controller.get()) }.on_disconnect();
        manager.remove_device(peripheral);
    }
}

// - hid_api implementation -

/// Translation of `hid_init()`: the Bluetooth manager, made on first use.
pub(super) fn hid_init() -> Result<()> {
    // (never nil)
    shared_instance();
    Ok(())
}

/// Translation of `hid_exit()` (the manager stays).
pub(super) fn hid_exit() -> Result<()> {
    Ok(())
}

/// Translation of `hid_ble_scan()`: start a scan until it is stopped, or
/// stop one.
pub(super) fn hid_ble_scan(start: bool) {
    let ble_manager = shared_instance();
    if start {
        ble_manager.start_scan(0);
    } else {
        ble_manager.stop_scan();
    }
}

/// Translation of `hid_open()`: devices are only opened by path.
pub(super) fn open(
    _vendor_id: u16,
    _product_id: u16,
    _serial_number: Option<&str>,
) -> Result<Device> {
    // Note: upstream returns NULL without an error message.
    Err(Error::unsupported())
}

/// An open device. Translation of `struct hid_device_`.
#[derive(Debug)]
pub(super) struct Device {
    /// `device_handle`: the `HIDBLEDevice` (`CFBridgingRetain`ed).
    device_handle: StrongPtr,
    /// `blocking` (reads never block here)
    blocking: AtomicBool,
    /// `device_info`, made on first use.
    device_info: OnceLock<DeviceInfo>,
}

/// Translation of `create_device_info_for_hid_device()`.
fn create_device_info_for_hid_device(device: &DeviceState) -> DeviceInfo {
    // We currently only support the Steam Controller
    DeviceInfo {
        path: Some(identifier_string(device.peripheral.get())),
        vendor_id: VALVE_USB_VID,
        product_id: device.pid.load(Ordering::SeqCst),
        product_string: Some("Steam Controller".into()),
        manufacturer_string: Some("Valve Corporation".into()),
        bus_type: BusType::Bluetooth,
        // (the rest is zero, from a memset)
        ..DeviceInfo::default()
    }
}

/// Translation of `hid_open_path()`: a connected and ready device by its
/// peripheral's identifier.
pub(super) fn open_path(path: &str) -> Result<Device> {
    autoreleasepool(|| {
        let ble_manager = shared_instance();
        for device in ble_manager.devices() {
            // SAFETY: a device of the map, retained.
            let state = unsafe { device_state(device.get()) };
            // we have the device but it hasn't found its service or characteristics until it is connected
            let input = state.input_characteristic();
            if !state.is_ready() || !state.is_connected() || input.is_nil() {
                continue;
            }

            if identifier_string(state.peripheral.get()) == path {
                // enable reporting input events on the characteristic
                state.set_notify_value(true, &input);
                return Ok(Device {
                    device_handle: device,
                    blocking: AtomicBool::new(false),
                    device_info: OnceLock::new(),
                });
            }
        }
        // Note: upstream returns NULL without an error message.
        Err(Error::new(
            "No connected Bluetooth LE device with that path",
        ))
    })
}

/// Translation of `hid_enumerate()`: the Steam Controllers connected and
/// ready, in the reverse of the map's order (each one is put first).
pub(super) fn hid_enumerate(vendor_id: u16, product_id: u16) -> Vec<DeviceInfo> {
    autoreleasepool(|| {
        let mut root = Vec::new();

        let ble_manager = shared_instance();
        ble_manager.update_connected_steam_controllers(false);
        for device in ble_manager.devices() {
            // SAFETY: a device of the map, retained.
            let state = unsafe { device_state(device.get()) };
            // SAFETY: its peripheral.
            let peripheral_state: NSInteger = unsafe { msg_send![state.peripheral, state] };
            // there are several brief windows in connecting to an already paired device and
            // one long window waiting for users to confirm pairing where we don't want
            // to consider a device ready - if we hand it back to SDL or another
            // Steam Controller consumer, their additional SC setup work will fail
            // in unusual/silent ways and we can actually corrupt the BLE stack for
            // the entire system and kill the appletv remote's Menu button (!)
            if peripheral_state != CB_PERIPHERAL_STATE_CONNECTED
                || !state.is_connected()
                || !state.is_ready()
            {
                let input = state.input_characteristic();
                if !state.is_ready() && !input.is_nil() {
                    // attempt to register for input reports. this call will silently fail
                    // until the pairing finalizes with user acceptance. oh, apple.
                    state.set_notify_value(true, &input);
                }
                continue;
            }

            let pid = state.pid.load(Ordering::SeqCst);
            if (vendor_id != 0 && vendor_id != VALVE_USB_VID)
                || (product_id != 0 && product_id != pid)
            {
                continue;
            }

            /* See if there are any devices we should skip in enumeration */
            if super::should_ignore_device(
                BusType::Bluetooth,
                VALVE_USB_VID,
                pid,
                0,
                0,
                false,
                false,
            ) {
                continue;
            }

            root.insert(0, create_device_info_for_hid_device(state));
        }
        root
    })
}

/// "Device is disconnected" (Note: upstream returns -1 without an error
/// message).
fn disconnected() -> Error {
    Error::new("Device is disconnected")
}

impl Device {
    fn state(&self) -> &DeviceState {
        // SAFETY: the device is retained by self.
        unsafe { device_state(self.device_handle.get()) }
    }

    /// Translation of `hid_write()`.
    pub(super) fn write(&self, data: &[u8]) -> Result<usize> {
        let device_handle = self.state();

        if !device_handle.is_connected() {
            return Err(disconnected());
        }

        device_handle.send_report(data)
    }

    /// Translation of `hid_read_timeout()`: reads never wait.
    pub(super) fn read_timeout(&self, data: &mut [u8], milliseconds: i32) -> Result<usize> {
        let device_handle = self.state();

        if !device_handle.is_connected() {
            return Err(disconnected());
        }

        if milliseconds != 0 {
            log("hid_read_timeout with non-zero wait");
        }
        // (a disabled FEATURE_REPORT_LOGGING NSLogs the report)
        Ok(device_handle.read_input_report(data))
    }

    /// Translation of `hid_read()`.
    pub(super) fn read(&self, data: &mut [u8]) -> Result<usize> {
        if !self.state().is_connected() {
            return Err(disconnected());
        }

        self.read_timeout(data, 0)
    }

    /// Translation of `hid_set_nonblocking()`.
    pub(super) fn set_nonblocking(&self, nonblock: bool) -> Result<()> {
        /* All Nonblocking operation is handled by the library. */
        self.blocking.store(!nonblock, Ordering::Relaxed);

        Ok(())
    }

    /// Translation of `hid_send_feature_report()`.
    pub(super) fn send_feature_report(&self, data: &[u8]) -> Result<usize> {
        let device_handle = self.state();

        if !device_handle.is_connected() {
            return Err(disconnected());
        }

        Ok(device_handle.send_feature_report(data))
    }

    /// Translation of `hid_get_feature_report()`.
    pub(super) fn get_feature_report(&self, data: &mut [u8]) -> Result<usize> {
        let device_handle = self.state();

        if !device_handle.is_connected() {
            return Err(disconnected());
        }
        if data.is_empty() {
            // Note (upstream): the report ID is read from an empty buffer.
            return Err(Error::invalid_param("data"));
        }

        let written = device_handle.get_feature_report(data);

        match ble::feature_report_result(written, data.len()) {
            // Note: upstream returns -1 without an error message.
            n if n < 0 => Err(Error::new("Couldn't get the feature report")),
            n => Ok(n as usize),
        }
    }

    /// Translation of `hid_get_input_report()`: not supported.
    pub(super) fn get_input_report(&self, _data: &mut [u8]) -> Result<usize> {
        // Not supported
        Err(Error::unsupported())
    }

    /// Translation of `hid_get_manufacturer_string()`.
    pub(super) fn get_manufacturer_string(&self) -> Result<String> {
        // FIXME (upstream): the string is copied whatever the buffer's
        // `maxlen`.
        Ok("Valve Corporation".into())
    }

    /// Translation of `hid_get_product_string()`.
    pub(super) fn get_product_string(&self) -> Result<String> {
        // FIXME (upstream): as above.
        Ok("Steam Controller".into())
    }

    /// Translation of `hid_get_serial_number_string()`.
    pub(super) fn get_serial_number_string(&self) -> Result<String> {
        // FIXME (upstream): as above.
        Ok("12345".into())
    }

    /// Translation of `hid_get_indexed_string()`: not supported.
    pub(super) fn get_indexed_string(&self, _string_index: i32) -> Result<String> {
        Err(Error::unsupported())
    }

    /// Translation of `hid_get_device_info()`.
    pub(super) fn get_device_info(&self) -> Result<DeviceInfo> {
        // Lazy initialize device_info
        Ok(self
            .device_info
            .get_or_init(|| create_device_info_for_hid_device(self.state()))
            .clone())
    }

    /// Translation of `hid_get_report_descriptor()`: not implemented.
    pub(super) fn get_report_descriptor(&self, _buf: &mut [u8]) -> Result<usize> {
        // Not implemented
        Err(Error::unsupported())
    }
}

impl Drop for Device {
    /// Translation of `hid_close()`.
    fn drop(&mut self) {
        let device_handle = self.state();

        // disable reporting input events on the characteristic
        if device_handle.is_connected() {
            let input = device_handle.input_characteristic();
            device_handle.set_notify_value(false, &input);
        }
    }
}
