// Rust translation of the device, fullscreen and window state parts of
// src/video/SDL_video.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The video device: driver selection, init and quit, the state shared by
//! windows and displays, the window state callbacks (`SDL_OnWindow*()`),
//! fullscreen handling, input grabs, and the [`VideoHooks`] the event core
//! reaches windows through.
//!
//! The device (upstream's `_this`) lives behind a re-entrant lock; it is
//! borrowed only for short reads and writes, never across a driver call or
//! an event, since both may come back into the video API.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::events::mouse::{self, MouseButtonFlags, MouseFeature};
use crate::events::window::{send_window_event, DisplayCore, VideoHooks, WindowCore, WindowFlags};
use crate::events::{keyboard, DisplayID, EventType, WindowID};
use crate::hints;
use crate::thread::{current_thread_id, ReentrantMutex, ThreadID};
use crate::video::Rect;

use super::display::{self, display_bounds, display_index, primary_display};
use super::drivers;
use super::sysvideo::{
    CurrentMode, DeviceCaps, FullscreenOp, FullscreenResult, SystemTheme, VideoBootStrap,
    VideoDisplay, VideoDriver, WindowData,
};
use super::window;

/// The video device. Translation of the data of `struct SDL_VideoDevice`
/// common to all drivers (the function pointers are [`VideoDriver`]).
pub(crate) struct VideoDevice {
    /// The name of this video driver
    pub(crate) name: &'static str,
    pub(crate) driver: Arc<dyn VideoDriver>,
    pub(crate) caps: DeviceCaps,
    pub(crate) thread: ThreadID,
    pub(crate) checked_texture_framebuffer: bool,
    pub(crate) suspend_screensaver: bool,
    pub(crate) wakeup_window: Option<WindowID>,
    pub(crate) displays: Vec<VideoDisplay>,
    pub(crate) desktop_bounds: Rect,
    /// The windows, the most recently created first.
    pub(crate) windows: Vec<WindowData>,
    pub(crate) grabbed_window: Option<WindowID>,
    pub(crate) setting_display_mode: bool,
    pub(crate) system_theme: SystemTheme,
    pub(crate) screen_keyboard_shown: bool,
    pub(crate) is_quitting: bool,
    /// `gl_config.driver_loaded`
    pub(crate) gl_driver_loaded: i32,
    pub(crate) gl_driver_path: Option<String>,
    /// `vulkan_config.loader_loaded`
    pub(crate) vulkan_loader_loaded: i32,
    pub(crate) vulkan_loader_path: Option<String>,
    sync_hint_watch: Option<hints::Callback>,
}

/// Translation of `_this`.
static VIDEO_DEVICE: ReentrantMutex<RefCell<Option<VideoDevice>>> =
    ReentrantMutex::new(RefCell::new(None));

/// Available video drivers. Translation of `bootstrap`.
#[cfg(not(test))]
static BOOTSTRAP: &[&VideoBootStrap] = &[
    &drivers::offscreen::OFFSCREEN_BOOTSTRAP,
    &drivers::dummy::DUMMY_BOOTSTRAP,
];
#[cfg(test)]
static BOOTSTRAP: &[&VideoBootStrap] = &[
    &drivers::offscreen::OFFSCREEN_BOOTSTRAP,
    &drivers::dummy::DUMMY_BOOTSTRAP,
    &super::tests::TESTVIDEO_BOOTSTRAP,
];

/// The video bootstrap list (for message boxes shown without video).
pub(crate) fn video_bootstraps() -> &'static [&'static VideoBootStrap] {
    BOOTSTRAP
}

/// Hint to treat all window ops as synchronous. Translation of `syncHint`.
static SYNC_HINT: AtomicBool = AtomicBool::new(false);

/// The error for using video before it is initialized. Translation of
/// `SDL_UninitializedVideo()`.
pub(crate) fn uninitialized_video() -> Error {
    Error::new("Video subsystem has not been initialized")
}

/// Run `f` on the video device.
pub(crate) fn with_device<R>(f: impl FnOnce(&mut VideoDevice) -> R) -> Result<R> {
    let guard = VIDEO_DEVICE.lock();
    let mut device = guard.borrow_mut();
    device.as_mut().map(f).ok_or_else(uninitialized_video)
}

/// Whether the video subsystem is up (`_this != NULL`).
pub(crate) fn initialized() -> bool {
    VIDEO_DEVICE.lock().borrow().is_some()
}

/// The driver's entry points (cloned out, so they are called with nothing
/// borrowed).
pub(crate) fn driver() -> Result<Arc<dyn VideoDriver>> {
    with_device(|v| v.driver.clone())
}

/// The device flags (none if video isn't up).
pub(crate) fn caps() -> DeviceCaps {
    with_device(|v| v.caps).unwrap_or_default()
}

/// Run `f` on a window's state, failing like `CHECK_WINDOW_MAGIC`.
pub(crate) fn with_window<R>(id: WindowID, f: impl FnOnce(&mut WindowData) -> R) -> Result<R> {
    with_device(|v| {
        v.windows
            .iter_mut()
            .find(|w| w.core.id == id)
            .map(f)
            .ok_or_else(|| Error::new("Invalid window"))
    })?
}

/// Run `f` on a display's state; `None` if there is no such display.
pub(crate) fn with_display<R>(id: DisplayID, f: impl FnOnce(&mut VideoDisplay) -> R) -> Option<R> {
    with_device(|v| v.displays.iter_mut().find(|d| d.id == id).map(f))
        .ok()
        .flatten()
}

/// The IDs of all windows, the most recently created first.
pub(crate) fn window_ids() -> Vec<WindowID> {
    with_device(|v| v.windows.iter().map(|w| w.core.id).collect()).unwrap_or_default()
}

/// Deduplicated list of video bootstrap drivers. Translation of
/// `deduped_bootstrap`.
fn deduped_bootstrap() -> Vec<&'static VideoBootStrap> {
    let mut out: Vec<&'static VideoBootStrap> = Vec::new();
    // Build a list of unique video drivers.
    for (i, b) in BOOTSTRAP.iter().enumerate() {
        if !BOOTSTRAP[..i].iter().any(|o| o.name == b.name) {
            out.push(b);
        }
    }
    out
}

/// The number of video drivers compiled into SDL. Translation of
/// `SDL_GetNumVideoDrivers()`.
pub fn num_video_drivers() -> usize {
    deduped_bootstrap().len()
}

/// The name of a built in video driver ("x11", "wayland", "dummy"...).
/// Translation of `SDL_GetVideoDriver()`.
pub fn video_driver(index: usize) -> Result<&'static str> {
    deduped_bootstrap()
        .get(index)
        .map(|b| b.name)
        .ok_or_else(|| Error::invalid_param("index"))
}

/// The name of the current video driver. Translation of
/// `SDL_GetCurrentVideoDriver()`.
pub fn current_video_driver() -> Result<&'static str> {
    with_device(|v| v.name)
}

/// Translation of `SDL_IsUbuntuTouch()`.
fn is_ubuntu_touch() -> bool {
    cfg!(target_os = "linux")
        && crate::stdlib::getenv("XDG_SESSION_DESKTOP").as_deref() == Some("ubuntu-touch")
}

/// Initialize the video and event subsystems: pick a driver (`driver_name`,
/// a comma-separated list to try, or the [`hints::VIDEO_DRIVER`] hint, or
/// the first that works) and set it up. Translation of `SDL_InitVideo()`.
pub(crate) fn init_video(driver_name: Option<&str>) -> Result<()> {
    // Check to make sure we don't overwrite '_this'
    if initialized() {
        quit_video();
    }

    crate::timer::ticks_ns(); // (SDL_InitTicks())

    // Start the event loop
    crate::init::init_subsystem(crate::init::InitFlags::EVENTS)?;
    let mut inited = 0;
    let pre_driver_error = |inited: i32, e: Error| -> Result<()> {
        if inited >= 4 {
            crate::events::pen::quit_pen();
        }
        if inited >= 3 {
            crate::events::touch::quit_touch();
        }
        if inited >= 2 {
            mouse::quit_mouse();
        }
        if inited >= 1 {
            keyboard::quit_keyboard();
        }
        crate::init::quit_subsystem(crate::init::InitFlags::EVENTS);
        Err(e)
    };
    if let Err(e) = keyboard::init_keyboard() {
        return pre_driver_error(inited, e);
    }
    inited += 1;
    if let Err(e) = mouse::pre_init_mouse() {
        return pre_driver_error(inited, e);
    }
    inited += 1;
    if let Err(e) = crate::events::touch::init_touch() {
        return pre_driver_error(inited, e);
    }
    inited += 1;
    if let Err(e) = crate::events::pen::init_pen() {
        return pre_driver_error(inited, e);
    }
    inited += 1;

    // Select the proper video driver
    let mut driver_name = driver_name.map(str::to_owned);
    // https://github.com/libsdl-org/SDL/issues/12247
    if is_ubuntu_touch() {
        driver_name = Some("wayland".to_owned());
    }
    if driver_name.is_none() {
        driver_name = hints::get(hints::VIDEO_DRIVER);
    }
    let mut video = None;
    match driver_name.as_deref() {
        Some(name) if !name.is_empty() => {
            for driver_attempt in name.split(',') {
                if video.is_some() || driver_attempt.is_empty() {
                    break;
                }
                for b in BOOTSTRAP {
                    if !b.is_preferred && b.name.eq_ignore_ascii_case(driver_attempt) {
                        if let Some(device) = (b.create)() {
                            video = Some((*b, device));
                            break;
                        }
                    }
                }
            }
        }
        _ => {
            for b in BOOTSTRAP {
                if let Some(device) = (b.create)() {
                    video = Some((*b, device));
                    break;
                }
            }
        }
    }
    let Some((bootstrap, driver)) = video else {
        return pre_driver_error(
            inited,
            match driver_name {
                Some(name) => Error::new(format!("{name} not available")),
                None => Error::new("No available video device"),
            },
        );
    };
    crate::debug!(
        crate::log::Category::Video,
        "SDL chose video backend '{}'",
        bootstrap.name
    );

    /* From this point on, use quit_video to cleanup on error, rather than
    pre_driver_error. */
    *VIDEO_DEVICE.lock().borrow_mut() = Some(VideoDevice {
        name: bootstrap.name,
        caps: driver.caps(),
        driver: driver.clone(),
        thread: current_thread_id(),
        checked_texture_framebuffer: false,
        suspend_screensaver: false,
        wakeup_window: None,
        displays: Vec::new(),
        desktop_bounds: Rect::default(),
        windows: Vec::new(),
        grabbed_window: None,
        setting_display_mode: false,
        system_theme: SystemTheme::Unknown,
        screen_keyboard_shown: false,
        is_quitting: false,
        gl_driver_loaded: 0,
        gl_driver_path: None,
        vulkan_loader_loaded: 0,
        vulkan_loader_path: None,
        sync_hint_watch: None,
    });
    crate::events::window::set_video(Some(Arc::new(Hooks)));

    // Set some very sane GL defaults
    super::gl::gl_reset_attributes();

    // Initialize the video subsystem
    if let Err(e) = driver.video_init() {
        quit_video();
        return Err(e);
    }

    // Make sure some displays were added
    if with_device(|v| v.displays.is_empty()).unwrap_or(true) {
        quit_video();
        return Err(Error::new("The video driver did not add any displays"));
    }

    let watch = hints::watch(hints::VIDEO_SYNC_WINDOW_OPERATIONS, |change| {
        SYNC_HINT.store(
            hints::string_to_bool(change.new_value, false),
            Ordering::Relaxed,
        );
    })
    .ok();
    let _ = with_device(|v| v.sync_hint_watch = watch);

    /* Disable the screen saver by default. This is a change from <= 2.0.1,
    but most things using SDL are games or media players; you wouldn't
    want a screensaver to trigger if you're playing exclusively with a
    joystick, or passively watching a movie. Things that use SDL but
    function more like a normal desktop app should explicitly re-enable the
    screensaver. */
    if !hints::get_bool(hints::VIDEO_ALLOW_SCREENSAVER, false) {
        let _ = disable_screen_saver();
    }

    mouse::post_init_mouse();

    // We're ready to go!
    Ok(())
}

/// Whether the caller is on the thread that initialized video.
/// Translation of `SDL_OnVideoThread()`.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn on_video_thread() -> bool {
    with_device(|v| v.thread == current_thread_id()).unwrap_or(false)
}

/// Translation of `SDL_SetSystemTheme()`.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn set_system_theme(theme: SystemTheme) {
    let changed = with_device(|v| {
        if theme != v.system_theme {
            v.system_theme = theme;
            true
        } else {
            false
        }
    })
    .unwrap_or(false);
    if changed {
        crate::events::send_system_theme_changed_event();
    }
}

/// The current system theme. Translation of `SDL_GetSystemTheme()`.
pub fn system_theme() -> SystemTheme {
    with_device(|v| v.system_theme).unwrap_or(SystemTheme::Unknown)
}

/// Translation of `SDL_SyncIfRequired()`.
pub(crate) fn sync_if_required(window: WindowID) {
    if SYNC_HINT.load(Ordering::Relaxed) {
        let _ = window::sync_window(window);
    }
}

/// Re-parent a window in the window hierarchy. Translation of
/// `SDL_UpdateWindowHierarchy()`.
pub(crate) fn update_window_hierarchy(window: WindowID, parent: Option<WindowID>) {
    let _ = with_device(|v| {
        // Unlink the window from the existing parent.
        let old_parent = v
            .windows
            .iter_mut()
            .find(|w| w.core.id == window)
            .and_then(|w| w.parent.take());
        if let Some(old_parent) = old_parent {
            if let Some(p) = v.windows.iter_mut().find(|w| w.core.id == old_parent) {
                p.children.retain(|&c| c != window);
            }
        }

        if let Some(parent) = parent {
            if let Some(p) = v.windows.iter_mut().find(|w| w.core.id == parent) {
                p.children.insert(0, window);
            }
        }
        if let Some(w) = v.windows.iter_mut().find(|w| w.core.id == window) {
            w.parent = parent;
            w.core.has_parent = parent.is_some();
        }
    });
}

/// The display a window is in, by its position. Translation of
/// `SDL_GetDisplayForWindowPosition()`.
pub(crate) fn display_for_window_position(window: WindowID) -> Result<DisplayID> {
    let (x, y, w, h, fullscreen) = with_window(window, |w| {
        (w.core.x, w.core.y, w.core.w, w.core.h, w.flags())
    })?;

    let mut display_id = driver()?.display_for_window(window).unwrap_or(0);

    /* A backend implementation may fail to get a display for the window
     * (for example if the window is off-screen), but other code may expect it
     * to succeed in that situation, so we fall back to a generic position-
     * based implementation in that case. */
    let (x, y) = window::relative_to_global_for_window(window, x, y);

    if display_id == 0 {
        /* Fullscreen windows may be larger than the display if they were moved between differently sized
         * displays and the new position was received before the new size or vice versa. Using the center
         * of the window rect in this case can report the wrong display, so use the origin.
         */
        display_id = if fullscreen.contains(WindowFlags::FULLSCREEN) {
            display::display_for_rect_xywh(x, y, 1, 1)
        } else {
            display::display_for_rect_xywh(x, y, w, h)
        };
    }
    if display_id == 0 {
        // Use the primary display for a window if we can't find it anywhere else
        display_id = primary_display().unwrap_or(0);
    }
    Ok(display_id)
}

/// The display a window should go fullscreen on. Translation of
/// `SDL_GetVideoDisplayForFullscreenWindow()` (returning the display's ID,
/// 0 if none).
pub(crate) fn display_for_fullscreen_window(window: WindowID) -> Result<DisplayID> {
    let (current, pending_display, x, y, w, h) = with_window(window, |w| {
        (
            w.current_fullscreen_mode.display_id,
            w.core.pending_display_id,
            if w.core.last_position_pending {
                w.pending.x
            } else {
                w.core.x
            },
            if w.core.last_position_pending {
                w.pending.y
            } else {
                w.core.y
            },
            if w.core.last_size_pending {
                w.pending.w
            } else {
                w.core.w
            },
            if w.core.last_size_pending {
                w.pending.h
            } else {
                w.core.h
            },
        )
    })?;

    // An explicit fullscreen display overrides all
    let mut display_id = current;

    /* This is used to handle the very common pattern of SDL_SetWindowPosition()
     * followed immediately by SDL_SetWindowFullscreen() to make the window fullscreen
     * desktop on a specific display. If the backend doesn't support changing the
     * window position, or an async window manager hasn't yet actually moved the window,
     * the current position won't be updated at the time of the fullscreen call.
     */
    if display_id == 0 {
        display_id = pending_display;
    }
    if display_id == 0 {
        // Check if the window is exactly at the origin of a display. Otherwise, fall back to the generic check.
        display_id = display::display_at_origin(x, y);
        if display_id == 0 {
            display_id = display::display_for_rect_xywh(x, y, w, h);
        }
    }
    if display_id == 0 {
        // Use the primary display for a window if we can't find it anywhere else
        display_id = primary_display().unwrap_or(0);
    }
    Ok(if display_index(display_id).is_ok() {
        display_id
    } else {
        0
    })
}

/// Translation of `SDL_CheckWindowDisplayChanged()`.
pub(crate) fn check_window_display_changed(window: WindowID) {
    if caps().contains(DeviceCaps::SENDS_DISPLAY_CHANGES) {
        return;
    }

    let Ok(display_id) = display_for_window_position(window) else {
        return;
    };
    let Ok((old_display, window_rect)) = with_window(window, |w| {
        (
            w.core.display_id,
            Rect::new(w.core.x, w.core.y, w.core.w, w.core.h),
        )
    }) else {
        return;
    };

    if display_id != old_display {
        // See if we are fully committed to the new display
        // 80% is about the right value, tested with 350% scale on the left monitor and 100% scale on the right
        if let (Ok(old_bounds), Ok(new_bounds)) =
            (display_bounds(old_display), display_bounds(display_id))
        {
            if let (Some(old_overlap), Some(new_overlap)) = (
                old_bounds.intersection(&window_rect),
                new_bounds.intersection(&window_rect),
            ) {
                let old_area = old_overlap.w * old_overlap.h;
                let new_area = new_overlap.w * new_overlap.h;
                let new_overlap_ratio = new_area as f32 / (old_area as f32 + new_area as f32);
                if new_overlap_ratio < 0.80 {
                    return;
                }
            }
        }
    }

    if display_id != old_display {
        // Sanity check our fullscreen windows
        let mut minimize = None;
        let _ = with_device(|v| {
            let display_index = v.displays.iter().position(|d| d.id == display_id);
            for i in 0..v.displays.len() {
                if v.displays[i].fullscreen_window == Some(window) {
                    if display_index != Some(i) {
                        match display_index {
                            // (upstream sets display_index here, unused afterwards)
                            None => {}
                            Some(new_index) => {
                                // The window was moved to a different display
                                let new_display = &mut v.displays[new_index];
                                if let Some(other) = new_display.fullscreen_window {
                                    if other != window {
                                        // Uh oh, there's already a fullscreen window here; minimize it
                                        minimize = Some(other);
                                    }
                                }
                                new_display.fullscreen_window = Some(window);
                                v.displays[i].fullscreen_window = None;
                            }
                        }
                    }
                    break;
                }
            }
        });
        if let Some(other) = minimize {
            let _ = window::minimize_window(other);
        }

        send_window_event(
            window,
            EventType::WINDOW_DISPLAY_CHANGED,
            display_id as i32,
            0,
        );
    }
}

/// Translation of `SDL_CheckWindowDisplayScaleChanged()`.
pub(crate) fn check_window_display_scale_changed(window: WindowID) {
    let Ok(driver) = driver() else { return };
    let display_scale = match driver.window_content_scale(window) {
        Some(scale) => scale,
        None => {
            let pixel_density = window::window_pixel_density(window).unwrap_or(1.0);
            let display_id = with_window(window, |w| w.core.display_id).unwrap_or(0);
            let content_scale = display::display_content_scale(display_id).unwrap_or(0.0);

            pixel_density * content_scale
        }
    };

    let changed = with_window(window, |w| {
        if display_scale != w.display_scale {
            w.display_scale = display_scale;
            true
        } else {
            false
        }
    })
    .unwrap_or(false);
    if changed {
        send_window_event(window, EventType::WINDOW_DISPLAY_SCALE_CHANGED, 0, 0);
    }
}

/// Move a window into or out of fullscreen (or update its fullscreen mode)
/// on the right display; with `commit`, also tell the backend.
/// Translation of `SDL_UpdateFullscreenMode()`.
pub(crate) fn update_fullscreen_mode(
    window: WindowID,
    mut fullscreen: FullscreenOp,
    commit: bool,
) -> Result<()> {
    let (destroying_or_hiding,) = with_window(window, |w| {
        w.fullscreen_exclusive = false;
        w.core.update_fullscreen_on_display_changed = false;
        (w.core.is_destroying || w.is_hiding,)
    })?;

    // If we are in the process of hiding don't go back to fullscreen
    if destroying_or_hiding {
        fullscreen = FullscreenOp::Leave;
    }
    let entering = fullscreen != FullscreenOp::Leave;

    let driver = driver()?;

    // Get the correct display for this operation
    let display: Option<DisplayID> = if entering {
        match display_for_fullscreen_window(window)? {
            0 => {
                // This should never happen, but it did...
                return finish_update_fullscreen(window, None);
            }
            id => Some(id),
        }
    } else {
        // (None if already not fullscreen on any display)
        with_device(|v| {
            v.displays
                .iter()
                .find(|d| d.fullscreen_window == Some(window))
                .map(|d| d.id)
        })?
    };

    let mut mode = None;
    if entering {
        mode = window::window_fullscreen_mode_match(window)?;
        with_window(window, |w| {
            if mode.is_some() {
                w.fullscreen_exclusive = true;
            } else {
                // Make sure the current mode is zeroed for fullscreen desktop.
                w.current_fullscreen_mode = Default::default();
            }
        })?;
    }

    if let Some(display) = display {
        // Restore the video mode on other displays if needed
        let others: Vec<DisplayID> = with_device(|v| {
            v.displays
                .iter()
                .filter(|d| d.id != display && d.fullscreen_window == Some(window))
                .map(|d| d.id)
                .collect()
        })?;
        for other in others {
            let _ = display::set_display_mode_for_display(other, None);
            with_display(other, |d| d.fullscreen_window = None);
        }
    }

    let flags = |window: WindowID| with_window(window, |w| w.flags()).unwrap_or_default();
    let sends_dimensions = caps().contains(DeviceCaps::SENDS_FULLSCREEN_DIMENSIONS);

    if entering {
        let display = display.unwrap_or_default();
        let mut resized = false;

        // Hide any other fullscreen window on this display
        let other = with_display(display, |d| d.fullscreen_window).flatten();
        if let Some(other) = other.filter(|&o| o != window) {
            let _ = window::minimize_window(other);
        }

        let exclusive = with_window(window, |w| w.fullscreen_exclusive)?;
        with_display(display, |d| d.fullscreen_active = exclusive);

        if let Err(e) =
            display::set_display_mode_for_display(display, mode.map(|m| m.for_display(display)))
        {
            return fullscreen_error(window, fullscreen, commit, e);
        }
        if commit {
            let ret = match driver.set_window_fullscreen(window, display, fullscreen) {
                Some(ret) => ret,
                None => {
                    resized = true;
                    FullscreenResult::Succeeded
                }
            };

            match ret {
                FullscreenResult::Succeeded => {
                    // Window is fullscreen immediately upon return. If the driver hasn't already sent the event, do so now.
                    if !flags(window).contains(WindowFlags::FULLSCREEN) {
                        send_window_event(window, EventType::WINDOW_ENTER_FULLSCREEN, 0, 0);
                    }
                }
                FullscreenResult::Failed => {
                    with_display(display, |d| d.fullscreen_active = false);
                    return fullscreen_error(
                        window,
                        fullscreen,
                        commit,
                        Error::new("Couldn't set the window fullscreen"),
                    );
                }
                FullscreenResult::Pending => {}
            }
        }

        if flags(window).contains(WindowFlags::FULLSCREEN) {
            with_display(display, |d| d.fullscreen_window = Some(window));

            /* Android may not resize the window to exactly what our fullscreen mode is,
             * especially on windowed Android environments like the Chromebook or Samsung DeX.
             * Given this, we shouldn't use the mode size. Android's SetWindowFullscreen
             * will generate the window event for us with the proper final size.
             *
             * This is also unnecessary on Cocoa, Wayland, Win32, and X11 (will send SDL_EVENT_WINDOW_RESIZED).
             */
            if !sends_dimensions {
                let (mode_w, mode_h, display_rect) = match mode {
                    Some(m) => (
                        m.mode.w,
                        m.mode.h,
                        display_bounds(m.mode.display_id).unwrap_or_default(),
                    ),
                    None => {
                        let desktop = display::desktop_display_mode(display).unwrap_or_default();
                        (
                            desktop.w,
                            desktop.h,
                            display_bounds(display).unwrap_or_default(),
                        )
                    }
                };

                let (w, h) = with_window(window, |w| (w.core.w, w.core.h))?;
                if w != mode_w || h != mode_h {
                    resized = true;
                }

                send_window_event(
                    window,
                    EventType::WINDOW_MOVED,
                    display_rect.x,
                    display_rect.y,
                );

                if resized {
                    send_window_event(window, EventType::WINDOW_RESIZED, mode_w, mode_h);
                } else {
                    on_window_resized(window);
                }
            }
        }
    } else {
        let mut resized = false;

        // Restore the desktop mode
        if let Some(display) = display {
            with_display(display, |d| d.fullscreen_active = false);

            let _ = display::set_display_mode_for_display(display, None);
        }
        if commit {
            let ret = match display.or_else(|| {
                display_for_fullscreen_window(window)
                    .ok()
                    .filter(|&d| d != 0)
            }) {
                Some(full_screen_display) => {
                    match driver.set_window_fullscreen(
                        window,
                        full_screen_display,
                        FullscreenOp::Leave,
                    ) {
                        Some(ret) => ret,
                        None => {
                            resized = true;
                            FullscreenResult::Succeeded
                        }
                    }
                }
                None => {
                    // (upstream: the backend isn't called without a display,
                    // but a missing SetWindowFullscreen still means resized)
                    // FIXME (upstream): with no display found and no backend
                    // call, the result stays "succeeded".
                    resized = true;
                    FullscreenResult::Succeeded
                }
            };

            match ret {
                FullscreenResult::Succeeded => {
                    // Window left fullscreen immediately upon return. If the driver hasn't already sent the event, do so now.
                    if flags(window).contains(WindowFlags::FULLSCREEN) {
                        send_window_event(window, EventType::WINDOW_LEAVE_FULLSCREEN, 0, 0);
                    }
                }
                FullscreenResult::Failed => {
                    return fullscreen_error(
                        window,
                        fullscreen,
                        commit,
                        Error::new("Couldn't leave fullscreen"),
                    );
                }
                FullscreenResult::Pending => {}
            }
        }

        if !flags(window).contains(WindowFlags::FULLSCREEN) {
            if let Some(display) = display {
                with_display(display, |d| d.fullscreen_window = None);
            }

            if !sends_dimensions {
                let windowed = with_window(window, |w| w.core.windowed)?;
                send_window_event(window, EventType::WINDOW_MOVED, windowed.x, windowed.y);
                if resized {
                    send_window_event(window, EventType::WINDOW_RESIZED, windowed.w, windowed.h);
                } else {
                    on_window_resized(window);
                }
            }
        }
    }

    finish_update_fullscreen(window, display)
}

/// The `done:` label of `SDL_UpdateFullscreenMode()`.
fn finish_update_fullscreen(window: WindowID, display: Option<DisplayID>) -> Result<()> {
    with_window(window, |w| {
        w.last_fullscreen_exclusive_display = match display {
            Some(d) if w.flags().contains(WindowFlags::FULLSCREEN) && w.fullscreen_exclusive => d,
            _ => 0,
        };
    })
}

/// The `error:` label of `SDL_UpdateFullscreenMode()`.
fn fullscreen_error(
    window: WindowID,
    fullscreen: FullscreenOp,
    commit: bool,
    e: Error,
) -> Result<()> {
    if fullscreen != FullscreenOp::Leave {
        // Something went wrong and the window is no longer fullscreen.
        let _ = update_fullscreen_mode(window, FullscreenOp::Leave, commit);
    }
    Err(e)
}

// ---------------------------------------------------------------------------
// Window state callbacks (called after a window event is posted)
// ---------------------------------------------------------------------------

/// Translation of `SDL_OnWindowShown()`.
pub(crate) fn on_window_shown(window: WindowID) {
    // Set window state if we have pending window flags cached
    let Ok(pending) = with_window(window, |w| w.core.pending_flags) else {
        return;
    };
    window::apply_window_flags(window, pending);
    let _ = with_window(window, |w| w.core.pending_flags = WindowFlags::NONE);
}

/// Translation of `SDL_OnWindowHidden()`.
pub(crate) fn on_window_hidden(window: WindowID) {
    /* Store the maximized and fullscreen flags for restoration later, in case
     * this was initiated by the window manager due to the window being unmapped
     * when minimized.
     */
    let _ = with_window(window, |w| {
        w.core.pending_flags |= w.flags() & (WindowFlags::FULLSCREEN | WindowFlags::MAXIMIZED);
    });

    // The window is already hidden at this point, so just change the mode back if necessary.
    let _ = update_fullscreen_mode(window, FullscreenOp::Leave, false);
}

/// Translation of `SDL_OnWindowDisplayChanged()`.
pub(crate) fn on_window_display_changed(window: WindowID) {
    let Ok((update, fullscreen, requested)) = with_window(window, |w| {
        (
            w.core.update_fullscreen_on_display_changed,
            w.flags().contains(WindowFlags::FULLSCREEN),
            w.requested_fullscreen_mode,
        )
    }) else {
        return;
    };

    // Don't run this if a fullscreen change was made in an event watcher callback in response to a display changed event.
    if update && fullscreen {
        let auto_mode_switch = hints::get_bool(hints::VIDEO_MATCH_EXCLUSIVE_MODE_ON_MOVE, true);

        let mut current = Default::default();
        if auto_mode_switch && (requested.w != 0 || requested.h != 0) {
            let display_id = display_for_window_position(window).unwrap_or(0);
            let include_high_density_modes = requested.pixel_density > 1.0;
            let found = display::closest_fullscreen_display_mode(
                display_id,
                requested.w,
                requested.h,
                requested.refresh_rate,
                include_high_density_modes,
            );

            // If a mode without matching dimensions was not found, just go to fullscreen desktop.
            if let Ok(found) = found {
                if requested.w == found.w && requested.h == found.h {
                    current = found;
                }
            }
        }
        let _ = with_window(window, |w| w.current_fullscreen_mode = current);

        if with_window(window, |w| w.fullscreen_visible()).unwrap_or(false) {
            let _ = update_fullscreen_mode(window, FullscreenOp::Update, true);
        }
    }

    check_window_pixel_size_changed(window);
}

/// Translation of `SDL_OnWindowMoved()`.
pub(crate) fn on_window_moved(window: WindowID) {
    check_window_display_changed(window);
}

/// Translation of `SDL_OnWindowResized()`.
pub(crate) fn on_window_resized(window: WindowID) {
    check_window_display_changed(window);
    check_window_pixel_size_changed(window);
    check_window_safe_area_changed(window);

    let shape = with_window(window, |w| {
        if w.flags().contains(WindowFlags::TRANSPARENT) {
            w.props.as_ref().and_then(|p| {
                p.get_any::<crate::video::Surface<'static>>(window::PROP_WINDOW_SHAPE_POINTER)
            })
        } else {
            None
        }
    })
    .ok()
    .flatten();
    if let (Some(surface), Ok(driver)) = (shape, driver()) {
        let _ = driver.update_window_shape(window, Some(&surface));
    }
}

/// Send the pixel size and check the display scale. Translation of
/// `SDL_CheckWindowPixelSizeChanged()`.
pub(crate) fn check_window_pixel_size_changed(window: WindowID) {
    let (pixel_w, pixel_h) = window::window_size_in_pixels(window).unwrap_or((0, 0));
    send_window_event(
        window,
        EventType::WINDOW_PIXEL_SIZE_CHANGED,
        pixel_w,
        pixel_h,
    );

    check_window_display_scale_changed(window);
}

/// Translation of `SDL_OnWindowPixelSizeChanged()`.
pub(crate) fn on_window_pixel_size_changed(window: WindowID) {
    let _ = with_window(window, |w| w.surface_valid = false);
}

/// Called by backends during a live resize. Translation of
/// `SDL_OnWindowLiveResizeUpdate()`.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn on_window_live_resize_update(window: WindowID) {
    if crate::app::has_main_callbacks() {
        let _ = crate::app::iterate_main_callbacks(false);
    } else {
        // Send an expose event so the application can redraw
        send_window_event(window, EventType::WINDOW_EXPOSED, 1, 0);
    }

    crate::events::queue::pump_event_maintenance();
}

/// Translation of `SDL_CheckWindowSafeAreaChanged()`.
pub(crate) fn check_window_safe_area_changed(window: WindowID) {
    let changed = with_window(window, |w| {
        let rect = Rect::new(
            w.safe_inset_left,
            w.safe_inset_top,
            w.core.w - (w.safe_inset_right + w.safe_inset_left),
            w.core.h - (w.safe_inset_top + w.safe_inset_bottom),
        );
        if rect != w.safe_rect {
            w.safe_rect = rect;
            true
        } else {
            false
        }
    })
    .unwrap_or(false);
    if changed {
        send_window_event(window, EventType::WINDOW_SAFE_AREA_CHANGED, 0, 0);
    }
}

/// Translation of `SDL_SetWindowSafeAreaInsets()`.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn set_window_safe_area_insets(
    window: WindowID,
    left: i32,
    right: i32,
    top: i32,
    bottom: i32,
) {
    let _ = with_window(window, |w| {
        w.safe_inset_left = left;
        w.safe_inset_right = right;
        w.safe_inset_top = top;
        w.safe_inset_bottom = bottom;
    });
    check_window_safe_area_changed(window);
}

/// Translation of `SDL_OnWindowMinimized()`.
pub(crate) fn on_window_minimized(window: WindowID) {
    if with_window(window, |w| w.flags().contains(WindowFlags::FULLSCREEN)).unwrap_or(false) {
        let _ = update_fullscreen_mode(window, FullscreenOp::Leave, false);
    }
}

/// Translation of `SDL_OnWindowMaximized()`.
pub(crate) fn on_window_maximized(_window: WindowID) {}

/// Translation of `SDL_OnWindowRestored()`.
pub(crate) fn on_window_restored(window: WindowID) {
    /*
     * FIXME: Is this fine to just remove this, or should it be preserved just
     * for the fullscreen case? In principle it seems like just hiding/showing
     * windows shouldn't affect the stacking order; maybe the right fix is to
     * re-decouple OnWindowShown and OnWindowRestored.
     */
    // SDL_RaiseWindow(window);

    if with_window(window, |w| w.flags().contains(WindowFlags::FULLSCREEN)).unwrap_or(false) {
        let _ = update_fullscreen_mode(window, FullscreenOp::Enter, false);
    }
}

/// Translation of `SDL_OnWindowEnter()`.
pub(crate) fn on_window_enter(window: WindowID) {
    if let Ok(driver) = driver() {
        let _ = driver.on_window_enter(window);
    }
    let _ = mouse::update_relative_mouse_mode();
}

/// Translation of `SDL_OnWindowLeave()`.
pub(crate) fn on_window_leave(_window: WindowID) {
    let _ = mouse::update_relative_mouse_mode();
}

/// Translation of `SDL_OnWindowFocusGained()`.
pub(crate) fn on_window_focus_gained(window: WindowID) {
    if mouse::relative_mode_enabled() {
        mouse::set_mouse_focus(Some(window));
    }

    update_window_grab(window);
}

/// Translation of `SDL_ShouldMinimizeOnFocusLoss()`.
fn should_minimize_on_focus_loss(window: WindowID) -> bool {
    let Ok((fullscreen, destroying, exclusive)) = with_window(window, |w| {
        (
            w.flags().contains(WindowFlags::FULLSCREEN),
            w.core.is_destroying,
            w.fullscreen_exclusive,
        )
    }) else {
        return false;
    };

    if !fullscreen || destroying {
        return false;
    }

    // Real fullscreen windows should minimize on focus loss so the desktop video mode is restored
    match hints::get(hints::VIDEO_MINIMIZE_ON_FOCUS_LOSS).as_deref() {
        None | Some("") => {}
        Some(h) if h.eq_ignore_ascii_case("auto") => {}
        Some(_) => return hints::get_bool(hints::VIDEO_MINIMIZE_ON_FOCUS_LOSS, false),
    }
    exclusive && !caps().contains(DeviceCaps::MODE_SWITCHING_EMULATED)
}

/// Translation of `SDL_OnWindowFocusLost()`.
pub(crate) fn on_window_focus_lost(window: WindowID) {
    update_window_grab(window);

    if should_minimize_on_focus_loss(window) {
        let _ = window::minimize_window(window);
    }
}

/// The toplevel parent of the window with keyboard focus. Translation of
/// `SDL_GetToplevelForKeyboardFocus()`.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn toplevel_for_keyboard_focus() -> Option<WindowID> {
    let mut focus = keyboard::keyboard_focus()?;
    // Get the toplevel parent window.
    while let Ok(Some(parent)) = with_window(focus, |w| w.parent) {
        focus = parent;
    }
    Some(focus)
}

/// Apply the input grabs a window asks for (only while it has focus).
/// Translation of `SDL_UpdateWindowGrab()`.
pub(crate) fn update_window_grab(window: WindowID) {
    let Ok(flags) = with_window(window, |w| w.flags()) else {
        return;
    };
    let Ok(driver) = driver() else { return };

    let (mouse_grabbed, keyboard_grabbed) = if flags.contains(WindowFlags::INPUT_FOCUS) {
        (
            mouse::relative_mode_enabled() || flags.contains(WindowFlags::MOUSE_GRABBED),
            flags.contains(WindowFlags::KEYBOARD_GRABBED),
        )
    } else {
        (false, false)
    };

    if mouse_grabbed || keyboard_grabbed {
        let previous = with_device(|v| v.grabbed_window).ok().flatten();
        if let Some(previous) = previous.filter(|&p| p != window) {
            // stealing a grab from another window!
            let _ = with_window(previous, |w| {
                w.core.flags &= !(WindowFlags::MOUSE_GRABBED | WindowFlags::KEYBOARD_GRABBED);
            });
            let _ = driver.set_window_mouse_grab(previous, false);
            let _ = driver.set_window_keyboard_grab(previous, false);
        }
        let _ = with_device(|v| v.grabbed_window = Some(window));
    } else {
        let _ = with_device(|v| {
            if v.grabbed_window == Some(window) {
                v.grabbed_window = None; // ungrabbing input.
            }
        });
    }

    if let Some(Err(_)) = driver.set_window_mouse_grab(window, mouse_grabbed) {
        let _ = with_window(window, |w| w.core.flags &= !WindowFlags::MOUSE_GRABBED);
    }
    if let Some(Err(_)) = driver.set_window_keyboard_grab(window, keyboard_grabbed) {
        let _ = with_window(window, |w| w.core.flags &= !WindowFlags::KEYBOARD_GRABBED);
    }

    let _ = with_device(|v| {
        if let Some(grabbed) = v.grabbed_window {
            let still = v
                .windows
                .iter()
                .find(|w| w.core.id == grabbed)
                .is_some_and(|w| {
                    w.flags()
                        .intersects(WindowFlags::MOUSE_GRABBED | WindowFlags::KEYBOARD_GRABBED)
                });
            if !still {
                v.grabbed_window = None;
            }
        }
    });
}

/// The window that currently has an input grab. Translation of
/// `SDL_GetGrabbedWindow()`.
pub(crate) fn grabbed_window() -> Option<WindowID> {
    with_device(|v| {
        let grabbed = v.grabbed_window?;
        v.windows
            .iter()
            .find(|w| w.core.id == grabbed)
            .filter(|w| {
                w.flags()
                    .intersects(WindowFlags::MOUSE_GRABBED | WindowFlags::KEYBOARD_GRABBED)
            })
            .map(|w| w.core.id)
    })
    .ok()
    .flatten()
}

// ---------------------------------------------------------------------------
// Screen saver
// ---------------------------------------------------------------------------

/// Whether the screen saver is currently enabled. Translation of
/// `SDL_ScreenSaverEnabled()`.
pub fn screen_saver_enabled() -> bool {
    with_device(|v| !v.suspend_screensaver).unwrap_or(true)
}

/// Allow the screen to be blanked by a screen saver. Translation of
/// `SDL_EnableScreenSaver()`.
pub fn enable_screen_saver() -> Result<()> {
    let changed = with_device(|v| {
        let changed = v.suspend_screensaver;
        v.suspend_screensaver = false;
        changed
    })?;
    if !changed {
        return Ok(());
    }
    driver()?
        .suspend_screen_saver(false)
        .unwrap_or_else(|| Err(Error::unsupported()))
}

/// Prevent the screen from being blanked by a screen saver (the default
/// while video is up). Translation of `SDL_DisableScreenSaver()`.
pub fn disable_screen_saver() -> Result<()> {
    let changed = with_device(|v| {
        let changed = !v.suspend_screensaver;
        v.suspend_screensaver = true;
        changed
    })?;
    if !changed {
        return Ok(());
    }
    driver()?
        .suspend_screen_saver(true)
        .unwrap_or_else(|| Err(Error::unsupported()))
}

/// Shut down the video subsystem, destroying all windows. Translation of
/// `SDL_QuitVideo()`.
pub(crate) fn quit_video() {
    if with_device(|v| v.is_quitting = true).is_err() {
        return;
    }

    // Halt event processing before doing anything else
    crate::events::touch::quit_touch();
    mouse::quit_mouse();
    keyboard::quit_keyboard();
    crate::init::quit_subsystem(crate::init::InitFlags::EVENTS);

    let _ = enable_screen_saver();

    // Clean up the system video
    while let Some(window) = window_ids().first().copied() {
        window::destroy_window(window);
    }

    let Ok(driver) = driver() else { return };
    if with_device(|v| v.gl_driver_loaded).unwrap_or(0) != 0 && driver.gl_unload_library().is_some()
    {
        let _ = with_device(|v| v.gl_driver_loaded = 0);
    }
    if with_device(|v| v.vulkan_loader_loaded).unwrap_or(0) != 0
        && driver.vulkan_unload_library().is_some()
    {
        let _ = with_device(|v| v.vulkan_loader_loaded = 0);
    }

    driver.video_quit();

    let ids = display::displays().unwrap_or_default();
    for id in ids.iter().rev() {
        display::del_video_display(*id, false);
    }

    crate::sdl_assert!(with_device(|v| v.displays.is_empty()).unwrap_or(true));

    super::clipboard::cancel_clipboard_data(0);
    super::clipboard::quit_clipboard();

    let device = VIDEO_DEVICE.lock().borrow_mut().take();
    crate::events::window::set_video(None);
    drop(device);

    // This needs to happen after the video subsystem has removed pen data
    crate::events::pen::quit_pen();
}

/// The no-dynamic-library error. Translation of `SDL_DllNotSupported()`.
pub(crate) fn dll_not_supported(name: &str) -> Error {
    let driver = current_video_driver().unwrap_or("");
    Error::new(format!(
        "No dynamic {name} support in current SDL video driver ({driver})"
    ))
}

/// The missing-context-support error. Translation of `SDL_ContextNotSupported()`.
pub(crate) fn context_not_supported(name: &str) -> Error {
    let driver = current_video_driver().unwrap_or("");
    Error::new(format!(
        "{name} support is either not configured in SDL or not available in current SDL video driver ({driver}) or platform"
    ))
}

// ---------------------------------------------------------------------------
// The event core's view of the video subsystem
// ---------------------------------------------------------------------------

/// The [`VideoHooks`] the event core reaches windows and displays through.
struct Hooks;

impl VideoHooks for Hooks {
    fn with_window(&self, id: WindowID, f: &mut dyn FnMut(&mut WindowCore)) -> bool {
        with_window(id, |w| f(&mut w.core)).is_ok()
    }

    fn is_quitting(&self) -> bool {
        with_device(|v| v.is_quitting).unwrap_or(false)
    }

    fn on_window_event(&self, id: WindowID, event_type: EventType) {
        match event_type {
            EventType::WINDOW_SHOWN => on_window_shown(id),
            EventType::WINDOW_HIDDEN => on_window_hidden(id),
            EventType::WINDOW_MOVED => on_window_moved(id),
            EventType::WINDOW_RESIZED => on_window_resized(id),
            EventType::WINDOW_PIXEL_SIZE_CHANGED => on_window_pixel_size_changed(id),
            EventType::WINDOW_MINIMIZED => on_window_minimized(id),
            EventType::WINDOW_MAXIMIZED => on_window_maximized(id),
            EventType::WINDOW_RESTORED => on_window_restored(id),
            EventType::WINDOW_MOUSE_ENTER => on_window_enter(id),
            EventType::WINDOW_MOUSE_LEAVE => on_window_leave(id),
            EventType::WINDOW_FOCUS_GAINED => on_window_focus_gained(id),
            EventType::WINDOW_FOCUS_LOST => on_window_focus_lost(id),
            EventType::WINDOW_DISPLAY_CHANGED => on_window_display_changed(id),
            _ => {}
        }
    }

    fn check_window_pixel_size_changed(&self, id: WindowID) {
        check_window_pixel_size_changed(id);
    }

    fn visible_toplevel_window_count(&self) -> usize {
        with_device(|v| {
            v.windows
                .iter()
                .filter(|w| w.parent.is_none() && !w.flags().contains(WindowFlags::HIDDEN))
                .count()
        })
        .unwrap_or(0)
    }

    fn has_windows(&self) -> bool {
        with_device(|v| !v.windows.is_empty()).unwrap_or(false)
    }

    fn pump_events(&self) {
        if let Ok(driver) = driver() {
            let _ = driver.pump_events();
        }
    }

    fn can_wait(&self) -> bool {
        driver().is_ok_and(|d| d.can_wait_events())
    }

    fn wait_event_timeout(&self, timeout: Option<Duration>) -> i32 {
        driver()
            .ok()
            .and_then(|d| d.wait_event_timeout(timeout))
            .unwrap_or(0)
    }

    fn set_wakeup_window(&self, window: Option<WindowID>) {
        let _ = with_device(|v| v.wakeup_window = window);
    }

    fn find_active_window(&self) -> Option<WindowID> {
        with_device(|v| {
            v.windows
                .iter()
                .find(|w| !w.core.is_destroying)
                .map(|w| w.core.id)
        })
        .ok()
        .flatten()
    }

    fn send_wakeup_event(&self) {
        // We only want to do this once while waiting for an event, so take it here
        if let Ok(Some(window)) = with_device(|v| v.wakeup_window.take()) {
            if let Ok(driver) = driver() {
                let _ = driver.send_wakeup_event(window);
            }
        }
    }

    fn toggle_drag_and_drop_support(&self) {
        window::toggle_drag_and_drop_support();
    }

    fn on_display_event(&self, id: DisplayID, event_type: EventType) {
        match event_type {
            EventType::DISPLAY_ADDED => display::on_display_added(id),
            EventType::DISPLAY_MOVED => display::on_display_moved(id),
            _ => {}
        }
    }

    fn with_display(&self, id: DisplayID, f: &mut dyn FnMut(&mut DisplayCore)) -> bool {
        with_display(id, |d| {
            let mut core = DisplayCore {
                id,
                current_orientation: d.current_orientation as i32,
            };
            f(&mut core);
            d.current_orientation =
                super::sysvideo::DisplayOrientation::from_i32(core.current_orientation);
        })
        .is_some()
    }

    fn clipboard_lost(&self, mime_types: &[String]) {
        super::clipboard::clipboard_lost(mime_types);
    }

    fn supports_mouse_feature(&self, feature: MouseFeature) -> bool {
        driver().is_ok_and(|d| d.implements_mouse_feature(feature))
    }

    fn minimize_window(&self, id: WindowID) {
        let _ = window::minimize_window(id);
    }

    fn update_window_grab(&self, id: WindowID) {
        update_window_grab(id);
    }

    fn warp_mouse(&self, id: WindowID, x: f32, y: f32) -> Result<()> {
        driver()?
            .warp_mouse(id, x, y)
            .unwrap_or_else(|| Err(Error::unsupported()))
    }

    fn warp_mouse_global(&self, x: f32, y: f32) -> Result<()> {
        driver()?
            .warp_mouse_global(x, y)
            .unwrap_or_else(|| Err(Error::unsupported()))
    }

    fn set_relative_mouse_mode(&self, enabled: bool) -> Result<()> {
        driver()?
            .set_relative_mouse_mode(enabled)
            .unwrap_or_else(|| Err(Error::unsupported()))
    }

    fn capture_mouse(&self, window: Option<WindowID>) -> Result<()> {
        driver()?
            .capture_mouse(window)
            .unwrap_or_else(|| Err(Error::unsupported()))
    }

    fn global_mouse_state(&self) -> Option<(f32, f32, MouseButtonFlags)> {
        driver().ok()?.global_mouse_state()
    }

    fn create_cursor(
        &self,
        surface: &crate::video::Surface<'_>,
        hot_x: i32,
        hot_y: i32,
    ) -> Option<Result<mouse::Cursor>> {
        driver().ok()?.create_cursor(surface, hot_x, hot_y)
    }

    fn create_animated_cursor(
        &self,
        frames: &[mouse::CursorFrame<'_>],
        hot_x: i32,
        hot_y: i32,
    ) -> Option<Result<mouse::Cursor>> {
        driver().ok()?.create_animated_cursor(frames, hot_x, hot_y)
    }

    fn create_system_cursor(&self, id: mouse::SystemCursor) -> Result<mouse::Cursor> {
        driver()?
            .create_system_cursor(id)
            .unwrap_or_else(|| Err(Error::new("CreateSystemCursor is not currently supported")))
    }

    fn show_cursor(&self, cursor: Option<&mouse::Cursor>) {
        if let Ok(driver) = driver() {
            let _ = driver.show_cursor(cursor);
        }
    }

    fn move_cursor(&self, cursor: &mouse::Cursor) {
        if let Ok(driver) = driver() {
            let _ = driver.move_cursor(cursor);
        }
    }

    fn message_box_count(&self) -> i32 {
        super::messagebox::message_box_count()
    }

    fn reset_touch(&self) -> bool {
        driver().is_ok_and(|d| d.reset_touch().is_some())
    }

    fn start_text_input(&self, id: WindowID) {
        super::textinput::backend_start_text_input(id);
    }

    fn stop_text_input(&self, id: WindowID) {
        super::textinput::backend_stop_text_input(id);
    }
}

/// The current mode of `display` as `CurrentMode` (for comparisons).
#[allow(dead_code)]
pub(crate) fn display_current_mode_ref(display: DisplayID) -> Option<CurrentMode> {
    with_display(display, |d| d.current_mode)
}
