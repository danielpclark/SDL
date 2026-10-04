// Rust translation of src/video/wayland/SDL_waylandmouse.c and
// SDL_waylandmouse.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see
// LICENSE.txt.

//! Cursors: custom (animated) cursors in shared memory buffers, system
//! cursors from the cursor-shape protocol or a wl_cursor theme, showing them
//! on the pointers and tablet tools of the seats (with the animation driven
//! by frame callbacks on the event thread), warping, relative mode and the
//! global mouse state.
//!
//! A cursor state ([`CursorState`]) holds the cursor it shows, so a cursor
//! is never freed while shown (upstream's `Wayland_FreeCursorData()` has to
//! detach it from the seats first).

use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use super::client::{AsProxy, Fixed, Obj, Proxy};
use super::eventthread::lock_event_thread;
use super::protocols::cursor_shape_v1::*;
use super::protocols::pointer_constraints_unstable_v1::*;
use super::protocols::pointer_warp_v1::*;
use super::protocols::tablet_v2::*;
use super::protocols::viewporter::*;
use super::protocols::wayland::*;
use super::shmbuffer::ShmPool;
use super::sys::*;
use super::video::{VideoData, WaylandVideo};
use super::wldyn::WaylandSyms;
use crate::error::{Error, Result};
use crate::events::mouse::{self, Cursor, CursorFrame, MouseButtonFlags, SystemCursor};
use crate::events::window::WindowFlags;
use crate::events::WindowID;
use crate::video::core::{css_cursor_name, with_window};
use crate::video::sysvideo::HitTestResult;
use crate::video::{PixelFormat, Surface};

const CURSOR_NODE: &str = "org.freedesktop.portal.Desktop";
const CURSOR_PATH: &str = "/org/freedesktop/portal/desktop";
const CURSOR_INTERFACE: &str = "org.freedesktop.portal.Settings";
const CURSOR_NAMESPACE: &str = "org.gnome.desktop.interface";
const CURSOR_SIGNAL_NAME: &str = "SettingChanged";
const CURSOR_SIZE_KEY: &str = "cursor-size";
const CURSOR_THEME_KEY: &str = "cursor-theme";

/// The cursor theme settings from the desktop portal (`dbus_cursor_size`
/// and `dbus_cursor_theme`).
static DBUS_CURSOR: Mutex<(i32, Option<String>)> = Mutex::new((0, None));

fn dbus_cursor() -> MutexGuard<'static, (i32, Option<String>)> {
    DBUS_CURSOR.lock().unwrap_or_else(|e| e.into_inner())
}

/// Bumped whenever the cursor themes are freed: the system cursor buffers
/// of an older generation belong to destroyed themes.
static THEME_GENERATION: AtomicU64 = AtomicU64::new(1);

/// A loaded cursor theme at a size. Translation of `SDL_WaylandCursorTheme`.
pub(crate) struct CursorTheme {
    theme: NonNull<wl_cursor_theme>,
    size: i32,
    syms: Arc<WaylandSyms>,
}

// SAFETY: the theme is only used with the device's state locked.
unsafe impl Send for CursorTheme {}

impl Drop for CursorTheme {
    fn drop(&mut self) {
        // SAFETY: the theme was loaded by wl_cursor_theme_load; its buffers
        // are no longer attached to a cursor surface of this generation.
        unsafe { (self.syms.cursor.wl_cursor_theme_destroy)(self.theme.as_ptr()) }
    }
}

/// An image of a custom cursor frame. Translation of `CustomCursorImage`.
struct CustomCursorImage {
    width: i32,
    height: i32,
    buffer: Proxy<WlBuffer>,
}

/// The system cursor buffers of a theme size. Translation of
/// `Wayland_CachedSystemCursor`.
struct CachedSystemCursor {
    size: i32,
    /// The theme generation the buffers belong to.
    generation: u64,
    /// The theme's `wl_buffer`s (owned by the theme).
    buffers: Vec<usize>,
}

/// The animation data of a system cursor, filled in when it is first shown.
#[derive(Default)]
struct SystemCursorData {
    num_frames: i32,
    frame_durations_ms: Vec<u32>,
    total_duration_ms: u32,
    cursor_buffer_cache: Vec<CachedSystemCursor>,
}

/// The kinds of cursor data (`cursor_data` of `SDL_CursorData`).
enum CursorKind {
    /// `Wayland_CustomCursor`
    Custom {
        // The base dimensions of the cursor.
        width: i32,
        height: i32,
        hot_x: i32,
        hot_y: i32,
        images_per_frame: i32,
        images: Vec<Option<CustomCursorImage>>,
        frame_durations_ms: Vec<u32>,
        total_duration_ms: u32,
        num_frames: i32,
    },
    /// `Wayland_SystemCursor`
    System {
        id: SystemCursor,
        data: Mutex<SystemCursorData>,
    },
}

/// The backend data of a cursor. Translation of `struct SDL_CursorData`;
/// dropping it is `Wayland_FreeCursor()`.
pub(crate) struct CursorData {
    kind: CursorKind,
}

impl CursorData {
    fn is_system_cursor(&self) -> bool {
        matches!(self.kind, CursorKind::System { .. })
    }

    /// The frame durations, their total and the frame count.
    fn animation(&self) -> (Vec<u32>, u32, i32) {
        match &self.kind {
            CursorKind::Custom {
                frame_durations_ms,
                total_duration_ms,
                num_frames,
                ..
            } => (frame_durations_ms.clone(), *total_duration_ms, *num_frames),
            CursorKind::System { data, .. } => {
                let d = data.lock().unwrap_or_else(|e| e.into_inner());
                (
                    d.frame_durations_ms.clone(),
                    d.total_duration_ms,
                    d.num_frames,
                )
            }
        }
    }

    fn num_frames(&self) -> i32 {
        self.animation().2
    }
}

/// The data of a Wayland cursor.
fn cursor_data(cursor: &Cursor) -> Option<&CursorData> {
    cursor.internal::<CursorData>()
}

/// The cursor of a pointer or tablet tool. Translation of
/// `SDL_WaylandCursorState`.
pub(crate) struct CursorState {
    pub(crate) current_cursor: Option<Cursor>,
    pub(crate) cursor_shape: Option<Proxy<WpCursorShapeDeviceV1>>,
    pub(crate) surface: Option<Proxy<WlSurface>>,
    pub(crate) viewport: Option<Proxy<WpViewport>>,

    pub(crate) scale: f64,

    /// The buffers of the system cursor shown (`system_cursor_handle`).
    system_cursor_handle: Option<(u64, Vec<usize>)>,

    /// The cursor animation thread lock must be held when modifying this.
    pub(crate) frame_callback: Option<Proxy<WlCallback>>,

    pub(crate) last_frame_callback_time_ms: u64,
    pub(crate) current_frame_time_ms: u32,

    /// 0 or greater if a buffer is attached, -1 if in the reset state.
    pub(crate) current_frame: i32,

    #[allow(dead_code)] // (unused upstream too)
    pub(crate) hit_test_result: HitTestResult,
}

impl Default for CursorState {
    fn default() -> CursorState {
        CursorState {
            current_cursor: None,
            cursor_shape: None,
            surface: None,
            viewport: None,
            scale: 0.0,
            system_cursor_handle: None,
            frame_callback: None,
            last_frame_callback_time_ms: 0,
            current_frame_time_ms: 0,
            current_frame: 0,
            hit_test_result: HitTestResult::Normal,
        }
    }
}

/// A cursor state, shared with its frame callback on the event thread.
pub(crate) type CursorStateCell = Arc<Mutex<CursorState>>;

/// Lock a cursor state.
pub(crate) fn lock_state(cell: &CursorStateCell) -> MutexGuard<'_, CursorState> {
    cell.lock().unwrap_or_else(|e| e.into_inner())
}

/// The state of `SDL_waylandmouse.c`: its `sys_cursors`, and the portal's
/// cursor settings filter.
#[derive(Default)]
pub(crate) struct MouseData {
    pub(crate) sys_cursors: [Option<Cursor>; HitTestResult::ResizeLeft as usize + 1],
    dbus_filter: Option<DbusCursorFilter>,
}

/// The D-Bus filter for the portal's cursor settings, removed on drop.
struct DbusCursorFilter {
    // (the filter borrows the connection of the context: declared first so
    // that it goes first)
    _filter: crate::core::linux::dbus::Filter<'static>,
    _ctx: Arc<crate::core::linux::dbus::Context>,
}

// SAFETY: the filter's data is a `Send` closure, and the connection it is
// removed from is thread safe (`Connection` is `Send + Sync`).
unsafe impl Send for DbusCursorFilter {}

/// What a cursor is shown on (`Wayland_PointerObject`).
#[derive(Clone, Copy)]
pub(crate) enum PointerObject<'a> {
    Pointer(Obj<'a, WlPointer>),
    Tool(Obj<'a, ZwpTabletToolV2>),
}

impl PointerObject<'_> {
    fn set_cursor(self, serial: u32, surface: Option<Obj<'_, WlSurface>>, hot_x: i32, hot_y: i32) {
        match self {
            PointerObject::Pointer(p) => p.set_cursor(serial, surface, hot_x, hot_y),
            PointerObject::Tool(t) => t.set_cursor(serial, surface, hot_x, hot_y),
        }
    }
}

/// The scales of the window a cursor is over (`focus` of
/// `Wayland_CursorStateSetCursor()`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct CursorFocus {
    pub(crate) scale_factor: f64,
    pub(crate) pointer_scale_x: f64,
    pub(crate) pointer_scale_y: f64,
}

impl VideoData {
    /// The cursor scales of a window.
    pub(crate) fn cursor_focus(&self, window: Option<WindowID>) -> Option<CursorFocus> {
        let w = self.window(window?)?;
        Some(CursorFocus {
            scale_factor: w.scale_factor,
            pointer_scale_x: w.pointer_scale.x,
            pointer_scale_y: w.pointer_scale.y,
        })
    }
}

/// Translation of `Wayland_ReadDBusProperty()` and `Wayland_ParseDBusReply()`:
/// the value of a setting of the portal.
fn wayland_read_dbus_property(key: &str) -> Option<crate::core::linux::dbus::Value> {
    use crate::core::linux::dbus::{self, Arg, Value};

    const IFACE: &str = "org.gnome.desktop.interface";

    let reply = dbus::call_method(
        CURSOR_NODE,
        CURSOR_PATH,
        CURSOR_INTERFACE,
        "Read", // Method
        &[Arg::Str(IFACE), Arg::Str(key)],
    )?;

    let v = reply.args().into_iter().next()?;
    let Value::Variant(v) = v else {
        return None;
    };
    let Value::Variant(v) = *v else {
        return None;
    };
    Some(*v)
}

/// Translation of `Wayland_DBusCursorMessageFilter()`.
fn wayland_dbus_cursor_message_filter(
    video: &Weak<WaylandVideo>,
    msg: &crate::core::linux::dbus::Message,
) -> crate::core::linux::dbus::HandlerResult {
    use crate::core::linux::dbus::{HandlerResult, Value};

    if msg.is_signal(CURSOR_INTERFACE, CURSOR_SIGNAL_NAME) {
        let args = msg.args();
        let mut signal_iter = args.iter();

        // Check if the parameters are what we expect
        let Some(Value::Str(namespace)) = signal_iter.next() else {
            return HandlerResult::NotYetHandled;
        };
        if namespace != CURSOR_NAMESPACE {
            return HandlerResult::NotYetHandled;
        }
        let Some(Value::Str(key)) = signal_iter.next() else {
            return HandlerResult::NotYetHandled;
        };
        let Some(Value::Variant(variant)) = signal_iter.next() else {
            return HandlerResult::NotYetHandled;
        };
        if key == CURSOR_SIZE_KEY {
            let Value::I32(new_cursor_size) = **variant else {
                return HandlerResult::NotYetHandled;
            };

            let changed = {
                let mut c = dbus_cursor();
                if c.0 != new_cursor_size {
                    c.0 = new_cursor_size;
                    true
                } else {
                    false
                }
            };
            if changed {
                mouse::redraw_cursor(); // Force cursor update
            }
        } else if key == CURSOR_THEME_KEY {
            let Value::Str(new_cursor_theme) = &**variant else {
                return HandlerResult::NotYetHandled;
            };

            let changed = {
                let mut c = dbus_cursor();
                if c.1.as_deref() != Some(new_cursor_theme.as_str()) {
                    c.1 = Some(new_cursor_theme.clone());
                    true
                } else {
                    false
                }
            };
            if changed {
                // Purge the current cached themes and force a cursor refresh.
                if let Some(v) = video.upgrade() {
                    let themes = v.with_data(|d| {
                        let context = v.event_thread();
                        let _guard = lock_event_thread(&context);
                        let themes = std::mem::take(&mut d.cursor_themes);
                        THEME_GENERATION.fetch_add(1, Ordering::AcqRel);
                        themes
                    });
                    drop(themes);
                }
                mouse::redraw_cursor();
            }
        } else {
            return HandlerResult::NotYetHandled;
        }

        return HandlerResult::Handled;
    }

    HandlerResult::NotYetHandled
}

/// Translation of `Wayland_CursorStateGetFrame()`: the buffer of a frame
/// (a raw `wl_buffer`).
fn wayland_cursor_state_get_frame(state: &CursorState, frame_index: i32) -> Option<usize> {
    let data = state.current_cursor.as_ref().and_then(cursor_data)?;

    match &data.kind {
        CursorKind::Custom {
            width,
            height,
            images_per_frame,
            images,
            ..
        } => {
            let offset = (*images_per_frame * frame_index) as usize;

            /* Find the closest image. Images that are larger than the
             * desired size are preferred over images that are smaller.
             */
            let target_area = (*width as f64 * *height as f64 * state.scale).round() as i64;
            let mut closest: Option<&CustomCursorImage> = None;
            let mut closest_area: i64 = 0;
            for i in 0..*images_per_frame as usize {
                if closest_area >= target_area {
                    break;
                }
                let Some(Some(image)) = images.get(offset + i) else {
                    break;
                };
                closest = Some(image);
                closest_area = image.width as i64 * image.height as i64;
            }

            closest.map(|c| c.buffer.raw() as usize)
        }
        CursorKind::System { .. } => {
            let (generation, buffers) = state.system_cursor_handle.as_ref()?;
            // (the buffers of a freed theme aren't used: upstream would)
            if *generation != THEME_GENERATION.load(Ordering::Acquire) {
                return None;
            }
            buffers.get(frame_index as usize).copied()
        }
    }
}

/// Attach a raw buffer (or none) to a cursor surface.
fn attach_raw(surface: &Proxy<WlSurface>, buffer: Option<usize>) {
    // SAFETY: the buffer is a live wl_buffer of this display: a custom
    // cursor's (the cursor is held by the state) or a theme's of the current
    // generation (themes are only freed with the event thread locked, and
    // the generation bumped).
    let buffer =
        buffer.and_then(|b| unsafe { Obj::<WlBuffer>::from_raw(b as *mut _, surface.conn()) });
    surface.attach(buffer, 0, 0);
}

/// Damage a whole cursor surface.
fn damage_all(surface: &Proxy<WlSurface>) {
    if surface.version() >= WlSurface::DAMAGE_BUFFER_SINCE_VERSION {
        surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
    } else {
        surface.damage(0, 0, i32::MAX, i32::MAX);
    }
}

/// Translation of `cursor_frame_done()` (on the event thread).
fn cursor_frame_done(cell: &Weak<Mutex<CursorState>>, cb: usize) {
    let Some(cell) = cell.upgrade() else {
        return;
    };
    let mut state = lock_state(&cell);
    // (a callback replaced or destroyed in the meantime)
    if state.frame_callback.as_ref().map(|c| c.raw() as usize) != Some(cb) {
        return;
    }
    let Some(c) = state.current_cursor.as_ref().and_then(cursor_data) else {
        return;
    };

    let (frames, total_duration_ms, num_frames) = c.animation();

    let now = crate::timer::ticks_ms();
    let elapsed = if total_duration_ms != 0 {
        ((now - state.last_frame_callback_time_ms) % total_duration_ms as u64) as u32
    } else {
        // FIXME (upstream): a cursor whose frame durations add up to 0
        // divides by zero here; Rust panics on that, so no time elapses.
        0
    };
    let mut advance: u32 = 0;
    let mut next = state.current_frame.max(0) as usize;

    state.current_frame_time_ms = state.current_frame_time_ms.wrapping_add(elapsed);

    // Calculate the next frame based on the elapsed duration.
    let mut t = frames.get(next).copied().unwrap_or(0);
    while t <= state.current_frame_time_ms {
        next = (next + 1) % num_frames.max(1) as usize;
        advance = t;

        // Make sure we don't end up in an infinite loop if a cursor has frame durations of 0.
        if frames.get(next).copied().unwrap_or(0) == 0 {
            break;
        }
        t = t.wrapping_add(frames[next]);
    }

    // (wl_callback_destroy())
    state.frame_callback = None;

    // Don't queue another callback if this frame time is infinite.
    if frames.get(next).copied().unwrap_or(0) != 0 {
        if let Some(surface) = &state.surface {
            let mut callback = surface.frame();
            let weak = Arc::downgrade(&cell);
            callback.listen(move |cb, _| cursor_frame_done(&weak, cb.raw() as usize));
            state.frame_callback = Some(callback);
        }
    }

    state.current_frame_time_ms -= advance;
    state.last_frame_callback_time_ms = now;
    state.current_frame = next as i32;

    let buffer = wayland_cursor_state_get_frame(&state, next as i32);
    if let Some(surface) = &state.surface {
        attach_raw(surface, buffer);
        damage_all(surface);
        surface.commit();
    }
}

impl WaylandVideo {
    /// Translation of `Wayland_CursorStateSetFrameCallback()` (the event
    /// thread lock is taken by the caller, with the state's).
    fn wayland_cursor_state_set_frame_callback(cell: &CursorStateCell, state: &mut CursorState) {
        if let Some(surface) = &state.surface {
            let mut callback = surface.frame();
            let weak = Arc::downgrade(cell);
            callback.listen(move |cb, _| cursor_frame_done(&weak, cb.raw() as usize));
            state.frame_callback = Some(callback);
        }
    }

    /// Translation of `Wayland_CursorStateDestroyFrameCallback()` (with the
    /// event thread locked by the caller).
    fn wayland_cursor_state_destroy_frame_callback(state: &mut CursorState) {
        state.frame_callback = None;
    }

    /// Translation of `Wayland_CursorStateResetAnimationState()`.
    fn wayland_cursor_state_reset_animation_state(state: &mut CursorState) {
        state.last_frame_callback_time_ms = crate::timer::ticks_ms();
        state.current_frame_time_ms = 0;
        state.current_frame = 0;
    }

    /// Release a cursor state's objects. Translation of
    /// `Wayland_CursorStateRelease()`.
    pub(crate) fn wayland_cursor_state_release(&self, cell: &CursorStateCell) {
        let context = self.event_thread();
        let _guard = lock_event_thread(&context);
        let mut state = lock_state(cell);
        Self::wayland_cursor_state_destroy_frame_callback(&mut state);
        state.cursor_shape = None;
        state.viewport = None;
        if let Some(surface) = state.surface.take() {
            surface.attach(None, 0, 0);
            surface.commit();
            drop(surface);
        }

        *state = CursorState::default();
    }

    /// Translation of `Wayland_GetSystemCursor()`: the cursor's buffers at
    /// the state's scale, and its size and hot spot in surface units.
    fn wayland_get_system_cursor(
        &self,
        vdata: &mut VideoData,
        id: SystemCursor,
        sys: &Mutex<SystemCursorData>,
        state: &mut CursorState,
    ) -> Option<(i32, i32, i32)> {
        let scale_factor = state.scale;
        let mut theme_size = dbus_cursor().0;

        // Fallback envvar if the DBus properties don't exist
        if theme_size <= 0 {
            if let Some(xcursor_size) = crate::stdlib::getenv("XCURSOR_SIZE") {
                theme_size = crate::stdlib::atoi(&xcursor_size);
            }
        }
        if theme_size <= 0 {
            theme_size = 24;
        }

        // First, find the appropriate theme based on the current scale...
        let scaled_size = (theme_size as f64 * scale_factor).round() as i32;
        let theme = match vdata.cursor_themes.iter().find(|t| t.size == scaled_size) {
            Some(t) => t.theme,
            None => {
                let mut xcursor_theme = dbus_cursor().1.clone();

                // Fallback envvar if the DBus properties don't exist
                if xcursor_theme.is_none() {
                    xcursor_theme = crate::stdlib::getenv("XCURSOR_THEME");
                }

                let shm = vdata.g.shm.as_ref()?;
                let name = xcursor_theme.as_deref().map(super::video::c_string);
                // SAFETY: the name is NULL or NUL-terminated; the shm global
                // is alive.
                let theme = unsafe {
                    (self.syms().cursor.wl_cursor_theme_load)(
                        name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()),
                        scaled_size,
                        shm.raw(),
                    )
                };
                // FIXME (upstream): a theme that fails to load (NULL) is
                // cached and then used; libwayland-cursor's
                // wl_cursor_theme_get_cursor() would crash on it. It isn't
                // cached here.
                let theme = NonNull::new(theme)?;
                vdata.cursor_themes.push(CursorTheme {
                    theme,
                    size: scaled_size,
                    syms: self.syms().clone(),
                });
                theme
            }
        };

        let (css_name, fallback_name) = css_cursor_name(id);
        let get = |name: &str| {
            let name = super::video::c_string(name);
            // SAFETY: the theme is loaded; the name is NUL-terminated.
            NonNull::new(unsafe {
                (self.syms().cursor.wl_cursor_theme_get_cursor)(theme.as_ptr(), name.as_ptr())
            })
        };
        let mut cursor = get(css_name);
        if cursor.is_none() {
            if let Some(fallback) = fallback_name {
                cursor = get(fallback);
            }
        }

        // Fallback to the default cursor if the chosen one wasn't found
        if cursor.is_none() {
            cursor = get("default");
        }
        // Try the old X11 name as a last resort
        if cursor.is_none() {
            cursor = get("left_ptr");
        }
        let cursor = cursor?;

        // SAFETY: the cursor belongs to the loaded theme; its images array
        // has image_count entries.
        let images: Vec<&wl_cursor_image> = unsafe {
            let c = cursor.as_ref();
            (0..c.image_count as usize)
                .map(|i| &**c.images.add(i))
                .collect()
        };

        // ... Set the cursor data, finally.
        let mut sys = sys.lock().unwrap_or_else(|e| e.into_inner());
        sys.num_frames = images.len() as i32;
        let generation = THEME_GENERATION.load(Ordering::Acquire);
        let buffers =
            wayland_cache_system_cursor(self.syms(), &mut sys, &images, theme_size, generation);
        state.system_cursor_handle = Some((generation, buffers));

        if images.len() > 1 && sys.frame_durations_ms.is_empty() {
            sys.total_duration_ms = 0;
            sys.frame_durations_ms = images.iter().map(|i| i.delay).collect();
            sys.total_duration_ms = images.iter().map(|i| i.delay).sum();
        }

        let first = images.first()?;
        let dst_size = (first.width as f64 / state.scale).round() as i32;
        let hot_x = (first.hotspot_x as f64 / state.scale).round() as i32;
        let hot_y = (first.hotspot_y as f64 / state.scale).round() as i32;

        Some((dst_size, hot_x, hot_y))
    }

    /// Translation of `Wayland_CreateAnimatedCursor()`.
    pub(crate) fn wayland_create_animated_cursor(
        &self,
        frames: &[CursorFrame<'_>],
        hot_x: i32,
        hot_y: i32,
    ) -> Result<Cursor> {
        let Some(first) = frames.first() else {
            return Err(Error::invalid_param("frames"));
        };

        let mut pool_size = 0i32;
        let mut max_images = 0usize;

        // Calculate the total allocation size.
        let mut surfaces: Vec<Vec<&Surface<'_>>> = Vec::with_capacity(frames.len());
        for frame in frames {
            let mut images: Vec<&Surface<'_>> = std::iter::once(frame.surface)
                .chain(frame.surface.alternate_images().iter())
                .collect();

            // (a stable sort by area, as surface_sort_callback orders them)
            images.sort_by_key(|s| s.width() as i64 * s.height() as i64);
            max_images = max_images.max(images.len());
            for s in &images {
                pool_size += s.width() * s.height() * 4;
            }
            surfaces.push(images);
        }

        let shm = self.with_data(|d| d.g.shm.as_ref().map(|s| s.raw()));
        let Some(shm) = shm else {
            return Err(Error::new("wayland: no wl_shm"));
        };
        // SAFETY: the shm global stays alive while the device exists.
        let shm = unsafe { Obj::<WlShm>::from_raw(shm.cast(), &self.conn) }
            .ok_or_else(|| Error::new("wayland: no wl_shm"))?;
        let mut shm_pool = ShmPool::alloc(shm, pool_size)?;

        let mut images: Vec<Option<CustomCursorImage>> = Vec::new();
        images.resize_with(max_images * frames.len(), || None);
        let mut frame_durations_ms = Vec::with_capacity(frames.len());
        let mut total_duration_ms: u32 = 0;

        for (i, frame) in frames.iter().enumerate() {
            frame_durations_ms.push(frame.duration);
            if total_duration_ms < u32::MAX {
                if frame.duration > 0 {
                    total_duration_ms = total_duration_ms.saturating_add(frame.duration);
                } else {
                    total_duration_ms = u32::MAX;
                }
            }

            let offset = i * max_images;
            for (j, &surface) in surfaces[i].iter().enumerate() {
                // Convert the surface format, if required.
                let converted;
                let surface = if surface.format() != PixelFormat::ARGB8888 {
                    converted = surface.convert(PixelFormat::ARGB8888)?;
                    &converted
                } else {
                    surface
                };

                let (w, h) = (surface.width(), surface.height());
                let Some((buffer, buf_data)) = shm_pool.alloc_buffer(w, h) else {
                    return Err(Error::new("wayland: failed to allocate a cursor buffer"));
                };
                // Wayland requires premultiplied alpha for its surfaces.
                crate::video::surface::premultiply_alpha(
                    w,
                    h,
                    surface.format(),
                    surface.pixels().unwrap_or(&[]),
                    surface.pitch(),
                    PixelFormat::ARGB8888,
                    buf_data,
                    w * 4,
                    true,
                )?;

                images[offset + j] = Some(CustomCursorImage {
                    width: w,
                    height: h,
                    buffer,
                });
            }
        }

        drop(shm_pool);

        Ok(Cursor::with_internal(CursorData {
            kind: CursorKind::Custom {
                width: first.surface.width(),
                height: first.surface.height(),
                hot_x,
                hot_y,
                images_per_frame: max_images as i32,
                images,
                frame_durations_ms,
                total_duration_ms,
                num_frames: frames.len() as i32,
            },
        }))
    }

    /// Translation of `Wayland_CreateCursor()`.
    pub(crate) fn wayland_create_cursor(
        &self,
        surface: &Surface<'_>,
        hot_x: i32,
        hot_y: i32,
    ) -> Result<Cursor> {
        let frame = CursorFrame {
            surface,
            duration: 0,
        };

        self.wayland_create_animated_cursor(&[frame], hot_x, hot_y)
    }

    /// Translation of `Wayland_CreateSystemCursor()`.
    pub(crate) fn wayland_create_system_cursor(&self, id: SystemCursor) -> Cursor {
        Cursor::with_internal(CursorData {
            kind: CursorKind::System {
                id,
                data: Mutex::new(SystemCursorData::default()),
            },
        })
    }

    /// Translation of `Wayland_CreateDefaultCursor()`.
    fn wayland_create_default_cursor(&self) -> Cursor {
        let id = mouse::default_system_cursor();
        self.wayland_create_system_cursor(id)
    }

    /// Show `cursor` (or hide the cursor with `None`) on a pointer or tool.
    /// Translation of `Wayland_CursorStateSetCursor()`.
    fn wayland_cursor_state_set_cursor(
        &self,
        vdata: &mut VideoData,
        cell: &CursorStateCell,
        obj: PointerObject<'_>,
        focus: Option<CursorFocus>,
        serial: u32,
        cursor: Option<&Cursor>,
    ) {
        let context = self.event_thread();
        let _guard = lock_event_thread(&context);
        let mut state = lock_state(cell);
        let state = &mut *state;

        let cursor_data_of =
            |c: Option<&Cursor>| c.and_then(cursor_data).map(|d| d as *const CursorData);
        let new_data = cursor_data_of(cursor);
        let same_cursor =
            new_data.is_some() && new_data == cursor_data_of(state.current_cursor.as_ref());

        // Stop the frame callback for old animated cursors.
        if !same_cursor {
            Self::wayland_cursor_state_destroy_frame_callback(state);
        }

        let Some((cursor, cdata)) = cursor.and_then(|c| cursor_data(c).map(|d| (c, d))) else {
            // (a cursor that isn't ours is shown as none, as upstream's NULL
            // internal data)
            Self::wayland_cursor_state_destroy_frame_callback(state);
            state.current_cursor = None;

            if let Some(surface) = &state.surface {
                surface.attach(None, 0, 0);
                surface.commit();
            }

            obj.set_cursor(serial, None, 0, 0);
            return;
        };

        if same_cursor && state.current_frame >= 0 {
            // Restart the animation sequence if the cursor didn't change.
            if cdata.num_frames() > 1 {
                Self::wayland_cursor_state_reset_animation_state(state);
            }

            return;
        }

        let has_viewporter = vdata.g.viewporter.is_some();
        let dst_width;
        let dst_height;
        let hot_x;
        let hot_y;

        match &cdata.kind {
            CursorKind::System { id, data } => {
                // If the cursor shape protocol is supported, the compositor will draw nicely scaled cursors for us, so nothing more to do.
                if let Some(shape_device) = &state.cursor_shape {
                    // Don't need the surface or viewport if using the cursor shape protocol.
                    if let Some(surface) = state.surface.take() {
                        surface.attach(None, 0, 0);
                        surface.commit();

                        obj.set_cursor(serial, None, 0, 0);

                        state.viewport = None;

                        drop(surface);
                    }

                    let shape = wayland_get_system_cursor_shape(*id);
                    shape_device.set_shape(serial, shape);
                    state.current_cursor = Some(cursor.clone());
                    state.current_frame = 0;

                    return;
                }

                // If viewports aren't available, the scale is always 1.0.
                state.scale = match focus {
                    Some(f) if has_viewporter => f.scale_factor,
                    _ => 1.0,
                };
                let Some((size, hx, hy)) = self.wayland_get_system_cursor(vdata, *id, data, state)
                else {
                    return;
                };

                dst_width = size;
                dst_height = size;
                hot_x = hx;
                hot_y = hy;
            }
            CursorKind::Custom {
                width,
                height,
                hot_x: chx,
                hot_y: chy,
                ..
            } => {
                /* If viewports aren't available, the scale is always 1.0.
                 *
                 * If the pointer scale values are 1.0, the preferred backing buffer scale is the window scale.
                 *
                 * If the pointer is scaled, the dimensions are scaled by the pointer scale, so custom cursors will be scaled
                 * relative to the viewport size.
                 */
                match focus {
                    Some(f) if !(f.pointer_scale_x == 1.0 && f.pointer_scale_y == 1.0) => {
                        // The preferred buffer scale is the inverse of the pointer scale.
                        state.scale = 1.0 / f.pointer_scale_x.min(f.pointer_scale_y);
                        dst_width = ((*width as f64 / f.pointer_scale_x).round() as i32).max(1);
                        dst_height = ((*height as f64 / f.pointer_scale_y).round() as i32).max(1);
                        hot_x = (*chx as f64 / f.pointer_scale_x).round() as i32;
                        hot_y = (*chy as f64 / f.pointer_scale_y).round() as i32;
                    }
                    _ => {
                        state.scale = match focus {
                            Some(f) if has_viewporter => f.scale_factor,
                            _ => 1.0,
                        };
                        dst_width = *width;
                        dst_height = *height;
                        hot_x = *chx;
                        hot_y = *chy;
                    }
                }
            }
        }

        state.current_cursor = Some(cursor.clone());

        if state.surface.is_none() {
            let Some(compositor) = &vdata.g.compositor else {
                return;
            };
            state.surface = Some(match &context {
                Some(context) => {
                    let compositor_wrapper = context.create_proxy_wrapper(compositor.obj());
                    let surface = compositor_wrapper.create_surface();
                    drop(compositor_wrapper);
                    surface
                }
                None => compositor.create_surface(),
            });
        }

        let buffer = wayland_cursor_state_get_frame(state, 0);
        let Some(surface) = &state.surface else {
            return;
        };
        attach_raw(surface, buffer);
        state.current_frame = 0;

        if state.scale != 1.0 {
            if state.viewport.is_none() {
                if let Some(viewporter) = &vdata.g.viewporter {
                    state.viewport = Some(viewporter.get_viewport(surface.obj()));
                }
            }

            if let Some(viewport) = &state.viewport {
                viewport.set_source(
                    Fixed::from_int(-1),
                    Fixed::from_int(-1),
                    Fixed::from_int(-1),
                    Fixed::from_int(-1),
                );
                viewport.set_destination(dst_width, dst_height);
            }
        } else {
            state.viewport = None;
        }

        obj.set_cursor(serial, Some(surface.obj()), hot_x, hot_y);

        damage_all(surface);

        // If more than one frame is available, create a frame callback to run the animation.
        if cdata.num_frames() > 1 {
            Self::wayland_cursor_state_reset_animation_state(state);
            Self::wayland_cursor_state_set_frame_callback(cell, state);
        }

        if let Some(surface) = &state.surface {
            surface.commit();
        }
    }

    /// Translation of `Wayland_CursorStateResetCursor()`.
    fn wayland_cursor_state_reset_cursor(&self, cell: &CursorStateCell) {
        let context = self.event_thread();
        let _guard = lock_event_thread(&context);
        let mut state = lock_state(cell);
        // Stop the frame callback and set the reset status.
        Self::wayland_cursor_state_destroy_frame_callback(&mut state);
        state.current_frame = -1;
    }

    /// Translation of `Wayland_DisplayUpdatePointerFocusedScale()`.
    pub(crate) fn wayland_display_update_pointer_focused_scale(&self, updated_window: WindowID) {
        let Some((new_scale, seats, tools)) = self.with_data(|d| {
            let w = d.window(updated_window)?;
            let new_scale = w.pointer_scale.x.min(w.pointer_scale.y);
            let needs = |cell: &CursorStateCell| {
                let state = lock_state(cell);
                state
                    .current_cursor
                    .as_ref()
                    .and_then(cursor_data)
                    .is_some_and(|c| !c.is_system_cursor())
                    && state.scale != new_scale
            };
            let mut seats = Vec::new();
            let mut tools = Vec::new();
            for seat in &d.seat_list {
                if seat.pointer.focus == Some(updated_window) && needs(&seat.pointer.cursor_state) {
                    seats.push(seat.registry_id);
                }
                for tool in &seat.tablet.tool_list {
                    if tool.focus == Some(updated_window) && needs(&tool.cursor_state) {
                        tools.push((seat.registry_id, tool.key()));
                    }
                }
            }
            Some((new_scale, seats, tools))
        }) else {
            return;
        };
        let _ = new_scale;

        for seat in seats {
            if let Some(cell) =
                self.with_data(|d| d.seat(seat).map(|s| s.pointer.cursor_state.clone()))
            {
                self.wayland_cursor_state_reset_cursor(&cell);
                self.wayland_seat_update_pointer_cursor(seat);
            }
        }
        for (seat, tool) in tools {
            if let Some(cell) =
                self.with_data(|d| d.seat(seat)?.tool(tool).map(|t| t.cursor_state.clone()))
            {
                self.wayland_cursor_state_reset_cursor(&cell);
                self.wayland_tablet_tool_update_cursor(seat, tool);
            }
        }
    }

    /// Translation of `Wayland_ShowCursor()`.
    pub(crate) fn wayland_show_cursor(&self, cursor: Option<&Cursor>) -> Result<()> {
        let mouse_focus = mouse::mouse_focus();
        let cur_cursor = mouse::cursor();

        self.with_data(|d| {
            let seats: Vec<u32> = d.seat_list.iter().map(|s| s.registry_id).collect();
            for key in seats {
                let Some(seat) = d.seat(key) else {
                    continue;
                };
                let pointer = seat.pointer.wl_pointer.as_ref().map(|p| p.raw() as usize);
                let focus = seat.pointer.focus;
                let enter_serial = seat.pointer.enter_serial;
                let cell = seat.pointer.cursor_state.clone();
                let tools: Vec<(usize, Option<WindowID>, u32, CursorStateCell)> = seat
                    .tablet
                    .tool_list
                    .iter()
                    .map(|t| (t.key(), t.focus, t.proximity_serial, t.cursor_state.clone()))
                    .collect();

                if let Some(pointer) = pointer {
                    if mouse_focus.is_some() && mouse_focus == focus {
                        let f = d.cursor_focus(focus);
                        // SAFETY: the seat's pointer is alive (the seat is
                        // borrowed through `d` for this call).
                        if let Some(p) =
                            unsafe { Obj::<WlPointer>::from_raw(pointer as *mut _, &self.conn) }
                        {
                            self.wayland_cursor_state_set_cursor(
                                d,
                                &cell,
                                PointerObject::Pointer(p),
                                f,
                                enter_serial,
                                cursor,
                            );
                        }
                    } else if focus.is_none() {
                        self.wayland_cursor_state_reset_cursor(&cell);
                    }
                }

                for (tool, tool_focus, proximity_serial, tool_cell) in tools {
                    /* The current cursor is explicitly set on tablet tools, as there may be no pointer device, or
                     * the pointer may not have focus, which would instead cause the default cursor to be set.
                     */
                    if tool_focus.is_some() && (mouse_focus.is_none() || mouse_focus == tool_focus)
                    {
                        let f = d.cursor_focus(tool_focus);
                        // SAFETY: the tool is alive (in its seat's list).
                        if let Some(t) =
                            unsafe { Obj::<ZwpTabletToolV2>::from_raw(tool as *mut _, &self.conn) }
                        {
                            self.wayland_cursor_state_set_cursor(
                                d,
                                &tool_cell,
                                PointerObject::Tool(t),
                                f,
                                proximity_serial,
                                cur_cursor.as_ref(),
                            );
                        }
                    } else if tool_focus.is_none() {
                        self.wayland_cursor_state_reset_cursor(&tool_cell);
                    }
                }
            }
        });

        Ok(())
    }

    /// Warp the pointer of a seat within a window. Translation of
    /// `Wayland_SeatWarpMouse()`.
    pub(crate) fn wayland_seat_warp_mouse(&self, seat: u32, window: WindowID, x: f32, y: f32) {
        let Some((update_grabs, send_warp, sdl_id)) = self.with_data(|d| {
            let (pointer_scale, logical, surface) = {
                let w = d.window(window)?;
                (
                    w.pointer_scale,
                    (w.current.logical_width, w.current.logical_height),
                    w.surface_id(),
                )
            };
            let warp = d.g.wp_pointer_warp_v1.as_ref().map(|w| w.raw() as usize);
            let constraints = d.g.pointer_constraints.as_ref().map(|c| c.raw() as usize);
            let s = d.seat_mut(seat)?;
            let p = s.pointer.wl_pointer.as_ref()?;
            let conn = &self.conn;
            // SAFETY: the window's surface is alive (its data is borrowed).
            let surface = unsafe { Obj::<WlSurface>::from_raw(surface as *mut _, conn) }?;
            let mut update_grabs = false;
            if let Some(warp) = warp {
                // SAFETY: the global is alive.
                let warp = unsafe { Obj::<WpPointerWarpV1>::from_raw(warp as *mut _, conn) }?;
                // It's a protocol error to warp the pointer outside of the surface, so clamp the position.
                let f_x =
                    Fixed::from_f64((x as f64 / pointer_scale.x).clamp(0.0, logical.0 as f64));
                let f_y =
                    Fixed::from_f64((y as f64 / pointer_scale.y).clamp(0.0, logical.1 as f64));
                warp.warp_pointer(surface, p.obj(), f_x, f_y, s.pointer.enter_serial);
            } else {
                // Pointers can only have one confinement type active on a surface at one time.
                if s.pointer.confined_pointer.take().is_some() {
                    update_grabs = true;
                }
                if s.pointer.locked_pointer.take().is_some() {
                    update_grabs = true;
                }

                /* The pointer confinement protocol allows setting a hint to warp the pointer,
                 * but only when the pointer is locked.
                 *
                 * Lock the pointer, set the position hint, unlock, and hope for the best.
                 */
                let constraints = constraints?;
                // SAFETY: the global is alive.
                let constraints = unsafe {
                    Obj::<ZwpPointerConstraintsV1>::from_raw(constraints as *mut _, conn)
                }?;
                let warp_lock = constraints.lock_pointer(
                    surface,
                    p.obj(),
                    None,
                    ZwpPointerConstraintsV1Lifetime::ONESHOT.0,
                );

                let f_x = Fixed::from_f64(x as f64 / pointer_scale.x);
                let f_y = Fixed::from_f64(y as f64 / pointer_scale.y);
                warp_lock.set_cursor_position_hint(f_x, f_y);
                surface.commit();

                drop(warp_lock);
            }

            let send_warp = p.version() < WlPointer::WARP_SINCE_VERSION;
            Some((update_grabs, send_warp, s.pointer.sdl_id))
        }) else {
            return;
        };

        if update_grabs {
            self.wayland_seat_update_pointer_grab(seat);
        }

        if send_warp {
            mouse::send_mouse_warp(std::time::Duration::ZERO, Some(window), sdl_id, x, y);
        }
    }

    /// Translation of `Wayland_WarpMouseRelative()`.
    pub(crate) fn wayland_warp_mouse_relative(
        &self,
        window: WindowID,
        x: f32,
        y: f32,
    ) -> Result<()> {
        let (supported, seats) = self.with_data(|d| {
            (
                d.g.wp_pointer_warp_v1.is_some() || d.g.pointer_constraints.is_some(),
                d.seat_list
                    .iter()
                    .filter(|s| s.pointer.focus == Some(window))
                    .map(|s| s.registry_id)
                    .collect::<Vec<_>>(),
            )
        });
        if supported {
            for seat in seats {
                self.wayland_seat_warp_mouse(seat, window, x, y);
            }
        } else {
            return Err(Error::new("wayland: mouse warp failed; compositor lacks support for the required wp_pointer_warp_v1 or zwp_pointer_confinement_v1 protocol"));
        }

        Ok(())
    }

    /// Translation of `Wayland_WarpMouseGlobal()`.
    pub(crate) fn wayland_warp_mouse_global(&self, x: f32, y: f32) -> Result<()> {
        let (supported, seats) = self.with_data(|d| {
            (
                d.g.wp_pointer_warp_v1.is_some() || d.g.pointer_constraints.is_some(),
                d.seat_list
                    .iter()
                    .map(|s| (s.registry_id, s.pointer.focus.or(s.keyboard.focus)))
                    .collect::<Vec<_>>(),
            )
        });
        if supported {
            for (seat, wind) in seats {
                // If the client wants the coordinates warped to within a focused window, just convert the coordinates to relative.
                let Some(window) = wind else {
                    continue;
                };
                let Ok((wx, wy, ww, wh)) =
                    with_window(window, |w| (w.core.x, w.core.y, w.core.w, w.core.h))
                else {
                    continue;
                };

                let (abs_x, abs_y) =
                    crate::video::window::relative_to_global_for_window(window, wx, wy);

                let p = (x, y);
                let r = (abs_x as f32, abs_y as f32, ww as f32, wh as f32);

                // Try to warp the cursor if the point is within the seat's focused window.
                if p.0 >= r.0 && p.0 < r.0 + r.2 && p.1 >= r.1 && p.1 < r.1 + r.3 {
                    self.wayland_seat_warp_mouse(
                        seat,
                        window,
                        p.0 - abs_x as f32,
                        p.1 - abs_y as f32,
                    );
                }
            }
        } else {
            return Err(Error::new("wayland: mouse warp failed; compositor lacks support for the required wp_pointer_warp_v1 or zwp_pointer_confinement_v1 protocol"));
        }

        Ok(())
    }

    /// Translation of `Wayland_SetRelativeMouseMode()`.
    pub(crate) fn wayland_set_relative_mouse_mode(&self, _enabled: bool) -> Result<()> {
        let (relative, constraints) = self.with_data(|d| {
            (
                d.g.relative_pointer_manager.is_some(),
                d.g.pointer_constraints.is_some(),
            )
        });

        // Relative mode requires both the relative motion and pointer confinement protocols.
        if !relative {
            return Err(Error::new("Failed to enable relative mode: compositor lacks support for the required zwp_relative_pointer_manager_v1 protocol"));
        }
        if !constraints {
            return Err(Error::new("Failed to enable relative mode: compositor lacks support for the required zwp_pointer_constraints_v1 protocol"));
        }

        // Windows have a relative mode flag, so just update the grabs on a state change.
        self.wayland_display_update_pointer_grabs(None);
        Ok(())
    }

    /// Wayland doesn't support getting the true global cursor position, but
    /// it can be faked well enough for what most applications use it for:
    /// querying the global cursor coordinates and transforming them to the
    /// window-relative coordinates manually.
    ///
    /// The global position is derived by taking the cursor position relative
    /// to the toplevel window, and offsetting it by the origin of the output
    /// the window is currently considered to be on. The cursor position and
    /// button state when the cursor is outside an application window are
    /// unknown, but this gives 'correct' coordinates when the window has
    /// focus, which is good enough for most applications.
    ///
    /// Translation of `Wayland_GetGlobalMouseState()`.
    pub(crate) fn wayland_get_global_mouse_state(&self) -> (f32, f32, MouseButtonFlags) {
        let (focus, mx, my) = mouse::with_mouse(|m| (m.focus, m.x, m.y));

        // If there is no window with mouse focus, we have no idea what the actual position or button state is.
        if let Some(focus) = focus {
            let (fx, fy) = with_window(focus, |w| (w.core.x, w.core.y)).unwrap_or((0, 0));
            let (off_x, off_y) = crate::video::window::relative_to_global_for_window(focus, fx, fy);
            let x = mx + off_x as f32;
            let y = my + off_y as f32;

            // Query the buttons from the seats directly, as this may be called from within a hit test handler.
            let result = self.with_data(|d| {
                d.seat_list
                    .iter()
                    .fold(0u32, |acc, s| acc | s.pointer.buttons_pressed)
            });
            (x, y, MouseButtonFlags(result))
        } else {
            (0.0, 0.0, MouseButtonFlags::NONE)
        }
    }

    /// Translation of `Wayland_InitMouse()`.
    pub(crate) fn wayland_init_mouse(&self) {
        // (the mouse entry points are the VideoDriver implementation, with
        // have_explicit_warp_event)

        let sys_cursors: [Option<Cursor>; HitTestResult::ResizeLeft as usize + 1] = [
            HitTestResult::Normal,
            HitTestResult::Draggable,
            HitTestResult::ResizeTopLeft,
            HitTestResult::ResizeTop,
            HitTestResult::ResizeTopRight,
            HitTestResult::ResizeRight,
            HitTestResult::ResizeBottomRight,
            HitTestResult::ResizeBottom,
            HitTestResult::ResizeBottomLeft,
            HitTestResult::ResizeLeft,
        ]
        .map(|r| {
            let id = match r {
                HitTestResult::Normal => SystemCursor::Default,
                HitTestResult::Draggable => SystemCursor::Default,
                HitTestResult::ResizeTopLeft => SystemCursor::NwResize,
                HitTestResult::ResizeTop => SystemCursor::NResize,
                HitTestResult::ResizeTopRight => SystemCursor::NeResize,
                HitTestResult::ResizeRight => SystemCursor::EResize,
                HitTestResult::ResizeBottomRight => SystemCursor::SeResize,
                HitTestResult::ResizeBottom => SystemCursor::SResize,
                HitTestResult::ResizeBottomLeft => SystemCursor::SwResize,
                HitTestResult::ResizeLeft => SystemCursor::WResize,
            };
            Some(self.wayland_create_system_cursor(id))
        });

        /* The D-Bus cursor properties are only needed when manually loading themes and system cursors.
         * If the cursor shape protocol is present, the compositor will handle it internally.
         */
        let has_shape = self.with_data(|d| {
            d.mouse.sys_cursors = sys_cursors;
            d.g.cursor_shape_manager.is_some()
        });
        if !has_shape {
            self.wayland_dbus_init_cursor_properties();
        }

        mouse::set_default_cursor(Some(self.wayland_create_default_cursor()));
    }

    /// Translation of `Wayland_DBusInitCursorProperties()`.
    fn wayland_dbus_init_cursor_properties(&self) {
        use crate::core::linux::dbus::{self, Value};

        let Some(ctx) = dbus::context() else {
            return;
        };
        let mut add_filter = false;

        if let Some(Value::I32(size)) = wayland_read_dbus_property(CURSOR_SIZE_KEY) {
            dbus_cursor().0 = size;
            add_filter = true;
        }

        if let Some(Value::Str(theme)) = wayland_read_dbus_property(CURSOR_THEME_KEY) {
            add_filter = true;
            dbus_cursor().1 = Some(theme);
        }

        // Only add the filter if at least one of the settings we want is present.
        if add_filter {
            let _ = ctx.session_conn.add_match(&format!(
                "type='signal', interface='{CURSOR_INTERFACE}',member='{CURSOR_SIGNAL_NAME}', arg0='{CURSOR_NAMESPACE}'"
            ));
            let weak = self.weak();
            // SAFETY: the filter borrows the context's connection; the
            // context is kept alive in the same struct, which drops the
            // filter first.
            let conn: &'static dbus::Connection =
                unsafe { &*(&ctx.session_conn as *const dbus::Connection) };
            if let Some(filter) =
                conn.add_filter(move |msg| wayland_dbus_cursor_message_filter(&weak, msg))
            {
                self.with_data(|d| {
                    d.mouse.dbus_filter = Some(DbusCursorFilter {
                        _filter: filter,
                        _ctx: ctx.clone(),
                    })
                });
            }
            ctx.session_conn.pump();
        }
    }

    /// Translation of `Wayland_FiniMouse()`.
    pub(crate) fn wayland_fini_mouse(&self) {
        let (cursors, themes, filter) = self.with_data(|d| {
            let context = self.event_thread();
            let _guard = lock_event_thread(&context);
            let cursors = std::mem::take(&mut d.mouse.sys_cursors);
            let themes = std::mem::take(&mut d.cursor_themes);
            THEME_GENERATION.fetch_add(1, Ordering::AcqRel);
            (cursors, themes, d.mouse.dbus_filter.take())
        });
        drop(cursors);
        drop(themes);

        // (Wayland_DBusFinishCursorProperties())
        drop(filter);
        dbus_cursor().1 = None;
    }

    /// Translation of `Wayland_SeatResetCursor()`.
    pub(crate) fn wayland_seat_reset_cursor(&self, seat: u32) {
        if let Some(cell) = self.with_data(|d| d.seat(seat).map(|s| s.pointer.cursor_state.clone()))
        {
            self.wayland_cursor_state_reset_cursor(&cell);
        }
    }

    /// Translation of `Wayland_SeatSetDefaultCursor()`.
    pub(crate) fn wayland_seat_set_default_cursor(&self, seat: u32) {
        let def_cursor = mouse::default_cursor();
        self.with_data(|d| {
            let Some(s) = d.seat(seat) else {
                return;
            };
            let pointer_focus = s.pointer.focus;
            let enter_serial = s.pointer.enter_serial;
            let cell = s.pointer.cursor_state.clone();
            let Some(pointer) = s.pointer.wl_pointer.as_ref().map(|p| p.raw() as usize) else {
                return;
            };
            let f = d.cursor_focus(pointer_focus);
            // SAFETY: the seat's pointer is alive.
            if let Some(p) = unsafe { Obj::<WlPointer>::from_raw(pointer as *mut _, &self.conn) } {
                self.wayland_cursor_state_set_cursor(
                    d,
                    &cell,
                    PointerObject::Pointer(p),
                    f,
                    enter_serial,
                    def_cursor.as_ref(),
                );
            }
        });
    }

    /// Translation of `Wayland_SeatUpdatePointerCursor()`.
    pub(crate) fn wayland_seat_update_pointer_cursor(&self, seat: u32) {
        let (cursor_visible, relative_mode_hide_cursor, cur_cursor) = mouse::with_mouse(|m| {
            (
                m.cursor_visible,
                m.relative_mode_hide_cursor,
                m.cur_cursor.clone(),
            )
        });

        self.with_data(|d| {
            let Some(s) = d.seat(seat) else {
                return;
            };
            let pointer_focus = s.pointer.focus;
            let enter_serial = s.pointer.enter_serial;
            let relative = s.pointer.relative_pointer.is_some();
            let cell = s.pointer.cursor_state.clone();
            let pointer = s.pointer.wl_pointer.as_ref().map(|p| p.raw() as usize);

            let Some(focus) = pointer_focus else {
                self.wayland_cursor_state_reset_cursor(&cell);
                return;
            };
            let Some(pointer) = pointer else {
                return;
            };
            let rc = d
                .window(focus)
                .map_or(HitTestResult::Normal, |w| w.hit_test_result);
            let f = d.cursor_focus(pointer_focus);
            // SAFETY: the seat's pointer is alive.
            let Some(obj) = (unsafe { Obj::<WlPointer>::from_raw(pointer as *mut _, &self.conn) })
            else {
                return;
            };
            let obj = PointerObject::Pointer(obj);

            if cursor_visible {
                if !relative || !relative_mode_hide_cursor {
                    if relative || rc == HitTestResult::Normal || rc == HitTestResult::Draggable {
                        self.wayland_cursor_state_set_cursor(
                            d,
                            &cell,
                            obj,
                            f,
                            enter_serial,
                            cur_cursor.as_ref(),
                        );
                    } else {
                        let sys = d.mouse.sys_cursors[rc as usize].clone();
                        self.wayland_cursor_state_set_cursor(
                            d,
                            &cell,
                            obj,
                            f,
                            enter_serial,
                            sys.as_ref(),
                        );
                    }
                } else {
                    // Hide the cursor in relative mode, unless requested otherwise by the hint.
                    self.wayland_cursor_state_set_cursor(d, &cell, obj, f, enter_serial, None);
                }
            } else {
                self.wayland_cursor_state_set_cursor(d, &cell, obj, f, enter_serial, None);
            }
        });
    }

    /// Translation of `Wayland_TabletToolUpdateCursor()`.
    pub(crate) fn wayland_tablet_tool_update_cursor(&self, seat: u32, tool: usize) {
        let (cursor_visible, relative_mode_hide_cursor, pen_mouse_events, cur_cursor) =
            mouse::with_mouse(|m| {
                (
                    m.cursor_visible,
                    m.relative_mode_hide_cursor,
                    m.pen_mouse_events,
                    m.cur_cursor.clone(),
                )
            });

        let Some((tool_focus, proximity_serial, cell)) = self.with_data(|d| {
            let t = d.seat(seat)?.tool(tool)?;
            Some((t.focus, t.proximity_serial, t.cursor_state.clone()))
        }) else {
            return;
        };

        let Some(focus) = tool_focus else {
            self.wayland_cursor_state_reset_cursor(&cell);
            return;
        };

        // Relative mode is only relevant if the tool sends pointer events.
        let relative = pen_mouse_events
            && with_window(focus, |w| {
                w.flags().contains(WindowFlags::MOUSE_RELATIVE_MODE)
            })
            .unwrap_or(false);

        self.with_data(|d| {
            let f = d.cursor_focus(tool_focus);
            // SAFETY: the tool is alive (in its seat's list).
            let Some(t) = (unsafe { Obj::<ZwpTabletToolV2>::from_raw(tool as *mut _, &self.conn) })
            else {
                return;
            };
            let obj = PointerObject::Tool(t);
            if cursor_visible {
                if !relative || !relative_mode_hide_cursor {
                    self.wayland_cursor_state_set_cursor(
                        d,
                        &cell,
                        obj,
                        f,
                        proximity_serial,
                        cur_cursor.as_ref(),
                    );
                } else {
                    // Hide the cursor in relative mode, unless requested otherwise by the hint.
                    self.wayland_cursor_state_set_cursor(d, &cell, obj, f, proximity_serial, None);
                }
            } else {
                self.wayland_cursor_state_set_cursor(d, &cell, obj, f, proximity_serial, None);
            }
        });
    }
}

/// Translation of `Wayland_CacheSystemCursor()`: the theme's buffers for a
/// cursor at `size`, cached in the cursor.
fn wayland_cache_system_cursor(
    syms: &WaylandSyms,
    cdata: &mut SystemCursorData,
    images: &[&wl_cursor_image],
    size: i32,
    generation: u64,
) -> Vec<usize> {
    // Is this cursor already cached at the target scale?
    // FIXME (upstream): the cache is keyed by the unscaled theme size, so
    // after a scale change the buffers of the old scale are reused.
    if let Some(c) = cdata
        .cursor_buffer_cache
        .iter()
        .find(|c| c.size == size && c.generation == generation)
    {
        return c.buffers.clone();
    }
    // (entries of freed themes are dropped)
    cdata
        .cursor_buffer_cache
        .retain(|c| c.generation == generation);

    let buffers: Vec<usize> = images
        .iter()
        .map(|image| {
            // SAFETY: the image belongs to a loaded theme.
            unsafe {
                (syms.cursor.wl_cursor_image_get_buffer)(*image as *const wl_cursor_image as *mut _)
                    as usize
            }
        })
        .collect();

    cdata.cursor_buffer_cache.insert(
        0,
        CachedSystemCursor {
            size,
            generation,
            buffers: buffers.clone(),
        },
    );

    buffers
}

/// Translation of `Wayland_GetSystemCursorShape()`.
fn wayland_get_system_cursor_shape(id: SystemCursor) -> WpCursorShapeDeviceV1Shape {
    use SystemCursor as C;
    use WpCursorShapeDeviceV1Shape as S;
    match id {
        C::Default => S::DEFAULT,
        C::Text => S::TEXT,
        C::Wait => S::WAIT,
        C::Crosshair => S::CROSSHAIR,
        C::Progress => S::PROGRESS,
        C::NwseResize => S::NWSE_RESIZE,
        C::NeswResize => S::NESW_RESIZE,
        C::EwResize => S::EW_RESIZE,
        C::NsResize => S::NS_RESIZE,
        C::Move => S::MOVE,
        C::NotAllowed => S::NOT_ALLOWED,
        C::Pointer => S::POINTER,
        C::NwResize => S::NW_RESIZE,
        C::NResize => S::N_RESIZE,
        C::NeResize => S::NE_RESIZE,
        C::EResize => S::E_RESIZE,
        C::SeResize => S::SE_RESIZE,
        C::SResize => S::S_RESIZE,
        C::SwResize => S::SW_RESIZE,
        C::WResize => S::W_RESIZE,
        C::ContextMenu => S::CONTEXT_MENU,
        C::Help => S::HELP,
        C::Cell => S::CELL,
        C::VerticalText => S::VERTICAL_TEXT,
        C::Alias => S::ALIAS,
        C::Copy => S::COPY,
        C::NoDrop => S::NO_DROP,
        C::Grab => S::GRAB,
        C::Grabbing => S::GRABBING,
        C::ColResize => S::COL_RESIZE,
        C::RowResize => S::ROW_RESIZE,
        C::AllScroll => S::ALL_SCROLL,
        C::ZoomIn => S::ZOOM_IN,
        C::ZoomOut => S::ZOOM_OUT,
    }
}
