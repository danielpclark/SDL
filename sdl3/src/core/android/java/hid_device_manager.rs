// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// HIDDeviceManager.java from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `public class HIDDeviceManager`: the HID devices of HIDAPI's Android
//! backend (`crate::hidapi::android`, upstream's hid.cpp), passed on through
//! its natives (`HIDDeviceConnected()`, ...):
//!
//! * USB: the `UsbManager`'s devices, at `initialize(usb)` and as they are
//!   attached and detached (a `BroadcastReceiver`), each HID interface of
//!   theirs (and the vendor-specific interfaces of Xbox 360 and Xbox One
//!   controllers, of the vendors upstream lists) a device
//!   (`super::hid_device_usb`); opening one asks for the user's permission
//!   when the app hasn't it (`UsbManager.requestPermission()`, answered
//!   through the receiver);
//! * Bluetooth: Valve's Steam Controllers, at `initialize(bluetooth)` the
//!   bonded ones and then as they connect and disconnect (ACL broadcasts,
//!   another receiver), each a device once configured
//!   (`super::hid_device_ble_steam`). It needs `BLUETOOTH_CONNECT` (API 31
//!   and later) or `BLUETOOTH` (before), and Bluetooth LE;
//! * the device IDs, kept in the app's shared preferences (`hidapi`) by
//!   identifier, so that a device keeps its ID;
//! * `setFrozen()`, from the activity's pause, resume and focus changes:
//!   the USB devices' interfaces released while the app isn't in front,
//!   and the reports dropped.
//!
//! The manager is a singleton, acquired and released by the activity's
//! `onCreate()` and `onDestroy()`. Its class is the DEX file's
//! (`super::super::classes`), with its natives and the methods hid.cpp
//! calls through JNI (`initialize()`, `openDevice()`, `writeReport()`,
//! `readReport()`, `closeDevice()`, declared native too, so that Java can
//! call them); the natives' bodies are hid.cpp's Rust, which this calls
//! directly. Its fields are Rust (a lock standing in for the monitor its
//! `synchronized` blocks take, reentrant as Java's: a device disconnected
//! under it closes itself through `closeDevice()`). Upstream's two
//! anonymous `BroadcastReceiver`s are the DEX file's
//! `org.libsdl.app.HIDDeviceManagerUsbReceiver` and
//! `HIDDeviceManagerBluetoothReceiver` (Note: other names than upstream's
//! `HIDDeviceManager$1` and `$2`), whose `onReceive()` is native.

use std::collections::HashMap;
use std::ffi::CStr;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use super::super::classes::{
    HID_DEVICE_MANAGER, HID_DEVICE_MANAGER_BLUETOOTH_RECEIVER, HID_DEVICE_MANAGER_USB_RECEIVER,
};
use super::super::env::{self, Arg, Env, Global, JniResult, Local, Obj};
use super::super::jni::*;
use super::super::ndk::{self, LogPriority};
use super::hid_device::HidDevice;
use super::hid_device_ble_steam::HidDeviceBleSteamController;
use super::hid_device_usb::HidDeviceUsb;
use super::{activity, anonymous};
use crate::hidapi::android as hid;
use crate::thread::ReentrantMutex;

const TAG: &str = "hidapi";
const ACTION_USB_PERMISSION: &str = "org.libsdl.app.USB_PERMISSION";

/// `Context.USB_SERVICE`, `Context.BLUETOOTH_SERVICE`.
const USB_SERVICE: &str = "usb";
pub(super) const BLUETOOTH_SERVICE: &str = "bluetooth";
/// `Context.MODE_PRIVATE`.
const MODE_PRIVATE: jint = 0;
/// `Context.RECEIVER_EXPORTED`.
const RECEIVER_EXPORTED: jint = 0x2;
/// `UsbManager.ACTION_USB_DEVICE_ATTACHED`, `ACTION_USB_DEVICE_DETACHED`,
/// `EXTRA_DEVICE`, `EXTRA_PERMISSION_GRANTED`.
const ACTION_USB_DEVICE_ATTACHED: &str = "android.hardware.usb.action.USB_DEVICE_ATTACHED";
const ACTION_USB_DEVICE_DETACHED: &str = "android.hardware.usb.action.USB_DEVICE_DETACHED";
const USB_EXTRA_DEVICE: &str = "device";
const EXTRA_PERMISSION_GRANTED: &str = "permission";
/// `BluetoothDevice.ACTION_ACL_CONNECTED`, `ACTION_ACL_DISCONNECTED`,
/// `EXTRA_DEVICE`, `DEVICE_TYPE_LE`.
const ACTION_ACL_CONNECTED: &str = "android.bluetooth.device.action.ACL_CONNECTED";
const ACTION_ACL_DISCONNECTED: &str = "android.bluetooth.device.action.ACL_DISCONNECTED";
const BLUETOOTH_EXTRA_DEVICE: &str = "android.bluetooth.device.extra.DEVICE";
const DEVICE_TYPE_LE: jint = 2;
/// `BluetoothProfile.GATT`.
pub(super) const GATT: jint = 7;
/// `UsbConstants.USB_CLASS_HID`, `USB_CLASS_VENDOR_SPEC`.
const USB_CLASS_HID: jint = 3;
const USB_CLASS_VENDOR_SPEC: jint = 0xff;
/// `PackageManager.PERMISSION_GRANTED`, `FEATURE_BLUETOOTH_LE`.
const PERMISSION_GRANTED: jint = 0;
const FEATURE_BLUETOOTH_LE: &str = "android.hardware.bluetooth_le";
/// `PendingIntent.FLAG_MUTABLE`, but don't require SDK 31
const FLAG_MUTABLE: jint = 0x02000000;

/// `Log.v(TAG, ...)`, `Log.d()`, ...
pub(super) fn log(priority: LogPriority, message: &str) {
    ndk::log_write(priority, TAG, message);
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// Java helpers, for this file and the devices'.

/// `throw new CLASS(message)`.
pub(super) fn throw<'a, T>(env: Env<'a>, class: &CStr, message: &CStr) -> JniResult<'a, T> {
    let class = env.find_class(class)?;
    env.throw_new(class.obj(), message);
    env.pending()?;
    unreachable!("ThrowNew() threw")
}

/// The `NullPointerException` of a call on null (`what`).
pub(super) fn null_pointer<'a, T>(env: Env<'a>, what: &CStr) -> JniResult<'a, T> {
    throw(env, c"java/lang/NullPointerException", what)
}

/// A non-null reference, else the `NullPointerException` of a call on it.
pub(super) fn non_null<'a, 'o>(env: Env<'a>, obj: Obj<'o>, what: &CStr) -> JniResult<'a, Obj<'o>> {
    if obj.is_null() {
        null_pointer(env, what)
    } else {
        Ok(obj)
    }
}

/// `String.valueOf(obj)` (`"null"` for null), as Java's string
/// concatenation makes it.
pub(super) fn string_of<'a>(env: Env<'a>, obj: Obj<'_>) -> JniResult<'a, String> {
    if obj.is_null() {
        return Ok("null".into());
    }
    let string = env
        .call_method(obj, c"toString", c"()Ljava/lang/String;", &[])?
        .object();
    match string {
        Some(string) => env.get_string(string.obj()),
        None => Ok("null".into()),
    }
}

/// A method returning a `String` (`None` for null).
pub(super) fn call_string<'a>(
    env: Env<'a>,
    obj: Obj<'_>,
    name: &CStr,
) -> JniResult<'a, Option<String>> {
    match env
        .call_method(obj, name, c"()Ljava/lang/String;", &[])?
        .object()
    {
        Some(string) => env.get_string(string.obj()).map(Some),
        None => Ok(None),
    }
}

/// `a.equals(b)`.
pub(super) fn java_equals<'a>(env: Env<'a>, a: Obj<'_>, b: Obj<'_>) -> JniResult<'a, bool> {
    let a = non_null(env, a, c"equals() on null")?;
    Ok(env
        .call_method(a, c"equals", c"(Ljava/lang/Object;)Z", &[Arg::Object(b)])?
        .boolean())
}

/// A `Collection`'s elements (`toArray()`), in its iteration order.
pub(super) fn collection<'a>(env: Env<'a>, collection: Obj<'_>) -> JniResult<'a, Vec<Local<'a>>> {
    let collection = non_null(env, collection, c"a null collection")?;
    let array = env
        .call_method(collection, c"toArray", c"()[Ljava/lang/Object;", &[])?
        .object()
        .expect("toArray() returns an array");
    let mut elements = Vec::new();
    for i in 0..env.get_array_length(array.obj()) {
        if let Some(element) = env.get_object_array_element(array.obj(), i)? {
            elements.push(element);
        }
    }
    Ok(elements)
}

/// `Log.getStackTraceString(e)`.
pub(super) fn stack_trace_string<'a>(env: Env<'a>, e: Obj<'_>) -> JniResult<'a, String> {
    let log_class = env.find_class(c"android/util/Log")?;
    let string = env
        .call_static_method(
            log_class.obj(),
            c"getStackTraceString",
            c"(Ljava/lang/Throwable;)Ljava/lang/String;",
            &[Arg::Object(e)],
        )?
        .object();
    match string {
        Some(string) => env.get_string(string.obj()),
        None => Ok(String::new()),
    }
}

/// `catch (Exception e) { Log.e(TAG, "Got exception: " + Log.getStackTraceString(e)); }`.
fn log_exception<'a>(env: Env<'a>, thrown: env::Thrown<'a>) -> JniResult<'a, ()> {
    let e = thrown.catch_exception()?;
    let trace = stack_trace_string(env, e.obj())?;
    log(LogPriority::Error, &format!("Got exception: {trace}"));
    Ok(())
}

/// `context.getSystemService(name)`.
pub(super) fn system_service<'a>(
    env: Env<'a>,
    context: Obj<'_>,
    name: &str,
) -> JniResult<'a, Option<Local<'a>>> {
    let context = non_null(env, context, c"getSystemService() on a null context")?;
    let name = env.new_string(name)?;
    Ok(env
        .call_method(
            context,
            c"getSystemService",
            c"(Ljava/lang/String;)Ljava/lang/Object;",
            &[Arg::Object(name.obj())],
        )?
        .object())
}

/// `new Handler(Looper.getMainLooper())`.
pub(super) fn main_handler<'a>(env: Env<'a>) -> JniResult<'a, Local<'a>> {
    let looper_class = env.find_class(c"android/os/Looper")?;
    let looper = env
        .call_static_method(
            looper_class.obj(),
            c"getMainLooper",
            c"()Landroid/os/Looper;",
            &[],
        )?
        .object();
    let handler_class = env.find_class(c"android/os/Handler")?;
    env.new_object(
        handler_class.obj(),
        c"(Landroid/os/Looper;)V",
        &[Arg::Object(looper.as_ref().map_or(Obj::NULL, Local::obj))],
    )
}

/// `handler.postDelayed(new Runnable() {...}, delay_ms)` (`post()` for no
/// delay).
pub(super) fn post_delayed<'a>(
    env: Env<'a>,
    handler: Obj<'_>,
    delay_ms: jint,
    f: impl FnOnce(Env<'_>) + Send + 'static,
) -> JniResult<'a, ()> {
    let runnable = anonymous::runnable(env, f)?;
    if delay_ms == 0 {
        env.call_method(
            handler,
            c"post",
            c"(Ljava/lang/Runnable;)Z",
            &[Arg::Object(runnable.obj())],
        )?;
    } else {
        env.call_method(
            handler,
            c"postDelayed",
            c"(Ljava/lang/Runnable;J)Z",
            &[
                Arg::Object(runnable.obj()),
                Arg::Long(jlong::from(delay_ms)),
            ],
        )?;
    }
    Ok(())
}

/// `Build.VERSION.SDK_INT`.
fn sdk_int(env: Env<'_>) -> jint {
    super::super::sdk_int(env)
}

// The manager

/// `sManager` and `sManagerRefCount`.
struct Singleton {
    manager: Option<Arc<HidDeviceManager>>,
    ref_count: i32,
}

static S_MANAGER: Mutex<Singleton> = Mutex::new(Singleton {
    manager: None,
    ref_count: 0,
});

/// Translation of `acquire()`.
pub(crate) fn acquire<'a>(env: Env<'a>, context: Obj<'_>) -> JniResult<'a, Arc<HidDeviceManager>> {
    let mut s = lock(&S_MANAGER);
    if s.ref_count == 0 {
        s.manager = Some(HidDeviceManager::new(env, context)?);
    }
    s.ref_count += 1;
    Ok(s.manager.clone().expect("the manager"))
}

/// Translation of `release()`.
pub(crate) fn release<'a>(env: Env<'a>, manager: &Arc<HidDeviceManager>) -> JniResult<'a, ()> {
    let closed = {
        let mut s = lock(&S_MANAGER);
        if s.manager.as_ref().is_some_and(|m| Arc::ptr_eq(m, manager)) {
            s.ref_count -= 1;
            if s.ref_count == 0 {
                s.manager.take()
            } else {
                None
            }
        } else {
            None
        }
    };
    // (closed with the singleton's lock released: the receivers take it)
    if let Some(manager) = closed {
        manager.close(env)?;
    }
    Ok(())
}

/// The manager of a Java object of its class (`this`), if it's the
/// singleton's.
fn manager_of(env: Env<'_>, this: Obj<'_>) -> Option<Arc<HidDeviceManager>> {
    lock(&S_MANAGER)
        .manager
        .as_ref()
        .filter(|m| env.is_same_object(m.this.obj(), this))
        .cloned()
}

/// Translation of `class HIDDeviceManager`'s fields.
pub(crate) struct HidDeviceManager {
    /// The Java object.
    this: Global,
    /// `mContext`
    context: Global,
    /// `mDevicesById`
    devices_by_id: Mutex<HashMap<jint, Arc<dyn HidDevice>>>,
    /// `mBluetoothDevices`, by the devices' addresses (Note: a
    /// `BluetoothDevice` equals one of the same address)
    bluetooth_devices: Mutex<HashMap<String, Arc<HidDeviceBleSteamController>>>,
    /// `mNextDeviceId`
    next_device_id: Mutex<jint>,
    /// `mSharedPreferences`
    shared_preferences: Global,
    /// `mIsChromebook`
    is_chromebook: bool,
    /// `mUsbManager`
    usb_manager: Mutex<Option<Global>>,
    /// `mHandler`
    handler: Mutex<Option<Global>>,
    /// `mBluetoothManager`
    bluetooth_manager: Mutex<Option<Global>>,
    /// `mLastBluetoothDevices`
    last_bluetooth_devices: Mutex<Option<Vec<Global>>>,
    /// `mUsbBroadcast`
    usb_broadcast: Global,
    /// `mBluetoothBroadcast`
    bluetooth_broadcast: Global,
    /// The monitor of `synchronized (this)`.
    monitor: ReentrantMutex<()>,
    weak_self: Weak<HidDeviceManager>,
}

impl std::fmt::Debug for HidDeviceManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HidDeviceManager")
            .field("this", &self.this)
            .finish_non_exhaustive()
    }
}

/// `new CLASS()`, a class of the DEX file's.
fn new_app_object<'a>(env: Env<'a>, name: &str) -> JniResult<'a, Local<'a>> {
    let class = env::app_class(name).expect("JNI_OnLoad() found the HID device manager's classes");
    env.new_object(class.obj(), c"()V", &[])
}

fn global(env: Env<'_>, obj: Obj<'_>) -> Global {
    Global::new(env, obj).expect("a global reference")
}

/// `context.registerReceiver(receiver, filter)` (exported from API 33, as
/// upstream asks).
fn register_receiver<'a>(
    env: Env<'a>,
    context: Obj<'_>,
    receiver: Obj<'_>,
    actions: &[&str],
) -> JniResult<'a, ()> {
    let filter_class = env.find_class(c"android/content/IntentFilter")?;
    let filter = env.new_object(filter_class.obj(), c"()V", &[])?;
    for action in actions {
        let action = env.new_string(action)?;
        env.call_method(
            filter.obj(),
            c"addAction",
            c"(Ljava/lang/String;)V",
            &[Arg::Object(action.obj())],
        )?;
    }
    if sdk_int(env) >= 33
    /* Android 13.0 (TIRAMISU) */
    {
        env.call_method(
            context,
            c"registerReceiver",
            c"(Landroid/content/BroadcastReceiver;Landroid/content/IntentFilter;I)Landroid/content/Intent;",
            &[
                Arg::Object(receiver),
                Arg::Object(filter.obj()),
                Arg::Int(RECEIVER_EXPORTED),
            ],
        )?;
    } else {
        env.call_method(
            context,
            c"registerReceiver",
            c"(Landroid/content/BroadcastReceiver;Landroid/content/IntentFilter;)Landroid/content/Intent;",
            &[Arg::Object(receiver), Arg::Object(filter.obj())],
        )?;
    }
    Ok(())
}

/// `try { mContext.unregisterReceiver(receiver); } catch (Exception e) {
/// // We may not have registered, that's okay }`.
fn unregister_receiver<'a>(env: Env<'a>, context: Obj<'_>, receiver: Obj<'_>) -> JniResult<'a, ()> {
    if let Err(thrown) = env.call_method(
        context,
        c"unregisterReceiver",
        c"(Landroid/content/BroadcastReceiver;)V",
        &[Arg::Object(receiver)],
    ) {
        // We may not have registered, that's okay
        drop(thrown.catch_exception()?);
    }
    Ok(())
}

/// What `isHIDDeviceInterface()` and the Xbox checks read of a
/// `UsbInterface`.
#[derive(Clone, Copy, Debug)]
struct UsbInterfaceInfo {
    id: jint,
    class: jint,
    subclass: jint,
    protocol: jint,
}

impl UsbInterfaceInfo {
    fn of<'a>(env: Env<'a>, usb_interface: Obj<'_>) -> JniResult<'a, UsbInterfaceInfo> {
        let get = |name: &CStr| -> JniResult<'a, jint> {
            Ok(env.call_method(usb_interface, name, c"()I", &[])?.int())
        };
        Ok(UsbInterfaceInfo {
            id: get(c"getId")?,
            class: get(c"getInterfaceClass")?,
            subclass: get(c"getInterfaceSubclass")?,
            protocol: get(c"getInterfaceProtocol")?,
        })
    }
}

/// Translation of `isHIDDeviceInterface()`.
fn is_hid_device_interface(vendor_id: jint, usb_interface: UsbInterfaceInfo) -> bool {
    if usb_interface.class == USB_CLASS_HID {
        return true;
    }
    if is_xbox360_controller(vendor_id, usb_interface)
        || is_xbox_one_controller(vendor_id, usb_interface)
    {
        return true;
    }
    false
}

/// Translation of `isXbox360Controller()`.
fn is_xbox360_controller(vendor_id: jint, usb_interface: UsbInterfaceInfo) -> bool {
    const XB360_IFACE_SUBCLASS: jint = 93;
    const XB360_IFACE_PROTOCOL: jint = 1; // Wired
    const XB360W_IFACE_PROTOCOL: jint = 129; // Wireless
    const SUPPORTED_VENDORS: [jint; 33] = [
        0x0079, // GPD Win 2
        0x0351, // CRKD
        0x044f, // Thrustmaster
        0x045e, // Microsoft
        0x046d, // Logitech
        0x056e, // Elecom
        0x06a3, // Saitek
        0x0738, // Mad Catz
        0x07ff, // Mad Catz
        0x0e6f, // PDP
        0x0f0d, // Hori
        0x1038, // SteelSeries
        0x10f5, // Turtle Beach
        0x11c9, // Nacon
        0x1209, // Generic
        0x12ab, // Unknown
        0x1430, // RedOctane
        0x146b, // BigBen
        0x1532, // Razer Sabertooth
        0x15e4, // Numark
        0x162e, // Joytech
        0x1689, // Razer Onza
        0x1949, // Lab126, Inc.
        0x1bad, // Harmonix
        0x20d6, // PowerA
        0x24c6, // PowerA
        0x2c22, // Qanba
        0x2dc8, // 8BitDo
        0x3537, // GameSir
        0x3651, // CRKD
        0x37d7, // Flydigi
        0x3958, // Red Octane Games
        0x9886, // ASTRO Gaming
    ];

    if usb_interface.class == USB_CLASS_VENDOR_SPEC
        && usb_interface.subclass == XB360_IFACE_SUBCLASS
        && (usb_interface.protocol == XB360_IFACE_PROTOCOL
            || usb_interface.protocol == XB360W_IFACE_PROTOCOL)
    {
        return SUPPORTED_VENDORS.contains(&vendor_id);
    }
    false
}

/// Translation of `isXboxOneController()`.
fn is_xbox_one_controller(vendor_id: jint, usb_interface: UsbInterfaceInfo) -> bool {
    const XB1_IFACE_SUBCLASS: jint = 71;
    const XB1_IFACE_PROTOCOL: jint = 208;
    const SUPPORTED_VENDORS: [jint; 22] = [
        0x0351, // CRKD
        0x03f0, // HP
        0x044f, // Thrustmaster
        0x045e, // Microsoft
        0x0738, // Mad Catz
        0x0b05, // ASUS
        0x0e6f, // PDP
        0x0f0d, // Hori
        0x10f5, // Turtle Beach
        0x1209, // Generic
        0x1532, // Razer Wildcat
        0x20d6, // PowerA
        0x24c6, // PowerA
        0x294b, // Snakebyte
        0x2dc8, // 8BitDo
        0x2e24, // Hyperkin
        0x2e95, // SCUF
        0x3285, // Nacon
        0x3537, // GameSir
        0x3651, // CRKD
        0x366c, // ByoWave
        0x3958, // Red Octane Games
    ];

    if usb_interface.id == 0
        && usb_interface.class == USB_CLASS_VENDOR_SPEC
        && usb_interface.subclass == XB1_IFACE_SUBCLASS
        && usb_interface.protocol == XB1_IFACE_PROTOCOL
    {
        return SUPPORTED_VENDORS.contains(&vendor_id);
    }
    false
}

/// Translation of `isSteamController()`'s test of a device's name and
/// type: Steam Controllers will always support Bluetooth Low Energy, and
/// advertise with the name of the original Steam Controller or of the new
/// second-generation one.
fn is_steam_controller_name(name: Option<&str>, device_type: jint) -> bool {
    // If the device has no local name, we really don't want to try an equality check against it.
    let Some(name) = name else {
        return false;
    };

    // Steam Controllers will always support Bluetooth Low Energy
    if (device_type & DEVICE_TYPE_LE) == 0 {
        return false;
    }

    // Match on the name either the original Steam Controller or the new second-generation one advertise with.
    name == "SteamController" || name.starts_with("Steam Ctrl")
}

impl HidDeviceManager {
    /// Translation of the constructor `HIDDeviceManager(Context)` (with the
    /// fields' initializers: the receivers).
    fn new<'a>(env: Env<'a>, context: Obj<'_>) -> JniResult<'a, Arc<HidDeviceManager>> {
        let this = new_app_object(env, HID_DEVICE_MANAGER)?;
        let usb_broadcast = new_app_object(env, HID_DEVICE_MANAGER_USB_RECEIVER)?;
        let bluetooth_broadcast = new_app_object(env, HID_DEVICE_MANAGER_BLUETOOTH_RECEIVER)?;

        hid::hid_device_register_callback(env, this.obj());

        let context = non_null(env, context, c"getSharedPreferences() on a null context")?;
        let name = env.new_string("hidapi")?;
        let shared_preferences = env
            .call_method(
                context,
                c"getSharedPreferences",
                c"(Ljava/lang/String;I)Landroid/content/SharedPreferences;",
                &[Arg::Object(name.obj()), Arg::Int(MODE_PRIVATE)],
            )?
            .object()
            .expect("the shared preferences");
        let is_chromebook = activity::is_chromebook(env)?;

        // (a commented-out shouldClear clears the preferences)
        let key = env.new_string("next_device_id")?;
        let next_device_id = env
            .call_method(
                shared_preferences.obj(),
                c"getInt",
                c"(Ljava/lang/String;I)I",
                &[Arg::Object(key.obj()), Arg::Int(0)],
            )?
            .int();

        Ok(Arc::new_cyclic(|weak_self| HidDeviceManager {
            this: global(env, this.obj()),
            context: global(env, context),
            devices_by_id: Mutex::new(HashMap::new()),
            bluetooth_devices: Mutex::new(HashMap::new()),
            next_device_id: Mutex::new(next_device_id),
            shared_preferences: global(env, shared_preferences.obj()),
            is_chromebook,
            usb_manager: Mutex::new(None),
            handler: Mutex::new(None),
            bluetooth_manager: Mutex::new(None),
            last_bluetooth_devices: Mutex::new(None),
            usb_broadcast: global(env, usb_broadcast.obj()),
            bluetooth_broadcast: global(env, bluetooth_broadcast.obj()),
            monitor: ReentrantMutex::new(()),
            weak_self: weak_self.clone(),
        }))
    }

    /// The Java object.
    pub(crate) fn object(&self) -> Obj<'_> {
        self.this.obj()
    }

    /// A device's `mManager`.
    pub(super) fn weak(&self) -> Weak<HidDeviceManager> {
        self.weak_self.clone()
    }

    /// Translation of `getContext()`.
    pub(super) fn get_context(&self) -> Obj<'_> {
        self.context.obj()
    }

    /// Translation of `getDeviceIDForIdentifier()`. (Note (upstream): the
    /// first ID given is 0, which reads as none: that device gets another
    /// one the next time.)
    pub(super) fn get_device_id_for_identifier<'a>(
        &self,
        env: Env<'a>,
        identifier: &str,
    ) -> JniResult<'a, jint> {
        let prefs = self.shared_preferences.obj();
        let spedit = env
            .call_method(
                prefs,
                c"edit",
                c"()Landroid/content/SharedPreferences$Editor;",
                &[],
            )?
            .object()
            .expect("an editor");
        let put_int = |key: Obj<'_>, value: jint| -> JniResult<'a, ()> {
            env.call_method(
                spedit.obj(),
                c"putInt",
                c"(Ljava/lang/String;I)Landroid/content/SharedPreferences$Editor;",
                &[Arg::Object(key), Arg::Int(value)],
            )?;
            Ok(())
        };

        let key = env.new_string(identifier)?;
        let mut result = env
            .call_method(
                prefs,
                c"getInt",
                c"(Ljava/lang/String;I)I",
                &[Arg::Object(key.obj()), Arg::Int(0)],
            )?
            .int();
        if result == 0 {
            let mut next_device_id = lock(&self.next_device_id);
            result = *next_device_id;
            *next_device_id += 1;
            let next_key = env.new_string("next_device_id")?;
            put_int(next_key.obj(), *next_device_id)?;
        }

        put_int(key.obj(), result)?;
        env.call_method(spedit.obj(), c"apply", c"()V", &[])?;
        Ok(result)
    }

    /// Translation of `initializeUSB()`.
    fn initialize_usb<'a>(&self, env: Env<'a>) -> JniResult<'a, ()> {
        let usb_manager = system_service(env, self.context.obj(), USB_SERVICE)?;
        *lock(&self.usb_manager) = usb_manager.as_ref().map(|m| global(env, m.obj()));
        let Some(usb_manager) = usb_manager else {
            return Ok(());
        };

        // (a commented-out loop logs the devices, their interfaces and
        // endpoints)

        // Register for USB broadcasts and permission completions
        register_receiver(
            env,
            self.context.obj(),
            self.usb_broadcast.obj(),
            &[
                ACTION_USB_DEVICE_ATTACHED,
                ACTION_USB_DEVICE_DETACHED,
                ACTION_USB_PERMISSION,
            ],
        )?;

        let device_list = env
            .call_method(
                usb_manager.obj(),
                c"getDeviceList",
                c"()Ljava/util/HashMap;",
                &[],
            )?
            .object();
        let values = match &device_list {
            Some(list) => env
                .call_method(list.obj(), c"values", c"()Ljava/util/Collection;", &[])?
                .object(),
            None => None,
        };
        for usb_device in collection(env, values.as_ref().map_or(Obj::NULL, Local::obj))? {
            self.handle_usb_device_attached(env, usb_device.obj())?;
        }
        Ok(())
    }

    /// Translation of `getUSBManager()`.
    pub(super) fn get_usb_manager<'a>(&self, env: Env<'a>) -> Option<Local<'a>> {
        lock(&self.usb_manager)
            .as_ref()
            .and_then(|m| env.new_local_ref(m.obj()))
    }

    /// Translation of `shutdownUSB()`.
    fn shutdown_usb<'a>(&self, env: Env<'a>) -> JniResult<'a, ()> {
        unregister_receiver(env, self.context.obj(), self.usb_broadcast.obj())
    }

    /// Translation of `handleUsbDeviceAttached()`.
    fn handle_usb_device_attached<'a>(
        &self,
        env: Env<'a>,
        usb_device: Obj<'_>,
    ) -> JniResult<'a, ()> {
        self.connect_hid_device_usb(env, usb_device)
    }

    /// The devices, as Java's iteration of `mDevicesById.values()` sees
    /// them.
    fn devices(&self) -> Vec<Arc<dyn HidDevice>> {
        lock(&self.devices_by_id).values().cloned().collect()
    }

    /// The devices of a `UsbDevice` (`usbDevice.equals(device.getDevice())`).
    fn devices_of<'a>(
        &self,
        env: Env<'a>,
        usb_device: Obj<'_>,
    ) -> JniResult<'a, Vec<Arc<dyn HidDevice>>> {
        let mut devices = Vec::new();
        for device in self.devices() {
            let device_usb = device.get_device(env);
            if java_equals(
                env,
                usb_device,
                device_usb.as_ref().map_or(Obj::NULL, Local::obj),
            )? {
                devices.push(device);
            }
        }
        Ok(devices)
    }

    /// Translation of `handleUsbDeviceDetached()`.
    fn handle_usb_device_detached<'a>(
        &self,
        env: Env<'a>,
        usb_device: Obj<'_>,
    ) -> JniResult<'a, ()> {
        let devices: Vec<jint> = self
            .devices_of(env, usb_device)?
            .iter()
            .map(|device| device.get_id())
            .collect();
        for id in devices {
            let device = lock(&self.devices_by_id).remove(&id);
            let Some(device) = device else {
                return null_pointer(env, c"shutdown() on a null device");
            };
            device.shutdown(env)?;
            hid::hid_device_disconnected(id);
        }
        Ok(())
    }

    /// Translation of `handleUsbDevicePermission()`.
    fn handle_usb_device_permission<'a>(
        &self,
        env: Env<'a>,
        usb_device: Obj<'_>,
        permission_granted: bool,
    ) -> JniResult<'a, ()> {
        for device in self.devices_of(env, usb_device)? {
            let mut opened = false;
            if permission_granted {
                opened = device.open(env)?;
            }
            hid::hid_device_open_result(device.get_id(), opened);
        }
        Ok(())
    }

    /// Translation of `connectHIDDeviceUSB()` (synchronized).
    fn connect_hid_device_usb<'a>(&self, env: Env<'a>, usb_device: Obj<'_>) -> JniResult<'a, ()> {
        let _synchronized = self.monitor.lock();
        let usb_device = non_null(env, usb_device, c"getInterfaceCount() on a null device")?;
        let vendor_id = env
            .call_method(usb_device, c"getVendorId", c"()I", &[])?
            .int();
        let mut interface_mask: jint = 0;
        let interface_count = env
            .call_method(usb_device, c"getInterfaceCount", c"()I", &[])?
            .int();
        for interface_index in 0..interface_count {
            let usb_interface = env
                .call_method(
                    usb_device,
                    c"getInterface",
                    c"(I)Landroid/hardware/usb/UsbInterface;",
                    &[Arg::Int(interface_index)],
                )?
                .object()
                .expect("an interface");
            let info = UsbInterfaceInfo::of(env, usb_interface.obj())?;
            if is_hid_device_interface(vendor_id, info) {
                // Check to see if we've already added this interface
                // This happens with the Xbox Series X controller which has a duplicate interface 0, which is inactive
                let interface_id = info.id;
                // (Java's shift, of the low 5 bits)
                let bit = 1i32.wrapping_shl(interface_id as u32);
                if (interface_mask & bit) != 0 {
                    continue;
                }
                interface_mask |= bit;

                let device = HidDeviceUsb::new(env, self, usb_device, interface_index)?;
                let id = device.get_id();
                lock(&self.devices_by_id).insert(id, device.clone());
                let identifier = device.get_identifier(env)?;
                let serial_number = device.get_serial_number(env)?;
                let manufacturer = device.get_manufacturer_name(env)?;
                let product = device.get_product_name(env)?;
                hid::hid_device_connected(&hid::ConnectedDevice {
                    device_id: id,
                    identifier: &identifier,
                    vendor_id: device.get_vendor_id(env)?,
                    product_id: device.get_product_id(env)?,
                    serial_number: &serial_number,
                    release_number: device.get_version(),
                    manufacturer: &manufacturer,
                    product: &product,
                    interface: info.id,
                    interface_class: info.class,
                    interface_subclass: info.subclass,
                    interface_protocol: info.protocol,
                    bluetooth: false,
                    report_id: 0,
                });
            }
        }
        Ok(())
    }

    /// `checkPermission(permission, getPackageName()) == PERMISSION_GRANTED`.
    fn has_permission<'a>(&self, env: Env<'a>, permission: &str) -> JniResult<'a, bool> {
        let package_manager = self.package_manager(env)?;
        let package_name = env
            .call_method(
                self.context.obj(),
                c"getPackageName",
                c"()Ljava/lang/String;",
                &[],
            )?
            .object();
        let permission = env.new_string(permission)?;
        Ok(env
            .call_method(
                package_manager.obj(),
                c"checkPermission",
                c"(Ljava/lang/String;Ljava/lang/String;)I",
                &[
                    Arg::Object(permission.obj()),
                    Arg::Object(package_name.as_ref().map_or(Obj::NULL, Local::obj)),
                ],
            )?
            .int()
            == PERMISSION_GRANTED)
    }

    fn package_manager<'a>(&self, env: Env<'a>) -> JniResult<'a, Local<'a>> {
        Ok(env
            .call_method(
                self.context.obj(),
                c"getPackageManager",
                c"()Landroid/content/pm/PackageManager;",
                &[],
            )?
            .object()
            .expect("a package manager"))
    }

    /// Translation of `initializeBluetooth()`.
    fn initialize_bluetooth<'a>(&self, env: Env<'a>) -> JniResult<'a, ()> {
        log(LogPriority::Debug, "Initializing Bluetooth");

        if sdk_int(env) >= 31 /* Android 12  */
            && !self.has_permission(env, "android.permission.BLUETOOTH_CONNECT")?
        {
            log(
                LogPriority::Debug,
                "Couldn't initialize Bluetooth, missing android.permission.BLUETOOTH_CONNECT",
            );
            return Ok(());
        }

        if sdk_int(env) <= 30 /* Android 11.0 (R) */
            && !self.has_permission(env, "android.permission.BLUETOOTH")?
        {
            log(
                LogPriority::Debug,
                "Couldn't initialize Bluetooth, missing android.permission.BLUETOOTH",
            );
            return Ok(());
        }

        let package_manager = self.package_manager(env)?;
        let feature = env.new_string(FEATURE_BLUETOOTH_LE)?;
        if !env
            .call_method(
                package_manager.obj(),
                c"hasSystemFeature",
                c"(Ljava/lang/String;)Z",
                &[Arg::Object(feature.obj())],
            )?
            .boolean()
        {
            log(
                LogPriority::Debug,
                "Couldn't initialize Bluetooth, this version of Android does not support Bluetooth LE",
            );
            return Ok(());
        }

        // Find bonded bluetooth controllers and create SteamControllers for them
        let bluetooth_manager = system_service(env, self.context.obj(), BLUETOOTH_SERVICE)?;
        *lock(&self.bluetooth_manager) = bluetooth_manager.as_ref().map(|m| global(env, m.obj()));
        let Some(bluetooth_manager) = bluetooth_manager else {
            // This device doesn't support Bluetooth.
            return Ok(());
        };

        let bt_adapter = env
            .call_method(
                bluetooth_manager.obj(),
                c"getAdapter",
                c"()Landroid/bluetooth/BluetoothAdapter;",
                &[],
            )?
            .object();
        let Some(bt_adapter) = bt_adapter else {
            // This device has Bluetooth support in the codebase, but has no available adapters.
            return Ok(());
        };

        // Get our bonded devices.
        let bonded = env
            .call_method(
                bt_adapter.obj(),
                c"getBondedDevices",
                c"()Ljava/util/Set;",
                &[],
            )?
            .object();
        for device in collection(env, bonded.as_ref().map_or(Obj::NULL, Local::obj))? {
            log(
                LogPriority::Debug,
                &format!(
                    "Bluetooth device available: {}",
                    string_of(env, device.obj())?
                ),
            );
            if self.is_steam_controller(env, device.obj())? {
                self.connect_bluetooth_device(env, device.obj())?;
            }
        }

        // NOTE: These don't work on Chromebooks, to my undying dismay.
        register_receiver(
            env,
            self.context.obj(),
            self.bluetooth_broadcast.obj(),
            &[ACTION_ACL_CONNECTED, ACTION_ACL_DISCONNECTED],
        )?;

        if self.is_chromebook {
            *lock(&self.handler) = Some(global(env, main_handler(env)?.obj()));
            *lock(&self.last_bluetooth_devices) = Some(Vec::new());

            // (a commented-out postDelayed() runs chromebookConnectionHandler() in 5 s)
        }
        Ok(())
    }

    /// Translation of `shutdownBluetooth()`.
    fn shutdown_bluetooth<'a>(&self, env: Env<'a>) -> JniResult<'a, ()> {
        unregister_receiver(env, self.context.obj(), self.bluetooth_broadcast.obj())
    }

    /// Chromebooks do not pass along ACTION_ACL_CONNECTED / ACTION_ACL_DISCONNECTED properly.
    /// This function provides a sort of dummy version of that, watching for changes in the
    /// connected devices and attempting to add controllers as things change.
    /// Translation of `chromebookConnectionHandler()` (Note (upstream):
    /// never called, its first call is commented out).
    #[allow(dead_code)]
    fn chromebook_connection_handler<'a>(&self, env: Env<'a>) -> JniResult<'a, ()> {
        if !self.is_chromebook {
            return Ok(());
        }

        let Some(bluetooth_manager) = lock(&self.bluetooth_manager)
            .as_ref()
            .and_then(|m| env.new_local_ref(m.obj()))
        else {
            return null_pointer(env, c"getConnectedDevices() on a null manager");
        };
        let current = env
            .call_method(
                bluetooth_manager.obj(),
                c"getConnectedDevices",
                c"(I)Ljava/util/List;",
                &[Arg::Int(GATT)],
            )?
            .object();
        let current_connected = collection(env, current.as_ref().map_or(Obj::NULL, Local::obj))?;
        let last: Vec<Local<'a>> = lock(&self.last_bluetooth_devices)
            .as_ref()
            .map(|devices| {
                devices
                    .iter()
                    .filter_map(|d| env.new_local_ref(d.obj()))
                    .collect()
            })
            .unwrap_or_default();

        let contains = |list: &[Local<'_>], device: Obj<'_>| -> JniResult<'a, bool> {
            for d in list {
                if java_equals(env, device, d.obj())? {
                    return Ok(true);
                }
            }
            Ok(false)
        };
        let mut connected = Vec::new();
        let mut disconnected = Vec::new();
        for bluetooth_device in &current_connected {
            if !contains(&last, bluetooth_device.obj())? {
                connected.push(bluetooth_device);
            }
        }
        for bluetooth_device in &last {
            if !contains(&current_connected, bluetooth_device.obj())? {
                disconnected.push(bluetooth_device);
            }
        }

        *lock(&self.last_bluetooth_devices) = Some(
            current_connected
                .iter()
                .map(|d| global(env, d.obj()))
                .collect(),
        );

        for bluetooth_device in disconnected {
            self.disconnect_bluetooth_device(env, bluetooth_device.obj())?;
        }
        for bluetooth_device in connected {
            self.connect_bluetooth_device(env, bluetooth_device.obj())?;
        }

        let final_this = self.weak_self.clone();
        let handler = lock(&self.handler)
            .as_ref()
            .and_then(|h| env.new_local_ref(h.obj()));
        let Some(handler) = handler else {
            return null_pointer(env, c"postDelayed() on a null handler");
        };
        post_delayed(env, handler.obj(), 10000, move |env| {
            if let Some(final_this) = final_this.upgrade() {
                if let Err(thrown) = final_this.chromebook_connection_handler(env) {
                    // (thrown in the main thread's Java)
                    drop(thrown);
                }
            }
        })
    }

    /// Translation of `connectBluetoothDevice()`.
    pub(super) fn connect_bluetooth_device<'a>(
        &self,
        env: Env<'a>,
        bluetooth_device: Obj<'_>,
    ) -> JniResult<'a, bool> {
        log(
            LogPriority::Verbose,
            &format!(
                "connectBluetoothDevice device={}",
                string_of(env, bluetooth_device)?
            ),
        );
        let _synchronized = self.monitor.lock();
        let address = device_address(env, bluetooth_device)?;
        let existing = lock(&self.bluetooth_devices).get(&address).cloned();
        if let Some(device) = existing {
            log(
                LogPriority::Verbose,
                &format!(
                    "Steam controller with address {} already exists, attempting reconnect",
                    string_of(env, bluetooth_device)?
                ),
            );

            device.reconnect(env)?;

            return Ok(false);
        }
        let device = HidDeviceBleSteamController::new(env, self, bluetooth_device)?;
        let id = device.get_id();
        lock(&self.bluetooth_devices).insert(address, device.clone());
        lock(&self.devices_by_id).insert(id, device);

        // The Steam Controller will mark itself connected once initialization is complete
        Ok(true)
    }

    /// Translation of `disconnectBluetoothDevice()`.
    pub(super) fn disconnect_bluetooth_device<'a>(
        &self,
        env: Env<'a>,
        bluetooth_device: Obj<'_>,
    ) -> JniResult<'a, ()> {
        let _synchronized = self.monitor.lock();
        let address = device_address(env, bluetooth_device)?;
        let Some(device) = lock(&self.bluetooth_devices).get(&address).cloned() else {
            return Ok(());
        };

        let id = device.get_id();
        lock(&self.bluetooth_devices).remove(&address);
        lock(&self.devices_by_id).remove(&id);
        device.shutdown(env)?;
        hid::hid_device_disconnected(id);
        Ok(())
    }

    /// Translation of `isSteamController()`.
    fn is_steam_controller<'a>(
        &self,
        env: Env<'a>,
        bluetooth_device: Obj<'_>,
    ) -> JniResult<'a, bool> {
        // Sanity check.  If you pass in a null device, by definition it is never a Steam Controller.
        if bluetooth_device.is_null() {
            return Ok(false);
        }

        let name = call_string(env, bluetooth_device, c"getName")?;
        if name.is_none() {
            return Ok(false);
        }
        let device_type = env
            .call_method(bluetooth_device, c"getType", c"()I", &[])?
            .int();
        Ok(is_steam_controller_name(name.as_deref(), device_type))
    }

    /// Translation of `close()`.
    fn close<'a>(&self, env: Env<'a>) -> JniResult<'a, ()> {
        self.shutdown_usb(env)?;
        self.shutdown_bluetooth(env)?;
        let _synchronized = self.monitor.lock();
        for device in self.devices() {
            device.shutdown(env)?;
        }
        lock(&self.devices_by_id).clear();
        lock(&self.bluetooth_devices).clear();
        hid::hid_device_release_callback(env, self.this.obj());
        Ok(())
    }

    /// Translation of `setFrozen()`.
    pub(crate) fn set_frozen<'a>(&self, env: Env<'a>, frozen: bool) -> JniResult<'a, ()> {
        let _synchronized = self.monitor.lock();
        for device in self.devices() {
            device.set_frozen(env, frozen)?;
        }
        Ok(())
    }

    /// Translation of `getDevice()` (synchronized).
    fn get_device(&self, id: jint) -> Option<Arc<dyn HidDevice>> {
        let _synchronized = self.monitor.lock();
        let result = lock(&self.devices_by_id).get(&id).cloned();
        if result.is_none() {
            log(LogPriority::Verbose, &format!("No device for id: {id}"));
            // (the key set's string: Java's HashMap of small IDs keeps them in order)
            let mut ids: Vec<jint> = lock(&self.devices_by_id).keys().copied().collect();
            ids.sort_unstable();
            let ids: Vec<String> = ids.iter().map(jint::to_string).collect();
            log(
                LogPriority::Verbose,
                &format!("Available devices: [{}]", ids.join(", ")),
            );
        }
        result
    }

    // JNI interface functions

    /// Translation of `initialize()`.
    fn initialize<'a>(&self, env: Env<'a>, usb: bool, bluetooth: bool) -> JniResult<'a, bool> {
        log(
            LogPriority::Verbose,
            &format!("initialize({usb}, {bluetooth})"),
        );

        if usb {
            self.initialize_usb(env)?;
        }
        if bluetooth {
            self.initialize_bluetooth(env)?;
        }
        Ok(true)
    }

    /// Translation of `openDevice()`.
    fn open_device<'a>(&self, env: Env<'a>, device_id: jint) -> JniResult<'a, bool> {
        log(
            LogPriority::Verbose,
            &format!("openDevice deviceID={device_id}"),
        );
        let Some(device) = self.get_device(device_id) else {
            hid::hid_device_disconnected(device_id);
            return Ok(false);
        };

        // Look to see if this is a USB device and we have permission to access it
        if let Some(usb_device) = device.get_device(env) {
            let Some(usb_manager) = self.get_usb_manager(env) else {
                return null_pointer(env, c"hasPermission() on a null manager");
            };
            let has_permission = env
                .call_method(
                    usb_manager.obj(),
                    c"hasPermission",
                    c"(Landroid/hardware/usb/UsbDevice;)Z",
                    &[Arg::Object(usb_device.obj())],
                )?
                .boolean();
            if !has_permission {
                hid::hid_device_open_pending(device_id);
                if let Err(thrown) =
                    self.request_permission(env, usb_manager.obj(), usb_device.obj())
                {
                    drop(thrown.catch_exception()?);
                    log(
                        LogPriority::Verbose,
                        &format!(
                            "Couldn't request permission for USB device {}",
                            string_of(env, usb_device.obj())?
                        ),
                    );
                    hid::hid_device_open_result(device_id, false);
                }
                return Ok(false);
            }
        }

        match device.open(env) {
            Ok(opened) => Ok(opened),
            Err(thrown) => {
                log_exception(env, thrown)?;
                Ok(false)
            }
        }
    }

    /// `openDevice()`'s permission request.
    fn request_permission<'a>(
        &self,
        env: Env<'a>,
        usb_manager: Obj<'_>,
        usb_device: Obj<'_>,
    ) -> JniResult<'a, ()> {
        let flags = if sdk_int(env) >= 31
        /* Android 12.0 (S) */
        {
            FLAG_MUTABLE
        } else {
            0
        };

        let intent_class = env.find_class(c"android/content/Intent")?;
        let action = env.new_string(ACTION_USB_PERMISSION)?;
        let intent = env.new_object(
            intent_class.obj(),
            c"(Ljava/lang/String;)V",
            &[Arg::Object(action.obj())],
        )?;
        let package_name = env
            .call_method(
                self.context.obj(),
                c"getPackageName",
                c"()Ljava/lang/String;",
                &[],
            )?
            .object();
        env.call_method(
            intent.obj(),
            c"setPackage",
            c"(Ljava/lang/String;)Landroid/content/Intent;",
            &[Arg::Object(
                package_name.as_ref().map_or(Obj::NULL, Local::obj),
            )],
        )?;
        let pending_intent_class = env.find_class(c"android/app/PendingIntent")?;
        let pending_intent = env
            .call_static_method(
                pending_intent_class.obj(),
                c"getBroadcast",
                c"(Landroid/content/Context;ILandroid/content/Intent;I)Landroid/app/PendingIntent;",
                &[
                    Arg::Object(self.context.obj()),
                    Arg::Int(0),
                    Arg::Object(intent.obj()),
                    Arg::Int(flags),
                ],
            )?
            .object();
        env.call_method(
            usb_manager,
            c"requestPermission",
            c"(Landroid/hardware/usb/UsbDevice;Landroid/app/PendingIntent;)V",
            &[
                Arg::Object(usb_device),
                Arg::Object(pending_intent.as_ref().map_or(Obj::NULL, Local::obj)),
            ],
        )?;
        Ok(())
    }

    /// Translation of `writeReport()`.
    fn write_report<'a>(
        &self,
        env: Env<'a>,
        device_id: jint,
        report: &[u8],
        feature: bool,
    ) -> JniResult<'a, jint> {
        // (Log.v("writeReport deviceID=" + deviceID + " length=" + report.length), commented out)
        let Some(device) = self.get_device(device_id) else {
            hid::hid_device_disconnected(device_id);
            return Ok(-1);
        };

        match device.write_report(env, report, feature) {
            Ok(written) => Ok(written),
            Err(thrown) => {
                log_exception(env, thrown)?;
                Ok(-1)
            }
        }
    }

    /// Translation of `readReport()`.
    fn read_report<'a>(
        &self,
        env: Env<'a>,
        device_id: jint,
        report: &[u8],
        feature: bool,
    ) -> JniResult<'a, bool> {
        // (Log.v("readReport deviceID=" + deviceID), commented out)
        let Some(device) = self.get_device(device_id) else {
            hid::hid_device_disconnected(device_id);
            return Ok(false);
        };

        match device.read_report(env, report, feature) {
            Ok(read) => Ok(read),
            Err(thrown) => {
                log_exception(env, thrown)?;
                Ok(false)
            }
        }
    }

    /// Translation of `closeDevice()`.
    fn close_device<'a>(&self, env: Env<'a>, device_id: jint) -> JniResult<'a, ()> {
        log(
            LogPriority::Verbose,
            &format!("closeDevice deviceID={device_id}"),
        );
        let Some(device) = self.get_device(device_id) else {
            hid::hid_device_disconnected(device_id);
            return Ok(());
        };

        if let Err(thrown) = device.close(env) {
            log_exception(env, thrown)?;
        }
        Ok(())
    }
}

/// `bluetoothDevice.getAddress()`: a `BluetoothDevice`'s key (its
/// `equals()` and `hashCode()` are its address's).
fn device_address<'a>(env: Env<'a>, bluetooth_device: Obj<'_>) -> JniResult<'a, String> {
    let bluetooth_device = non_null(env, bluetooth_device, c"getAddress() on a null device")?;
    Ok(call_string(env, bluetooth_device, c"getAddress")?.unwrap_or_default())
}

// The methods hid.cpp calls through JNI, on the manager it holds (Note:
// with no manager of that object, a released one's, they do nothing).

/// `initialize(usb, bluetooth)`.
pub(crate) fn initialize<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    usb: bool,
    bluetooth: bool,
) -> JniResult<'a, bool> {
    match manager_of(env, this) {
        Some(manager) => manager.initialize(env, usb, bluetooth),
        None => Ok(false),
    }
}

/// `openDevice(deviceID)`.
pub(crate) fn open_device<'a>(env: Env<'a>, this: Obj<'_>, device_id: jint) -> JniResult<'a, bool> {
    match manager_of(env, this) {
        Some(manager) => manager.open_device(env, device_id),
        None => Ok(false),
    }
}

/// `writeReport(deviceID, report, feature)`.
pub(crate) fn write_report<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    device_id: jint,
    report: &[u8],
    feature: bool,
) -> JniResult<'a, jint> {
    match manager_of(env, this) {
        Some(manager) => manager.write_report(env, device_id, report, feature),
        None => Ok(-1),
    }
}

/// `readReport(deviceID, report, feature)`.
pub(crate) fn read_report<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    device_id: jint,
    report: &[u8],
    feature: bool,
) -> JniResult<'a, bool> {
    match manager_of(env, this) {
        Some(manager) => manager.read_report(env, device_id, report, feature),
        None => Ok(false),
    }
}

/// `closeDevice(deviceID)`.
pub(crate) fn close_device<'a>(env: Env<'a>, this: Obj<'_>, device_id: jint) -> JniResult<'a, ()> {
    match manager_of(env, this) {
        Some(manager) => manager.close_device(env, device_id),
        None => Ok(()),
    }
}

/// A native method's body: an exception it throws is left pending, for
/// the Java code it returns to.
fn native<'a, T: Default>(env: Env<'a>, body: impl FnOnce(Env<'a>) -> JniResult<'a, T>) -> T {
    body(env).unwrap_or_else(|thrown| {
        // (pending, for the caller)
        drop(thrown);
        T::default()
    })
}

/// The Java `byte[]`'s bytes.
fn java_bytes<'a>(env: Env<'a>, array: Obj<'_>) -> JniResult<'a, Vec<u8>> {
    let array = non_null(env, array, c"a null report")?;
    env.get_byte_array(array)
}

/// `HIDDeviceManager.initialize()`, for Java.
pub(crate) extern "system" fn initialize_java(
    env: *mut JNIEnv,
    this: jobject,
    usb: jboolean,
    bluetooth: jboolean,
) -> jboolean {
    // SAFETY: a native method's environment and arguments.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    native(env, |env| {
        initialize(env, this, usb != JNI_FALSE, bluetooth != JNI_FALSE)
    }) as jboolean
}

/// `HIDDeviceManager.openDevice()`, for Java.
pub(crate) extern "system" fn open_device_java(
    env: *mut JNIEnv,
    this: jobject,
    device_id: jint,
) -> jboolean {
    // SAFETY: a native method's environment and arguments.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    native(env, |env| open_device(env, this, device_id)) as jboolean
}

/// `HIDDeviceManager.writeReport()`, for Java.
pub(crate) extern "system" fn write_report_java(
    env: *mut JNIEnv,
    this: jobject,
    device_id: jint,
    report: jbyteArray,
    feature: jboolean,
) -> jint {
    // SAFETY: a native method's environment and arguments.
    let (env, this, report) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(this),
            Obj::from_raw(report),
        )
    };
    native(env, |env| {
        let report = java_bytes(env, report)?;
        write_report(env, this, device_id, &report, feature != JNI_FALSE)
    })
}

/// `HIDDeviceManager.readReport()`, for Java.
pub(crate) extern "system" fn read_report_java(
    env: *mut JNIEnv,
    this: jobject,
    device_id: jint,
    report: jbyteArray,
    feature: jboolean,
) -> jboolean {
    // SAFETY: a native method's environment and arguments.
    let (env, this, report) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(this),
            Obj::from_raw(report),
        )
    };
    native(env, |env| {
        let report = java_bytes(env, report)?;
        read_report(env, this, device_id, &report, feature != JNI_FALSE)
    }) as jboolean
}

/// `HIDDeviceManager.closeDevice()`, for Java.
pub(crate) extern "system" fn close_device_java(env: *mut JNIEnv, this: jobject, device_id: jint) {
    // SAFETY: a native method's environment and arguments.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    native(env, |env| close_device(env, this, device_id));
}

/// `intent.getParcelableExtra(name)`.
fn parcelable_extra<'a>(
    env: Env<'a>,
    intent: Obj<'_>,
    name: &str,
) -> JniResult<'a, Option<Local<'a>>> {
    let name = env.new_string(name)?;
    Ok(env
        .call_method(
            intent,
            c"getParcelableExtra",
            c"(Ljava/lang/String;)Landroid/os/Parcelable;",
            &[Arg::Object(name.obj())],
        )?
        .object())
}

/// `intent.getAction()` (the `NullPointerException` of `action.equals()`
/// for none).
fn intent_action<'a>(env: Env<'a>, intent: Obj<'_>) -> JniResult<'a, String> {
    let intent = non_null(env, intent, c"getAction() on a null intent")?;
    match call_string(env, intent, c"getAction")? {
        Some(action) => Ok(action),
        None => null_pointer(env, c"equals() on a null action"),
    }
}

/// The singleton's manager, when `this` is its `receiver`.
fn manager_of_receiver(
    env: Env<'_>,
    this: Obj<'_>,
    receiver: fn(&HidDeviceManager) -> &Global,
) -> Option<Arc<HidDeviceManager>> {
    lock(&S_MANAGER)
        .manager
        .as_ref()
        .filter(|m| env.is_same_object(receiver(m).obj(), this))
        .cloned()
}

/// `mUsbBroadcast.onReceive()`.
pub(crate) extern "system" fn usb_broadcast_on_receive(
    env: *mut JNIEnv,
    this: jobject,
    _context: jobject,
    intent: jobject,
) {
    // SAFETY: a native method's environment and arguments.
    let (env, this, intent) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(this),
            Obj::from_raw(intent),
        )
    };
    let Some(manager) = manager_of_receiver(env, this, |m| &m.usb_broadcast) else {
        return;
    };
    native(env, |env| {
        let action = intent_action(env, intent)?;
        if action == ACTION_USB_DEVICE_ATTACHED {
            let usb_device = parcelable_extra(env, intent, USB_EXTRA_DEVICE)?;
            manager.handle_usb_device_attached(
                env,
                usb_device.as_ref().map_or(Obj::NULL, Local::obj),
            )?;
        } else if action == ACTION_USB_DEVICE_DETACHED {
            let usb_device = parcelable_extra(env, intent, USB_EXTRA_DEVICE)?;
            manager.handle_usb_device_detached(
                env,
                usb_device.as_ref().map_or(Obj::NULL, Local::obj),
            )?;
        } else if action == ACTION_USB_PERMISSION {
            let usb_device = parcelable_extra(env, intent, USB_EXTRA_DEVICE)?;
            let name = env.new_string(EXTRA_PERMISSION_GRANTED)?;
            let granted = env
                .call_method(
                    intent,
                    c"getBooleanExtra",
                    c"(Ljava/lang/String;Z)Z",
                    &[Arg::Object(name.obj()), Arg::Boolean(false)],
                )?
                .boolean();
            manager.handle_usb_device_permission(
                env,
                usb_device.as_ref().map_or(Obj::NULL, Local::obj),
                granted,
            )?;
        }
        Ok(())
    });
}

/// `mBluetoothBroadcast.onReceive()`.
pub(crate) extern "system" fn bluetooth_broadcast_on_receive(
    env: *mut JNIEnv,
    this: jobject,
    _context: jobject,
    intent: jobject,
) {
    // SAFETY: a native method's environment and arguments.
    let (env, this, intent) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(this),
            Obj::from_raw(intent),
        )
    };
    let Some(manager) = manager_of_receiver(env, this, |m| &m.bluetooth_broadcast) else {
        return;
    };
    native(env, |env| {
        let action = intent_action(env, intent)?;
        // Bluetooth device was connected. If it was a Steam Controller, handle it
        if action == ACTION_ACL_CONNECTED {
            let device = parcelable_extra(env, intent, BLUETOOTH_EXTRA_DEVICE)?;
            let device = device.as_ref().map_or(Obj::NULL, Local::obj);
            log(
                LogPriority::Debug,
                &format!("Bluetooth device connected: {}", string_of(env, device)?),
            );

            if manager.is_steam_controller(env, device)? {
                manager.connect_bluetooth_device(env, device)?;
            }
        }

        // Bluetooth device was disconnected, remove from controller manager (if any)
        if action == ACTION_ACL_DISCONNECTED {
            let device = parcelable_extra(env, intent, BLUETOOTH_EXTRA_DEVICE)?;
            let device = device.as_ref().map_or(Obj::NULL, Local::obj);
            log(
                LogPriority::Debug,
                &format!("Bluetooth device disconnected: {}", string_of(env, device)?),
            );

            manager.disconnect_bluetooth_device(env, device)?;
        }
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(id: jint, class: jint, subclass: jint, protocol: jint) -> UsbInterfaceInfo {
        UsbInterfaceInfo {
            id,
            class,
            subclass,
            protocol,
        }
    }

    #[test]
    fn hid_interfaces() {
        // Any vendor's HID interface.
        assert!(is_hid_device_interface(0x1234, interface(2, 3, 0, 0)));
        // Xbox 360 controllers, wired and wireless, of the vendors listed.
        assert!(is_hid_device_interface(0x045e, interface(0, 0xff, 93, 1)));
        assert!(is_hid_device_interface(0x9886, interface(1, 0xff, 93, 129)));
        assert!(!is_hid_device_interface(0x1234, interface(0, 0xff, 93, 1)));
        assert!(!is_hid_device_interface(0x045e, interface(0, 0xff, 93, 2)));
        // Xbox One controllers: interface 0 only.
        assert!(is_hid_device_interface(0x045e, interface(0, 0xff, 71, 208)));
        assert!(is_hid_device_interface(0x366c, interface(0, 0xff, 71, 208)));
        assert!(!is_hid_device_interface(
            0x045e,
            interface(1, 0xff, 71, 208)
        ));
        assert!(!is_hid_device_interface(
            0x0079,
            interface(0, 0xff, 71, 208)
        ));
        // Neither.
        assert!(!is_hid_device_interface(0x045e, interface(0, 8, 6, 80)));
    }

    #[test]
    fn steam_controller_names() {
        assert!(is_steam_controller_name(
            Some("SteamController"),
            DEVICE_TYPE_LE
        ));
        assert!(is_steam_controller_name(Some("Steam Ctrl 1234"), 3));
        assert!(!is_steam_controller_name(Some("SteamController"), 1));
        assert!(!is_steam_controller_name(
            Some("Steam Controller"),
            DEVICE_TYPE_LE
        ));
        assert!(!is_steam_controller_name(None, DEVICE_TYPE_LE));
    }
}
