// Rust translation of src/core/linux/SDL_udev.c and SDL_udev.h from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Device discovery and hotplug through libudev, which is loaded at run time
//! (`libudev.so.1`, then `libudev.so.0`), so that SDL still starts on a
//! system without it.
//!
//! To list the properties of a device, try something like:
//! `udevadm info -a -n snd/hwC0D0` (for a sound card),
//! `udevadm info --query=all -n input/event3` (for a keyboard, mouse, etc),
//! `udevadm info --query=property -n input/event2`.
//!
//! The backends register a callback ([`add_callback`]) that is told about
//! added and removed device nodes; [`poll`] runs as part of the event pump.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void, CStr, CString};
use std::ptr::{self, NonNull};
use std::sync::Mutex;

use super::evdev_capabilities::{
    guess_device_class, AbsBits, DeviceClass, EvBits, KeyBits, PropBits, RelBits,
};
use super::input::input_id;
use crate::core::unix::{io_ready, IoReadyFlags};
use crate::error::{Error, Result};
use crate::loadso::SharedObject;
use crate::stdlib::string::{strcasestr, strtol, strtoul};

/// `struct udev` (opaque)
#[repr(C)]
pub(crate) struct udev {
    _private: [u8; 0],
}
/// `struct udev_device` (opaque)
#[repr(C)]
pub(crate) struct udev_device {
    _private: [u8; 0],
}
/// `struct udev_enumerate` (opaque)
#[repr(C)]
pub(crate) struct udev_enumerate {
    _private: [u8; 0],
}
/// `struct udev_list_entry` (opaque)
#[repr(C)]
pub(crate) struct udev_list_entry {
    _private: [u8; 0],
}
/// `struct udev_monitor` (opaque)
#[repr(C)]
pub(crate) struct udev_monitor {
    _private: [u8; 0],
}
/// `struct udev_hwdb` (opaque)
#[repr(C)]
pub(crate) struct udev_hwdb {
    _private: [u8; 0],
}

/// What happened to a device. Translation of `SDL_UDEV_deviceevent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum UdevDeviceEvent {
    /// `SDL_UDEV_DEVICEADDED`
    Added = 1,
    /// `SDL_UDEV_DEVICEREMOVED`
    Removed,
}

/// Told about devices being added or removed: the event, the device's
/// classes (unknown for a removal) and its device node.
/// Translation of `SDL_UDEV_Callback`.
pub(crate) type UdevCallback = fn(UdevDeviceEvent, DeviceClass, &str);

/// The libudev functions SDL uses. Translation of `SDL_UDEV_Symbols`.
#[derive(Clone, Copy)]
pub(crate) struct UdevSymbols {
    pub(crate) udev_device_get_action: unsafe extern "C" fn(*mut udev_device) -> *const c_char,
    pub(crate) udev_device_get_devnode: unsafe extern "C" fn(*mut udev_device) -> *const c_char,
    pub(crate) udev_device_get_driver: unsafe extern "C" fn(*mut udev_device) -> *const c_char,
    pub(crate) udev_device_get_syspath: unsafe extern "C" fn(*mut udev_device) -> *const c_char,
    pub(crate) udev_device_get_subsystem: unsafe extern "C" fn(*mut udev_device) -> *const c_char,
    pub(crate) udev_device_get_parent_with_subsystem_devtype:
        unsafe extern "C" fn(*mut udev_device, *const c_char, *const c_char) -> *mut udev_device,
    pub(crate) udev_device_get_property_value:
        unsafe extern "C" fn(*mut udev_device, *const c_char) -> *const c_char,
    pub(crate) udev_device_get_sysattr_value:
        unsafe extern "C" fn(*mut udev_device, *const c_char) -> *const c_char,
    pub(crate) udev_device_new_from_syspath:
        unsafe extern "C" fn(*mut udev, *const c_char) -> *mut udev_device,
    pub(crate) udev_device_unref: unsafe extern "C" fn(*mut udev_device) -> *mut udev_device,
    pub(crate) udev_enumerate_add_match_property:
        unsafe extern "C" fn(*mut udev_enumerate, *const c_char, *const c_char) -> c_int,
    pub(crate) udev_enumerate_add_match_subsystem:
        unsafe extern "C" fn(*mut udev_enumerate, *const c_char) -> c_int,
    pub(crate) udev_enumerate_get_list_entry:
        unsafe extern "C" fn(*mut udev_enumerate) -> *mut udev_list_entry,
    pub(crate) udev_enumerate_new: unsafe extern "C" fn(*mut udev) -> *mut udev_enumerate,
    pub(crate) udev_enumerate_scan_devices: unsafe extern "C" fn(*mut udev_enumerate) -> c_int,
    pub(crate) udev_enumerate_unref:
        unsafe extern "C" fn(*mut udev_enumerate) -> *mut udev_enumerate,
    pub(crate) udev_list_entry_get_name:
        unsafe extern "C" fn(*mut udev_list_entry) -> *const c_char,
    pub(crate) udev_list_entry_get_value:
        unsafe extern "C" fn(*mut udev_list_entry) -> *const c_char,
    pub(crate) udev_list_entry_get_next:
        unsafe extern "C" fn(*mut udev_list_entry) -> *mut udev_list_entry,
    pub(crate) udev_monitor_enable_receiving: unsafe extern "C" fn(*mut udev_monitor) -> c_int,
    pub(crate) udev_monitor_filter_add_match_subsystem_devtype:
        unsafe extern "C" fn(*mut udev_monitor, *const c_char, *const c_char) -> c_int,
    pub(crate) udev_monitor_get_fd: unsafe extern "C" fn(*mut udev_monitor) -> c_int,
    pub(crate) udev_monitor_new_from_netlink:
        unsafe extern "C" fn(*mut udev, *const c_char) -> *mut udev_monitor,
    pub(crate) udev_monitor_receive_device:
        unsafe extern "C" fn(*mut udev_monitor) -> *mut udev_device,
    pub(crate) udev_monitor_unref: unsafe extern "C" fn(*mut udev_monitor) -> *mut udev_monitor,
    pub(crate) udev_new: unsafe extern "C" fn() -> *mut udev,
    pub(crate) udev_unref: unsafe extern "C" fn(*mut udev) -> *mut udev,
    pub(crate) udev_device_new_from_devnum:
        unsafe extern "C" fn(*mut udev, c_char, libc::dev_t) -> *mut udev_device,
    pub(crate) udev_device_get_devnum: unsafe extern "C" fn(*mut udev_device) -> libc::dev_t,

    pub(crate) udev_hwdb_new: Option<unsafe extern "C" fn(*mut udev) -> *mut udev_hwdb>,
    pub(crate) udev_hwdb_unref: Option<unsafe extern "C" fn(*mut udev_hwdb) -> *mut udev_hwdb>,
    pub(crate) udev_hwdb_get_properties_list_entry:
        Option<unsafe extern "C" fn(*mut udev_hwdb, *const c_char, c_uint) -> *mut udev_list_entry>,
}

impl std::fmt::Debug for UdevSymbols {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UdevSymbols")
    }
}

/// The product information of a device node (`SDL_UDEV_GetProductInfo()`'s
/// out-parameters). Fields udev doesn't know stay zero.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProductInfo {
    /// The vendor, product and version (`ID_VENDOR_ID`, `ID_MODEL_ID`,
    /// `ID_REVISION`); the bus type is never set
    pub(crate) inpid: input_id,
    /// The device's classes
    pub(crate) class: DeviceClass,
    /// The kernel driver, if it was asked for
    pub(crate) driver: Option<String>,
}

/// Translation of `SDL_UDEV_PrivateData`.
struct UdevPrivateData {
    udev_handle: Option<SharedObject>,
    udev: *mut udev,
    udev_mon: *mut udev_monitor,
    ref_count: i32,
    /// Translation of the `first`/`last` callback list
    callbacks: Vec<UdevCallback>,

    // Function pointers (set once the library is loaded)
    syms: Option<UdevSymbols>,
}

// SAFETY: the udev handles are only used with the UDEV mutex held, so one
// thread at a time (upstream uses them unlocked from any thread).
unsafe impl Send for UdevPrivateData {}

/// Translation of `_this`.
static UDEV: Mutex<Option<UdevPrivateData>> = Mutex::new(None);

fn lock() -> std::sync::MutexGuard<'static, Option<UdevPrivateData>> {
    UDEV.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `SDL_UDEV_FALLBACK_LIBS`. (Upstream tries the build
/// environment's libudev, `SDL_UDEV_DYNAMIC`, first; there is no build
/// environment here.)
const SDL_UDEV_LIBS: [&str; 2] = ["libudev.so.1", "libudev.so.0"];

/// Where the symbols are looked up.
enum SymbolSource<'a> {
    /// The libraries already loaded into the process (`SDL_LoadFunction()`
    /// with a NULL handle, which is `dlsym(RTLD_DEFAULT, ...)`)
    Global,
    Object(&'a SharedObject),
}

/// Translation of `SDL_UDEV_load_sym()`.
///
/// # Safety
///
/// `F` must be the function pointer type of the symbol `name`.
unsafe fn load_sym<F: Copy>(source: &SymbolSource<'_>, name: &str) -> Result<F> {
    match source {
        SymbolSource::Object(so) => {
            // SAFETY: the caller guarantees F matches the symbol.
            unsafe { so.function(name) }
        }
        SymbolSource::Global => {
            const {
                assert!(size_of::<F>() == size_of::<*mut c_void>());
            }
            let cname = CString::new(name).map_err(|_| Error::invalid_param("name"))?;
            // SAFETY: cname is NUL-terminated; RTLD_DEFAULT searches the
            // objects already loaded.
            let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, cname.as_ptr()) };
            if p.is_null() {
                // SAFETY: dlerror() returns NULL or a NUL-terminated string.
                let e = unsafe { libc::dlerror() };
                let loaderror = if e.is_null() {
                    String::new()
                } else {
                    // SAFETY: as above; it is copied at once.
                    unsafe { CStr::from_ptr(e) }.to_string_lossy().into_owned()
                };
                return Err(Error::new(format!("Failed loading {name}: {loaderror}")));
            }
            // SAFETY: F is a function pointer type of the symbol's signature
            // (the caller's contract) and has the size of a pointer.
            Ok(unsafe { std::mem::transmute_copy::<*mut c_void, F>(&p) })
        }
    }
}

/// Translation of `SDL_UDEV_load_syms()`.
fn load_syms(source: &SymbolSource<'_>) -> Result<UdevSymbols> {
    macro_rules! sym {
        ($name:ident) => {
            // SAFETY: the field's type is the libudev prototype of the symbol.
            unsafe { load_sym(source, stringify!($name)) }?
        };
    }
    macro_rules! sym_optional {
        ($name:ident) => {
            // SAFETY: as above.
            unsafe { load_sym(source, stringify!($name)) }.ok()
        };
    }

    Ok(UdevSymbols {
        udev_device_get_action: sym!(udev_device_get_action),
        udev_device_get_devnode: sym!(udev_device_get_devnode),
        udev_device_get_driver: sym!(udev_device_get_driver),
        udev_device_get_syspath: sym!(udev_device_get_syspath),
        udev_device_get_subsystem: sym!(udev_device_get_subsystem),
        udev_device_get_parent_with_subsystem_devtype: sym!(
            udev_device_get_parent_with_subsystem_devtype
        ),
        udev_device_get_property_value: sym!(udev_device_get_property_value),
        udev_device_get_sysattr_value: sym!(udev_device_get_sysattr_value),
        udev_device_new_from_syspath: sym!(udev_device_new_from_syspath),
        udev_device_unref: sym!(udev_device_unref),
        udev_enumerate_add_match_property: sym!(udev_enumerate_add_match_property),
        udev_enumerate_add_match_subsystem: sym!(udev_enumerate_add_match_subsystem),
        udev_enumerate_get_list_entry: sym!(udev_enumerate_get_list_entry),
        udev_enumerate_new: sym!(udev_enumerate_new),
        udev_enumerate_scan_devices: sym!(udev_enumerate_scan_devices),
        udev_enumerate_unref: sym!(udev_enumerate_unref),
        udev_list_entry_get_name: sym!(udev_list_entry_get_name),
        udev_list_entry_get_value: sym!(udev_list_entry_get_value),
        udev_list_entry_get_next: sym!(udev_list_entry_get_next),
        udev_monitor_enable_receiving: sym!(udev_monitor_enable_receiving),
        udev_monitor_filter_add_match_subsystem_devtype: sym!(
            udev_monitor_filter_add_match_subsystem_devtype
        ),
        udev_monitor_get_fd: sym!(udev_monitor_get_fd),
        udev_monitor_new_from_netlink: sym!(udev_monitor_new_from_netlink),
        udev_monitor_receive_device: sym!(udev_monitor_receive_device),
        udev_monitor_unref: sym!(udev_monitor_unref),
        udev_new: sym!(udev_new),
        udev_unref: sym!(udev_unref),
        udev_device_new_from_devnum: sym!(udev_device_new_from_devnum),
        udev_device_get_devnum: sym!(udev_device_get_devnum),

        udev_hwdb_new: sym_optional!(udev_hwdb_new),
        udev_hwdb_unref: sym_optional!(udev_hwdb_unref),
        udev_hwdb_get_properties_list_entry: sym_optional!(udev_hwdb_get_properties_list_entry),
    })
}

/// A C string from libudev (NULL is `None`).
///
/// # Safety
///
/// `p` must be NULL or point to a NUL-terminated string.
unsafe fn c_str(p: *const c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        // SAFETY: the caller guarantees a NUL-terminated string.
        Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
    }
}

impl UdevPrivateData {
    fn syms(&self) -> &UdevSymbols {
        self.syms.as_ref().expect("libudev is loaded")
    }

    /// Translation of `SDL_UDEV_hotplug_update_available()`.
    fn hotplug_update_available(&self) -> bool {
        if !self.udev_mon.is_null() {
            // SAFETY: udev_mon is a live monitor.
            let fd = unsafe { (self.syms().udev_monitor_get_fd)(self.udev_mon) };
            if io_ready(fd, IoReadyFlags::READ, 0) > 0 {
                return true;
            }
        }
        false
    }

    /// `udev_device_get_property_value(dev, key)`
    fn property(&self, dev: *mut udev_device, key: &CStr) -> Option<String> {
        // SAFETY: dev is a live device and key is NUL-terminated; the result
        // is NULL or a string owned by dev.
        unsafe {
            c_str((self.syms().udev_device_get_property_value)(
                dev,
                key.as_ptr(),
            ))
        }
    }

    /// `udev_device_get_sysattr_value(dev, attr)`
    fn sysattr(&self, dev: *mut udev_device, attr: &CStr) -> Option<String> {
        // SAFETY: as for property().
        unsafe {
            c_str((self.syms().udev_device_get_sysattr_value)(
                dev,
                attr.as_ptr(),
            ))
        }
    }

    /// Translation of `get_caps()`.
    fn get_caps(&self, pdev: *mut udev_device, attr: &CStr, bitmask: &mut [c_ulong]) {
        bitmask.fill(0);
        let Some(value) = self.sysattr(pdev, attr) else {
            return;
        };

        // (`SDL_strlcpy(text, value, sizeof(text))` into a 4096 byte buffer)
        let mut text = value.as_bytes();
        let mut end = text.len().min(4095);
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        text = &text[..end];
        let mut i = 0;
        while let Some(word) = text.iter().rposition(|&c| c == b' ') {
            let v = strtoul(&text[word + 1..], 16).0 as c_ulong;
            if i < bitmask.len() {
                bitmask[i] = v;
            }
            i += 1;
            text = &text[..word];
        }
        let v = strtoul(text, 16).0 as c_ulong;
        if i < bitmask.len() {
            bitmask[i] = v;
        }
    }

    /// Translation of `guess_device_class()`.
    fn guess_device_class(&self, dev: *mut udev_device) -> DeviceClass {
        let mut bitmask_props: PropBits = Default::default();
        let mut bitmask_ev: EvBits = Default::default();
        let mut bitmask_abs: AbsBits = Default::default();
        let mut bitmask_key: KeyBits = Default::default();
        let mut bitmask_rel: RelBits = Default::default();

        /* walk up the parental chain until we find the real input device; the
         * argument is very likely a subdevice of this, like eventN */
        let mut pdev = dev;
        while !pdev.is_null() && self.sysattr(pdev, c"capabilities/ev").is_none() {
            // SAFETY: pdev is a live device; the parent is owned by it.
            pdev = unsafe {
                (self.syms().udev_device_get_parent_with_subsystem_devtype)(
                    pdev,
                    c"input".as_ptr(),
                    ptr::null(),
                )
            };
        }
        if pdev.is_null() {
            return DeviceClass::UNKNOWN;
        }

        self.get_caps(pdev, c"properties", &mut bitmask_props);
        self.get_caps(pdev, c"capabilities/ev", &mut bitmask_ev);
        self.get_caps(pdev, c"capabilities/abs", &mut bitmask_abs);
        self.get_caps(pdev, c"capabilities/rel", &mut bitmask_rel);
        self.get_caps(pdev, c"capabilities/key", &mut bitmask_key);

        guess_device_class(
            &bitmask_props,
            &bitmask_ev,
            &bitmask_abs,
            &bitmask_key,
            &bitmask_rel,
        )
    }

    /// Translation of `device_class()`.
    fn device_class(&self, dev: *mut udev_device) -> DeviceClass {
        let mut devclass = DeviceClass::UNKNOWN;

        // SAFETY: dev is a live device; the result is NULL or owned by it.
        let subsystem = unsafe { c_str((self.syms().udev_device_get_subsystem)(dev)) };
        let Some(subsystem) = subsystem else {
            return DeviceClass::UNKNOWN;
        };

        if subsystem == "sound" {
            devclass = DeviceClass::SOUND;
        } else if subsystem == "video4linux" {
            if let Some(val) = self.property(dev, c"ID_V4L_CAPABILITIES") {
                if strcasestr(&val, "capture").is_some() {
                    devclass = DeviceClass::VIDEO_CAPTURE;
                }
                // v4l2loopback may report output caps before reporting capture caps
                // and ID_V4L_CAPABILITIES is never updated
                else if strcasestr(&val, "video_output").is_some() {
                    let attr = self.sysattr(dev, c"state");
                    if attr.as_deref() == Some("capture") {
                        devclass = DeviceClass::VIDEO_CAPTURE;
                    }
                }
            }
        } else if subsystem == "input" {
            // udev rules reference: http://cgit.freedesktop.org/systemd/systemd/tree/src/udev/udev-builtin-input_id.c

            let is_set = |key: &CStr| self.property(dev, key).as_deref() == Some("1");

            if is_set(c"ID_INPUT_JOYSTICK") {
                devclass |= DeviceClass::JOYSTICK;
            }

            if is_set(c"ID_INPUT_ACCELEROMETER") {
                devclass |= DeviceClass::ACCELEROMETER;
            }

            if is_set(c"ID_INPUT_MOUSE") {
                devclass |= DeviceClass::MOUSE;
            }

            if is_set(c"ID_INPUT_TOUCHSCREEN") {
                devclass |= DeviceClass::TOUCHSCREEN;
            }

            /* The undocumented rule is:
               - All devices with keys get ID_INPUT_KEY
               - From this subset, if they have ESC, numbers, and Q to D, it also gets ID_INPUT_KEYBOARD

               Ref: http://cgit.freedesktop.org/systemd/systemd/tree/src/udev/udev-builtin-input_id.c#n183
            */
            if is_set(c"ID_INPUT_KEY") {
                devclass |= DeviceClass::HAS_KEYS;
            }

            if is_set(c"ID_INPUT_KEYBOARD") {
                devclass |= DeviceClass::KEYBOARD;
            }

            if devclass.is_empty() {
                // Fall back to old style input classes
                if let Some(val) = self.property(dev, c"ID_CLASS") {
                    if val == "joystick" {
                        devclass = DeviceClass::JOYSTICK;
                    } else if val == "mouse" {
                        devclass = DeviceClass::MOUSE;
                    } else if val == "kbd" {
                        devclass = DeviceClass::HAS_KEYS | DeviceClass::KEYBOARD;
                    }
                } else {
                    // We could be linked with libudev on a system that doesn't have udev running
                    devclass = self.guess_device_class(dev);
                }
            }
        }

        devclass
    }

    /// The part of `device_event()` that reads the device: the node and
    /// classes to tell the callbacks about, if any.
    fn device_event(
        &self,
        event: UdevDeviceEvent,
        dev: *mut udev_device,
    ) -> Option<(UdevDeviceEvent, DeviceClass, String)> {
        let mut devclass = DeviceClass::UNKNOWN;

        // SAFETY: dev is a live device; the result is NULL or owned by it.
        let path = unsafe { c_str((self.syms().udev_device_get_devnode)(dev)) }?;

        if event == UdevDeviceEvent::Added {
            devclass = self.device_class(dev);
            if devclass.is_empty() {
                return None;
            }
        } else {
            // The device has been removed, the class isn't available
        }

        Some((event, devclass, path))
    }

    /// The `udev_device` for a device node, if udev knows it (the
    /// `stat()` and `udev_device_new_from_devnum()` of
    /// `SDL_UDEV_GetProductInfo()`).
    fn device_from_path(&self, device_path: &str) -> Option<NonNull<udev_device>> {
        let metadata = std::fs::metadata(device_path).ok()?;

        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let file_type = metadata.file_type();
        let ty = if file_type.is_block_device() {
            b'b'
        } else if file_type.is_char_device() {
            b'c'
        } else {
            return None;
        };

        // SAFETY: udev is live; the device returned is ours to unref.
        NonNull::new(unsafe {
            (self.syms().udev_device_new_from_devnum)(
                self.udev,
                ty as c_char,
                metadata.rdev() as libc::dev_t,
            )
        })
    }

    /// `udev_device_unref(dev)`
    fn unref_device(&self, dev: *mut udev_device) {
        // SAFETY: dev is a device we hold a reference to.
        unsafe {
            (self.syms().udev_device_unref)(dev);
        }
    }
}

/// Tell the callbacks about a device. Upstream calls them while scanning;
/// here they run with the udev state unlocked, since they call back into
/// this module.
fn dispatch(events: Vec<(UdevDeviceEvent, DeviceClass, String)>) {
    for (event, devclass, path) in events {
        // Process callbacks
        let callbacks = match &*lock() {
            Some(this) => this.callbacks.clone(),
            None => return,
        };
        for callback in callbacks {
            callback(event, devclass, &path);
        }
    }
}

/// Load libudev and start watching for devices (counted: each call needs a
/// [`quit`]). Translation of `SDL_UDEV_Init()`.
pub(crate) fn init() -> Result<()> {
    let mut guard = lock();
    if guard.is_none() {
        *guard = Some(UdevPrivateData {
            udev_handle: None,
            udev: ptr::null_mut(),
            udev_mon: ptr::null_mut(),
            ref_count: 0,
            callbacks: Vec::new(),
            syms: None,
        });

        if let Err(e) = load_library_locked(&mut guard) {
            drop(guard);
            quit();
            return Err(e);
        }

        /* Set up udev monitoring
         * Listen for input devices (mouse, keyboard, joystick, etc) and sound devices
         */
        let this = guard.as_mut().expect("udev data");
        let syms = *this.syms();

        // SAFETY: udev_new has no preconditions.
        this.udev = unsafe { (syms.udev_new)() };
        if this.udev.is_null() {
            drop(guard);
            quit();
            return Err(Error::new("udev_new() failed"));
        }

        // SAFETY: udev is live; the name is NUL-terminated.
        this.udev_mon =
            unsafe { (syms.udev_monitor_new_from_netlink)(this.udev, c"udev".as_ptr()) };
        if this.udev_mon.is_null() {
            drop(guard);
            quit();
            return Err(Error::new("udev_monitor_new_from_netlink() failed"));
        }

        // SAFETY: udev_mon is live; the subsystem names are NUL-terminated
        // and a NULL devtype matches every type.
        unsafe {
            for subsystem in [c"input", c"sound", c"video4linux"] {
                (syms.udev_monitor_filter_add_match_subsystem_devtype)(
                    this.udev_mon,
                    subsystem.as_ptr(),
                    ptr::null(),
                );
            }
            (syms.udev_monitor_enable_receiving)(this.udev_mon);
        }
        drop(guard);

        // Do an initial scan of existing devices
        let scanned = scan();

        guard = lock();
        if guard.is_none() {
            // FIXME (upstream): when the initial scan fails, SDL_UDEV_Scan()
            // calls SDL_UDEV_Quit(), which frees _this, and SDL_UDEV_Init()
            // then dereferences NULL; the error is returned instead.
            return scanned;
        }
    }

    guard.as_mut().expect("udev data").ref_count += 1;

    Ok(())
}

/// Release a reference taken by [`init`], unloading libudev with the last
/// one. Translation of `SDL_UDEV_Quit()`.
pub(crate) fn quit() {
    let mut guard = lock();
    let Some(this) = guard.as_mut() else {
        return;
    };

    this.ref_count -= 1;

    if this.ref_count < 1 {
        if !this.udev_mon.is_null() {
            // SAFETY: udev_mon is live and released once, here.
            unsafe {
                (this.syms().udev_monitor_unref)(this.udev_mon);
            }
            this.udev_mon = ptr::null_mut();
        }
        if !this.udev.is_null() {
            // SAFETY: udev is live and released once, here.
            unsafe {
                (this.syms().udev_unref)(this.udev);
            }
            this.udev = ptr::null_mut();
        }

        // Remove existing devices
        this.callbacks.clear();

        unload_library_locked(this);
        *guard = None;
    }
}

/// Report every existing device to the callbacks as added.
/// Translation of `SDL_UDEV_Scan()`.
pub(crate) fn scan() -> Result<()> {
    let events = {
        let guard = lock();
        let Some(this) = guard.as_ref() else {
            return Ok(());
        };
        let syms = *this.syms();

        // SAFETY: udev is live; the enumeration is ours to unref.
        let enumerate = unsafe { (syms.udev_enumerate_new)(this.udev) };
        if enumerate.is_null() {
            drop(guard);
            // FIXME (upstream): this drops a reference that SDL_UDEV_Scan()'s
            // caller still thinks it holds.
            quit();
            return Err(Error::new("udev_enumerate_new() failed"));
        }

        let mut events = Vec::new();
        // SAFETY: enumerate is live; the list entries are owned by it and
        // stay valid until it is unreffed; their names are NUL-terminated.
        unsafe {
            for subsystem in [c"input", c"sound", c"video4linux"] {
                (syms.udev_enumerate_add_match_subsystem)(enumerate, subsystem.as_ptr());
            }

            (syms.udev_enumerate_scan_devices)(enumerate);
            let devs = (syms.udev_enumerate_get_list_entry)(enumerate);
            let mut item = devs;
            while !item.is_null() {
                let path = (syms.udev_list_entry_get_name)(item);
                let dev = (syms.udev_device_new_from_syspath)(this.udev, path);
                if !dev.is_null() {
                    events.extend(this.device_event(UdevDeviceEvent::Added, dev));
                    this.unref_device(dev);
                }
                item = (syms.udev_list_entry_get_next)(item);
            }

            (syms.udev_enumerate_unref)(enumerate);
        }
        events
    };
    dispatch(events);
    Ok(())
}

/// The vendor, product, version, classes and (if `want_driver`) driver
/// of a device node, or `None` if udev doesn't know it.
/// Translation of `SDL_UDEV_GetProductInfo()`.
pub(crate) fn product_info(device_path: &str, want_driver: bool) -> Option<ProductInfo> {
    let guard = lock();
    let this = guard.as_ref()?;

    let dev = this.device_from_path(device_path)?.as_ptr();
    let mut info = ProductInfo::default();

    if let Some(val) = this.property(dev, c"ID_VENDOR_ID") {
        info.inpid.vendor = strtol(&val, 16).0 as u16;
    }

    if let Some(val) = this.property(dev, c"ID_MODEL_ID") {
        info.inpid.product = strtol(&val, 16).0 as u16;
    }

    if let Some(val) = this.property(dev, c"ID_REVISION") {
        info.inpid.version = strtol(&val, 16).0 as u16;
    }

    if want_driver {
        // SAFETY: dev is live; the result is NULL or owned by it.
        let mut val = unsafe { c_str((this.syms().udev_device_get_driver)(dev)) };
        if val.is_none() {
            val = this.property(dev, c"ID_USB_DRIVER");
        }
        info.driver = val;
    }

    let class_temp = this.device_class(dev);
    if !class_temp.is_empty() {
        info.class = class_temp;
    }

    this.unref_device(dev);

    Some(info)
}

/// The serial number of a device node (`ID_SERIAL_SHORT`), if udev knows
/// one. Translation of `SDL_UDEV_GetProductSerial()`.
pub(crate) fn product_serial(device_path: &str) -> Option<String> {
    let guard = lock();
    let this = guard.as_ref()?;

    let dev = this.device_from_path(device_path)?.as_ptr();

    let result = this
        .property(dev, c"ID_SERIAL_SHORT")
        .filter(|val| !val.is_empty());

    this.unref_device(dev);

    result
}

/// Translation of `SDL_UDEV_UnloadLibrary()`.
fn unload_library_locked(this: &mut UdevPrivateData) {
    this.udev_handle = None;
}

/// Unload libudev. Translation of `SDL_UDEV_UnloadLibrary()`.
pub(crate) fn unload_library() {
    if let Some(this) = lock().as_mut() {
        unload_library_locked(this);
    }
}

/// Translation of `SDL_UDEV_LoadLibrary()`.
fn load_library_locked(guard: &mut Option<UdevPrivateData>) -> Result<()> {
    let Some(this) = guard.as_mut() else {
        return Err(Error::new("UDEV not initialized"));
    };

    // See if there is a udev library already loaded
    if let Ok(syms) = load_syms(&SymbolSource::Global) {
        this.syms = Some(syms);
        return Ok(());
    }

    let mut result = Err(Error::new("UDEV not initialized"));
    if this.udev_handle.is_none() {
        for lib in SDL_UDEV_LIBS {
            match SharedObject::load(lib) {
                Ok(handle) => match load_syms(&SymbolSource::Object(&handle)) {
                    Ok(syms) => {
                        this.syms = Some(syms);
                        this.udev_handle = Some(handle);
                        result = Ok(());
                        break;
                    }
                    Err(e) => {
                        result = Err(e);
                        // (SDL_UDEV_UnloadLibrary(): the handle is dropped)
                    }
                },
                Err(e) => {
                    // Don't call SDL_SetError(): SDL_LoadObject already did.
                    result = Err(e);
                }
            }
        }
    }

    result
}

/// Load libudev. Translation of `SDL_UDEV_LoadLibrary()`.
pub(crate) fn load_library() -> Result<()> {
    load_library_locked(&mut lock())
}

/// Tell the callbacks about the devices added and removed since the last
/// call. Runs as part of the event pump. Translation of `SDL_UDEV_Poll()`.
pub(crate) fn poll() {
    loop {
        let event = {
            let guard = lock();
            let Some(this) = guard.as_ref() else {
                return;
            };

            if !this.hotplug_update_available() {
                break;
            }
            // SAFETY: udev_mon is live (hotplug_update_available() checked);
            // the device returned is ours to unref.
            let dev = unsafe { (this.syms().udev_monitor_receive_device)(this.udev_mon) };
            if dev.is_null() {
                break;
            }
            // SAFETY: dev is live; the result is NULL or owned by it.
            let action = unsafe { c_str((this.syms().udev_device_get_action)(dev)) };

            let event = match action.as_deref() {
                Some("add") => this.device_event(UdevDeviceEvent::Added, dev),
                Some("remove") => this.device_event(UdevDeviceEvent::Removed, dev),
                _ => None,
            };

            this.unref_device(dev);
            event
        };
        dispatch(event.into_iter().collect());
    }
}

/// Register a callback for device additions and removals.
/// Translation of `SDL_UDEV_AddCallback()`.
pub(crate) fn add_callback(cb: UdevCallback) -> Result<()> {
    // FIXME (upstream): SDL_UDEV_AddCallback() dereferences _this without
    // checking that udev is initialized; an error is returned instead.
    let mut guard = lock();
    let this = guard
        .as_mut()
        .ok_or_else(|| Error::new("UDEV not initialized"))?;
    this.callbacks.push(cb);
    Ok(())
}

/// Unregister a callback. Translation of `SDL_UDEV_DelCallback()`.
pub(crate) fn del_callback(cb: UdevCallback) {
    let mut guard = lock();
    let Some(this) = guard.as_mut() else {
        return;
    };

    // found it, remove it.
    if let Some(i) = this
        .callbacks
        .iter()
        .position(|&item| std::ptr::fn_addr_eq(item, cb))
    {
        this.callbacks.remove(i);
    }
}

/// The libudev functions, taking a reference to the udev state that
/// [`release_udev_syms`] drops; they stay valid until then.
/// Translation of `SDL_UDEV_GetUdevSyms()`.
pub(crate) fn get_udev_syms() -> Result<UdevSymbols> {
    if init().is_err() {
        return Err(Error::new("Could not initialize UDEV"));
    }

    Ok(*lock().as_ref().expect("udev data").syms())
}

/// Translation of `SDL_UDEV_ReleaseUdevSyms()`.
pub(crate) fn release_udev_syms() {
    quit();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEEN: AtomicUsize = AtomicUsize::new(0);

    fn count_callback(event: UdevDeviceEvent, class: DeviceClass, path: &str) {
        assert_eq!(event, UdevDeviceEvent::Added);
        assert!(!class.is_empty());
        assert!(!path.is_empty());
        SEEN.fetch_add(1, Ordering::Relaxed);
    }

    #[test]
    fn init_scan_and_quit() {
        let _l = crate::test_support::test_lock();

        if SharedObject::load(SDL_UDEV_LIBS[0]).is_err()
            && SharedObject::load(SDL_UDEV_LIBS[1]).is_err()
        {
            assert!(init().is_err());
            println!("note: libudev isn't installed, skipping the udev test");
            return;
        }
        if let Err(e) = init() {
            // (no netlink socket in some sandboxes)
            println!(
                "note: udev unavailable ({}), skipping the udev test",
                e.message()
            );
            return;
        }
        // Counted: a second init only takes a reference
        init().unwrap();
        add_callback(count_callback).unwrap();
        scan().unwrap();
        // Whatever devices exist were all reported as added with a class
        let seen = SEEN.load(Ordering::Relaxed);
        poll();
        del_callback(count_callback);
        scan().unwrap();
        assert_eq!(SEEN.load(Ordering::Relaxed), seen);
        assert!(get_udev_syms().is_ok());
        release_udev_syms();
        assert!(product_info("/nonexistent", true).is_none());
        assert!(product_serial("/nonexistent").is_none());
        quit();
        quit();
        assert!(lock().is_none());
        // Quitting more often than initializing is harmless
        quit();
        assert!(add_callback(count_callback).is_err());
    }
}
