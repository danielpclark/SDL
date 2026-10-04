// Rust declarations of the parts of wayland-client-core.h, wayland-util.h,
// wayland-cursor.h, xkbcommon.h, xkbcommon-compose.h and libdecor.h used by
// the Wayland video driver of Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The C types and constants of the libraries the Wayland driver loads at
//! run time (libwayland-client, libwayland-cursor, libxkbcommon, libdecor),
//! declared by hand. The function pointers are in [`super::wldyn`].
//!
//! libdecor is declared as its 0.2 release series (the `libdecor_interface`
//! and `libdecor_frame_interface` tables with their reserved slots).

#![allow(non_camel_case_types)] // (the C names are kept)
#![allow(dead_code)] // (the declarations follow the headers)

use std::ffi::{c_char, c_int, c_void};

// ---------------------------------------------------------------------------
// wayland-util.h / wayland-client-core.h
// ---------------------------------------------------------------------------

/// `struct wl_proxy` (opaque).
pub(crate) enum wl_proxy {}
/// `struct wl_display` (opaque; it starts with a `wl_proxy`).
pub(crate) enum wl_display {}
/// `struct wl_event_queue` (opaque).
pub(crate) enum wl_event_queue {}

/// A pointer to an interface table in a `wl_message`'s `types` (a
/// `const struct wl_interface *`, NULL for non-object arguments).
pub(crate) type IfacePtr = Option<&'static wl_interface>;

/// `struct wl_message`: a request or event of an interface.
#[repr(C)]
pub(crate) struct wl_message {
    /// Message name
    pub(crate) name: *const c_char,
    /// Message signature
    pub(crate) signature: *const c_char,
    /// Object argument interfaces
    pub(crate) types: *const IfacePtr,
}

// SAFETY: the message tables are immutable statics of string literals and
// other statics.
unsafe impl Sync for wl_message {}

/// `struct wl_interface`: the description of a protocol interface.
#[repr(C)]
pub(crate) struct wl_interface {
    /// Interface name
    pub(crate) name: *const c_char,
    /// Interface version
    pub(crate) version: c_int,
    /// Number of methods (requests)
    pub(crate) method_count: c_int,
    /// Method (request) signatures
    pub(crate) methods: *const wl_message,
    /// Number of events
    pub(crate) event_count: c_int,
    /// Event signatures
    pub(crate) events: *const wl_message,
}

// SAFETY: as for wl_message.
unsafe impl Sync for wl_interface {}

/// `struct wl_array`.
#[repr(C)]
pub(crate) struct wl_array {
    /// Array size
    pub(crate) size: usize,
    /// Allocated space
    pub(crate) alloc: usize,
    /// Array data
    pub(crate) data: *mut c_void,
}

/// `wl_fixed_t`: a signed 24.8 fixed point number.
pub(crate) type wl_fixed_t = i32;

/// `union wl_argument`: one argument of a request or event.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) union wl_argument {
    /// `int`
    pub(crate) i: i32,
    /// `uint`
    pub(crate) u: u32,
    /// `fixed`
    pub(crate) f: wl_fixed_t,
    /// `string`
    pub(crate) s: *const c_char,
    /// `object`
    pub(crate) o: *mut wl_proxy,
    /// `new_id`
    pub(crate) n: u32,
    /// `array`
    pub(crate) a: *mut wl_array,
    /// `fd`
    pub(crate) h: i32,
}

/// `wl_dispatcher_func_t`.
pub(crate) type wl_dispatcher_func_t = unsafe extern "C" fn(
    user_data: *const c_void,
    target: *mut c_void,
    opcode: u32,
    msg: *const wl_message,
    args: *mut wl_argument,
) -> c_int;

/// `wl_log_func_t` (its `va_list` is passed through as a pointer).
pub(crate) type wl_log_func_t = Option<unsafe extern "C" fn(*const c_char, *mut c_void)>;

/// `WL_MARSHAL_FLAG_DESTROY`: destroy the proxy after marshalling.
pub(crate) const WL_MARSHAL_FLAG_DESTROY: u32 = 1 << 0;

/// `struct wl_list`.
#[repr(C)]
pub(crate) struct wl_list {
    pub(crate) prev: *mut wl_list,
    pub(crate) next: *mut wl_list,
}

// ---------------------------------------------------------------------------
// wayland-cursor.h
// ---------------------------------------------------------------------------

/// `struct wl_cursor_theme` (opaque).
pub(crate) enum wl_cursor_theme {}

/// `struct wl_cursor_image`.
#[repr(C)]
pub(crate) struct wl_cursor_image {
    /// Actual width
    pub(crate) width: u32,
    /// Actual height
    pub(crate) height: u32,
    /// Hot spot x (must be inside image)
    pub(crate) hotspot_x: u32,
    /// Hot spot y (must be inside image)
    pub(crate) hotspot_y: u32,
    /// Animation delay to next frame (ms)
    pub(crate) delay: u32,
}

/// `struct wl_cursor`.
#[repr(C)]
pub(crate) struct wl_cursor {
    pub(crate) image_count: u32,
    pub(crate) images: *mut *mut wl_cursor_image,
    pub(crate) name: *mut c_char,
}

// ---------------------------------------------------------------------------
// xkbcommon.h, xkbcommon-compose.h
// ---------------------------------------------------------------------------

/// `struct xkb_context` (opaque).
pub(crate) enum xkb_context {}
/// `struct xkb_keymap` (opaque).
pub(crate) enum xkb_keymap {}
/// `struct xkb_state` (opaque).
pub(crate) enum xkb_state {}
/// `struct xkb_compose_table` (opaque).
pub(crate) enum xkb_compose_table {}
/// `struct xkb_compose_state` (opaque).
pub(crate) enum xkb_compose_state {}

pub(crate) type xkb_keycode_t = u32;
pub(crate) type xkb_keysym_t = u32;
pub(crate) type xkb_layout_index_t = u32;
pub(crate) type xkb_level_index_t = u32;
pub(crate) type xkb_mod_index_t = u32;
pub(crate) type xkb_mod_mask_t = u32;

/// `xkb_keymap_key_iter_t`.
pub(crate) type xkb_keymap_key_iter_t =
    unsafe extern "C" fn(keymap: *mut xkb_keymap, key: xkb_keycode_t, data: *mut c_void);

/// `XKB_MOD_INVALID`
pub(crate) const XKB_MOD_INVALID: u32 = 0xffffffff;
/// `XKB_KEY_NoSymbol`
#[allow(non_upper_case_globals)] // (the C name)
pub(crate) const XKB_KEY_NoSymbol: xkb_keysym_t = 0;

/// `XKB_CONTEXT_NO_FLAGS` (`enum xkb_context_flags`)
pub(crate) const XKB_CONTEXT_NO_FLAGS: c_int = 0;
/// `XKB_KEYMAP_FORMAT_TEXT_V1` (`enum xkb_keymap_format`)
pub(crate) const XKB_KEYMAP_FORMAT_TEXT_V1: c_int = 1;
/// `XKB_KEYMAP_COMPILE_NO_FLAGS` (`enum xkb_keymap_compile_flags`)
pub(crate) const XKB_KEYMAP_COMPILE_NO_FLAGS: c_int = 0;
/// `XKB_COMPOSE_COMPILE_NO_FLAGS` (`enum xkb_compose_compile_flags`)
pub(crate) const XKB_COMPOSE_COMPILE_NO_FLAGS: c_int = 0;
/// `XKB_COMPOSE_STATE_NO_FLAGS` (`enum xkb_compose_state_flags`)
pub(crate) const XKB_COMPOSE_STATE_NO_FLAGS: c_int = 0;

/// `enum xkb_compose_feed_result`
pub(crate) const XKB_COMPOSE_FEED_IGNORED: c_int = 0;
pub(crate) const XKB_COMPOSE_FEED_ACCEPTED: c_int = 1;

/// `enum xkb_compose_status`
pub(crate) const XKB_COMPOSE_NOTHING: c_int = 0;
pub(crate) const XKB_COMPOSE_COMPOSING: c_int = 1;
pub(crate) const XKB_COMPOSE_COMPOSED: c_int = 2;
pub(crate) const XKB_COMPOSE_CANCELLED: c_int = 3;

// The modifier names of xkbcommon-names.h.
pub(crate) const XKB_MOD_NAME_SHIFT: &std::ffi::CStr = c"Shift";
pub(crate) const XKB_MOD_NAME_CAPS: &std::ffi::CStr = c"Lock";
pub(crate) const XKB_MOD_NAME_CTRL: &std::ffi::CStr = c"Control";
pub(crate) const XKB_MOD_NAME_ALT: &std::ffi::CStr = c"Mod1";
pub(crate) const XKB_MOD_NAME_NUM: &std::ffi::CStr = c"Mod2";
pub(crate) const XKB_MOD_NAME_LOGO: &std::ffi::CStr = c"Mod4";
pub(crate) const XKB_MOD_NAME_MOD3: &std::ffi::CStr = c"Mod3";
pub(crate) const XKB_MOD_NAME_MOD5: &std::ffi::CStr = c"Mod5";
pub(crate) const XKB_VMOD_NAME_ALT: &std::ffi::CStr = c"Alt";
pub(crate) const XKB_VMOD_NAME_LEVEL3: &std::ffi::CStr = c"LevelThree";
pub(crate) const XKB_VMOD_NAME_LEVEL5: &std::ffi::CStr = c"LevelFive";
pub(crate) const XKB_VMOD_NAME_NUM: &std::ffi::CStr = c"NumLock";
pub(crate) const XKB_VMOD_NAME_SUPER: &std::ffi::CStr = c"Super";

// ---------------------------------------------------------------------------
// libdecor.h (0.2)
// ---------------------------------------------------------------------------

/// `struct libdecor` (opaque).
pub(crate) enum libdecor {}
/// `struct libdecor_frame` (opaque).
pub(crate) enum libdecor_frame {}
/// `struct libdecor_state` (opaque).
pub(crate) enum libdecor_state {}
/// `struct libdecor_configuration` (opaque).
pub(crate) enum libdecor_configuration {}

/// `enum libdecor_error`
pub(crate) type libdecor_error = c_int;

/// `enum libdecor_window_state`
pub(crate) type libdecor_window_state = c_int;
pub(crate) const LIBDECOR_WINDOW_STATE_NONE: libdecor_window_state = 0;
pub(crate) const LIBDECOR_WINDOW_STATE_ACTIVE: libdecor_window_state = 1 << 0;
pub(crate) const LIBDECOR_WINDOW_STATE_MAXIMIZED: libdecor_window_state = 1 << 1;
pub(crate) const LIBDECOR_WINDOW_STATE_FULLSCREEN: libdecor_window_state = 1 << 2;
pub(crate) const LIBDECOR_WINDOW_STATE_TILED_LEFT: libdecor_window_state = 1 << 3;
pub(crate) const LIBDECOR_WINDOW_STATE_TILED_RIGHT: libdecor_window_state = 1 << 4;
pub(crate) const LIBDECOR_WINDOW_STATE_TILED_TOP: libdecor_window_state = 1 << 5;
pub(crate) const LIBDECOR_WINDOW_STATE_TILED_BOTTOM: libdecor_window_state = 1 << 6;
pub(crate) const LIBDECOR_WINDOW_STATE_SUSPENDED: libdecor_window_state = 1 << 7;

/// `enum libdecor_resize_edge`
pub(crate) type libdecor_resize_edge = c_int;
pub(crate) const LIBDECOR_RESIZE_EDGE_NONE: libdecor_resize_edge = 0;
pub(crate) const LIBDECOR_RESIZE_EDGE_TOP: libdecor_resize_edge = 1;
pub(crate) const LIBDECOR_RESIZE_EDGE_BOTTOM: libdecor_resize_edge = 2;
pub(crate) const LIBDECOR_RESIZE_EDGE_LEFT: libdecor_resize_edge = 3;
pub(crate) const LIBDECOR_RESIZE_EDGE_TOP_LEFT: libdecor_resize_edge = 4;
pub(crate) const LIBDECOR_RESIZE_EDGE_BOTTOM_LEFT: libdecor_resize_edge = 5;
pub(crate) const LIBDECOR_RESIZE_EDGE_RIGHT: libdecor_resize_edge = 6;
pub(crate) const LIBDECOR_RESIZE_EDGE_TOP_RIGHT: libdecor_resize_edge = 7;
pub(crate) const LIBDECOR_RESIZE_EDGE_BOTTOM_RIGHT: libdecor_resize_edge = 8;

/// `enum libdecor_capabilities`
pub(crate) type libdecor_capabilities = c_int;
pub(crate) const LIBDECOR_ACTION_MOVE: libdecor_capabilities = 1 << 0;
pub(crate) const LIBDECOR_ACTION_RESIZE: libdecor_capabilities = 1 << 1;
pub(crate) const LIBDECOR_ACTION_MINIMIZE: libdecor_capabilities = 1 << 2;
pub(crate) const LIBDECOR_ACTION_FULLSCREEN: libdecor_capabilities = 1 << 3;
pub(crate) const LIBDECOR_ACTION_CLOSE: libdecor_capabilities = 1 << 4;

/// `struct libdecor_interface`.
#[repr(C)]
pub(crate) struct libdecor_interface {
    pub(crate) error: Option<
        unsafe extern "C" fn(context: *mut libdecor, error: libdecor_error, message: *const c_char),
    >,
    pub(crate) reserved: [Option<unsafe extern "C" fn()>; 10],
}

/// `struct libdecor_frame_interface`.
#[repr(C)]
pub(crate) struct libdecor_frame_interface {
    pub(crate) configure: Option<
        unsafe extern "C" fn(
            frame: *mut libdecor_frame,
            configuration: *mut libdecor_configuration,
            user_data: *mut c_void,
        ),
    >,
    pub(crate) close:
        Option<unsafe extern "C" fn(frame: *mut libdecor_frame, user_data: *mut c_void)>,
    pub(crate) commit:
        Option<unsafe extern "C" fn(frame: *mut libdecor_frame, user_data: *mut c_void)>,
    pub(crate) dismiss_popup: Option<
        unsafe extern "C" fn(
            frame: *mut libdecor_frame,
            seat_name: *const c_char,
            user_data: *mut c_void,
        ),
    >,
    pub(crate) reserved: [Option<unsafe extern "C" fn()>; 10],
}

/// Linux input event codes (`linux/input-event-codes.h`): Wayland mouse and
/// stylus buttons are defined as these.
pub(crate) const BTN_LEFT: u32 = 0x110;
pub(crate) const BTN_RIGHT: u32 = 0x111;
pub(crate) const BTN_MIDDLE: u32 = 0x112;
pub(crate) const BTN_SIDE: u32 = 0x113;
pub(crate) const BTN_EXTRA: u32 = 0x114;
pub(crate) const BTN_STYLUS: u32 = 0x14b;
pub(crate) const BTN_STYLUS2: u32 = 0x14c;
pub(crate) const BTN_STYLUS3: u32 = 0x149;
