// Rust translation of src/video/emscripten/SDL_emscriptenvideo.c and
// SDL_emscriptenvideo.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Emscripten video driver: a window is a `<canvas>` of the page (by
//! default `#canvas`, or the element the [`hints::EMSCRIPTEN_CANVAS_SELECTOR`]
//! hint or the [`PROP_WINDOW_CREATE_EMSCRIPTEN_CANVAS_ID_STRING`] property
//! names), the one display is the screen, fullscreen is the browser's
//! Fullscreen API, and the framebuffer is drawn with the canvas's 2D
//! context ([`framebuffer`]). The mouse ([`mouse`]) has CSS cursors and
//! pointer lock.
//!
//! The JavaScript is upstream's, in the `sdl3-emscripten` crate
//! (`sdl3_emscripten::video`). JavaScript calls two Rust functions by
//! name, through the callback registry: `_Emscripten_SendSystemThemeChangedEvent()`
//! (the `prefers-color-scheme` listener) and `_requestFullscreenThroughSDL()`
//! (emscripten's own `Module.requestFullscreen()`), registered when the
//! driver is created.
//!
//! Not translated yet: the DOM event handlers of `SDL_emscriptenevents.c`
//! but for the ones the driver needs ([`events`]: the window's `resize`,
//! `fullscreenchange`, `pointerlockchange` and the fullscreen canvas
//! resize), and the WebGL contexts of `SDL_emscriptenopengles.c` (no
//! `GL_*` entry points: OpenGL windows fail to create, as on a driver
//! without OpenGL).
//!
//! Note (upstream): the driver is unavailable without a DOM (under node):
//! upstream's creates its device anyway, and its first JavaScript (the
//! system theme's `window.matchMedia`) throws a ReferenceError, which ends
//! the program; here video initialization goes on to the next driver (none
//! under node, but the dummy and offscreen drivers when asked for), and a
//! message box fails.

mod events;
mod framebuffer;
mod mouse;

use std::ffi::{c_char, c_int, c_void, CString};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, Once};

use sdl3_emscripten::sys;
use sdl3_emscripten::video as js;

use crate::error::{Error, Result};
use crate::events::mouse::{
    self as sdl_mouse, Cursor, MouseButtonFlags, MouseFeature, SystemCursor,
};
use crate::events::window::{send_window_event, WindowFlags};
use crate::events::{keyboard, DisplayID, EventType, WindowID};
use crate::hints;
use crate::io::IoStream;
use crate::properties::Properties;
use crate::stdlib::math::{floor, lroundf};
use crate::video::core::{set_system_theme, with_device, with_window};
use crate::video::display::add_basic_video_display;
use crate::video::messagebox::{MessageBoxButtonFlags, MessageBoxColorType, MessageBoxData};
use crate::video::sysvideo::{
    DisplayMode, FullscreenOp, FullscreenResult, SystemTheme, VideoBootStrap, VideoDriver,
};
use crate::video::window::{
    WindowOp, PROP_WINDOW_CREATE_EMSCRIPTEN_CANVAS_ID_STRING,
    PROP_WINDOW_CREATE_EMSCRIPTEN_KEYBOARD_ELEMENT_STRING, PROP_WINDOW_EMSCRIPTEN_CANVAS_ID_STRING,
    PROP_WINDOW_EMSCRIPTEN_KEYBOARD_ELEMENT_STRING,
};
use crate::video::{PixelFormat, Rect, Surface};

/// Translation of `EMSCRIPTENVID_DRIVER_NAME`.
const EMSCRIPTENVID_DRIVER_NAME: &str = "emscripten";

/// The window data. Translation of `struct SDL_WindowData` (the window is
/// its ID; the framebuffer surface is the video core's, and the WebGL
/// context comes with the WebGL driver).
///
/// Some fields are only read by the event handlers that come with the
/// Emscripten port's event handling (PR 8): `window`, `keyboard_element`,
/// `has_pointer_lock` (which the pointer lock handler here keeps) and
/// `mouse_focus_loss_pending`. Upstream writes `fullscreen_mode_flags` and
/// never reads it.
#[allow(dead_code)]
pub(crate) struct EmscriptenWindow {
    window: WindowID,

    non_fill_document_width: i32,
    non_fill_document_height: i32,

    /// The canvas's CSS selector.
    canvas_id: CString,
    keyboard_element: CString,

    pixel_ratio: f32,

    external_size: bool,

    fullscreen_mode_flags: WindowFlags,
    fullscreen_resize: bool,
    fullscreen_change_in_progress: bool,

    has_pointer_lock: bool,

    mouse_focus_loss_pending: bool,
}

/// Run `f` on a window's [`EmscriptenWindow`] (`window->internal`);
/// `None` without one.
fn with_data<R>(window: WindowID, f: impl FnOnce(&mut EmscriptenWindow) -> R) -> Option<R> {
    with_window(window, |w| {
        w.internal
            .as_mut()
            .and_then(|i| i.downcast_mut::<EmscriptenWindow>())
            .map(f)
    })
    .ok()
    .flatten()
}

/// The canvas selector of a window (`window->internal->canvas_id`).
fn canvas_id(window: WindowID) -> Option<CString> {
    with_data(window, |d| d.canvas_id.clone())
}

/// A string for C (the JavaScript reads it up to its NUL): cut at the first
/// NUL, as C's would be.
fn c_string(s: &str) -> CString {
    let s = s.split('\0').next().unwrap_or_default();
    CString::new(s).unwrap_or_default()
}

/// Translation of `Emscripten_fill_document_window`.
static FILL_DOCUMENT_WINDOW: Mutex<Option<WindowID>> = Mutex::new(None);

/// The window that fills the document, if any.
pub(crate) fn fill_document_window() -> Option<WindowID> {
    *FILL_DOCUMENT_WINDOW
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn set_fill_document_window(window: Option<WindowID>) {
    *FILL_DOCUMENT_WINDOW
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = window;
}

/// Translation of `pumpevents_has_run`.
static PUMPEVENTS_HAS_RUN: AtomicBool = AtomicBool::new(false);
/// Translation of `pending_swap_interval`.
static PENDING_SWAP_INTERVAL: AtomicI32 = AtomicI32::new(-1);

// * * * The system theme

/// Translation of `Emscripten_GetSystemTheme()`.
fn get_system_theme() -> SystemTheme {
    /* Technically, light theme can mean explicit light theme or no preference.
    https://developer.mozilla.org/en-US/docs/Web/CSS/@media/prefers-color-scheme#syntax */

    // SAFETY: plain JavaScript.
    let theme_code = unsafe { js::sdl3_get_system_theme() };

    match theme_code {
        0 => SystemTheme::Light,
        1 => SystemTheme::Dark,
        _ => SystemTheme::Unknown,
    }
}

/// Translation of `Emscripten_ListenSystemTheme()`.
fn listen_system_theme() {
    // SAFETY: plain JavaScript; the function it calls is registered.
    unsafe { js::sdl3_listen_system_theme() };
}

/// Translation of `Emscripten_UnlistenSystemTheme()`.
fn unlisten_system_theme() {
    // SAFETY: plain JavaScript.
    unsafe { js::sdl3_unlisten_system_theme() };
}

/// `_Emscripten_SendSystemThemeChangedEvent()`, which the theme listener
/// calls. Translation of `Emscripten_SendSystemThemeChangedEvent()`.
extern "C" fn send_system_theme_changed_event() {
    set_system_theme(get_system_theme());
}

/// `_requestFullscreenThroughSDL(window)`, which `Module.requestFullscreen()`
/// calls. Translation of `requestFullscreenThroughSDL()` (with the
/// window's ID for its `SDL_Window *`).
extern "C" fn request_fullscreen_through_sdl(window: WindowID) {
    let _ = crate::video::window::Window::from_raw(window).set_fullscreen(true);
}

/// Makes the functions upstream marks `EMSCRIPTEN_KEEPALIVE` in this file
/// callable from JavaScript (see `sdl3_emscripten::Callback`).
fn register_callbacks() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: extern "C" functions with the parameters the JavaScript
        // passes; they live as long as the program.
        unsafe {
            js::EMSCRIPTEN_SEND_SYSTEM_THEME_CHANGED_EVENT
                .register(send_system_theme_changed_event as *const ());
            js::REQUEST_FULLSCREEN_THROUGH_SDL
                .register(request_fullscreen_through_sdl as *const ());
        }
    });
}

/// Whether the program runs in a page with a DOM.
fn has_dom() -> bool {
    // SAFETY: plain JavaScript.
    unsafe { js::sdl3_video_has_dom() != 0 }
}

// * * * The device

struct EmscriptenVideo;

/// Translation of `Emscripten_CreateDevice()`.
fn create_device() -> Option<Arc<dyn VideoDriver>> {
    // Note (upstream): no device without a DOM (see the module
    // documentation).
    if !has_dom() {
        return None;
    }

    /* Firefox sends blur event which would otherwise prevent full screen
     * when the user clicks to allow full screen.
     * See https://bugzilla.mozilla.org/show_bug.cgi?id=1144964
     */
    let _ = hints::set(hints::VIDEO_MINIMIZE_ON_FOCUS_LOSS, "0");

    register_callbacks();
    listen_system_theme();
    // (device->system_theme is set in video_init(), once the device exists)

    Some(Arc::new(EmscriptenVideo))
}

/// Translation of `Emscripten_ShowMessagebox()`, which also serves without
/// video.
fn show_message_box(messageboxdata: &MessageBoxData) -> Result<i32> {
    // Note (upstream): without a DOM, an error (upstream's alert() throws a
    // ReferenceError there, which ends the program).
    if !has_dom() {
        return Err(Error::new(
            "Emscripten message boxes need a page (no DOM: not in a browser)",
        ));
    }

    // SAFETY: a plain call.
    let asyncify = unsafe { sys::emscripten_has_asyncify() } != 0;
    if asyncify && hints::get_bool(hints::EMSCRIPTEN_ASYNCIFY, true) {
        let rgb = |kind: MessageBoxColorType| {
            messageboxdata
                .color_scheme
                .map_or_else(String::new, |scheme| {
                    let color = scheme.colors[kind as usize];
                    format!("rgb({}, {}, {})", color.r, color.g, color.b)
                })
        };
        let dialog_background = c_string(&rgb(MessageBoxColorType::Background));
        let dialog_color = c_string(&rgb(MessageBoxColorType::Text));
        let button_border = c_string(&rgb(MessageBoxColorType::ButtonBorder));
        let button_background = c_string(&rgb(MessageBoxColorType::ButtonBackground));
        let button_hovered = c_string(&rgb(MessageBoxColorType::ButtonSelected));

        // TODO: Handle parent window when multiple windows can be added in Emscripten builds
        let dialog_id = c_string(&format!(
            "SDL3_messagebox_{}",
            crate::stdlib::random::next_u32()
        ));
        let title = c_string(&messageboxdata.title);
        let message = c_string(&messageboxdata.message);
        // SAFETY: NUL-terminated strings, for the call.
        unsafe {
            js::sdl3_messagebox_show(
                title.as_ptr(),
                message.as_ptr(),
                dialog_background.as_ptr(),
                dialog_color.as_ptr(),
                dialog_id.as_ptr(),
            )
        };

        for button in &messageboxdata.buttons {
            let text = c_string(&button.text);
            // SAFETY: NUL-terminated strings, for the call.
            let created = unsafe {
                js::sdl3_messagebox_add_button(
                    dialog_id.as_ptr(),
                    text.as_ptr(),
                    button.button_id,
                    (button.flags.0 & MessageBoxButtonFlags::RETURNKEY_DEFAULT.0) as i32,
                    (button.flags.0 & MessageBoxButtonFlags::ESCAPEKEY_DEFAULT.0) as i32,
                    button_border.as_ptr(),
                    button_background.as_ptr(),
                    button_hovered.as_ptr(),
                )
            };

            if created == 0 {
                // (upstream returns false without an error message)
                return Err(Error::new("The message box's dialog went away"));
            }
        }

        loop {
            // give back control to browser for screen refresh
            // SAFETY: with Asyncify, a plain call.
            unsafe { sys::emscripten_sleep(0) };

            // SAFETY: a NUL-terminated string.
            let dialog_open = unsafe { js::sdl3_messagebox_is_open(dialog_id.as_ptr()) };

            if dialog_open != 0 {
                continue;
            }

            // SAFETY: a NUL-terminated string.
            return Ok(unsafe { js::sdl3_messagebox_button(dialog_id.as_ptr()) });
        }
    }

    // Cannot add elements to DOM and block without Asyncify. So, fall back to the alert function.
    let title = c_string(&messageboxdata.title);
    let message = c_string(&messageboxdata.message);
    // SAFETY: NUL-terminated strings, for the call.
    unsafe { js::sdl3_messagebox_alert(title.as_ptr(), message.as_ptr()) };
    // (no button: upstream leaves the caller's buttonID as it was)
    Ok(-1)
}

/// Translation of `Emscripten_bootstrap`.
pub(crate) static EMSCRIPTEN_BOOTSTRAP: VideoBootStrap = VideoBootStrap {
    name: EMSCRIPTENVID_DRIVER_NAME,
    desc: "SDL emscripten video driver",
    create: create_device,
    show_message_box: Some(show_message_box),
    is_preferred: false,
};

/// Translation of `Emscripten_VideoInit()`.
fn video_init() -> Result<()> {
    // (Emscripten_CreateDevice() sets device->system_theme, which exists
    // once the driver is chosen; no event is sent)
    let theme = get_system_theme();
    let _ = with_device(|v| v.system_theme = theme);

    // Use a fake 32-bpp desktop mode
    let (mut w, mut h): (c_int, c_int) = (0, 0);
    // SAFETY: two ints to write.
    let pixel_density = unsafe {
        sys::emscripten_get_screen_size(&mut w, &mut h);
        sys::emscripten_get_device_pixel_ratio()
    };
    let mode = DisplayMode {
        format: PixelFormat::RGBA32,
        w,
        h,
        pixel_density: pixel_density as f32,
        ..Default::default()
    };

    if add_basic_video_display(Some(&mode)) == 0 {
        return Err(crate::video::core::uninitialized_video());
    }

    mouse::init_mouse();

    // Assume we have a mouse and keyboard
    keyboard::add_keyboard(keyboard::DEFAULT_KEYBOARD_ID, None);
    sdl_mouse::add_mouse(sdl_mouse::DEFAULT_MOUSE_ID, None);

    events::register_global_event_handlers();

    // We're done!
    Ok(())
}

/// Translation of `Emscripten_VideoQuit()`.
fn video_quit() {
    events::unregister_global_event_handlers();
    mouse::quit_mouse();
    unlisten_system_theme();
    PUMPEVENTS_HAS_RUN.store(false, Ordering::Relaxed);
    PENDING_SWAP_INTERVAL.store(-1, Ordering::Relaxed);
}

/// Translation of `Emscripten_GetDisplayUsableBounds()`.
fn display_usable_bounds() -> Rect {
    // SAFETY: plain JavaScript.
    unsafe {
        Rect::new(
            0,
            0,
            js::sdl3_window_inner_width(),
            js::sdl3_window_inner_height(),
        )
    }
}

/// Whether the WebGL driver may set the swap interval now; if not, it's
/// kept for the first event pump. Translation of
/// `Emscripten_ShouldSetSwapInterval()`.
#[allow(dead_code)] // (the WebGL driver's, PR 9 of the Emscripten port)
pub(crate) fn should_set_swap_interval(interval: i32) -> bool {
    if !PUMPEVENTS_HAS_RUN.load(Ordering::Relaxed) {
        PENDING_SWAP_INTERVAL.store(interval, Ordering::Relaxed);
        return false;
    }
    true
}

/// Translation of `Emscripten_GetPendingSwapInterval()`.
#[allow(dead_code)] // (the WebGL driver's, PR 9 of the Emscripten port)
pub(crate) fn pending_swap_interval() -> i32 {
    PENDING_SWAP_INTERVAL.load(Ordering::Relaxed)
}

/// `Emscripten_GLES_SetSwapInterval()`: a stub until the WebGL driver
/// (`SDL_emscriptenopengles.c`, PR 9 of the Emscripten port), when nothing
/// sets a pending interval.
fn gles_set_swap_interval(_interval: i32) {}

/// Translation of `Emscripten_PumpEvents()`.
fn pump_events() {
    if !PUMPEVENTS_HAS_RUN.load(Ordering::Relaxed) {
        // we assume you've set a mainloop by the time you've called pumpevents, so we delay initial SetInterval changes until then.
        // otherwise you'll get a warning on the javascript console.
        PUMPEVENTS_HAS_RUN.store(true, Ordering::Relaxed);
        let pending = PENDING_SWAP_INTERVAL.load(Ordering::Relaxed);
        if pending >= 0 {
            gles_set_swap_interval(pending);
            PENDING_SWAP_INTERVAL.store(-1, Ordering::Relaxed);
        }
    }
}

// * * * Windows

/// `emscripten_set_canvas_element_size()`.
fn set_canvas_element_size(canvas_id: &CString, w: i64, h: i64) {
    // SAFETY: a NUL-terminated selector.
    unsafe { sys::emscripten_set_canvas_element_size(canvas_id.as_ptr(), w as c_int, h as c_int) };
}

/// `emscripten_set_element_css_size()`.
fn set_element_css_size(canvas_id: &CString, w: f64, h: f64) {
    // SAFETY: a NUL-terminated selector.
    unsafe { sys::emscripten_set_element_css_size(canvas_id.as_ptr(), w, h) };
}

/// `emscripten_get_element_css_size()` (zeros if there's no such element).
fn element_css_size(canvas_id: &CString) -> (f64, f64) {
    let (mut w, mut h) = (0.0, 0.0);
    // SAFETY: a NUL-terminated selector and two doubles to write.
    unsafe { sys::emscripten_get_element_css_size(canvas_id.as_ptr(), &mut w, &mut h) };
    (w, h)
}

/// Translation of `Emscripten_SetWindowFillDocument()`.
fn set_window_fill_document(window: WindowID, fill: bool) -> Result<()> {
    let current = fill_document_window();
    if fill && current.is_some() && current != Some(window) {
        return Err(Error::new(
            "Only one fill-document window allowed at a time.",
        ));
    }

    let Some((canvas_id, pixel_ratio, external_size)) = with_data(window, |d| {
        (d.canvas_id.clone(), d.pixel_ratio, d.external_size)
    }) else {
        return Err(Error::new("Invalid window"));
    };

    // fill_document takes up the entire page and resizes as the browser window resizes.
    if fill {
        set_fill_document_window(Some(window));

        // SAFETY: plain JavaScript.
        let (w, h) = unsafe {
            (
                js::sdl3_window_inner_width(),
                js::sdl3_window_inner_height(),
            )
        };
        // (int times float: float arithmetic, as C's)
        let scaled_w = f64::from(w as f32 * pixel_ratio);
        let scaled_h = f64::from(h as f32 * pixel_ratio);

        let (window_w, window_h) = with_window(window, |w| (w.core.w, w.core.h))?;
        with_data(window, |d| {
            d.non_fill_document_width = window_w;
            d.non_fill_document_height = window_h;
        });

        // SAFETY: a NUL-terminated selector.
        unsafe { js::sdl3_fill_document(canvas_id.as_ptr()) };

        set_canvas_element_size(
            &canvas_id,
            lroundf(scaled_w as f32),
            lroundf(scaled_h as f32),
        );

        // set_canvas_size unsets this
        if pixel_ratio != 1.0 {
            set_element_css_size(&canvas_id, f64::from(w), f64::from(h));
        }

        // force the event to trigger, so pixel ratio changes can be handled
        with_window(window, |w| {
            w.core.w = 0;
            w.core.h = 0;
        })?;
        send_window_event(window, EventType::WINDOW_RESIZED, w, h);
    } else {
        let transitioning = current == Some(window);
        if transitioning {
            // SAFETY: plain JavaScript.
            unsafe { js::sdl3_unfill_document() };
            set_fill_document_window(None);
        }

        let Some((non_fill_w, non_fill_h)) = with_data(window, |d| {
            (d.non_fill_document_width, d.non_fill_document_height)
        }) else {
            return Err(Error::new("Invalid window"));
        };
        with_window(window, |w| {
            w.core.w = non_fill_w;
            w.core.h = non_fill_h;
        })?;
        let scaled_w = floor(f64::from(non_fill_w as f32 * pixel_ratio));
        let scaled_h = floor(f64::from(non_fill_h as f32 * pixel_ratio));
        set_canvas_element_size(
            &canvas_id,
            lroundf(scaled_w as f32),
            lroundf(scaled_h as f32),
        );

        // if the size is not being controlled by css, we need to scale down for hidpi
        if !external_size && pixel_ratio != 1.0 {
            // scale canvas down
            set_element_css_size(&canvas_id, f64::from(non_fill_w), f64::from(non_fill_h));
        }

        if transitioning {
            with_window(window, |w| {
                w.core.w = 0;
                w.core.h = 0;
            })?;
            send_window_event(window, EventType::WINDOW_RESIZED, non_fill_w, non_fill_h);
        }
    }

    Ok(())
}

/// Translation of `Emscripten_CreateWindow()`.
fn create_window(window: WindowID, props: &Properties) -> Result<()> {
    let flags = with_window(window, |w| w.flags())?;
    let mut fill_document = flags.contains(WindowFlags::FILL_DOCUMENT);
    if fill_document && fill_document_window().is_some() {
        fill_document = false; // only one allowed at a time.
        with_window(window, |w| w.core.flags &= !WindowFlags::FILL_DOCUMENT)?; // !!! FIXME: should this fail instead?
    }

    // Allocate window internal data
    let canvas_id = hints::get(hints::EMSCRIPTEN_CANVAS_SELECTOR)
        .filter(|s| !s.is_empty())
        .or_else(|| props.get_string(PROP_WINDOW_CREATE_EMSCRIPTEN_CANVAS_ID_STRING))
        .unwrap_or_else(|| "#canvas".to_owned());

    let keyboard_element = hints::get(hints::EMSCRIPTEN_KEYBOARD_ELEMENT)
        .filter(|s| !s.is_empty())
        .or_else(|| props.get_string(PROP_WINDOW_CREATE_EMSCRIPTEN_KEYBOARD_ELEMENT_STRING))
        .unwrap_or_else(|| "#window".to_owned());

    let pixel_ratio = if flags.contains(WindowFlags::HIGH_PIXEL_DENSITY) {
        // SAFETY: a plain call.
        unsafe { sys::emscripten_get_device_pixel_ratio() as f32 }
    } else {
        1.0
    };

    // set a fake size to check if there is any CSS sizing the canvas
    let canvas = c_string(&canvas_id);
    set_canvas_element_size(&canvas, 1, 1);
    let (css_w, css_h) = element_css_size(&canvas);

    let external_size = floor(css_w) != 1.0 || floor(css_h) != 1.0;
    let (w, h) = with_window(window, |w| {
        if external_size {
            w.core.w = css_w as i32;
            w.core.h = css_h as i32;
        }
        (w.core.w, w.core.h)
    })?;
    if external_size {
        fill_document = false; // can't be resizable if something else is controlling it.
    }

    // Setup driver data for this window
    let data = EmscriptenWindow {
        window,
        non_fill_document_width: w,
        non_fill_document_height: h,
        canvas_id: canvas,
        keyboard_element: c_string(&keyboard_element),
        pixel_ratio,
        external_size,
        fullscreen_mode_flags: WindowFlags::NONE,
        fullscreen_resize: false,
        fullscreen_change_in_progress: false,
        has_pointer_lock: false,
        mouse_focus_loss_pending: false,
    };
    with_window(window, |w| w.internal = Some(Box::new(data)))?;

    let _ = set_window_fill_document(window, fill_document);

    // One window, it always has focus
    sdl_mouse::set_mouse_focus(Some(window));
    let _ = keyboard::set_keyboard_focus(Some(window));

    events::register_event_handlers(window);

    // Make the emscripten "fullscreen" button go through SDL.
    // SAFETY: plain JavaScript; the function it calls is registered.
    unsafe { js::sdl3_set_request_fullscreen(window) };

    // Ensure various things are added to the window's properties
    let window_props = with_window(window, |w| w.properties())?;
    window_props.set(PROP_WINDOW_EMSCRIPTEN_CANVAS_ID_STRING, canvas_id)?;
    window_props.set(
        PROP_WINDOW_EMSCRIPTEN_KEYBOARD_ELEMENT_STRING,
        keyboard_element,
    )?;

    // Window has been successfully created
    Ok(())
}

/// Translation of `Emscripten_SetWindowSize()`.
fn set_window_size(window: WindowID) {
    let Some((canvas_id, external_size)) =
        with_data(window, |d| (d.canvas_id.clone(), d.external_size))
    else {
        return;
    };
    if fill_document_window() == Some(window) {
        return; // canvas size is being dictated by the browser window size, refuse request.
    }

    let Ok((flags, pending)) = with_window(window, |w| (w.flags(), w.pending)) else {
        return;
    };

    // update pixel ratio
    if flags.contains(WindowFlags::HIGH_PIXEL_DENSITY) {
        // SAFETY: a plain call.
        let ratio = unsafe { sys::emscripten_get_device_pixel_ratio() } as f32;
        with_data(window, |d| d.pixel_ratio = ratio);
    }
    let pixel_ratio = with_data(window, |d| d.pixel_ratio).unwrap_or(1.0);
    set_canvas_element_size(
        &canvas_id,
        lroundf(pending.w as f32 * pixel_ratio),
        lroundf(pending.h as f32 * pixel_ratio),
    );

    // scale canvas down
    if !external_size && pixel_ratio != 1.0 {
        set_element_css_size(&canvas_id, f64::from(pending.w), f64::from(pending.h));
    }

    send_window_event(window, EventType::WINDOW_RESIZED, pending.w, pending.h);
}

/// Translation of `Emscripten_GetWindowSizeInPixels()`.
fn window_size_in_pixels(window: WindowID) -> Option<(i32, i32)> {
    let pixel_ratio = with_data(window, |d| d.pixel_ratio)?;
    let (w, h) = with_window(window, |w| (w.core.w, w.core.h)).ok()?;
    Some((
        lroundf(w as f32 * pixel_ratio) as i32,
        lroundf(h as f32 * pixel_ratio) as i32,
    ))
}

/// Translation of `Emscripten_DestroyWindow()`.
fn destroy_window(window: WindowID) {
    let _ = set_window_fill_document(window, false);

    let data = with_window(window, |w| w.internal.take())
        .ok()
        .flatten()
        .and_then(|i| i.downcast::<EmscriptenWindow>().ok());
    if let Some(data) = data {
        events::unregister_event_handlers(&data);

        // We can't destroy the canvas, so resize it to zero instead
        set_canvas_element_size(&data.canvas_id, 0, 0);
        if data.pixel_ratio != 1.0 {
            set_element_css_size(&data.canvas_id, 1.0, 1.0);
        }
        // (the strings and the data go with `data`)
    }

    // SAFETY: plain JavaScript.
    unsafe { js::sdl3_unset_request_fullscreen() };
}

/// Translation of `Emscripten_SetWindowFullscreen()`.
fn set_window_fullscreen(window: WindowID, fullscreen: FullscreenOp) -> FullscreenResult {
    let Some((canvas_id, in_progress)) = with_data(window, |d| {
        (d.canvas_id.clone(), d.fullscreen_change_in_progress)
    }) else {
        return FullscreenResult::Failed;
    };

    if in_progress {
        return FullscreenResult::Failed;
    }

    let mut fsevent = std::mem::MaybeUninit::<sys::EmscriptenFullscreenChangeEvent>::zeroed();
    // SAFETY: emscripten fills in the structure (all of it on success).
    if unsafe { sys::emscripten_get_fullscreen_status(fsevent.as_mut_ptr()) }
        == sys::EMSCRIPTEN_RESULT_SUCCESS
    {
        // SAFETY: filled in (and zeroed before: plain data).
        let fsevent = unsafe { fsevent.assume_init() };
        if (fullscreen == FullscreenOp::Enter) == fsevent.isFullscreen {
            return FullscreenResult::Succeeded; // already there.
        }
    }

    let res = if fullscreen != FullscreenOp::Leave {
        let Ok((fullscreen_exclusive, flags)) =
            with_window(window, |w| (w.fullscreen_exclusive, w.flags()))
        else {
            return FullscreenResult::Failed;
        };
        let is_fullscreen_desktop = !fullscreen_exclusive;

        let strategy = sys::EmscriptenFullscreenStrategy {
            scaleMode: if is_fullscreen_desktop {
                sys::EMSCRIPTEN_FULLSCREEN_SCALE_STRETCH
            } else {
                sys::EMSCRIPTEN_FULLSCREEN_SCALE_ASPECT
            },
            canvasResolutionScaleMode: if !is_fullscreen_desktop {
                sys::EMSCRIPTEN_FULLSCREEN_CANVAS_SCALE_NONE
            } else if flags.contains(WindowFlags::HIGH_PIXEL_DENSITY) {
                sys::EMSCRIPTEN_FULLSCREEN_CANVAS_SCALE_HIDEF
            } else {
                sys::EMSCRIPTEN_FULLSCREEN_CANVAS_SCALE_STDDEF
            },
            filteringMode: sys::EMSCRIPTEN_FULLSCREEN_FILTERING_DEFAULT,
            canvasResizedCallback: Some(events::handle_canvas_resize),
            // (the window's ID for its SDL_WindowData *)
            canvasResizedCallbackUserData: window as usize as *mut c_void,
            canvasResizedCallbackTargetThread: 0,
        };

        with_data(window, |d| {
            d.fullscreen_mode_flags = flags & WindowFlags::FULLSCREEN;
            d.fullscreen_resize = is_fullscreen_desktop;
        });

        // SAFETY: a NUL-terminated selector, and the strategy, which
        // emscripten copies.
        unsafe { sys::emscripten_request_fullscreen_strategy(canvas_id.as_ptr(), true, &strategy) }
    } else {
        // SAFETY: a plain call.
        unsafe { sys::emscripten_exit_fullscreen() }
    };

    if res == sys::EMSCRIPTEN_RESULT_SUCCESS {
        with_data(window, |d| d.fullscreen_change_in_progress = true); // even on success, this might animate to the new state.
        FullscreenResult::Succeeded
    } else if res == sys::EMSCRIPTEN_RESULT_DEFERRED {
        with_data(window, |d| d.fullscreen_change_in_progress = true);
        FullscreenResult::Pending
    } else {
        FullscreenResult::Failed
    }
}

/// Translation of `Emscripten_SetWindowTitle()`.
fn set_window_title(window: WindowID) {
    let title = with_window(window, |w| w.title.clone()).ok().flatten();
    let title = c_string(title.as_deref().unwrap_or_default());
    // SAFETY: a NUL-terminated string.
    unsafe { sys::emscripten_set_window_title(title.as_ptr()) };
}

/// Translation of `Emscripten_SetWindowIcon()`.
fn set_window_icon(icon: &Surface<'static>) -> Result<()> {
    // Create dynamic memory stream for PNG encoding
    let mut stream = IoStream::from_dynamic_mem();

    // Encode icon to PNG using SDL's native PNG encoder
    // (save_png_io() borrows the surface mutably: a copy of the icon)
    icon.duplicate()?.save_png_io(&mut stream)?;

    // Get the PNG data from the stream
    let png_data = stream.dynamic_memory().unwrap_or_default();

    // Pass PNG data to JavaScript
    // SAFETY: the bytes, which the JavaScript copies (into a Blob).
    unsafe { js::sdl3_set_window_icon(png_data.as_ptr(), png_data.len()) };

    Ok(())
}

impl VideoDriver for EmscriptenVideo {
    fn video_init(&self) -> Result<()> {
        video_init()
    }

    fn video_quit(&self) {
        video_quit()
    }

    fn display_usable_bounds(&self, _display: DisplayID) -> Option<Result<Rect>> {
        Some(Ok(display_usable_bounds()))
    }

    fn set_display_mode(&self, _display: DisplayID, _mode: &DisplayMode) -> Option<Result<()>> {
        // can't do this
        Some(Ok(()))
    }

    fn pump_events(&self) -> Option<()> {
        pump_events();
        Some(())
    }

    fn create_window(&self, window: WindowID, create_props: &Properties) -> Option<Result<()>> {
        Some(create_window(window, create_props))
    }

    fn set_window_title(&self, window: WindowID) -> Option<()> {
        set_window_title(window);
        Some(())
    }

    fn set_window_icon(&self, _window: WindowID, icon: &Surface<'static>) -> Option<Result<()>> {
        Some(set_window_icon(icon))
    }

    fn set_window_size(&self, window: WindowID) -> Option<()> {
        set_window_size(window);
        Some(())
    }

    fn set_window_resizable(&self, _window: WindowID, _resizable: bool) -> Option<()> {
        // this function just has to exist or the higher level won't let the window change its SDL_WINDOW_RESIZABLE flag.
        Some(())
    }

    fn window_size_in_pixels(&self, window: WindowID) -> Option<(i32, i32)> {
        window_size_in_pixels(window)
    }

    fn destroy_window(&self, window: WindowID) -> Option<()> {
        destroy_window(window);
        Some(())
    }

    fn set_window_fullscreen(
        &self,
        window: WindowID,
        _display: DisplayID,
        fullscreen: FullscreenOp,
    ) -> Option<FullscreenResult> {
        Some(set_window_fullscreen(window, fullscreen))
    }

    fn set_window_fill_document(&self, window: WindowID, fill: bool) -> Option<Result<()>> {
        Some(set_window_fill_document(window, fill))
    }

    fn implements_window_op(&self, op: WindowOp) -> bool {
        matches!(op, WindowOp::SetResizable | WindowOp::SetFullscreen)
    }

    fn implements_fill_document(&self) -> bool {
        true
    }

    // The framebuffer (SDL_emscriptenframebuffer.c)

    fn implements_window_framebuffer(&self) -> bool {
        true
    }

    fn create_window_framebuffer(
        &self,
        window: WindowID,
        w: i32,
        h: i32,
    ) -> Option<Result<Surface<'static>>> {
        Some(framebuffer::create_window_framebuffer(window, w, h))
    }

    fn update_window_framebuffer(
        &self,
        window: WindowID,
        surface: &Surface<'static>,
        rects: &[Rect],
    ) -> Option<Result<()>> {
        Some(framebuffer::update_window_framebuffer(
            window, surface, rects,
        ))
    }

    fn destroy_window_framebuffer(&self, window: WindowID) -> Option<()> {
        framebuffer::destroy_window_framebuffer(window);
        Some(())
    }

    // The mouse (SDL_emscriptenmouse.c)

    fn create_cursor(
        &self,
        surface: &Surface<'_>,
        hot_x: i32,
        hot_y: i32,
    ) -> Option<Result<Cursor>> {
        Some(mouse::create_cursor(surface, hot_x, hot_y))
    }

    fn create_system_cursor(&self, id: SystemCursor) -> Option<Result<Cursor>> {
        Some(Ok(mouse::create_system_cursor(id)))
    }

    fn show_cursor(&self, cursor: Option<&Cursor>) -> Option<Result<()>> {
        mouse::show_cursor(cursor);
        Some(Ok(()))
    }

    fn set_relative_mouse_mode(&self, enabled: bool) -> Option<Result<()>> {
        Some(mouse::set_relative_mouse_mode(enabled))
    }

    fn global_mouse_state(&self) -> Option<(f32, f32, MouseButtonFlags)> {
        Some(mouse::global_mouse_state())
    }

    fn implements_mouse_feature(&self, feature: MouseFeature) -> bool {
        matches!(
            feature,
            MouseFeature::SetRelativeMouseMode | MouseFeature::GetGlobalMouseState
        )
    }

    // WebGL (SDL_emscriptenopengles.c: the GL_* entry points) is PR 9 of
    // the Emscripten port.
}

/// The C string the JavaScript gave, as Rust's, freed with `SDL_free()`
/// (`None` for NULL).
fn take_sdl_string(s: *mut c_char) -> Option<String> {
    if s.is_null() {
        return None;
    }
    // SAFETY: a NUL-terminated string from SDL_malloc(), freed once.
    unsafe {
        let string = std::ffi::CStr::from_ptr(s).to_string_lossy().into_owned();
        crate::stdlib::malloc::free(s.cast());
        Some(string)
    }
}

#[cfg(test)]
mod tests;
