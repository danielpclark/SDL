// The Rust side of libwayland-client for the Wayland video driver of Simple
// DirectMedia Layer: what wayland-client.h and the wayland-scanner output
// provide to upstream's C code.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Owned and borrowed proxies, event queues and the display connection.
//!
//! * [`Conn`] is the `wl_display` connection (disconnected when the last
//!   reference goes, unless it belongs to the application).
//! * [`Proxy<I>`] owns a protocol object of interface `I` and destroys it on
//!   drop, with the interface's destructor request when it has one (the
//!   `*_destroy()`/`*_release()` functions of the scanner's C header); a
//!   proxy created with [`Obj::create_wrapper`] is a proxy wrapper instead
//!   (`wl_proxy_wrapper_destroy()` on drop).
//! * [`Obj<'_, I>`] is a borrowed object: a request target or argument, or
//!   an object passed in an event.
//! * Listeners are closures: [`Proxy::listen`] installs one dispatcher per
//!   interface (`wl_proxy_add_dispatcher()`), which decodes the event's
//!   arguments into the interface's event enum and calls the closure.
//!
//! A panic in a listener is caught at the C boundary and resumed when the
//! libwayland call that dispatched it returns.

use std::any::Any;
use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::marker::PhantomData;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use std::ptr::{self, NonNull};
use std::sync::{Arc, Mutex, Weak};

use super::protocols::wayland::{WlCallback, WlCallbackEvent, WlDisplay};
use super::sys::*;
use super::wldyn::WaylandSyms;

// ---------------------------------------------------------------------------
// Panics in callbacks
// ---------------------------------------------------------------------------

thread_local! {
    /// A panic caught in a listener, to be resumed on the way out of C.
    static PENDING_PANIC: RefCell<Option<Box<dyn Any + Send>>> = const { RefCell::new(None) };
}

/// Run a callback called from C, catching a panic (it is resumed by
/// [`resume_pending_panic`] once the C code returns).
pub(crate) fn catch_callback<R: Default>(f: impl FnOnce() -> R) -> R {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(payload) => {
            PENDING_PANIC.with(|p| {
                let mut p = p.borrow_mut();
                if p.is_none() {
                    *p = Some(payload);
                }
            });
            R::default()
        }
    }
}

/// Resume a panic caught in a callback during the last call into C.
pub(crate) fn resume_pending_panic() {
    if let Some(payload) = PENDING_PANIC.with(|p| p.borrow_mut().take()) {
        std::panic::resume_unwind(payload);
    }
}

// ---------------------------------------------------------------------------
// Fixed point numbers
// ---------------------------------------------------------------------------

/// `wl_fixed_t`: a signed 24.8 fixed point number.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub(crate) struct Fixed(pub(crate) wl_fixed_t);

impl Fixed {
    /// Translation of `wl_fixed_to_double()`.
    pub(crate) fn to_f64(self) -> f64 {
        self.0 as f64 / 256.0
    }

    /// Translation of `wl_fixed_from_double()` (rounding to nearest, ties to
    /// even, as its floating point trick does).
    pub(crate) fn from_f64(d: f64) -> Fixed {
        let v = d + (3i64 << (51 - 8)) as f64;
        Fixed(v.to_bits() as i64 as i32)
    }

    /// Translation of `wl_fixed_to_int()`.
    pub(crate) fn to_int(self) -> i32 {
        self.0 / 256
    }

    /// Translation of `wl_fixed_from_int()`.
    pub(crate) fn from_int(i: i32) -> Fixed {
        Fixed(i.wrapping_mul(256))
    }
}

// ---------------------------------------------------------------------------
// Interfaces
// ---------------------------------------------------------------------------

/// The event type of an interface without events.
#[derive(Debug)]
pub(crate) enum NoEvents {}

/// A protocol interface (implemented by the generated marker types of
/// [`super::protocols`]).
pub(crate) trait Interface: 'static + Sized {
    /// The interface name.
    const NAME: &'static str;
    /// The highest version the protocol description knows.
    #[allow(dead_code)] // (part of the protocol description; read by the tests)
    const VERSION: u32;
    /// The request a dropped proxy is destroyed with: its opcode and the
    /// version it appeared in (older proxies are just destroyed locally).
    const DESTRUCTOR: Option<(u32, u32)>;
    /// The events, borrowing their strings, arrays and objects from the
    /// dispatch.
    type Event<'a>;

    /// The `wl_interface` table.
    fn interface() -> &'static wl_interface;

    /// Decode the arguments of event `opcode`.
    ///
    /// # Safety
    ///
    /// `args` holds the arguments of event `opcode` of this interface, as
    /// libwayland passes them to a dispatcher.
    unsafe fn decode<'a>(
        conn: &'a Arc<Conn>,
        opcode: u32,
        args: *const wl_argument,
    ) -> Option<Self::Event<'a>>;
}

/// The proxy tag strings of `SDL_WAYLAND_register_surface()` and
/// friends: `wl_proxy_set_tag()` takes the address of a string pointer.
#[repr(transparent)]
pub(crate) struct Tag(pub(crate) *const c_char);

// SAFETY: the tag points at a string literal; it is only compared and read.
unsafe impl Sync for Tag {}

// ---------------------------------------------------------------------------
// The connection
// ---------------------------------------------------------------------------

/// A connection to a Wayland compositor (`struct wl_display *`).
pub(crate) struct Conn {
    syms: Arc<WaylandSyms>,
    display: NonNull<wl_display>,
    /// Whether this is our connection to disconnect (else it is the
    /// application's, from `SDL_PROP_GLOBAL_VIDEO_WAYLAND_WL_DISPLAY_POINTER`).
    owned: bool,
}

// SAFETY: libwayland-client serializes all use of a display with its
// internal mutex; requests may be sent from any thread.
unsafe impl Send for Conn {}
// SAFETY: as above.
unsafe impl Sync for Conn {}

impl std::fmt::Debug for Conn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Conn({:p})", self.display)
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: the display was connected by Conn::connect and every
            // proxy of it holds a reference to this Conn, so none is left.
            unsafe { (self.syms.client.wl_display_disconnect)(self.display.as_ptr()) };
        }
    }
}

/// The result of a libwayland call that returns -1 and sets `errno` on
/// failure: the `errno` value.
pub(crate) type WlResult<T> = std::result::Result<T, i32>;

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

impl Conn {
    /// Connect to the compositor in `WAYLAND_DISPLAY` (`wl_display_connect(NULL)`).
    pub(crate) fn connect(syms: &Arc<WaylandSyms>) -> WlResult<Arc<Conn>> {
        // SAFETY: a NULL name selects $WAYLAND_DISPLAY (or $WAYLAND_SOCKET).
        let display = unsafe { (syms.client.wl_display_connect)(ptr::null()) };
        match NonNull::new(display) {
            Some(display) => Ok(Arc::new(Conn {
                syms: syms.clone(),
                display,
                owned: true,
            })),
            None => Err(errno()),
        }
    }

    /// Use a display the application connected.
    ///
    /// # Safety
    ///
    /// `display` is a live `struct wl_display *` that outlives the returned
    /// connection and everything created on it.
    pub(crate) unsafe fn external(
        syms: &Arc<WaylandSyms>,
        display: NonNull<wl_display>,
    ) -> Arc<Conn> {
        Arc::new(Conn {
            syms: syms.clone(),
            display,
            owned: false,
        })
    }

    /// The loaded functions.
    pub(crate) fn syms(&self) -> &Arc<WaylandSyms> {
        &self.syms
    }

    /// The `struct wl_display *`.
    pub(crate) fn raw(&self) -> *mut wl_display {
        self.display.as_ptr()
    }

    /// Whether the application owns the display.
    pub(crate) fn is_external(&self) -> bool {
        !self.owned
    }

    /// The display as a protocol object (its requests are `sync` and
    /// `get_registry`).
    pub(crate) fn obj<'a>(self: &'a Arc<Self>) -> Obj<'a, WlDisplay> {
        Obj {
            raw: self.display.cast(),
            conn: self,
            _marker: PhantomData,
        }
    }

    fn check(r: c_int) -> WlResult<i32> {
        if r < 0 {
            Err(errno())
        } else {
            Ok(r)
        }
    }

    /// `wl_display_get_fd()`
    pub(crate) fn fd(&self) -> RawFd {
        // SAFETY: the display is connected.
        unsafe { (self.syms.client.wl_display_get_fd)(self.raw()) }
    }

    /// `wl_display_roundtrip()`
    pub(crate) fn roundtrip(&self) -> WlResult<i32> {
        // SAFETY: the display is connected; listeners may run.
        let r = unsafe { (self.syms.client.wl_display_roundtrip)(self.raw()) };
        let e = errno();
        resume_pending_panic();
        if r < 0 {
            Err(e)
        } else {
            Ok(r)
        }
    }

    /// `wl_display_dispatch()`
    pub(crate) fn dispatch(&self) -> WlResult<i32> {
        // SAFETY: as above.
        let r = unsafe { (self.syms.client.wl_display_dispatch)(self.raw()) };
        let e = errno();
        resume_pending_panic();
        if r < 0 {
            Err(e)
        } else {
            Ok(r)
        }
    }

    /// `wl_display_dispatch_pending()`
    pub(crate) fn dispatch_pending(&self) -> WlResult<i32> {
        // SAFETY: as above.
        let r = unsafe { (self.syms.client.wl_display_dispatch_pending)(self.raw()) };
        let e = errno();
        resume_pending_panic();
        if r < 0 {
            Err(e)
        } else {
            Ok(r)
        }
    }

    /// `wl_display_dispatch_queue()`
    pub(crate) fn dispatch_queue(&self, queue: &EventQueue) -> WlResult<i32> {
        // SAFETY: the queue belongs to this display.
        let r = unsafe { (self.syms.client.wl_display_dispatch_queue)(self.raw(), queue.raw()) };
        let e = errno();
        resume_pending_panic();
        if r < 0 {
            Err(e)
        } else {
            Ok(r)
        }
    }

    /// `wl_display_dispatch_queue_pending()`
    pub(crate) fn dispatch_queue_pending(&self, queue: &EventQueue) -> WlResult<i32> {
        // SAFETY: as above.
        let r = unsafe {
            (self.syms.client.wl_display_dispatch_queue_pending)(self.raw(), queue.raw())
        };
        let e = errno();
        resume_pending_panic();
        if r < 0 {
            Err(e)
        } else {
            Ok(r)
        }
    }

    /// `wl_display_prepare_read()`: `true` when prepared, `false` (-1) if
    /// events are queued already.
    pub(crate) fn prepare_read(&self) -> bool {
        // SAFETY: the display is connected.
        unsafe { (self.syms.client.wl_display_prepare_read)(self.raw()) == 0 }
    }

    /// `wl_display_prepare_read_queue()`
    pub(crate) fn prepare_read_queue(&self, queue: &EventQueue) -> bool {
        // SAFETY: the queue belongs to this display.
        unsafe { (self.syms.client.wl_display_prepare_read_queue)(self.raw(), queue.raw()) == 0 }
    }

    /// `wl_display_read_events()`
    pub(crate) fn read_events(&self) -> WlResult<i32> {
        // SAFETY: the display is connected (and prepared for reading).
        Self::check(unsafe { (self.syms.client.wl_display_read_events)(self.raw()) })
    }

    /// `wl_display_cancel_read()`
    pub(crate) fn cancel_read(&self) {
        // SAFETY: the display is connected (and prepared for reading).
        unsafe { (self.syms.client.wl_display_cancel_read)(self.raw()) }
    }

    /// `wl_display_flush()`
    pub(crate) fn flush(&self) -> WlResult<i32> {
        // SAFETY: the display is connected.
        Self::check(unsafe { (self.syms.client.wl_display_flush)(self.raw()) })
    }

    /// `wl_display_get_error()`
    #[allow(dead_code)] // (part of the connection API; read by the tests)
    pub(crate) fn error(&self) -> i32 {
        // SAFETY: the display is connected.
        unsafe { (self.syms.client.wl_display_get_error)(self.raw()) }
    }

    /// A new event queue. Translation of `Wayland_DisplayCreateQueue()`
    /// (named when the library supports it).
    pub(crate) fn create_queue(self: &Arc<Self>, name: &CStr) -> Option<EventQueue> {
        // SAFETY: the display is connected; the name is NUL-terminated.
        let raw = unsafe {
            match self.syms.client.wl_display_create_queue_with_name {
                Some(f) => f(self.raw(), name.as_ptr()),
                None => (self.syms.client.wl_display_create_queue)(self.raw()),
            }
        };
        NonNull::new(raw).map(|raw| EventQueue {
            raw,
            conn: self.clone(),
        })
    }
}

/// An event queue (`struct wl_event_queue *`), destroyed on drop.
pub(crate) struct EventQueue {
    raw: NonNull<wl_event_queue>,
    conn: Arc<Conn>,
}

// SAFETY: queues are dispatched with libwayland's locking; the pointer is
// only passed to libwayland.
unsafe impl Send for EventQueue {}
// SAFETY: as above.
unsafe impl Sync for EventQueue {}

impl std::fmt::Debug for EventQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EventQueue({:p})", self.raw)
    }
}

impl EventQueue {
    /// The `struct wl_event_queue *`.
    pub(crate) fn raw(&self) -> *mut wl_event_queue {
        self.raw.as_ptr()
    }
}

impl Drop for EventQueue {
    fn drop(&mut self) {
        // SAFETY: the queue was created on this display; the proxies of
        // the queue are gone or moved by the time it is dropped.
        unsafe { (self.conn.syms.client.wl_event_queue_destroy)(self.raw()) }
    }
}

// ---------------------------------------------------------------------------
// Objects
// ---------------------------------------------------------------------------

/// A borrowed protocol object of interface `I`.
pub(crate) struct Obj<'a, I> {
    raw: NonNull<wl_proxy>,
    conn: &'a Arc<Conn>,
    _marker: PhantomData<fn() -> I>,
}

impl<I> Clone for Obj<'_, I> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<I> Copy for Obj<'_, I> {}

impl<I: Interface> std::fmt::Debug for Obj<'_, I> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{:p}", I::NAME, self.raw)
    }
}

/// An object of an interface the protocol doesn't specify.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RawObj(pub(crate) NonNull<wl_proxy>);

impl RawObj {
    /// The `struct wl_proxy *`.
    #[allow(dead_code)] // (part of the proxy API; read by the tests)
    pub(crate) fn raw(self) -> *mut wl_proxy {
        self.0.as_ptr()
    }
}

impl<'a, I: Interface> Obj<'a, I> {
    /// Borrow a proxy created elsewhere (by libdecor, the application...).
    ///
    /// # Safety
    ///
    /// `raw` is NULL or a live proxy of interface `I` on `conn`'s display,
    /// alive for `'a`.
    pub(crate) unsafe fn from_raw(raw: *mut c_void, conn: &'a Arc<Conn>) -> Option<Obj<'a, I>> {
        NonNull::new(raw.cast()).map(|raw| Obj {
            raw,
            conn,
            _marker: PhantomData,
        })
    }

    /// The `struct wl_proxy *`.
    pub(crate) fn raw(self) -> *mut wl_proxy {
        self.raw.as_ptr()
    }

    /// The connection.
    #[allow(dead_code)] // (part of the proxy API; read by the tests)
    pub(crate) fn conn(self) -> &'a Arc<Conn> {
        self.conn
    }

    fn client(self) -> &'a super::wldyn::WaylandClient {
        &self.conn.syms.client
    }

    /// `wl_proxy_get_version()`
    pub(crate) fn version(self) -> u32 {
        // SAFETY: the proxy is alive.
        unsafe { (self.client().wl_proxy_get_version)(self.raw()) }
    }

    /// `wl_proxy_get_id()`
    pub(crate) fn id(self) -> u32 {
        // SAFETY: the proxy is alive.
        unsafe { (self.client().wl_proxy_get_id)(self.raw()) }
    }

    /// `wl_proxy_get_user_data()`, as the number it was set to.
    pub(crate) fn user_data(self) -> usize {
        // SAFETY: the proxy is alive.
        unsafe { (self.client().wl_proxy_get_user_data)(self.raw()) as usize }
    }

    /// `wl_proxy_set_user_data()` with a number (an ID; never dereferenced).
    pub(crate) fn set_user_data(self, data: usize) {
        // SAFETY: the proxy is alive; the data is only read back.
        unsafe { (self.client().wl_proxy_set_user_data)(self.raw(), data as *mut c_void) }
    }

    /// `wl_proxy_set_tag()`
    pub(crate) fn set_tag(self, tag: &'static Tag) {
        // SAFETY: the proxy is alive; the tag is a static.
        unsafe { (self.client().wl_proxy_set_tag)(self.raw(), &tag.0) }
    }

    /// Whether `wl_proxy_get_tag()` is `tag`.
    pub(crate) fn has_tag(self, tag: &'static Tag) -> bool {
        // SAFETY: the proxy is alive.
        let t = unsafe { (self.client().wl_proxy_get_tag)(self.raw()) };
        ptr::eq(t, &tag.0)
    }

    /// Whether two objects are the same.
    #[allow(dead_code)] // (part of the proxy API; read by the tests)
    pub(crate) fn same(self, other: Obj<'_, I>) -> bool {
        self.raw == other.raw
    }

    /// Send request `opcode`. Translation of the scanner's request
    /// functions (`wl_proxy_marshal_flags()`).
    ///
    /// # Safety
    ///
    /// `args` matches the signature of request `opcode` of `I`; with
    /// [`WL_MARSHAL_FLAG_DESTROY`] the proxy is destroyed and must not be used
    /// again.
    pub(crate) unsafe fn marshal(self, opcode: u32, args: &mut [wl_argument], flags: u32) {
        // SAFETY: as documented.
        unsafe {
            (self.client().wl_proxy_marshal_array_flags)(
                self.raw(),
                opcode,
                ptr::null(),
                self.version(),
                flags,
                args.as_mut_ptr(),
            );
        }
    }

    /// Send request `opcode`, which creates an object of interface `J`.
    ///
    /// # Safety
    ///
    /// As [`marshal`](Self::marshal), and `J` is the interface of the request's
    /// `new_id` argument.
    pub(crate) unsafe fn marshal_constructor<J: Interface>(
        self,
        opcode: u32,
        args: &mut [wl_argument],
        version: u32,
        flags: u32,
    ) -> Proxy<J> {
        // SAFETY: as documented.
        let raw = unsafe {
            (self.client().wl_proxy_marshal_array_flags)(
                self.raw(),
                opcode,
                J::interface(),
                version,
                flags,
                args.as_mut_ptr(),
            )
        };
        // (libwayland fails only when it can't allocate the proxy)
        let raw = NonNull::new(raw).expect("out of memory creating a Wayland proxy");
        Proxy::adopt(raw, self.conn.clone(), false)
    }

    /// A proxy wrapper of this object whose new objects (and their events)
    /// go to `queue`: `wl_proxy_create_wrapper()` and `wl_proxy_set_queue()`.
    pub(crate) fn create_wrapper(self, queue: &EventQueue) -> Proxy<I> {
        // SAFETY: the proxy is alive; the queue belongs to its display.
        let raw = unsafe {
            let wrapper = (self.client().wl_proxy_create_wrapper)(self.raw().cast());
            if !wrapper.is_null() {
                (self.client().wl_proxy_set_queue)(wrapper.cast(), queue.raw());
            }
            wrapper
        };
        let raw = NonNull::new(raw.cast()).expect("out of memory creating a Wayland proxy wrapper");
        Proxy::adopt(raw, self.conn.clone(), true)
    }
}

/// Something that is (or borrows) an object of [`AsProxy::Interface`]: the
/// requests of the interface are available on it.
pub(crate) trait AsProxy {
    /// The interface.
    type Interface: Interface;
    /// The object.
    fn obj(&self) -> Obj<'_, Self::Interface>;
}

impl<I: Interface> AsProxy for Obj<'_, I> {
    type Interface = I;
    fn obj(&self) -> Obj<'_, I> {
        *self
    }
}

/// The listener of a proxy: the closure and the connection it borrows its
/// objects from.
struct Handler<I: Interface> {
    conn: Arc<Conn>,
    #[allow(clippy::type_complexity)]
    f: Box<dyn Fn(Obj<'_, I>, I::Event<'_>) + Send + Sync>,
}

/// An owned protocol object of interface `I`, destroyed on drop.
pub(crate) struct Proxy<I: Interface> {
    raw: NonNull<wl_proxy>,
    conn: Arc<Conn>,
    /// The listener (an `Arc<Handler<I>>` turned into a raw pointer: the
    /// dispatcher's data).
    handler: Option<NonNull<Handler<I>>>,
    /// A proxy wrapper rather than an object.
    wrapper: bool,
}

// SAFETY: libwayland-client serializes requests with the display mutex; the
// listener is `Send + Sync`, and events are only dispatched on the thread
// dispatching the proxy's queue.
unsafe impl<I: Interface> Send for Proxy<I> {}
// SAFETY: as above.
unsafe impl<I: Interface> Sync for Proxy<I> {}

impl<I: Interface> std::fmt::Debug for Proxy<I> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{:p}", I::NAME, self.raw)
    }
}

impl<I: Interface> AsProxy for Proxy<I> {
    type Interface = I;
    fn obj(&self) -> Obj<'_, I> {
        Obj {
            raw: self.raw,
            conn: &self.conn,
            _marker: PhantomData,
        }
    }
}

impl<I: Interface> PartialEq<Obj<'_, I>> for Proxy<I> {
    fn eq(&self, other: &Obj<'_, I>) -> bool {
        self.raw == other.raw
    }
}

/// The dispatcher of proxies of interface `I` (`wl_dispatcher_func_t`).
unsafe extern "C" fn dispatcher<I: Interface>(
    data: *const c_void,
    target: *mut c_void,
    opcode: u32,
    _msg: *const wl_message,
    args: *mut wl_argument,
) -> c_int {
    let data = data.cast::<Handler<I>>();
    // SAFETY: `data` is the `Arc<Handler<I>>` of a live proxy (libwayland
    // doesn't dispatch to destroyed proxies); another reference is taken
    // so the listener may drop its own proxy.
    let handler = unsafe {
        Arc::increment_strong_count(data);
        Arc::from_raw(data)
    };
    catch_callback(|| {
        // SAFETY: libwayland passes the arguments of event `opcode`.
        if let Some(event) = unsafe { I::decode(&handler.conn, opcode, args) } {
            // SAFETY: the target is the live proxy the event is for.
            if let Some(obj) = unsafe { Obj::from_raw(target, &handler.conn) } {
                (handler.f)(obj, event);
            }
        }
    });
    0
}

impl<I: Interface> Proxy<I> {
    /// Take ownership of a proxy.
    fn adopt(raw: NonNull<wl_proxy>, conn: Arc<Conn>, wrapper: bool) -> Proxy<I> {
        Proxy {
            raw,
            conn,
            handler: None,
            wrapper,
        }
    }

    /// Take ownership of a proxy created by libwayland for an event.
    ///
    /// # Safety
    ///
    /// `raw` is a new proxy of interface `I` that nothing else owns.
    pub(crate) unsafe fn from_new(raw: NonNull<wl_proxy>, conn: &Arc<Conn>) -> Proxy<I> {
        Proxy::adopt(raw, conn.clone(), false)
    }

    /// The `struct wl_proxy *`.
    pub(crate) fn raw(&self) -> *mut wl_proxy {
        self.raw.as_ptr()
    }

    /// The connection.
    pub(crate) fn conn(&self) -> &Arc<Conn> {
        &self.conn
    }

    /// `wl_proxy_get_version()`
    pub(crate) fn version(&self) -> u32 {
        self.obj().version()
    }

    /// Install the listener (`*_add_listener()`): `f` is called with the
    /// object and each event. The current user data is kept.
    pub(crate) fn listen(&mut self, f: impl Fn(Obj<'_, I>, I::Event<'_>) + Send + Sync + 'static) {
        let handler = Arc::new(Handler {
            conn: self.conn.clone(),
            f: Box::new(f),
        });
        let data = Arc::into_raw(handler);
        let user_data = self.obj().user_data();
        // SAFETY: the proxy is alive and has no listener yet; the
        // dispatcher matches the handler's type; the handler is kept alive
        // until the proxy is destroyed.
        let r = unsafe {
            (self.conn.syms.client.wl_proxy_add_dispatcher)(
                self.raw(),
                dispatcher::<I>,
                data.cast(),
                user_data as *mut c_void,
            )
        };
        if r != 0 {
            // (already had a listener: libwayland logs this)
            // SAFETY: the reference made above, not handed to libwayland.
            drop(unsafe { Arc::from_raw(data) });
            return;
        }
        // SAFETY: Arc::into_raw never returns NULL.
        self.handler = Some(unsafe { NonNull::new_unchecked(data.cast_mut()) });
    }

    /// Forget a proxy that a destructor request just destroyed.
    pub(crate) fn forget_destroyed(self) {
        let mut this = std::mem::ManuallyDrop::new(self);
        this.drop_handler();
        // SAFETY: the connection is dropped exactly once, here, instead of
        // by Drop (which isn't run).
        unsafe { ptr::drop_in_place(&mut this.conn) };
    }

    fn drop_handler(&mut self) {
        if let Some(handler) = self.handler.take() {
            // SAFETY: the reference made by listen(); the proxy is destroyed,
            // so libwayland won't dispatch to it any more (a dispatch in
            // progress holds its own reference).
            drop(unsafe { Arc::from_raw(handler.as_ptr()) });
        }
    }
}

impl<I: Interface> Drop for Proxy<I> {
    fn drop(&mut self) {
        let client = &self.conn.syms.client;
        // SAFETY: the proxy is alive and ours; it is destroyed once, here.
        unsafe {
            if self.wrapper {
                (client.wl_proxy_wrapper_destroy)(self.raw().cast());
            } else {
                match I::DESTRUCTOR {
                    Some((opcode, since)) if self.version() >= since => {
                        (client.wl_proxy_marshal_array_flags)(
                            self.raw(),
                            opcode,
                            ptr::null(),
                            self.version(),
                            WL_MARSHAL_FLAG_DESTROY,
                            ptr::null_mut(),
                        );
                    }
                    _ => (client.wl_proxy_destroy)(self.raw()),
                }
            }
        }
        self.drop_handler();
    }
}

// ---------------------------------------------------------------------------
// Event argument decoding (used by the generated decoders)
// ---------------------------------------------------------------------------

/// A `string` argument.
///
/// # Safety
///
/// `a` is a string argument of an event being dispatched.
pub(crate) unsafe fn arg_str<'a>(a: &wl_argument) -> Option<&'a CStr> {
    // SAFETY: as documented; libwayland passes NUL-terminated strings (or
    // NULL) that live for the dispatch.
    unsafe {
        if a.s.is_null() {
            None
        } else {
            Some(CStr::from_ptr(a.s))
        }
    }
}

/// An `object` argument.
///
/// # Safety
///
/// `a` is an object argument of interface `I` of an event being dispatched.
pub(crate) unsafe fn arg_obj<'a, I: Interface>(
    conn: &'a Arc<Conn>,
    a: &wl_argument,
) -> Option<Obj<'a, I>> {
    // SAFETY: as documented.
    unsafe { Obj::from_raw(a.o.cast(), conn) }
}

/// An `object` argument of an unspecified interface.
///
/// # Safety
///
/// `a` is an object argument of an event being dispatched.
pub(crate) unsafe fn arg_raw_obj(a: &wl_argument) -> Option<RawObj> {
    // SAFETY: as documented.
    NonNull::new(unsafe { a.o }).map(RawObj)
}

/// A `new_id` argument: the object libwayland created for the event.
///
/// # Safety
///
/// `a` is a new_id argument of interface `I` of an event being dispatched.
pub(crate) unsafe fn arg_new<I: Interface>(conn: &Arc<Conn>, a: &wl_argument) -> Option<Proxy<I>> {
    // SAFETY: as documented; the new proxy is ours.
    NonNull::new(unsafe { a.o }).map(|raw| unsafe { Proxy::from_new(raw, conn) })
}

/// An `array` argument.
///
/// # Safety
///
/// `a` is an array argument of an event being dispatched.
pub(crate) unsafe fn arg_array<'a>(a: &wl_argument) -> &'a [u8] {
    // SAFETY: as documented; the array lives for the dispatch.
    unsafe {
        let array = a.a;
        if array.is_null() || (*array).size == 0 || (*array).data.is_null() {
            &[]
        } else {
            std::slice::from_raw_parts((*array).data.cast::<u8>(), (*array).size)
        }
    }
}

/// An `fd` argument, which the receiver owns.
///
/// # Safety
///
/// `a` is an fd argument of an event being dispatched.
pub(crate) unsafe fn arg_fd(a: &wl_argument) -> OwnedFd {
    // SAFETY: as documented; libwayland hands the descriptor to the listener.
    unsafe { OwnedFd::from_raw_fd(a.h) }
}

/// A `string` argument of a request: the text up to its first NUL.
pub(crate) fn cstring_arg(s: &str) -> CString {
    let bytes = s.as_bytes();
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    // (no NUL in the slice up to `end`)
    CString::new(&bytes[..end]).unwrap_or_default()
}

/// The 32-bit values of an `array` argument (`wl_array_for_each()` over
/// `uint32_t`s).
pub(crate) fn array_u32(a: &[u8]) -> impl Iterator<Item = u32> + '_ {
    a.chunks_exact(4)
        .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
}

// ---------------------------------------------------------------------------
// One-shot callbacks
// ---------------------------------------------------------------------------

/// The `wl_callback`s of sync points in flight (upstream's callbacks that
/// destroy themselves in their done handler): each runs its closure once and
/// is destroyed; the ones still pending are destroyed with the set.
#[derive(Clone, Default)]
pub(crate) struct Callbacks(Arc<Mutex<Vec<Proxy<WlCallback>>>>);

impl std::fmt::Debug for Callbacks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Callbacks")
    }
}

impl Callbacks {
    /// An empty set.
    pub(crate) fn new() -> Callbacks {
        Callbacks::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Proxy<WlCallback>>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run `f` with the callback data when `cb` is done, then destroy it.
    pub(crate) fn add(&self, mut cb: Proxy<WlCallback>, f: impl FnOnce(u32) + Send + 'static) {
        let set: Weak<Mutex<Vec<Proxy<WlCallback>>>> = Arc::downgrade(&self.0);
        let f = Mutex::new(Some(f));
        cb.listen(move |cb, event| {
            let WlCallbackEvent::Done { callback_data } = event;
            // Destroy the callback (it fires once).
            let mine = set.upgrade().and_then(|set| {
                let mut set = set.lock().unwrap_or_else(|e| e.into_inner());
                let i = set.iter().position(|p| p.raw() == cb.raw())?;
                Some(set.remove(i))
            });
            drop(mine);
            let f = f.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(f) = f {
                f(callback_data);
            }
        });
        self.lock().push(cb);
    }

    /// Destroy the callbacks that haven't fired.
    pub(crate) fn clear(&self) {
        let pending = std::mem::take(&mut *self.lock());
        drop(pending);
    }
}
