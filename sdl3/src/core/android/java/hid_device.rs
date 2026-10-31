// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// HIDDevice.java from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `interface HIDDevice`: a device of the HID device manager
//! (`super::hid_device_manager`), a USB device's interface
//! (`super::hid_device_usb`) or a Steam Controller over Bluetooth LE
//! (`super::hid_device_ble_steam`). It is upstream's own interface, which
//! the framework doesn't use: a Rust trait here. The reports are Rust
//! bytes (Note: upstream passes the `byte[]` hid.cpp made of them).

use super::super::env::{Env, JniResult, Local};
use super::super::jni::jint;

/// Translation of `interface HIDDevice`.
pub(crate) trait HidDevice: Send + Sync + std::fmt::Debug {
    fn get_id(&self) -> jint;
    fn get_vendor_id<'a>(&self, env: Env<'a>) -> JniResult<'a, jint>;
    fn get_product_id<'a>(&self, env: Env<'a>) -> JniResult<'a, jint>;
    fn get_serial_number<'a>(&self, env: Env<'a>) -> JniResult<'a, String>;
    fn get_version(&self) -> jint;
    fn get_manufacturer_name<'a>(&self, env: Env<'a>) -> JniResult<'a, String>;
    fn get_product_name<'a>(&self, env: Env<'a>) -> JniResult<'a, String>;
    /// The `UsbDevice`, `None` (null) for a device of another bus.
    fn get_device<'a>(&self, env: Env<'a>) -> Option<Local<'a>>;
    fn open<'a>(&self, env: Env<'a>) -> JniResult<'a, bool>;
    fn write_report<'a>(&self, env: Env<'a>, report: &[u8], feature: bool) -> JniResult<'a, jint>;
    fn read_report<'a>(&self, env: Env<'a>, report: &[u8], feature: bool) -> JniResult<'a, bool>;
    fn set_frozen<'a>(&self, env: Env<'a>, frozen: bool) -> JniResult<'a, ()>;
    fn close<'a>(&self, env: Env<'a>) -> JniResult<'a, ()>;
    fn shutdown<'a>(&self, env: Env<'a>) -> JniResult<'a, ()>;
}
