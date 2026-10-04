// Rust translation of src/core/linux/SDL_dbus.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! D-Bus IPC over libdbus, which is loaded at run time (we never link
//! directly to libdbus): the session and system bus connections, method
//! calls and property queries, and the desktop services SDL uses through
//! D-Bus (screensaver inhibition, the OpenURI, Documents and Camera
//! portals, the machine ID).
//!
//! Upstream's variadic `SDL_DBus_CallMethod(..., DBUS_TYPE_x, &in, ...,
//! DBUS_TYPE_INVALID, DBUS_TYPE_y, &out, ..., DBUS_TYPE_INVALID)` becomes a
//! slice of typed [`Arg`]s in, and the reply's decoded [`Value`]s out.
//!
//! The menu export of `SDL_DBus_ExportMenu()` and friends belongs to the
//! D-Bus tray and comes with it.

#![allow(non_camel_case_types)]

use crate::error::{Error, Result};
use crate::loadso::SharedObject;
use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use std::sync::{Arc, Mutex, MutexGuard};

// * * * libdbus declarations (dbus/dbus.h)

type dbus_bool_t = u32;
#[repr(C)]
pub(crate) struct DBusConnection {
    _opaque: [u8; 0],
}
#[repr(C)]
pub(crate) struct DBusMessage {
    _opaque: [u8; 0],
}

/// `DBusError`: two strings, five one-bit fields and a pointer of padding.
#[repr(C)]
struct DBusError {
    name: *const c_char,
    message: *const c_char,
    dummy_bits: c_uint,
    padding1: *mut c_void,
}

impl DBusError {
    fn new() -> DBusError {
        DBusError {
            name: std::ptr::null(),
            message: std::ptr::null(),
            dummy_bits: 0,
            padding1: std::ptr::null_mut(),
        }
    }
}

/// `DBusMessageIter`: opaque, but allocated by the caller.
#[repr(C)]
#[derive(Clone, Copy)]
struct DBusMessageIter {
    dummy1: *mut c_void,
    dummy2: *mut c_void,
    dummy3: u32,
    dummy4: c_int,
    dummy5: c_int,
    dummy6: c_int,
    dummy7: c_int,
    dummy8: c_int,
    dummy9: c_int,
    dummy10: c_int,
    dummy11: c_int,
    pad1: c_int,
    pad2: *mut c_void,
    pad3: *mut c_void,
}

impl DBusMessageIter {
    fn new() -> DBusMessageIter {
        // SAFETY: the iterator is plain data that libdbus initializes.
        unsafe { std::mem::zeroed() }
    }
}

const DBUS_BUS_SESSION: c_int = 0;
const DBUS_BUS_SYSTEM: c_int = 1;
const DBUS_DISPATCH_DATA_REMAINS: c_int = 0;
const DBUS_TIMEOUT_USE_DEFAULT: c_int = -1;
const DBUS_ERROR_NAME_HAS_NO_OWNER: &CStr = c"org.freedesktop.DBus.Error.NameHasNoOwner";

/// A filter's verdict on a message (`DBusHandlerResult`).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum HandlerResult {
    Handled = 0,
    NotYetHandled = 1,
    NeedMemory = 2,
}

type DBusHandleMessageFunction =
    unsafe extern "C" fn(*mut DBusConnection, *mut DBusMessage, *mut c_void) -> HandlerResult;
type DBusFreeFunction = unsafe extern "C" fn(*mut c_void);

// D-Bus type codes (DBUS_TYPE_*).
const TYPE_INVALID: c_int = 0;
const TYPE_BYTE: c_int = b'y' as c_int;
const TYPE_BOOLEAN: c_int = b'b' as c_int;
const TYPE_INT16: c_int = b'n' as c_int;
const TYPE_UINT16: c_int = b'q' as c_int;
const TYPE_INT32: c_int = b'i' as c_int;
const TYPE_UINT32: c_int = b'u' as c_int;
const TYPE_INT64: c_int = b'x' as c_int;
const TYPE_UINT64: c_int = b't' as c_int;
const TYPE_DOUBLE: c_int = b'd' as c_int;
const TYPE_STRING: c_int = b's' as c_int;
const TYPE_OBJECT_PATH: c_int = b'o' as c_int;
const TYPE_SIGNATURE: c_int = b'g' as c_int;
const TYPE_UNIX_FD: c_int = b'h' as c_int;
const TYPE_ARRAY: c_int = b'a' as c_int;
const TYPE_VARIANT: c_int = b'v' as c_int;
const TYPE_STRUCT: c_int = b'r' as c_int;
const TYPE_DICT_ENTRY: c_int = b'e' as c_int;

/// The libdbus entry points SDL uses (`SDL_DBusContext`'s function
/// pointers).
struct Fns {
    bus_get_private: unsafe extern "C" fn(c_int, *mut DBusError) -> *mut DBusConnection,
    bus_register: unsafe extern "C" fn(*mut DBusConnection, *mut DBusError) -> dbus_bool_t,
    bus_add_match: unsafe extern "C" fn(*mut DBusConnection, *const c_char, *mut DBusError),
    bus_remove_match: unsafe extern "C" fn(*mut DBusConnection, *const c_char, *mut DBusError),
    bus_get_unique_name: unsafe extern "C" fn(*mut DBusConnection) -> *const c_char,
    connection_open_private:
        unsafe extern "C" fn(*const c_char, *mut DBusError) -> *mut DBusConnection,
    connection_set_exit_on_disconnect: unsafe extern "C" fn(*mut DBusConnection, dbus_bool_t),
    connection_get_is_connected: unsafe extern "C" fn(*mut DBusConnection) -> dbus_bool_t,
    connection_add_filter: unsafe extern "C" fn(
        *mut DBusConnection,
        DBusHandleMessageFunction,
        *mut c_void,
        Option<DBusFreeFunction>,
    ) -> dbus_bool_t,
    connection_remove_filter:
        unsafe extern "C" fn(*mut DBusConnection, DBusHandleMessageFunction, *mut c_void),
    connection_send:
        unsafe extern "C" fn(*mut DBusConnection, *mut DBusMessage, *mut u32) -> dbus_bool_t,
    connection_send_with_reply_and_block: unsafe extern "C" fn(
        *mut DBusConnection,
        *mut DBusMessage,
        c_int,
        *mut DBusError,
    ) -> *mut DBusMessage,
    connection_close: unsafe extern "C" fn(*mut DBusConnection),
    connection_unref: unsafe extern "C" fn(*mut DBusConnection),
    connection_flush: unsafe extern "C" fn(*mut DBusConnection),
    connection_read_write: unsafe extern "C" fn(*mut DBusConnection, c_int) -> dbus_bool_t,
    connection_read_write_dispatch: unsafe extern "C" fn(*mut DBusConnection, c_int) -> dbus_bool_t,
    connection_dispatch: unsafe extern "C" fn(*mut DBusConnection) -> c_int,
    type_is_fixed: unsafe extern "C" fn(c_int) -> dbus_bool_t,
    message_is_signal:
        unsafe extern "C" fn(*mut DBusMessage, *const c_char, *const c_char) -> dbus_bool_t,
    message_is_method_call:
        unsafe extern "C" fn(*mut DBusMessage, *const c_char, *const c_char) -> dbus_bool_t,
    message_has_path: unsafe extern "C" fn(*mut DBusMessage, *const c_char) -> dbus_bool_t,
    message_new_method_call: unsafe extern "C" fn(
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
    ) -> *mut DBusMessage,
    message_new_signal:
        unsafe extern "C" fn(*const c_char, *const c_char, *const c_char) -> *mut DBusMessage,
    message_new_method_return: unsafe extern "C" fn(*mut DBusMessage) -> *mut DBusMessage,
    message_new_error:
        unsafe extern "C" fn(*mut DBusMessage, *const c_char, *const c_char) -> *mut DBusMessage,
    message_set_no_reply: unsafe extern "C" fn(*mut DBusMessage, dbus_bool_t),
    message_iter_init_append: unsafe extern "C" fn(*mut DBusMessage, *mut DBusMessageIter),
    message_iter_open_container: unsafe extern "C" fn(
        *mut DBusMessageIter,
        c_int,
        *const c_char,
        *mut DBusMessageIter,
    ) -> dbus_bool_t,
    message_iter_append_basic:
        unsafe extern "C" fn(*mut DBusMessageIter, c_int, *const c_void) -> dbus_bool_t,
    message_iter_append_fixed_array:
        unsafe extern "C" fn(*mut DBusMessageIter, c_int, *const c_void, c_int) -> dbus_bool_t,
    message_iter_close_container:
        unsafe extern "C" fn(*mut DBusMessageIter, *mut DBusMessageIter) -> dbus_bool_t,
    message_iter_init: unsafe extern "C" fn(*mut DBusMessage, *mut DBusMessageIter) -> dbus_bool_t,
    message_iter_next: unsafe extern "C" fn(*mut DBusMessageIter) -> dbus_bool_t,
    message_iter_get_basic: unsafe extern "C" fn(*mut DBusMessageIter, *mut c_void),
    message_iter_get_arg_type: unsafe extern "C" fn(*mut DBusMessageIter) -> c_int,
    message_iter_recurse: unsafe extern "C" fn(*mut DBusMessageIter, *mut DBusMessageIter),
    message_unref: unsafe extern "C" fn(*mut DBusMessage),
    bus_request_name:
        unsafe extern "C" fn(*mut DBusConnection, *const c_char, c_uint, *mut DBusError) -> c_int,
    threads_init_default: unsafe extern "C" fn() -> dbus_bool_t,
    error_init: unsafe extern "C" fn(*mut DBusError),
    error_is_set: unsafe extern "C" fn(*const DBusError) -> dbus_bool_t,
    error_has_name: unsafe extern "C" fn(*const DBusError, *const c_char) -> dbus_bool_t,
    error_free: unsafe extern "C" fn(*mut DBusError),
    get_local_machine_id: unsafe extern "C" fn() -> *mut c_char,
    try_get_local_machine_id: Option<unsafe extern "C" fn(*mut DBusError) -> *mut c_char>,
    free: unsafe extern "C" fn(*mut c_void),
    shutdown: unsafe extern "C" fn(),
}

// we never link directly to libdbus.
const DBUS_LIBRARY: &str = "libdbus-1.so.3";

/// Translation of `LoadDBUSSyms()`.
fn load_dbus_syms(so: &SharedObject) -> Option<Fns> {
    macro_rules! sym {
        ($name:literal) => {
            // SAFETY: the field's type is the libdbus function's signature.
            unsafe { so.function(concat!("dbus_", $name)) }.ok()?
        };
    }
    Some(Fns {
        bus_get_private: sym!("bus_get_private"),
        bus_register: sym!("bus_register"),
        bus_add_match: sym!("bus_add_match"),
        bus_remove_match: sym!("bus_remove_match"),
        bus_get_unique_name: sym!("bus_get_unique_name"),
        connection_open_private: sym!("connection_open_private"),
        connection_set_exit_on_disconnect: sym!("connection_set_exit_on_disconnect"),
        connection_get_is_connected: sym!("connection_get_is_connected"),
        connection_add_filter: sym!("connection_add_filter"),
        connection_remove_filter: sym!("connection_remove_filter"),
        connection_send: sym!("connection_send"),
        connection_send_with_reply_and_block: sym!("connection_send_with_reply_and_block"),
        connection_close: sym!("connection_close"),
        connection_unref: sym!("connection_unref"),
        connection_flush: sym!("connection_flush"),
        connection_read_write: sym!("connection_read_write"),
        connection_read_write_dispatch: sym!("connection_read_write_dispatch"),
        connection_dispatch: sym!("connection_dispatch"),
        type_is_fixed: sym!("type_is_fixed"),
        message_is_signal: sym!("message_is_signal"),
        message_is_method_call: sym!("message_is_method_call"),
        message_has_path: sym!("message_has_path"),
        message_new_method_call: sym!("message_new_method_call"),
        message_new_signal: sym!("message_new_signal"),
        message_new_method_return: sym!("message_new_method_return"),
        message_new_error: sym!("message_new_error"),
        message_set_no_reply: sym!("message_set_no_reply"),
        message_iter_init_append: sym!("message_iter_init_append"),
        message_iter_open_container: sym!("message_iter_open_container"),
        message_iter_append_basic: sym!("message_iter_append_basic"),
        message_iter_append_fixed_array: sym!("message_iter_append_fixed_array"),
        message_iter_close_container: sym!("message_iter_close_container"),
        message_iter_init: sym!("message_iter_init"),
        message_iter_next: sym!("message_iter_next"),
        message_iter_get_basic: sym!("message_iter_get_basic"),
        message_iter_get_arg_type: sym!("message_iter_get_arg_type"),
        message_iter_recurse: sym!("message_iter_recurse"),
        message_unref: sym!("message_unref"),
        bus_request_name: sym!("bus_request_name"),
        threads_init_default: sym!("threads_init_default"),
        error_init: sym!("error_init"),
        error_is_set: sym!("error_is_set"),
        error_has_name: sym!("error_has_name"),
        error_free: sym!("error_free"),
        get_local_machine_id: sym!("get_local_machine_id"),
        // SAFETY: the field's type is the libdbus function's signature.
        try_get_local_machine_id: unsafe { so.function("dbus_try_get_local_machine_id") }.ok(),
        free: sym!("free"),
        shutdown: sym!("shutdown"),
    })
}

/// The loaded library and its entry points.
pub(crate) struct Lib {
    fns: Fns,
    _so: SharedObject,
}

impl std::fmt::Debug for Lib {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Lib")
    }
}

/// Load libdbus once per process. Translation of `LoadDBUSLibrary()`.
fn lib() -> Option<&'static Lib> {
    static LIB: std::sync::OnceLock<Option<Lib>> = std::sync::OnceLock::new();
    LIB.get_or_init(|| {
        // Don't report an error: SDL_LoadObject already did.
        let so = SharedObject::load(DBUS_LIBRARY).ok()?;
        let fns = load_dbus_syms(&so)?;
        Some(Lib { fns, _so: so })
    })
    .as_ref()
}

/// A `DBusError` that frees itself.
struct ErrorGuard<'a> {
    lib: &'a Lib,
    err: DBusError,
}

impl<'a> ErrorGuard<'a> {
    fn new(lib: &'a Lib) -> ErrorGuard<'a> {
        let mut err = DBusError::new();
        // SAFETY: err is a valid DBusError to initialize.
        unsafe { (lib.fns.error_init)(&mut err) };
        ErrorGuard { lib, err }
    }

    fn is_set(&self) -> bool {
        // SAFETY: err was initialized by dbus_error_init.
        unsafe { (self.lib.fns.error_is_set)(&self.err) != 0 }
    }

    fn has_name(&self, name: &CStr) -> bool {
        // SAFETY: as above; name is NUL-terminated.
        unsafe { (self.lib.fns.error_has_name)(&self.err, name.as_ptr()) != 0 }
    }

    /// "name: message", as upstream's `SDL_SetError("%s: %s", err.name, err.message)`.
    fn to_error(&self) -> Error {
        let s = |p: *const c_char| {
            if p.is_null() {
                String::new()
            } else {
                // SAFETY: a set DBusError holds NUL-terminated strings.
                unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
            }
        };
        Error::new(format!("{}: {}", s(self.err.name), s(self.err.message)))
    }

    fn free(&mut self) {
        // SAFETY: err was initialized; freeing an unset error is a no-op.
        unsafe { (self.lib.fns.error_free)(&mut self.err) };
    }
}

impl Drop for ErrorGuard<'_> {
    fn drop(&mut self) {
        self.free();
    }
}

fn cstring(s: &str) -> CString {
    // (D-Bus names, paths and strings can't carry NUL either)
    CString::new(s.replace('\0', "")).expect("NULs removed")
}

/// A connection to a bus. Closed and released on drop.
pub(crate) struct Connection {
    lib: &'static Lib,
    raw: *mut DBusConnection,
}

// SAFETY: libdbus connections are thread-safe once dbus_threads_init_default()
// has run, which init() does before opening any.
unsafe impl Send for Connection {}
// SAFETY: as above.
unsafe impl Sync for Connection {}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("raw", &self.raw)
            .finish()
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        // SAFETY: raw is a private connection we own; closed and released once.
        unsafe {
            (self.lib.fns.connection_close)(self.raw);
            (self.lib.fns.connection_unref)(self.raw);
        }
    }
}

impl Connection {
    /// Open a private connection to the session or system bus.
    fn bus(lib: &'static Lib, bus_type: c_int) -> Result<Connection> {
        let mut err = ErrorGuard::new(lib);
        // SAFETY: err is initialized; the call returns NULL or a new
        // private connection.
        let raw = unsafe { (lib.fns.bus_get_private)(bus_type, &mut err.err) };
        if err.is_set() || raw.is_null() {
            return Err(err.to_error());
        }
        // SAFETY: raw is a valid connection.
        unsafe { (lib.fns.connection_set_exit_on_disconnect)(raw, 0) };
        Ok(Connection { lib, raw })
    }

    /// Open a private connection to a bus by address and register with it
    /// (`dbus_connection_open_private()` + `dbus_bus_register()`).
    pub(crate) fn open_address(address: &str) -> Result<Connection> {
        let lib = lib().ok_or_else(|| Error::new("D-Bus is not available"))?;
        // SAFETY: called once before any connection is made, as upstream does.
        if unsafe { (lib.fns.threads_init_default)() } == 0 {
            return Err(Error::out_of_memory());
        }
        let addr = cstring(address);
        let mut err = ErrorGuard::new(lib);
        // SAFETY: addr is NUL-terminated; err is initialized.
        let raw = unsafe { (lib.fns.connection_open_private)(addr.as_ptr(), &mut err.err) };
        if raw.is_null() {
            return Err(err.to_error());
        }
        // SAFETY: raw is a valid connection.
        unsafe { (lib.fns.connection_set_exit_on_disconnect)(raw, 0) };
        let conn = Connection { lib, raw };
        // SAFETY: as above.
        if unsafe { (lib.fns.bus_register)(raw, &mut err.err) } == 0 {
            return Err(err.to_error());
        }
        Ok(conn)
    }

    /// The connection's unique bus name (":1.42").
    pub(crate) fn unique_name(&self) -> Option<String> {
        // SAFETY: raw is valid; the name is owned by the connection.
        let p = unsafe { (self.lib.fns.bus_get_unique_name)(self.raw) };
        // SAFETY: a non-NULL name is NUL-terminated.
        (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
    }

    /// Whether the connection is still open.
    pub(crate) fn is_connected(&self) -> bool {
        // SAFETY: raw is valid.
        unsafe { (self.lib.fns.connection_get_is_connected)(self.raw) != 0 }
    }

    /// Ask the bus for a well-known name; returns the
    /// `DBUS_REQUEST_NAME_REPLY_*` code.
    pub(crate) fn request_name(&self, name: &str, flags: u32) -> Result<i32> {
        let n = cstring(name);
        let mut err = ErrorGuard::new(self.lib);
        // SAFETY: n is NUL-terminated; err is initialized.
        let r =
            unsafe { (self.lib.fns.bus_request_name)(self.raw, n.as_ptr(), flags, &mut err.err) };
        if err.is_set() {
            return Err(err.to_error());
        }
        Ok(r)
    }

    /// Add a match rule (`dbus_bus_add_match()`).
    pub(crate) fn add_match(&self, rule: &str) -> Result<()> {
        let r = cstring(rule);
        let mut err = ErrorGuard::new(self.lib);
        // SAFETY: r is NUL-terminated; err is initialized.
        unsafe { (self.lib.fns.bus_add_match)(self.raw, r.as_ptr(), &mut err.err) };
        if err.is_set() {
            Err(err.to_error())
        } else {
            Ok(())
        }
    }

    /// Remove a match rule (`dbus_bus_remove_match()`).
    pub(crate) fn remove_match(&self, rule: &str) -> Result<()> {
        let r = cstring(rule);
        let mut err = ErrorGuard::new(self.lib);
        // SAFETY: as above.
        unsafe { (self.lib.fns.bus_remove_match)(self.raw, r.as_ptr(), &mut err.err) };
        if err.is_set() {
            Err(err.to_error())
        } else {
            Ok(())
        }
    }

    /// A new method call message.
    pub(crate) fn new_method_call(
        &self,
        node: &str,
        path: &str,
        interface: &str,
        method: &str,
    ) -> Option<Message> {
        let (n, p, i, m) = (
            cstring(node),
            cstring(path),
            cstring(interface),
            cstring(method),
        );
        // SAFETY: all strings are NUL-terminated.
        let raw = unsafe {
            (self.lib.fns.message_new_method_call)(n.as_ptr(), p.as_ptr(), i.as_ptr(), m.as_ptr())
        };
        Message::from_raw(self.lib, raw)
    }

    /// A new signal message.
    pub(crate) fn new_signal(&self, path: &str, interface: &str, name: &str) -> Option<Message> {
        let (p, i, n) = (cstring(path), cstring(interface), cstring(name));
        // SAFETY: all strings are NUL-terminated.
        let raw = unsafe { (self.lib.fns.message_new_signal)(p.as_ptr(), i.as_ptr(), n.as_ptr()) };
        Message::from_raw(self.lib, raw)
    }

    /// Send a message and wait up to `timeout_ms` (-1: libdbus's default)
    /// for its reply.
    pub(crate) fn send_with_reply_and_block(
        &self,
        msg: &Message,
        timeout_ms: i32,
    ) -> Result<Message> {
        let mut err = ErrorGuard::new(self.lib);
        // SAFETY: raw and msg are valid; err is initialized.
        let reply = unsafe {
            (self.lib.fns.connection_send_with_reply_and_block)(
                self.raw,
                msg.raw,
                timeout_ms,
                &mut err.err,
            )
        };
        match Message::from_raw(self.lib, reply) {
            Some(r) => Ok(r),
            None if err.is_set() => Err(err.to_error()),
            None => Err(Error::new("D-Bus call failed")),
        }
    }

    /// Queue a message without waiting for a reply, then flush.
    pub(crate) fn send(&self, msg: &Message) -> bool {
        // SAFETY: raw and msg are valid; the serial isn't wanted.
        if unsafe { (self.lib.fns.connection_send)(self.raw, msg.raw, std::ptr::null_mut()) } == 0 {
            return false;
        }
        // SAFETY: raw is valid.
        unsafe { (self.lib.fns.connection_flush)(self.raw) };
        true
    }

    /// Read and write pending data without blocking, then dispatch.
    /// Translation of the body of `SDL_DBus_PumpEvents()`.
    pub(crate) fn pump(&self) {
        // SAFETY: raw is valid.
        unsafe {
            (self.lib.fns.connection_read_write)(self.raw, 0);
            while (self.lib.fns.connection_dispatch)(self.raw) == DBUS_DISPATCH_DATA_REMAINS {
                // Do nothing, actual work happens in DBus_MessageFilter
                crate::timer::delay(std::time::Duration::from_micros(10));
            }
        }
    }

    /// Block until data arrives (or `timeout_ms` passes, -1: forever) and
    /// dispatch it; false once the connection is closed.
    pub(crate) fn read_write_dispatch(&self, timeout_ms: i32) -> bool {
        // SAFETY: raw is valid.
        unsafe { (self.lib.fns.connection_read_write_dispatch)(self.raw, timeout_ms) != 0 }
    }

    /// Install a message filter, removed when the returned guard drops.
    pub(crate) fn add_filter<F>(&self, filter: F) -> Option<Filter<'_>>
    where
        F: FnMut(&Message) -> HandlerResult + Send + 'static,
    {
        type Boxed = Box<dyn FnMut(&Message) -> HandlerResult + Send>;
        unsafe extern "C" fn trampoline(
            _conn: *mut DBusConnection,
            msg: *mut DBusMessage,
            data: *mut c_void,
        ) -> HandlerResult {
            // SAFETY: data is the Box<Boxed> installed below, alive until the
            // filter is removed; msg is borrowed for the call.
            unsafe {
                let filter = &mut *(data as *mut Boxed);
                let Some(lib) = lib() else {
                    return HandlerResult::NotYetHandled;
                };
                let msg = std::mem::ManuallyDrop::new(Message { lib, raw: msg });
                filter(&msg)
            }
        }
        let data: *mut Boxed = Box::into_raw(Box::new(Box::new(filter)));
        // SAFETY: the trampoline and data stay valid until remove_filter.
        let ok = unsafe {
            (self.lib.fns.connection_add_filter)(self.raw, trampoline, data.cast(), None)
        } != 0;
        if !ok {
            // SAFETY: not installed; reclaim the box.
            drop(unsafe { Box::from_raw(data) });
            return None;
        }
        Some(Filter {
            conn: self,
            func: trampoline,
            data: data.cast(),
            free: |p| {
                // SAFETY: p is the Box<Boxed> created above.
                drop(unsafe { Box::from_raw(p as *mut Boxed) })
            },
        })
    }
}

/// Whether a failed call's error is the D-Bus error `name`; errors read
/// "name: message".
pub(crate) fn error_has_name(e: &Error, name: &CStr) -> bool {
    let name = name.to_bytes();
    e.message()
        .as_bytes()
        .strip_prefix(name)
        .is_some_and(|rest| rest.starts_with(b":"))
}

/// A message filter, removed on drop.
pub(crate) struct Filter<'a> {
    conn: &'a Connection,
    func: DBusHandleMessageFunction,
    data: *mut c_void,
    free: fn(*mut c_void),
}

impl std::fmt::Debug for Filter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Filter")
    }
}

impl Drop for Filter<'_> {
    fn drop(&mut self) {
        // SAFETY: the pair (func, data) was added to this connection.
        unsafe {
            (self.conn.lib.fns.connection_remove_filter)(self.conn.raw, self.func, self.data)
        };
        (self.free)(self.data);
    }
}

/// A message. Released on drop.
pub(crate) struct Message {
    lib: &'static Lib,
    raw: *mut DBusMessage,
}

// SAFETY: a message may move between threads; libdbus locks what it shares.
unsafe impl Send for Message {}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Message").field("raw", &self.raw).finish()
    }
}

impl Drop for Message {
    fn drop(&mut self) {
        // SAFETY: we own one reference to raw.
        unsafe { (self.lib.fns.message_unref)(self.raw) };
    }
}

/// An argument to append (`DBUS_TYPE_x, &value`).
#[derive(Clone, Copy, Debug)]
pub(crate) enum Arg<'a> {
    Byte(u8),
    Bool(bool),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    F64(f64),
    Str(&'a str),
    ObjectPath(&'a str),
    Signature(&'a str),
    UnixFd(RawFd),
    /// An `a{sv}` dictionary of string values.
    StrDict(&'a [(&'a str, &'a str)]),
}

/// A decoded value from a message. (Two `UnixFd`s are equal when they are
/// the same descriptor.)
#[derive(Debug)]
pub(crate) enum Value {
    Byte(u8),
    Bool(bool),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    F64(f64),
    Str(String),
    ObjectPath(String),
    Signature(String),
    UnixFd(OwnedFd),
    Array(Vec<Value>),
    Struct(Vec<Value>),
    DictEntry(Box<Value>, Box<Value>),
    Variant(Box<Value>),
}

impl PartialEq for Value {
    fn eq(&self, other: &Value) -> bool {
        use std::os::fd::AsRawFd;
        use Value::*;
        match (self, other) {
            (Byte(a), Byte(b)) => a == b,
            (Bool(a), Bool(b)) => a == b,
            (I16(a), I16(b)) => a == b,
            (U16(a), U16(b)) => a == b,
            (I32(a), I32(b)) => a == b,
            (U32(a), U32(b)) => a == b,
            (I64(a), I64(b)) => a == b,
            (U64(a), U64(b)) => a == b,
            (F64(a), F64(b)) => a == b,
            (Str(a), Str(b)) | (ObjectPath(a), ObjectPath(b)) | (Signature(a), Signature(b)) => {
                a == b
            }
            (UnixFd(a), UnixFd(b)) => a.as_raw_fd() == b.as_raw_fd(),
            (Array(a), Array(b)) | (Struct(a), Struct(b)) => a == b,
            (DictEntry(ak, av), DictEntry(bk, bv)) => ak == bk && av == bv,
            (Variant(a), Variant(b)) => a == b,
            _ => false,
        }
    }
}

impl Value {
    /// The string of a `Str`, `ObjectPath` or `Signature`.
    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) | Value::ObjectPath(s) | Value::Signature(s) => Some(s),
            _ => None,
        }
    }

    /// The value inside a variant (or the value itself).
    pub(crate) fn unvariant(&self) -> &Value {
        match self {
            Value::Variant(v) => v.unvariant(),
            v => v,
        }
    }
}

/// An append iterator (`DBusMessageIter` from `dbus_message_iter_init_append()`).
pub(crate) struct Writer<'m> {
    lib: &'static Lib,
    iter: DBusMessageIter,
    _msg: std::marker::PhantomData<&'m mut Message>,
}

impl Writer<'_> {
    fn basic<T>(&mut self, ty: c_int, v: &T) -> bool {
        // SAFETY: v points to a value of the C type matching ty.
        unsafe {
            (self.lib.fns.message_iter_append_basic)(&mut self.iter, ty, (v as *const T).cast())
                != 0
        }
    }

    fn string(&mut self, ty: c_int, s: &str) -> bool {
        let c = cstring(s);
        let p = c.as_ptr();
        self.basic(ty, &p)
    }

    /// Append one argument.
    pub(crate) fn append(&mut self, arg: &Arg<'_>) -> bool {
        match *arg {
            Arg::Byte(v) => self.basic(TYPE_BYTE, &v),
            Arg::Bool(v) => self.basic(TYPE_BOOLEAN, &(v as dbus_bool_t)),
            Arg::I16(v) => self.basic(TYPE_INT16, &v),
            Arg::U16(v) => self.basic(TYPE_UINT16, &v),
            Arg::I32(v) => self.basic(TYPE_INT32, &v),
            Arg::U32(v) => self.basic(TYPE_UINT32, &v),
            Arg::I64(v) => self.basic(TYPE_INT64, &v),
            Arg::U64(v) => self.basic(TYPE_UINT64, &v),
            Arg::F64(v) => self.basic(TYPE_DOUBLE, &v),
            Arg::Str(s) => self.string(TYPE_STRING, s),
            Arg::ObjectPath(s) => self.string(TYPE_OBJECT_PATH, s),
            Arg::Signature(s) => self.string(TYPE_SIGNATURE, s),
            Arg::UnixFd(fd) => self.basic(TYPE_UNIX_FD, &(fd as c_int)),
            Arg::StrDict(entries) => self.append_dict_with_keys_and_values(entries),
        }
    }

    /// Open a container, fill it, close it.
    pub(crate) fn container(
        &mut self,
        ty: u8,
        signature: Option<&str>,
        fill: impl FnOnce(&mut Writer<'_>) -> bool,
    ) -> bool {
        let sig = signature.map(cstring);
        let mut sub = Writer {
            lib: self.lib,
            iter: DBusMessageIter::new(),
            _msg: std::marker::PhantomData,
        };
        // SAFETY: self.iter is an append iterator; sub.iter receives the
        // sub-iterator; sig is NUL-terminated or NULL.
        let opened = unsafe {
            (self.lib.fns.message_iter_open_container)(
                &mut self.iter,
                c_int::from(ty),
                sig.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                &mut sub.iter,
            ) != 0
        };
        if !opened || !fill(&mut sub) {
            /* message_iter_abandon_container_if_open() and message_iter_abandon_container() might be
             * missing if libdbus is too old. Instead, we just return without cleaning up any eventual
             * open container */
            return false;
        }
        // SAFETY: sub.iter was opened from self.iter.
        unsafe { (self.lib.fns.message_iter_close_container)(&mut self.iter, &mut sub.iter) != 0 }
    }

    /// Append an `a{sv}` dictionary with string values. Translation of
    /// `SDL_DBus_AppendDictWithKeysAndValues()`.
    pub(crate) fn append_dict_with_keys_and_values(&mut self, entries: &[(&str, &str)]) -> bool {
        self.container(b'a', Some("{sv}"), |dict| {
            entries.iter().all(|(key, value)| {
                dict.container(b'e', None, |entry| {
                    entry.string(TYPE_STRING, key)
                        && entry.container(b'v', Some("s"), |variant| {
                            variant.string(TYPE_STRING, value)
                        })
                })
            })
        })
    }

    /// Append an `i32` array.
    pub(crate) fn append_i32_array(&mut self, values: &[i32]) -> bool {
        self.container(b'a', Some("i"), |a| {
            let p = values.as_ptr();
            // SAFETY: p points to values.len() i32s; libdbus takes a pointer
            // to the array pointer.
            unsafe {
                (a.lib.fns.message_iter_append_fixed_array)(
                    &mut a.iter,
                    TYPE_INT32,
                    (&p as *const *const i32).cast(),
                    values.len() as c_int,
                ) != 0
            }
        })
    }
}

impl Message {
    fn from_raw(lib: &'static Lib, raw: *mut DBusMessage) -> Option<Message> {
        (!raw.is_null()).then(|| Message { lib, raw })
    }

    /// A method return for this method call.
    pub(crate) fn new_method_return(&self) -> Option<Message> {
        // SAFETY: raw is valid.
        let r = unsafe { (self.lib.fns.message_new_method_return)(self.raw) };
        Message::from_raw(self.lib, r)
    }

    /// An error reply for this method call.
    pub(crate) fn new_error(&self, name: &str, message: &str) -> Option<Message> {
        let (n, m) = (cstring(name), cstring(message));
        // SAFETY: raw is valid; both strings are NUL-terminated.
        let r = unsafe { (self.lib.fns.message_new_error)(self.raw, n.as_ptr(), m.as_ptr()) };
        Message::from_raw(self.lib, r)
    }

    /// Mark a method call as not wanting a reply.
    pub(crate) fn set_no_reply(&mut self, no_reply: bool) {
        // SAFETY: raw is valid.
        unsafe { (self.lib.fns.message_set_no_reply)(self.raw, no_reply as dbus_bool_t) };
    }

    /// An iterator appending to the end of the message.
    pub(crate) fn writer(&mut self) -> Writer<'_> {
        let mut iter = DBusMessageIter::new();
        // SAFETY: raw is valid; iter receives the append iterator.
        unsafe { (self.lib.fns.message_iter_init_append)(self.raw, &mut iter) };
        Writer {
            lib: self.lib,
            iter,
            _msg: std::marker::PhantomData,
        }
    }

    /// Append arguments (`dbus_message_append_args()`).
    pub(crate) fn append_args(&mut self, args: &[Arg<'_>]) -> bool {
        let mut w = self.writer();
        args.iter().all(|a| w.append(a))
    }

    /// Whether this is the signal `interface.name`.
    pub(crate) fn is_signal(&self, interface: &str, name: &str) -> bool {
        let (i, n) = (cstring(interface), cstring(name));
        // SAFETY: raw is valid; strings are NUL-terminated.
        unsafe { (self.lib.fns.message_is_signal)(self.raw, i.as_ptr(), n.as_ptr()) != 0 }
    }

    /// Whether this is a call of the method `interface.method`.
    pub(crate) fn is_method_call(&self, interface: &str, method: &str) -> bool {
        let (i, m) = (cstring(interface), cstring(method));
        // SAFETY: raw is valid; strings are NUL-terminated.
        unsafe { (self.lib.fns.message_is_method_call)(self.raw, i.as_ptr(), m.as_ptr()) != 0 }
    }

    /// Whether the message's object path is `path`.
    pub(crate) fn has_path(&self, path: &str) -> bool {
        let p = cstring(path);
        // SAFETY: raw is valid; p is NUL-terminated.
        unsafe { (self.lib.fns.message_has_path)(self.raw, p.as_ptr()) != 0 }
    }

    /// Decode every argument of the message.
    pub(crate) fn args(&self) -> Vec<Value> {
        let mut iter = DBusMessageIter::new();
        // SAFETY: raw is valid; iter receives a read iterator.
        if unsafe { (self.lib.fns.message_iter_init)(self.raw, &mut iter) } == 0 {
            return Vec::new();
        }
        read_all(self.lib, &mut iter)
    }
}

fn read_all(lib: &Lib, iter: &mut DBusMessageIter) -> Vec<Value> {
    let mut out = Vec::new();
    loop {
        match read_one(lib, iter) {
            Some(v) => out.push(v),
            None => break,
        }
        // SAFETY: iter is a valid read iterator.
        if unsafe { (lib.fns.message_iter_next)(iter) } == 0 {
            break;
        }
    }
    out
}

fn read_one(lib: &Lib, iter: &mut DBusMessageIter) -> Option<Value> {
    // SAFETY: iter is a valid read iterator.
    let ty = unsafe { (lib.fns.message_iter_get_arg_type)(iter) };
    macro_rules! basic {
        ($t:ty, $init:expr) => {{
            let mut v: $t = $init;
            // SAFETY: the current argument has the C type matching $t.
            unsafe { (lib.fns.message_iter_get_basic)(iter, (&mut v as *mut $t).cast()) };
            v
        }};
    }
    let string = |iter: &mut DBusMessageIter| {
        let mut p: *const c_char = std::ptr::null();
        // SAFETY: the current argument is a string type; p receives a
        // pointer owned by the message.
        unsafe { (lib.fns.message_iter_get_basic)(iter, (&mut p as *mut *const c_char).cast()) };
        if p.is_null() {
            String::new()
        } else {
            // SAFETY: libdbus strings are NUL-terminated (and valid UTF-8).
            unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
        }
    };
    let recurse = |iter: &mut DBusMessageIter| {
        let mut sub = DBusMessageIter::new();
        // SAFETY: the current argument is a container.
        unsafe { (lib.fns.message_iter_recurse)(iter, &mut sub) };
        // An empty array's sub-iterator starts at TYPE_INVALID.
        read_all(lib, &mut sub)
    };
    Some(match ty {
        TYPE_INVALID => return None,
        TYPE_BYTE => Value::Byte(basic!(u8, 0)),
        TYPE_BOOLEAN => Value::Bool(basic!(dbus_bool_t, 0) != 0),
        TYPE_INT16 => Value::I16(basic!(i16, 0)),
        TYPE_UINT16 => Value::U16(basic!(u16, 0)),
        TYPE_INT32 => Value::I32(basic!(i32, 0)),
        TYPE_UINT32 => Value::U32(basic!(u32, 0)),
        TYPE_INT64 => Value::I64(basic!(i64, 0)),
        TYPE_UINT64 => Value::U64(basic!(u64, 0)),
        TYPE_DOUBLE => Value::F64(basic!(f64, 0.0)),
        TYPE_STRING => Value::Str(string(iter)),
        TYPE_OBJECT_PATH => Value::ObjectPath(string(iter)),
        TYPE_SIGNATURE => Value::Signature(string(iter)),
        TYPE_UNIX_FD => {
            // libdbus hands out a duplicate the caller owns.
            let fd = basic!(c_int, -1);
            if fd < 0 {
                return None;
            }
            // SAFETY: fd is a fresh descriptor owned by us.
            Value::UnixFd(unsafe { OwnedFd::from_raw_fd(fd) })
        }
        TYPE_ARRAY => Value::Array(recurse(iter)),
        TYPE_STRUCT => Value::Struct(recurse(iter)),
        TYPE_VARIANT => Value::Variant(Box::new(recurse(iter).into_iter().next()?)),
        TYPE_DICT_ENTRY => {
            let mut kv = recurse(iter).into_iter();
            let k = kv.next()?;
            let v = kv.next()?;
            Value::DictEntry(Box::new(k), Box::new(v))
        }
        _ => return None,
    })
}

// * * * The context

/// The session and system bus connections (`SDL_DBusContext`).
#[derive(Debug)]
pub(crate) struct Context {
    pub(crate) session_conn: Connection,
    pub(crate) system_conn: Option<Connection>,
}

struct State {
    initialized: bool,
    is_dbus_available: bool,
    context: Option<Arc<Context>>,
    inhibit_handle: Option<String>,
    screensaver_cookie: u32,
    interface_unavailable: bool,
}

static STATE: Mutex<State> = Mutex::new(State {
    initialized: false,
    is_dbus_available: true,
    context: None,
    inhibit_handle: None,
    screensaver_cookie: 0,
    interface_unavailable: false,
});

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Connect to the session bus (required) and the system bus (optional).
/// After a failure, D-Bus isn't tried again. Translation of `SDL_DBus_Init()`.
pub(crate) fn init() {
    let mut s = state();
    init_locked(&mut s);
}

fn init_locked(s: &mut State) {
    if !s.is_dbus_available {
        return; // don't keep trying if this fails.
    }
    if s.initialized {
        return;
    }
    let connect = || -> Option<Context> {
        let lib = lib()?;
        // SAFETY: called before any connection is made.
        if unsafe { (lib.fns.threads_init_default)() } == 0 {
            return None;
        }
        // session bus is required
        let session_conn = Connection::bus(lib, DBUS_BUS_SESSION).ok()?;
        // system bus is optional
        let system_conn = Connection::bus(lib, DBUS_BUS_SYSTEM).ok();
        Some(Context {
            session_conn,
            system_conn,
        })
    };
    match connect() {
        Some(ctx) => {
            s.context = Some(Arc::new(ctx));
            s.initialized = true;
        }
        None => {
            s.is_dbus_available = false;
            quit_locked(s);
        }
    }
}

/// Close the connections. With `SDL_HINT_SHUTDOWN_DBUS_ON_QUIT`, libdbus
/// is also shut down. Translation of `SDL_DBus_Quit()`.
pub(crate) fn quit() {
    let mut s = state();
    quit_locked(&mut s);
}

fn quit_locked(s: &mut State) {
    let ctx = s.context.take();
    drop(ctx); // closes and releases the connections once nobody uses them
    if crate::hints::get_bool(crate::hints::SHUTDOWN_DBUS_ON_QUIT, false) {
        if let Some(lib) = lib() {
            // SAFETY: our connections are closed; dbus_shutdown frees
            // libdbus's global state.
            unsafe { (lib.fns.shutdown)() };
        }
    }
    /* Leaving libdbus loaded when skipping dbus_shutdown() avoids
     * spurious leak warnings from LeakSanitizer on internal D-Bus
     * allocations that would be freed by dbus_shutdown(). */
    s.inhibit_handle = None;
    s.screensaver_cookie = 0;
    s.initialized = false;
}

/// The connections, initializing them on first use, or `None` without
/// D-Bus. Translation of `SDL_DBus_GetContext()`.
pub(crate) fn context() -> Option<Arc<Context>> {
    let mut s = state();
    if s.context.is_none() {
        init_locked(&mut s);
    }
    s.context.clone()
}

/// Call a method and return its reply, or `None` if the call couldn't be
/// made or failed. The reply's values are in [`Message::args`].
/// Translation of `SDL_DBus_CallMethodOnConnection()`.
pub(crate) fn call_method_on_connection(
    conn: &Connection,
    node: &str,
    path: &str,
    interface: &str,
    method: &str,
    args: &[Arg<'_>],
) -> Option<Message> {
    let mut msg = conn.new_method_call(node, path, interface, method)?;
    if !msg.append_args(args) {
        return None;
    }
    conn.send_with_reply_and_block(&msg, 300).ok()
}

/// [`call_method_on_connection`] on the session bus. Translation of
/// `SDL_DBus_CallMethod()`.
pub(crate) fn call_method(
    node: &str,
    path: &str,
    interface: &str,
    method: &str,
    args: &[Arg<'_>],
) -> Option<Message> {
    let ctx = current_context()?;
    call_method_on_connection(&ctx.session_conn, node, path, interface, method, args)
}

/// Send a method call without waiting for a reply. Translation of
/// `SDL_DBus_CallVoidMethodOnConnection()`.
pub(crate) fn call_void_method_on_connection(
    conn: &Connection,
    node: &str,
    path: &str,
    interface: &str,
    method: &str,
    args: &[Arg<'_>],
) -> bool {
    let Some(mut msg) = conn.new_method_call(node, path, interface, method) else {
        return false;
    };
    if !msg.append_args(args) {
        return false;
    }
    msg.set_no_reply(true);
    conn.send(&msg)
}

/// [`call_void_method_on_connection`] on the session bus. Translation of
/// `SDL_DBus_CallVoidMethod()`.
pub(crate) fn call_void_method(
    node: &str,
    path: &str,
    interface: &str,
    method: &str,
    args: &[Arg<'_>],
) -> bool {
    match current_context() {
        Some(ctx) => {
            call_void_method_on_connection(&ctx.session_conn, node, path, interface, method, args)
        }
        None => false,
    }
}

/// Send `msg` and return the first value of the reply (inside a variant if
/// it is one). Translation of `SDL_DBus_CallWithBasicReply()`, which also
/// checks the type: callers match on the returned [`Value`].
fn call_with_basic_reply(conn: &Connection, msg: &Message) -> Option<Value> {
    let reply = conn.send_with_reply_and_block(msg, 300).ok()?;
    let v = reply.args().into_iter().next()?;
    Some(match v {
        Value::Variant(inner) => *inner,
        v => v,
    })
}

/// Read a property through `org.freedesktop.DBus.Properties.Get`.
/// Translation of `SDL_DBus_QueryPropertyOnConnection()`.
pub(crate) fn query_property_on_connection(
    conn: &Connection,
    node: &str,
    path: &str,
    interface: &str,
    property: &str,
) -> Option<Value> {
    let mut msg = conn.new_method_call(node, path, "org.freedesktop.DBus.Properties", "Get")?;
    if !msg.append_args(&[Arg::Str(interface), Arg::Str(property)]) {
        return None;
    }
    call_with_basic_reply(conn, &msg)
}

/// [`query_property_on_connection`] on the session bus. Translation of
/// `SDL_DBus_QueryProperty()`.
pub(crate) fn query_property(
    node: &str,
    path: &str,
    interface: &str,
    property: &str,
) -> Option<Value> {
    let ctx = current_context()?;
    query_property_on_connection(&ctx.session_conn, node, path, interface, property)
}

/// The context if D-Bus is initialized (without initializing it).
fn current_context() -> Option<Arc<Context>> {
    state().context.clone()
}

/// Tell the screensaver the user is active, unless we're inhibiting it.
/// Translation of `SDL_DBus_ScreensaverTickle()`.
pub(crate) fn screensaver_tickle() {
    let inhibiting = {
        let s = state();
        s.screensaver_cookie != 0 || s.inhibit_handle.is_some()
    };
    if !inhibiting {
        // no need to tickle if we're inhibiting.
        // org.gnome.ScreenSaver is the legacy interface, but it'll either do nothing or just be a second harmless tickle on newer systems, so we leave it for now.
        call_void_method(
            "org.gnome.ScreenSaver",
            "/org/gnome/ScreenSaver",
            "org.gnome.ScreenSaver",
            "SimulateUserActivity",
            &[],
        );
        call_void_method(
            "org.freedesktop.ScreenSaver",
            "/org/freedesktop/ScreenSaver",
            "org.freedesktop.ScreenSaver",
            "SimulateUserActivity",
            &[],
        );
    }
}

/// Open a URI, or a local file or directory, through the desktop portal.
/// Translation of `SDL_DBus_OpenURI()`.
pub(crate) fn open_uri(uri: &str, window_id: Option<&str>, activation_token: Option<&str>) -> bool {
    const BUS_NAME: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    const INTERFACE: &str = "org.freedesktop.portal.OpenURI";

    let Some(ctx) = current_context() else {
        /* We either lost connection to the session bus or were not able to
         * load the D-Bus library at all.
         */
        return false;
    };
    let conn = &ctx.session_conn;

    let has_file_scheme = uri.len() >= 6 && uri.as_bytes()[..6].eq_ignore_ascii_case(b"file:/");
    let mut fd: Option<OwnedFd> = None;

    // The OpenURI method can't open 'file://' URIs or local paths, so OpenFile must be used instead.
    let msg = if has_file_scheme || !crate::utils::is_uri(uri) {
        // Decode the path if it is a URI.
        let path: Vec<u8> = if has_file_scheme {
            match crate::utils::uri_to_local(uri) {
                Some(p) => p,
                None => return false,
            }
        } else {
            uri.as_bytes().to_vec()
        };
        let Ok(cpath) = CString::new(path) else {
            return false;
        };
        // SAFETY: stat is plain data; cpath is NUL-terminated.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: as above.
        if unsafe { libc::stat(cpath.as_ptr(), &mut st) } == 0 {
            /* Open files and directories as read-only, as the fd is only used as proof
             * of access by the portal, and a mismatch between fd write permissions and
             * the portal write parameter will cause the fd to be rejected.
             */
            let mut oflags = libc::O_RDONLY | libc::O_CLOEXEC;
            if (st.st_mode & libc::S_IFMT) == libc::S_IFDIR {
                oflags |= libc::O_DIRECTORY;
            }
            // SAFETY: cpath is NUL-terminated.
            let raw = unsafe { libc::open(cpath.as_ptr(), oflags) };
            if raw >= 0 {
                // SAFETY: raw is a fresh descriptor we own.
                fd = Some(unsafe { OwnedFd::from_raw_fd(raw) });
            }
        }
        if fd.is_none() {
            return false;
        }
        conn.new_method_call(BUS_NAME, PATH, INTERFACE, "OpenFile")
    } else {
        conn.new_method_call(BUS_NAME, PATH, INTERFACE, "OpenURI")
    };
    let Some(mut msg) = msg else {
        return false;
    };

    let mut w = msg.writer();
    if !w.append(&Arg::Str(window_id.unwrap_or(""))) {
        return false;
    }
    let target_ok = match &fd {
        Some(fd) => w.append(&Arg::UnixFd(std::os::fd::AsRawFd::as_raw_fd(fd))),
        None => w.append(&Arg::Str(uri)),
    };
    if !target_ok {
        return false;
    }
    // The array must be in the parameter list, even if empty.
    let options: &[(&str, &str)] = match activation_token {
        Some(token) => &[("activation_token", token)],
        None => &[],
    };
    if !w.append_dict_with_keys_and_values(options) {
        return false;
    }

    // The file descriptor is duplicated by D-Bus, so it can be closed on this end.
    conn.send_with_reply_and_block(&msg, DBUS_TIMEOUT_USE_DEFAULT)
        .is_ok()
}

/// Inhibit (or release) the screensaver: through the Inhibit portal in a
/// sandbox, else `org.freedesktop.ScreenSaver`. Translation of
/// `SDL_DBus_ScreensaverInhibit()`.
pub(crate) fn screensaver_inhibit(inhibit: bool) -> bool {
    const DEFAULT_INHIBIT_REASON: &str = "Playing a game";

    let (cookie, handle) = {
        let s = state();
        // If the interface was previously queried and is unavailable, return false.
        if s.interface_unavailable {
            return false;
        }
        (s.screensaver_cookie, s.inhibit_handle.clone())
    };

    if (inhibit && (cookie != 0 || handle.is_some()))
        || (!inhibit && (cookie == 0 && handle.is_none()))
    {
        return true;
    }

    let Some(ctx) = current_context() else {
        /* We either lost connection to the session bus or were not able to
         * load the D-Bus library at all. */
        return false;
    };
    let conn = &ctx.session_conn;
    let reason = crate::hints::get(crate::hints::SCREENSAVER_INHIBIT_ACTIVITY_NAME)
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| DEFAULT_INHIBIT_REASON.to_owned());

    if crate::init::sandbox() != crate::init::Sandbox::None {
        const BUS_NAME: &str = "org.freedesktop.portal.Desktop";
        const PATH: &str = "/org/freedesktop/portal/desktop";
        const INTERFACE: &str = "org.freedesktop.portal.Inhibit";
        const WINDOW: &str = ""; // As a future improvement we could gather the X11 XID or Wayland surface identifier
        const INHIBIT_IDLE: u32 = 8; // Taken from the portal API reference

        if inhibit {
            let Some(mut msg) = conn.new_method_call(BUS_NAME, PATH, INTERFACE, "Inhibit") else {
                return false;
            };
            // a{sv}
            if !msg.append_args(&[
                Arg::Str(WINDOW),
                Arg::U32(INHIBIT_IDLE),
                Arg::StrDict(&[("reason", &reason)]),
            ]) {
                return false;
            }
            match call_with_basic_reply(conn, &msg) {
                Some(Value::ObjectPath(reply_path)) => {
                    state().inhibit_handle = Some(reply_path);
                    true
                }
                _ => {
                    state().interface_unavailable = true;
                    false
                }
            }
        } else {
            let handle = handle.unwrap_or_default();
            if !call_void_method_on_connection(
                conn,
                BUS_NAME,
                &handle,
                "org.freedesktop.portal.Request",
                "Close",
                &[],
            ) {
                return false;
            }
            state().inhibit_handle = None;
            true
        }
    } else {
        const BUS_NAME: &str = "org.freedesktop.ScreenSaver";
        const PATH: &str = "/org/freedesktop/ScreenSaver";
        const INTERFACE: &str = "org.freedesktop.ScreenSaver";

        if inhibit {
            let app = crate::init::app_metadata_property(crate::init::AppMetadata::Name)
                .unwrap_or_default();
            let reply = call_method_on_connection(
                conn,
                BUS_NAME,
                PATH,
                INTERFACE,
                "Inhibit",
                &[Arg::Str(&app), Arg::Str(&reason)],
            );
            let cookie = match reply.as_ref().map(Message::args).as_deref() {
                Some([Value::U32(c), ..]) => *c,
                _ => {
                    state().interface_unavailable = true;
                    return false;
                }
            };
            state().screensaver_cookie = cookie;
            cookie != 0
        } else {
            if !call_void_method_on_connection(
                conn,
                BUS_NAME,
                PATH,
                INTERFACE,
                "UnInhibit",
                &[Arg::U32(cookie)],
            ) {
                return false;
            }
            state().screensaver_cookie = 0;
            true
        }
    }
}

/// Dispatch whatever arrived on the session bus. Translation of
/// `SDL_DBus_PumpEvents()`.
pub(crate) fn pump_events() {
    if let Some(ctx) = current_context() {
        ctx.session_conn.pump();
    }
}

/// The machine ID. Translation of `SDL_DBus_GetLocalMachineId()`.
pub(crate) fn get_local_machine_id() -> Result<String> {
    let lib = lib().ok_or_else(|| Error::new("Error getting D-Bus machine ID"))?;
    let mut err = ErrorGuard::new(lib);
    let result = match lib.fns.try_get_local_machine_id {
        // Available since dbus 1.12.0, has proper error-handling
        // SAFETY: err is initialized.
        Some(f) => unsafe { f(&mut err.err) },
        /* Available since time immemorial, but has no error-handling:
         * if the machine ID can't be read, many versions of libdbus will
         * treat that as a fatal mis-installation and abort() */
        // SAFETY: no arguments.
        None => unsafe { (lib.fns.get_local_machine_id)() },
    };
    if !result.is_null() {
        // SAFETY: a NUL-terminated string allocated by libdbus, freed with dbus_free.
        let id = unsafe { CStr::from_ptr(result) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: as above.
        unsafe { (lib.fns.free)(result.cast()) };
        return Ok(id);
    }
    if err.is_set() {
        Err(err.to_error())
    } else {
        Err(Error::new("Error getting D-Bus machine ID"))
    }
}

/// Convert file drops with mime type "application/vnd.portal.filetransfer"
/// to file paths. Translation of `SDL_DBus_DocumentsPortalRetrieveFiles()`.
/// <https://flatpak.github.io/xdg-desktop-portal/#gdbus-method-org-freedesktop-portal-FileTransfer.RetrieveFiles>
pub(crate) fn documents_portal_retrieve_files(key: &str) -> Result<Vec<String>> {
    // Make sure we have a connection to the dbus session bus
    let Some(ctx) = context() else {
        /* We either cannot connect to the session bus or were unable to
         * load the D-Bus library at all. */
        return Err(Error::new(format!(
            "Error retrieving paths for documents portal \"{key}\""
        )));
    };
    let conn = &ctx.session_conn;
    let Some(mut msg) = conn.new_method_call(
        "org.freedesktop.portal.Documents",    // Node
        "/org/freedesktop/portal/documents",   // Path
        "org.freedesktop.portal.FileTransfer", // Interface
        "RetrieveFiles",                       // Method
    ) else {
        return Err(Error::out_of_memory());
    };

    // First argument is a "application/vnd.portal.filetransfer" key from a DnD or clipboard event
    /* Second argument is a variant dictionary for options.
     * The spec doesn't define any entries yet so it's empty. */
    if !msg.append_args(&[Arg::Str(key), Arg::StrDict(&[])]) {
        return Err(Error::out_of_memory());
    }

    let reply = conn.send_with_reply_and_block(&msg, DBUS_TIMEOUT_USE_DEFAULT)?;
    match reply.args().as_slice() {
        [Value::Array(paths), ..] => Ok(paths
            .iter()
            .filter_map(|p| p.as_str().map(str::to_owned))
            .collect()),
        _ => Err(Error::new(format!(
            "Error retrieving paths for documents portal \"{key}\""
        ))),
    }
}

/// What [`camera_portal_request_access`] can return besides a file
/// descriptor.
#[derive(Debug)]
pub(crate) enum CameraPortalAccess {
    /// A PipeWire remote file descriptor for the PipeWire camera driver.
    Granted(OwnedFd),
    /// Access was denied, or there is no portal (or no sandbox).
    Denied,
    /// The request failed.
    Error,
}

const SIGNAL_NAMEOWNERCHANGED: &str = "type='signal',\
        sender='org.freedesktop.DBus',\
        interface='org.freedesktop.DBus',\
        member='NameOwnerChanged',\
        arg0='org.freedesktop.portal.Desktop',\
        arg2=''";

/// Requests access for the camera. Translation of
/// `SDL_DBus_CameraPortalRequestAccess()` (upstream returns -1 on error, -2
/// on denied access or missing portal, else the file descriptor).
/// <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Camera.html>
pub(crate) fn camera_portal_request_access() -> CameraPortalAccess {
    if crate::init::sandbox() == crate::init::Sandbox::None {
        return CameraPortalAccess::Denied;
    }
    let Some(ctx) = context() else {
        return CameraPortalAccess::Denied;
    };
    camera_portal_request_access_on(&ctx.session_conn)
}

fn camera_portal_request_access_on(conn: &Connection) -> CameraPortalAccess {
    const NODE: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    const INTERFACE: &str = "org.freedesktop.portal.Camera";

    let Some(mut msg) = conn.new_method_call(NODE, PATH, INTERFACE, "AccessCamera") else {
        return CameraPortalAccess::Error;
    };
    if !msg.append_args(&[Arg::StrDict(&[])]) {
        return CameraPortalAccess::Error;
    }
    let request_path = match conn.send_with_reply_and_block(&msg, DBUS_TIMEOUT_USE_DEFAULT) {
        Ok(reply) => match reply.args().into_iter().next() {
            Some(Value::ObjectPath(p)) => p,
            _ => return CameraPortalAccess::Error,
        },
        Err(e) if error_has_name(&e, DBUS_ERROR_NAME_HAS_NO_OWNER) => {
            return CameraPortalAccess::Denied
        }
        Err(_) => return CameraPortalAccess::Error,
    };

    if conn.add_match(SIGNAL_NAMEOWNERCHANGED).is_err() {
        return CameraPortalAccess::Error;
    }

    /// Translation of `SDL_DBus_CameraPortalMessageHandlerData`.
    #[derive(Default)]
    struct HandlerData {
        response: Option<u32>,
        failed: bool,
        done: bool,
    }
    let data = Arc::new(Mutex::new(HandlerData::default()));
    let d2 = data.clone();
    // Translation of `SDL_DBus_CameraPortalMessageHandler()`.
    let filter = conn.add_filter(move |msg: &Message| {
        let mut data = d2.lock().unwrap_or_else(|e| e.into_inner());
        if msg.is_signal("org.freedesktop.DBus", "NameOwnerChanged") {
            let args = msg.args();
            let (Some(name), Some(new)) = (
                args.first().and_then(Value::as_str),
                args.get(2).and_then(Value::as_str),
            ) else {
                data.failed = true;
                data.done = true;
                return HandlerResult::NotYetHandled;
            };
            if name != "org.freedesktop.portal.Desktop" || !new.is_empty() {
                return HandlerResult::NotYetHandled;
            }
            data.done = true;
            data.response = Some(u32::MAX);
            return HandlerResult::Handled;
        }
        if !msg.has_path(&request_path)
            || !msg.is_signal("org.freedesktop.portal.Request", "Response")
        {
            return HandlerResult::NotYetHandled;
        }
        match msg.args().first() {
            Some(Value::U32(r)) => data.response = Some(*r),
            _ => data.failed = true,
        }
        data.done = true;
        HandlerResult::Handled
    });
    let Some(filter) = filter else {
        return CameraPortalAccess::Error;
    };
    while !data.lock().unwrap_or_else(|e| e.into_inner()).done && conn.read_write_dispatch(-1) {}

    if conn.remove_match(SIGNAL_NAMEOWNERCHANGED).is_err() {
        return CameraPortalAccess::Error;
    }
    drop(filter);
    let data = data.lock().unwrap_or_else(|e| e.into_inner());
    if !data.done || data.failed {
        return CameraPortalAccess::Error;
    }
    match data.response {
        Some(1) | Some(2) => return CameraPortalAccess::Denied,
        Some(0) => {}
        _ => return CameraPortalAccess::Error,
    }

    let Some(mut msg) = conn.new_method_call(NODE, PATH, INTERFACE, "OpenPipeWireRemote") else {
        return CameraPortalAccess::Error;
    };
    if !msg.append_args(&[Arg::StrDict(&[])]) {
        return CameraPortalAccess::Error;
    }
    match conn.send_with_reply_and_block(&msg, DBUS_TIMEOUT_USE_DEFAULT) {
        Ok(reply) => match reply.args().into_iter().next() {
            Some(Value::UnixFd(fd)) => CameraPortalAccess::Granted(fd),
            _ => CameraPortalAccess::Error,
        },
        Err(_) => CameraPortalAccess::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};

    /// A private session bus for the test, killed on drop.
    struct Bus {
        child: Child,
        address: String,
    }

    impl Bus {
        fn start() -> Option<Bus> {
            if lib().is_none() {
                println!("libdbus isn't available; skipping");
                return None;
            }
            let mut child = match Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(_) => {
                    println!("dbus-daemon isn't installed; skipping");
                    return None;
                }
            };
            let mut line = String::new();
            BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut line)
                .ok()?;
            Some(Bus {
                child,
                address: line.trim().to_owned(),
            })
        }
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    /// Answer method calls on `conn` from another thread until `stop`.
    fn serve(
        address: String,
        name: &'static str,
        handler: impl Fn(&Message) -> Option<Message> + Send + 'static,
    ) -> (
        std::thread::JoinHandle<()>,
        Arc<std::sync::atomic::AtomicBool>,
    ) {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let s2 = stop.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let t = std::thread::spawn(move || {
            let conn = Connection::open_address(&address).unwrap();
            assert_eq!(conn.request_name(name, 0).unwrap(), 1, "primary owner");
            let replies = Arc::new(Mutex::new(Vec::new()));
            let r2 = replies.clone();
            let _filter = conn
                .add_filter(move |msg| match handler(msg) {
                    Some(reply) => {
                        r2.lock().unwrap().push(reply);
                        HandlerResult::Handled
                    }
                    None => HandlerResult::NotYetHandled,
                })
                .unwrap();
            tx.send(()).unwrap();
            while !s2.load(std::sync::atomic::Ordering::SeqCst) {
                conn.read_write_dispatch(20);
                for reply in replies.lock().unwrap().drain(..) {
                    conn.send(&reply);
                }
            }
        });
        rx.recv().unwrap();
        (t, stop)
    }

    #[test]
    fn calls_properties_and_values() {
        let Some(bus) = Bus::start() else { return };
        let conn = Connection::open_address(&bus.address).unwrap();
        assert!(conn.is_connected());
        assert!(conn.unique_name().unwrap().starts_with(':'));

        // The bus itself answers these.
        let reply = call_method_on_connection(
            &conn,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            &[Arg::Str("org.freedesktop.DBus")],
        )
        .unwrap();
        assert_eq!(
            reply.args(),
            vec![Value::Str("org.freedesktop.DBus".into())]
        );

        let reply = call_method_on_connection(
            &conn,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "ListNames",
            &[],
        )
        .unwrap();
        let args = reply.args();
        let [Value::Array(names)] = args.as_slice() else {
            panic!("ListNames returns as");
        };
        assert!(names.contains(&Value::Str("org.freedesktop.DBus".into())));

        // A property of the bus: Features is an array of strings.
        let features = query_property_on_connection(
            &conn,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "Features",
        );
        assert!(matches!(features, Some(Value::Array(_))), "{features:?}");

        // A method of a name nobody owns fails (on a bus with activation,
        // as ServiceUnknown).
        let msg = conn
            .new_method_call("org.libsdl.Nobody", "/", "org.libsdl.Nobody", "Hello")
            .unwrap();
        let err = conn.send_with_reply_and_block(&msg, 1000).unwrap_err();
        assert!(
            error_has_name(&err, c"org.freedesktop.DBus.Error.ServiceUnknown"),
            "{}",
            err.message()
        );
        assert!(!error_has_name(&err, DBUS_ERROR_NAME_HAS_NO_OWNER));
        assert!(call_void_method_on_connection(
            &conn,
            "org.libsdl.Nobody",
            "/",
            "org.libsdl.Nobody",
            "Hello",
            &[]
        ));
    }

    #[test]
    fn values_round_trip_through_a_peer() {
        let Some(bus) = Bus::start() else { return };
        // An echo service: replies with the call's own arguments.
        let (t, stop) = serve(bus.address.clone(), "org.libsdl.Echo", |msg| {
            if !msg.is_method_call("org.libsdl.Echo", "Echo") {
                return None;
            }
            let mut reply = msg.new_method_return()?;
            let args = msg.args();
            let mut w = reply.writer();
            for a in &args {
                match a {
                    Value::Str(s) => w.append(&Arg::Str(s)),
                    Value::U32(v) => w.append(&Arg::U32(*v)),
                    Value::I64(v) => w.append(&Arg::I64(*v)),
                    Value::Bool(v) => w.append(&Arg::Bool(*v)),
                    Value::F64(v) => w.append(&Arg::F64(*v)),
                    Value::ObjectPath(p) => w.append(&Arg::ObjectPath(p)),
                    Value::Array(entries) => {
                        let pairs: Vec<(String, String)> = entries
                            .iter()
                            .filter_map(|e| match e {
                                Value::DictEntry(k, v) => Some((
                                    k.as_str()?.to_owned(),
                                    v.unvariant().as_str()?.to_owned(),
                                )),
                                _ => None,
                            })
                            .collect();
                        let refs: Vec<(&str, &str)> = pairs
                            .iter()
                            .map(|(k, v)| (k.as_str(), v.as_str()))
                            .collect();
                        w.append_dict_with_keys_and_values(&refs)
                    }
                    _ => false,
                };
            }
            Some(reply)
        });

        let conn = Connection::open_address(&bus.address).unwrap();
        let reply = call_method_on_connection(
            &conn,
            "org.libsdl.Echo",
            "/org/libsdl/Echo",
            "org.libsdl.Echo",
            "Echo",
            &[
                Arg::Str("héllo"),
                Arg::U32(0xDEAD_BEEF),
                Arg::I64(-5),
                Arg::Bool(true),
                Arg::F64(1.5),
                Arg::ObjectPath("/a/b"),
                Arg::StrDict(&[("reason", "Playing a game"), ("k", "v")]),
            ],
        )
        .unwrap();
        let entry = |k: &str, v: &str| {
            Value::DictEntry(
                Box::new(Value::Str(k.into())),
                Box::new(Value::Variant(Box::new(Value::Str(v.into())))),
            )
        };
        assert_eq!(
            reply.args(),
            vec![
                Value::Str("héllo".into()),
                Value::U32(0xDEAD_BEEF),
                Value::I64(-5),
                Value::Bool(true),
                Value::F64(1.5),
                Value::ObjectPath("/a/b".into()),
                Value::Array(vec![entry("reason", "Playing a game"), entry("k", "v")]),
            ]
        );
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        t.join().unwrap();
    }

    #[test]
    fn screensaver_inhibit_through_the_session_bus() {
        let _l = crate::test_support::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let Some(bus) = Bus::start() else { return };
        if crate::init::sandbox() != crate::init::Sandbox::None {
            println!("running in a sandbox; skipping");
            return;
        }
        // A fake org.freedesktop.ScreenSaver: Inhibit returns cookie 42.
        let calls = Arc::new(Mutex::new(Vec::new()));
        let c2 = calls.clone();
        let (t, stop) = serve(
            bus.address.clone(),
            "org.freedesktop.ScreenSaver",
            move |msg| {
                for method in ["Inhibit", "UnInhibit", "SimulateUserActivity"] {
                    if msg.is_method_call("org.freedesktop.ScreenSaver", method) {
                        c2.lock().unwrap().push((method, msg.args()));
                        let mut reply = msg.new_method_return()?;
                        if method == "Inhibit" {
                            reply.append_args(&[Arg::U32(42)]);
                        }
                        return Some(reply);
                    }
                }
                None
            },
        );

        // Point the session context at the private bus.
        quit();
        {
            let mut s = state();
            s.is_dbus_available = true;
            s.interface_unavailable = false;
            let session_conn = Connection::open_address(&bus.address).unwrap();
            s.context = Some(Arc::new(Context {
                session_conn,
                system_conn: None,
            }));
            s.initialized = true;
        }
        crate::hints::set(crate::hints::SCREENSAVER_INHIBIT_ACTIVITY_NAME, "Testing").unwrap();
        assert!(screensaver_inhibit(true));
        assert_eq!(state().screensaver_cookie, 42);
        assert!(screensaver_inhibit(true), "already inhibited");
        screensaver_tickle(); // nothing to do while inhibiting
        assert!(screensaver_inhibit(false));
        assert_eq!(state().screensaver_cookie, 0);
        screensaver_tickle();
        // (the void calls are queued; let them arrive)
        let start = std::time::Instant::now();
        while calls.lock().unwrap().len() < 3 && start.elapsed() < std::time::Duration::from_secs(5)
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let calls = calls.lock().unwrap();
        let app = crate::init::app_metadata_property(crate::init::AppMetadata::Name).unwrap();
        assert_eq!(
            calls[0],
            (
                "Inhibit",
                vec![Value::Str(app), Value::Str("Testing".into())]
            )
        );
        assert_eq!(calls[1], ("UnInhibit", vec![Value::U32(42)]));
        assert_eq!(calls[2].0, "SimulateUserActivity");
        drop(calls);

        crate::hints::reset(crate::hints::SCREENSAVER_INHIBIT_ACTIVITY_NAME);
        quit();
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        t.join().unwrap();
    }

    #[test]
    fn camera_portal_without_a_portal() {
        let Some(bus) = Bus::start() else { return };
        let conn = Connection::open_address(&bus.address).unwrap();
        // No portal on the private bus. The bus answers ServiceUnknown,
        // which upstream treats as an error (only NameHasNoOwner means
        // "no portal").
        assert!(matches!(
            camera_portal_request_access_on(&conn),
            CameraPortalAccess::Error
        ));
    }

    #[test]
    fn machine_id() {
        if lib().is_none() {
            return;
        }
        // Containers may lack /etc/machine-id; then the error is reported.
        match get_local_machine_id() {
            Ok(id) => assert_eq!(id.len(), 32, "{id}"),
            Err(e) => assert!(!e.message().is_empty()),
        }
    }
}
