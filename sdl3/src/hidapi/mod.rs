// Rust translation of src/hidapi/SDL_hidapi.c, SDL_hidapi_c.h and
// include/SDL3/SDL_hidapi.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! HID devices: low level access to USB and Bluetooth "human interface
//! devices" (game controllers, keyboards, ...), as SDL's copy of
//! [hidapi](https://github.com/libusb/hidapi) provides it.
//!
//! [`enumerate`] lists the devices; a [`HidDevice`] is an open device,
//! closed when it is dropped. Reports are plain byte slices: the first byte
//! is the report number (0 for devices that only have one report).
//!
//! The backends are the platform's own HID interfaces: `hidraw` on Linux
//! (devices found through libudev, which is loaded at run time) and the HID
//! class driver on Windows (`hid.dll` and `cfgmgr32.dll`, loaded at run
//! time). The libusb, macOS (IOHIDManager), Android and iOS backends of
//! upstream are not translated: on platforms other than Linux and Windows
//! there are no devices, as in an SDL built without a HID backend.
//!
//! Every function initializes the library on first use; [`init`] and
//! [`exit`] are counted, like upstream's `SDL_hid_init()` and
//! `SDL_hid_exit()`.
//!
//! Original hybrid wrapper for Linux by Valve Software. Their original
//! notes: The libusb version doesn't support Bluetooth, but not all Linux
//! distributions allow access to /dev/hidraw* This merges the two, at a
//! small performance cost, until distributions have granted access to
//! /dev/hidraw*

#[cfg(any(windows, test))]
mod descriptor_reconstruct;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::error::{Error, Result};
use crate::hints;
use crate::joystick::usb_ids::*;
use crate::properties::Properties;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(windows)]
use windows as platform;

#[cfg(windows)]
pub(crate) use windows::find_interface_path;

/// The platform backend of a system without one (upstream's build without
/// `HAVE_PLATFORM_BACKEND`): no devices, and nothing can be opened.
#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    use super::DeviceInfo;
    use crate::error::{Error, Result};

    /// No device can exist.
    #[derive(Debug)]
    pub(super) enum Device {}

    impl Device {
        pub(super) fn write(&self, _data: &[u8]) -> Result<usize> {
            match *self {}
        }
        pub(super) fn read_timeout(&self, _data: &mut [u8], _milliseconds: i32) -> Result<usize> {
            match *self {}
        }
        pub(super) fn read(&self, _data: &mut [u8]) -> Result<usize> {
            match *self {}
        }
        pub(super) fn set_nonblocking(&self, _nonblock: bool) -> Result<()> {
            match *self {}
        }
        pub(super) fn send_feature_report(&self, _data: &[u8]) -> Result<usize> {
            match *self {}
        }
        pub(super) fn get_feature_report(&self, _data: &mut [u8]) -> Result<usize> {
            match *self {}
        }
        pub(super) fn get_input_report(&self, _data: &mut [u8]) -> Result<usize> {
            match *self {}
        }
        pub(super) fn get_manufacturer_string(&self) -> Result<String> {
            match *self {}
        }
        pub(super) fn get_product_string(&self) -> Result<String> {
            match *self {}
        }
        pub(super) fn get_serial_number_string(&self) -> Result<String> {
            match *self {}
        }
        pub(super) fn get_indexed_string(&self, _string_index: i32) -> Result<String> {
            match *self {}
        }
        pub(super) fn get_device_info(&self) -> Result<DeviceInfo> {
            match *self {}
        }
        pub(super) fn get_report_descriptor(&self, _buf: &mut [u8]) -> Result<usize> {
            match *self {}
        }
    }

    pub(super) fn open(
        _vendor_id: u16,
        _product_id: u16,
        _serial_number: Option<&str>,
    ) -> Result<Device> {
        Err(Error::unsupported())
    }

    pub(super) fn open_path(_path: &str) -> Result<Device> {
        Err(Error::unsupported())
    }
}

/// The maximum size of a HID report descriptor (`HID_API_MAX_REPORT_DESCRIPTOR_SIZE`).
pub const MAX_REPORT_DESCRIPTOR_SIZE: usize = 4096;

/// The `libusb_device_handle` of a device opened through libusb, in the
/// device's [`HidDevice::properties`]. (The libusb backend isn't translated,
/// so it is never set.) Translation of `SDL_PROP_HIDAPI_LIBUSB_DEVICE_HANDLE_POINTER`.
pub const PROP_HIDAPI_LIBUSB_DEVICE_HANDLE_POINTER: &str = "SDL.hidapi.libusb.device.handle";

/// The bus a HID device is connected through. Translation of `SDL_hid_bus_type`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(u8)]
pub enum BusType {
    /// Unknown bus type (`SDL_HID_API_BUS_UNKNOWN`)
    #[default]
    Unknown = 0x00,
    /// USB bus (`SDL_HID_API_BUS_USB`).
    /// Specifications: <https://usb.org/hid>
    Usb = 0x01,
    /// Bluetooth or Bluetooth LE bus (`SDL_HID_API_BUS_BLUETOOTH`).
    /// Specifications: <https://www.bluetooth.com/specifications/specs/human-interface-device-profile-1-1-1/>
    Bluetooth = 0x02,
    /// I2C bus (`SDL_HID_API_BUS_I2C`).
    /// Specifications: <https://docs.microsoft.com/previous-versions/windows/hardware/design/dn642101(v=vs.85)>
    I2c = 0x03,
    /// SPI bus (`SDL_HID_API_BUS_SPI`).
    /// Specifications: <https://www.microsoft.com/download/details.aspx?id=103325>
    Spi = 0x04,
}

/// Information about a connected HID device. Translation of
/// `SDL_hid_device_info` (the list it heads is a `Vec` here).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct DeviceInfo {
    /// Platform-specific device path
    pub path: Option<String>,
    /// Device Vendor ID
    pub vendor_id: u16,
    /// Device Product ID
    pub product_id: u16,
    /// Serial Number
    pub serial_number: Option<String>,
    /// Device Release Number in binary-coded decimal, also known as Device
    /// Version Number
    pub release_number: u16,
    /// Manufacturer String
    pub manufacturer_string: Option<String>,
    /// Product string
    pub product_string: Option<String>,
    /// Usage Page for this Device/Interface (Windows/Mac/hidraw only)
    pub usage_page: u16,
    /// Usage for this Device/Interface (Windows/Mac/hidraw only)
    pub usage: u16,
    /// The USB interface which this logical device represents. Valid only
    /// if the device is a USB HID device; -1 in all other cases.
    pub interface_number: i32,
    /// Additional information about the USB interface. Valid on libusb and
    /// Android implementations.
    pub interface_class: i32,
    /// See [`DeviceInfo::interface_class`].
    pub interface_subclass: i32,
    /// See [`DeviceInfo::interface_class`].
    pub interface_protocol: i32,
    /// Underlying bus type
    pub bus_type: BusType,
}

// Discovery

/// Translation of `SDL_HIDAPI_discovery.m_unDeviceChangeCounter` (bumped by
/// the Windows window procedure, so it lives outside the state lock).
static DEVICE_CHANGE_COUNTER: AtomicU32 = AtomicU32::new(0);

/// How hidraw devices are found on Linux. Translation of `LinuxEnumerationMethod`.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LinuxEnumerationMethod {
    Unset,
    Libudev,
    Fallback,
}

/// The udev monitor of the discovery (`m_pUdev`, `m_pUdevMonitor`,
/// `m_nUdevFd` and `usyms`).
#[cfg(target_os = "linux")]
struct UdevDiscovery {
    syms: crate::core::linux::udev::UdevSymbols,
    udev: *mut crate::core::linux::udev::udev,
    monitor: *mut crate::core::linux::udev::udev_monitor,
    fd: std::ffi::c_int,
}

/// The device notification window of the discovery (`m_nThreadID`,
/// `m_wndClass`, `m_hwndMsg`, `m_hNotify`).
#[cfg(windows)]
struct WindowsDiscovery {
    thread_id: crate::thread::ThreadID,
    hinstance: windows_sys::Win32::Foundation::HINSTANCE,
    hwnd: windows_sys::Win32::Foundation::HWND,
    notify: windows_sys::Win32::UI::WindowsAndMessaging::HDEVNOTIFY,
}

/// Translation of `SDL_HIDAPI_discovery` (without the change counter).
struct Discovery {
    initialized: bool,
    can_get_notifications: bool,
    last_detect: u64,
    #[cfg(target_os = "linux")]
    udev: Option<UdevDiscovery>,
    /// Translation of `inotify_fd`.
    #[cfg(target_os = "linux")]
    inotify_fd: std::ffi::c_int,
    #[cfg(windows)]
    windows: Option<WindowsDiscovery>,
}

/// The library state (the file-level statics of `SDL_hidapi.c`).
struct HidapiState {
    /// Translation of `SDL_hidapi_refcount`.
    refcount: i32,
    discovery: Discovery,
    /// Translation of `linux_enumeration_method`.
    #[cfg(target_os = "linux")]
    linux_enumeration_method: LinuxEnumerationMethod,
    /// The hint callbacks `SDL_hid_init()` adds.
    hint_watches: Vec<hints::Callback>,
}

// SAFETY: the udev and window handles are only used with the state mutex
// held, so from one thread at a time.
unsafe impl Send for HidapiState {}

static STATE: Mutex<HidapiState> = Mutex::new(HidapiState {
    refcount: 0,
    discovery: Discovery {
        initialized: false,
        can_get_notifications: false,
        last_detect: 0,
        #[cfg(target_os = "linux")]
        udev: None,
        #[cfg(target_os = "linux")]
        inotify_fd: -1,
        #[cfg(windows)]
        windows: None,
    },
    #[cfg(target_os = "linux")]
    linux_enumeration_method: LinuxEnumerationMethod::Unset,
    hint_watches: Vec::new(),
});

fn lock_state() -> std::sync::MutexGuard<'static, HidapiState> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `ControllerWndProc()`.
#[cfg(windows)]
unsafe extern "system" fn controller_wnd_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DefWindowProcA, DBT_DEVICEARRIVAL, DBT_DEVICEREMOVECOMPLETE, DBT_DEVTYP_DEVICEINTERFACE,
        DEV_BROADCAST_HDR, WM_DEVICECHANGE,
    };

    if message == WM_DEVICECHANGE {
        if wparam == DBT_DEVICEARRIVAL as usize || wparam == DBT_DEVICEREMOVECOMPLETE as usize {
            // SAFETY: for these notifications lparam points to a
            // DEV_BROADCAST_HDR.
            let devicetype = unsafe { (*(lparam as *const DEV_BROADCAST_HDR)).dbch_devicetype };
            if devicetype == DBT_DEVTYP_DEVICEINTERFACE {
                DEVICE_CHANGE_COUNTER.fetch_add(1, Ordering::Relaxed);
            }
        }
        return 1; // TRUE
    }

    // SAFETY: the arguments are the ones the window procedure was given.
    unsafe { DefWindowProcA(hwnd, message, wparam, lparam) }
}

/// The class name of the device detection window.
#[cfg(windows)]
const DETECTION_CLASS_NAME: &[u8] = b"SDL_HIDAPI_DEVICE_DETECTION\0";

/// Translation of `HIDAPI_InitializeDiscovery()`.
fn initialize_discovery(state: &mut HidapiState) {
    let discovery = &mut state.discovery;
    discovery.initialized = true;
    DEVICE_CHANGE_COUNTER.store(1, Ordering::Relaxed);
    discovery.can_get_notifications = false;
    discovery.last_detect = 0;

    #[cfg(windows)]
    {
        use windows_sys::core::GUID;
        use windows_sys::Win32::System::LibraryLoader::GetModuleHandleA;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExA, RegisterClassExA, RegisterDeviceNotificationA,
            DBT_DEVTYP_DEVICEINTERFACE, DEVICE_NOTIFY_ALL_INTERFACE_CLASSES,
            DEVICE_NOTIFY_WINDOW_HANDLE, DEV_BROADCAST_DEVICEINTERFACE_A, HWND_MESSAGE,
            WNDCLASSEXA,
        };

        /// `GUID_DEVINTERFACE_USB_DEVICE`
        const GUID_DEVINTERFACE_USB_DEVICE: GUID = GUID {
            data1: 0xA5DCBF10,
            data2: 0x6530,
            data3: 0x11D2,
            data4: [0x90, 0x1F, 0x00, 0xC0, 0x4F, 0xB9, 0x51, 0xED],
        };

        let thread_id = crate::thread::current_thread_id();

        // SAFETY: a null module name is the executable.
        let hinstance = unsafe { GetModuleHandleA(std::ptr::null()) };
        // SAFETY: WNDCLASSEXA is plain data, all-zero is a valid value.
        let mut wnd_class: WNDCLASSEXA = unsafe { std::mem::zeroed() };
        wnd_class.hInstance = hinstance;
        wnd_class.lpszClassName = DETECTION_CLASS_NAME.as_ptr();
        wnd_class.lpfnWndProc = Some(controller_wnd_proc); // This function is called by windows
        wnd_class.cbSize = size_of::<WNDCLASSEXA>() as u32;

        // SAFETY: the class structure is valid and its name is static.
        unsafe {
            RegisterClassExA(&wnd_class);
        }
        // SAFETY: a message-only window of the class registered above.
        let hwnd = unsafe {
            CreateWindowExA(
                0,
                DETECTION_CLASS_NAME.as_ptr(),
                std::ptr::null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };

        // SAFETY: DEV_BROADCAST_DEVICEINTERFACE_A is plain data.
        let mut dev_broadcast: DEV_BROADCAST_DEVICEINTERFACE_A = unsafe { std::mem::zeroed() };
        dev_broadcast.dbcc_size = size_of::<DEV_BROADCAST_DEVICEINTERFACE_A>() as u32;
        dev_broadcast.dbcc_devicetype = DBT_DEVTYP_DEVICEINTERFACE;
        dev_broadcast.dbcc_classguid = GUID_DEVINTERFACE_USB_DEVICE;

        /* DEVICE_NOTIFY_ALL_INTERFACE_CLASSES is important, makes GUID_DEVINTERFACE_USB_DEVICE ignored,
         * but that seems to be necessary to get a notice after each individual usb input device actually
         * installs, rather than just as the composite device is seen.
         */
        // SAFETY: the filter is valid for the call; the window outlives
        // the registration (it is unregistered first at shutdown).
        let notify = unsafe {
            RegisterDeviceNotificationA(
                hwnd,
                (&dev_broadcast as *const DEV_BROADCAST_DEVICEINTERFACE_A).cast(),
                DEVICE_NOTIFY_WINDOW_HANDLE | DEVICE_NOTIFY_ALL_INTERFACE_CLASSES,
            )
        };
        discovery.can_get_notifications = !notify.is_null();
        discovery.windows = Some(WindowsDiscovery {
            thread_id,
            hinstance,
            hwnd,
            notify,
        });
    }

    #[cfg(target_os = "linux")]
    {
        use crate::core::linux::udev;

        if state.linux_enumeration_method == LinuxEnumerationMethod::Libudev {
            discovery.udev = None;

            if let Ok(syms) = udev::get_udev_syms() {
                // SAFETY: the libudev functions stay loaded while the
                // reference get_udev_syms() took is held.
                let udev = unsafe { (syms.udev_new)() };
                let mut monitor = std::ptr::null_mut();
                let mut fd = -1;
                if !udev.is_null() {
                    // SAFETY: udev is a live context; the name is NUL-terminated.
                    monitor =
                        unsafe { (syms.udev_monitor_new_from_netlink)(udev, c"udev".as_ptr()) };
                    if !monitor.is_null() {
                        // SAFETY: monitor is a live monitor.
                        unsafe {
                            (syms.udev_monitor_enable_receiving)(monitor);
                            fd = (syms.udev_monitor_get_fd)(monitor);
                        }
                        discovery.can_get_notifications = true;
                    }
                }
                discovery.udev = Some(UdevDiscovery {
                    syms,
                    udev,
                    monitor,
                    fd,
                });
            }
        } else {
            // SAFETY: inotify_init1 has no preconditions.
            let inotify_fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
            discovery.inotify_fd = inotify_fd;

            if inotify_fd < 0 {
                crate::log::warn!(
                    crate::log::Category::Input,
                    "Unable to initialize inotify, falling back to polling: {}",
                    std::io::Error::last_os_error()
                );
                return;
            }

            /* We need to watch for attribute changes in addition to
             * creation, because when a device is first created, it has
             * permissions that we can't read. When udev chmods it to
             * something that we maybe *can* read, we'll get an
             * IN_ATTRIB event to tell us. */
            // SAFETY: the fd is ours and the path is NUL-terminated.
            if unsafe {
                libc::inotify_add_watch(
                    inotify_fd,
                    c"/dev".as_ptr(),
                    libc::IN_CREATE | libc::IN_DELETE | libc::IN_MOVE | libc::IN_ATTRIB,
                )
            } < 0
            {
                let error = std::io::Error::last_os_error();
                // SAFETY: the fd is ours.
                unsafe {
                    libc::close(inotify_fd);
                }
                discovery.inotify_fd = -1;
                crate::log::warn!(
                    crate::log::Category::Input,
                    "Unable to add inotify watch, falling back to polling: {error}"
                );
                return;
            }

            discovery.can_get_notifications = true;
        }
    }
}

/// Whether an inotify event name is a hidraw node ("hidraw" and digits).
/// Translation of the `StrHasPrefix()`/`StrIsInteger()` check.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn is_hidraw_node(name: &[u8]) -> bool {
    match name.strip_prefix(b"hidraw") {
        Some(number) => !number.is_empty() && number.iter().all(u8::is_ascii_digit),
        None => false,
    }
}

/// Translation of `HIDAPI_UpdateDiscovery()`.
fn update_discovery(state: &mut HidapiState) {
    if !state.discovery.initialized {
        initialize_discovery(state);
    }
    let discovery = &mut state.discovery;

    if !discovery.can_get_notifications {
        const SDL_HIDAPI_DETECT_INTERVAL_MS: u64 = 3000; // Update every 3 seconds
        let now = crate::timer::ticks_ms();
        if discovery.last_detect == 0
            || now >= discovery.last_detect + SDL_HIDAPI_DETECT_INTERVAL_MS
        {
            DEVICE_CHANGE_COUNTER.fetch_add(1, Ordering::Relaxed);
            discovery.last_detect = now;
        }
        return;
    }

    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageA, GetMessageA, PeekMessageA, TranslateMessage, MSG, PM_NOREMOVE,
        };

        if crate::init::is_video_thread() {
            // just let the usual SDL_PumpEvents loop dispatch these, fixing bug 2998. --ryan.
        } else if let Some(win) = &discovery.windows {
            // We'll only get messages on the same thread that created the window
            if crate::thread::current_thread_id() == win.thread_id {
                // SAFETY: MSG is plain data.
                let mut msg: MSG = unsafe { std::mem::zeroed() };
                // SAFETY: msg is writable; the window belongs to this thread.
                while unsafe { PeekMessageA(&mut msg, win.hwnd, 0, 0, PM_NOREMOVE) } != 0 {
                    // SAFETY: as above.
                    if unsafe { GetMessageA(&mut msg, win.hwnd, 0, 0) } != 0 {
                        // SAFETY: msg was filled in by GetMessageA.
                        unsafe {
                            TranslateMessage(&msg);
                            DispatchMessageA(&msg);
                        }
                    }
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if state.linux_enumeration_method == LinuxEnumerationMethod::Libudev {
            if let Some(u) = &discovery.udev {
                if u.fd >= 0 {
                    /* Drain all notification events.
                     * We don't expect a lot of device notifications so just
                     * do a new discovery on any kind or number of notifications.
                     * This could be made more restrictive if necessary.
                     */
                    loop {
                        let mut poll_udev = libc::pollfd {
                            fd: u.fd,
                            events: libc::POLLIN,
                            revents: 0,
                        };
                        // SAFETY: one valid pollfd.
                        if unsafe { libc::poll(&mut poll_udev, 1, 0) } != 1 {
                            break;
                        }

                        // SAFETY: the monitor is live while the discovery is.
                        let device = unsafe { (u.syms.udev_monitor_receive_device)(u.monitor) };
                        if !device.is_null() {
                            // SAFETY: device is a live udev device; the
                            // action string is copied before the unref.
                            let action = unsafe {
                                let p = (u.syms.udev_device_get_action)(device);
                                (!p.is_null()).then(|| std::ffi::CStr::from_ptr(p).to_owned())
                            };
                            match action.as_deref().map(|a| a.to_bytes()) {
                                None | Some(b"add") | Some(b"remove") => {
                                    DEVICE_CHANGE_COUNTER.fetch_add(1, Ordering::Relaxed);
                                }
                                _ => {}
                            }
                            // SAFETY: we own this reference.
                            unsafe {
                                (u.syms.udev_device_unref)(device);
                            }
                        }
                    }
                }
            }
        } else if discovery.inotify_fd >= 0 {
            const HEADER: usize = size_of::<libc::inotify_event>();
            let mut buf = [0u8; 4096];

            // SAFETY: buf is writable for its length.
            let bytes =
                unsafe { libc::read(discovery.inotify_fd, buf.as_mut_ptr().cast(), buf.len()) };
            let remain = if bytes > 0 { bytes as usize } else { 0 };

            // (upstream moves each handled event out of the buffer; this
            // walks it)
            let mut offset = 0;
            while offset + HEADER <= remain {
                // SAFETY: a whole inotify_event header lies at offset; it
                // may be unaligned in the byte buffer.
                let event: libc::inotify_event =
                    unsafe { std::ptr::read_unaligned(buf.as_ptr().add(offset).cast()) };
                let len = event.len as usize;
                if len > 0 {
                    let end = (offset + HEADER + len).min(remain);
                    let name = &buf[offset + HEADER..end];
                    let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())];
                    if is_hidraw_node(name) {
                        DEVICE_CHANGE_COUNTER.fetch_add(1, Ordering::Relaxed);
                        /* We found an hidraw change. We still continue to
                         * drain the inotify fd to avoid leaving old
                         * notifications in the queue. */
                    }
                }

                offset += HEADER + len;
            }
        }
    }
}

/// Translation of `HIDAPI_ShutdownDiscovery()`.
fn shutdown_discovery(state: &mut HidapiState) {
    if !state.discovery.initialized {
        return;
    }

    #[cfg(windows)]
    if let Some(win) = state.discovery.windows.take() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DestroyWindow, UnregisterClassA, UnregisterDeviceNotification,
        };

        // SAFETY: the handles were created by initialize_discovery().
        unsafe {
            if !win.notify.is_null() {
                UnregisterDeviceNotification(win.notify);
            }

            if !win.hwnd.is_null() {
                DestroyWindow(win.hwnd);
            }

            UnregisterClassA(DETECTION_CLASS_NAME.as_ptr(), win.hinstance);
        }
    }

    #[cfg(target_os = "linux")]
    {
        if state.linux_enumeration_method == LinuxEnumerationMethod::Libudev {
            if let Some(u) = state.discovery.udev.take() {
                // SAFETY: the handles were created by initialize_discovery()
                // and are released once.
                unsafe {
                    if !u.monitor.is_null() {
                        (u.syms.udev_monitor_unref)(u.monitor);
                    }
                    if !u.udev.is_null() {
                        (u.syms.udev_unref)(u.udev);
                    }
                }
                crate::core::linux::udev::release_udev_syms();
            }
        } else if state.discovery.inotify_fd >= 0 {
            // SAFETY: the fd is ours.
            unsafe {
                libc::close(state.discovery.inotify_fd);
            }
            state.discovery.inotify_fd = -1;
        }
    }

    state.discovery.initialized = false;
}

// Device filtering

/// Translation of `SDL_libusb_required`: devices we know to be accessible
/// _exclusively_ via libusb; these are typically devices that look like
/// HIDs but have a quirk that requires direct access to the hardware.
///
/// If the platform has any backend other than libusb, try to avoid using
/// libusb as the main backend for devices, since it detaches drivers and
/// therefore makes devices inaccessible to the rest of the OS.
const LIBUSB_REQUIRED: [(u16, u16); 6] = [
    (USB_VENDOR_NINTENDO, USB_PRODUCT_NINTENDO_GAMECUBE_ADAPTER),
    (
        USB_VENDOR_NINTENDO,
        USB_PRODUCT_NINTENDO_SWITCH2_GAMECUBE_CONTROLLER,
    ),
    (
        USB_VENDOR_NINTENDO,
        USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_LEFT,
    ),
    (
        USB_VENDOR_NINTENDO,
        USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_RIGHT,
    ),
    (USB_VENDOR_NINTENDO, USB_PRODUCT_NINTENDO_SWITCH2_PRO),
    (USB_VENDOR_MICROSOFT, USB_PRODUCT_XBOX360_BIGBUTTON_RECEIVER),
];

/// Translation of `RequiresLibUSB()`.
fn requires_libusb(vendor: u16, product: u16, libusb_xbox: bool) -> bool {
    if LIBUSB_REQUIRED.contains(&(vendor, product)) {
        return true;
    }
    // On macOS we want to use libusb if possible for Xbox controllers
    // that are not supported by the OS. Opening the device via libusb
    // will fail if the device is supported (and opened) by the OS, so
    // any devices we return here and can open are fair game.
    if cfg!(target_os = "macos") && libusb_xbox {
        return true;
    }
    false
}

/// Translation of `SDL_HINT_HIDAPI_LIBUSB_WHITELIST_DEFAULT`: there is a
/// platform backend where one is translated, so the whitelist is used to
/// get the devices where libusb is preferred.
const LIBUSB_WHITELIST_DEFAULT: bool = cfg!(any(target_os = "linux", windows));

/// Translation of `use_libusb_whitelist`.
static USE_LIBUSB_WHITELIST: AtomicBool = AtomicBool::new(LIBUSB_WHITELIST_DEFAULT);
/// Translation of `use_libusb_gamecube`.
static USE_LIBUSB_GAMECUBE: AtomicBool = AtomicBool::new(true);
/// Translation of `SDL_hidapi_only_controllers`.
static ONLY_CONTROLLERS: AtomicBool = AtomicBool::new(false);
/// Translation of `SDL_hidapi_ignored_devices`.
static IGNORED_DEVICES: Mutex<Option<String>> = Mutex::new(None);

/// Whether a device should be left out of the enumeration.
/// Translation of `SDL_HIDAPI_ShouldIgnoreDevice()`.
pub(crate) fn should_ignore_device(
    bus: BusType,
    vendor_id: u16,
    product_id: u16,
    usage_page: u16,
    usage: u16,
    libusb: bool,
    libusb_xbox: bool,
) -> bool {
    if libusb {
        if USE_LIBUSB_WHITELIST.load(Ordering::Relaxed)
            && !requires_libusb(vendor_id, product_id, libusb_xbox)
        {
            return true;
        }
        if !USE_LIBUSB_GAMECUBE.load(Ordering::Relaxed)
            && vendor_id == USB_VENDOR_NINTENDO
            && product_id == USB_PRODUCT_NINTENDO_GAMECUBE_ADAPTER
        {
            return true;
        }
    } else if requires_libusb(vendor_id, product_id, libusb_xbox) {
        return true;
    }

    // See if there are any devices we should skip in enumeration
    if ONLY_CONTROLLERS.load(Ordering::Relaxed) && usage_page != 0 {
        if vendor_id == USB_VENDOR_VALVE {
            // Ignore the mouse/keyboard interface on Steam Controllers
            // (on Windows the usage page and usage are checked on both USB
            // and Bluetooth, elsewhere only on USB)
            if (cfg!(windows) || bus == BusType::Usb)
                && usage_page == USB_USAGEPAGE_GENERIC_DESKTOP
                && (usage == USB_USAGE_GENERIC_KEYBOARD || usage == USB_USAGE_GENERIC_MOUSE)
            {
                return true;
            }
        } else if (vendor_id == USB_VENDOR_FLYDIGI_V1
            && product_id == USB_PRODUCT_FLYDIGI_V1_GAMEPAD)
            || (vendor_id == USB_VENDOR_FLYDIGI_V2
                && (product_id == USB_PRODUCT_FLYDIGI_V2_APEX
                    || product_id == USB_PRODUCT_FLYDIGI_V2_VADER
                    || product_id == USB_PRODUCT_FLYDIGI_V2_APEX6))
        {
            // (two branches upstream, for the V1 and the V2 controllers)
            if usage_page == USB_USAGEPAGE_VENDOR_FLYDIGI {
                return false;
            }
            return true;
        } else if usage_page == USB_USAGEPAGE_GENERIC_DESKTOP
            && (usage == USB_USAGE_GENERIC_JOYSTICK
                || usage == USB_USAGE_GENERIC_GAMEPAD
                || usage == USB_USAGE_GENERIC_MULTIAXISCONTROLLER)
        {
            // This is a controller
        } else {
            return true;
        }
    }

    let ignored = IGNORED_DEVICES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ignored) = ignored.as_deref() {
        let vendor_match = format!("0x{vendor_id:04x}/0x0000");
        let product_match = format!("0x{vendor_id:04x}/0x{product_id:04x}");
        if crate::stdlib::string::strcasestr(ignored, &vendor_match).is_some()
            || crate::stdlib::string::strcasestr(ignored, &product_match).is_some()
        {
            return true;
        }
    }

    false
}

// The library

/// Initialize the HIDAPI library. Calls are counted: each needs an
/// [`exit`]. Calling it is optional, the other functions initialize the
/// library when it isn't. Fails if no backend could start (on Linux, when
/// libudev isn't available). Translation of `SDL_hid_init()`.
pub fn init() -> Result<()> {
    init_locked(&mut lock_state())
}

fn init_locked(state: &mut HidapiState) -> Result<()> {
    if state.refcount > 0 {
        state.refcount += 1;
        return Ok(());
    }

    // (translations of OnlyControllersChanged() and IgnoredDevicesChanged())
    state.hint_watches.clear();
    if let Ok(watch) = hints::watch(hints::HIDAPI_ENUMERATE_ONLY_CONTROLLERS, |change| {
        ONLY_CONTROLLERS.store(
            hints::string_to_bool(change.new_value, true),
            Ordering::Relaxed,
        );
    }) {
        state.hint_watches.push(watch);
    }
    if let Ok(watch) = hints::watch(hints::HIDAPI_IGNORE_DEVICES, |change| {
        *IGNORED_DEVICES.lock().unwrap_or_else(|e| e.into_inner()) = change
            .new_value
            .filter(|hint| !hint.is_empty())
            .map(str::to_owned);
    }) {
        state.hint_watches.push(watch);
    }

    #[cfg(target_os = "linux")]
    {
        if !hints::get_bool(hints::HIDAPI_UDEV, true) {
            crate::log::debug!(
                crate::log::Category::Input,
                "udev disabled by SDL_HINT_HIDAPI_UDEV"
            );
            state.linux_enumeration_method = LinuxEnumerationMethod::Fallback;
        } else if crate::init::sandbox() != crate::init::Sandbox::None {
            crate::log::debug!(
                crate::log::Category::Input,
                "Container detected, disabling HIDAPI udev integration"
            );
            state.linux_enumeration_method = LinuxEnumerationMethod::Fallback;
        } else {
            crate::log::debug!(
                crate::log::Category::Input,
                "Using udev for HIDAPI joystick device discovery"
            );
            state.linux_enumeration_method = LinuxEnumerationMethod::Libudev;
        }
    }

    USE_LIBUSB_WHITELIST.store(
        hints::get_bool(hints::HIDAPI_LIBUSB_WHITELIST, LIBUSB_WHITELIST_DEFAULT),
        Ordering::Relaxed,
    );
    USE_LIBUSB_GAMECUBE.store(
        hints::get_bool(hints::HIDAPI_LIBUSB_GAMECUBE, true),
        Ordering::Relaxed,
    );

    // (the libusb backend isn't translated, which leaves the platform
    // backend's attempt)
    #[cfg(any(target_os = "linux", windows))]
    let (attempts, success) = (1, i32::from(platform::hid_init().is_ok()));
    #[cfg(not(any(target_os = "linux", windows)))]
    let (attempts, success) = (0, 0);

    if attempts > 0 && success == 0 {
        // FIXME (upstream): a failed SDL_hid_init() leaves its hint
        // callbacks registered; they are replaced by the next init.
        return Err(Error::new("Couldn't initialize hidapi"));
    }

    state.refcount += 1;
    Ok(())
}

/// Release a reference taken by [`init`], shutting the library down with
/// the last one. Translation of `SDL_hid_exit()`.
pub fn exit() -> Result<()> {
    let mut state = lock_state();

    if state.refcount == 0 {
        return Ok(());
    }
    state.refcount -= 1;
    if state.refcount > 0 {
        return Ok(());
    }
    state.refcount = 0;

    shutdown_discovery(&mut state);

    #[cfg(any(target_os = "linux", windows))]
    let result = platform::hid_exit();
    #[cfg(not(any(target_os = "linux", windows)))]
    let result = Ok(());

    state.hint_watches.clear();
    *IGNORED_DEVICES.lock().unwrap_or_else(|e| e.into_inner()) = None;

    result
}

/// Initialize the library if it isn't (`SDL_hidapi_refcount == 0 &&
/// SDL_hid_init() < 0`).
fn ensure_init() -> Result<()> {
    let mut state = lock_state();
    if state.refcount == 0 {
        init_locked(&mut state)?;
    }
    Ok(())
}

/// A counter that changes with each potential device change (it may also
/// change when nothing did): devices only need to be enumerated again when
/// it differs from the last value. Zero if the library can't be
/// initialized. Translation of `SDL_hid_device_change_count()`.
pub fn device_change_count() -> u32 {
    let mut state = lock_state();
    if state.refcount == 0 && init_locked(&mut state).is_err() {
        return 0;
    }

    update_discovery(&mut state);

    if DEVICE_CHANGE_COUNTER.load(Ordering::Relaxed) == 0 {
        // Counter wrapped!
        DEVICE_CHANGE_COUNTER.fetch_add(1, Ordering::Relaxed);
    }
    DEVICE_CHANGE_COUNTER.load(Ordering::Relaxed)
}

/// The HID devices attached to the system, of all kinds or matching a
/// vendor and/or product ID (0 matches any). Translation of
/// `SDL_hid_enumerate()`.
pub fn enumerate(vendor_id: u16, product_id: u16) -> Result<Vec<DeviceInfo>> {
    ensure_init()?;

    // Collect the available devices
    // (the libusb and Steam Xbox driver backends aren't translated, which
    // leaves the platform's devices, in their order)
    #[cfg(any(target_os = "linux", windows))]
    let raw_devs = platform::hid_enumerate(vendor_id, product_id);
    #[cfg(not(any(target_os = "linux", windows)))]
    let raw_devs: Vec<DeviceInfo> = {
        let _ = (vendor_id, product_id);
        Vec::new()
    };

    // (AddDeviceToEnumeration() copies each one into the list)
    Ok(raw_devs)
}

/// Start or stop a Bluetooth LE scan on iOS and tvOS to pair Steam
/// Controllers (a no-op elsewhere). Translation of `SDL_hid_ble_scan()`.
pub fn ble_scan(active: bool) {
    // (the iOS backend isn't translated)
    let _ = active;
}

/// An open HID device, closed when dropped. Translation of `SDL_hid_device`.
///
/// The device can be used from several threads at once (the joystick
/// drivers write rumble reports from their own thread while reading input
/// on another).
#[derive(Debug)]
pub struct HidDevice {
    device: platform::Device,
    props: OnceLock<Properties>,
}

impl HidDevice {
    /// Open a device by vendor and product ID and, optionally, serial
    /// number (the first match otherwise). Translation of `SDL_hid_open()`.
    pub fn open(vendor_id: u16, product_id: u16, serial_number: Option<&str>) -> Result<HidDevice> {
        ensure_init()?;
        let device = platform::open(vendor_id, product_id, serial_number)?;
        Ok(HidDevice::wrap(device))
    }

    /// Open a device by its platform-specific path ([`DeviceInfo::path`]).
    /// Translation of `SDL_hid_open_path()`.
    pub fn open_path(path: &str) -> Result<HidDevice> {
        ensure_init()?;
        let device = platform::open_path(path)?;
        Ok(HidDevice::wrap(device))
    }

    /// Translation of `CreateHIDDeviceWrapper()`.
    fn wrap(device: platform::Device) -> HidDevice {
        HidDevice {
            device,
            props: OnceLock::new(),
        }
    }

    /// The properties of the device, created on first use. Translation of
    /// `SDL_hid_get_properties()`.
    pub fn properties(&self) -> Properties {
        self.props.get_or_init(Properties::new).clone()
    }

    /// Write an output report. The first byte is the report number (0 for
    /// devices with a single report), followed by the report data. Returns
    /// the number of bytes written. Translation of `SDL_hid_write()`.
    pub fn write(&self, data: &[u8]) -> Result<usize> {
        self.device.write(data)
    }

    /// Read an input report into `data`, waiting at most `timeout` (`None`
    /// waits until one arrives). Returns the number of bytes read, 0 if no
    /// report arrived in time. For devices with multiple reports, the first
    /// byte is the report number. Translation of `SDL_hid_read_timeout()`.
    pub fn read_timeout(&self, data: &mut [u8], timeout: Option<Duration>) -> Result<usize> {
        let milliseconds = match timeout {
            None => -1,
            Some(timeout) => i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX),
        };
        self.read_timeout_ms(data, milliseconds)
    }

    /// [`HidDevice::read_timeout`] with upstream's timeout in milliseconds
    /// (-1 waits forever).
    pub(crate) fn read_timeout_ms(&self, data: &mut [u8], milliseconds: i32) -> Result<usize> {
        self.device.read_timeout(data, milliseconds)
    }

    /// Read an input report, waiting for one unless the device is in
    /// non-blocking mode ([`HidDevice::set_nonblocking`]). Translation of
    /// `SDL_hid_read()`.
    pub fn read(&self, data: &mut [u8]) -> Result<usize> {
        self.device.read(data)
    }

    /// Make [`HidDevice::read`] return immediately when no report is
    /// waiting. Translation of `SDL_hid_set_nonblocking()`.
    pub fn set_nonblocking(&self, nonblock: bool) -> Result<()> {
        self.device.set_nonblocking(nonblock)
    }

    /// Send a feature report. The first byte is the report number (0 for
    /// devices with a single report). Returns the number of bytes sent.
    /// Translation of `SDL_hid_send_feature_report()`.
    pub fn send_feature_report(&self, data: &[u8]) -> Result<usize> {
        self.device.send_feature_report(data)
    }

    /// Get a feature report: set the first byte of `data` to the report
    /// number to read (0 for devices with a single report). Returns the
    /// number of bytes read, including the report number.
    /// Translation of `SDL_hid_get_feature_report()`.
    pub fn get_feature_report(&self, data: &mut [u8]) -> Result<usize> {
        self.device.get_feature_report(data)
    }

    /// Get an input report: set the first byte of `data` to the report
    /// number to read. Returns the number of bytes read, including the
    /// report number. Translation of `SDL_hid_get_input_report()`.
    pub fn get_input_report(&self, data: &mut [u8]) -> Result<usize> {
        self.device.get_input_report(data)
    }

    /// The manufacturer string. Translation of `SDL_hid_get_manufacturer_string()`.
    pub fn manufacturer_string(&self) -> Result<String> {
        self.device.get_manufacturer_string()
    }

    /// The product string. Translation of `SDL_hid_get_product_string()`.
    pub fn product_string(&self) -> Result<String> {
        self.device.get_product_string()
    }

    /// The serial number string. Translation of `SDL_hid_get_serial_number_string()`.
    pub fn serial_number_string(&self) -> Result<String> {
        self.device.get_serial_number_string()
    }

    /// A string from the device's string descriptors.
    /// Translation of `SDL_hid_get_indexed_string()`.
    pub fn indexed_string(&self, string_index: i32) -> Result<String> {
        self.device.get_indexed_string(string_index)
    }

    /// The information about the device. Translation of `SDL_hid_get_device_info()`.
    pub fn device_info(&self) -> Result<DeviceInfo> {
        self.device.get_device_info()
    }

    /// The report descriptor, copied into `buf` (at most
    /// [`MAX_REPORT_DESCRIPTOR_SIZE`] bytes); returns its length.
    /// Translation of `SDL_hid_get_report_descriptor()`.
    pub fn report_descriptor(&self, buf: &mut [u8]) -> Result<usize> {
        self.device.get_report_descriptor(buf)
    }
}

#[cfg(test)]
mod tests;
