// Rust translation of src/video/wayland/SDL_waylandevents.c and
// SDL_waylandevents_c.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Input: the seats with their keyboards (xkbcommon keymaps, compose, key
//! repeat, modifier reconciliation), pointers (frames, buttons, axes,
//! relative motion, constraints, gestures), touch, tablet tools (pens), data
//! devices (the clipboard and drag and drop) and primary selection devices,
//! and text input; and the event loop (`Wayland_PumpEvents()`,
//! `Wayland_WaitEventTimeout()`).
//!
//! Seats are identified by the registry name of their `wl_seat`; windows by
//! their ID, surfaces by their address (only compared, never followed).
//! A listener borrows the device's data for its bookkeeping, and calls into
//! SDL with nothing borrowed, as SDL may call back into the driver.
//!
//! Text comes from the text-input protocol, or (without it) from the
//! IBus/Fcitx input methods of `core::linux::ime` (`SDL_USE_IME`), or from
//! keysyms and the compose table.

use std::ffi::{c_char, c_void, CStr};
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::client::{array_u32, AsProxy, Fixed, Obj, Proxy};
use super::datamanager::*;
use super::mouse::{CursorState, CursorStateCell};
use super::protocols::cursor_shape_v1::*;
use super::protocols::input_timestamps_unstable_v1::*;
use super::protocols::keyboard_shortcuts_inhibit_unstable_v1::*;
use super::protocols::pointer_constraints_unstable_v1::*;
use super::protocols::pointer_gestures_unstable_v1::*;
use super::protocols::relative_pointer_unstable_v1::*;
use super::protocols::tablet_v2::*;
use super::protocols::text_input_unstable_v3::*;
use super::protocols::wayland::*;
use super::protocols::wp_primary_selection_unstable_v1::*;
use super::protocols::xdg_shell::*;
use super::sys::*;
use super::video::{VideoData, WaylandVideo};
use super::window::{
    ShellSurfaceType, WAYLAND_TOPLEVEL_CONSTRAINED_BOTTOM, WAYLAND_TOPLEVEL_CONSTRAINED_LEFT,
    WAYLAND_TOPLEVEL_CONSTRAINED_RIGHT, WAYLAND_TOPLEVEL_CONSTRAINED_TOP,
};
use super::wldyn::{WaylandSyms, WaylandXkb};
use crate::core::unix::{io_ready, IoReadyFlags};
use crate::events::keyboard::{self, Keycode, Keymap, Keymod, Scancode};
use crate::events::keysym_to_keycode::get_key_code_from_key_sym;
use crate::events::keysym_to_scancode::get_scancode_from_key_sym;
use crate::events::mouse::MouseID;
use crate::events::mouse::{
    self, MouseWheelDirection, BUTTON_LEFT, BUTTON_MIDDLE, BUTTON_RIGHT, BUTTON_X1,
};
use crate::events::pen::{self, PenAxis, PenCapabilityFlags, PenID, PenInfo, PenSubtype};
use crate::events::scancode_tables::{get_scancode_from_table, ScancodeTable};
use crate::events::touch::{self, TouchDeviceType};
use crate::events::window::WindowFlags;
use crate::events::{EventType, KeyboardID, WindowID};
use crate::hints;
use crate::video::core::with_window;
use crate::video::sysvideo::HitTestResult;
use crate::video::{Point, Rect};

/// Weston uses a ratio of 10 units per scroll tick
const WAYLAND_WHEEL_AXIS_UNIT: f32 = 10.0;

// Keyboard and mouse names to match XWayland
const WAYLAND_DEFAULT_KEYBOARD_NAME: &str = "Virtual core keyboard";
const WAYLAND_DEFAULT_POINTER_NAME: &str = "Virtual core pointer";
const WAYLAND_DEFAULT_TOUCH_NAME: &str = "Virtual core touch";

/// Focus clickthrough timeout
const WAYLAND_FOCUS_CLICK_TIMEOUT_NS: u64 = 10_000_000;

// Timer rollover detection thresholds
const WAYLAND_TIMER_ROLLOVER_INTERVAL_LOW: u32 = u32::MAX / 16;
const WAYLAND_TIMER_ROLLOVER_INTERVAL_HIGH: u32 = WAYLAND_TIMER_ROLLOVER_INTERVAL_LOW * 15;

const NS_PER_SECOND: u64 = 1_000_000_000;

const fn ms_to_ns(ms: u64) -> u64 {
    ms * 1_000_000
}

/// `SDL_BUTTON_MASK()` (0 for buttons past the 32 that fit: upstream shifts
/// out of range there, see [`pointer_handle_button`](WaylandVideo::pointer_handle_button)).
fn button_mask(button: u8) -> u32 {
    if button == 0 {
        return 0;
    }
    1u32.checked_shl(button as u32 - 1).unwrap_or(0)
}

/// Translation of `enum SDL_WaylandAxisEvent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum WaylandAxisEvent {
    #[default]
    Continuous,
    Discrete,
    Value120,
}

// ---------------------------------------------------------------------------
// xkbcommon objects
// ---------------------------------------------------------------------------

macro_rules! xkb_object {
    ($(#[$doc:meta])* $Name:ident, $raw:ty, $unref:ident) => {
        $(#[$doc])*
        pub(crate) struct $Name {
            raw: std::ptr::NonNull<$raw>,
            syms: Arc<WaylandSyms>,
        }

        // SAFETY: xkbcommon objects may be used from any thread, one at a
        // time; they are only reached through the device's locked state.
        unsafe impl Send for $Name {}

        impl $Name {
            /// The raw pointer.
            pub(crate) fn raw(&self) -> *mut $raw {
                self.raw.as_ptr()
            }

            /// Take ownership of a new object (`None` for NULL).
            pub(crate) fn adopt(syms: &Arc<WaylandSyms>, raw: *mut $raw) -> Option<$Name> {
                std::ptr::NonNull::new(raw).map(|raw| $Name { raw, syms: syms.clone() })
            }
        }

        impl Drop for $Name {
            fn drop(&mut self) {
                // SAFETY: we own one reference to the object.
                unsafe { (self.syms.xkb.$unref)(self.raw()) }
            }
        }
    };
}

xkb_object!(
    /// An `xkb_context`, unreferenced on drop.
    XkbContext, xkb_context, xkb_context_unref
);
xkb_object!(
    /// An `xkb_keymap`, unreferenced on drop.
    XkbKeymap, xkb_keymap, xkb_keymap_unref
);
xkb_object!(
    /// An `xkb_state`, unreferenced on drop.
    XkbState, xkb_state, xkb_state_unref
);
xkb_object!(
    /// An `xkb_compose_table`, unreferenced on drop.
    XkbComposeTable, xkb_compose_table, xkb_compose_table_unref
);
xkb_object!(
    /// An `xkb_compose_state`, unreferenced on drop.
    XkbComposeState, xkb_compose_state, xkb_compose_state_unref
);

impl XkbContext {
    /// `xkb_context_new(0)`.
    pub(crate) fn new(syms: &Arc<WaylandSyms>) -> Option<XkbContext> {
        // SAFETY: no preconditions.
        let raw = unsafe { (syms.xkb.xkb_context_new)(XKB_CONTEXT_NO_FLAGS) };
        XkbContext::adopt(syms, raw)
    }
}

impl XkbKeymap {
    /// `xkb_keymap_new_from_string()` with the text format.
    pub(crate) fn from_string(context: &XkbContext, map: &CStr) -> Option<XkbKeymap> {
        // SAFETY: the context is alive; the string is NUL-terminated.
        let raw = unsafe {
            (context.syms.xkb.xkb_keymap_new_from_string)(
                context.raw(),
                map.as_ptr(),
                XKB_KEYMAP_FORMAT_TEXT_V1,
                XKB_KEYMAP_COMPILE_NO_FLAGS,
            )
        };
        XkbKeymap::adopt(&context.syms, raw)
    }

    /// The keycodes of the keymap, in `xkb_keymap_key_for_each()` order.
    pub(crate) fn keys(&self) -> Vec<xkb_keycode_t> {
        unsafe extern "C" fn iter(_keymap: *mut xkb_keymap, key: xkb_keycode_t, data: *mut c_void) {
            // SAFETY: data is the Vec passed below, alive for the call.
            unsafe { (*(data as *mut Vec<xkb_keycode_t>)).push(key) };
        }
        let mut keys: Vec<xkb_keycode_t> = Vec::new();
        // SAFETY: the keymap is alive; the callback only pushes to `keys`.
        unsafe {
            (self.syms.xkb.xkb_keymap_key_for_each)(
                self.raw(),
                iter,
                (&mut keys as *mut Vec<xkb_keycode_t>).cast(),
            )
        };
        keys
    }

    /// The keysyms of a key at a layout and level
    /// (`xkb_keymap_key_get_syms_by_level()`).
    pub(crate) fn syms_by_level(
        &self,
        key: xkb_keycode_t,
        layout: xkb_layout_index_t,
        level: xkb_level_index_t,
    ) -> &[xkb_keysym_t] {
        let mut syms: *const xkb_keysym_t = std::ptr::null();
        // SAFETY: the keymap is alive; xkbcommon returns an array of n
        // keysyms owned by the keymap.
        unsafe {
            let n = (self.syms.xkb.xkb_keymap_key_get_syms_by_level)(
                self.raw(),
                key,
                layout,
                level,
                &mut syms,
            );
            if n > 0 && !syms.is_null() {
                std::slice::from_raw_parts(syms, n as usize)
            } else {
                &[]
            }
        }
    }

    /// `xkb_keymap_num_levels_for_key()`
    pub(crate) fn num_levels_for_key(&self, key: xkb_keycode_t, layout: xkb_layout_index_t) -> u32 {
        // SAFETY: the keymap is alive.
        unsafe { (self.syms.xkb.xkb_keymap_num_levels_for_key)(self.raw(), key, layout) }
    }

    /// `xkb_keymap_num_layouts()`
    pub(crate) fn num_layouts(&self) -> u32 {
        // SAFETY: the keymap is alive.
        unsafe { (self.syms.xkb.xkb_keymap_num_layouts)(self.raw()) }
    }

    /// `xkb_keymap_key_repeats()`
    pub(crate) fn key_repeats(&self, key: xkb_keycode_t) -> bool {
        // SAFETY: the keymap is alive.
        unsafe { (self.syms.xkb.xkb_keymap_key_repeats)(self.raw(), key) != 0 }
    }

    /// Whether layout 0 has a name (`xkb_keymap_layout_get_name()`).
    pub(crate) fn layout_has_name(&self, layout: xkb_layout_index_t) -> bool {
        // SAFETY: the keymap is alive.
        !unsafe { (self.syms.xkb.xkb_keymap_layout_get_name)(self.raw(), layout) }.is_null()
    }

    /// The mask of a modifier: `xkb_keymap_mod_get_mask()` by its
    /// (virtual) name when the library has it (1.10), else from the index
    /// of its real modifier, as upstream's builds for older versions.
    pub(crate) fn mod_mask(&self, name: &CStr, legacy_name: &CStr) -> xkb_mod_mask_t {
        let xkb = &self.syms.xkb;
        match xkb.xkb_keymap_mod_get_mask {
            // SAFETY: the keymap is alive; the name is NUL-terminated.
            Some(f) => unsafe { f(self.raw(), name.as_ptr()) },
            None => {
                // SAFETY: as above.
                let index =
                    unsafe { (xkb.xkb_keymap_mod_get_index)(self.raw(), legacy_name.as_ptr()) };
                // FIXME (upstream): `1 << GET_MOD_INDEX(mod)` shifts by
                // XKB_MOD_INVALID when the keymap lacks the modifier
                // (undefined behaviour in C); the mask is 0 here.
                1u32.checked_shl(index).unwrap_or(0)
            }
        }
    }
}

impl XkbState {
    /// `xkb_state_new()`
    pub(crate) fn new(keymap: &XkbKeymap) -> Option<XkbState> {
        // SAFETY: the keymap is alive.
        let raw = unsafe { (keymap.syms.xkb.xkb_state_new)(keymap.raw()) };
        XkbState::adopt(&keymap.syms, raw)
    }

    /// `xkb_state_update_mask()`
    pub(crate) fn update_mask(&self, depressed: u32, latched: u32, locked: u32, layout: u32) {
        // SAFETY: the state is alive.
        unsafe {
            (self.syms.xkb.xkb_state_update_mask)(
                self.raw(),
                depressed,
                latched,
                locked,
                0,
                0,
                layout,
            )
        };
    }

    /// The keysyms of a key in the current state (`xkb_state_key_get_syms()`).
    pub(crate) fn key_get_syms(&self, key: xkb_keycode_t) -> &[xkb_keysym_t] {
        let mut syms: *const xkb_keysym_t = std::ptr::null();
        // SAFETY: the state is alive; xkbcommon returns an array of n
        // keysyms owned by the keymap.
        unsafe {
            let n = (self.syms.xkb.xkb_state_key_get_syms)(self.raw(), key, &mut syms);
            if n > 0 && !syms.is_null() {
                std::slice::from_raw_parts(syms, n as usize)
            } else {
                &[]
            }
        }
    }

    /// `xkb_state_key_get_level()`
    pub(crate) fn key_get_level(
        &self,
        key: xkb_keycode_t,
        layout: xkb_layout_index_t,
    ) -> xkb_level_index_t {
        // SAFETY: the state is alive.
        unsafe { (self.syms.xkb.xkb_state_key_get_level)(self.raw(), key, layout) }
    }
}

impl XkbComposeState {
    /// `xkb_compose_state_reset()`
    pub(crate) fn reset(&self) {
        // SAFETY: the compose state is alive.
        unsafe { (self.syms.xkb.xkb_compose_state_reset)(self.raw()) }
    }
}

/// The UTF-8 text of a keysym (`xkb_keysym_to_utf8()`), if any.
fn keysym_to_utf8(xkb: &WaylandXkb, sym: xkb_keysym_t) -> Option<String> {
    let mut text = [0 as c_char; 8];
    // SAFETY: the buffer has room for 8 bytes.
    let n = unsafe { (xkb.xkb_keysym_to_utf8)(sym, text.as_mut_ptr(), text.len()) };
    if n > 0 {
        // SAFETY: xkbcommon wrote a NUL-terminated string.
        let s = unsafe { CStr::from_ptr(text.as_ptr()) };
        Some(s.to_string_lossy().into_owned())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// The seat records
// ---------------------------------------------------------------------------

/// Translation of `SDL_WaylandKeyboardRepeat`.
#[derive(Clone, Debug, Default)]
pub(crate) struct KeyboardRepeat {
    /// Repeat rate in range of [1, 1000] character(s) per second
    pub(crate) repeat_rate: i32,
    /// Time to first repeat event in milliseconds
    pub(crate) repeat_delay_ms: i32,

    /// Key code of the repeating key
    pub(crate) key: u32,
    /// Scancode of the repeating key
    pub(crate) scancode: Scancode,
    /// ID of the source keyboard
    pub(crate) keyboard_id: KeyboardID,
    /// Key press time as reported by the Wayland API in milliseconds
    pub(crate) wl_press_time_ms: u32,
    /// Key press time as reported by the Wayland API in nanoseconds
    pub(crate) base_time_ns: u64,
    /// Key press time expressed in SDL ticks
    pub(crate) sdl_press_time_ns: u64,
    /// Next repeat event in nanoseconds
    pub(crate) next_repeat_ns: u64,
    /// (at most 7 bytes, as upstream's `char text[8]`)
    pub(crate) text: String,
}

/// A key without a scancode in the table, given a reserved one.
/// Translation of `Wayland_ReservedKey`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReservedKey {
    pub(crate) key: u32,
    pub(crate) scancode: Scancode,
}

/// The modifier masks of a keymap (`seat->keyboard.xkb.*_mask`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ModMasks {
    pub(crate) shift_mask: xkb_mod_mask_t,
    pub(crate) ctrl_mask: xkb_mod_mask_t,
    pub(crate) alt_mask: xkb_mod_mask_t,
    pub(crate) gui_mask: xkb_mod_mask_t,
    pub(crate) level3_mask: xkb_mod_mask_t,
    pub(crate) level5_mask: xkb_mod_mask_t,
    pub(crate) num_mask: xkb_mod_mask_t,
    pub(crate) caps_mask: xkb_mod_mask_t,
}

impl ModMasks {
    /// The masks of a keymap (the mask lookups of `keyboard_handle_keymap()`).
    pub(crate) fn of(keymap: &XkbKeymap) -> ModMasks {
        ModMasks {
            shift_mask: keymap.mod_mask(XKB_MOD_NAME_SHIFT, XKB_MOD_NAME_SHIFT),
            ctrl_mask: keymap.mod_mask(XKB_MOD_NAME_CTRL, XKB_MOD_NAME_CTRL),
            alt_mask: keymap.mod_mask(XKB_VMOD_NAME_ALT, XKB_MOD_NAME_ALT),
            gui_mask: keymap.mod_mask(XKB_VMOD_NAME_SUPER, XKB_MOD_NAME_LOGO),
            // Note: This is correct: Mod3 is typically level 5 shift, and Mod5 is typically level 3 shift.
            level3_mask: keymap.mod_mask(XKB_VMOD_NAME_LEVEL3, XKB_MOD_NAME_MOD5),
            level5_mask: keymap.mod_mask(XKB_VMOD_NAME_LEVEL5, XKB_MOD_NAME_MOD3),
            num_mask: keymap.mod_mask(XKB_VMOD_NAME_NUM, XKB_MOD_NAME_NUM),
            caps_mask: keymap.mod_mask(XKB_MOD_NAME_CAPS, XKB_MOD_NAME_CAPS),
        }
    }
}

/// The xkbcommon state of a keyboard (`seat->keyboard.xkb`).
#[derive(Default)]
pub(crate) struct SeatXkb {
    pub(crate) keymap: Option<XkbKeymap>,
    pub(crate) state: Option<XkbState>,
    pub(crate) compose_table: Option<XkbComposeTable>,
    pub(crate) compose_state: Option<XkbComposeState>,

    /// Current keyboard layout (aka 'group')
    pub(crate) num_layouts: xkb_layout_index_t,
    pub(crate) current_layout: xkb_layout_index_t,

    /// Modifier bitshift values
    pub(crate) masks: ModMasks,

    /// Current system modifier flags
    pub(crate) wl_pressed_modifiers: xkb_mod_mask_t,
    pub(crate) wl_latched_modifiers: xkb_mod_mask_t,
    pub(crate) wl_locked_modifiers: xkb_mod_mask_t,
}

/// `seat->keyboard`.
#[derive(Default)]
pub(crate) struct SeatKeyboard {
    pub(crate) wl_keyboard: Option<Proxy<WlKeyboard>>,
    pub(crate) timestamps: Option<Proxy<ZwpInputTimestampsV1>>,
    pub(crate) key_inhibitor: Option<Proxy<ZwpKeyboardShortcutsInhibitorV1>>,
    pub(crate) focus: Option<WindowID>,
    /// The keymaps of the layouts (`sdl_keymap`).
    pub(crate) sdl_keymap: Vec<Keymap>,
    pub(crate) reserved_scancodes: Vec<ReservedKey>,
    pub(crate) current_locale: Option<String>,

    pub(crate) repeat: KeyboardRepeat,
    pub(crate) highres_timestamp_ns: Arc<AtomicU64>,

    /// Current SDL modifier flags
    pub(crate) pressed_modifiers: Keymod,
    pub(crate) locked_modifiers: Keymod,

    pub(crate) sdl_id: KeyboardID,
    pub(crate) is_virtual: bool,

    pub(crate) xkb: SeatXkb,
}

/// The axis data of a pointer frame (`pending_frame.axis`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PendingAxis {
    pub(crate) x_axis_type: WaylandAxisEvent,
    pub(crate) x: f32,

    pub(crate) y_axis_type: WaylandAxisEvent,
    pub(crate) y: f32,

    pub(crate) direction: MouseWheelDirection,
}

/// Information about axis events on the current frame (`pending_frame`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PendingFrame {
    pub(crate) have_absolute: bool,
    pub(crate) have_relative: bool,
    pub(crate) have_warp: bool,
    pub(crate) have_axis: bool,

    pub(crate) buttons_pressed: u32,
    pub(crate) buttons_released: u32,

    pub(crate) absolute_sx: Fixed,
    pub(crate) absolute_sy: Fixed,

    pub(crate) relative_dx: Fixed,
    pub(crate) relative_dy: Fixed,
    pub(crate) relative_dx_unaccel: Fixed,
    pub(crate) relative_dy_unaccel: Fixed,

    pub(crate) axis: PendingAxis,

    pub(crate) enter_surface: usize,
    pub(crate) leave_surface: usize,

    /// Event timestamp in nanoseconds
    pub(crate) timestamp_ns: u64,
}

/// `seat->pointer`.
#[derive(Default)]
pub(crate) struct SeatPointer {
    pub(crate) wl_pointer: Option<Proxy<WlPointer>>,
    pub(crate) relative_pointer: Option<Proxy<ZwpRelativePointerV1>>,
    pub(crate) timestamps: Option<Proxy<ZwpInputTimestampsV1>>,
    pub(crate) locked_pointer: Option<Proxy<ZwpLockedPointerV1>>,
    pub(crate) confined_pointer: Option<Proxy<ZwpConfinedPointerV1>>,
    pub(crate) gesture_pinch: Option<Proxy<ZwpPointerGesturePinchV1>>,

    pub(crate) focus: Option<WindowID>,
    pub(crate) focus_surface: usize,

    /// According to the spec, a seat can only have one active gesture of any type at a time.
    pub(crate) gesture_focus: Option<WindowID>,

    pub(crate) highres_timestamp_ns: Arc<AtomicU64>,
    pub(crate) enter_serial: u32,
    pub(crate) buttons_pressed: u32,
    pub(crate) last_motion: Point,
    pub(crate) is_confined: bool,

    pub(crate) sdl_id: MouseID,

    pub(crate) pending_frame: PendingFrame,

    pub(crate) cursor_state: CursorStateCell,
}

/// Translation of `SDL_WaylandTouchPoint`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TouchPoint {
    pub(crate) id: i32,
    pub(crate) fx: Fixed,
    pub(crate) fy: Fixed,
    pub(crate) surface: usize,
}

/// `seat->touch`.
#[derive(Default)]
pub(crate) struct SeatTouch {
    pub(crate) wl_touch: Option<Proxy<WlTouch>>,
    pub(crate) timestamps: Option<Proxy<ZwpInputTimestampsV1>>,
    pub(crate) highres_timestamp_ns: Arc<AtomicU64>,
    /// The touch points, the newest first (as upstream's list).
    pub(crate) points: Vec<TouchPoint>,
}

impl SeatTouch {
    /// The touch device ID (`(SDL_TouchID)(uintptr_t)seat->touch.wl_touch`).
    fn touch_id(&self) -> u64 {
        self.wl_touch
            .as_ref()
            .map_or(0, |t| t.raw() as usize as u64)
    }
}

/// `seat->text_input`.
#[derive(Default)]
pub(crate) struct SeatTextInput {
    pub(crate) zwp_text_input: Option<Proxy<ZwpTextInputV3>>,
    pub(crate) text_input_rect: Rect,
    pub(crate) text_input_cursor: i32,
    pub(crate) enabled: bool,
    pub(crate) has_preedit: bool,
}

/// `WAYLAND_TABLET_TOOL_BUTTON_*`
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum ToolButtonState {
    #[default]
    None,
    Down,
    Up,
}

/// `WAYLAND_TABLET_TOOL_STATE_*`
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum ToolState {
    #[default]
    None,
    Down,
    Up,
}

/// The frame of a tablet tool (`SDL_WaylandPenTool.frame`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ToolFrame {
    pub(crate) x: f32,
    pub(crate) y: f32,

    pub(crate) axes: [f32; PenAxis::COUNT],
    pub(crate) axes_set: u32,

    pub(crate) buttons: [ToolButtonState; 3],

    pub(crate) tool_state: ToolState,

    pub(crate) in_proximity: bool,

    pub(crate) have_motion: bool,
    pub(crate) have_proximity: bool,
}

/// A stylus, etc, on a tablet. Translation of `SDL_WaylandPenTool`.
pub(crate) struct WaylandPenTool {
    pub(crate) instance_id: PenID,
    pub(crate) info: PenInfo,
    pub(crate) focus: Option<WindowID>,
    pub(crate) wltool: Proxy<ZwpTabletToolV2>,
    pub(crate) proximity_serial: u32,

    pub(crate) frame: ToolFrame,

    pub(crate) cursor_state: CursorStateCell,
}

impl WaylandPenTool {
    /// The tool's key (its proxy's address).
    pub(crate) fn key(&self) -> usize {
        self.wltool.raw() as usize
    }
}

/// `seat->tablet`.
#[derive(Default)]
pub(crate) struct SeatTablet {
    pub(crate) wl_tablet_seat: Option<Proxy<ZwpTabletSeatV2>>,
    /// The tools, the newest first (as upstream's list).
    pub(crate) tool_list: Vec<WaylandPenTool>,
}

/// A seat. Translation of `SDL_WaylandSeat`.
pub(crate) struct WaylandSeat {
    pub(crate) wl_seat: Proxy<WlSeat>,
    pub(crate) data_device: Option<DataDevice>,
    pub(crate) primary_selection_device: Option<PrimarySelectionDevice>,
    pub(crate) name: Option<String>,

    /// The serial of the last implicit grab event for window activation and selection data.
    pub(crate) last_implicit_grab_serial: u32,
    /// The ID of the Wayland seat object,
    pub(crate) registry_id: u32,

    pub(crate) keyboard: SeatKeyboard,
    pub(crate) pointer: SeatPointer,
    pub(crate) touch: SeatTouch,
    pub(crate) text_input: SeatTextInput,
    pub(crate) tablet: SeatTablet,
}

impl WaylandSeat {
    /// A tool of the seat by its key.
    pub(crate) fn tool(&self, key: usize) -> Option<&WaylandPenTool> {
        self.tablet.tool_list.iter().find(|t| t.key() == key)
    }

    /// A tool of the seat by its key.
    pub(crate) fn tool_mut(&mut self, key: usize) -> Option<&mut WaylandPenTool> {
        self.tablet.tool_list.iter_mut().find(|t| t.key() == key)
    }
}

// ---------------------------------------------------------------------------
// Timestamps
// ---------------------------------------------------------------------------

/// The statics of `Wayland_AdjustEventTimestampBase()` and
/// `Wayland_EventTimestampMSToNS()`.
struct TimestampState {
    base_offset: u64,
    rollover_offset: u64,
    last: u32,
}

static TIMESTAMPS: Mutex<TimestampState> = Mutex::new(TimestampState {
    base_offset: 0,
    rollover_offset: 0,
    last: 0,
});

fn timestamps() -> std::sync::MutexGuard<'static, TimestampState> {
    TIMESTAMPS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `Wayland_AdjustEventTimestampBase()`.
fn wayland_adjust_event_timestamp_base(mut ns_timestamp: u64) -> u64 {
    let now = crate::timer::ticks_ns();
    let mut t = timestamps();

    if t.base_offset == 0 {
        t.base_offset = now.wrapping_sub(ns_timestamp);
    }
    ns_timestamp = ns_timestamp.wrapping_add(t.base_offset);

    if ns_timestamp > now {
        t.base_offset = t.base_offset.wrapping_sub(ns_timestamp - now);
        ns_timestamp = now;
    }

    ns_timestamp
}

/// This should only be called with 32-bit millisecond timestamps received
/// in Wayland events! No synthetic or high-res timestamps, as they can
/// corrupt the rollover offset! Translation of `Wayland_EventTimestampMSToNS()`.
fn wayland_event_timestamp_ms_to_ns(wl_timestamp_ms: u32) -> u64 {
    let mut t = timestamps();
    let mut timestamp = ms_to_ns(wl_timestamp_ms as u64).wrapping_add(t.rollover_offset);

    if wl_timestamp_ms >= t.last {
        if t.rollover_offset != 0
            && t.last < WAYLAND_TIMER_ROLLOVER_INTERVAL_LOW
            && wl_timestamp_ms > WAYLAND_TIMER_ROLLOVER_INTERVAL_HIGH
        {
            // A time that crossed backwards across zero was received. Subtract the increased time base offset.
            timestamp = timestamp.wrapping_sub(ms_to_ns(0x1_0000_0000));
        } else {
            t.last = wl_timestamp_ms;
        }
    } else {
        /* Only increment the base time offset if the timer actually crossed forward across 0,
         * and not if this is just a timestamp from a slightly older event.
         */
        if wl_timestamp_ms < WAYLAND_TIMER_ROLLOVER_INTERVAL_LOW
            && t.last > WAYLAND_TIMER_ROLLOVER_INTERVAL_HIGH
        {
            t.rollover_offset = t.rollover_offset.wrapping_add(ms_to_ns(0x1_0000_0000));
            timestamp = timestamp.wrapping_add(ms_to_ns(0x1_0000_0000));
            t.last = wl_timestamp_ms;
        }
    }

    timestamp
}

/* Even if high-res timestamps are available, the millisecond timestamps are still processed
 * to accumulate the rollover offset if needed later.
 */

/// The timestamp of an input event: the high resolution one if the
/// device has input timestamps, else the event's (the bodies of
/// `Wayland_GetKeyboardTimestamp()`, `Wayland_GetPointerTimestamp()` and
/// `Wayland_GetTouchTimestamp()`).
fn wayland_get_input_timestamp(highres: Option<&AtomicU64>, wl_timestamp_ms: u32) -> u64 {
    let adjusted_timestamp_ns = wayland_event_timestamp_ms_to_ns(wl_timestamp_ms);
    wayland_adjust_event_timestamp_base(match highres {
        Some(h) => h.load(Ordering::Relaxed),
        None => adjusted_timestamp_ns,
    })
}

impl VideoData {
    /// Translation of `Wayland_GetKeyboardTimestamp()`.
    fn keyboard_timestamp(&self, seat: u32, wl_timestamp_ms: u32) -> u64 {
        let h = self.seat(seat).and_then(|s| {
            s.keyboard
                .timestamps
                .as_ref()
                .map(|_| s.keyboard.highres_timestamp_ns.clone())
        });
        wayland_get_input_timestamp(h.as_deref(), wl_timestamp_ms)
    }

    /// Translation of `Wayland_GetPointerTimestamp()`.
    fn pointer_timestamp(&self, seat: u32, wl_timestamp_ms: u32) -> u64 {
        let h = self.seat(seat).and_then(|s| {
            s.pointer
                .timestamps
                .as_ref()
                .map(|_| s.pointer.highres_timestamp_ns.clone())
        });
        wayland_get_input_timestamp(h.as_deref(), wl_timestamp_ms)
    }

    /// Translation of `Wayland_GetTouchTimestamp()`.
    pub(crate) fn touch_timestamp(&self, seat: u32, wl_timestamp_ms: u32) -> u64 {
        let h = self.seat(seat).and_then(|s| {
            s.touch
                .timestamps
                .as_ref()
                .map(|_| s.touch.highres_timestamp_ns.clone())
        });
        wayland_get_input_timestamp(h.as_deref(), wl_timestamp_ms)
    }
}

/// A timestamp in nanoseconds as a `Duration`.
fn ns(t: u64) -> Duration {
    Duration::from_nanos(t)
}

// ---------------------------------------------------------------------------
// Keymaps
// ---------------------------------------------------------------------------

/// Fallback for `xkb_keymap_key_get_mods_for_level()`, which is only
/// available from 1.0.0, while the SDL minimum is 0.5.0. Translation of
/// `xkb_legacy_get_mods_for_level()`.
fn xkb_legacy_get_mods_for_level(
    state: &XkbState,
    masks: &ModMasks,
    key: xkb_keycode_t,
    layout: xkb_layout_index_t,
    level: xkb_level_index_t,
    masks_out: &mut [xkb_mod_mask_t],
) -> usize {
    if masks_out.is_empty() {
        return 0;
    }

    // Level 0 is always unmodified, so early out.
    if level == 0 {
        masks_out[0] = 0;
        return 1;
    }

    let mut mask_idx = 0;
    let keymod_masks: [xkb_mod_mask_t; 12] = [
        0,
        masks.shift_mask,
        masks.caps_mask,
        masks.shift_mask | masks.caps_mask,
        masks.level3_mask,
        masks.level3_mask | masks.shift_mask,
        masks.level3_mask | masks.caps_mask,
        masks.level3_mask | masks.shift_mask | masks.caps_mask,
        masks.level5_mask,
        masks.level5_mask | masks.shift_mask,
        masks.level5_mask | masks.caps_mask,
        masks.level5_mask | masks.shift_mask | masks.caps_mask,
    ];
    let pressed_mod_mask = masks.shift_mask | masks.level3_mask | masks.level5_mask;
    let locked_mod_mask = masks.caps_mask;

    for m in keymod_masks {
        state.update_mask(m & pressed_mod_mask, 0, m & locked_mod_mask, layout);
        if state.key_get_level(key, layout) == level {
            masks_out[mask_idx] = m;

            mask_idx += 1;
            if mask_idx == masks_out.len() {
                break;
            }
        }
    }

    mask_idx
}

/// The SDL keymaps of an xkb keymap, one per layout, and the keys given
/// reserved scancodes: the body of `Wayland_KeymapIterator()` over all keys
/// (upstream runs it with `xkb_keymap_key_for_each()` on the seat).
pub(crate) fn wayland_build_keymaps(
    keymap: &XkbKeymap,
    state: &XkbState,
    masks: &ModMasks,
    num_layouts: xkb_layout_index_t,
    is_virtual: bool,
) -> (Vec<Keymap>, Vec<ReservedKey>) {
    let mut sdl_keymap: Vec<Keymap> = (0..num_layouts).map(|_| Keymap::new()).collect();
    let mut reserved_scancodes = Vec::new();

    for key in keymap.keys() {
        wayland_keymap_iterator(
            keymap,
            state,
            masks,
            num_layouts,
            is_virtual,
            &mut sdl_keymap,
            &mut reserved_scancodes,
            key,
        );
    }

    (sdl_keymap, reserved_scancodes)
}

/// Translation of `Wayland_KeymapIterator()`.
#[allow(clippy::too_many_arguments)]
fn wayland_keymap_iterator(
    keymap: &XkbKeymap,
    state: &XkbState,
    masks: &ModMasks,
    num_layouts: xkb_layout_index_t,
    is_virtual: bool,
    sdl_keymap: &mut [Keymap],
    reserved_scancodes: &mut Vec<ReservedKey>,
    key: xkb_keycode_t,
) {
    let xkb = &keymap.syms.xkb;
    let mut scancode = Scancode::UNKNOWN;

    // Only the shift, alt, level 3, level 5 and caps lock modifiers affect SDL keymaps.
    let xkb_valid_mod_mask =
        masks.shift_mask | masks.alt_mask | masks.level3_mask | masks.level5_mask | masks.caps_mask;

    // Look up the scancode for hardware keyboards. Virtual keyboards get the scancode from the keysym.
    if !is_virtual {
        scancode = get_scancode_from_table(ScancodeTable::Linux, key as i32 - 8);
    }

    for layout in 0..num_layouts {
        let num_levels = keymap.num_levels_for_key(key, layout);
        for level in 0..num_levels {
            let syms = keymap.syms_by_level(key, layout, level);
            if syms.is_empty() {
                continue;
            }
            /* If the keyboard is virtual, try to look up the scancode from the keysym. If there is still no corresponding
             * scancode, skip this mapping for now, as it will be dynamically added with a reserved scancode on first use.
             */
            if scancode == Scancode::UNKNOWN && is_virtual {
                scancode = get_scancode_from_key_sym(syms[0], key);
                if scancode == Scancode::UNKNOWN {
                    continue;
                }
            }

            let mut xkb_mod_masks = [0 as xkb_mod_mask_t; 16];
            let num_masks = match xkb.xkb_keymap_key_get_mods_for_level {
                // SAFETY: the keymap is alive; the output has room for 16
                // masks.
                Some(f) => unsafe {
                    f(
                        keymap.raw(),
                        key,
                        layout,
                        level,
                        xkb_mod_masks.as_mut_ptr(),
                        xkb_mod_masks.len(),
                    )
                },
                None => xkb_legacy_get_mods_for_level(
                    state,
                    masks,
                    key,
                    layout,
                    level,
                    &mut xkb_mod_masks,
                ),
            };
            for &mask in &xkb_mod_masks[..num_masks.min(xkb_mod_masks.len())] {
                // Ignore this modifier set if it uses unsupported modifier types.
                if (mask | xkb_valid_mod_mask) != xkb_valid_mod_mask {
                    continue;
                }

                let mut sdl_mod = Keymod::NONE;
                if mask & masks.shift_mask != 0 {
                    sdl_mod |= Keymod::SHIFT;
                }
                if mask & masks.alt_mask != 0 {
                    sdl_mod |= Keymod::ALT;
                }
                if mask & masks.level3_mask != 0 {
                    sdl_mod |= Keymod::MODE;
                }
                if mask & masks.level5_mask != 0 {
                    sdl_mod |= Keymod::LEVEL5;
                }
                if mask & masks.caps_mask != 0 {
                    sdl_mod |= Keymod::CAPS;
                }

                let mut keycode = get_key_code_from_key_sym(syms[0], key, sdl_mod);

                /* For hardware keyboards, map unknown keys with valid keycodes to the reserved scancode range.
                 * Reserved codes are always assigned from layout zero to avoid potential overlap.
                 */
                if keycode != Keycode::UNKNOWN && scancode == Scancode::UNKNOWN {
                    scancode = sdl_keymap[0].next_reserved_scancode();
                    if level != 0 {
                        // Make sure the base level always has this scancode mapped, since it is a unique key.
                        sdl_keymap[layout as usize].set_entry(
                            scancode,
                            Keymod::NONE,
                            Keycode::UNKNOWN,
                        );
                    }
                    reserved_scancodes.push(ReservedKey {
                        scancode,
                        key: key - 8,
                    });
                }

                if keycode == Keycode::UNKNOWN {
                    keycode = match scancode {
                        Scancode::RETURN => Keycode::RETURN,
                        Scancode::ESCAPE => Keycode::ESCAPE,
                        Scancode::BACKSPACE => Keycode::BACKSPACE,
                        Scancode::DELETE => Keycode::DELETE,
                        _ => Keycode::from_scancode(scancode),
                    };
                }

                sdl_keymap[layout as usize].set_entry(scancode, sdl_mod, keycode);
            }
        }
    }
}

/// Whether the bound keymap is (a binding of) `keymap`: upstream compares
/// the pointers; the keyboard core keeps a copy, which the layout detection
/// of `SDL_SetKeymap()` marks.
fn keymap_is_current(keymap: &Keymap) -> bool {
    keyboard::with_current_keymap(true, |current| {
        current.is_some_and(|c| {
            let mut c = c.clone();
            c.layout_determined = keymap.layout_determined;
            c.french_numbers = keymap.french_numbers;
            c.latin_letters = keymap.latin_letters;
            c.thai_keyboard = keymap.thai_keyboard;
            c == *keymap
        })
    })
}

/// The modifier keys among keycodes (`SDLK_LSHIFT`... in the switches of
/// `keyboard_handle_enter()` and `Wayland_HandleModifierKeys()`): their
/// modifier.
fn modifier_for_keycode(keycode: Keycode) -> Option<Keymod> {
    Some(match keycode {
        Keycode::LSHIFT => Keymod::LSHIFT,
        Keycode::RSHIFT => Keymod::RSHIFT,
        Keycode::LCTRL => Keymod::LCTRL,
        Keycode::RCTRL => Keymod::RCTRL,
        Keycode::LALT => Keymod::LALT,
        Keycode::RALT => Keymod::RALT,
        Keycode::LGUI => Keymod::LGUI,
        Keycode::RGUI => Keymod::RGUI,
        Keycode::MODE => Keymod::MODE,
        Keycode::LEVEL5_SHIFT => Keymod::LEVEL5,
        _ => return None,
    })
}

/// Translation of `Wayland_ReconcileModifiers()` on a keyboard's state: the
/// new modifier state (for `SDL_SetModState()`).
fn wayland_reconcile_modifiers(kb: &mut SeatKeyboard, key_pressed: bool) -> Keymod {
    let m = kb.xkb.masks;
    let wl_pressed = kb.xkb.wl_pressed_modifiers;

    /* Handle explicit pressed modifier state. This will correct the modifier state
     * if common modifier keys were remapped and the modifiers presumed to be set
     * during a key press event were incorrect, or if the modifier was set to the
     * pressed state via means other than pressing the physical key.
     */
    if !key_pressed {
        for (mask, sdl) in [
            (m.shift_mask, Keymod::SHIFT),
            (m.ctrl_mask, Keymod::CTRL),
            (m.alt_mask, Keymod::ALT),
            (m.gui_mask, Keymod::GUI),
            (m.level3_mask, Keymod::MODE),
            (m.level5_mask, Keymod::LEVEL5),
        ] {
            if wl_pressed & mask != 0 {
                if !kb.pressed_modifiers.intersects(sdl) {
                    kb.pressed_modifiers |= sdl;
                }
            } else {
                kb.pressed_modifiers &= !sdl;
            }
        }
    }

    /* If a latch or lock was activated by a keypress, the latch/lock will
     * be tied to the specific left/right key that initiated it. Otherwise,
     * the ambiguous left/right combo is used.
     *
     * The modifier will remain active until the latch/lock is released by
     * the system.
     */
    let xkb_locked_modifiers = kb.xkb.wl_latched_modifiers | kb.xkb.wl_locked_modifiers;

    for (mask, sdl) in [
        (m.shift_mask, Keymod::SHIFT),
        (m.ctrl_mask, Keymod::CTRL),
        (m.alt_mask, Keymod::ALT),
        (m.gui_mask, Keymod::GUI),
    ] {
        if xkb_locked_modifiers & mask != 0 {
            if kb.pressed_modifiers.intersects(sdl) {
                kb.locked_modifiers &= !sdl;
                kb.locked_modifiers |= kb.pressed_modifiers & sdl;
            } else if !kb.locked_modifiers.intersects(sdl) {
                kb.locked_modifiers |= sdl;
            }
        } else {
            kb.locked_modifiers &= !sdl;
        }
    }

    for (mask, sdl) in [
        (m.level3_mask, Keymod::MODE),
        (m.level5_mask, Keymod::LEVEL5),
        // Capslock and Numlock can only be locked, not pressed.
        (m.caps_mask, Keymod::CAPS),
        (m.num_mask, Keymod::NUM),
    ] {
        if xkb_locked_modifiers & mask != 0 {
            kb.locked_modifiers |= sdl;
        } else {
            kb.locked_modifiers &= !sdl;
        }
    }

    kb.pressed_modifiers | kb.locked_modifiers
}

/// The part of `keyboard_input_get_text()` after `SDL_IME_ProcessKeyEvent()`
/// (see [`WaylandVideo::keyboard_input_get_text`]): the text of a key
/// (`None` if it has none), and whether the composition took it.
fn keyboard_input_get_text(seat: &WaylandSeat, key: u32, down: bool) -> (Option<String>, bool) {
    let mut handled_by_ime = false;
    let Some(state) = seat.keyboard.xkb.state.as_ref() else {
        return (None, false);
    };
    if seat.keyboard.focus.is_none() {
        return (None, false);
    }

    // TODO: Can this happen?
    let syms = state.key_get_syms(key + 8);
    if syms.len() != 1 {
        return (None, false);
    }
    let mut sym = syms[0];

    if !down {
        return (None, false);
    }

    let xkb = &state.syms.xkb;
    if let Some(compose_state) = &seat.keyboard.xkb.compose_state {
        // SAFETY: the compose state is alive.
        if unsafe { (xkb.xkb_compose_state_feed)(compose_state.raw(), sym) }
            == XKB_COMPOSE_FEED_ACCEPTED
        {
            // SAFETY: as above.
            match unsafe { (xkb.xkb_compose_state_get_status)(compose_state.raw()) } {
                XKB_COMPOSE_COMPOSING => {
                    handled_by_ime = true;
                    return (Some(String::new()), handled_by_ime);
                }
                XKB_COMPOSE_NOTHING => {}
                XKB_COMPOSE_COMPOSED => {
                    // SAFETY: as above.
                    sym = unsafe { (xkb.xkb_compose_state_get_one_sym)(compose_state.raw()) };
                }
                // XKB_COMPOSE_CANCELLED and anything else
                _ => sym = XKB_KEY_NoSymbol,
            }
        }
    }

    (keysym_to_utf8(xkb, sym), handled_by_ime)
}

/// The scancode of a key, with the first keysym of a virtual keyboard's
/// key. Translation of `Wayland_GetScancodeForKey()`.
///
/// Virtual keyboards can have arbitrary layouts, arbitrary
/// scancodes/keycodes, etc... Key presses from these devices must be looked
/// up by their keysym value.
fn wayland_get_scancode_for_key(seat: &WaylandSeat, key: u32) -> (Scancode, Option<xkb_keysym_t>) {
    let mut scancode = Scancode::UNKNOWN;
    let mut first_sym = None;

    if !seat.keyboard.is_virtual {
        scancode = get_scancode_from_table(ScancodeTable::Linux, key as i32);

        // No table entry? Check the reserved list.
        if scancode == Scancode::UNKNOWN {
            if let Some(i) = seat
                .keyboard
                .reserved_scancodes
                .iter()
                .find(|i| i.key == key)
            {
                return (i.scancode, None);
            }
        }
    } else if let Some(state) = &seat.keyboard.xkb.state {
        let keysym = state.key_get_syms(key + 8);
        if let Some(&sym) = keysym.first() {
            scancode = get_scancode_from_key_sym(sym, key + 8);
            first_sym = Some(sym);
        }
    }

    (scancode, first_sym)
}

// ---------------------------------------------------------------------------
// The listeners
// ---------------------------------------------------------------------------

/// What a pointer leave of `pointer_dispatch_leave()` hands to SDL.
struct PointerLeave {
    window: WindowID,
    buttons: Vec<u8>,
    sdl_id: MouseID,
    pointer_focus_count: i32,
    active_touch_count: i32,
}

impl WaylandVideo {
    /// Translation of `Wayland_SeatAddTouch()`.
    fn wayland_seat_add_touch(
        d: &mut VideoData,
        seat: u32,
        id: i32,
        fx: Fixed,
        fy: Fixed,
        surface: usize,
    ) {
        if let Some(s) = d.seat_mut(seat) {
            s.touch.points.insert(
                0,
                TouchPoint {
                    id,
                    fx,
                    fy,
                    surface,
                },
            );
        }
    }

    /// Translation of `Wayland_SeatCancelTouch()` (the point is already out
    /// of the list).
    fn wayland_seat_cancel_touch(&self, seat: u32, tp: TouchPoint) {
        if tp.surface == 0 {
            return;
        }
        let Some((window, x, y, touch_id)) = self.with_data(|d| {
            let window = d.window_for_surface(tp.surface)?;
            let touch_id = d.seat(seat).map_or(0, |s| s.touch.touch_id());
            let wd = d.window_mut(window)?;
            let x = (tp.fx.to_f64() / wd.current.logical_width as f64) as f32;
            let y = (tp.fy.to_f64() / wd.current.logical_height as f64) as f32;
            wd.active_touch_count -= 1;
            Some((window, x, y, touch_id))
        }) else {
            return;
        };

        touch::send_touch(
            Duration::ZERO,
            touch_id,
            (tp.id + 1) as u64,
            Some(window),
            EventType::FINGER_CANCELED,
            x,
            y,
            0.0,
        );

        self.maybe_lose_mouse_focus(window);
    }

    /// If the window currently has mouse focus and has no currently active
    /// keyboards, pointers, or touch events, then consider mouse focus to be
    /// lost (the end of `Wayland_SeatCancelTouch()` and `touch_handler_up()`).
    fn maybe_lose_mouse_focus(&self, window: WindowID) {
        let counts = self.with_data(|d| {
            d.window(window).map(|w| {
                (
                    w.keyboard_focus_count,
                    w.pointer_focus_count,
                    w.active_touch_count,
                )
            })
        });
        if let Some((k, p, t)) = counts {
            if mouse::mouse_focus() == Some(window) && k == 0 && p == 0 && t == 0 {
                mouse::set_mouse_focus(None);
            }
        }
    }

    /// Translation of `Wayland_SeatUpdateTouch()`: the touch point's
    /// surface.
    fn wayland_seat_update_touch(
        d: &mut VideoData,
        seat: u32,
        id: i32,
        fx: Fixed,
        fy: Fixed,
    ) -> usize {
        let Some(s) = d.seat_mut(seat) else {
            return 0;
        };
        match s.touch.points.iter_mut().find(|tp| tp.id == id) {
            Some(tp) => {
                tp.fx = fx;
                tp.fy = fy;
                tp.surface
            }
            None => 0,
        }
    }

    /// Translation of `Wayland_SeatRemoveTouch()`.
    fn wayland_seat_remove_touch(d: &mut VideoData, seat: u32, id: i32) -> Option<TouchPoint> {
        let s = d.seat_mut(seat)?;
        let i = s.touch.points.iter().position(|tp| tp.id == id)?;
        Some(s.touch.points.remove(i))
    }

    /// Translation of `Wayland_GetScaledMouseRect()`.
    fn wayland_get_scaled_mouse_rect(mouse_rect: Rect, pointer_scale: (f64, f64)) -> Rect {
        Rect {
            x: (mouse_rect.x as f64 / pointer_scale.0).floor() as i32,
            y: (mouse_rect.y as f64 / pointer_scale.1).floor() as i32,
            w: (mouse_rect.w as f64 / pointer_scale.0).ceil() as i32,
            h: (mouse_rect.h as f64 / pointer_scale.1).ceil() as i32,
        }
    }

    /// Translation of `Wayland_SeatRegisterInputTimestampListeners()`.
    fn wayland_seat_register_input_timestamp_listeners(&self, d: &mut VideoData, seat: u32) {
        let Some(manager) = d.g.input_timestamps_manager.as_ref() else {
            return;
        };
        let Some(s) = d.seat_list.iter_mut().find(|s| s.registry_id == seat) else {
            return;
        };

        fn listen(ts: &mut Proxy<ZwpInputTimestampsV1>, target: Arc<AtomicU64>) {
            ts.listen(move |_, ev| {
                // Translation of `input_timestamp_listener()`.
                let ZwpInputTimestampsV1Event::Timestamp {
                    tv_sec_hi,
                    tv_sec_lo,
                    tv_nsec,
                } = ev;
                let v = ((((tv_sec_hi as u64) << 32) | tv_sec_lo as u64) * NS_PER_SECOND)
                    + tv_nsec as u64;
                target.store(v, Ordering::Relaxed);
            });
        }

        if let (Some(kb), None) = (&s.keyboard.wl_keyboard, &s.keyboard.timestamps) {
            let mut ts = manager.get_keyboard_timestamps(kb.obj());
            listen(&mut ts, s.keyboard.highres_timestamp_ns.clone());
            s.keyboard.timestamps = Some(ts);
        }

        if let (Some(p), None) = (&s.pointer.wl_pointer, &s.pointer.timestamps) {
            let mut ts = manager.get_pointer_timestamps(p.obj());
            listen(&mut ts, s.pointer.highres_timestamp_ns.clone());
            s.pointer.timestamps = Some(ts);
        }

        if let (Some(t), None) = (&s.touch.wl_touch, &s.touch.timestamps) {
            let mut ts = manager.get_touch_timestamps(t.obj());
            listen(&mut ts, s.touch.highres_timestamp_ns.clone());
            s.touch.timestamps = Some(ts);
        }
    }

    /// Translation of `Wayland_DisplayInitInputTimestampManager()`.
    pub(crate) fn wayland_display_init_input_timestamp_manager(&self, d: &mut VideoData) {
        if d.g.input_timestamps_manager.is_some() {
            let seats: Vec<u32> = d.seat_list.iter().map(|s| s.registry_id).collect();
            for seat in seats {
                self.wayland_seat_register_input_timestamp_listeners(d, seat);
            }
        }
    }

    /// The pinch gesture listener (`gesture_pinch_listener`).
    fn handle_gesture_pinch_event(&self, seat: u32, event: ZwpPointerGesturePinchV1Event<'_>) {
        match event {
            ZwpPointerGesturePinchV1Event::Begin { time, surface, .. } => {
                // Translation of `handle_pinch_begin()`.
                let Some(surface) = surface else {
                    return;
                };
                let Some((wind, timestamp)) = self.with_data(|d| {
                    let wind = d.window_for_surface(surface.raw() as usize)?;
                    d.seat_mut(seat)?.pointer.gesture_focus = Some(wind);
                    Some((wind, d.pointer_timestamp(seat, time)))
                }) else {
                    return;
                };
                touch::send_pinch(
                    EventType::PINCH_BEGIN,
                    ns(timestamp),
                    wind,
                    0.0,
                    -1.0,
                    -1.0,
                    -1.0,
                    -1.0,
                );
            }
            ZwpPointerGesturePinchV1Event::Update { time, scale, .. } => {
                // Translation of `handle_pinch_update()`.
                let Some((focus, timestamp)) = self.with_data(|d| {
                    let focus = d.seat(seat)?.pointer.gesture_focus?;
                    Some((focus, d.pointer_timestamp(seat, time)))
                }) else {
                    return;
                };
                let s = scale.to_f64() as f32;
                touch::send_pinch(
                    EventType::PINCH_UPDATE,
                    ns(timestamp),
                    focus,
                    s,
                    -1.0,
                    -1.0,
                    -1.0,
                    -1.0,
                );
            }
            ZwpPointerGesturePinchV1Event::End { time, .. } => {
                // Translation of `handle_pinch_end()`.
                let Some((focus, timestamp)) = self.with_data(|d| {
                    let focus = d.seat(seat)?.pointer.gesture_focus?;
                    Some((focus, d.pointer_timestamp(seat, time)))
                }) else {
                    return;
                };
                touch::send_pinch(
                    EventType::PINCH_END,
                    ns(timestamp),
                    focus,
                    0.0,
                    -1.0,
                    -1.0,
                    -1.0,
                    -1.0,
                );

                self.with_data(|d| {
                    if let Some(s) = d.seat_mut(seat) {
                        s.pointer.gesture_focus = None;
                    }
                });
            }
        }
    }

    /// Translation of `Wayland_SeatCreatePointerGestures()`.
    fn wayland_seat_create_pointer_gestures(&self, d: &mut VideoData, seat: u32) {
        let Some(gestures) = d.g.zwp_pointer_gestures.as_ref() else {
            return;
        };
        let Some(s) = d.seat_list.iter_mut().find(|s| s.registry_id == seat) else {
            return;
        };
        if let (Some(p), None) = (&s.pointer.wl_pointer, &s.pointer.gesture_pinch) {
            let mut pinch = gestures.get_pinch_gesture(p.obj());
            self.listen(&mut pinch, move |v, _, ev| {
                v.handle_gesture_pinch_event(seat, ev)
            });
            s.pointer.gesture_pinch = Some(pinch);
        }
    }

    /// Translation of `Wayland_DisplayInitPointerGestureManager()`.
    pub(crate) fn wayland_display_init_pointer_gesture_manager(&self, d: &mut VideoData) {
        let seats: Vec<u32> = d.seat_list.iter().map(|s| s.registry_id).collect();
        for seat in seats {
            self.wayland_seat_create_pointer_gestures(d, seat);
        }
    }

    /// Translation of `Wayland_SeatCreateCursorShape()`.
    fn wayland_seat_create_cursor_shape(d: &mut VideoData, seat: u32) {
        let Some(manager) = d.g.cursor_shape_manager.as_ref() else {
            return;
        };
        let Some(s) = d.seat_list.iter_mut().find(|s| s.registry_id == seat) else {
            return;
        };
        if let Some(p) = &s.pointer.wl_pointer {
            let mut state = super::mouse::lock_state(&s.pointer.cursor_state);
            if state.cursor_shape.is_none() {
                state.cursor_shape = Some(manager.get_pointer(p.obj()));
            }
        }

        for tool in s.tablet.tool_list.iter_mut() {
            let mut state = super::mouse::lock_state(&tool.cursor_state);
            if state.cursor_shape.is_none() {
                state.cursor_shape = Some(manager.get_tablet_tool_v2(tool.wltool.obj()));
            }
        }
    }

    /// Translation of `Wayland_DisplayInitCursorShapeManager()`.
    pub(crate) fn wayland_display_init_cursor_shape_manager(&self, d: &mut VideoData) {
        let seats: Vec<u32> = d.seat_list.iter().map(|s| s.registry_id).collect();
        for seat in seats {
            Self::wayland_seat_create_cursor_shape(d, seat);
        }
    }

    /// Bind the seat's keymap of its current layout, if it isn't bound.
    /// Translation of `Wayland_SeatSetKeymap()`.
    fn wayland_seat_set_keymap(&self, seat: u32) {
        let Some((keymap, modstate, send_event)) = self.with_data(|d| {
            let send_event = !d.initializing;
            let s = d.seat(seat)?;
            let kb = &s.keyboard;
            if kb.sdl_keymap.is_empty() || kb.xkb.current_layout >= kb.xkb.num_layouts {
                return None;
            }
            let keymap = kb.sdl_keymap.get(kb.xkb.current_layout as usize)?;
            if keymap_is_current(keymap) {
                return None;
            }
            Some((
                keymap.clone(),
                kb.pressed_modifiers | kb.locked_modifiers,
                send_event,
            ))
        }) else {
            return;
        };

        keyboard::set_keymap(Some(keymap), send_event);
        keyboard::set_mod_state(modstate);
    }

    /// Synthesize key repeat events. Translation of `keyboard_repeat_handle()`.
    fn keyboard_repeat_handle(&self, seat: u32, elapsed: u64) {
        let modstate = keyboard::mod_state();
        let Some(events) = self.with_data(|d| {
            let repeat_info = &mut d.seat_mut(seat)?.keyboard.repeat;
            let mut events = Vec::new();
            if repeat_info.repeat_rate <= 0 {
                return Some(events);
            }
            while elapsed >= repeat_info.next_repeat_ns {
                let key = (repeat_info.scancode != Scancode::UNKNOWN).then(|| {
                    let timestamp = repeat_info.base_time_ns + repeat_info.next_repeat_ns;
                    (
                        timestamp,
                        repeat_info.keyboard_id,
                        repeat_info.key,
                        repeat_info.scancode,
                    )
                });
                let text = (!repeat_info.text.is_empty()
                    && !modstate.intersects(Keymod::CTRL | Keymod::ALT))
                .then(|| repeat_info.text.clone());
                events.push((key, text));
                repeat_info.next_repeat_ns += NS_PER_SECOND / repeat_info.repeat_rate as u64;
            }
            Some(events)
        }) else {
            return;
        };

        for (key, text) in events {
            if let Some((timestamp, keyboard_id, key, scancode)) = key {
                keyboard::send_keyboard_key_ignore_modifiers(
                    ns(wayland_adjust_event_timestamp_base(timestamp)),
                    keyboard_id,
                    key as i32,
                    scancode,
                    true,
                );
            }
            if let Some(text) = text {
                keyboard::send_keyboard_text(&text);
            }
        }
    }

    /// Translation of `keyboard_repeat_set()`.
    fn keyboard_repeat_set(
        repeat_info: &mut KeyboardRepeat,
        keyboard_id: KeyboardID,
        key: u32,
        wl_press_time_ms: u32,
        base_time_ns: u64,
        scancode: Scancode,
        text: Option<&str>,
    ) {
        if repeat_info.repeat_rate == 0 {
            return;
        }
        repeat_info.keyboard_id = keyboard_id;
        repeat_info.key = key;
        repeat_info.wl_press_time_ms = wl_press_time_ms;
        repeat_info.base_time_ns = base_time_ns;
        repeat_info.sdl_press_time_ns = crate::timer::ticks_ns();
        repeat_info.next_repeat_ns = ms_to_ns(repeat_info.repeat_delay_ms.max(0) as u64);
        repeat_info.scancode = scancode;
        repeat_info.text.clear();
        if let Some(text) = text {
            repeat_info.text = truncate_utf8(text, 7).to_owned();
        }
    }

    /// Queue a sync event to unblock the main event queue if it's being
    /// waited on. Translation of `Wayland_SendWakeupEvent()`.
    pub(crate) fn wayland_send_wakeup_event(&self, _window: WindowID) {
        let cb = self.conn.obj().sync();
        // (sync_done_handler(): nothing to do, just destroy the callback)
        self.callbacks.add(cb, |_| {});
        let _ = self.conn.flush();
    }

    /// Translation of `keyboard_input_get_text()`: the text of a key of a
    /// seat (`None` if it has none), and whether the input method or the
    /// composition took it. The D-Bus input method is asked with nothing
    /// borrowed.
    fn keyboard_input_get_text(&self, seat: u32, key: u32, down: bool) -> (Option<String>, bool) {
        let sym = self.with_data(|d| {
            let s = d.seat(seat)?;
            s.keyboard.focus?;
            let syms = s.keyboard.xkb.state.as_ref()?.key_get_syms(key + 8);
            // TODO: Can this happen?
            (syms.len() == 1).then(|| syms[0])
        });
        let Some(sym) = sym else {
            return (None, false);
        };

        if crate::core::linux::ime::process_key_event(sym, key + 8, down) {
            return (Some(String::new()), true);
        }

        self.with_data(|d| d.seat(seat).map(|s| keyboard_input_get_text(s, key, down)))
            .unwrap_or((None, false))
    }

    /// Translation of `Wayland_WaitEventTimeout()`.
    pub(crate) fn wayland_wait_event_timeout(&self, mut timeout_ns: i64) -> i32 {
        let d = &self.conn;
        let mut start = crate::timer::ticks_ns();
        let display_fd = d.fd();
        let mut poll_alarm_set = false;

        let keyboard_focus = keyboard::keyboard_focus();
        if self.with_data(|d| d.g.text_input_manager.is_none())
            && keyboard_focus.is_some_and(keyboard::text_input_active)
        {
            // If a DBus IME is active with no text input protocol, periodically wake to poll it.
            if !(0..200_000_000).contains(&timeout_ns) {
                timeout_ns = 200_000_000;
                poll_alarm_set = true;
            }
        }

        // If key repeat is active, we'll need to cap our maximum wait time to handle repeats
        let repeats: Vec<(u64, u64)> = self.with_data(|data| {
            data.seat_list
                .iter()
                .filter(|s| s.keyboard.repeat.key != 0)
                .map(|s| {
                    (
                        s.keyboard.repeat.sdl_press_time_ns,
                        s.keyboard.repeat.next_repeat_ns,
                    )
                })
                .collect()
        });
        for (sdl_press_time_ns, next_repeat_ns) in repeats {
            let elapsed = start.wrapping_sub(sdl_press_time_ns);
            let next_repeat_wait_time = next_repeat_ns.wrapping_sub(elapsed).wrapping_add(1);
            if timeout_ns < 0 || next_repeat_wait_time <= timeout_ns as u64 {
                timeout_ns = next_repeat_wait_time as i64;
                poll_alarm_set = true;
            }
        }

        if d.prepare_read() {
            if timeout_ns > 0 {
                let now = crate::timer::ticks_ns();
                let elapsed = now - start;
                start = now;
                timeout_ns = if elapsed <= timeout_ns as u64 {
                    timeout_ns - elapsed as i64
                } else {
                    0
                };
            }

            let mut ret = match d.flush() {
                Ok(n) => n,
                Err(e) => {
                    set_errno(e);
                    -1
                }
            };

            if ret == -1 && errno() == libc::EAGAIN {
                // Unable to write to the socket; poll until the socket can be written to, it times out, or is interrupted.
                ret = io_ready(
                    display_fd,
                    IoReadyFlags::WRITE | IoReadyFlags::NO_RETRY,
                    timeout_ns,
                );

                if ret <= 0 {
                    // The poll operation timed out or experienced an error, so see if there are any events to read without waiting.
                    timeout_ns = 0;
                }
            }

            if ret < 0 {
                // Pump events on an interrupt or broken pipe to handle the error.
                let e = errno();
                d.cancel_read();
                return if e == libc::EINTR || e == libc::EPIPE {
                    1
                } else {
                    ret
                };
            }

            if timeout_ns > 0 {
                let now = crate::timer::ticks_ns();
                let elapsed = now - start;
                timeout_ns = if elapsed <= timeout_ns as u64 {
                    timeout_ns - elapsed as i64
                } else {
                    0
                };
            }

            // Use SDL_IOR_NO_RETRY to catch EINTR.
            let ret = io_ready(
                display_fd,
                IoReadyFlags::READ | IoReadyFlags::NO_RETRY,
                timeout_ns,
            );
            if ret <= 0 {
                let e = errno();
                // Timeout or error, cancel the read.
                d.cancel_read();

                // The poll timed out with no data to read, but signal the caller to pump events if polling is required.
                if ret == 0 {
                    return if poll_alarm_set { 1 } else { 0 };
                } else {
                    // Pump events on an interrupt or broken pipe to handle the error.
                    return if e == libc::EINTR || e == libc::EPIPE {
                        1
                    } else {
                        ret
                    };
                }
            }

            if d.read_events().is_err() {
                return -1;
            }
        }

        // Signal to the caller that there might be an event available.
        1
    }

    /// Translation of `Wayland_PumpEvents()`.
    pub(crate) fn wayland_pump_events(&self) {
        let d = &self.conn;
        let display_fd = d.fd();

        let keyboard_focus = keyboard::keyboard_focus();
        if self.with_data(|data| data.g.text_input_manager.is_none())
            && keyboard_focus.is_some_and(keyboard::text_input_active)
        {
            crate::core::linux::ime::pump_events();
        }

        let libdecor = self.with_data(|data| data.libdecor.as_ref().map(|l| l.raw));
        if let (Some(libdecor), Some(l)) = (libdecor, &self.syms().libdecor) {
            // SAFETY: the context is alive (only video_quit frees it, on this
            // thread); listeners may run.
            unsafe { (l.libdecor_dispatch)(libdecor.as_ptr(), 0) };
            super::client::resume_pending_panic();
        }

        /* If the queue isn't empty, dispatch any old events, and try to prepare for reading again.
         * If preparing to read returns -1 on the second try, wl_display_read_events() enqueued new
         * events at some point between dispatching the old events and preparing for the read,
         * probably from another thread, which means that the events in the queue are current.
         */
        let mut ret: i32;
        let mut prepared = d.prepare_read();
        let mut connection_error = false;
        if !prepared {
            match d.dispatch_pending() {
                Ok(_) => {}
                Err(_) => connection_error = true,
            }
            if !connection_error {
                prepared = d.prepare_read();
            }
        }

        if connection_error {
            ret = -1;
        } else if prepared {
            ret = match d.flush() {
                Ok(n) => n,
                Err(e) => {
                    set_errno(e);
                    -1
                }
            };

            if ret == -1 && errno() == libc::EAGAIN {
                // Unable to write to the socket; wait a brief time to see if it becomes writable.
                ret = io_ready(display_fd, IoReadyFlags::WRITE, 4_000_000);
                if ret > 0 {
                    ret = match d.flush() {
                        Ok(n) => n,
                        Err(e) => {
                            set_errno(e);
                            -1
                        }
                    };
                }
            }

            // If the compositor closed the socket, just jump to the error handler.
            if ret < 0 && errno() == libc::EPIPE {
                d.cancel_read();
                self.wayland_handle_display_disconnected();
                return;
            }

            ret = io_ready(display_fd, IoReadyFlags::READ, 0);
            if ret > 0 {
                ret = match d.read_events() {
                    Ok(_) => d.dispatch_pending().unwrap_or(-1),
                    Err(_) => -1,
                };
            } else {
                d.cancel_read();
            }
        } else {
            ret = d.dispatch_pending().unwrap_or(-1);
        }

        if ret >= 0 {
            // Synthesize key repeat events.
            let seats: Vec<u32> = self.with_data(|data| {
                data.seat_list
                    .iter()
                    .filter(|s| s.keyboard.repeat.key != 0)
                    .map(|s| s.registry_id)
                    .collect()
            });
            for seat in seats {
                self.wayland_seat_set_keymap(seat);

                let press_time = self
                    .with_data(|data| data.seat(seat).map(|s| s.keyboard.repeat.sdl_press_time_ns));
                if let Some(press_time) = press_time {
                    let elapsed = crate::timer::ticks_ns().wrapping_sub(press_time);
                    self.keyboard_repeat_handle(seat, elapsed);
                }
            }
        }

        // connection_error:
        if ret < 0 {
            self.wayland_handle_display_disconnected();
        }
    }

    /// Translation of `pointer_dispatch_absolute_motion()`.
    fn pointer_dispatch_absolute_motion(&self, seat: u32, warp: bool) {
        let Some((window, sx, sy, timestamp, sdl_id, last_motion, toplevel_constraints)) = self
            .with_data(|d| {
                let s = d.seat(seat)?;
                let window = s.pointer.focus?;
                let pf = s.pointer.pending_frame;
                let focus_surface = s.pointer.focus_surface;
                let sdl_id = s.pointer.sdl_id;
                let wd = d.window(window)?;

                let mut sx = pf.absolute_sx.to_f64();
                let mut sy = pf.absolute_sy.to_f64();

                if wd
                    .mask
                    .surface
                    .as_ref()
                    .is_some_and(|m| m.raw() as usize == focus_surface)
                {
                    sx += wd.mask.offset_x as f64;
                    sy += wd.mask.offset_y as f64;
                }

                sx *= wd.pointer_scale.x;
                sy *= wd.pointer_scale.y;
                let constraints = wd.toplevel_constraints;

                let last_motion = Point {
                    x: sx.floor() as i32,
                    y: sy.floor() as i32,
                };
                d.seat_mut(seat)?.pointer.last_motion = last_motion;
                Some((
                    window,
                    sx,
                    sy,
                    pf.timestamp_ns,
                    sdl_id,
                    last_motion,
                    constraints,
                ))
            })
        else {
            return;
        };

        if !warp {
            mouse::send_mouse_motion(
                ns(timestamp),
                Some(window),
                sdl_id,
                false,
                sx as f32,
                sy as f32,
            );
        } else {
            mouse::send_mouse_warp(ns(timestamp), Some(window), sdl_id, sx as f32, sy as f32);
        }

        // If the pointer should be confined, but wasn't for some reason, keep trying until it is.
        let Ok((mouse_rect, flags, hit_test)) = with_window(window, |w| {
            (w.core.mouse_rect, w.flags(), w.hit_test.clone())
        }) else {
            return;
        };
        let is_confined = self.with_data(|d| d.seat(seat).is_some_and(|s| s.pointer.is_confined));
        if !mouse_rect.is_empty() && !is_confined {
            self.wayland_seat_update_pointer_grab(seat);
        }

        // Don't perform hit testing if an implicit grab is active.
        if !flags.contains(WindowFlags::MOUSE_CAPTURE) {
            if let Some(hit_test) = hit_test {
                let mut rc = hit_test(window, last_motion);

                // Apply the toplevel constraints if the window isn't resizable from those directions.
                let c = toplevel_constraints;
                let top = c & WAYLAND_TOPLEVEL_CONSTRAINED_TOP != 0;
                let bottom = c & WAYLAND_TOPLEVEL_CONSTRAINED_BOTTOM != 0;
                let left = c & WAYLAND_TOPLEVEL_CONSTRAINED_LEFT != 0;
                let right = c & WAYLAND_TOPLEVEL_CONSTRAINED_RIGHT != 0;
                rc = match rc {
                    HitTestResult::ResizeTopLeft => {
                        if top && left {
                            HitTestResult::Normal
                        } else if top {
                            HitTestResult::ResizeLeft
                        } else if left {
                            HitTestResult::ResizeTop
                        } else {
                            rc
                        }
                    }
                    HitTestResult::ResizeTop if top => HitTestResult::Normal,
                    HitTestResult::ResizeTopRight => {
                        if top && right {
                            HitTestResult::Normal
                        } else if top {
                            HitTestResult::ResizeRight
                        } else if right {
                            HitTestResult::ResizeTop
                        } else {
                            rc
                        }
                    }
                    HitTestResult::ResizeRight if right => HitTestResult::Normal,
                    HitTestResult::ResizeBottomRight => {
                        if bottom && right {
                            HitTestResult::Normal
                        } else if bottom {
                            HitTestResult::ResizeRight
                        } else if right {
                            HitTestResult::ResizeBottom
                        } else {
                            rc
                        }
                    }
                    HitTestResult::ResizeBottom if bottom => HitTestResult::Normal,
                    HitTestResult::ResizeBottomLeft => {
                        if bottom && left {
                            HitTestResult::Normal
                        } else if bottom {
                            HitTestResult::ResizeLeft
                        } else if left {
                            HitTestResult::ResizeBottom
                        } else {
                            rc
                        }
                    }
                    HitTestResult::ResizeLeft if left => HitTestResult::Normal,
                    _ => rc,
                };

                let changed = self.with_data(|d| {
                    let wd = d.window_mut(window)?;
                    if rc != wd.hit_test_result {
                        wd.hit_test_result = rc;
                        Some(())
                    } else {
                        None
                    }
                });
                if changed.is_some() {
                    self.wayland_seat_update_pointer_cursor(seat);
                }
            }
        }
    }

    /// The pointer listener (`pointer_listener`).
    fn handle_pointer_event(&self, seat: u32, event: WlPointerEvent<'_>) {
        match event {
            WlPointerEvent::Enter {
                serial,
                surface,
                surface_x,
                surface_y,
            } => self.pointer_handle_enter(
                seat,
                serial,
                surface.map(|s| s.raw() as usize),
                surface_x,
                surface_y,
            ),
            WlPointerEvent::Leave { surface, .. } => {
                self.pointer_handle_leave(seat, surface.map(|s| s.raw() as usize))
            }
            WlPointerEvent::Motion {
                time,
                surface_x,
                surface_y,
            } => self.pointer_handle_motion(seat, time, surface_x, surface_y),
            WlPointerEvent::Button {
                serial,
                time,
                button,
                state,
            } => self.pointer_handle_button(seat, serial, time, button, state),
            WlPointerEvent::Axis { time, axis, value } => {
                self.pointer_handle_axis(seat, time, axis, value)
            }
            WlPointerEvent::Frame => self.pointer_handle_frame(seat),
            WlPointerEvent::AxisSource { .. } => {
                // unimplemented
            }
            WlPointerEvent::AxisStop { .. } => {
                // unimplemented
            }
            WlPointerEvent::AxisDiscrete { axis, discrete } => self.pointer_handle_axis_common(
                seat,
                WaylandAxisEvent::Discrete,
                axis,
                Fixed::from_int(discrete),
            ),
            WlPointerEvent::AxisValue120 { axis, value120 } => self.pointer_handle_axis_common(
                seat,
                WaylandAxisEvent::Value120,
                axis,
                Fixed::from_int(value120),
            ),
            WlPointerEvent::AxisRelativeDirection { direction, .. } => self.with_data(|d| {
                // Translation of `pointer_handle_axis_relative_direction()`.
                if let Some(s) = d.seat_mut(seat) {
                    match direction {
                        WlPointerAxisRelativeDirection::IDENTICAL => {
                            s.pointer.pending_frame.axis.direction = MouseWheelDirection::Normal
                        }
                        WlPointerAxisRelativeDirection::INVERTED => {
                            s.pointer.pending_frame.axis.direction = MouseWheelDirection::Flipped
                        }
                        _ => {}
                    }
                }
            }),
            WlPointerEvent::Warp {
                surface_x,
                surface_y,
            } => self.with_data(|d| {
                // Translation of `pointer_handle_warp()`.
                if let Some(s) = d.seat_mut(seat) {
                    s.pointer.pending_frame.have_absolute = true;
                    s.pointer.pending_frame.have_warp = true;
                    s.pointer.pending_frame.absolute_sx = surface_x;
                    s.pointer.pending_frame.absolute_sy = surface_y;
                }
            }),
        }
    }

    /// The pointer's version (0 without one).
    fn pointer_version(d: &VideoData, seat: u32) -> u32 {
        d.seat(seat)
            .and_then(|s| s.pointer.wl_pointer.as_ref())
            .map_or(0, |p| p.version())
    }

    /// Translation of `pointer_handle_motion()`.
    fn pointer_handle_motion(&self, seat: u32, time: u32, sx: Fixed, sy: Fixed) {
        let dispatch_now = self.with_data(|d| {
            let timestamp = d.pointer_timestamp(seat, time);
            let version = Self::pointer_version(d, seat);
            let s = d.seat_mut(seat)?;
            let has_timestamps = s.pointer.timestamps.is_some();

            s.pointer.pending_frame.absolute_sx = sx;
            s.pointer.pending_frame.absolute_sy = sy;

            if version >= WlPointer::FRAME_SINCE_VERSION {
                s.pointer.pending_frame.have_absolute = true;

                /* The relative pointer timestamp is higher resolution than the default millisecond timestamp,
                 * but lower than the highres timestamp. Use the best timer available for this frame,
                 */
                if !s.pointer.pending_frame.have_relative || has_timestamps {
                    s.pointer.pending_frame.timestamp_ns = timestamp;
                }
                Some(false)
            } else {
                s.pointer.pending_frame.timestamp_ns = timestamp;
                Some(true)
            }
        });
        if dispatch_now == Some(true) {
            self.pointer_dispatch_absolute_motion(seat, false);
        }
    }

    /// Translation of `pointer_dispatch_enter()`.
    fn pointer_dispatch_enter(&self, seat: u32) {
        let (window, enter_surface, is_content) = self.with_data(|d| {
            let enter_surface = d
                .seat(seat)
                .map_or(0, |s| s.pointer.pending_frame.enter_surface);
            let window = d.window_for_surface(enter_surface);
            let is_content = window
                .and_then(|w| d.window(w))
                .is_some_and(|w| w.surface_id() == enter_surface);
            (window, enter_surface, is_content)
        });
        let Some(window) = window else {
            // Entering a surface not managed by SDL; just set the cursor reset flag.
            self.wayland_seat_reset_cursor(seat);
            return;
        };

        if !is_content {
            /* This surface is part of the window managed by SDL, but it is not the main content
             * surface and doesn't get focus. Just set the default cursor and leave.
             */
            self.wayland_seat_set_default_cursor(seat);
            return;
        }

        self.with_data(|d| {
            if let Some(s) = d.seat_mut(seat) {
                s.pointer.focus = Some(window);
                s.pointer.focus_surface = enter_surface;
            }
            if let Some(w) = d.window_mut(window) {
                w.pointer_focus_count += 1;
            }
        });
        mouse::set_mouse_focus(Some(window));

        // Send the initial position.
        self.pointer_dispatch_absolute_motion(seat, false);

        // Update the pointer grab state.
        self.wayland_seat_update_pointer_grab(seat);

        /* If the cursor was changed while our window didn't have pointer
         * focus, we might need to trigger another call to
         * wl_pointer_set_cursor() for the new cursor to be displayed.
         *
         * This will also update the cursor if a second pointer entered a
         * window that already has focus, as the focus change sequence
         * won't be run.
         */
        self.wayland_seat_update_pointer_cursor(seat);
    }

    /// Translation of `pointer_handle_enter()`.
    fn pointer_handle_enter(
        &self,
        seat: u32,
        serial: u32,
        surface: Option<usize>,
        sx_w: Fixed,
        sy_w: Fixed,
    ) {
        let Some(surface) = surface else {
            // Enter event for a destroyed surface.
            return;
        };

        let dispatch_now = self.with_data(|d| {
            let version = Self::pointer_version(d, seat);
            let s = d.seat_mut(seat)?;
            s.pointer.pending_frame.enter_surface = surface;
            s.pointer.enter_serial = serial;

            /* In the case of e.g. a pointer confine warp, we may receive an enter
             * event with no following motion event, but with the new coordinates
             * as part of the enter event.
             */
            s.pointer.pending_frame.absolute_sx = sx_w;
            s.pointer.pending_frame.absolute_sy = sy_w;

            if version < WlPointer::FRAME_SINCE_VERSION {
                // Dispatching an enter event generates an absolute motion event, for which there is no timestamp.
                s.pointer.pending_frame.timestamp_ns = 0;
                Some(true)
            } else {
                Some(false)
            }
        });
        if dispatch_now == Some(true) {
            self.pointer_dispatch_enter(seat);
        }
    }

    /// Translation of `pointer_dispatch_leave()`.
    fn pointer_dispatch_leave(&self, seat: u32, update_pointer: bool) {
        enum Leave {
            Nothing,
            Reset,
            Dispatch(PointerLeave),
        }
        let leave = self.with_data(|d| {
            let Some(leave_surface) = d.seat(seat).map(|s| s.pointer.pending_frame.leave_surface)
            else {
                return Leave::Nothing;
            };
            let Some(window) = d.window_for_surface(leave_surface) else {
                return Leave::Nothing;
            };
            let focus = d.seat(seat).and_then(|s| s.pointer.focus);
            let Some(focus) = focus else {
                return if update_pointer {
                    // Leaving a non-content surface managed by SDL; just set the cursor reset flag.
                    Leave::Reset
                } else {
                    Leave::Nothing
                };
            };
            if d.window(focus).map(|w| w.surface_id()) != Some(leave_surface) {
                return Leave::Nothing;
            }

            let Some(s) = d.seat_mut(seat) else {
                return Leave::Nothing;
            };
            s.pointer.focus = None;
            s.pointer.focus_surface = 0;
            let mut buttons = Vec::new();
            let mut i: u8 = 1;
            while s.pointer.buttons_pressed != 0 {
                if s.pointer.buttons_pressed & button_mask(i) != 0 {
                    buttons.push(i);
                    s.pointer.buttons_pressed &= !button_mask(i);
                }
                if i == u8::MAX || i >= 32 {
                    s.pointer.buttons_pressed = 0;
                    break;
                }
                i += 1;
            }
            let sdl_id = s.pointer.sdl_id;
            let Some(w) = d.window_mut(window) else {
                return Leave::Nothing;
            };
            w.pointer_focus_count -= 1;
            Leave::Dispatch(PointerLeave {
                window,
                buttons,
                sdl_id,
                pointer_focus_count: w.pointer_focus_count,
                active_touch_count: w.active_touch_count,
            })
        });

        match leave {
            Leave::Nothing => {}
            Leave::Reset => self.wayland_seat_reset_cursor(seat),
            Leave::Dispatch(l) => {
                // Clear the capture flag and raise all buttons
                let _ = with_window(l.window, |w| {
                    w.core.flags.set(WindowFlags::MOUSE_CAPTURE, false)
                });

                for i in l.buttons {
                    mouse::send_mouse_button(Duration::ZERO, Some(l.window), l.sdl_id, i, false);
                }

                /* A pointer leave event may be emitted if the compositor hides the pointer in response to receiving a touch event.
                 * Don't relinquish focus if the surface has active touches, as the compositor is just transitioning from mouse to touch mode.
                 */
                let had_focus = mouse::mouse_focus() == Some(l.window);
                if l.pointer_focus_count == 0 && had_focus && l.active_touch_count == 0 {
                    mouse::set_mouse_focus(None);
                }

                if update_pointer {
                    self.wayland_seat_update_pointer_grab(seat);
                    self.wayland_seat_update_pointer_cursor(seat);
                }
            }
        }
    }

    /// Translation of `pointer_handle_leave()`.
    fn pointer_handle_leave(&self, seat: u32, surface: Option<usize>) {
        let Some(surface) = surface else {
            // Leave event for a destroyed surface.
            return;
        };

        let dispatch_now = self.with_data(|d| {
            let version = Self::pointer_version(d, seat);
            let s = d.seat_mut(seat)?;
            s.pointer.pending_frame.leave_surface = surface;
            Some(version < WlPointer::FRAME_SINCE_VERSION)
        });
        if dispatch_now == Some(true) {
            self.pointer_dispatch_leave(seat, true);
        }
    }

    /// Translation of `Wayland_ProcessHitTest()`.
    fn wayland_process_hit_test(&self, seat: u32, serial: u32) -> bool {
        let Some((window, hit_test_result, locked)) = self.with_data(|d| {
            let s = d.seat(seat)?;
            let window = s.pointer.focus?;
            Some((
                window,
                d.window(window)?.hit_test_result,
                s.pointer.locked_pointer.is_some(),
            ))
        }) else {
            return false;
        };

        // Pointer is immobilized, do nothing.
        if locked {
            return false;
        }

        let has_hit_test = with_window(window, |w| w.hit_test.is_some()).unwrap_or(false);
        if !has_hit_test {
            return false;
        }

        const DIRECTIONS: [XdgToplevelResizeEdge; 8] = [
            XdgToplevelResizeEdge::TOP_LEFT,
            XdgToplevelResizeEdge::TOP,
            XdgToplevelResizeEdge::TOP_RIGHT,
            XdgToplevelResizeEdge::RIGHT,
            XdgToplevelResizeEdge::BOTTOM_RIGHT,
            XdgToplevelResizeEdge::BOTTOM,
            XdgToplevelResizeEdge::BOTTOM_LEFT,
            XdgToplevelResizeEdge::LEFT,
        ];

        const DIRECTIONS_LIBDECOR: [libdecor_resize_edge; 8] = [
            LIBDECOR_RESIZE_EDGE_TOP_LEFT,
            LIBDECOR_RESIZE_EDGE_TOP,
            LIBDECOR_RESIZE_EDGE_TOP_RIGHT,
            LIBDECOR_RESIZE_EDGE_RIGHT,
            LIBDECOR_RESIZE_EDGE_BOTTOM_RIGHT,
            LIBDECOR_RESIZE_EDGE_BOTTOM,
            LIBDECOR_RESIZE_EDGE_BOTTOM_LEFT,
            LIBDECOR_RESIZE_EDGE_LEFT,
        ];

        let libdecor = self.syms().libdecor.as_ref();
        self.with_data(|d| {
            let Some(s) = d.seat(seat) else {
                return false;
            };
            let wl_seat = s.wl_seat.raw();
            let Some(wd) = d.window(window) else {
                return false;
            };
            match hit_test_result {
                HitTestResult::Draggable => {
                    if wd.shell_surface_type == ShellSurfaceType::Libdecor {
                        if let (Some(frame), Some(l)) = (&wd.shell_surface.libdecor_frame, libdecor)
                        {
                            // SAFETY: the frame and seat are alive.
                            unsafe { (l.libdecor_frame_move)(frame.raw(), wl_seat, serial) };
                        }
                    } else if wd.shell_surface_type == ShellSurfaceType::XdgToplevel {
                        if let Some(toplevel) = &wd.shell_surface.xdg_toplevel {
                            toplevel.r#move(s.wl_seat.obj(), serial);
                        }
                    }
                    true
                }
                HitTestResult::ResizeTopLeft
                | HitTestResult::ResizeTop
                | HitTestResult::ResizeTopRight
                | HitTestResult::ResizeRight
                | HitTestResult::ResizeBottomRight
                | HitTestResult::ResizeBottom
                | HitTestResult::ResizeBottomLeft
                | HitTestResult::ResizeLeft => {
                    let i = hit_test_result as usize - HitTestResult::ResizeTopLeft as usize;
                    if wd.shell_surface_type == ShellSurfaceType::Libdecor {
                        if let (Some(frame), Some(l)) = (&wd.shell_surface.libdecor_frame, libdecor)
                        {
                            // SAFETY: the frame and seat are alive.
                            unsafe {
                                (l.libdecor_frame_resize)(
                                    frame.raw(),
                                    wl_seat,
                                    serial,
                                    DIRECTIONS_LIBDECOR[i],
                                )
                            };
                        }
                    } else if wd.shell_surface_type == ShellSurfaceType::XdgToplevel {
                        if let Some(toplevel) = &wd.shell_surface.xdg_toplevel {
                            toplevel.resize(s.wl_seat.obj(), serial, DIRECTIONS[i]);
                        }
                    }
                    true
                }
                _ => false,
            }
        })
    }

    /// Translation of `pointer_dispatch_button()`.
    fn pointer_dispatch_button(&self, seat: u32, sdl_button: u8, down: bool) {
        let Some(window) = self.with_data(|d| d.seat(seat)?.pointer.focus) else {
            return;
        };

        if down {
            let serial = self.with_data(|d| {
                let s = d.seat_mut(seat)?;
                s.pointer.buttons_pressed |= button_mask(sdl_button);
                Some(s.last_implicit_grab_serial)
            });

            if sdl_button == BUTTON_LEFT && self.wayland_process_hit_test(seat, serial.unwrap_or(0))
            {
                return; // don't pass this event on to app.
            }
        } else {
            self.with_data(|d| {
                if let Some(s) = d.seat_mut(seat) {
                    s.pointer.buttons_pressed &= !button_mask(sdl_button);
                }
            });
        }

        // Possibly ignore this click if it was to gain focus.
        let mut ignore_click = false;
        let last_focus = self.with_data(|d| {
            let w = d.window_mut(window)?;
            let t = w.last_focus_event_time_ns;
            if t != 0 {
                w.last_focus_event_time_ns = 0;
            }
            Some(t)
        });
        if let Some(t) = last_focus.filter(|&t| t != 0) {
            if down && crate::timer::ticks_ns().wrapping_sub(t) < WAYLAND_FOCUS_CLICK_TIMEOUT_NS {
                ignore_click = !hints::get_bool(hints::MOUSE_FOCUS_CLICKTHROUGH, false);
            }
        }

        /* Wayland won't let you "capture" the mouse, but it will automatically track
         * the mouse outside the window if you drag outside of it, until you let go
         * of all buttons (even if you add or remove presses outside the window, as
         * long as any button is still down, the capture remains).
         *
         * The mouse is not captured in relative mode.
         */
        let Some((relative, buttons_pressed, focus_surface, window_surface, timestamp, sdl_id)) =
            self.with_data(|d| {
                let s = d.seat(seat)?;
                Some((
                    s.pointer.relative_pointer.is_some(),
                    s.pointer.buttons_pressed,
                    s.pointer.focus_surface,
                    d.window(window)?.surface_id(),
                    s.pointer.pending_frame.timestamp_ns,
                    s.pointer.sdl_id,
                ))
            })
        else {
            return;
        };
        if !relative {
            if buttons_pressed != 0 {
                let _ = with_window(window, |w| {
                    w.core.flags.set(WindowFlags::MOUSE_CAPTURE, true)
                });
            } else {
                let _ = with_window(window, |w| {
                    w.core.flags.set(WindowFlags::MOUSE_CAPTURE, false)
                });

                // If ending the capture on a subsurface, dispatch a leave event to remove focus.
                if focus_surface != window_surface {
                    self.with_data(|d| {
                        if let Some(s) = d.seat_mut(seat) {
                            s.pointer.pending_frame.leave_surface = window_surface;
                        }
                    });
                }
            }
        }

        if !ignore_click {
            mouse::send_mouse_button(ns(timestamp), Some(window), sdl_id, sdl_button, down);
        }
    }

    /// Translation of `pointer_handle_button()`.
    fn pointer_handle_button(
        &self,
        seat: u32,
        serial: u32,
        time: u32,
        button: u32,
        state_w: WlPointerButtonState,
    ) {
        let sdl_button: u8 = match button {
            BTN_LEFT => BUTTON_LEFT,
            BTN_MIDDLE => BUTTON_MIDDLE,
            BTN_RIGHT => BUTTON_RIGHT,
            // FIXME (upstream): buttons below BTN_SIDE (other than the three
            // above) wrap around, and buttons past BTN_SIDE + 27 make
            // SDL_BUTTON_MASK() shift out of range (undefined behaviour in
            // C); the C truncation to Uint8 is kept, and the out of range
            // masks are 0 here.
            _ => (BUTTON_X1 as u32).wrapping_add(button.wrapping_sub(BTN_SIDE)) as u8,
        };

        if state_w.0 != 0 {
            self.wayland_update_implicit_grab_serial(seat, serial);
        }

        let dispatch_now = self.with_data(|d| {
            let timestamp = d.pointer_timestamp(seat, time);
            let s = d.seat_mut(seat)?;
            s.pointer.pending_frame.timestamp_ns = timestamp;

            if s.wl_seat.version() >= WlPointer::FRAME_SINCE_VERSION {
                if state_w.0 != 0 {
                    s.pointer.pending_frame.buttons_pressed |= button_mask(sdl_button);
                } else {
                    s.pointer.pending_frame.buttons_released |= button_mask(sdl_button);
                }
                Some(false)
            } else {
                Some(true)
            }
        });
        if dispatch_now == Some(true) {
            self.pointer_dispatch_button(seat, sdl_button, state_w.0 != 0);
        }
    }

    /// Translation of `pointer_handle_axis_common_v1()`.
    fn pointer_handle_axis_common_v1(
        &self,
        seat: u32,
        ns_timestamp: u64,
        axis: WlPointerAxis,
        value: Fixed,
    ) {
        let Some((window, sdl_id)) = self.with_data(|d| {
            let s = d.seat(seat)?;
            Some((s.pointer.focus?, s.pointer.sdl_id))
        }) else {
            return;
        };

        let (mut x, mut y) = match axis {
            WlPointerAxis::VERTICAL_SCROLL => (0.0, 0.0 - value.to_f64() as f32),
            WlPointerAxis::HORIZONTAL_SCROLL => (value.to_f64() as f32, 0.0),
            _ => return,
        };

        x /= WAYLAND_WHEEL_AXIS_UNIT;
        y /= WAYLAND_WHEEL_AXIS_UNIT;

        mouse::send_mouse_wheel(
            ns(ns_timestamp),
            Some(window),
            sdl_id,
            x,
            y,
            MouseWheelDirection::Normal,
        );
    }

    /// Translation of `pointer_handle_axis_common()`.
    fn pointer_handle_axis_common(
        &self,
        seat: u32,
        ty: WaylandAxisEvent,
        axis: WlPointerAxis,
        value: Fixed,
    ) {
        self.with_data(|d| {
            let Some(s) = d.seat_mut(seat) else {
                return;
            };
            if s.pointer.focus.is_none() {
                return;
            }
            s.pointer.pending_frame.have_axis = true;
            let a = &mut s.pointer.pending_frame.axis;
            let v = value.to_f64() as f32;

            match axis {
                WlPointerAxis::VERTICAL_SCROLL => match ty {
                    WaylandAxisEvent::Value120 => {
                        /*
                         * High resolution scroll event. The spec doesn't state that axis_value120
                         * events are limited to one per frame, so the values are accumulated.
                         */
                        if a.y_axis_type != WaylandAxisEvent::Value120 {
                            a.y_axis_type = WaylandAxisEvent::Value120;
                            a.y = 0.0;
                        }
                        a.y += 0.0 - v;
                    }
                    WaylandAxisEvent::Discrete => {
                        /*
                         * This is a discrete axis event, so we process it and set the
                         * flag to ignore future continuous axis events in this frame.
                         */
                        if a.y_axis_type != WaylandAxisEvent::Discrete {
                            a.y_axis_type = WaylandAxisEvent::Discrete;
                            a.y = 0.0 - v;
                        }
                    }
                    WaylandAxisEvent::Continuous => {
                        // Only process continuous events if no discrete events have been received.
                        if a.y_axis_type == WaylandAxisEvent::Continuous {
                            a.y = 0.0 - v;
                        }
                    }
                },
                WlPointerAxis::HORIZONTAL_SCROLL => match ty {
                    WaylandAxisEvent::Value120 => {
                        if a.x_axis_type != WaylandAxisEvent::Value120 {
                            a.x_axis_type = WaylandAxisEvent::Value120;
                            a.x = 0.0;
                        }
                        a.x += v;
                    }
                    WaylandAxisEvent::Discrete => {
                        if a.x_axis_type != WaylandAxisEvent::Discrete {
                            a.x_axis_type = WaylandAxisEvent::Discrete;
                            a.x = v;
                        }
                    }
                    WaylandAxisEvent::Continuous => {
                        if a.x_axis_type == WaylandAxisEvent::Continuous {
                            a.x = v;
                        }
                    }
                },
                _ => {}
            }
        });
    }

    /// Translation of `pointer_handle_axis()`.
    fn pointer_handle_axis(&self, seat: u32, time: u32, axis: WlPointerAxis, value: Fixed) {
        let Some((ns_timestamp, framed)) = self.with_data(|d| {
            let t = d.pointer_timestamp(seat, time);
            let s = d.seat_mut(seat)?;
            let framed = s.wl_seat.version() >= WlPointer::FRAME_SINCE_VERSION;
            if framed {
                s.pointer.pending_frame.timestamp_ns = t;
            }
            Some((t, framed))
        }) else {
            return;
        };

        if framed {
            self.pointer_handle_axis_common(seat, WaylandAxisEvent::Continuous, axis, value);
        } else {
            self.pointer_handle_axis_common_v1(seat, ns_timestamp, axis, value);
        }
    }

    /// Translation of `pointer_dispatch_relative_motion()`.
    fn pointer_dispatch_relative_motion(&self, seat: u32) {
        let (has_transform, system_scale) =
            mouse::with_mouse(|m| (m.input_transform.is_some(), m.enable_relative_system_scale));
        let Some((window, dx, dy, timestamp, sdl_id)) = self.with_data(|d| {
            let s = d.seat(seat)?;
            let window = s.pointer.focus?;
            let pf = s.pointer.pending_frame;
            let wd = d.window(window)?;
            let (dx, dy) = if has_transform || !system_scale {
                (
                    pf.relative_dx_unaccel.to_f64(),
                    pf.relative_dy_unaccel.to_f64(),
                )
            } else {
                (
                    pf.relative_dx.to_f64() * wd.pointer_scale.x,
                    pf.relative_dy.to_f64() * wd.pointer_scale.y,
                )
            };
            Some((window, dx, dy, pf.timestamp_ns, s.pointer.sdl_id))
        }) else {
            return;
        };

        mouse::send_mouse_motion(
            ns(timestamp),
            Some(window),
            sdl_id,
            true,
            dx as f32,
            dy as f32,
        );
    }

    /// Translation of `pointer_dispatch_axis()`.
    fn pointer_dispatch_axis(&self, seat: u32) {
        let Some((window, axis, timestamp, sdl_id)) = self.with_data(|d| {
            let s = d.seat(seat)?;
            Some((
                s.pointer.focus?,
                s.pointer.pending_frame.axis,
                s.pointer.pending_frame.timestamp_ns,
                s.pointer.sdl_id,
            ))
        }) else {
            return;
        };
        let direction = axis.direction;

        let x = match axis.x_axis_type {
            WaylandAxisEvent::Continuous => axis.x / WAYLAND_WHEEL_AXIS_UNIT,
            WaylandAxisEvent::Discrete => axis.x,
            WaylandAxisEvent::Value120 => axis.x / 120.0,
        };

        let y = match axis.y_axis_type {
            WaylandAxisEvent::Continuous => axis.y / WAYLAND_WHEEL_AXIS_UNIT,
            WaylandAxisEvent::Discrete => axis.y,
            WaylandAxisEvent::Value120 => axis.y / 120.0,
        };

        mouse::send_mouse_wheel(ns(timestamp), Some(window), sdl_id, x, y, direction);
    }

    /// Translation of `pointer_handle_frame()`.
    fn pointer_handle_frame(&self, seat: u32) {
        let Some(pf) = self.with_data(|d| d.seat(seat).map(|s| s.pointer.pending_frame)) else {
            return;
        };

        if pf.enter_surface != 0 {
            if pf.leave_surface != 0 {
                let (window_data, new_focus) = self.with_data(|d| {
                    (
                        d.seat(seat).and_then(|s| s.pointer.focus),
                        d.window_for_surface(pf.enter_surface),
                    )
                });
                let captured = window_data.is_some_and(|w| {
                    with_window(w, |wd| wd.flags().contains(WindowFlags::MOUSE_CAPTURE))
                        .unwrap_or(false)
                });

                if window_data.is_some() && captured && window_data == new_focus {
                    // The mouse is captured and moving between owned window surfaces. Just change the focused surface.
                    self.with_data(|d| {
                        if let Some(s) = d.seat_mut(seat) {
                            s.pointer.focus_surface = pf.enter_surface;
                            s.pointer.pending_frame.enter_surface = 0;
                            s.pointer.pending_frame.leave_surface = 0;
                        }
                    });
                } else {
                    // Leaving the previous surface before entering a new surface.
                    self.pointer_dispatch_leave(seat, false);
                    self.with_data(|d| {
                        if let Some(s) = d.seat_mut(seat) {
                            s.pointer.pending_frame.leave_surface = 0;
                        }
                    });
                }
            }

            let enter = self.with_data(|d| {
                d.seat(seat)
                    .map_or(0, |s| s.pointer.pending_frame.enter_surface)
            });
            if enter != 0 {
                self.pointer_dispatch_enter(seat);
            }
        }

        let pf = self
            .with_data(|d| d.seat(seat).map(|s| s.pointer.pending_frame))
            .unwrap_or_default();

        if pf.have_absolute {
            self.pointer_dispatch_absolute_motion(seat, pf.have_warp);
        }

        if pf.have_relative {
            self.pointer_dispatch_relative_motion(seat);
        }

        let mut i: u8 = 1;
        while let Some((pressed, released)) = self.with_data(|d| {
            d.seat(seat).map(|s| {
                (
                    s.pointer.pending_frame.buttons_pressed,
                    s.pointer.pending_frame.buttons_released,
                )
            })
        }) {
            if pressed == 0 && released == 0 {
                break;
            }
            let mask = button_mask(i);
            if pressed & mask != 0 {
                self.pointer_dispatch_button(seat, i, true);
                self.with_data(|d| {
                    if let Some(s) = d.seat_mut(seat) {
                        s.pointer.pending_frame.buttons_pressed &= !mask;
                    }
                });
            }
            if released & mask != 0 {
                self.pointer_dispatch_button(seat, i, false);
                self.with_data(|d| {
                    if let Some(s) = d.seat_mut(seat) {
                        s.pointer.pending_frame.buttons_released &= !mask;
                    }
                });
            }
            if i >= 32 {
                break;
            }
            i += 1;
        }

        if pf.have_axis {
            self.pointer_dispatch_axis(seat);
        }

        let leave = self.with_data(|d| {
            d.seat(seat)
                .map_or(0, |s| s.pointer.pending_frame.leave_surface)
        });
        if leave != 0 {
            self.pointer_dispatch_leave(seat, true);
        }

        self.with_data(|d| {
            if let Some(s) = d.seat_mut(seat) {
                s.pointer.pending_frame = PendingFrame::default();
            }
        });
    }

    /// The relative pointer listener (`relative_pointer_listener`).
    /// Translation of `relative_pointer_handle_relative_motion()`.
    fn relative_pointer_handle_relative_motion(&self, seat: u32, event: ZwpRelativePointerV1Event) {
        let ZwpRelativePointerV1Event::RelativeMotion {
            utime_hi,
            utime_lo,
            dx,
            dy,
            dx_unaccel,
            dy_unaccel,
        } = event;

        let dispatch_now = self.with_data(|d| {
            let version = Self::pointer_version(d, seat);
            let s = d.seat_mut(seat)?;
            // Relative pointer event times are in microsecond granularity.
            s.pointer.pending_frame.relative_dx = dx;
            s.pointer.pending_frame.relative_dy = dy;
            s.pointer.pending_frame.relative_dx_unaccel = dx_unaccel;
            s.pointer.pending_frame.relative_dy_unaccel = dy_unaccel;
            let us = ((utime_hi as u64) << 32) | utime_lo as u64;
            s.pointer.pending_frame.timestamp_ns =
                wayland_adjust_event_timestamp_base(us.wrapping_mul(1000));

            if version >= WlPointer::FRAME_SINCE_VERSION {
                s.pointer.pending_frame.have_relative = true;
                Some(false)
            } else {
                Some(true)
            }
        });
        if dispatch_now == Some(true) {
            self.pointer_dispatch_relative_motion(seat);
        }
    }

    /// Set the seat's pointer confinement state (`locked_pointer_locked()`
    /// and the other listeners of the locked and confined pointers).
    fn set_pointer_confined(&self, seat: u32, confined: bool) {
        self.with_data(|d| {
            if let Some(s) = d.seat_mut(seat) {
                s.pointer.is_confined = confined;
            }
        });
    }

    /// The touch listener (`touch_listener`).
    fn handle_touch_event(&self, seat: u32, touch: Obj<'_, WlTouch>, event: WlTouchEvent<'_>) {
        let touch_id = touch.raw() as usize as u64;
        match event {
            WlTouchEvent::Down {
                serial,
                time,
                surface,
                id,
                x,
                y,
            } => {
                // Translation of `touch_handler_down()`.
                // Check that this surface is valid.
                let Some(surface) = surface.map(|s| s.raw() as usize) else {
                    return;
                };

                self.with_data(|d| Self::wayland_seat_add_touch(d, seat, id, x, y, surface));
                self.wayland_update_implicit_grab_serial(seat, serial);
                let Some((window, fx, fy, timestamp)) = self.with_data(|d| {
                    let window = d.window_for_surface(surface)?;
                    let timestamp = d.touch_timestamp(seat, time);
                    let wd = d.window_mut(window)?;
                    if wd.surface_id() != surface {
                        return None;
                    }
                    let fx = if wd.current.logical_width <= 1 {
                        0.5
                    } else {
                        x.to_f64() as f32 / (wd.current.logical_width - 1) as f32
                    };
                    let fy = if wd.current.logical_height <= 1 {
                        0.5
                    } else {
                        y.to_f64() as f32 / (wd.current.logical_height - 1) as f32
                    };

                    wd.active_touch_count += 1;
                    Some((window, fx, fy, timestamp))
                }) else {
                    return;
                };
                mouse::set_mouse_focus(Some(window));

                touch::send_touch(
                    ns(timestamp),
                    touch_id,
                    (id + 1) as u64,
                    Some(window),
                    EventType::FINGER_DOWN,
                    fx,
                    fy,
                    1.0,
                );
            }
            WlTouchEvent::Up { time, id, .. } => {
                // Translation of `touch_handler_up()`.
                let Some((window, fx, fy, timestamp)) = self.with_data(|d| {
                    let tp = Self::wayland_seat_remove_touch(d, seat, id)?;
                    if tp.surface == 0 {
                        return None;
                    }
                    let window = d.window_for_surface(tp.surface)?;
                    let timestamp = d.touch_timestamp(seat, time);
                    let wd = d.window_mut(window)?;
                    if wd.surface_id() != tp.surface {
                        return None;
                    }
                    let fx = tp.fx.to_f64() as f32 / wd.current.logical_width as f32;
                    let fy = tp.fy.to_f64() as f32 / wd.current.logical_height as f32;
                    wd.active_touch_count -= 1;
                    Some((window, fx, fy, timestamp))
                }) else {
                    return;
                };

                touch::send_touch(
                    ns(timestamp),
                    touch_id,
                    (id + 1) as u64,
                    Some(window),
                    EventType::FINGER_UP,
                    fx,
                    fy,
                    0.0,
                );

                self.maybe_lose_mouse_focus(window);
            }
            WlTouchEvent::Motion { time, id, x, y } => {
                // Translation of `touch_handler_motion()`.
                let Some((window, fx, fy, timestamp)) = self.with_data(|d| {
                    let surface = Self::wayland_seat_update_touch(d, seat, id, x, y);
                    if surface == 0 {
                        return None;
                    }
                    let window = d.window_for_surface(surface)?;
                    let timestamp = d.touch_timestamp(seat, time);
                    let wd = d.window(window)?;
                    if wd.surface_id() != surface {
                        return None;
                    }
                    let fx = x.to_f64() as f32 / wd.current.logical_width as f32;
                    let fy = y.to_f64() as f32 / wd.current.logical_height as f32;
                    Some((window, fx, fy, timestamp))
                }) else {
                    return;
                };

                touch::send_touch_motion(
                    ns(timestamp),
                    touch_id,
                    (id + 1) as u64,
                    Some(window),
                    fx,
                    fy,
                    1.0,
                );
            }
            WlTouchEvent::Frame => {}
            WlTouchEvent::Cancel => self.touch_handler_cancel(seat),
            WlTouchEvent::Shape { .. } => {}
            WlTouchEvent::Orientation { .. } => {}
        }
    }

    /// Translation of `touch_handler_cancel()`.
    fn touch_handler_cancel(&self, seat: u32) {
        // (cancelling a touch point removes it from the list)
        let points = self.with_data(|d| {
            d.seat_mut(seat)
                .map(|s| std::mem::take(&mut s.touch.points))
        });
        for tp in points.unwrap_or_default() {
            self.wayland_seat_cancel_touch(seat, tp);
        }
    }

    /// Translation of `keyboard_handle_keymap()`.
    fn keyboard_handle_keymap(
        &self,
        seat: u32,
        format: WlKeyboardKeymapFormat,
        fd: OwnedFd,
        size: u32,
    ) {
        if format != WlKeyboardKeymapFormat::XKB_V1 {
            return;
        }

        // SAFETY: a private read-only mapping of the keymap file the
        // compositor sent; unmapped below.
        let map_str = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size as usize,
                libc::PROT_READ,
                libc::MAP_PRIVATE,
                fd.as_raw_fd(),
                0,
            )
        };
        if map_str == libc::MAP_FAILED {
            return;
        }
        // SAFETY: the mapping is `size` bytes long.
        let bytes = unsafe { std::slice::from_raw_parts(map_str as *const u8, size as usize) };
        // (the keymap is NUL-terminated within its size; anything after a
        // NUL is ignored, as C would)
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        let map = std::ffi::CString::new(&bytes[..end]).unwrap_or_default();
        // SAFETY: the mapping made above.
        unsafe { libc::munmap(map_str, size as usize) };
        drop(fd);

        let ok = self.with_data(|d| {
            let Some(context) = d.xkb_context.as_ref() else {
                return false;
            };
            let keymap = XkbKeymap::from_string(context, &map);
            let Some(s) = d.seat_mut(seat) else {
                return false;
            };
            let kb = &mut s.keyboard;

            /* if there's already a keymap loaded, throw it away rather than leaking it before
             * parsing the new one
             */
            kb.xkb.keymap = keymap;

            let Some(keymap) = kb.xkb.keymap.as_ref() else {
                return false;
            };

            // Clear the old layouts.
            kb.sdl_keymap.clear();
            kb.reserved_scancodes.clear();
            kb.xkb.num_layouts = 0;

            kb.xkb.masks = ModMasks::of(keymap);

            /* if there's already a state, throw it away rather than leaking it before
             * trying to create a new one with the new keymap.
             */
            kb.xkb.state = XkbState::new(keymap);
            let Some(state) = kb.xkb.state.as_ref() else {
                // (SDL_SetError("failed to create XKB state"): nothing reads it here)
                kb.xkb.keymap = None;
                return true;
            };

            /*
             * Assume that a nameless layout implies a virtual keyboard with an arbitrary layout.
             * TODO: Use a better method of detection?
             */
            kb.is_virtual = !keymap.layout_has_name(0);

            // Allocate and populate the new layout maps.
            kb.xkb.num_layouts = keymap.num_layouts();
            if kb.xkb.num_layouts != 0 {
                let (maps, reserved) = wayland_build_keymaps(
                    keymap,
                    state,
                    &kb.xkb.masks,
                    kb.xkb.num_layouts,
                    kb.is_virtual,
                );
                kb.sdl_keymap = maps;
                kb.reserved_scancodes = reserved;

                // Restore any previously set modifier/layout information, if valid.
                state.update_mask(
                    kb.xkb.wl_pressed_modifiers,
                    kb.xkb.wl_latched_modifiers,
                    kb.xkb.wl_locked_modifiers,
                    if kb.xkb.current_layout < kb.xkb.num_layouts {
                        kb.xkb.current_layout
                    } else {
                        0
                    },
                );
            }
            true
        });
        if !ok {
            return;
        }
        if self.with_data(|d| {
            d.seat(seat)
                .is_some_and(|s| s.keyboard.xkb.num_layouts != 0)
        }) {
            self.wayland_seat_set_keymap(seat);
        }

        /*
         * See https://blogs.s-osg.org/compose-key-support-weston/
         * for further explanation on dead keys in Wayland.
         */

        // Look up the preferred locale, falling back to "C" as default
        let locale = crate::stdlib::getenv("LC_ALL")
            .or_else(|| crate::stdlib::getenv("LC_CTYPE"))
            .or_else(|| crate::stdlib::getenv("LANG"))
            .unwrap_or_else(|| "C".to_owned());

        /* Set up the XKB compose table.
         *
         * This is a very slow operation, so it is only done during initialization,
         * or if the locale envvar changed during runtime.
         */
        let syms = self.syms().clone();
        self.with_data(|d| {
            let context = d.xkb_context.as_ref().map(|c| c.raw());
            let Some(s) = d.seat_mut(seat) else {
                return;
            };
            let kb = &mut s.keyboard;
            if kb.current_locale.as_deref() != Some(locale.as_str()) {
                // Cache the current locale for later comparison.
                kb.current_locale = Some(locale.clone());

                kb.xkb.compose_table = None;
                let Some(context) = context else {
                    return;
                };
                let c_locale = super::video::c_string(&locale);
                // SAFETY: the context is alive; the locale is NUL-terminated.
                let table = unsafe {
                    (syms.xkb.xkb_compose_table_new_from_locale)(
                        context,
                        c_locale.as_ptr(),
                        XKB_COMPOSE_COMPILE_NO_FLAGS,
                    )
                };
                kb.xkb.compose_table = XkbComposeTable::adopt(&syms, table);
                if let Some(table) = &kb.xkb.compose_table {
                    // Set up XKB compose state
                    kb.xkb.compose_state = None;
                    // SAFETY: the table is alive.
                    let state = unsafe {
                        (syms.xkb.xkb_compose_state_new)(table.raw(), XKB_COMPOSE_STATE_NO_FLAGS)
                    };
                    kb.xkb.compose_state = XkbComposeState::adopt(&syms, state);
                    if kb.xkb.compose_state.is_none() {
                        // (SDL_SetError("could not create XKB compose state"): nothing reads it here)
                        kb.xkb.compose_table = None;
                    }
                }
            } else if let Some(compose_state) = &kb.xkb.compose_state {
                compose_state.reset();
            }
        });
    }

    /// Translation of `Wayland_HandleModifierKeys()`.
    fn wayland_handle_modifier_keys(&self, seat: u32, scancode: Scancode, pressed: bool) {
        let Some(modstate) = self.with_data(|d| {
            let s = d.seat(seat)?;
            Some(s.keyboard.pressed_modifiers | s.keyboard.locked_modifiers)
        }) else {
            return;
        };
        let keycode = keyboard::key_from_scancode(scancode, modstate, false);

        /* SDL clients expect modifier state to be activated at the same time as the
         * source keypress, so we set pressed modifier state with the usual modifier
         * keys here, as the explicit modifier event won't arrive until after the
         * keypress event. If this is wrong, it will be corrected when the explicit
         * modifier state is sent at a later time.
         */
        let Some(m) = modifier_for_keycode(keycode) else {
            return;
        };

        let new_state = self.with_data(|d| {
            let s = d.seat_mut(seat)?;
            if pressed {
                s.keyboard.pressed_modifiers |= m;
            } else {
                s.keyboard.pressed_modifiers &= !m;
            }

            Some(wayland_reconcile_modifiers(&mut s.keyboard, true))
        });
        if let Some(new_state) = new_state {
            keyboard::set_mod_state(new_state);
        }
    }

    /// The keyboard listener (`keyboard_listener`).
    fn handle_keyboard_event(&self, seat: u32, event: WlKeyboardEvent<'_>) {
        match event {
            WlKeyboardEvent::Keymap { format, fd, size } => {
                self.keyboard_handle_keymap(seat, format, fd, size)
            }
            WlKeyboardEvent::Enter { surface, keys, .. } => {
                let keys: Vec<u32> = array_u32(keys).collect();
                self.keyboard_handle_enter(seat, surface.map(|s| s.raw() as usize), &keys)
            }
            WlKeyboardEvent::Leave { surface, .. } => {
                self.keyboard_handle_leave(seat, surface.map(|s| s.raw() as usize))
            }
            WlKeyboardEvent::Key {
                serial,
                time,
                key,
                state,
            } => self.keyboard_handle_key(seat, serial, time, key, state),
            WlKeyboardEvent::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => self.keyboard_handle_modifiers(
                seat,
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
            ),
            WlKeyboardEvent::RepeatInfo { rate, delay } => self.with_data(|d| {
                // Translation of `keyboard_handle_repeat_info()`.
                if let Some(s) = d.seat_mut(seat) {
                    s.keyboard.repeat.repeat_rate = rate;
                    s.keyboard.repeat.repeat_delay_ms = delay;

                    if rate == 0 {
                        s.keyboard.repeat.key = 0; // Cancel the repeat to avoid dividing by a rate of zero.
                    }
                }
            }),
        }
    }

    /// Translation of `keyboard_handle_enter()`.
    fn keyboard_handle_enter(&self, seat: u32, surface: Option<usize>, keys: &[u32]) {
        let Some(surface) = surface else {
            // Enter event for a destroyed surface.
            return;
        };

        let Some(window) = self.with_data(|d| {
            let window = d.window_for_surface(surface)?;
            d.window_mut(window)?.keyboard_focus_count += 1;
            d.seat_mut(seat)?.keyboard.focus = Some(window);
            Some(window)
        }) else {
            // Not a surface owned by SDL.
            return;
        };

        // Restore the keyboard focus to the child popup that was holding it
        let focus = with_window(window, |w| w.keyboard_focus).ok().flatten();
        let _ = keyboard::set_keyboard_focus(Some(focus.unwrap_or(window)));

        // Update the keyboard grab and any relative pointer grabs related to this keyboard focus.
        self.wayland_seat_update_keyboard_grab(seat);
        self.wayland_display_update_pointer_grabs(Some(window));

        // Update text input and IME focus.
        self.wayland_seat_update_text_input(seat);

        if self.with_data(|d| {
            d.seat(seat)
                .is_some_and(|s| s.text_input.zwp_text_input.is_none())
        }) {
            crate::core::linux::ime::set_focus(true);
        }

        let timestamp = crate::timer::ticks_ns();
        self.with_data(|d| {
            if let Some(w) = d.window_mut(window) {
                w.last_focus_event_time_ns = timestamp;
            }
        });

        self.wayland_seat_set_keymap(seat);

        for &key in keys {
            let Some((scancode, sdl_id)) = self.with_data(|d| {
                let s = d.seat(seat)?;
                Some((wayland_get_scancode_for_key(s, key).0, s.keyboard.sdl_id))
            }) else {
                continue;
            };
            if scancode != Scancode::UNKNOWN {
                let keycode = keyboard::key_from_scancode(scancode, Keymod::NONE, false);

                if modifier_for_keycode(keycode).is_some() {
                    self.wayland_handle_modifier_keys(seat, scancode, true);
                    keyboard::send_keyboard_key_ignore_modifiers(
                        ns(timestamp),
                        sdl_id,
                        key as i32,
                        scancode,
                        true,
                    );
                }
            }
        }
    }

    /// Translation of `keyboard_handle_leave()`.
    fn keyboard_handle_leave(&self, seat: u32, surface: Option<usize>) {
        let Some(surface) = surface else {
            // Leave event for a destroyed surface.
            return;
        };

        let Some(window) = self.with_data(|d| d.window_for_surface(surface)) else {
            // Not a surface owned by SDL.
            return;
        };

        // Stop key repeat before clearing keyboard focus
        self.with_data(|d| {
            if let Some(s) = d.seat_mut(seat) {
                s.keyboard.repeat.key = 0;
            }
        });

        let mut keyboard_focus = keyboard::keyboard_focus();

        // The keyboard focus may be a child popup
        while let Some(k) = keyboard_focus {
            match with_window(k, |w| (w.is_popup(), w.parent)) {
                Ok((true, parent)) => keyboard_focus = parent,
                _ => break,
            }
        }

        let had_focus = keyboard_focus == Some(window);
        let count = self.with_data(|d| {
            if let Some(s) = d.seat_mut(seat) {
                s.keyboard.focus = None;
            }
            let w = d.window_mut(window)?;
            w.keyboard_focus_count -= 1;
            Some(w.keyboard_focus_count)
        });

        // Only relinquish focus if this window has the active focus, and no other keyboards have focus on the window.
        if count == Some(0) && had_focus {
            let _ = keyboard::set_keyboard_focus(None);
        }

        // Release the keyboard grab and any relative pointer grabs related to this keyboard focus.
        self.wayland_seat_update_keyboard_grab(seat);
        self.wayland_display_update_pointer_grabs(Some(window));

        // Clear the pressed modifiers.
        self.with_data(|d| {
            if let Some(s) = d.seat_mut(seat) {
                s.keyboard.pressed_modifiers = Keymod::NONE;
            }
        });

        // Update text input and IME focus.
        self.wayland_seat_update_text_input(seat);

        let lose_ime_focus = self.with_data(|d| {
            d.seat(seat)
                .is_some_and(|s| s.text_input.zwp_text_input.is_none())
                && d.window(window)
                    .is_some_and(|w| w.keyboard_focus_count == 0)
        });
        if lose_ime_focus {
            crate::core::linux::ime::set_focus(false);
        }

        /* If the window has mouse focus, has no pointers within it, and no active touches, consider
         * mouse focus to be lost.
         */
        let counts = self.with_data(|d| {
            d.window(window)
                .map(|w| (w.pointer_focus_count, w.active_touch_count))
        });
        if let Some((0, 0)) = counts {
            if mouse::mouse_focus() == Some(window) {
                mouse::set_mouse_focus(None);
            }
        }
    }

    /// Translation of `keyboard_handle_key()`.
    fn keyboard_handle_key(
        &self,
        seat: u32,
        serial: u32,
        time: u32,
        key: u32,
        state_w: WlKeyboardKeyState,
    ) {
        let mut state = state_w;
        let mut has_text = None;
        let mut handled_by_ime = false;
        let timestamp_ns = self.with_data(|d| d.keyboard_timestamp(seat, time));

        self.wayland_update_implicit_grab_serial(seat, serial);

        if state == WlKeyboardKeyState::REPEATED {
            // If this key shouldn't be repeated, just return.
            let repeats = self.with_data(|d| {
                d.seat(seat)
                    .and_then(|s| s.keyboard.xkb.keymap.as_ref())
                    .map(|k| k.key_repeats(key + 8))
            });
            if repeats == Some(false) {
                return;
            }

            // SDL automatically handles key tracking and repeat status, so just map 'repeated' to 'pressed'.
            state = WlKeyboardKeyState::PRESSED;
        }

        self.wayland_seat_set_keymap(seat);

        let pressed = state == WlKeyboardKeyState::PRESSED;
        if pressed {
            let keyboard_focus = keyboard::keyboard_focus();
            if keyboard_focus.is_some_and(keyboard::text_input_active) {
                let (text, ime) = self.keyboard_input_get_text(seat, key, true);
                has_text = text;
                handled_by_ime = ime;
            }
        } else {
            let repeat = self.with_data(|d| {
                d.seat(seat)
                    .map(|s| (s.keyboard.repeat.key, s.keyboard.repeat.wl_press_time_ms))
            });
            if let Some((repeat_key, wl_press_time_ms)) = repeat {
                if key == repeat_key {
                    /* Send any due key repeat events before stopping the repeat and generating the key up event.
                     * Compute time based on the Wayland time, as it reports when the release event happened.
                     * Using SDL_GetTicks would be wrong, as it would report when the release event is processed,
                     * which may be off if the application hasn't pumped events for a while.
                     */
                    let elapsed = ms_to_ns(time.wrapping_sub(wl_press_time_ms) as u64);
                    self.keyboard_repeat_handle(seat, elapsed);
                    self.with_data(|d| {
                        if let Some(s) = d.seat_mut(seat) {
                            s.keyboard.repeat.key = 0;
                        }
                    });
                }
            }
            let (_, ime) = self.keyboard_input_get_text(seat, key, false);
            handled_by_ime |= ime;
        }

        let Some((mut scancode, syms)) =
            self.with_data(|d| d.seat(seat).map(|s| wayland_get_scancode_for_key(s, key)))
        else {
            return;
        };
        self.wayland_handle_modifier_keys(seat, scancode, pressed);

        // If we have a key with unknown scancode, check if the keysym corresponds to a valid Unicode value, and assign it a reserved scancode.
        if scancode == Scancode::UNKNOWN {
            if let Some(sym) = syms {
                let rebind = self.with_data(|d| {
                    let s = d.seat_mut(seat)?;
                    if s.keyboard.sdl_keymap.is_empty() {
                        return None;
                    }
                    let keycode = Keycode(crate::events::im_ks_to_ucs::key_sym_to_ucs4(sym));
                    if keycode == Keycode::UNKNOWN {
                        return None;
                    }
                    let layout = s.keyboard.xkb.current_layout as usize;
                    let map = s.keyboard.sdl_keymap.get_mut(layout)?;
                    let was_current = keymap_is_current(map);

                    // Check if this keycode already exists in the keymap.
                    let (sc, modstate) = map.scancode(keycode);
                    scancode = sc;

                    // Make sure we have this keycode in our keymap
                    if scancode == Scancode::UNKNOWN && keycode.0 < Keycode::SCANCODE_MASK {
                        scancode = map.next_reserved_scancode();
                        map.set_entry(scancode, modstate, keycode);
                        // (upstream changes the bound keymap itself; the
                        // keyboard core has a copy, updated silently)
                        if was_current {
                            return Some(map.clone());
                        }
                    }
                    None
                });
                if let Some(map) = rebind {
                    keyboard::set_keymap(Some(map), false);
                }
            }
        }

        let sdl_id = self.with_data(|d| d.seat(seat).map_or(0, |s| s.keyboard.sdl_id));
        keyboard::send_keyboard_key_ignore_modifiers(
            ns(timestamp_ns),
            sdl_id,
            key as i32,
            scancode,
            pressed,
        );

        if pressed {
            if handled_by_ime {
                has_text = None;
            }
            let has_text = has_text.filter(|t| !t.is_empty());
            if let Some(text) = &has_text {
                if !keyboard::mod_state().intersects(Keymod::CTRL | Keymod::ALT) {
                    keyboard::send_keyboard_text(text);
                }
            }
            self.with_data(|d| {
                let Some(s) = d.seat_mut(seat) else {
                    return;
                };
                if s.keyboard
                    .xkb
                    .keymap
                    .as_ref()
                    .is_some_and(|k| k.key_repeats(key + 8))
                {
                    let id = s.keyboard.sdl_id;
                    Self::keyboard_repeat_set(
                        &mut s.keyboard.repeat,
                        id,
                        key,
                        time,
                        timestamp_ns,
                        scancode,
                        has_text.as_deref(),
                    );
                }
            });
        }
    }

    /// Translation of `keyboard_handle_modifiers()`.
    fn keyboard_handle_modifiers(
        &self,
        seat: u32,
        mods_depressed: u32,
        mods_latched: u32,
        mods_locked: u32,
        group: u32,
    ) {
        let Some((new_state, has_state, layout_changed)) = self.with_data(|d| {
            let s = d.seat_mut(seat)?;
            let previous_layout = s.keyboard.xkb.current_layout;

            s.keyboard.xkb.wl_pressed_modifiers = mods_depressed;
            s.keyboard.xkb.wl_latched_modifiers = mods_latched;
            s.keyboard.xkb.wl_locked_modifiers = mods_locked;
            s.keyboard.xkb.current_layout = group;

            let new_state = wayland_reconcile_modifiers(&mut s.keyboard, false);
            Some((
                new_state,
                s.keyboard.xkb.state.is_some(),
                group != previous_layout,
            ))
        }) else {
            return;
        };
        keyboard::set_mod_state(new_state);

        // If we get a modifier notification before the keymap, there's no further state to update yet.
        if !has_state {
            return;
        }

        let repeat_key = self.with_data(|d| {
            let s = d.seat_mut(seat)?;
            if let Some(state) = &s.keyboard.xkb.state {
                state.update_mask(mods_depressed, mods_latched, mods_locked, group);
            }
            Some(s.keyboard.repeat.key).filter(|&k| k != 0)
        });

        // If a key is repeating, update the text to apply the modifier.
        if let Some(key) = repeat_key {
            if let (Some(text), _) = self.keyboard_input_get_text(seat, key, true) {
                self.with_data(|d| {
                    if let Some(s) = d.seat_mut(seat) {
                        s.keyboard.repeat.text = truncate_utf8(&text, 7).to_owned();
                    }
                });
            }
        }

        if layout_changed {
            self.wayland_seat_set_keymap(seat);

            self.with_data(|d| {
                if let Some(cs) = d
                    .seat(seat)
                    .and_then(|s| s.keyboard.xkb.compose_state.as_ref())
                {
                    // Reset the compose state so composite and dead keys don't carry over.
                    cs.reset();
                }
            });
        }
    }

    /// Translation of `Wayland_SeatDestroyPointer()`.
    fn wayland_seat_destroy_pointer(&self, seat: u32) {
        let Some(cell) = self.with_data(|d| d.seat(seat).map(|s| s.pointer.cursor_state.clone()))
        else {
            return;
        };
        self.wayland_cursor_state_release(&cell);

        // End any active gestures.
        if let Some(gesture_focus) =
            self.with_data(|d| d.seat(seat).and_then(|s| s.pointer.gesture_focus))
        {
            touch::send_pinch(
                EventType::PINCH_END,
                Duration::ZERO,
                gesture_focus,
                0.0,
                -1.0,
                -1.0,
                -1.0,
                -1.0,
            );
        }

        // Make sure focus is removed from a surface before the pointer is destroyed.
        let focus_surface = self.with_data(|d| {
            let focus = d.seat(seat)?.pointer.focus?;
            let surface = d.window(focus)?.surface_id();
            d.seat_mut(seat)?.pointer.pending_frame.leave_surface = surface;
            Some(())
        });
        if focus_surface.is_some() {
            self.pointer_dispatch_leave(seat, false);
            self.with_data(|d| {
                if let Some(s) = d.seat_mut(seat) {
                    s.pointer.pending_frame.leave_surface = 0;
                }
            });
        }

        let (sdl_id, pointer) = self.with_data(|d| {
            let Some(s) = d.seat_mut(seat) else {
                return (0, SeatPointer::default());
            };
            (s.pointer.sdl_id, std::mem::take(&mut s.pointer))
        });
        mouse::remove_mouse(sdl_id);

        // (dropping the pointer record destroys its objects in upstream's
        // order: the constraints, the relative pointer, the timestamps, the
        // gesture, and the pointer, released from version 3)
        let SeatPointer {
            wl_pointer,
            relative_pointer,
            timestamps,
            locked_pointer,
            confined_pointer,
            gesture_pinch,
            ..
        } = pointer;
        drop(confined_pointer);
        drop(locked_pointer);
        drop(relative_pointer);
        drop(timestamps);
        drop(gesture_pinch);
        drop(wl_pointer);
    }

    /// Translation of `Wayland_SeatDestroyKeyboard()`.
    fn wayland_seat_destroy_keyboard(&self, seat: u32) {
        // Make sure focus is removed from a surface before the keyboard is destroyed.
        let focus_surface = self.with_data(|d| {
            let focus = d.seat(seat)?.keyboard.focus?;
            Some(d.window(focus)?.surface_id())
        });
        if let Some(surface) = focus_surface {
            self.keyboard_handle_leave(seat, Some(surface));
        }

        let Some(kb) =
            self.with_data(|d| d.seat_mut(seat).map(|s| std::mem::take(&mut s.keyboard)))
        else {
            return;
        };

        keyboard::remove_keyboard(kb.sdl_id);

        if !kb.sdl_keymap.is_empty() && kb.xkb.current_layout < kb.xkb.num_layouts {
            if let Some(map) = kb.sdl_keymap.get(kb.xkb.current_layout as usize) {
                if keymap_is_current(map) {
                    keyboard::set_mod_state(Keymod::NONE);
                }
            }
        }

        // (dropping the keyboard record destroys the inhibitor, the
        // timestamps and the keyboard (released from version 3), then the
        // xkb objects)
        let SeatKeyboard {
            wl_keyboard,
            timestamps,
            key_inhibitor,
            xkb,
            ..
        } = kb;
        drop(key_inhibitor);
        drop(timestamps);
        drop(wl_keyboard);
        let SeatXkb {
            keymap,
            state,
            compose_table,
            compose_state,
            ..
        } = xkb;
        drop(compose_state);
        drop(compose_table);
        drop(state);
        drop(keymap);
    }

    /// Translation of `Wayland_SeatDestroyTouch()`.
    fn wayland_seat_destroy_touch(&self, seat: u32) {
        // Cancel any active touches before the touch object is destroyed.
        if self.with_data(|d| d.seat(seat).is_some_and(|s| s.touch.wl_touch.is_some())) {
            self.touch_handler_cancel(seat);
        }

        let Some(t) = self.with_data(|d| d.seat_mut(seat).map(|s| std::mem::take(&mut s.touch)))
        else {
            return;
        };
        touch::del_touch(t.touch_id());

        let SeatTouch {
            wl_touch,
            timestamps,
            ..
        } = t;
        drop(timestamps);
        // (released from version 3)
        drop(wl_touch);
    }

    /// The seat listener (`seat_listener`).
    fn handle_seat_event(&self, seat: u32, event: WlSeatEvent<'_>) {
        match event {
            WlSeatEvent::Capabilities { capabilities } => {
                self.seat_handle_capabilities(seat, capabilities)
            }
            WlSeatEvent::Name { name } => self.with_data(|d| {
                // Translation of `seat_handle_name()`.
                if let Some(s) = d.seat_mut(seat) {
                    if !name.to_bytes().is_empty() {
                        s.name = Some(name.to_string_lossy().into_owned());
                    }
                }
            }),
        }
    }

    /// Translation of `seat_handle_capabilities()`.
    fn seat_handle_capabilities(&self, seat: u32, capabilities: WlSeatCapability) {
        let (has_pointer, has_touch, has_keyboard) = self.with_data(|d| {
            d.seat(seat).map_or((false, false, false), |s| {
                (
                    s.pointer.wl_pointer.is_some(),
                    s.touch.wl_touch.is_some(),
                    s.keyboard.wl_keyboard.is_some(),
                )
            })
        });

        if capabilities.contains(WlSeatCapability::POINTER) && !has_pointer {
            let added = self.with_data(|d| {
                let s = d.seat_list.iter_mut().find(|s| s.registry_id == seat)?;
                let mut pointer = s.wl_seat.get_pointer();
                s.pointer.pending_frame.axis = PendingAxis::default();
                self.listen(&mut pointer, move |v, _, ev| {
                    v.handle_pointer_event(seat, ev)
                });
                s.pointer.wl_pointer = Some(pointer);

                Self::wayland_seat_create_cursor_shape(d, seat);

                // Pointer gestures
                self.wayland_seat_create_pointer_gestures(d, seat);

                let s = d.seat_mut(seat)?;
                s.pointer.sdl_id = crate::utils::next_object_id();

                let name = match &s.name {
                    Some(name) => format!("{WAYLAND_DEFAULT_POINTER_NAME} ({name})"),
                    None => format!("{WAYLAND_DEFAULT_POINTER_NAME} {}", s.pointer.sdl_id),
                };
                Some((s.pointer.sdl_id, name))
            });
            if let Some((id, name)) = added {
                mouse::add_mouse(id, Some(&name));
            }
        } else if !capabilities.contains(WlSeatCapability::POINTER) && has_pointer {
            self.wayland_seat_destroy_pointer(seat);
        }

        if capabilities.contains(WlSeatCapability::TOUCH) && !has_touch {
            let added = self.with_data(|d| {
                let s = d.seat_mut(seat)?;
                let mut wl_touch = s.wl_seat.get_touch();
                self.listen(&mut wl_touch, move |v, t, ev| {
                    v.handle_touch_event(seat, t, ev)
                });
                s.touch.wl_touch = Some(wl_touch);

                let name = match &s.name {
                    Some(name) => format!("{WAYLAND_DEFAULT_TOUCH_NAME} ({name})"),
                    None => format!("{WAYLAND_DEFAULT_TOUCH_NAME} {}", s.touch.touch_id()),
                };
                Some((s.touch.touch_id(), name))
            });
            if let Some((id, name)) = added {
                touch::add_touch(id, TouchDeviceType::Direct, &name);
            }
        } else if !capabilities.contains(WlSeatCapability::TOUCH) && has_touch {
            self.wayland_seat_destroy_touch(seat);
        }

        if capabilities.contains(WlSeatCapability::KEYBOARD) && !has_keyboard {
            let added = self.with_data(|d| {
                let s = d.seat_mut(seat)?;
                let mut wl_keyboard = s.wl_seat.get_keyboard();
                s.keyboard.reserved_scancodes.clear();
                self.listen(&mut wl_keyboard, move |v, _, ev| {
                    v.handle_keyboard_event(seat, ev)
                });
                s.keyboard.wl_keyboard = Some(wl_keyboard);

                s.keyboard.sdl_id = crate::utils::next_object_id();

                let name = match &s.name {
                    Some(name) => format!("{WAYLAND_DEFAULT_KEYBOARD_NAME} ({name})"),
                    None => format!("{WAYLAND_DEFAULT_KEYBOARD_NAME} {}", s.keyboard.sdl_id),
                };
                Some((s.keyboard.sdl_id, name))
            });
            if let Some((id, name)) = added {
                keyboard::add_keyboard(id, Some(&name));
            }
        } else if !capabilities.contains(WlSeatCapability::KEYBOARD) && has_keyboard {
            self.wayland_seat_destroy_keyboard(seat);
        }

        self.with_data(|d| self.wayland_seat_register_input_timestamp_listeners(d, seat));
    }

    /// The data offer listener (`data_offer_listener`).
    fn handle_data_offer_event(
        mimes: &MimeList,
        offer: Obj<'_, WlDataOffer>,
        event: WlDataOfferEvent<'_>,
    ) {
        match event {
            WlDataOfferEvent::Offer { mime_type } => {
                let mime_type = mime_type.to_string_lossy();
                wayland_offer_add_mime(mimes, &mime_type);
                crate::trace!(
                    crate::log::Category::Input,
                    ". In wl_data_offer_listener . data_offer_handle_offer on data_offer 0x{:08x} for MIME '{}'",
                    offer.id(),
                    mime_type
                );
            }
            WlDataOfferEvent::SourceActions { source_actions } => {
                crate::trace!(
                    crate::log::Category::Input,
                    ". In wl_data_offer_listener . data_offer_handle_source_actions on data_offer 0x{:08x} for Source Actions '{}'",
                    offer.id(),
                    source_actions.0
                );
            }
            WlDataOfferEvent::Action { dnd_action } => {
                crate::trace!(
                    crate::log::Category::Input,
                    ". In wl_data_offer_listener . data_offer_handle_actions on data_offer 0x{:08x} for DND Actions '{}'",
                    offer.id(),
                    dnd_action.0
                );
            }
        }
    }

    /// The data device listener (`data_device_listener`).
    fn handle_data_device_event(&self, seat: u32, event: WlDataDeviceEvent<'_>) {
        match event {
            WlDataDeviceEvent::DataOffer { id } => {
                // Translation of `data_device_handle_data_offer()`.
                let mut offer = id;
                let mimes: MimeList = Arc::default();
                let m = mimes.clone();
                offer.listen(move |o, ev| Self::handle_data_offer_event(&m, o, ev));
                crate::trace!(
                    crate::log::Category::Input,
                    ". In wl_data_device_listener . data_device_handle_data_offer on data_offer 0x{:08x}",
                    offer.obj().id()
                );
                let data_offer = DataOffer {
                    offer,
                    mimes,
                    data_device: seat,
                    callback: None,
                    read_fd: None,
                };
                self.with_data(|d| {
                    if let Some(dd) = d.seat_mut(seat).and_then(|s| s.data_device.as_mut()) {
                        dd.new_offers.push(data_offer);
                    }
                });
            }
            WlDataDeviceEvent::Enter {
                serial,
                surface,
                x,
                y,
                id,
            } => self.data_device_handle_enter(
                seat,
                serial,
                surface.map(|s| s.raw() as usize),
                x,
                y,
                id.map(|i| i.raw() as usize),
            ),
            WlDataDeviceEvent::Leave => self.data_device_handle_leave(seat),
            WlDataDeviceEvent::Motion { x, y, .. } => self.data_device_handle_motion(seat, x, y),
            WlDataDeviceEvent::Drop => self.data_device_handle_drop(seat),
            WlDataDeviceEvent::Selection { id } => {
                // Translation of `data_device_handle_selection()`.
                let id = id.map(|i| i.raw() as usize);
                let offer = self.with_data(|d| {
                    let dd = d.seat_mut(seat)?.data_device.as_mut()?;
                    let id = id?;
                    let i = dd
                        .new_offers
                        .iter()
                        .position(|o| o.offer.raw() as usize == id)?;
                    Some(dd.new_offers.remove(i))
                });

                crate::trace!(
                    crate::log::Category::Input,
                    ". In data_device_listener . data_device_handle_selection on data_offer 0x{:08x}",
                    offer.as_ref().map_or(-1i64, |o| o.offer.obj().id() as i64)
                );

                self.wayland_data_device_set_selection_offer(seat, offer);
            }
        }
    }

    /// Translation of `data_device_handle_enter()`.
    fn data_device_handle_enter(
        &self,
        seat: u32,
        serial: u32,
        surface: Option<usize>,
        x: Fixed,
        y: Fixed,
        id: Option<usize>,
    ) {
        let text_mime_types = super::clipboard::wayland_get_text_mime_types();
        let dbus_available = crate::core::linux::dbus::context().is_some();
        let position = self.with_data(|d| {
            let window = surface.and_then(|s| d.window_for_surface(s));
            let (accepts, pointer_scale, mask_surface, mask_offset) = window
                .and_then(|w| d.window(w))
                .map(|w| {
                    (
                        w.accepts_drag_and_drop,
                        w.pointer_scale,
                        w.mask.surface.as_ref().map(|m| m.raw() as usize),
                        (w.mask.offset_x, w.mask.offset_y),
                    )
                })
                .unwrap_or_default();
            let dd = d.seat_mut(seat)?.data_device.as_mut()?;
            dd.has_mime_file = false;
            dd.has_mime_text = false;

            dd.drag_serial = serial;

            // Save the drag offer so it can be freed later.
            if let Some(id) = id {
                if let Some(i) = dd.new_offers.iter().position(|o| o.offer.raw() as usize == id) {
                    dd.drag_offer = Some(dd.new_offers.remove(i));
                }
            }

            let drag_offer_matches = |dd: &DataDevice| {
                dd.drag_offer
                    .as_ref()
                    .is_some_and(|o| Some(o.offer.raw() as usize) == id)
            };

            if dd.drag_offer.is_some() && window.is_some() && accepts {
                let offer = dd.drag_offer.as_ref()?;
                // TODO: SDL Support more mime types
                if dbus_available && wayland_data_offer_has_mime(Some(offer), FILE_PORTAL_MIME) {
                    dd.has_mime_file = true;
                    dd.mime_type = Some(FILE_PORTAL_MIME);
                    if drag_offer_matches(dd) {
                        offer.offer.accept(serial, Some(FILE_PORTAL_MIME));
                    }
                }
                if wayland_data_offer_has_mime(Some(offer), FILE_MIME) {
                    dd.has_mime_file = true;
                    dd.mime_type = Some(FILE_MIME);
                    if drag_offer_matches(dd) {
                        offer.offer.accept(serial, Some(FILE_MIME));
                    }
                }

                for &mime in text_mime_types.iter() {
                    if wayland_data_offer_has_mime(Some(offer), mime) {
                        dd.has_mime_text = true;
                        dd.mime_type = Some(mime);
                        if drag_offer_matches(dd) {
                            offer.offer.accept(serial, Some(mime));
                        }
                        break;
                    }
                }

                if dd.has_mime_file || dd.has_mime_text {
                    // SDL only supports "copy" style drag and drop
                    if offer.offer.version() >= WlDataOffer::SET_ACTIONS_SINCE_VERSION {
                        offer.offer.set_actions(
                            WlDataDeviceManagerDndAction::COPY,
                            WlDataDeviceManagerDndAction::COPY,
                        );
                    }

                    // Set the destination window and send the initial position.
                    dd.dnd_window = window;
                    dd.dnd_surface = surface.unwrap_or(0);
                    let mut dx = x.to_f64();
                    let mut dy = y.to_f64();

                    // If over the mask, adjust the offset.
                    if surface.is_some() && surface == mask_surface {
                        dx += mask_offset.0 as f64;
                        dy += mask_offset.1 as f64;
                    }

                    dx *= pointer_scale.x;
                    dy *= pointer_scale.y;

                    crate::trace!(
                        crate::log::Category::Input,
                        ". In wl_data_device_listener . data_device_handle_enter on data_offer 0x{:08x} at {} x {} into window {} for serial {}",
                        offer.offer.obj().id(),
                        x.to_int(),
                        y.to_int(),
                        window.unwrap_or(0),
                        serial
                    );
                    return Some((window, dx as f32, dy as f32));
                } else {
                    // Decline the offer.
                    if drag_offer_matches(dd) {
                        offer.offer.accept(serial, None);
                    }
                    if offer.offer.version() >= WlDataOffer::SET_ACTIONS_SINCE_VERSION {
                        offer.offer.set_actions(
                            WlDataDeviceManagerDndAction::NONE,
                            WlDataDeviceManagerDndAction::NONE,
                        );
                    }

                    crate::trace!(
                        crate::log::Category::Input,
                        ". In wl_data_device_listener . data_device_handle_enter on data_offer 0x{:08x} at {} x {} for serial {}",
                        offer.offer.obj().id(),
                        x.to_int(),
                        y.to_int(),
                        serial
                    );
                }
            } else {
                dd.dnd_window = None;
                dd.dnd_surface = 0;

                // Decline the offer.
                if id.is_some() {
                    if let Some(offer) = dd.drag_offer.as_ref() {
                        if drag_offer_matches(dd) {
                            offer.offer.accept(serial, None);
                        }
                        if offer.offer.version() >= WlDataOffer::SET_ACTIONS_SINCE_VERSION {
                            offer.offer.set_actions(
                                WlDataDeviceManagerDndAction::NONE,
                                WlDataDeviceManagerDndAction::NONE,
                            );
                        }
                    }
                }
                crate::trace!(
                    crate::log::Category::Input,
                    ". In wl_data_device_listener . data_device_handle_enter on data_offer 0x{:08x} at {} x {} for serial {}",
                    dd.drag_offer.as_ref().map_or(-1i64, |o| o.offer.obj().id() as i64),
                    x.to_int(),
                    y.to_int(),
                    serial
                );
            }
            None
        });

        if let Some((window, dx, dy)) = position {
            crate::events::window::send_drop_position(window, dx, dy);
        }
    }

    /// Translation of `data_device_handle_leave()`.
    fn data_device_handle_leave(&self, seat: u32) {
        let (offer, dnd_window) = self
            .with_data(|d| {
                let dd = d.seat_mut(seat)?.data_device.as_mut()?;
                let offer = dd.drag_offer.take();
                let w = dd.dnd_window;
                dd.has_mime_file = false;
                dd.has_mime_text = false;
                Some((offer, w))
            })
            .unwrap_or((None, None));

        if let Some(offer) = offer {
            if let Some(window) = dnd_window {
                crate::events::window::send_drop_complete(Some(window));
                crate::trace!(
                    crate::log::Category::Input,
                    ". In wl_data_device_listener . data_device_handle_leave on data_offer 0x{:08x} from window {}",
                    offer.offer.obj().id(),
                    window
                );
            } else {
                crate::trace!(
                    crate::log::Category::Input,
                    ". In wl_data_device_listener . data_device_handle_leave on data_offer 0x{:08x}",
                    offer.offer.obj().id()
                );
            }
            drop(offer);
        } else {
            crate::trace!(
                crate::log::Category::Input,
                ". In wl_data_device_listener . data_device_handle_leave on data_offer 0x{:08x} for serial {}",
                -1,
                -1
            );
        }
    }

    /// Translation of `data_device_handle_motion()`.
    fn data_device_handle_motion(&self, seat: u32, x: Fixed, y: Fixed) {
        let position = self.with_data(|d| {
            let dd = d.seat(seat)?.data_device.as_ref()?;
            dd.drag_offer.as_ref()?;
            let window = dd.dnd_window?;
            if !(dd.has_mime_file || dd.has_mime_text) {
                return None;
            }
            let dnd_surface = dd.dnd_surface;
            let wd = d.window(window)?;
            let mut dx = x.to_f64();
            let mut dy = y.to_f64();

            // If over the mask, adjust the offset.
            if wd
                .mask
                .surface
                .as_ref()
                .is_some_and(|m| m.raw() as usize == dnd_surface)
            {
                dx += wd.mask.offset_x as f64;
                dy += wd.mask.offset_y as f64;
            }

            dx *= wd.pointer_scale.x;
            dy *= wd.pointer_scale.y;
            Some((window, dx as f32, dy as f32))
        });

        /* XXX: Send the filename here if the event system ever starts passing it though.
         *      Any future implementation should cache the filenames, as otherwise this could
         *      hammer the DBus interface hundreds or even thousands of times per second.
         */
        if let Some((window, dx, dy)) = position {
            crate::events::window::send_drop_position(Some(window), dx, dy);
        }
    }

    /// Translation of `data_device_handle_drop()`.
    fn data_device_handle_drop(&self, seat: u32) {
        let state = self.with_data(|d| {
            let dd = d.seat(seat)?.data_device.as_ref()?;
            dd.drag_offer.as_ref()?;
            let window = dd.dnd_window?;
            if !(dd.has_mime_file || dd.has_mime_text) {
                return None;
            }
            Some((window, dd.mime_type, dd.has_mime_file, dd.has_mime_text))
        });

        if let Some((window, mime_type, has_mime_file, has_mime_text)) = state {
            // TODO: SDL Support more mime types
            let mut drop_handled = false;

            let receive = |mime: &str| {
                self.with_data(|d| {
                    let offer = d.seat(seat)?.data_device.as_ref()?.drag_offer.as_ref();
                    if !wayland_data_offer_has_mime(offer, mime) {
                        return None;
                    }
                    self.wayland_data_offer_receive(offer, mime, false)
                        .ok()
                        .flatten()
                })
            };

            if crate::core::linux::dbus::context().is_some() {
                if let Some(buffer) = receive(FILE_PORTAL_MIME) {
                    let key = String::from_utf8_lossy(&buffer).into_owned();
                    let paths = crate::core::linux::dbus::documents_portal_retrieve_files(&key)
                        .unwrap_or_default();
                    // If dropped files contain a directory the list is empty
                    if !paths.is_empty() {
                        for path in &paths {
                            crate::events::window::send_drop_file(Some(window), None, path);
                        }
                        crate::events::window::send_drop_complete(Some(window));
                        drop_handled = true;
                    }
                }
            }

            /* If XDG document portal fails fallback.
             * When running a flatpak sandbox this will most likely be a list of
             * non paths that are not visible to the application
             */
            if !drop_handled {
                let buffer = self.with_data(|d| {
                    let offer = d.seat(seat)?.data_device.as_ref()?.drag_offer.as_ref();
                    self.wayland_data_offer_receive(offer, mime_type.unwrap_or(""), false)
                        .ok()
                        .flatten()
                });
                if has_mime_file {
                    if let Some(buffer) = buffer {
                        let text = String::from_utf8_lossy(&buffer).into_owned();
                        for token in text.split(['\r', '\n']).filter(|t| !t.is_empty()) {
                            if let Some(local) = crate::utils::uri_to_local(token) {
                                let local = String::from_utf8_lossy(&local).into_owned();
                                crate::events::window::send_drop_file(Some(window), None, &local);
                            }
                        }
                    }
                    crate::events::window::send_drop_complete(Some(window));
                    drop_handled = true;
                } else if has_mime_text {
                    if let Some(buffer) = buffer {
                        let text = String::from_utf8_lossy(&buffer).into_owned();
                        for token in text.split(['\r', '\n']).filter(|t| !t.is_empty()) {
                            crate::events::window::send_drop_text(Some(window), token);
                        }
                    }
                    /* Even though there has been a valid data offer,
                     *  and there have been valid Enter, Motion, and Drop callbacks,
                     *  Wayland_data_offer_receive may return an empty buffer,
                     *  because the data is actually in the primary selection device,
                     *  not in the data device.
                     */
                    crate::events::window::send_drop_complete(Some(window));
                    drop_handled = true;
                }
            }

            if drop_handled {
                self.with_data(|d| {
                    if let Some(offer) = d
                        .seat(seat)
                        .and_then(|s| s.data_device.as_ref())
                        .and_then(|dd| dd.drag_offer.as_ref())
                    {
                        if offer.offer.version() >= WlDataOffer::FINISH_SINCE_VERSION {
                            offer.offer.finish();
                        }
                    }
                });
            }
        } else {
            crate::trace!(
                crate::log::Category::Input,
                ". In wl_data_device_listener . data_device_handle_drop on data_offer 0x{:08x} serial {}",
                -1,
                -1
            );
        }

        let offer = self.with_data(|d| d.seat_mut(seat)?.data_device.as_mut()?.drag_offer.take());
        drop(offer);
    }

    /// The primary selection device listener
    /// (`primary_selection_device_listener`).
    fn handle_primary_selection_device_event(
        &self,
        seat: u32,
        event: ZwpPrimarySelectionDeviceV1Event<'_>,
    ) {
        match event {
            ZwpPrimarySelectionDeviceV1Event::DataOffer { offer } => {
                // Translation of `primary_selection_device_handle_offer()`.
                let mut id = offer;
                let mimes: MimeList = Arc::default();
                let m = mimes.clone();
                id.listen(move |o, ev| {
                    // Translation of `primary_selection_offer_handle_offer()`.
                    let ZwpPrimarySelectionOfferV1Event::Offer { mime_type } = ev;
                    let mime_type = mime_type.to_string_lossy();
                    wayland_offer_add_mime(&m, &mime_type);
                    crate::trace!(
                        crate::log::Category::Input,
                        ". In zwp_primary_selection_offer_v1_listener . primary_selection_offer_handle_offer on primary_selection_offer 0x{:08x} for MIME '{}'",
                        o.id(),
                        mime_type
                    );
                });
                crate::trace!(
                    crate::log::Category::Input,
                    ". In zwp_primary_selection_device_v1_listener . primary_selection_device_handle_offer on primary_selection_offer 0x{:08x}",
                    id.obj().id()
                );
                let offer = PrimarySelectionOffer {
                    offer: id,
                    mimes,
                    primary_selection_device: seat,
                };
                self.with_data(|d| {
                    d.current_primary_selection_seat = Some(seat);
                    if let Some(pd) = d
                        .seat_mut(seat)
                        .and_then(|s| s.primary_selection_device.as_mut())
                    {
                        pd.new_offers.push(offer);
                    }
                });
            }
            ZwpPrimarySelectionDeviceV1Event::Selection { id } => {
                // Translation of `primary_selection_device_handle_selection()`.
                let id = id.map(|i| i.raw() as usize);
                let old = self.with_data(|d| {
                    let pd = d.seat_mut(seat)?.primary_selection_device.as_mut()?;
                    let current = pd.selection_offer.as_ref().map(|o| o.offer.raw() as usize);
                    if current == id {
                        return None;
                    }
                    let offer = id.and_then(|id| {
                        let i = pd
                            .new_offers
                            .iter()
                            .position(|o| o.offer.raw() as usize == id)?;
                        Some(pd.new_offers.remove(i))
                    });
                    std::mem::replace(&mut pd.selection_offer, offer)
                });
                drop(old);
                crate::trace!(
                    crate::log::Category::Input,
                    ". In zwp_primary_selection_device_v1_listener . primary_selection_device_handle_selection on primary_selection_offer 0x{:08x}",
                    id.unwrap_or(0)
                );
            }
        }
    }

    /// The text input listener (`text_input_listener`).
    fn handle_text_input_event(&self, seat: u32, event: ZwpTextInputV3Event<'_>) {
        match event {
            ZwpTextInputV3Event::Enter { .. } => {
                // No-op
            }
            ZwpTextInputV3Event::Leave { .. } => {
                // No-op
            }
            ZwpTextInputV3Event::PreeditString {
                text,
                cursor_begin,
                cursor_end,
            } => {
                // Translation of `text_input_preedit_string()`.
                self.with_data(|d| {
                    if let Some(s) = d.seat_mut(seat) {
                        s.text_input.has_preedit = true;
                    }
                });
                if let Some(text) = text {
                    let bytes = text.to_bytes();
                    let cursor_begin_utf8 = if cursor_begin >= 0 {
                        crate::stdlib::string::utf8strnlen(bytes, cursor_begin as usize) as i32
                    } else {
                        -1
                    };
                    let cursor_end_utf8 = if cursor_end >= 0 {
                        crate::stdlib::string::utf8strnlen(bytes, cursor_end as usize) as i32
                    } else {
                        -1
                    };
                    let cursor_size_utf8 = if cursor_end_utf8 >= 0 {
                        if cursor_begin_utf8 >= 0 {
                            cursor_end_utf8 - cursor_begin_utf8
                        } else {
                            cursor_end_utf8
                        }
                    } else {
                        -1
                    };
                    keyboard::send_editing_text(
                        &text.to_string_lossy(),
                        cursor_begin_utf8,
                        cursor_size_utf8,
                    );
                } else {
                    keyboard::send_editing_text("", 0, 0);
                }
            }
            ZwpTextInputV3Event::CommitString { text } => {
                // Translation of `text_input_commit_string()`.
                // FIXME (upstream): a commit with a NULL text (allowed by
                // the protocol) is passed on to SDL_SendKeyboardText(),
                // which dereferences it; nothing is sent here.
                if let Some(text) = text {
                    keyboard::send_keyboard_text(&text.to_string_lossy());
                }
            }
            ZwpTextInputV3Event::DeleteSurroundingText { .. } => {
                // FIXME: Do we care about this event?
            }
            ZwpTextInputV3Event::Done { .. } => {
                // Translation of `text_input_done()`.
                let had_preedit = self.with_data(|d| {
                    d.seat_mut(seat)
                        .map(|s| std::mem::replace(&mut s.text_input.has_preedit, false))
                });
                if had_preedit == Some(false) {
                    keyboard::send_editing_text("", 0, 0);
                }
            }
        }
    }

    /// Translation of `Wayland_DataDeviceSetID()`.
    fn wayland_data_device_set_id() -> String {
        if let Some(ctx) = crate::core::linux::dbus::context() {
            if let Some(id) = ctx.session_conn.unique_name() {
                return id;
            }
        }
        // SAFETY: getpid() has no preconditions.
        let pid = unsafe { libc::getpid() } as u64;
        pid.to_string()
    }

    /// Translation of `Wayland_SeatCreateDataDevice()`.
    fn wayland_seat_create_data_device(&self, d: &mut VideoData, seat: u32) {
        let Some(manager) = d.g.data_device_manager.as_ref() else {
            return;
        };
        let Some(s) = d.seat_list.iter_mut().find(|s| s.registry_id == seat) else {
            return;
        };

        let mut data_device = manager.get_data_device(s.wl_seat.obj());
        self.listen(&mut data_device, move |v, _, ev| {
            v.handle_data_device_event(seat, ev)
        });
        s.data_device = Some(DataDevice {
            data_device,
            seat,
            id_str: Self::wayland_data_device_set_id(),
            drag_serial: 0,
            drag_offer: None,
            selection_offer: None,
            new_offers: Vec::new(),
            mime_type: None,
            has_mime_file: false,
            has_mime_text: false,
            dnd_window: None,
            dnd_surface: 0,
            selection_serial: 0,
            selection_source: None,
        });
    }

    /// Translation of `Wayland_DisplayInitDataDeviceManager()`.
    pub(crate) fn wayland_display_init_data_device_manager(&self, d: &mut VideoData) {
        let seats: Vec<u32> = d.seat_list.iter().map(|s| s.registry_id).collect();
        for seat in seats {
            self.wayland_seat_create_data_device(d, seat);
        }
    }

    /// Translation of `Wayland_SeatCreatePrimarySelectionDevice()`.
    fn wayland_seat_create_primary_selection_device(&self, d: &mut VideoData, seat: u32) {
        let Some(manager) = d.g.primary_selection_device_manager.as_ref() else {
            return;
        };
        let Some(s) = d.seat_list.iter_mut().find(|s| s.registry_id == seat) else {
            return;
        };

        let mut device = manager.get_device(s.wl_seat.obj());
        self.listen(&mut device, move |v, _, ev| {
            v.handle_primary_selection_device_event(seat, ev)
        });
        s.primary_selection_device = Some(PrimarySelectionDevice {
            primary_selection_device: device,
            seat,
            selection_serial: 0,
            selection_source: None,
            selection_offer: None,
            new_offers: Vec::new(),
        });
    }

    /// Translation of `Wayland_DisplayInitPrimarySelectionDeviceManager()`.
    pub(crate) fn wayland_display_init_primary_selection_device_manager(&self, d: &mut VideoData) {
        let seats: Vec<u32> = d.seat_list.iter().map(|s| s.registry_id).collect();
        for seat in seats {
            self.wayland_seat_create_primary_selection_device(d, seat);
        }
    }

    /// Translation of `Wayland_SeatCreateTextInput()`.
    fn wayland_seat_create_text_input(&self, d: &mut VideoData, seat: u32) {
        let Some(manager) = d.g.text_input_manager.as_ref() else {
            return;
        };
        let Some(s) = d.seat_list.iter_mut().find(|s| s.registry_id == seat) else {
            return;
        };
        let mut text_input = manager.get_text_input(s.wl_seat.obj());
        self.listen(&mut text_input, move |v, _, ev| {
            v.handle_text_input_event(seat, ev)
        });
        s.text_input.zwp_text_input = Some(text_input);
    }

    /// Translation of `Wayland_DisplayInitTextInputManager()`.
    pub(crate) fn wayland_display_init_text_input_manager(&self, d: &mut VideoData, _id: u32) {
        let seats: Vec<u32> = d.seat_list.iter().map(|s| s.registry_id).collect();
        for seat in seats {
            self.wayland_seat_create_text_input(d, seat);
        }
    }

    // Pen/Tablet support...

    /// The tablet tool listener (`tablet_tool_listener`).
    fn handle_tablet_tool_event(&self, seat: u32, tool: usize, event: ZwpTabletToolV2Event<'_>) {
        match event {
            ZwpTabletToolV2Event::Type { tool_type } => self.with_tool(seat, tool, |t| {
                // Translation of `tablet_tool_handle_type()`.
                t.info.subtype = match tool_type {
                    ZwpTabletToolV2Type::ERASER => PenSubtype::Eraser,
                    ZwpTabletToolV2Type::PEN => PenSubtype::Pen,
                    ZwpTabletToolV2Type::PENCIL => PenSubtype::Pencil,
                    ZwpTabletToolV2Type::AIRBRUSH => PenSubtype::Airbrush,
                    ZwpTabletToolV2Type::BRUSH => PenSubtype::Brush,
                    _ => PenSubtype::Unknown, // we'll decline to add this when the `done` event comes through.
                };
            }),
            ZwpTabletToolV2Event::HardwareSerial { .. } => {
                // don't care about this atm.
            }
            ZwpTabletToolV2Event::HardwareIdWacom { hardware_id_lo, .. } => {
                self.with_tool(seat, tool, |t| t.info.wacom_id = hardware_id_lo)
            }
            ZwpTabletToolV2Event::Capability { capability } => self.with_tool(seat, tool, |t| {
                // Translation of `tablet_tool_handle_capability()`.
                let c = match capability {
                    ZwpTabletToolV2Capability::TILT => {
                        PenCapabilityFlags::XTILT.0 | PenCapabilityFlags::YTILT.0
                    }
                    ZwpTabletToolV2Capability::PRESSURE => PenCapabilityFlags::PRESSURE.0,
                    ZwpTabletToolV2Capability::DISTANCE => PenCapabilityFlags::DISTANCE.0,
                    ZwpTabletToolV2Capability::ROTATION => PenCapabilityFlags::ROTATION.0,
                    ZwpTabletToolV2Capability::SLIDER => PenCapabilityFlags::SLIDER.0,
                    _ => 0, // unsupported here.
                };
                t.info.capabilities.0 |= c;
            }),
            ZwpTabletToolV2Event::Done => {
                // Translation of `tablet_tool_handle_done()`.
                let Some((info, focus)) = self.with_data(|d| {
                    let t = d.seat(seat)?.tool(tool)?;
                    Some((t.info, t.focus))
                }) else {
                    return;
                };
                if info.subtype != PenSubtype::Unknown {
                    // don't tell SDL about it if we don't know its role.
                    let id = pen::add_pen_device(
                        Duration::ZERO,
                        None,
                        focus,
                        Some(&info),
                        Arc::new(tool),
                        false,
                    );
                    self.with_tool(seat, tool, |t| t.instance_id = id);
                }
            }
            ZwpTabletToolV2Event::Removed => self.tablet_tool_handle_removed(seat, tool),
            ZwpTabletToolV2Event::ProximityIn {
                serial, surface, ..
            } => {
                // Translation of `tablet_tool_handle_proximity_in()`.
                let surface = surface.map(|s| s.raw() as usize);
                self.with_data(|d| {
                    let windowdata = surface.and_then(|s| d.window_for_surface(s));
                    let focus =
                        windowdata.filter(|&w| d.window(w).map(|w| w.surface_id()) == surface);
                    if let Some(t) = d.seat_mut(seat).and_then(|s| s.tool_mut(tool)) {
                        t.focus = focus;
                        t.proximity_serial = serial;
                        t.frame.have_proximity = true;
                        t.frame.in_proximity = true;
                    }
                });
                // According to the docs, this should be followed by a frame event, where we'll send our SDL events.
            }
            ZwpTabletToolV2Event::ProximityOut => self.with_tool(seat, tool, |t| {
                t.frame.have_proximity = true;
                t.frame.in_proximity = false;
            }),
            ZwpTabletToolV2Event::Down { .. } => {
                self.with_tool(seat, tool, |t| t.frame.tool_state = ToolState::Down)
            }
            ZwpTabletToolV2Event::Up => {
                self.with_tool(seat, tool, |t| t.frame.tool_state = ToolState::Up)
            }
            ZwpTabletToolV2Event::Motion { x, y } => self.with_data(|d| {
                // Translation of `tablet_tool_handle_motion()`.
                let Some(focus) = d
                    .seat(seat)
                    .and_then(|s| s.tool(tool))
                    .and_then(|t| t.focus)
                else {
                    return;
                };
                let Some(scale) = d.window(focus).map(|w| w.pointer_scale) else {
                    return;
                };
                if let Some(t) = d.seat_mut(seat).and_then(|s| s.tool_mut(tool)) {
                    t.frame.x = (x.to_f64() * scale.x) as f32;
                    t.frame.y = (y.to_f64() * scale.y) as f32;
                    t.frame.have_motion = true;
                }
            }),
            ZwpTabletToolV2Event::Pressure { pressure } => self.with_tool(seat, tool, |t| {
                t.frame.axes[PenAxis::Pressure as usize] = pressure as f32 / 65535.0;
                t.frame.axes_set |= 1 << PenAxis::Pressure as u32;
                if pressure != 0 {
                    t.frame.axes[PenAxis::Distance as usize] = 0.0;
                    t.frame.axes_set |= 1 << PenAxis::Distance as u32;
                }
            }),
            ZwpTabletToolV2Event::Distance { distance } => self.with_tool(seat, tool, |t| {
                t.frame.axes[PenAxis::Distance as usize] = distance as f32 / 65535.0;
                t.frame.axes_set |= 1 << PenAxis::Distance as u32;
                if distance != 0 {
                    t.frame.axes[PenAxis::Pressure as usize] = 0.0;
                    t.frame.axes_set |= 1 << PenAxis::Pressure as u32;
                }
            }),
            ZwpTabletToolV2Event::Tilt { tilt_x, tilt_y } => self.with_tool(seat, tool, |t| {
                t.frame.axes[PenAxis::XTilt as usize] = tilt_x.to_f64() as f32;
                t.frame.axes[PenAxis::YTilt as usize] = tilt_y.to_f64() as f32;
                t.frame.axes_set |= (1 << PenAxis::XTilt as u32) | (1 << PenAxis::YTilt as u32);
            }),
            ZwpTabletToolV2Event::Rotation { degrees } => self.with_tool(seat, tool, |t| {
                let rotation = degrees.to_f64() as f32;
                // map to -180.0f ... 179.0f range
                t.frame.axes[PenAxis::Rotation as usize] = if rotation > 180.0 {
                    rotation - 360.0
                } else {
                    rotation
                };
                t.frame.axes_set |= 1 << PenAxis::Rotation as u32;
            }),
            ZwpTabletToolV2Event::Slider { position } => self.with_tool(seat, tool, |t| {
                t.frame.axes[PenAxis::Slider as usize] = position as f32 / 65535.0;
                t.frame.axes_set |= 1 << PenAxis::Slider as u32;
            }),
            ZwpTabletToolV2Event::Wheel { .. } => {
                // not supported at the moment
            }
            ZwpTabletToolV2Event::Button { button, state, .. } => self.with_tool(seat, tool, |t| {
                // Translation of `tablet_tool_handle_button()`.
                let sdlbutton = match button {
                    BTN_STYLUS => 1,
                    BTN_STYLUS2 => 2,
                    BTN_STYLUS3 => 3,
                    _ => return, // don't care about this button, I guess.
                };

                crate::sdl_assert!((1..=t.frame.buttons.len()).contains(&sdlbutton));
                t.frame.buttons[sdlbutton - 1] = if state.0 == ZwpTabletPadV2ButtonState::PRESSED.0
                {
                    ToolButtonState::Down
                } else {
                    ToolButtonState::Up
                };
            }),
            ZwpTabletToolV2Event::Frame { time } => self.tablet_tool_handle_frame(seat, tool, time),
        }
    }

    /// Run `f` on a tool of a seat.
    fn with_tool(&self, seat: u32, tool: usize, f: impl FnOnce(&mut WaylandPenTool)) {
        self.with_data(|d| {
            if let Some(t) = d.seat_mut(seat).and_then(|s| s.tool_mut(tool)) {
                f(t);
            }
        });
    }

    /// Translation of `tablet_tool_handle_removed()`.
    fn tablet_tool_handle_removed(&self, seat: u32, tool: usize) {
        let Some(t) = self.with_data(|d| {
            let s = d.seat_mut(seat)?;
            let i = s.tablet.tool_list.iter().position(|t| t.key() == tool)?;
            Some(s.tablet.tool_list.remove(i))
        }) else {
            return;
        };
        if t.instance_id != 0 {
            pen::remove_pen_device(Duration::ZERO, t.focus, t.instance_id);
        }

        self.wayland_cursor_state_release(&t.cursor_state);
        drop(t);
    }

    /// Translation of `tablet_tool_handle_frame()`.
    fn tablet_tool_handle_frame(&self, seat: u32, tool: usize, time: u32) {
        let Some((instance_id, window, subtype, frame)) = self.with_data(|d| {
            let t = d.seat(seat)?.tool(tool)?;
            Some((t.instance_id, t.focus, t.info.subtype, t.frame))
        }) else {
            return;
        };
        if instance_id == 0 {
            return; // Not a pen we report on.
        }

        let timestamp = ns(wayland_adjust_event_timestamp_base(
            wayland_event_timestamp_ms_to_ns(time),
        ));
        let is_eraser = subtype == PenSubtype::Eraser;

        if frame.have_proximity && frame.in_proximity {
            pen::send_pen_proximity(timestamp, instance_id, window, true, true);
            self.wayland_tablet_tool_update_cursor(seat, tool);
        }

        // !!! FIXME: Should hit testing be done if pens generate pointer motion?

        // I don't know if this is necessary (or makes sense), but send motion before pen downs, but after pen ups, so you don't get unexpected lines drawn.
        if frame.have_motion && frame.tool_state != ToolState::None {
            if frame.tool_state == ToolState::Down {
                pen::send_pen_motion(timestamp, instance_id, window, frame.x, frame.y);
                pen::send_pen_touch(timestamp, instance_id, window, is_eraser, true);
            } else {
                pen::send_pen_touch(timestamp, instance_id, window, is_eraser, false);
                pen::send_pen_motion(timestamp, instance_id, window, frame.x, frame.y);
            }
        } else {
            if frame.tool_state != ToolState::None {
                pen::send_pen_touch(
                    timestamp,
                    instance_id,
                    window,
                    is_eraser,
                    frame.tool_state == ToolState::Down,
                );
            }

            if frame.have_motion {
                pen::send_pen_motion(timestamp, instance_id, window, frame.x, frame.y);
            }
        }

        for axis in PenAxis::ALL {
            if frame.axes_set & (1u32 << axis as u32) != 0 {
                pen::send_pen_axis(
                    timestamp,
                    instance_id,
                    window,
                    axis,
                    frame.axes[axis as usize],
                );
            }
        }

        for (i, state) in frame.buttons.iter().enumerate() {
            if *state != ToolButtonState::None {
                pen::send_pen_button(
                    timestamp,
                    instance_id,
                    window,
                    (i + 1) as u8,
                    *state == ToolButtonState::Down,
                );
            }
        }

        if frame.have_proximity && !frame.in_proximity {
            pen::send_pen_proximity(timestamp, instance_id, window, false, false);
            self.with_tool(seat, tool, |t| t.focus = None);
            self.wayland_tablet_tool_update_cursor(seat, tool);
        }

        // Reset for the next frame.
        self.with_tool(seat, tool, |t| t.frame = ToolFrame::default());
    }

    /// The tablet seat listener (`tablet_seat_listener`).
    fn handle_tablet_seat_event(&self, seat: u32, event: ZwpTabletSeatV2Event) {
        match event {
            ZwpTabletSeatV2Event::TabletAdded { id } => {
                // don't care atm.
                drop(id);
            }
            ZwpTabletSeatV2Event::ToolAdded { id } => {
                // Translation of `tablet_seat_handle_tool_added()`.
                let mut wltool = id;
                let key = wltool.raw() as usize;
                let cursor_state: CursorStateCell = Arc::new(Mutex::new(CursorState::default()));
                self.with_data(|d| {
                    if let Some(manager) = &d.g.cursor_shape_manager {
                        super::mouse::lock_state(&cursor_state).cursor_shape =
                            Some(manager.get_tablet_tool_v2(wltool.obj()));
                    }

                    // this will send a bunch of zwp_tablet_tool_v2 events right up front to tell
                    // us device details, with a "done" event to let us know we have everything.
                    self.listen(&mut wltool, move |v, _, ev| {
                        v.handle_tablet_tool_event(seat, key, ev)
                    });

                    let sdltool = WaylandPenTool {
                        instance_id: 0,
                        info: PenInfo {
                            max_tilt: -1,
                            num_buttons: -1,
                            ..PenInfo::default()
                        },
                        focus: None,
                        wltool,
                        proximity_serial: 0,
                        frame: ToolFrame::default(),
                        cursor_state,
                    };
                    if let Some(s) = d.seat_mut(seat) {
                        s.tablet.tool_list.insert(0, sdltool);
                    }
                });
            }
            ZwpTabletSeatV2Event::PadAdded { id } => {
                // we don't care atm.
                drop(id);
            }
        }
    }

    /// Translation of `Wayland_SeatInitTabletSupport()`.
    fn wayland_seat_init_tablet_support(&self, d: &mut VideoData, seat: u32) {
        let Some(manager) = d.g.tablet_manager.as_ref() else {
            return;
        };
        let Some(s) = d.seat_list.iter_mut().find(|s| s.registry_id == seat) else {
            return;
        };
        let mut tablet_seat = manager.get_tablet_seat(s.wl_seat.obj());
        self.listen(&mut tablet_seat, move |v, _, ev| {
            v.handle_tablet_seat_event(seat, ev)
        });
        s.tablet.wl_tablet_seat = Some(tablet_seat);
    }

    /// Translation of `Wayland_DisplayInitTabletManager()`.
    pub(crate) fn wayland_display_init_tablet_manager(&self, d: &mut VideoData) {
        let seats: Vec<u32> = d.seat_list.iter().map(|s| s.registry_id).collect();
        for seat in seats {
            self.wayland_seat_init_tablet_support(d, seat);
        }
    }

    /// Translation of `Wayland_SeatDestroyTablet()`.
    fn wayland_seat_destroy_tablet(&self, seat: u32, shutting_down: bool) {
        if !shutting_down {
            let tools: Vec<usize> = self.with_data(|d| {
                d.seat(seat)
                    .map(|s| s.tablet.tool_list.iter().map(|t| t.key()).collect())
                    .unwrap_or_default()
            });
            for tool in tools {
                // Remove all tools for this seat, sending PROXIMITY_OUT events.
                self.tablet_tool_handle_removed(seat, tool);
            }
        } else {
            // Shutting down, just delete everything.
            // (Wayland_remove_all_pens_callback(): the tools are released
            // with the seat below)
            pen::remove_all_pen_devices(|_, _| {});
            let tools = self.with_data(|d| {
                d.seat_mut(seat)
                    .map(|s| std::mem::take(&mut s.tablet.tool_list))
                    .unwrap_or_default()
            });
            for t in tools {
                self.wayland_cursor_state_release(&t.cursor_state);
                drop(t);
            }
        }

        let tablet = self.with_data(|d| d.seat_mut(seat).map(|s| std::mem::take(&mut s.tablet)));
        drop(tablet);
    }

    /// Translation of `Wayland_DisplayCreateSeat()`.
    pub(crate) fn wayland_display_create_seat(
        &self,
        display: &mut VideoData,
        mut wl_seat: Proxy<WlSeat>,
        id: u32,
    ) {
        self.listen(&mut wl_seat, move |v, _, ev| v.handle_seat_event(id, ev));

        let seat = WaylandSeat {
            wl_seat,
            data_device: None,
            primary_selection_device: None,
            name: None,
            last_implicit_grab_serial: 0,
            registry_id: id,
            keyboard: SeatKeyboard::default(),
            pointer: SeatPointer {
                cursor_state: Arc::new(Mutex::new(CursorState::default())),
                ..SeatPointer::default()
            },
            touch: SeatTouch::default(),
            text_input: SeatTextInput::default(),
            tablet: SeatTablet::default(),
        };

        // Keep the seats in the order in which they were added.
        display.seat_list.push(seat);

        self.wayland_seat_create_data_device(display, id);
        self.wayland_seat_create_primary_selection_device(display, id);
        self.wayland_seat_create_text_input(display, id);

        if display.g.tablet_manager.is_some() {
            self.wayland_seat_init_tablet_support(display, id);
        }
    }

    /// Translation of `Wayland_DisplayRemoveWindowReferencesFromSeats()`.
    pub(crate) fn wayland_display_remove_window_references_from_seats(&self, window: WindowID) {
        let Some(surface) = self.with_data(|d| d.window(window).map(|w| w.surface_id())) else {
            return;
        };
        let seats: Vec<u32> =
            self.with_data(|d| d.seat_list.iter().map(|s| s.registry_id).collect());
        for seat in seats {
            let (keyboard_focus, pointer_focus) = self.with_data(|d| {
                d.seat(seat).map_or((false, false), |s| {
                    (
                        s.keyboard.focus == Some(window),
                        s.pointer.focus == Some(window),
                    )
                })
            });
            if keyboard_focus {
                self.keyboard_handle_leave(seat, Some(surface));
            }

            if pointer_focus {
                self.with_data(|d| {
                    if let Some(s) = d.seat_mut(seat) {
                        s.pointer.pending_frame.leave_surface = surface;
                    }
                });
                self.pointer_dispatch_leave(seat, true);
                self.with_data(|d| {
                    if let Some(s) = d.seat_mut(seat) {
                        s.pointer.pending_frame.leave_surface = 0;
                    }
                });
            }

            // (cancelling a touch point removes it from the list)
            let cancelled = self.with_data(|d| {
                let s = d.seat_mut(seat)?;
                let (cancel, keep): (Vec<TouchPoint>, Vec<TouchPoint>) =
                    s.touch.points.iter().partition(|tp| tp.surface == surface);
                s.touch.points = keep;
                Some(cancel)
            });
            for tp in cancelled.unwrap_or_default() {
                self.wayland_seat_cancel_touch(seat, tp);
            }

            let tools: Vec<(usize, PenID)> = self.with_data(|d| {
                d.seat(seat)
                    .map(|s| {
                        s.tablet
                            .tool_list
                            .iter()
                            .filter(|t| t.focus == Some(window))
                            .map(|t| (t.key(), t.instance_id))
                            .collect()
                    })
                    .unwrap_or_default()
            });
            for (tool, instance_id) in tools {
                self.with_tool(seat, tool, |t| t.focus = None);
                self.wayland_tablet_tool_update_cursor(seat, tool);
                if instance_id != 0 {
                    pen::remove_pen_device(Duration::ZERO, Some(window), instance_id);
                    self.with_tool(seat, tool, |t| t.instance_id = 0);
                }
            }
        }
    }

    /// Translation of `Wayland_SeatDestroy()`.
    pub(crate) fn wayland_seat_destroy(&self, seat: u32, shutting_down: bool) {
        // (the data devices: the offers and sources go first)
        let devices = self.with_data(|d| {
            let s = d.seat_mut(seat)?;
            s.name = None;
            Some((
                s.data_device.take(),
                s.primary_selection_device.take(),
                s.text_input.zwp_text_input.take(),
            ))
        });
        let Some((data_device, primary_selection_device, text_input)) = devices else {
            return;
        };

        if let Some(mut dd) = data_device {
            dd.selection_offer = None;
            if let Some(source) = dd.selection_source.take() {
                wayland_data_source_destroy(source);
            }
            dd.drag_offer = None;
            dd.new_offers.clear();
            // (released from version 2)
            drop(dd);
        }

        if let Some(mut pd) = primary_selection_device {
            pd.selection_offer = None;
            pd.selection_source = None;
            pd.new_offers.clear();
            drop(pd);
        }

        drop(text_input);

        self.wayland_seat_destroy_keyboard(seat);
        self.wayland_seat_destroy_pointer(seat);
        self.wayland_seat_destroy_touch(seat);
        self.wayland_seat_destroy_tablet(seat, shutting_down);

        // (released from version 5)
        let removed = self.with_data(|d| {
            let i = d.seat_list.iter().position(|s| s.registry_id == seat)?;
            for k in [
                &mut d.last_implicit_grab_seat,
                &mut d.current_data_offer_seat,
                &mut d.current_primary_selection_seat,
            ] {
                if *k == Some(seat) {
                    // FIXME (upstream): the device keeps pointers to a
                    // destroyed seat in last_implicit_grab_seat and friends
                    // (followed later, a use after free); cleared here.
                    *k = None;
                }
            }
            Some(d.seat_list.remove(i))
        });
        drop(removed);
    }

    /// Translation of `Wayland_SeatUpdateRelativePointer()`.
    fn wayland_seat_update_relative_pointer(&self, seat: u32) {
        let warp_emulation_active = mouse::with_mouse(|m| m.warp_emulation_active);
        let focus_flags = self.with_data(|d| d.seat(seat).and_then(|s| s.pointer.focus));
        let relative_mode = focus_flags.map(|w| {
            with_window(w, |w| w.flags().contains(WindowFlags::MOUSE_RELATIVE_MODE))
                .unwrap_or(false)
        });

        self.with_data(|d| {
            let Some(manager) = d.g.relative_pointer_manager.as_ref() else {
                return;
            };
            let Some(s) = d.seat_list.iter().find(|s| s.registry_id == seat) else {
                return;
            };
            let mut relative_focus = false;

            if let Some(focus) = s.pointer.focus {
                /* If a seat has both keyboard and pointer capabilities, relative focus will follow the keyboard
                 * attached to that seat. Otherwise, relative focus will be gained if any other seat has keyboard
                 * focus on the window with pointer focus.
                 */
                if relative_mode == Some(true) {
                    if s.keyboard.wl_keyboard.is_some() {
                        relative_focus = s.keyboard.focus == s.pointer.focus;
                    } else {
                        relative_focus =
                            d.window(focus).is_some_and(|w| w.keyboard_focus_count != 0);
                    }
                } else {
                    relative_focus = warp_emulation_active;
                }
            }

            let pointer = s.pointer.wl_pointer.as_ref().map(|p| p.raw() as usize);
            let has_relative = s.pointer.relative_pointer.is_some();
            if relative_focus {
                if !has_relative {
                    let Some(pointer) = pointer else {
                        return;
                    };
                    // SAFETY: the seat's pointer is alive.
                    let Some(p) =
                        (unsafe { Obj::<WlPointer>::from_raw(pointer as *mut _, &self.conn) })
                    else {
                        return;
                    };
                    let mut relative = manager.get_relative_pointer(p);
                    self.listen(&mut relative, move |v, _, ev| {
                        v.relative_pointer_handle_relative_motion(seat, ev)
                    });
                    if let Some(s) = d.seat_mut(seat) {
                        s.pointer.relative_pointer = Some(relative);
                    }
                }
            } else if has_relative {
                if let Some(s) = d.seat_mut(seat) {
                    s.pointer.relative_pointer = None;
                }
            }
        });
    }

    /// Translation of `Wayland_SeatUpdateKeyboardGrab()`.
    fn wayland_seat_update_keyboard_grab(&self, seat: u32) {
        let focus = self.with_data(|d| d.seat(seat).and_then(|s| s.keyboard.focus));
        let grabbed = focus
            .map(|w| {
                with_window(w, |w| w.flags().contains(WindowFlags::KEYBOARD_GRABBED))
                    .unwrap_or(false)
            })
            .unwrap_or(false);

        self.with_data(|d| {
            let Some(manager) = d.g.key_inhibitor_manager.as_ref().map(|m| m.raw() as usize) else {
                return;
            };
            let surface = focus.and_then(|w| d.window(w)).map(|w| w.surface_id());
            let Some(s) = d.seat_mut(seat) else {
                return;
            };
            // Destroy the existing key inhibitor.
            s.keyboard.key_inhibitor = None;

            if s.keyboard.wl_keyboard.is_some() {
                if let Some(surface) = surface {
                    // Don't grab the keyboard if it shouldn't be grabbed.
                    if grabbed {
                        // SAFETY: the global and the window's surface are
                        // alive (the data is borrowed).
                        let (m, surf) = unsafe {
                            (
                                Obj::<ZwpKeyboardShortcutsInhibitManagerV1>::from_raw(
                                    manager as *mut _,
                                    &self.conn,
                                ),
                                Obj::<WlSurface>::from_raw(surface as *mut _, &self.conn),
                            )
                        };
                        if let (Some(m), Some(surf)) = (m, surf) {
                            s.keyboard.key_inhibitor =
                                Some(m.inhibit_shortcuts(surf, s.wl_seat.obj()));
                        }
                    }
                }
            }
        });
    }

    /// Translation of `Wayland_SeatUpdatePointerGrab()`.
    pub(crate) fn wayland_seat_update_pointer_grab(&self, seat: u32) {
        self.wayland_seat_update_relative_pointer(seat);

        if !self.with_data(|d| d.g.pointer_constraints.is_some()) {
            return;
        }

        let unlocked = self.with_data(|d| {
            let s = d.seat_mut(seat)?;
            if s.pointer.locked_pointer.is_some() && s.pointer.relative_pointer.is_none() {
                s.pointer.locked_pointer = None;
                Some(())
            } else {
                None
            }
        });
        if unlocked.is_some() {
            // Update the cursor after destroying a relative move lock.
            self.wayland_seat_update_pointer_cursor(seat);
        }

        let Some((has_pointer, relative, locked)) = self.with_data(|d| {
            let s = d.seat(seat)?;
            Some((
                s.pointer.wl_pointer.is_some(),
                s.pointer.relative_pointer.is_some(),
                s.pointer.locked_pointer.is_some(),
            ))
        }) else {
            return;
        };
        if !has_pointer {
            return;
        }

        // If relative mode is active, and the pointer focus matches the keyboard focus, lock it.
        if relative {
            if !locked {
                self.with_data(|d| {
                    let Some(constraints) =
                        d.g.pointer_constraints.as_ref().map(|c| c.raw() as usize)
                    else {
                        return;
                    };
                    let surface = d
                        .seat(seat)
                        .and_then(|s| s.pointer.focus)
                        .and_then(|w| d.window(w))
                        .map(|w| w.surface_id());
                    let Some(s) = d.seat_mut(seat) else {
                        return;
                    };
                    // Creating a lock on a surface with an active confinement region on the same seat is a protocol error.
                    s.pointer.confined_pointer = None;

                    // FIXME (upstream): with relative motion but no pointer
                    // focus, the lock is made on the NULL focus's surface (a
                    // crash); nothing is locked here.
                    let Some(surface) = surface else {
                        return;
                    };
                    let Some(p) = &s.pointer.wl_pointer else {
                        return;
                    };
                    // SAFETY: the global and the surface are alive.
                    let (c, surf) = unsafe {
                        (
                            Obj::<ZwpPointerConstraintsV1>::from_raw(
                                constraints as *mut _,
                                &self.conn,
                            ),
                            Obj::<WlSurface>::from_raw(surface as *mut _, &self.conn),
                        )
                    };
                    let (Some(c), Some(surf)) = (c, surf) else {
                        return;
                    };
                    let mut lock = c.lock_pointer(
                        surf,
                        p.obj(),
                        None,
                        ZwpPointerConstraintsV1Lifetime::PERSISTENT.0,
                    );
                    self.listen(&mut lock, move |v, _, ev| match ev {
                        ZwpLockedPointerV1Event::Locked => v.set_pointer_confined(seat, true),
                        ZwpLockedPointerV1Event::Unlocked => v.set_pointer_confined(seat, false),
                    });
                    s.pointer.locked_pointer = Some(lock);
                });

                // Ensure that the relative pointer is hidden, if required.
                self.wayland_seat_update_pointer_cursor(seat);
            }

            // Locked the cursor for relative mode, nothing more to do.
            return;
        }

        /* A confine may already be active, in which case we should destroy it and create a new one
         * in case it changed size.
         */
        let focus = self.with_data(|d| {
            let s = d.seat_mut(seat)?;
            s.pointer.confined_pointer = None;
            s.pointer.focus
        });
        let Some(w) = focus else {
            return;
        };

        let Ok((flags, mouse_rect)) = with_window(w, |w| (w.flags(), w.core.mouse_rect)) else {
            return;
        };

        // Don't confine the pointer if the window doesn't have input focus, or it shouldn't be confined.
        if !flags.contains(WindowFlags::INPUT_FOCUS)
            || (!flags.contains(WindowFlags::MOUSE_GRABBED) && mouse_rect.is_empty())
        {
            return;
        }

        let mut scaled_mouse_rect = None;
        if !mouse_rect.is_empty() {
            let Some((pointer_scale, last_motion)) = self.with_data(|d| {
                Some((
                    d.window(w)?.pointer_scale,
                    d.seat(seat)?.pointer.last_motion,
                ))
            }) else {
                return;
            };
            let r =
                Self::wayland_get_scaled_mouse_rect(mouse_rect, (pointer_scale.x, pointer_scale.y));
            scaled_mouse_rect = Some(r);

            /* Some compositors will only confine the pointer to an arbitrary region if the pointer
             * is already within the confinement area when it is created. Warp the pointer to the
             * closest point within the confinement zone if outside.
             */
            if !r.contains(last_motion) {
                /* Warp the pointer to the closest point within the confinement zone if outside,
                 * The confinement region will be created when a true position event is received.
                 */
                let mut closest_x = last_motion.x;
                let mut closest_y = last_motion.y;

                if closest_x < r.x {
                    closest_x = r.x;
                } else if closest_x >= r.x + r.w {
                    closest_x = (r.x + r.w) - 1;
                }

                if closest_y < r.y {
                    closest_y = r.y;
                } else if closest_y >= r.y + r.h {
                    closest_y = (r.y + r.h) - 1;
                }

                self.wayland_seat_warp_mouse(seat, w, closest_x as f32, closest_y as f32);
            }
        }

        if scaled_mouse_rect.is_some() || flags.contains(WindowFlags::MOUSE_GRABBED) {
            self.with_data(|d| {
                let Some(compositor) = d.g.compositor.as_ref() else {
                    return;
                };
                let Some(constraints) = d.g.pointer_constraints.as_ref().map(|c| c.raw() as usize)
                else {
                    return;
                };
                let confine_rect = scaled_mouse_rect.map(|r| {
                    let region = compositor.create_region();
                    region.add(r.x, r.y, r.w, r.h);
                    region
                });
                let Some(surface) = d.window(w).map(|w| w.surface_id()) else {
                    return;
                };
                let Some(s) = d.seat_mut(seat) else {
                    return;
                };
                let Some(p) = &s.pointer.wl_pointer else {
                    return;
                };
                // SAFETY: the global and the surface are alive.
                let (c, surf) = unsafe {
                    (
                        Obj::<ZwpPointerConstraintsV1>::from_raw(constraints as *mut _, &self.conn),
                        Obj::<WlSurface>::from_raw(surface as *mut _, &self.conn),
                    )
                };
                let (Some(c), Some(surf)) = (c, surf) else {
                    return;
                };
                if mouse_rect.w != 1 && mouse_rect.h != 1 {
                    let mut confined = c.confine_pointer(
                        surf,
                        p.obj(),
                        confine_rect.as_ref().map(|r| r.obj()),
                        ZwpPointerConstraintsV1Lifetime::PERSISTENT.0,
                    );
                    self.listen(&mut confined, move |v, _, ev| match ev {
                        ZwpConfinedPointerV1Event::Confined => v.set_pointer_confined(seat, true),
                        ZwpConfinedPointerV1Event::Unconfined => {
                            v.set_pointer_confined(seat, false)
                        }
                    });
                    s.pointer.confined_pointer = Some(confined);
                } else {
                    /* Use a lock for 1x1 confinement regions, as the pointer can exhibit subpixel motion otherwise.
                     * A null region is used since the warp *should* have placed the pointer where we want it, but
                     * better to lock it slightly off than let the pointer escape, as confining to a specific region
                     * seems to be a racy operation on some compositors.
                     */
                    let mut lock = c.lock_pointer(
                        surf,
                        p.obj(),
                        None,
                        ZwpPointerConstraintsV1Lifetime::PERSISTENT.0,
                    );
                    self.listen(&mut lock, move |v, _, ev| match ev {
                        ZwpLockedPointerV1Event::Locked => v.set_pointer_confined(seat, true),
                        ZwpLockedPointerV1Event::Unlocked => v.set_pointer_confined(seat, false),
                    });
                    s.pointer.locked_pointer = Some(lock);
                }
                // (the region is destroyed when dropped)
                drop(confine_rect);
            });
        }
    }

    /// Translation of `Wayland_DisplayUpdatePointerGrabs()`.
    pub(crate) fn wayland_display_update_pointer_grabs(&self, window: Option<WindowID>) {
        let seats: Vec<u32> = self.with_data(|d| {
            d.seat_list
                .iter()
                .filter(|s| window.is_none() || s.pointer.focus == window)
                .map(|s| s.registry_id)
                .collect()
        });
        for seat in seats {
            self.wayland_seat_update_pointer_grab(seat);
        }
    }

    /// Translation of `Wayland_DisplayUpdateKeyboardGrabs()`.
    pub(crate) fn wayland_display_update_keyboard_grabs(&self, window: Option<WindowID>) {
        let seats: Vec<u32> = self.with_data(|d| {
            d.seat_list
                .iter()
                .filter(|s| window.is_none() || s.keyboard.focus == window)
                .map(|s| s.registry_id)
                .collect()
        });
        for seat in seats {
            self.wayland_seat_update_keyboard_grab(seat);
        }
    }

    /// The implicit grab serial needs to be updated on:
    /// - Keyboard key down/up
    /// - Mouse button down
    /// - Touch event down
    /// - Tablet tool down
    /// - Tablet tool button down/up
    ///
    /// Translation of `Wayland_UpdateImplicitGrabSerial()`.
    pub(crate) fn wayland_update_implicit_grab_serial(&self, seat: u32, serial: u32) {
        self.with_data(|d| {
            let Some(s) = d.seat_mut(seat) else {
                return;
            };
            if serial > s.last_implicit_grab_serial {
                s.last_implicit_grab_serial = serial;
                wayland_data_device_set_serial(s.data_device.as_mut(), serial);
                wayland_primary_selection_device_set_serial(
                    s.primary_selection_device.as_mut(),
                    serial,
                );
                d.last_implicit_grab_seat = Some(seat);
            }
        });
    }
}

/// The longest prefix of `s` of at most `max` bytes that ends on a
/// character boundary (`SDL_strlcpy()` into a small buffer).
fn truncate_utf8(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// The current `errno`.
fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Set `errno` (the libwayland wrappers report it as an error value).
fn set_errno(e: i32) {
    // SAFETY: __errno_location() returns this thread's errno.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    unsafe {
        *libc::__errno_location() = e;
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let _ = e;
}
