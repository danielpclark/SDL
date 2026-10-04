// Rust translation of src/video/x11/SDL_x11modes.c and SDL_x11modes.h from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Displays and display modes: XRandR (1.3 or newer) when available, else
//! one generic display for the default screen. (Upstream SDL 3 no longer
//! uses Xinerama or XVidMode.)

use std::ffi::{c_int, c_uchar, c_ulong, CStr};

use super::edid::{c_field, decode_edid, MonitorInfo};
use super::sys::*;
use super::video::{intern_atom, x11_use_direct_color_visuals, X11Video};
use super::window::PENDING_FOCUS_TIME;
use super::x11dyn::X11Syms;
use crate::error::{Error, Result};
use crate::events::window::send_display_event;
use crate::events::{DisplayID, EventType};
use crate::hints;
use crate::stdlib::math::sqrtf;
use crate::video::core::{with_device, with_display};
use crate::video::display::{
    add_fullscreen_display_mode, add_video_display, del_video_display, displays,
    finalize_display_mode, set_desktop_display_mode, set_display_content_scale,
    set_display_hdr_properties,
};
use crate::video::sysvideo::{DisplayMode, VideoDisplay};
use crate::video::{PixelFormat, PixelMasks, Rect};

// #define X11MODES_DEBUG

/// Timeout and revert mode switches if the timespan has elapsed without the window becoming fullscreen.
/// 5 seconds seems good from testing.
const MODE_SWITCH_TIMEOUT_NS: u64 = 1_000_000_000 * 5;

/// The SDR white level of scRGB content (`SCRGB_NITS`).
const SCRGB_NITS: f32 = 80.0;

/* I'm becoming more and more convinced that the application should never
 * use XRandR, and it's the window manager's responsibility to track and
 * manage display modes for fullscreen windows.  Right now XRandR is completely
 * broken with respect to window manager behavior on every window manager that
 * I can find.  For example, on Unity 3D if you show a fullscreen window while
 * the resolution is changing (within ~250 ms) your window will retain the
 * fullscreen state hint but be decorated and windowed.
 *
 * However, many people swear by it, so let them swear at it. :)
 */
// #define XRANDR_DISABLED_BY_DEFAULT

/// The driver data of a display. Translation of `struct SDL_DisplayData`
/// (plus the `SDL_DisplayModeData` of its modes, kept here because the
/// core's modes don't carry backend data).
#[derive(Clone, Debug)]
pub(crate) struct X11DisplayData {
    pub(crate) screen: c_int,
    pub(crate) visual: *mut Visual,
    pub(crate) depth: c_int,
    pub(crate) scanline_pad: c_int,
    pub(crate) x: c_int,
    pub(crate) y: c_int,
    pub(crate) mode_switch_deadline_ns: u64,
    pub(crate) use_xrandr: bool,
    pub(crate) xrandr_output: RROutput,
    pub(crate) connector_name: String,
    /// `mode->internal->xrandr_mode` of the modes handed to the video core
    /// (the desktop mode and the fullscreen modes), by the mode.
    pub(crate) modedata: Vec<(DisplayMode, RRMode)>,
}

// SAFETY: the visual is an Xlib object of the display (read-only data
// owned by Xlib for the life of the connection).
unsafe impl Send for X11DisplayData {}

impl X11DisplayData {
    fn new() -> X11DisplayData {
        X11DisplayData {
            screen: 0,
            visual: std::ptr::null_mut(),
            depth: 0,
            scanline_pad: 0,
            x: 0,
            y: 0,
            mode_switch_deadline_ns: 0,
            use_xrandr: false,
            xrandr_output: 0,
            connector_name: String::new(),
            modedata: Vec::new(),
        }
    }

    /// Remember the `SDL_DisplayModeData` of a mode.
    fn set_mode_data(&mut self, mode: &DisplayMode, xrandr_mode: RRMode) {
        let key = mode_key(mode);
        self.modedata.retain(|(m, _)| mode_key(m) != key);
        self.modedata.push((*mode, xrandr_mode));
    }

    /// The `SDL_DisplayModeData` of a mode the video core handed back.
    fn mode_data(&self, mode: &DisplayMode) -> Option<RRMode> {
        let key = mode_key(mode);
        self.modedata
            .iter()
            .find(|(m, _)| mode_key(m) == key)
            .map(|(_, id)| *id)
    }
}

/// The parts of a mode that identify it once finalized (the core adds the
/// display ID and fills in the refresh rate).
fn mode_key(mode: &DisplayMode) -> (i32, i32, i32, i32, u32) {
    let mut m = *mode;
    finalize_display_mode(&mut m);
    (
        m.w,
        m.h,
        m.refresh_rate_numerator,
        m.refresh_rate_denominator,
        m.format.0,
    )
}

/// Run `f` on a display and its X11 data (`display->internal`).
pub(crate) fn with_x11_display<R>(
    display: DisplayID,
    f: impl FnOnce(&mut VideoDisplay, &mut X11DisplayData) -> R,
) -> Option<R> {
    with_display(display, |d| {
        let mut internal = d.internal.take();
        let r = internal
            .as_mut()
            .and_then(|b| b.downcast_mut::<X11DisplayData>())
            .map(|data| f(d, data));
        d.internal = internal;
        r
    })
    .flatten()
}

/// A copy of a display's X11 data (`SDL_GetDisplayDriverData()`).
pub(crate) fn display_driver_data(display: DisplayID) -> Option<X11DisplayData> {
    with_x11_display(display, |_, data| data.clone())
}

/// The X11 data of the display a window is on.
/// Translation of `SDL_GetDisplayDriverDataForWindow()`.
pub(crate) fn display_driver_data_for_window(
    window: crate::events::WindowID,
) -> Option<X11DisplayData> {
    let display = crate::video::window::display_for_window(window).ok()?;
    display_driver_data(display)
}

/// Translation of `X11_GetGlobalContentScale()`.
pub(crate) fn x11_get_global_content_scale(
    x: &X11Syms,
    display: *mut Display,
    client: Option<&super::xsettings_client::XSettingsClient>,
) -> f32 {
    let mut scale_factor = 0.0f64;

    // First use the forced scaling factor specified by the app/user
    if let Some(hint) = hints::get(hints::VIDEO_X11_SCALING_FACTOR).filter(|h| !h.is_empty()) {
        let value = crate::stdlib::string::strtod(&hint).0;
        if (1.0..=10.0).contains(&value) {
            scale_factor = value;
        }
    }

    // If that failed, try "Xft.dpi" from the XResourcesDatabase...
    // We attempt to read this directly to get the live value, XResourceManagerString
    // is cached per display connection.
    if scale_factor <= 0.0 {
        let mut real_type: Atom = 0;
        let mut real_format: c_int = 0;
        let mut items_read: c_ulong = 0;
        let mut items_left: c_ulong = 0;
        let mut resource_manager: *mut c_uchar = std::ptr::null_mut();
        let mut owns_resource_manager = false;

        // SAFETY: the display is open; the out-parameters are valid; the
        // resource string is NUL-terminated (property data always is) and
        // freed with XFree when it came from XGetWindowProperty.
        unsafe {
            (x.XrmInitialize)();
            let res_mgr = intern_atom(x, display, "RESOURCE_MANAGER", false);
            let status = (x.XGetWindowProperty)(
                display,
                RootWindow(display, DefaultScreen(display)),
                res_mgr,
                0,
                8192,
                False,
                XA_STRING,
                &mut real_type,
                &mut real_format,
                &mut items_read,
                &mut items_left,
                &mut resource_manager,
            );

            if status == Success && !resource_manager.is_null() {
                owns_resource_manager = true;
            } else {
                // Fall back to XResourceManagerString. This will not be updated if the
                // dpi value is later changed but should allow getting the initial value.
                resource_manager = (x.XResourceManagerString)(display).cast();
            }

            if !resource_manager.is_null() {
                let db = (x.XrmGetStringDatabase)(resource_manager.cast());
                let mut value = XrmValue {
                    size: 0,
                    addr: std::ptr::null_mut(),
                };
                let mut type_: *mut std::ffi::c_char = std::ptr::null_mut();

                // Get the value of Xft.dpi from the Database
                if (x.XrmGetResource)(
                    db,
                    c"Xft.dpi".as_ptr().cast_mut(),
                    c"String".as_ptr().cast_mut(),
                    &mut type_,
                    &mut value,
                ) != 0
                    && !value.addr.is_null()
                    && !type_.is_null()
                    && CStr::from_ptr(type_).to_bytes() == b"String"
                {
                    let dpi =
                        crate::stdlib::string::strtol(CStr::from_ptr(value.addr).to_bytes(), 10).0
                            as c_int;
                    scale_factor = dpi as f64 / 96.0;
                }
                (x.XrmDestroyDatabase)(db);

                if owns_resource_manager {
                    (x.XFree)(resource_manager.cast());
                }
            }
        }
    }

    // If that failed, try the XSETTINGS keys...
    if scale_factor <= 0.0 {
        scale_factor = super::settings::x11_get_xsettings_client_int_key(
            client,
            "Gdk/WindowScalingFactor",
            -1,
        ) as f64;

        // The Xft/DPI key is stored in increments of 1024th
        if scale_factor <= 0.0 {
            let dpi = super::settings::x11_get_xsettings_client_int_key(client, "Xft/DPI", -1);
            if dpi > 0 {
                scale_factor = dpi as f64 / 1024.0;
                scale_factor /= 96.0;
            }
        }
    }

    // If that failed, try the GDK_SCALE envvar...
    if scale_factor <= 0.0 {
        if let Some(scale_str) = crate::stdlib::getenv("GDK_SCALE") {
            scale_factor = crate::stdlib::string::strtol(&scale_str, 10).0 as c_int as f64;
        }
    }

    // Nothing or a bad value, just fall back to 1.0
    if scale_factor <= 0.0 {
        scale_factor = 1.0;
    }

    scale_factor as f32
}

/// Translation of `get_visualinfo()`.
fn get_visualinfo(x: &X11Syms, display: *mut Display, screen: c_int) -> Option<XVisualInfo> {
    // Look for an exact visual, if requested
    if let Some(visual_id) = hints::get(hints::VIDEO_X11_VISUALID).filter(|h| !h.is_empty()) {
        let mut template = XVisualInfo {
            visualid: crate::stdlib::string::strtol(&visual_id, 0).0 as VisualID,
            ..Default::default()
        };
        let mut nvis = 0;
        // SAFETY: the display is open; the template and count are valid;
        // a non-NULL result has at least one entry and is freed here.
        unsafe {
            let vi = (x.XGetVisualInfo)(display, VisualIDMask, &mut template, &mut nvis);
            if !vi.is_null() {
                let vinfo = *vi;
                (x.XFree)(vi.cast());
                return Some(vinfo);
            }
        }
    }

    let mut vinfo = XVisualInfo::default();
    // SAFETY: the display is open and the screen one of its screens; vinfo
    // is a valid out-parameter.
    unsafe {
        let depth = DefaultDepth(display, screen);
        if (x11_use_direct_color_visuals()
            && (x.XMatchVisualInfo)(display, screen, depth, DirectColor, &mut vinfo) != 0)
            || (x.XMatchVisualInfo)(display, screen, depth, TrueColor, &mut vinfo) != 0
            || (x.XMatchVisualInfo)(display, screen, depth, PseudoColor, &mut vinfo) != 0
            || (x.XMatchVisualInfo)(display, screen, depth, StaticColor, &mut vinfo) != 0
        {
            return Some(vinfo);
        }
    }
    Option::None
}

/// Translation of `X11_GetVisualInfoFromVisual()`.
pub(crate) fn x11_get_visual_info_from_visual(
    x: &X11Syms,
    display: *mut Display,
    visual: *mut Visual,
) -> Option<XVisualInfo> {
    let mut nvis = 0;
    // SAFETY: the display is open and the visual one of its visuals; a
    // non-NULL result has at least one entry and is freed here.
    unsafe {
        let mut vinfo = XVisualInfo {
            visualid: (x.XVisualIDFromVisual)(visual),
            ..Default::default()
        };
        let vi = (x.XGetVisualInfo)(display, VisualIDMask, &mut vinfo, &mut nvis);
        if !vi.is_null() {
            let vinfo = *vi;
            (x.XFree)(vi.cast());
            return Some(vinfo);
        }
    }
    Option::None
}

/// Translation of `X11_GetPixelFormatFromVisualInfo()`.
pub(crate) fn x11_get_pixel_format_from_visual_info(
    x: &X11Syms,
    display: *mut Display,
    vinfo: &XVisualInfo,
) -> PixelFormat {
    if vinfo.class == DirectColor || vinfo.class == TrueColor {
        // SAFETY: the visual of a visual info Xlib returned.
        let visual = unsafe { &*vinfo.visual };
        let rmask = visual.red_mask as u32;
        let gmask = visual.green_mask as u32;
        let bmask = visual.blue_mask as u32;
        let amask = if vinfo.depth == 32 {
            !(rmask | gmask | bmask)
        } else {
            0
        };

        let mut bpp = vinfo.depth;
        if bpp == 24 {
            let mut n = 0;
            // SAFETY: the display is open; a non-NULL result has `n`
            // entries and is freed here.
            unsafe {
                let p = (x.XListPixmapFormats)(display, &mut n);
                if !p.is_null() {
                    for format in std::slice::from_raw_parts(p, n.max(0) as usize) {
                        if format.depth == 24 {
                            bpp = format.bits_per_pixel;
                            break;
                        }
                    }
                    (x.XFree)(p.cast());
                }
            }
        }

        return PixelFormat::from_masks(PixelMasks {
            bpp: bpp as u32,
            r: rmask,
            g: gmask,
            b: bmask,
            a: amask,
        })
        .unwrap_or(PixelFormat::UNKNOWN);
    }

    if vinfo.class == PseudoColor || vinfo.class == StaticColor {
        // SAFETY: the display is open.
        let lsb = unsafe { BitmapBitOrder(display) } == LSBFirst;
        match vinfo.depth {
            8 => return PixelFormat::INDEX8,
            4 => {
                return if lsb {
                    PixelFormat::INDEX4LSB
                } else {
                    PixelFormat::INDEX4MSB
                };
            }
            1 => {
                return if lsb {
                    PixelFormat::INDEX1LSB
                } else {
                    PixelFormat::INDEX1MSB
                };
            }
            _ => {}
        }
    }

    PixelFormat::UNKNOWN
}

/// The scanline padding for a visual depth (the `XListPixmapFormats()`
/// loops of `X11_AddGenericDisplay()` and `X11_FillXRandRDisplayInfo()`).
fn scanline_pad_for_depth(x: &X11Syms, dpy: *mut Display, depth: c_int, default: c_int) -> c_int {
    let mut scanline_pad = default;
    let mut n = 0;
    // SAFETY: the display is open; a non-NULL result has `n` entries and is
    // freed here.
    unsafe {
        let pixmapformats = (x.XListPixmapFormats)(dpy, &mut n);
        if !pixmapformats.is_null() {
            for format in std::slice::from_raw_parts(pixmapformats, n.max(0) as usize) {
                if format.depth == depth {
                    scanline_pad = format.scanline_pad;
                    break;
                }
            }
            (x.XFree)(pixmapformats.cast());
        }
    }
    scanline_pad
}

impl X11Video {
    /// Translation of `X11_GetGlobalContentScaleForDevice()`.
    pub(crate) fn x11_get_global_content_scale_for_device(&self) -> f32 {
        self.with_xsettings_client(|client| {
            x11_get_global_content_scale(&self.x, self.display, client)
        })
    }

    /// Translation of `X11_AddGenericDisplay()`.
    fn x11_add_generic_display(&self, send_event: bool) -> Result<DisplayID> {
        // !!! FIXME: a lot of copy/paste from X11_InitModes_XRandR in this function.
        let x = &self.x;
        let dpy = self.display;
        // SAFETY: the display is open.
        let default_screen = unsafe { DefaultScreen(dpy) };
        // SAFETY: the default screen is a screen of the display.
        let screen = unsafe { ScreenOfDisplay(dpy, default_screen) };

        // note that generally even if you have a multiple physical monitors, ScreenCount(dpy) still only reports ONE screen.

        let Some(vinfo) = get_visualinfo(x, dpy, default_screen) else {
            return Err(Error::new(
                "Failed to find an X11 visual for the primary display",
            ));
        };

        let pixelformat = x11_get_pixel_format_from_visual_info(x, dpy, &vinfo);
        if pixelformat.is_indexed() {
            return Err(Error::new("Palettized video modes are no longer supported"));
        }

        // SAFETY: the screen of an open display.
        let mode = unsafe {
            DisplayMode {
                w: WidthOfScreen(screen),
                h: HeightOfScreen(screen),
                format: pixelformat,
                ..Default::default()
            }
        };

        let mut displaydata = X11DisplayData::new();
        displaydata.set_mode_data(&mode, 0);

        displaydata.screen = default_screen;
        displaydata.visual = vinfo.visual;
        displaydata.depth = vinfo.depth;

        let scanline_pad = scanline_pad_for_depth(
            x,
            dpy,
            vinfo.depth,
            pixelformat.bytes_per_pixel() as c_int * 8,
        );

        displaydata.scanline_pad = scanline_pad;
        displaydata.x = 0;
        displaydata.y = 0;
        displaydata.use_xrandr = false;

        let mut display = VideoDisplay::new();
        display.name = Some("Generic X11 Display".to_owned());
        display.desktop_mode = mode;
        display.internal = Some(Box::new(displaydata));
        display.content_scale = self.x11_get_global_content_scale_for_device();
        match add_video_display(display, send_event) {
            0 => Err(crate::video::core::uninitialized_video()),
            id => Ok(id),
        }
    }

    /// Translation of `X11_RemoveGenericDisplay()`.
    fn x11_remove_generic_display(&self) {
        if let Ok(displays) = displays() {
            for id in displays {
                let xrandr_output = with_x11_display(id, |_, data| data.xrandr_output);
                if xrandr_output == Some(0) {
                    del_video_display(id, true);
                }
            }
        }
    }
}

/// Translation of `CheckXRandR()`: the version if XRandR is usable.
fn check_xrandr(x: &X11Syms, display: *mut Display) -> Option<(c_int, c_int)> {
    // Default the extension not available

    // Allow environment override
    if !hints::get_bool(hints::VIDEO_X11_XRANDR, true) {
        // (X11MODES_DEBUG: "XRandR disabled due to hint")
        return Option::None;
    }

    let Some(xrandr) = &x.xrandr else {
        // (X11MODES_DEBUG: "XRandR support not available")
        return Option::None;
    };

    // Query the extension version
    let mut major = 1;
    let mut minor = 3; // we want 1.3
                       // SAFETY: the display is open; the out-parameters are valid.
    if unsafe { (xrandr.XRRQueryVersion)(display, &mut major, &mut minor) } == 0 {
        // (X11MODES_DEBUG: "XRandR not active on the display")
        return Option::None;
    }
    // (X11MODES_DEBUG: "XRandR available at version %d.%d!")
    Some((major, minor))
}

const XRANDR_ROTATION_LEFT: Rotation = 1 << 1;
const XRANDR_ROTATION_RIGHT: Rotation = 1 << 3;

/// Translation of `CalculateXRandRRefreshRate()`: (numerator, denominator).
fn calculate_xrandr_refresh_rate(info: &XRRModeInfo) -> (i32, i32) {
    let mut v_total = info.vTotal;

    if info.modeFlags & RR_DoubleScan != 0 {
        // doublescan doubles the number of lines
        v_total *= 2;
    }

    if info.modeFlags & RR_Interlace != 0 {
        // interlace splits the frame into two fields
        // the field rate is what is typically reported by monitors
        v_total /= 2;
    }

    if info.hTotal != 0 && v_total != 0 {
        (
            info.dotClock as i32,
            info.hTotal.wrapping_mul(v_total) as i32,
        )
    } else {
        (0, 0)
    }
}

/// Translation of `SetXRandRModeInfo()`: fills in `mode` (and its
/// `xrandr_mode`, the returned value) if `mode_id` is one of `res`'s modes.
///
/// # Safety
///
/// `res` must be valid screen resources of the open `display`.
unsafe fn set_xrandr_mode_info(
    x: &X11Syms,
    display: *mut Display,
    res: *mut XRRScreenResources,
    crtc: RRCrtc,
    mode_id: RRMode,
    mode: &mut DisplayMode,
) -> Option<RRMode> {
    let xrandr = x.xrandr.as_ref()?;
    // SAFETY: the caller's contract; `modes` has `nmode` entries.
    let modes = unsafe { std::slice::from_raw_parts((*res).modes, (*res).nmode.max(0) as usize) };
    for info in modes {
        if info.id == mode_id {
            let mut rotation: Rotation = 0;
            let mut scale_w: XFixed = 0x10000;
            let mut scale_h: XFixed = 0x10000;
            let mut attr: *mut XRRCrtcTransformAttributes = std::ptr::null_mut();

            // SAFETY: the caller's contract; the results are freed here.
            unsafe {
                let crtcinfo = (xrandr.XRRGetCrtcInfo)(display, res, crtc);
                if !crtcinfo.is_null() {
                    rotation = (*crtcinfo).rotation;
                    (xrandr.XRRFreeCrtcInfo)(crtcinfo);
                }
                if (xrandr.XRRGetCrtcTransform)(display, crtc, &mut attr) != 0 && !attr.is_null() {
                    scale_w = (*attr).currentTransform.matrix[0][0];
                    scale_h = (*attr).currentTransform.matrix[1][1];
                    (x.XFree)(attr.cast());
                }
            }

            let scaled = |v: u32, scale: XFixed| ((v as i64 * scale as i64 + 0xffff) >> 16) as i32;
            if rotation & (XRANDR_ROTATION_LEFT | XRANDR_ROTATION_RIGHT) != 0 {
                mode.w = scaled(info.height, scale_w);
                mode.h = scaled(info.width, scale_h);
            } else {
                mode.w = scaled(info.width, scale_w);
                mode.h = scaled(info.height, scale_h);
            }
            let (num, den) = calculate_xrandr_refresh_rate(info);
            mode.refresh_rate_numerator = num;
            mode.refresh_rate_denominator = den;
            // (X11MODES_DEBUG logs the mode here)
            return Some(mode_id);
        }
    }
    Option::None
}

/// Translation of `GetRootWindowCardinalProperty()`.
fn get_root_window_cardinal_property(
    x: &X11Syms,
    dpy: *mut Display,
    screen: c_int,
    name: &str,
) -> Option<u32> {
    let atom = intern_atom(x, dpy, name, false);
    if atom == None {
        return Option::None;
    }

    let mut real_format: c_int = 0;
    let mut real_type: Atom = 0;
    let mut items_read: c_ulong = 0;
    let mut items_left: c_ulong = 0;
    let mut propdata: *mut c_uchar = std::ptr::null_mut();
    let mut result = Option::None;

    // SAFETY: the display is open; the out-parameters are valid; the
    // property data has at least `items_read` items of `real_format` bits
    // (longs for 32) and is freed here.
    unsafe {
        let status = (x.XGetWindowProperty)(
            dpy,
            RootWindow(dpy, screen),
            atom,
            0,
            1024,
            False,
            AnyPropertyType,
            &mut real_type,
            &mut real_format,
            &mut items_read,
            &mut items_left,
            &mut propdata,
        );
        if status == Success && !propdata.is_null() {
            if real_type == XA_CARDINAL && items_read > 0 {
                let p = |i: usize| *propdata.add(i) as u32;
                result = Some(match real_format {
                    8 => p(0),
                    16 => p(0) | (p(1) << 8),
                    _ => p(0) | (p(1) << 8) | (p(2) << 16) | (p(3) << 24),
                });
            }
            (x.XFree)(propdata.cast());
        }
    }
    result
}

/// Translation of `GetRootWindowBoolProperty()`.
fn get_root_window_bool_property(
    x: &X11Syms,
    dpy: *mut Display,
    screen: c_int,
    name: &str,
) -> Option<bool> {
    get_root_window_cardinal_property(x, dpy, screen, name).map(|v| v != 0)
}

/// Translation of `GetRootWindowFloatProperty()`.
fn get_root_window_float_property(
    x: &X11Syms,
    dpy: *mut Display,
    screen: c_int,
    name: &str,
) -> Option<f32> {
    get_root_window_cardinal_property(x, dpy, screen, name).map(f32::from_bits)
}

/// Translation of `GetGamescopeSDRWhiteLevel()`.
fn get_gamescope_sdr_white_level(x: &X11Syms) -> f32 {
    // These properties are currently only available on X display :0
    // If this changes, please remove the XCloseDisplay() call below.
    // SAFETY: the name is NUL-terminated.
    let dpy = unsafe { (x.XOpenDisplay)(c":0".as_ptr()) };
    let screen = 0;
    if dpy.is_null() {
        return 0.0;
    }

    let mut sdr_white_level = 0.0;
    let external_display =
        get_root_window_bool_property(x, dpy, screen, "GAMESCOPE_DISPLAY_IS_EXTERNAL");
    if external_display == Some(true) {
        if let Some(level) = get_root_window_float_property(
            x,
            dpy,
            screen,
            "GAMESCOPE_SDR_ON_HDR_CONTENT_BRIGHTNESS",
        ) {
            sdr_white_level = level;
        }
    } else {
        // We're using the Steam Deck internal display, which has an SDR white level of 500 nits
        sdr_white_level = 500.0;
    }

    // Closing the display opened above
    // SAFETY: the display opened above, closed once.
    unsafe {
        (x.XCloseDisplay)(dpy);
    }

    sdr_white_level
}

/// Translation of `GetMonitorInfo()`: the EDID of an output, and whether it
/// came from gamescope.
fn get_monitor_info(
    x: &X11Syms,
    dpy: *mut Display,
    screen: c_int,
    output: RROutput,
) -> (Option<MonitorInfo>, bool) {
    let mut info: Option<MonitorInfo> = Option::None;
    let mut real_format: c_int = 0;
    let mut real_type: Atom = 0;
    let mut items_read: c_ulong = 0;
    let mut items_left: c_ulong = 0;
    let mut propdata: *mut c_uchar = std::ptr::null_mut();
    let mut gamescope = false;

    if info.is_none() {
        let gamescope_display_edid_path = intern_atom(x, dpy, "GAMESCOPE_DISPLAY_EDID_PATH", false);
        if gamescope_display_edid_path != None {
            // SAFETY: the display is open; the out-parameters are valid;
            // the property data is NUL-terminated and freed here.
            unsafe {
                let status = (x.XGetWindowProperty)(
                    dpy,
                    RootWindow(dpy, screen),
                    gamescope_display_edid_path,
                    0,
                    1024,
                    False,
                    AnyPropertyType,
                    &mut real_type,
                    &mut real_format,
                    &mut items_read,
                    &mut items_left,
                    &mut propdata,
                );
                if status == Success && !propdata.is_null() {
                    let path = CStr::from_ptr(propdata.cast())
                        .to_string_lossy()
                        .into_owned();
                    if let Ok(data) = crate::io::load_file(&path) {
                        info = decode_edid(&data);
                        if info.is_some() {
                            gamescope = true;
                        }
                    }
                }
                if !propdata.is_null() {
                    (x.XFree)(propdata.cast());
                    propdata = std::ptr::null_mut();
                }
            }
        }
    }

    if info.is_none() {
        let edid = intern_atom(x, dpy, "EDID", false);
        if let (true, Some(xrandr)) = (edid != None, &x.xrandr) {
            let mut nprop = 0;
            // SAFETY: the display is open and the output one of its
            // outputs; the property list has `nprop` atoms, the property
            // data `items_read` bytes; both are freed here.
            unsafe {
                let props = (xrandr.XRRListOutputProperties)(dpy, output, &mut nprop);
                if !props.is_null() {
                    for &prop in std::slice::from_raw_parts(props, nprop.max(0) as usize) {
                        if prop == edid {
                            let status = (xrandr.XRRGetOutputProperty)(
                                dpy,
                                output,
                                prop,
                                0,
                                128,
                                False,
                                False,
                                AnyPropertyType,
                                &mut real_type,
                                &mut real_format,
                                &mut items_read,
                                &mut items_left,
                                &mut propdata,
                            );
                            if status == Success && !propdata.is_null() {
                                info = decode_edid(std::slice::from_raw_parts(
                                    propdata,
                                    items_read as usize,
                                ));
                            }
                            if !propdata.is_null() {
                                (x.XFree)(propdata.cast());
                            }
                            break;
                        }
                    }
                    (x.XFree)(props.cast());
                }
            }
        }
    }

    // (X11MODES_DEBUG: dump_monitor_info(info))
    (info, gamescope)
}

/// Translation of `SetXRandRDisplayName()`.
fn set_xrandr_display_name(
    info: Option<&MonitorInfo>,
    name: &mut String,
    namelen: usize,
    widthmm: c_ulong,
    heightmm: c_ulong,
) {
    if let Some(info) = info {
        *name = c_field(&info.dsc_product_name);
        truncate_to(name, namelen - 1);
    }

    let (w, h) = (widthmm as f32, heightmm as f32);
    let inches = ((sqrtf(w * w + h * h) / 25.4) + 0.5) as i32;
    if !name.is_empty() && inches != 0 {
        name.push_str(&format!(" {inches}\""));
        truncate_to(name, namelen - 1);
    }

    // (X11MODES_DEBUG: "Display name: %s")
}

/// `SDL_strlcpy()`'s truncation to a buffer of `max + 1` bytes.
fn truncate_to(s: &mut String, max: usize) {
    if s.len() > max {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
}

/// The size of the `display_name` buffers of upstream.
const DISPLAY_NAME_SIZE: usize = 128;
/// The size of `SDL_DisplayData::connector_name`.
const CONNECTOR_NAME_SIZE: usize = 16;

impl X11Video {
    /// Translation of `X11_FillXRandRDisplayInfo()`.
    ///
    /// # Safety
    ///
    /// `res` must be valid screen resources of the screen of `dpy`.
    unsafe fn x11_fill_xrandr_display_info(
        &self,
        dpy: *mut Display,
        screen: c_int,
        outputid: RROutput,
        res: *mut XRRScreenResources,
    ) -> Option<VideoDisplay> {
        let x = &self.x;
        let xrandr = x.xrandr.as_ref()?;

        let vinfo = get_visualinfo(x, dpy, screen)?; // uh, skip this screen?

        let pixelformat = x11_get_pixel_format_from_visual_info(x, dpy, &vinfo);
        if pixelformat.is_indexed() {
            return Option::None; // Palettized video modes are no longer supported, ignore this one.
        }

        let scanline_pad = scanline_pad_for_depth(
            x,
            dpy,
            vinfo.depth,
            pixelformat.bytes_per_pixel() as c_int * 8,
        );

        let mut display_name;
        let display_mm_width;
        let display_mm_height;
        let output_crtc;
        // SAFETY: the caller's contract; the output info is freed here.
        unsafe {
            let output_info = (xrandr.XRRGetOutputInfo)(dpy, res, outputid);
            if output_info.is_null()
                || (*output_info).crtc == 0
                || (*output_info).connection == RR_Disconnected
            {
                if !output_info.is_null() {
                    (xrandr.XRRFreeOutputInfo)(output_info);
                }
                return Option::None; // ignore this one.
            }

            display_name = CStr::from_ptr((*output_info).name)
                .to_string_lossy()
                .into_owned();
            truncate_to(&mut display_name, DISPLAY_NAME_SIZE - 1);
            display_mm_width = (*output_info).mm_width;
            display_mm_height = (*output_info).mm_height;
            output_crtc = (*output_info).crtc;
            (xrandr.XRRFreeOutputInfo)(output_info);
        }

        let mode_id;
        let mut mode;
        let display_x;
        let display_y;
        // SAFETY: as above; the crtc info is freed here.
        unsafe {
            let crtc = (xrandr.XRRGetCrtcInfo)(dpy, res, output_crtc);
            if crtc.is_null() {
                return Option::None; // oh well, ignore it.
            }

            mode_id = (*crtc).mode;
            mode = DisplayMode {
                w: (*crtc).width as i32,
                h: (*crtc).height as i32,
                format: pixelformat,
                ..Default::default()
            };

            display_x = (*crtc).x;
            display_y = (*crtc).y;

            (xrandr.XRRFreeCrtcInfo)(crtc);
        }

        let mut displaydata = X11DisplayData::new();

        displaydata.screen = screen;
        displaydata.visual = vinfo.visual;
        displaydata.depth = vinfo.depth;
        displaydata.scanline_pad = scanline_pad;
        displaydata.x = display_x;
        displaydata.y = display_y;
        displaydata.use_xrandr = true;
        displaydata.xrandr_output = outputid;
        displaydata.connector_name = display_name.clone();
        truncate_to(&mut displaydata.connector_name, CONNECTOR_NAME_SIZE - 1);

        let (info, gamescope) = get_monitor_info(x, dpy, screen, outputid);

        // SAFETY: the caller's contract.
        unsafe {
            set_xrandr_mode_info(x, dpy, res, output_crtc, mode_id, &mut mode);
        }
        displaydata.set_mode_data(&mode, mode_id);
        set_xrandr_display_name(
            info.as_ref(),
            &mut display_name,
            DISPLAY_NAME_SIZE,
            display_mm_width,
            display_mm_height,
        );

        let mut display = VideoDisplay::new();
        if !display_name.is_empty() {
            display.name = Some(display_name);
        }
        display.desktop_mode = mode;
        display.content_scale = self.x11_get_global_content_scale_for_device();

        if let Some(info) = info {
            let sdr_white_level = if gamescope {
                get_gamescope_sdr_white_level(x)
            } else {
                // Support for HDR on X11 seems spotty, let's disable this for now
                0.0
            };

            // (X11MODES_DEBUG: "HDR values: %f / %f")
            if sdr_white_level > 0.0 && info.max_luminance as f32 > sdr_white_level {
                display.hdr.hdr_headroom = info.max_luminance as f32 / sdr_white_level;
                display.hdr.sdr_white_level = sdr_white_level / SCRGB_NITS;
            }
        }
        display.internal = Some(Box::new(displaydata));

        Some(display)
    }

    /// Translation of `X11_AddXRandRDisplay()`.
    ///
    /// # Safety
    ///
    /// As for [`x11_fill_xrandr_display_info`](Self::x11_fill_xrandr_display_info).
    unsafe fn x11_add_xrandr_display(
        &self,
        dpy: *mut Display,
        screen: c_int,
        outputid: RROutput,
        res: *mut XRRScreenResources,
        send_event: bool,
    ) -> bool {
        // SAFETY: the caller's contract.
        let Some(display) =
            (unsafe { self.x11_fill_xrandr_display_info(dpy, screen, outputid, res) })
        else {
            return true; // failed to query data, skip this display
        };

        let display_id = add_video_display(display, false);
        if display_id == 0 {
            return false;
        }

        // We added an XRandR display, remove the generic display, if any
        self.x11_remove_generic_display();

        if send_event {
            send_display_event(display_id, EventType::DISPLAY_ADDED, 0, 0);
        }
        true
    }

    /// Translation of `X11_UpdateXRandRDisplay()`.
    ///
    /// # Safety
    ///
    /// As for [`x11_fill_xrandr_display_info`](Self::x11_fill_xrandr_display_info).
    unsafe fn x11_update_xrandr_display(
        &self,
        dpy: *mut Display,
        screen: c_int,
        outputid: RROutput,
        res: *mut XRRScreenResources,
        existing_display: DisplayID,
    ) -> bool {
        // SAFETY: the caller's contract.
        let Some(display) =
            (unsafe { self.x11_fill_xrandr_display_info(dpy, screen, outputid, res) })
        else {
            return false; // failed to query current display state
        };
        let Some(new_data) = display
            .internal
            .as_ref()
            .and_then(|b| b.downcast_ref::<X11DisplayData>())
            .cloned()
        else {
            return false;
        };

        // update mode - this call takes ownership of display.desktop_mode.internal
        let desktop_mode_id = new_data.mode_data(&display.desktop_mode).unwrap_or(0);
        with_x11_display(existing_display, |_, data| {
            data.set_mode_data(&display.desktop_mode, desktop_mode_id);
        });
        set_desktop_display_mode(existing_display, &display.desktop_mode);

        // update bounds
        let moved = with_x11_display(existing_display, |_, data| {
            if data.x != new_data.x || data.y != new_data.y {
                data.x = new_data.x;
                data.y = new_data.y;
                true
            } else {
                false
            }
        })
        .unwrap_or(false);
        if moved {
            send_display_event(existing_display, EventType::DISPLAY_MOVED, 0, 0);
        }

        // update scale
        set_display_content_scale(existing_display, display.content_scale);

        // update HDR properties
        set_display_hdr_properties(existing_display, display.hdr);

        // SDL_DisplayData is updated piece-meal above, free our local copy of this data
        true
    }
}

/// Translation of `X11_GetScreenResources()`.
fn x11_get_screen_resources(
    x: &X11Syms,
    dpy: *mut Display,
    screen: c_int,
) -> *mut XRRScreenResources {
    let Some(xrandr) = &x.xrandr else {
        return std::ptr::null_mut();
    };
    // SAFETY: the display is open and the screen one of its screens.
    unsafe {
        let mut res = (xrandr.XRRGetScreenResourcesCurrent)(dpy, RootWindow(dpy, screen));
        if res.is_null() || (*res).noutput == 0 {
            if !res.is_null() {
                (xrandr.XRRFreeScreenResources)(res);
            }
            res = (xrandr.XRRGetScreenResources)(dpy, RootWindow(dpy, screen));
        }
        res
    }
}

/// The outputs of screen resources.
///
/// # Safety
///
/// `res` must be valid screen resources.
unsafe fn res_outputs<'a>(res: *mut XRRScreenResources) -> &'a [RROutput] {
    // SAFETY: the caller's contract; `outputs` has `noutput` entries.
    unsafe { std::slice::from_raw_parts((*res).outputs, (*res).noutput.max(0) as usize) }
}

impl X11Video {
    /// Translation of `X11_CheckDisplaysMoved()`.
    pub(crate) fn x11_check_displays_moved(&self, dpy: *mut Display) {
        let Some(xrandr) = &self.x.xrandr else {
            return;
        };
        // SAFETY: the display is open.
        let screencount = unsafe { ScreenCount(dpy) };

        let Ok(displays) = displays() else {
            return;
        };

        for screen in 0..screencount {
            let res = x11_get_screen_resources(&self.x, dpy, screen);
            if res.is_null() {
                continue;
            }

            for &display in &displays {
                let Some((output, data_screen)) =
                    with_x11_display(display, |_, data| (data.xrandr_output, data.screen))
                else {
                    continue;
                };
                if output != 0 && data_screen == screen {
                    // SAFETY: res is valid resources of this screen.
                    unsafe {
                        self.x11_update_xrandr_display(dpy, screen, output, res, display);
                    }
                }
            }
            // SAFETY: res came from XRRGetScreenResources*.
            unsafe {
                (xrandr.XRRFreeScreenResources)(res);
            }
        }
    }

    /// Translation of `X11_CheckDisplaysRemoved()`.
    fn x11_check_displays_removed(&self, dpy: *mut Display) {
        let Some(xrandr) = &self.x.xrandr else {
            return;
        };
        // SAFETY: the display is open.
        let screencount = unsafe { ScreenCount(dpy) };

        let Ok(mut displays) = displays() else {
            return;
        };

        for screen in 0..screencount {
            let res = x11_get_screen_resources(&self.x, dpy, screen);
            if res.is_null() {
                continue;
            }

            // SAFETY: res is valid until freed below.
            for &output in unsafe { res_outputs(res) } {
                for display in displays.iter_mut() {
                    if *display == 0 {
                        // We already removed this display from the list
                        continue;
                    }

                    let xrandr_output = with_x11_display(*display, |_, data| data.xrandr_output);
                    if xrandr_output == Some(output) {
                        // This display is active, remove it from the list
                        *display = 0;
                        break;
                    }
                }
            }
            // SAFETY: res came from XRRGetScreenResources*.
            unsafe {
                (xrandr.XRRFreeScreenResources)(res);
            }
        }

        for &display in &displays {
            if display != 0 {
                let xrandr_output = with_x11_display(display, |_, data| data.xrandr_output);
                if xrandr_output.is_some_and(|o| o != 0) {
                    // This display wasn't in the XRandR list
                    del_video_display(display, true);
                }
            }
        }
    }

    /// Translation of `X11_HandleXRandROutputChange()`.
    fn x11_handle_xrandr_output_change(&self, ev: &XRROutputChangeNotifyEvent) {
        let mut display = 0;

        // (SDL_Log of the event, disabled upstream)

        // XWayland doesn't always send output disconnected events
        self.x11_check_displays_removed(ev.display);

        if let Ok(displays) = displays() {
            for id in displays {
                if with_x11_display(id, |_, data| data.xrandr_output) == Some(ev.output) {
                    display = id;
                    break;
                }
            }
        }

        if ev.connection == RR_Disconnected {
            // output is going away
            if display != 0 {
                // Add the generic display if we're about to remove the last XRandR display
                let mut generic_display = 0;
                if displays().map(|d| d.len()).unwrap_or(0) == 1 {
                    generic_display = self.x11_add_generic_display(false).unwrap_or(0);
                }

                del_video_display(display, true);

                if generic_display != 0 {
                    send_display_event(generic_display, EventType::DISPLAY_ADDED, 0, 0);
                }
            }
            self.x11_check_displays_moved(ev.display);
        } else if ev.connection == RR_Connected {
            // output is coming online
            if display == 0 {
                let dpy = ev.display;
                // SAFETY: the event's display is our open display.
                let screen = unsafe { DefaultScreen(dpy) };
                let res = x11_get_screen_resources(&self.x, dpy, screen);
                if !res.is_null() {
                    // SAFETY: res is valid resources of this screen, freed
                    // here.
                    unsafe {
                        self.x11_add_xrandr_display(dpy, screen, ev.output, res, true);
                        if let Some(xrandr) = &self.x.xrandr {
                            (xrandr.XRRFreeScreenResources)(res);
                        }
                    }
                }
            }
            self.x11_check_displays_moved(ev.display);
        }
    }

    /// Translation of `X11_HandleXRandREvent()`.
    pub(crate) fn x11_handle_xrandr_event(&self, xevent: &XEvent) {
        let xrandr_event_base = self.with_data(|d| d.xrandr_event_base);
        crate::sdl_assert!(xevent.get_type() == xrandr_event_base + RRNotify);

        // SAFETY: an RRNotify event is an XRRNotifyEvent.
        let notify = unsafe { xevent.cast::<XRRNotifyEvent>() };
        if notify.subtype == RRNotify_OutputChange {
            // SAFETY: an RRNotify_OutputChange event is an
            // XRROutputChangeNotifyEvent.
            let change = unsafe { xevent.cast::<XRROutputChangeNotifyEvent>() };
            self.x11_handle_xrandr_output_change(change);
        }
    }

    /// Translation of `X11_SortOutputsByPriorityHint()`.
    fn x11_sort_outputs_by_priority_hint(&self) {
        let Some(name_hint) = hints::get(hints::VIDEO_DISPLAY_PRIORITY) else {
            return;
        };

        let _ = with_device(|v| {
            let mut remaining: Vec<Option<VideoDisplay>> = v.displays.drain(..).map(Some).collect();
            let mut sorted_list: Vec<VideoDisplay> = Vec::with_capacity(remaining.len());

            // Sort the requested displays to the front of the list.
            for token in name_hint.split(',').filter(|t| !t.is_empty()) {
                for slot in remaining.iter_mut() {
                    let matches = slot.as_ref().is_some_and(|d| {
                        d.internal
                            .as_ref()
                            .and_then(|b| b.downcast_ref::<X11DisplayData>())
                            .is_some_and(|data| data.connector_name == token)
                    });
                    if matches {
                        sorted_list.extend(slot.take());
                        break;
                    }
                }
            }

            // Append the remaining displays to the end of the list.
            sorted_list.extend(remaining.into_iter().flatten());

            // Copy the sorted list back to the display list.
            v.displays = sorted_list;
        });
    }

    /// Translation of `X11_InitModes_XRandR()`.
    fn x11_init_modes_xrandr(&self) -> Result<()> {
        let Some(xrandr) = &self.x.xrandr else {
            return Err(Error::new("XRRQueryExtension failed"));
        };
        let dpy = self.display;
        // SAFETY: the display is open.
        let (screencount, default_screen) = unsafe { (ScreenCount(dpy), DefaultScreen(dpy)) };
        // SAFETY: as above.
        let primary = unsafe { (xrandr.XRRGetOutputPrimary)(dpy, RootWindow(dpy, default_screen)) };
        let mut xrandr_event_base = 0;
        let mut xrandr_error_base = 0;

        // SAFETY: as above; the out-parameters are valid.
        if unsafe {
            (xrandr.XRRQueryExtension)(dpy, &mut xrandr_event_base, &mut xrandr_error_base)
        } == 0
        {
            return Err(Error::new("XRRQueryExtension failed"));
        }
        self.with_data(|d| d.xrandr_event_base = xrandr_event_base);

        for looking_for_primary in [true, false] {
            for screen in 0..screencount {
                // we want the primary output first, and then skipped later.
                if looking_for_primary && screen != default_screen {
                    continue;
                }

                let res = x11_get_screen_resources(&self.x, dpy, screen);
                if res.is_null() {
                    continue;
                }

                // SAFETY: res is valid until freed below.
                for &output in unsafe { res_outputs(res) } {
                    // The primary output _should_ always be sorted first, but just in case...
                    if (looking_for_primary && output != primary)
                        || (!looking_for_primary && screen == default_screen && output == primary)
                    {
                        continue;
                    }
                    // SAFETY: res is valid resources of this screen.
                    if !unsafe { self.x11_add_xrandr_display(dpy, screen, output, res, false) } {
                        break;
                    }
                }

                // SAFETY: res came from XRRGetScreenResources*; the root
                // window is of an open display.
                unsafe {
                    (xrandr.XRRFreeScreenResources)(res);

                    // This will generate events for displays that come and go at runtime.
                    (xrandr.XRRSelectInput)(dpy, RootWindow(dpy, screen), RROutputChangeNotifyMask);
                }
            }
        }

        if displays().map(|d| d.is_empty()).unwrap_or(true) {
            return Err(Error::new("No available displays"));
        }

        self.x11_sort_outputs_by_priority_hint();

        Ok(())
    }

    /// This is used if there's no better functionality--like XRandR--to use.
    /// It won't attempt to supply different display modes at all, but it can
    /// enumerate the current displays and their current sizes.
    /// Translation of `X11_InitModes_StdXlib()`.
    fn x11_init_modes_std_xlib(&self) -> Result<()> {
        self.x11_add_generic_display(true).map(|_| ())
    }

    /// Translation of `X11_InitModes()`.
    pub(crate) fn x11_init_modes(&self) -> Result<()> {
        /* XRandR is the One True Modern Way to do this on X11. If this
        fails, we just won't report any display modes except the current
        desktop size. */
        // require at least XRandR v1.3
        if let Some((xrandr_major, xrandr_minor)) = check_xrandr(&self.x, self.display) {
            if (xrandr_major >= 2 || (xrandr_major == 1 && xrandr_minor >= 3))
                && self.x11_init_modes_xrandr().is_ok()
            {
                return Ok(());
            }
        }

        // still here? Just set up an extremely basic display.
        self.x11_init_modes_std_xlib()
    }

    /// Translation of `X11_GetDisplayModes()`.
    pub(crate) fn x11_get_display_modes(&self, sdl_display: DisplayID) -> Result<()> {
        let Some((data, desktop_format)) =
            with_x11_display(sdl_display, |d, data| (data.clone(), d.desktop_mode.format))
        else {
            return Ok(());
        };

        /* Unfortunately X11 requires the window to be created with the correct
         * visual and depth ahead of time, but the SDL API allows you to create
         * a window before setting the fullscreen display mode.  This means that
         * we have to use the same format for all windows and all display modes.
         * (or support recreating the window with a new visual behind the scenes)
         */
        let mut mode = DisplayMode {
            format: desktop_format,
            ..Default::default()
        };

        if data.use_xrandr {
            let Some(xrandr) = &self.x.xrandr else {
                return Ok(());
            };
            let display = self.display;

            // SAFETY: the display is open; the resources and output info
            // are valid until freed below.
            unsafe {
                let res = (xrandr.XRRGetScreenResources)(display, RootWindow(display, data.screen));
                if !res.is_null() {
                    let output_info = (xrandr.XRRGetOutputInfo)(display, res, data.xrandr_output);
                    if !output_info.is_null() && (*output_info).connection != RR_Disconnected {
                        let modes = std::slice::from_raw_parts(
                            (*output_info).modes,
                            (*output_info).nmode.max(0) as usize,
                        );
                        for &mode_id in modes {
                            if let Some(xrandr_mode) = set_xrandr_mode_info(
                                &self.x,
                                display,
                                res,
                                (*output_info).crtc,
                                mode_id,
                                &mut mode,
                            ) {
                                if add_fullscreen_display_mode(sdl_display, &mode) {
                                    with_x11_display(sdl_display, |_, d| {
                                        d.set_mode_data(&mode, xrandr_mode)
                                    });
                                }
                            }
                        }
                    }
                    if !output_info.is_null() {
                        (xrandr.XRRFreeOutputInfo)(output_info);
                    }
                    (xrandr.XRRFreeScreenResources)(res);
                }
            }
        }
        Ok(())
    }
}

/// The error handler `X11_SetDisplayMode()` replaced
/// (`PreXRRSetScreenSizeErrorHandler`).
static PRE_XRR_SET_SCREEN_SIZE_ERROR_HANDLER: std::sync::Mutex<XErrorHandler> =
    std::sync::Mutex::new(Option::None);

/// This catches an error from XRRSetScreenSize, as a workaround for now.
/// !!! FIXME: remove this later when we have a better solution.
/// Translation of `SDL_XRRSetScreenSizeErrHandler()`.
unsafe extern "C" fn sdl_xrr_set_screen_size_err_handler(
    d: *mut Display,
    e: *mut XErrorEvent,
) -> c_int {
    // BadMatch: https://github.com/libsdl-org/SDL/issues/4561
    // BadValue: https://github.com/libsdl-org/SDL/issues/4840
    // SAFETY: Xlib passes a valid error event.
    let code = unsafe { (*e).error_code };
    if code == BadMatch || code == BadValue {
        return 0;
    }

    let handler = *PRE_XRR_SET_SCREEN_SIZE_ERROR_HANDLER
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match handler {
        // SAFETY: the previous handler, called as Xlib would.
        Some(h) => unsafe { h(d, e) },
        Option::None => 0,
    }
}

impl X11Video {
    /// Translation of `X11_SetDisplayMode()`.
    pub(crate) fn x11_set_display_mode(
        &self,
        sdl_display: DisplayID,
        mode: &DisplayMode,
    ) -> Result<()> {
        let now = crate::timer::ticks_ms();
        self.with_data(|d| d.last_mode_change_deadline = now + (PENDING_FOCUS_TIME * 2));

        // (upstream compares the mode pointers; the current mode is compared
        // by value here)
        let current = crate::video::display::current_display_mode(sdl_display).ok();

        // XWayland mode switches are emulated with viewports and thus instantaneous.
        if !self.is_xwayland {
            let deadline = if current.as_ref() != Some(mode) {
                crate::timer::ticks_ns() + MODE_SWITCH_TIMEOUT_NS
            } else {
                0
            };
            with_x11_display(sdl_display, |_, data| {
                data.mode_switch_deadline_ns = deadline
            });
        }

        let Some(data) = display_driver_data(sdl_display) else {
            return Ok(());
        };

        if data.use_xrandr {
            let Some(xrandr) = &self.x.xrandr else {
                return Ok(());
            };
            let display = self.display;
            let xrandr_mode = data.mode_data(mode).unwrap_or(0);
            let mut status;

            // SAFETY: the display is open; every Xlib/XRandR object is
            // checked for NULL and freed below.
            unsafe {
                let res = (xrandr.XRRGetScreenResources)(display, RootWindow(display, data.screen));
                if res.is_null() {
                    return Err(Error::new("Couldn't get XRandR screen resources"));
                }

                let output_info = (xrandr.XRRGetOutputInfo)(display, res, data.xrandr_output);
                if output_info.is_null() || (*output_info).connection == RR_Disconnected {
                    if !output_info.is_null() {
                        (xrandr.XRRFreeOutputInfo)(output_info);
                    }
                    (xrandr.XRRFreeScreenResources)(res);
                    return Err(Error::new("Couldn't get XRandR output info"));
                }

                let crtc = (xrandr.XRRGetCrtcInfo)(display, res, (*output_info).crtc);
                if crtc.is_null() {
                    (xrandr.XRRFreeOutputInfo)(output_info);
                    (xrandr.XRRFreeScreenResources)(res);
                    return Err(Error::new("Couldn't get XRandR crtc info"));
                }

                if (*crtc).mode == xrandr_mode {
                    // (X11MODES_DEBUG: "already in desired mode 0x%lx (%ux%u), nothing to do")
                    status = Success;
                } else {
                    (self.x.XGrabServer)(display);
                    status = (xrandr.XRRSetCrtcConfig)(
                        display,
                        res,
                        (*output_info).crtc,
                        CurrentTime,
                        0,
                        0,
                        None,
                        (*crtc).rotation,
                        std::ptr::null_mut(),
                        0,
                    );
                    if status == Success {
                        let mm_width = mode.w * DisplayWidthMM(display, data.screen)
                            / DisplayWidth(display, data.screen);
                        let mm_height = mode.h * DisplayHeightMM(display, data.screen)
                            / DisplayHeight(display, data.screen);

                        /* !!! FIXME: this can get into a problem scenario when a window is
                        bigger than a physical monitor in a configuration where one screen
                        spans multiple physical monitors. A detailed reproduction case is
                        discussed at https://github.com/libsdl-org/SDL/issues/4561 ...
                        for now we cheat and just catch the X11 error and carry on, which
                        is likely to cause subtle issues but is better than outright
                        crashing */
                        (self.x.XSync)(display, False);
                        let previous =
                            (self.x.XSetErrorHandler)(Some(sdl_xrr_set_screen_size_err_handler));
                        *PRE_XRR_SET_SCREEN_SIZE_ERROR_HANDLER
                            .lock()
                            .unwrap_or_else(|e| e.into_inner()) = previous;
                        (xrandr.XRRSetScreenSize)(
                            display,
                            RootWindow(display, data.screen),
                            mode.w,
                            mode.h,
                            mm_width,
                            mm_height,
                        );
                        (self.x.XSync)(display, False);
                        (self.x.XSetErrorHandler)(previous);

                        let mut output = data.xrandr_output;
                        status = (xrandr.XRRSetCrtcConfig)(
                            display,
                            res,
                            (*output_info).crtc,
                            CurrentTime,
                            (*crtc).x,
                            (*crtc).y,
                            xrandr_mode,
                            (*crtc).rotation,
                            &mut output,
                            1,
                        );
                    }

                    // ungrabServer:
                    (self.x.XUngrabServer)(display);
                }
                // freeInfo:
                (xrandr.XRRFreeCrtcInfo)(crtc);
                (xrandr.XRRFreeOutputInfo)(output_info);
                (xrandr.XRRFreeScreenResources)(res);
            }

            if status != Success {
                return Err(Error::new("X11_XRRSetCrtcConfig failed"));
            }
        }

        Ok(())
    }

    /// Translation of `X11_QuitModes()`.
    pub(crate) fn x11_quit_modes(&self) {}

    /// Translation of `X11_GetDisplayBounds()`.
    pub(crate) fn x11_get_display_bounds(&self, sdl_display: DisplayID) -> Result<Rect> {
        with_x11_display(sdl_display, |d, data| {
            let current = d.current_mode();
            Rect::new(data.x, data.y, current.w, current.h)
        })
        .ok_or_else(|| Error::new("Invalid display"))
    }

    /// Translation of `X11_GetDisplayUsableBounds()`.
    pub(crate) fn x11_get_display_usable_bounds(&self, sdl_display: DisplayID) -> Result<Rect> {
        let display = self.display;
        let x = &self.x;
        let mut real_format: c_int = 0;
        let mut real_type: Atom = 0;
        let mut items_read: c_ulong = 0;
        let mut items_left: c_ulong = 0;
        let mut propdata: *mut c_uchar = std::ptr::null_mut();
        let mut result = Err(Error::new("Couldn't get _NET_WORKAREA"));

        let mut rect = self.x11_get_display_bounds(sdl_display)?;

        let _NET_WORKAREA = self.intern_atom("_NET_WORKAREA", false);
        // SAFETY: the display is open; the out-parameters are valid; the
        // property data (at least 4 longs when items_read >= 4) is freed
        // here.
        unsafe {
            let status = (x.XGetWindowProperty)(
                display,
                DefaultRootWindow(display),
                _NET_WORKAREA,
                0,
                4,
                False,
                XA_CARDINAL,
                &mut real_type,
                &mut real_format,
                &mut items_read,
                &mut items_left,
                &mut propdata,
            );
            if status == Success && items_read >= 4 {
                let p = propdata as *const std::ffi::c_long;
                let usable = Rect::new(
                    *p as i32,
                    *p.add(1) as i32,
                    *p.add(2) as i32,
                    *p.add(3) as i32,
                );
                rect = rect.intersection(&usable).unwrap_or_default();
                result = Ok(rect);
            }

            if !propdata.is_null() {
                (x.XFree)(propdata.cast());
            }
        }

        // (upstream fills in the bounds even when it fails)
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_rates() {
        let mut info = XRRModeInfo {
            id: 1,
            width: 1920,
            height: 1080,
            dotClock: 148_500_000,
            hSyncStart: 0,
            hSyncEnd: 0,
            hTotal: 2200,
            hSkew: 0,
            vSyncStart: 0,
            vSyncEnd: 0,
            vTotal: 1125,
            name: std::ptr::null_mut(),
            nameLength: 0,
            modeFlags: 0,
        };
        assert_eq!(
            calculate_xrandr_refresh_rate(&info),
            (148_500_000, 2200 * 1125)
        );
        info.modeFlags = RR_Interlace;
        assert_eq!(
            calculate_xrandr_refresh_rate(&info),
            (148_500_000, 2200 * 562)
        );
        info.modeFlags = RR_DoubleScan;
        assert_eq!(
            calculate_xrandr_refresh_rate(&info),
            (148_500_000, 2200 * 2250)
        );
        info.hTotal = 0;
        assert_eq!(calculate_xrandr_refresh_rate(&info), (0, 0));
    }

    #[test]
    fn display_names() {
        let mut name = "HDMI-1".to_owned();
        set_xrandr_display_name(Option::None, &mut name, DISPLAY_NAME_SIZE, 527, 296);
        assert_eq!(name, "HDMI-1 24\"");
        let mut name = String::new();
        set_xrandr_display_name(Option::None, &mut name, DISPLAY_NAME_SIZE, 527, 296);
        assert_eq!(name, "");
        let mut name = "VGA-1".to_owned();
        set_xrandr_display_name(Option::None, &mut name, DISPLAY_NAME_SIZE, 0, 0);
        assert_eq!(name, "VGA-1");
    }

    #[test]
    fn mode_data_lookup() {
        let mut data = X11DisplayData::new();
        let mode = DisplayMode {
            w: 1024,
            h: 768,
            format: PixelFormat::XRGB8888,
            refresh_rate_numerator: 60,
            refresh_rate_denominator: 1,
            ..Default::default()
        };
        data.set_mode_data(&mode, 42);
        let mut finalized = mode;
        finalized.display_id = 7;
        finalize_display_mode(&mut finalized);
        assert_eq!(data.mode_data(&finalized), Some(42));
        data.set_mode_data(&finalized, 43);
        assert_eq!(data.modedata.len(), 1);
        assert_eq!(data.mode_data(&mode), Some(43));
    }
}
