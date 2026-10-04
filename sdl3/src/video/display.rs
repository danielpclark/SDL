// Rust translation of the display parts of src/video/SDL_video.c from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Displays and display modes.
//!
//! Displays are referred to by [`DisplayID`]; the first display is the
//! primary one. Positions in the desktop coordinate space are shared by all
//! displays (see [`display_bounds`]).

use crate::error::{Error, Result};
use crate::events::window::send_display_event;
use crate::events::{DisplayID, EventType, WindowID};
use crate::hints;
use crate::properties::Properties;
use crate::stdlib::math::{fabsf, roundf};
use crate::video::{Point, Rect};

use super::core::{self, uninitialized_video, with_device, with_window};
use super::sysvideo::{
    CurrentMode, DeviceCaps, DisplayMode, DisplayOrientation, HdrOutputProperties, VideoDisplay,
};

/// Translation of `SDL_PROP_DISPLAY_HDR_ENABLED_BOOLEAN`.
pub const PROP_DISPLAY_HDR_ENABLED_BOOLEAN: &str = "SDL.display.HDR_enabled";
/// Translation of `SDL_PROP_DISPLAY_KMSDRM_PANEL_ORIENTATION_NUMBER`.
pub const PROP_DISPLAY_KMSDRM_PANEL_ORIENTATION_NUMBER: &str =
    "SDL.display.KMSDRM.panel_orientation";
/// Translation of `SDL_PROP_DISPLAY_WINDOWS_HMONITOR_POINTER` (the
/// `HMONITOR`, as a number).
pub const PROP_DISPLAY_WINDOWS_HMONITOR_POINTER: &str = "SDL.display.windows.hmonitor";

/// Run `f` on a display, failing like `SDL_GetVideoDisplay()` ("Invalid
/// display", or the uninitialized error).
pub(crate) fn with_video_display<R>(
    display_id: DisplayID,
    f: impl FnOnce(&mut VideoDisplay) -> R,
) -> Result<R> {
    with_device(|v| {
        v.displays
            .iter_mut()
            .find(|d| d.id == display_id)
            .map(f)
            .ok_or_else(|| Error::new("Invalid display"))
    })?
}

/// Translation of `cmpmodes()`: larger, deeper, faster modes first.
pub(crate) fn cmpmodes(a: &DisplayMode, b: &DisplayMode) -> i32 {
    let a_refresh_rate = (a.refresh_rate * 100.0) as i32;
    let b_refresh_rate = (b.refresh_rate * 100.0) as i32;
    let a_pixel_density = (a.pixel_density * 100.0) as i32;
    let b_pixel_density = (b.pixel_density * 100.0) as i32;

    if a.w != b.w {
        b.w - a.w
    } else if a.h != b.h {
        b.h - a.h
    } else if a.format.bits_per_pixel() != b.format.bits_per_pixel() {
        b.format.bits_per_pixel() as i32 - a.format.bits_per_pixel() as i32
    } else if a.format.pixel_layout() as i32 != b.format.pixel_layout() as i32 {
        b.format.pixel_layout() as i32 - a.format.pixel_layout() as i32
    } else if a_refresh_rate != b_refresh_rate {
        b_refresh_rate - a_refresh_rate
    } else if a_pixel_density != b_pixel_density {
        a_pixel_density - b_pixel_density
    } else {
        0
    }
}

/// The union of all display bounds. Translation of `SDL_UpdateDesktopBounds()`.
pub(crate) fn update_desktop_bounds() {
    let mut rect = Rect::default();

    if let Ok(ids) = displays() {
        for (i, id) in ids.iter().enumerate() {
            if let Ok(bounds) = display_bounds(*id) {
                if i == 0 {
                    rect = bounds;
                } else {
                    rect = rect.union(&bounds).unwrap_or(rect);
                }
            }
        }
    }
    let _ = with_device(|v| v.desktop_bounds = rect);
}

/// Make sure all the fields of a mode are set up correctly. Translation of
/// `SDL_FinalizeDisplayMode()`.
pub(crate) fn finalize_display_mode(mode: &mut DisplayMode) {
    // Make sure all the fields are set up correctly
    if mode.pixel_density <= 0.0 {
        mode.pixel_density = 1.0;
    }

    if mode.refresh_rate_numerator > 0 {
        if mode.refresh_rate_denominator <= 0 {
            mode.refresh_rate_denominator = 1;
        }
        mode.refresh_rate =
            mode.refresh_rate_numerator as f32 / mode.refresh_rate_denominator as f32;
    } else {
        let (num, den) = crate::utils::approximate_fraction(mode.refresh_rate);
        mode.refresh_rate_numerator = num;
        mode.refresh_rate_denominator = den;
    }
    mode.refresh_rate = roundf(mode.refresh_rate * 100.0) / 100.0;
}

/// Add a display with just a desktop mode. Translation of
/// `SDL_AddBasicVideoDisplay()`.
pub(crate) fn add_basic_video_display(desktop_mode: Option<&DisplayMode>) -> DisplayID {
    let mut display = VideoDisplay::new();
    if let Some(mode) = desktop_mode {
        display.desktop_mode = *mode;
    }
    add_video_display(display, false)
}

/// Add a display the backend filled in, returning its new ID (0 if the
/// video subsystem isn't up). Translation of `SDL_AddVideoDisplay()`.
pub(crate) fn add_video_display(mut display: VideoDisplay, send_event: bool) -> DisplayID {
    let id = crate::utils::next_object_id();
    display.id = id;
    if display.name.is_none() {
        display.name = Some(id.to_string());
    }
    if display.content_scale == 0.0 {
        display.content_scale = 1.0;
    }

    display.desktop_mode.display_id = id;
    display.current_mode = CurrentMode::Desktop;
    finalize_display_mode(&mut display.desktop_mode);

    for mode in &mut display.fullscreen_modes {
        mode.display_id = id;
    }

    display.hdr.hdr_headroom = display.hdr.hdr_headroom.max(1.0);
    display.hdr.sdr_white_level = display.hdr.sdr_white_level.max(1.0);
    let hdr_enabled = display.hdr.hdr_headroom > 1.0;

    if with_device(|v| v.displays.push(display)).is_err() {
        return 0;
    }

    if let Ok(props) = display_properties(id) {
        let _ = props.set(PROP_DISPLAY_HDR_ENABLED_BOOLEAN, hdr_enabled);
    }

    update_desktop_bounds();

    if send_event {
        send_display_event(id, EventType::DISPLAY_ADDED, 0, 0);
    }

    id
}

/// See if any windows have changed to the new display. Translation of
/// `SDL_OnDisplayAdded()`.
pub(crate) fn on_display_added(_display: DisplayID) {
    for window in core::window_ids() {
        core::check_window_display_changed(window);
    }
}

/// Translation of `SDL_OnDisplayMoved()`.
pub(crate) fn on_display_moved(_display: DisplayID) {
    update_desktop_bounds();
}

/// Remove a display. Translation of `SDL_DelVideoDisplay()`.
pub(crate) fn del_video_display(display_id: DisplayID, send_event: bool) {
    if display_index(display_id).is_err() {
        return;
    }

    if send_event {
        send_display_event(display_id, EventType::DISPLAY_REMOVED, 0, 0);
    }

    let _ = with_device(|v| v.displays.retain(|d| d.id != display_id));

    update_desktop_bounds();
}

/// The currently connected displays, the primary display first.
/// Translation of `SDL_GetDisplays()`.
pub fn displays() -> Result<Vec<DisplayID>> {
    with_device(|v| v.displays.iter().map(|d| d.id).collect())
}

/// The primary display. Translation of `SDL_GetPrimaryDisplay()`.
pub fn primary_display() -> Result<DisplayID> {
    with_device(|v| v.displays.first().map(|d| d.id))
        .ok()
        .flatten()
        .ok_or_else(uninitialized_video)
}

/// The index of a display in the display list. Translation of
/// `SDL_GetDisplayIndex()`.
pub(crate) fn display_index(display_id: DisplayID) -> Result<usize> {
    with_device(|v| v.displays.iter().position(|d| d.id == display_id))?
        .ok_or_else(|| Error::new("Invalid display"))
}

/// The properties of a display. Translation of `SDL_GetDisplayProperties()`.
pub fn display_properties(display_id: DisplayID) -> Result<Properties> {
    with_video_display(display_id, |d| {
        d.props.get_or_insert_with(Properties::new).clone()
    })
}

/// The name of a display. Translation of `SDL_GetDisplayName()`.
pub fn display_name(display_id: DisplayID) -> Result<String> {
    with_video_display(display_id, |d| d.name.clone().unwrap_or_default())
}

/// The desktop area represented by a display; the primary display is at
/// (0, 0). Translation of `SDL_GetDisplayBounds()`.
pub fn display_bounds(display_id: DisplayID) -> Result<Rect> {
    let current_mode = with_video_display(display_id, |d| d.current_mode())?;

    let driver = core::driver()?;
    if let Some(Ok(rect)) = driver.display_bounds(display_id) {
        return Ok(rect);
    }

    // Assume that the displays are left to right
    let mut rect = Rect::default();
    if Ok(display_id) == primary_display() {
        rect.x = 0;
        rect.y = 0;
    } else {
        let index = display_index(display_id)?;
        let previous = with_device(|v| v.displays[index - 1].id)?;
        if let Ok(previous_rect) = display_bounds(previous) {
            rect = previous_rect;
        }
        rect.x += rect.w;
    }
    rect.w = current_mode.w;
    rect.h = current_mode.h;
    Ok(rect)
}

/// Translation of `ParseDisplayUsableBoundsHint()` (`SDL_sscanf(hint,
/// "%d,%d,%d,%d", ...)`).
fn parse_display_usable_bounds_hint() -> Option<Rect> {
    let hint = hints::get(hints::DISPLAY_USABLE_BOUNDS)?;
    let mut text = hint.as_bytes();
    let mut values = [0i32; 4];
    for (i, value) in values.iter_mut().enumerate() {
        if i > 0 {
            text = text.strip_prefix(b",")?;
        }
        let skipped = text.iter().take_while(|c| c.is_ascii_whitespace()).count();
        let (v, used) = crate::stdlib::string::strtol(&text[skipped..], 10);
        if used == 0 {
            return None;
        }
        *value = v as i32;
        text = &text[skipped + used..];
    }
    Some(Rect::new(values[0], values[1], values[2], values[3]))
}

/// The usable desktop area of a display (without the menu bar, taskbar and
/// such), which may be overridden with the
/// [`hints::DISPLAY_USABLE_BOUNDS`] hint for the primary display.
/// Translation of `SDL_GetDisplayUsableBounds()`.
pub fn display_usable_bounds(display_id: DisplayID) -> Result<Rect> {
    with_video_display(display_id, |_| ())?;

    if Ok(display_id) == primary_display() {
        if let Some(rect) = parse_display_usable_bounds_hint() {
            return Ok(rect);
        }
    }

    let driver = core::driver()?;
    if let Some(Ok(rect)) = driver.display_usable_bounds(display_id) {
        return Ok(rect);
    }

    // Oh well, just give the entire display bounds.
    display_bounds(display_id)
}

/// The orientation of a display when it is unrotated (landscape if the
/// driver doesn't know). Translation of `SDL_GetNaturalDisplayOrientation()`.
pub fn natural_display_orientation(display_id: DisplayID) -> DisplayOrientation {
    match with_video_display(display_id, |d| d.natural_orientation) {
        Err(_) => DisplayOrientation::Unknown,
        // Default to landscape if the driver hasn't set it
        Ok(DisplayOrientation::Unknown) => DisplayOrientation::Landscape,
        Ok(orientation) => orientation,
    }
}

/// The orientation of a display (landscape if the driver doesn't know).
/// Translation of `SDL_GetCurrentDisplayOrientation()`.
pub fn current_display_orientation(display_id: DisplayID) -> DisplayOrientation {
    match with_video_display(display_id, |d| d.current_orientation) {
        Err(_) => DisplayOrientation::Unknown,
        // Default to landscape if the driver hasn't set it
        Ok(DisplayOrientation::Unknown) => DisplayOrientation::Landscape,
        Ok(orientation) => orientation,
    }
}

/// Translation of `SDL_SetDisplayContentScale()`.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn set_display_content_scale(display_id: DisplayID, scale: f32) {
    let changed = with_video_display(display_id, |d| {
        if scale != d.content_scale {
            d.content_scale = scale;
            true
        } else {
            false
        }
    })
    .unwrap_or(false);
    if changed {
        send_display_event(display_id, EventType::DISPLAY_CONTENT_SCALE_CHANGED, 0, 0);

        // Check the windows on this display
        for window in core::window_ids() {
            if with_window(window, |w| w.core.display_id) == Ok(display_id) {
                core::check_window_display_scale_changed(window);
            }
        }
    }
}

/// The content scale of a display: 2.0 for a 200% scaled user interface,
/// say. Translation of `SDL_GetDisplayContentScale()`.
pub fn display_content_scale(display_id: DisplayID) -> Result<f32> {
    with_video_display(display_id, |d| d.content_scale)
}

/// Translation of `SDL_SetWindowHDRProperties()`.
pub(crate) fn set_window_hdr_properties(
    window: WindowID,
    hdr: HdrOutputProperties,
    send_event: bool,
) {
    let hdr_clamped = HdrOutputProperties {
        sdr_white_level: hdr.sdr_white_level.max(1.0),
        hdr_headroom: hdr.hdr_headroom.max(1.0),
    };

    let props = with_window(window, |w| {
        if w.hdr.hdr_headroom != hdr_clamped.hdr_headroom
            || w.hdr.sdr_white_level != hdr_clamped.sdr_white_level
        {
            w.hdr = hdr_clamped;
            Some(w.properties())
        } else {
            None
        }
    });
    if let Ok(Some(window_props)) = props {
        let _ = window_props.set(
            super::window::PROP_WINDOW_HDR_HEADROOM_FLOAT,
            hdr_clamped.hdr_headroom,
        );
        let _ = window_props.set(
            super::window::PROP_WINDOW_SDR_WHITE_LEVEL_FLOAT,
            hdr_clamped.sdr_white_level,
        );
        let _ = window_props.set(
            super::window::PROP_WINDOW_HDR_ENABLED_BOOLEAN,
            hdr_clamped.hdr_headroom > 1.0,
        );

        if send_event {
            crate::events::window::send_window_event(
                window,
                EventType::WINDOW_HDR_STATE_CHANGED,
                (hdr_clamped.hdr_headroom > 1.0) as i32,
                0,
            );
        }
    }
}

/// Translation of `SDL_SetDisplayHDRProperties()`.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn set_display_hdr_properties(display_id: DisplayID, hdr: HdrOutputProperties) {
    let hdr_clamped = HdrOutputProperties {
        sdr_white_level: hdr.sdr_white_level.max(1.0),
        hdr_headroom: hdr.hdr_headroom.max(1.0),
    };
    let Ok(changed) = with_video_display(display_id, |d| {
        let changed = hdr_clamped != d.hdr;
        d.hdr = hdr_clamped;
        changed
    }) else {
        return;
    };

    if changed && !core::caps().contains(DeviceCaps::SENDS_HDR_CHANGES) {
        for w in core::window_ids() {
            if super::window::display_for_window(w) == Ok(display_id) {
                set_window_hdr_properties(w, hdr_clamped, true);
            }
        }
    }
}

/// Have the backend list a display's modes, if it hasn't yet.
/// Translation of `SDL_UpdateFullscreenDisplayModes()`.
fn update_fullscreen_display_modes(display_id: DisplayID) {
    if with_video_display(display_id, |d| d.fullscreen_modes.is_empty()).unwrap_or(false) {
        if let Ok(driver) = core::driver() {
            let _ = driver.display_modes(display_id);
        }
    }
}

/// A mode in a display's fullscreen mode list (upstream's pointer into it).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct ModeMatch {
    pub(crate) display: DisplayID,
    pub(crate) index: usize,
    pub(crate) mode: DisplayMode,
}

impl ModeMatch {
    /// The mode as `display`'s current mode.
    pub(crate) fn for_display(&self, display: DisplayID) -> CurrentMode {
        if self.display == display {
            CurrentMode::Fullscreen(self.index)
        } else {
            CurrentMode::Other(self.mode)
        }
    }
}

/// The fullscreen mode matching `mode`, or `None` for the desktop mode.
/// Translation of `SDL_GetFullscreenModeMatch()`.
pub(crate) fn fullscreen_mode_match(mode: &DisplayMode) -> Option<ModeMatch> {
    if mode.w <= 0 || mode.h <= 0 {
        // Use the desktop mode
        return None;
    }

    let mut fullscreen_mode = *mode;
    if fullscreen_mode.display_id == 0 {
        fullscreen_mode.display_id = primary_display().unwrap_or(0);
    }
    finalize_display_mode(&mut fullscreen_mode);

    let display = fullscreen_mode.display_id;
    if with_video_display(display, |_| ()).is_err() {
        return None;
    }
    update_fullscreen_display_modes(display);

    with_video_display(display, |d| {
        // Search for an exact match
        let index = d
            .fullscreen_modes
            .iter()
            .position(|m| *m == fullscreen_mode)
            // Search for a mode with the same characteristics
            .or_else(|| {
                d.fullscreen_modes
                    .iter()
                    .position(|m| cmpmodes(&fullscreen_mode, m) == 0)
            })?;
        Some(ModeMatch {
            display,
            index,
            mode: d.fullscreen_modes[index],
        })
    })
    .ok()
    .flatten()
}

/// Add a fullscreen mode to a display, unless it has an equivalent one;
/// returns whether it was added. Translation of `SDL_AddFullscreenDisplayMode()`.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn add_fullscreen_display_mode(display_id: DisplayID, mode: &DisplayMode) -> bool {
    with_video_display(display_id, |d| add_fullscreen_display_mode_to(d, mode)).unwrap_or(false)
}

/// [`add_fullscreen_display_mode`] on a display record a backend is still
/// filling in, before [`add_video_display`] (upstream passes such records to
/// `SDL_AddFullscreenDisplayMode()` too).
pub(crate) fn add_fullscreen_display_mode_to(d: &mut VideoDisplay, mode: &DisplayMode) -> bool {
    // Finalize the mode for the display
    let mut new_mode = *mode;
    new_mode.display_id = d.id;
    finalize_display_mode(&mut new_mode);

    // Make sure we don't already have the mode in the list
    if d.fullscreen_modes
        .iter()
        .any(|m| cmpmodes(&new_mode, m) == 0)
    {
        return false;
    }

    // Go ahead and add the new mode
    d.fullscreen_modes.push(new_mode);

    // Re-sort video modes
    // (upstream's current mode is a pointer into the list, so after the
    // sort it may point at a different mode; fixed here by following the
    // current mode to its new place)
    let current = match d.current_mode {
        CurrentMode::Fullscreen(i) => d.fullscreen_modes.get(i).copied(),
        _ => None,
    };
    d.fullscreen_modes.sort_by(|a, b| cmpmodes(a, b).cmp(&0));
    if let Some(current) = current {
        if let Some(i) = d.fullscreen_modes.iter().position(|m| *m == current) {
            d.current_mode = CurrentMode::Fullscreen(i);
        }
    }
    true
}

/// Translation of `SDL_ResetFullscreenDisplayModes()`.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn reset_fullscreen_display_modes(display_id: DisplayID) {
    let _ = with_video_display(display_id, |d| {
        d.fullscreen_modes.clear();
        d.current_mode = CurrentMode::Desktop;
    });
}

/// The fullscreen-only modes of a display, from largest to smallest.
/// Translation of `SDL_GetFullscreenDisplayModes()`.
pub fn fullscreen_display_modes(display_id: DisplayID) -> Result<Vec<DisplayMode>> {
    with_video_display(display_id, |_| ())?;
    update_fullscreen_display_modes(display_id);
    with_video_display(display_id, |d| d.fullscreen_modes.clone())
}

/// The fullscreen mode closest to `w`x`h` at `refresh_rate` (0.0 for the
/// desktop's rate), at least as large as the request. Translation of
/// `SDL_GetClosestFullscreenDisplayMode()`.
pub fn closest_fullscreen_display_mode(
    display_id: DisplayID,
    w: i32,
    h: i32,
    mut refresh_rate: f32,
    include_high_density_modes: bool,
) -> Result<DisplayMode> {
    let desktop_refresh_rate = with_video_display(display_id, |d| d.desktop_mode.refresh_rate)?;

    let aspect_ratio = if h > 0 { w as f32 / h as f32 } else { 1.0 };

    if refresh_rate == 0.0 {
        refresh_rate = desktop_refresh_rate;
    }

    update_fullscreen_display_modes(display_id);

    let modes = with_video_display(display_id, |d| d.fullscreen_modes.clone())?;
    let mut closest: Option<&DisplayMode> = None;
    for mode in &modes {
        if w > mode.w {
            // Out of sorted modes large enough here
            break;
        }
        if h > mode.h {
            /* Wider, but not tall enough, due to a different aspect ratio.
             * This mode must be skipped, but closer modes may still follow */
            continue;
        }
        if mode.pixel_density > 1.0 && !include_high_density_modes {
            continue;
        }
        if let Some(c) = closest {
            let current_aspect_ratio = mode.w as f32 / mode.h as f32;
            let closest_aspect_ratio = c.w as f32 / c.h as f32;
            if fabsf(aspect_ratio - closest_aspect_ratio)
                < fabsf(aspect_ratio - current_aspect_ratio)
            {
                // The mode we already found has a better aspect ratio match
                continue;
            }

            if mode.w == c.w && mode.h == c.h {
                if fabsf(c.refresh_rate - refresh_rate) < fabsf(mode.refresh_rate - refresh_rate) {
                    /* We already found a mode and the new mode is further from our
                     * refresh rate target */
                    continue;
                }
                if c.format.bytes_per_pixel() > mode.format.bytes_per_pixel() {
                    // Prefer the highest color depth
                    continue;
                }
            }
        }

        closest = Some(mode);
    }
    closest
        .copied()
        .ok_or_else(|| Error::new("Couldn't find any matching video modes"))
}

/// Translation of `DisplayModeChanged()`.
fn display_mode_changed(old_mode: &DisplayMode, new_mode: &DisplayMode) -> bool {
    (old_mode.display_id != 0 && old_mode.display_id != new_mode.display_id)
        || (old_mode.format.0 != 0 && old_mode.format != new_mode.format)
        || (old_mode.w != 0
            && old_mode.h != 0
            && (old_mode.w != new_mode.w || old_mode.h != new_mode.h))
        || (old_mode.pixel_density != 0.0 && old_mode.pixel_density != new_mode.pixel_density)
        || (old_mode.refresh_rate != 0.0 && old_mode.refresh_rate != new_mode.refresh_rate)
}

/// Translation of `SDL_SetDesktopDisplayMode()`.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn set_desktop_display_mode(display_id: DisplayID, mode: &DisplayMode) {
    let Ok(Some((changed, current_is_desktop))) = with_video_display(display_id, |d| {
        if d.fullscreen_active {
            // This is a temporary mode change, don't save the desktop mode
            return None;
        }

        let last_mode = d.desktop_mode;

        d.desktop_mode = *mode;
        d.desktop_mode.display_id = d.id;
        finalize_display_mode(&mut d.desktop_mode);

        Some((
            display_mode_changed(&last_mode, &d.desktop_mode),
            d.current_mode == CurrentMode::Desktop,
        ))
    }) else {
        return;
    };

    if changed {
        send_display_event(
            display_id,
            EventType::DISPLAY_DESKTOP_MODE_CHANGED,
            mode.w,
            mode.h,
        );
        if current_is_desktop {
            send_display_event(
                display_id,
                EventType::DISPLAY_CURRENT_MODE_CHANGED,
                mode.w,
                mode.h,
            );
        }
    }
}

/// The desktop mode of a display (its mode when no fullscreen window has
/// changed it). Translation of `SDL_GetDesktopDisplayMode()`.
pub fn desktop_display_mode(display_id: DisplayID) -> Result<DisplayMode> {
    with_video_display(display_id, |d| d.desktop_mode)
}

/// Translation of `SDL_SetCurrentDisplayMode()`.
pub(crate) fn set_current_display_mode(display_id: DisplayID, mode: CurrentMode) {
    let Ok((changed, new_mode)) = with_video_display(display_id, |d| {
        let last_mode = d.current_mode();
        d.current_mode = mode;
        let new_mode = d.current_mode();
        (display_mode_changed(&last_mode, &new_mode), new_mode)
    }) else {
        return;
    };

    if changed {
        send_display_event(
            display_id,
            EventType::DISPLAY_CURRENT_MODE_CHANGED,
            new_mode.w,
            new_mode.h,
        );
    }
}

/// The current mode of a display (a fullscreen mode while a fullscreen
/// window changed it). Translation of `SDL_GetCurrentDisplayMode()`.
pub fn current_display_mode(display_id: DisplayID) -> Result<DisplayMode> {
    with_video_display(display_id, |d| d.current_mode())
}

/// Switch a display to a mode (`None`: its desktop mode). Translation of
/// `SDL_SetDisplayModeForDisplay()`.
pub(crate) fn set_display_mode_for_display(
    display_id: DisplayID,
    mode: Option<CurrentMode>,
) -> Result<()> {
    /* Mode switching is being emulated per-window; nothing to do and cannot fail,
     * except for XWayland, which still needs the actual mode setting call since
     * it's emulated via the XRandR interface.
     */
    if core::caps().contains(DeviceCaps::MODE_SWITCHING_EMULATED)
        && core::current_video_driver().ok() != Some("x11")
    {
        return Ok(());
    }

    let mode = mode.unwrap_or(CurrentMode::Desktop);

    let (same, value) = with_video_display(display_id, |d| {
        let value = match mode {
            CurrentMode::Desktop => d.desktop_mode,
            CurrentMode::Fullscreen(i) => d.fullscreen_modes.get(i).copied().unwrap_or_default(),
            CurrentMode::Other(m) => m,
        };
        (mode == d.current_mode, value)
    })?;
    if same {
        return Ok(());
    }

    // Actually change the display mode
    let driver = core::driver()?;
    let _ = with_device(|v| v.setting_display_mode = true);
    let result = driver.set_display_mode(display_id, &value);
    let _ = with_device(|v| v.setting_display_mode = false);
    if let Some(result) = result {
        result?;
    }

    set_current_display_mode(display_id, mode);

    Ok(())
}

/// If `point` is outside of `rect`, snaps it to the closest point inside.
/// Translation of `SDL_GetClosestPointOnRect()`.
fn closest_point_on_rect(rect: &Rect, point: &mut Point) {
    let right = rect.x + rect.w - 1;
    let bottom = rect.y + rect.h - 1;

    if point.x < rect.x {
        point.x = rect.x;
    } else if point.x > right {
        point.x = right;
    }

    if point.y < rect.y {
        point.y = rect.y;
    } else if point.y > bottom {
        point.y = bottom;
    }
}

/// The display containing the center of a rectangle, or the closest one
/// (0 if there are none). Translation of `GetDisplayForRect()`.
pub(crate) fn display_for_rect_xywh(x: i32, y: i32, w: i32, h: i32) -> DisplayID {
    let mut closest: DisplayID = 0;
    let mut closest_dist = 0x7FFFFFFF;
    let center = Point {
        x: x.wrapping_add(w / 2),
        y: y.wrapping_add(h / 2),
    };

    for id in displays().unwrap_or_default() {
        let display_rect = display_bounds(id).unwrap_or_default();

        // Check if the window is fully enclosed
        if Rect::enclosing_points(&[center], Some(&display_rect)).is_some() {
            return id;
        }

        // Snap window center to the display rect
        let mut closest_point_on_display = center;
        closest_point_on_rect(&display_rect, &mut closest_point_on_display);

        let dx = center.x.wrapping_sub(closest_point_on_display.x);
        let dy = center.y.wrapping_sub(closest_point_on_display.y);
        let dist = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
        if dist < closest_dist {
            closest = id;
            closest_dist = dist;
        }
    }

    closest
}

/// The display containing a point (or the closest one). Translation of
/// `SDL_GetDisplayForPoint()`.
pub fn display_for_point(point: Point) -> Result<DisplayID> {
    match display_for_rect_xywh(point.x, point.y, 1, 1) {
        0 => Err(Error::new("Couldn't find any displays")),
        id => Ok(id),
    }
}

/// The display containing the center of a rectangle (or the closest one).
/// Translation of `SDL_GetDisplayForRect()`.
pub fn display_for_rect(rect: &Rect) -> Result<DisplayID> {
    match display_for_rect_xywh(rect.x, rect.y, rect.w, rect.h) {
        0 => Err(Error::new("Couldn't find any displays")),
        id => Ok(id),
    }
}

/// The display whose origin is exactly at (`x`, `y`), or 0. Translation of
/// `GetDisplayAtOrigin()`.
pub(crate) fn display_at_origin(x: i32, y: i32) -> DisplayID {
    for id in displays().unwrap_or_default() {
        if let Ok(rect) = display_bounds(id) {
            if x == rect.x && y == rect.y {
                return id;
            }
        }
    }
    0
}
