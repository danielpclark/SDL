// Rust translation of src/video/wayland/SDL_waylanddyn.c, SDL_waylanddyn.h
// and SDL_waylandsym.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Loading libwayland-client, libwayland-cursor, libxkbcommon and libdecor
//! at run time.
//!
//! Upstream's `SDL_waylandsym.h` lists every function SDL calls, grouped in
//! modules; `SDL_waylanddyn.c` looks each one up in all of the libraries it
//! opened and clears a module's `SDL_WAYLAND_HAVE_*` flag when a required
//! symbol is missing (`SDL_WAYLAND_SYM_OPT` symbols may be missing). Here a
//! module is a struct of function pointers that exists exactly when all of
//! its required symbols were found; libwayland-client, libwayland-cursor and
//! libxkbcommon are required, libdecor is optional (and only loaded when
//! [`hints::VIDEO_WAYLAND_ALLOW_LIBDECOR`] allows it).
//!
//! Differences from upstream's list:
//!
//! * `wl_proxy_add_dispatcher` is loaded as well: the protocol listeners are
//!   Rust closures, dispatched by one generic dispatcher per interface (see
//!   [`super::client`]) instead of tables of C callbacks.
//! * The libwayland-egl module isn't loaded, since EGL isn't translated (see
//!   the module documentation).
//! * Upstream picks some xkbcommon functions at build time from the
//!   installed version: `xkb_keymap_key_get_mods_for_level()` (1.0) and
//!   `xkb_keymap_mod_get_mask()` (1.10) are optional here, with the fallbacks
//!   upstream uses for older versions when they're missing.
//!
//! The libraries are reference counted like `wayland_load_refcount`: each
//! user holds a [`WaylandSyms`] handle, and the libraries are closed when the
//! last one goes.

#![allow(non_snake_case)] // (the names of SDL_waylandsym.h)
#![allow(dead_code)] // (the symbol table lists every function of SDL_waylandsym.h)

use std::ffi::{c_char, c_int, c_void};
use std::sync::{Arc, Mutex, Weak};

use super::sys::*;
use crate::hints;
use crate::loadso::SharedObject;

const DEBUG_DYNAMIC_WAYLAND: bool = false;

/// A library of `waylandlibs[]`: its name, and the hint (with its default)
/// that has to allow loading it.
struct WaylandDynLib {
    libname: &'static str,
    hint: Option<(&'static str, bool)>,
}

/// The libraries SDL looks symbols up in (`waylandlibs[]`), in order.
const WAYLANDLIBS: [WaylandDynLib; 4] = [
    WaylandDynLib {
        libname: "libwayland-client.so.0", // SDL_VIDEO_DRIVER_WAYLAND_DYNAMIC
        hint: None,
    },
    WaylandDynLib {
        libname: "libwayland-cursor.so.0", // SDL_VIDEO_DRIVER_WAYLAND_DYNAMIC_CURSOR
        hint: None,
    },
    WaylandDynLib {
        libname: "libxkbcommon.so.0", // SDL_VIDEO_DRIVER_WAYLAND_DYNAMIC_XKBCOMMON
        hint: None,
    },
    WaylandDynLib {
        libname: "libdecor-0.so.0", // SDL_VIDEO_DRIVER_WAYLAND_DYNAMIC_LIBDECOR
        hint: Some((hints::VIDEO_WAYLAND_ALLOW_LIBDECOR, true)),
    },
];

/// Look a symbol up in the loaded libraries. Translation of
/// `WAYLAND_GetSym()`: a missing required symbol clears `have_module`
/// ("kill this module").
fn wayland_get_sym<F: Copy>(
    libs: &[SharedObject],
    fnname: &str,
    have_module: &mut bool,
    required: bool,
) -> Option<F> {
    let mut found = None;
    for lib in libs {
        // SAFETY: F is the function pointer type declared for `fnname` in
        // the module tables below, matching the C prototype in
        // SDL_waylandsym.h; the libraries outlive the pointers (they are kept
        // in the same WaylandSyms).
        if let Ok(f) = unsafe { lib.function::<F>(fnname) } {
            found = Some((f, lib.name()));
            break;
        }
    }

    if DEBUG_DYNAMIC_WAYLAND {
        match &found {
            Some((_, libname)) => crate::log!("WAYLAND: Found '{fnname}' in {libname}"),
            None => crate::log!("WAYLAND: Symbol '{fnname}' NOT FOUND!"),
        }
    }

    if found.is_none() && required {
        *have_module = false; // kill this module.
    }

    found.map(|(f, _)| f)
}

/// Declare a module of `SDL_waylandsym.h`: a struct of the module's function
/// pointers that loads when all of its required ones are found; the
/// optional ones (`SDL_WAYLAND_SYM_OPT`) are `Option`s.
macro_rules! wayland_module {
    ($(#[$doc:meta])* $Name:ident {
        $($sym:ident: $ty:ty,)*
    } $(optional {
        $($osym:ident: $oty:ty,)*
    })?) => {
        $(#[$doc])*
        pub(crate) struct $Name {
            $(pub(crate) $sym: $ty,)*
            $($(pub(crate) $osym: Option<$oty>,)*)?
        }

        impl $Name {
            /// Look up the module's symbols (`SDL_WAYLAND_HAVE_*` stays set
            /// only if every required one is found).
            fn load(libs: &[SharedObject]) -> Option<$Name> {
                let mut have = true; // default yes
                $( let $sym = wayland_get_sym::<$ty>(libs, stringify!($sym), &mut have, true); )*
                $($( let $osym = wayland_get_sym::<$oty>(libs, stringify!($osym), &mut have, false); )*)?
                if !have {
                    return None;
                }
                Some($Name { $($sym: $sym?,)* $($($osym,)*)? })
            }
        }
    };
}

wayland_module! {
    /// `SDL_WAYLAND_MODULE(WAYLAND_CLIENT)`: libwayland-client.
    WaylandClient {
        wl_proxy_marshal: unsafe extern "C" fn(*mut wl_proxy, u32, ...),
        wl_proxy_create: unsafe extern "C" fn(*mut wl_proxy, *const wl_interface) -> *mut wl_proxy,
        wl_proxy_destroy: unsafe extern "C" fn(*mut wl_proxy),
        wl_proxy_add_listener: unsafe extern "C" fn(*mut wl_proxy, *mut Option<unsafe extern "C" fn()>, *mut c_void) -> c_int,
        wl_proxy_set_user_data: unsafe extern "C" fn(*mut wl_proxy, *mut c_void),
        wl_proxy_get_user_data: unsafe extern "C" fn(*mut wl_proxy) -> *mut c_void,
        wl_proxy_get_version: unsafe extern "C" fn(*mut wl_proxy) -> u32,
        wl_proxy_get_id: unsafe extern "C" fn(*mut wl_proxy) -> u32,
        wl_proxy_get_class: unsafe extern "C" fn(*mut wl_proxy) -> *const c_char,
        wl_proxy_set_queue: unsafe extern "C" fn(*mut wl_proxy, *mut wl_event_queue),
        wl_proxy_create_wrapper: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
        wl_proxy_wrapper_destroy: unsafe extern "C" fn(*mut c_void),
        wl_display_connect: unsafe extern "C" fn(*const c_char) -> *mut wl_display,
        wl_display_connect_to_fd: unsafe extern "C" fn(c_int) -> *mut wl_display,
        wl_display_disconnect: unsafe extern "C" fn(*mut wl_display),
        wl_display_get_fd: unsafe extern "C" fn(*mut wl_display) -> c_int,
        wl_display_dispatch: unsafe extern "C" fn(*mut wl_display) -> c_int,
        wl_display_dispatch_queue: unsafe extern "C" fn(*mut wl_display, *mut wl_event_queue) -> c_int,
        wl_display_dispatch_queue_pending: unsafe extern "C" fn(*mut wl_display, *mut wl_event_queue) -> c_int,
        wl_display_dispatch_pending: unsafe extern "C" fn(*mut wl_display) -> c_int,
        wl_display_prepare_read: unsafe extern "C" fn(*mut wl_display) -> c_int,
        wl_display_prepare_read_queue: unsafe extern "C" fn(*mut wl_display, *mut wl_event_queue) -> c_int,
        wl_display_read_events: unsafe extern "C" fn(*mut wl_display) -> c_int,
        wl_display_cancel_read: unsafe extern "C" fn(*mut wl_display),
        wl_display_get_error: unsafe extern "C" fn(*mut wl_display) -> c_int,
        wl_display_flush: unsafe extern "C" fn(*mut wl_display) -> c_int,
        wl_display_roundtrip: unsafe extern "C" fn(*mut wl_display) -> c_int,
        wl_display_create_queue: unsafe extern "C" fn(*mut wl_display) -> *mut wl_event_queue,
        wl_event_queue_destroy: unsafe extern "C" fn(*mut wl_event_queue),
        wl_log_set_handler_client: unsafe extern "C" fn(wl_log_func_t),
        wl_list_init: unsafe extern "C" fn(*mut wl_list),
        wl_list_insert: unsafe extern "C" fn(*mut wl_list, *mut wl_list),
        wl_list_remove: unsafe extern "C" fn(*mut wl_list),
        wl_list_length: unsafe extern "C" fn(*const wl_list) -> c_int,
        wl_list_empty: unsafe extern "C" fn(*const wl_list) -> c_int,
        wl_list_insert_list: unsafe extern "C" fn(*mut wl_list, *mut wl_list),
        wl_array_init: unsafe extern "C" fn(*mut wl_array),
        wl_array_release: unsafe extern "C" fn(*mut wl_array),
        wl_array_add: unsafe extern "C" fn(*mut wl_array, usize) -> *mut c_void,
        wl_array_copy: unsafe extern "C" fn(*mut wl_array, *mut wl_array) -> c_int,
        wl_proxy_marshal_constructor: unsafe extern "C" fn(*mut wl_proxy, u32, *const wl_interface, ...) -> *mut wl_proxy,
        wl_proxy_marshal_constructor_versioned: unsafe extern "C" fn(*mut wl_proxy, u32, *const wl_interface, u32, ...) -> *mut wl_proxy,
        wl_proxy_set_tag: unsafe extern "C" fn(*mut wl_proxy, *const *const c_char),
        wl_proxy_get_tag: unsafe extern "C" fn(*mut wl_proxy) -> *const *const c_char,
        wl_proxy_marshal_flags: unsafe extern "C" fn(*mut wl_proxy, u32, *const wl_interface, u32, u32, ...) -> *mut wl_proxy,
        wl_proxy_marshal_array_flags: unsafe extern "C" fn(*mut wl_proxy, u32, *const wl_interface, u32, u32, *mut wl_argument) -> *mut wl_proxy,
        // (not in upstream's list: see the module documentation)
        wl_proxy_add_dispatcher: unsafe extern "C" fn(*mut wl_proxy, wl_dispatcher_func_t, *const c_void, *mut c_void) -> c_int,
    } optional {
        wl_display_create_queue_with_name: unsafe extern "C" fn(*mut wl_display, *const c_char) -> *mut wl_event_queue,
    }
}

wayland_module! {
    /// `SDL_WAYLAND_MODULE(WAYLAND_CURSOR)`: libwayland-cursor.
    WaylandCursor {
        wl_cursor_theme_load: unsafe extern "C" fn(*const c_char, c_int, *mut wl_proxy) -> *mut wl_cursor_theme,
        wl_cursor_theme_destroy: unsafe extern "C" fn(*mut wl_cursor_theme),
        wl_cursor_theme_get_cursor: unsafe extern "C" fn(*mut wl_cursor_theme, *const c_char) -> *mut wl_cursor,
        wl_cursor_image_get_buffer: unsafe extern "C" fn(*mut wl_cursor_image) -> *mut wl_proxy,
        wl_cursor_frame: unsafe extern "C" fn(*mut wl_cursor, u32) -> c_int,
    }
}

wayland_module! {
    /// `SDL_WAYLAND_MODULE(WAYLAND_XKB)`: libxkbcommon (and its compose
    /// functions).
    WaylandXkb {
        xkb_state_key_get_syms: unsafe extern "C" fn(*mut xkb_state, xkb_keycode_t, *mut *const xkb_keysym_t) -> c_int,
        xkb_keysym_to_utf8: unsafe extern "C" fn(xkb_keysym_t, *mut c_char, usize) -> c_int,
        xkb_keymap_new_from_string: unsafe extern "C" fn(*mut xkb_context, *const c_char, c_int, c_int) -> *mut xkb_keymap,
        xkb_state_new: unsafe extern "C" fn(*mut xkb_keymap) -> *mut xkb_state,
        xkb_keymap_key_repeats: unsafe extern "C" fn(*mut xkb_keymap, xkb_keycode_t) -> c_int,
        xkb_keymap_unref: unsafe extern "C" fn(*mut xkb_keymap),
        xkb_state_unref: unsafe extern "C" fn(*mut xkb_state),
        xkb_context_unref: unsafe extern "C" fn(*mut xkb_context),
        xkb_context_new: unsafe extern "C" fn(c_int) -> *mut xkb_context,
        xkb_state_update_mask: unsafe extern "C" fn(*mut xkb_state, xkb_mod_mask_t, xkb_mod_mask_t, xkb_mod_mask_t, xkb_layout_index_t, xkb_layout_index_t, xkb_layout_index_t) -> c_int,
        xkb_compose_table_new_from_locale: unsafe extern "C" fn(*mut xkb_context, *const c_char, c_int) -> *mut xkb_compose_table,
        xkb_compose_state_reset: unsafe extern "C" fn(*mut xkb_compose_state),
        xkb_compose_table_unref: unsafe extern "C" fn(*mut xkb_compose_table),
        xkb_compose_state_new: unsafe extern "C" fn(*mut xkb_compose_table, c_int) -> *mut xkb_compose_state,
        xkb_compose_state_unref: unsafe extern "C" fn(*mut xkb_compose_state),
        xkb_compose_state_feed: unsafe extern "C" fn(*mut xkb_compose_state, xkb_keysym_t) -> c_int,
        xkb_compose_state_get_status: unsafe extern "C" fn(*mut xkb_compose_state) -> c_int,
        xkb_compose_state_get_one_sym: unsafe extern "C" fn(*mut xkb_compose_state) -> xkb_keysym_t,
        xkb_keymap_key_for_each: unsafe extern "C" fn(*mut xkb_keymap, xkb_keymap_key_iter_t, *mut c_void),
        xkb_keymap_num_layouts: unsafe extern "C" fn(*mut xkb_keymap) -> xkb_layout_index_t,
        xkb_keymap_key_get_syms_by_level: unsafe extern "C" fn(*mut xkb_keymap, xkb_keycode_t, xkb_layout_index_t, xkb_level_index_t, *mut *const xkb_keysym_t) -> c_int,
        xkb_keymap_num_levels_for_key: unsafe extern "C" fn(*mut xkb_keymap, xkb_keycode_t, xkb_layout_index_t) -> xkb_level_index_t,
        xkb_keysym_to_utf32: unsafe extern "C" fn(xkb_keysym_t) -> u32,
        xkb_keymap_mod_get_index: unsafe extern "C" fn(*mut xkb_keymap, *const c_char) -> u32,
        xkb_keymap_layout_get_name: unsafe extern "C" fn(*mut xkb_keymap, xkb_layout_index_t) -> *const c_char,
        // Only needed in the fallback replacement for xkb_keymap_key_get_mods_for_level().
        xkb_state_key_get_level: unsafe extern "C" fn(*mut xkb_state, xkb_keycode_t, xkb_layout_index_t) -> xkb_level_index_t,
    } optional {
        // (SDL_XKBCOMMON_CHECK_VERSION(1, 0, 0) and (1, 10, 0) at build time
        // upstream; optional here, see the module documentation)
        xkb_keymap_key_get_mods_for_level: unsafe extern "C" fn(*mut xkb_keymap, xkb_keycode_t, xkb_layout_index_t, xkb_level_index_t, *mut xkb_mod_mask_t, usize) -> usize,
        xkb_keymap_mod_get_mask: unsafe extern "C" fn(*mut xkb_keymap, *const c_char) -> xkb_mod_mask_t,
    }
}

wayland_module! {
    /// `SDL_WAYLAND_MODULE(WAYLAND_LIBDECOR)`: libdecor.
    Libdecor {
        libdecor_unref: unsafe extern "C" fn(*mut libdecor),
        libdecor_new: unsafe extern "C" fn(*mut wl_display, *const libdecor_interface) -> *mut libdecor,
        libdecor_decorate: unsafe extern "C" fn(*mut libdecor, *mut wl_proxy, *const libdecor_frame_interface, *mut c_void) -> *mut libdecor_frame,
        libdecor_frame_unref: unsafe extern "C" fn(*mut libdecor_frame),
        libdecor_frame_set_title: unsafe extern "C" fn(*mut libdecor_frame, *const c_char),
        libdecor_frame_set_app_id: unsafe extern "C" fn(*mut libdecor_frame, *const c_char),
        libdecor_frame_set_max_content_size: unsafe extern "C" fn(*mut libdecor_frame, c_int, c_int),
        libdecor_frame_set_min_content_size: unsafe extern "C" fn(*mut libdecor_frame, c_int, c_int),
        libdecor_frame_resize: unsafe extern "C" fn(*mut libdecor_frame, *mut wl_proxy, u32, libdecor_resize_edge),
        libdecor_frame_move: unsafe extern "C" fn(*mut libdecor_frame, *mut wl_proxy, u32),
        libdecor_frame_commit: unsafe extern "C" fn(*mut libdecor_frame, *mut libdecor_state, *mut libdecor_configuration),
        libdecor_frame_set_minimized: unsafe extern "C" fn(*mut libdecor_frame),
        libdecor_frame_set_maximized: unsafe extern "C" fn(*mut libdecor_frame),
        libdecor_frame_unset_maximized: unsafe extern "C" fn(*mut libdecor_frame),
        libdecor_frame_set_fullscreen: unsafe extern "C" fn(*mut libdecor_frame, *mut wl_proxy),
        libdecor_frame_unset_fullscreen: unsafe extern "C" fn(*mut libdecor_frame),
        libdecor_frame_set_capabilities: unsafe extern "C" fn(*mut libdecor_frame, libdecor_capabilities),
        libdecor_frame_unset_capabilities: unsafe extern "C" fn(*mut libdecor_frame, libdecor_capabilities),
        libdecor_frame_has_capability: unsafe extern "C" fn(*mut libdecor_frame, libdecor_capabilities) -> bool,
        libdecor_frame_set_visibility: unsafe extern "C" fn(*mut libdecor_frame, bool),
        libdecor_frame_is_visible: unsafe extern "C" fn(*mut libdecor_frame) -> bool,
        libdecor_frame_is_floating: unsafe extern "C" fn(*mut libdecor_frame) -> bool,
        libdecor_frame_set_parent: unsafe extern "C" fn(*mut libdecor_frame, *mut libdecor_frame),
        libdecor_frame_show_window_menu: unsafe extern "C" fn(*mut libdecor_frame, *mut wl_proxy, u32, c_int, c_int),
        libdecor_frame_get_xdg_surface: unsafe extern "C" fn(*mut libdecor_frame) -> *mut wl_proxy,
        libdecor_frame_get_xdg_toplevel: unsafe extern "C" fn(*mut libdecor_frame) -> *mut wl_proxy,
        libdecor_frame_translate_coordinate: unsafe extern "C" fn(*mut libdecor_frame, c_int, c_int, *mut c_int, *mut c_int),
        libdecor_frame_map: unsafe extern "C" fn(*mut libdecor_frame),
        libdecor_state_new: unsafe extern "C" fn(c_int, c_int) -> *mut libdecor_state,
        libdecor_state_free: unsafe extern "C" fn(*mut libdecor_state),
        libdecor_configuration_get_content_size: unsafe extern "C" fn(*mut libdecor_configuration, *mut libdecor_frame, *mut c_int, *mut c_int) -> bool,
        libdecor_configuration_get_window_state: unsafe extern "C" fn(*mut libdecor_configuration, *mut libdecor_window_state) -> bool,
        libdecor_dispatch: unsafe extern "C" fn(*mut libdecor, c_int) -> c_int,
    } optional {
        // Only found in libdecor 0.1.1 or higher, so failure to load them is not fatal.
        libdecor_frame_get_min_content_size: unsafe extern "C" fn(*const libdecor_frame, *mut c_int, *mut c_int),
        libdecor_frame_get_max_content_size: unsafe extern "C" fn(*const libdecor_frame, *mut c_int, *mut c_int),
    }
}

/// The loaded Wayland functions: the modules of `SDL_waylandsym.h`, `None`
/// when their `SDL_WAYLAND_HAVE_*` flag is 0 (only libdecor may be
/// missing).
pub(crate) struct WaylandSyms {
    /// `SDL_WAYLAND_HAVE_WAYLAND_CLIENT`
    pub(crate) client: WaylandClient,
    /// `SDL_WAYLAND_HAVE_WAYLAND_CURSOR`
    pub(crate) cursor: WaylandCursor,
    /// `SDL_WAYLAND_HAVE_WAYLAND_XKB`
    pub(crate) xkb: WaylandXkb,
    /// `SDL_WAYLAND_HAVE_WAYLAND_LIBDECOR`
    pub(crate) libdecor: Option<Libdecor>,
    /// The libraries the functions live in, unloaded on drop (after the
    /// function pointers above, which are never used past that point).
    _libs: Vec<SharedObject>,
}

impl WaylandSyms {
    /// Open the libraries and look up all the symbols; `None` if a required
    /// module isn't all there. The body of `SDL_WAYLAND_LoadSymbols()`.
    fn load() -> Option<WaylandSyms> {
        let libs: Vec<SharedObject> = WAYLANDLIBS
            .iter()
            .filter(|lib| match lib.hint {
                Some((hint, default)) => hints::get_bool(hint, default),
                None => true,
            })
            .filter_map(|lib| SharedObject::load(lib.libname).ok())
            .collect();

        let client = WaylandClient::load(&libs);
        let cursor = WaylandCursor::load(&libs);
        let xkb = WaylandXkb::load(&libs);
        let libdecor = Libdecor::load(&libs);

        // All required symbols loaded, only libdecor is optional (in case
        // something got loaded otherwise, dropping `libs` unloads it).
        let syms = WaylandSyms {
            client: client?,
            cursor: cursor?,
            xkb: xkb?,
            libdecor,
            _libs: libs,
        };
        Some(syms)
    }
}

/// The loaded symbols, shared while anyone holds them
/// (`wayland_load_refcount`).
static LOADED: Mutex<Weak<WaylandSyms>> = Mutex::new(Weak::new());

/// Load the libraries (or take another reference to them). Translation of
/// `SDL_WAYLAND_LoadSymbols()`; dropping the handle is
/// `SDL_WAYLAND_UnloadSymbols()`.
pub(crate) fn load_symbols() -> Option<Arc<WaylandSyms>> {
    let mut loaded = LOADED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(syms) = loaded.upgrade() {
        return Some(syms);
    }
    let syms = Arc::new(WaylandSyms::load()?);
    *loaded = Arc::downgrade(&syms);
    Some(syms)
}
