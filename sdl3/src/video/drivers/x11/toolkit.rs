// Rust translation of src/video/x11/SDL_x11toolkit.c and SDL_x11toolkit.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A small widget toolkit drawn with core Xlib (used by the message boxes):
//! a window with its own fonts, colors, scaling (from XSETTINGS, with a
//! pixmap scaled down for fractional scales), double buffering (Xdbe) and
//! event loop, and the icon, label and button controls.
//!
//! As a build without FriBidi and libthai: text runs aren't reordered or
//! shaped, and Thai runs are measured and drawn as they are.
//!
//! The controls' C "virtual functions" are a match on [`ControlKind`];
//! controls are referred to by their index in the window's list.

use std::ffi::{c_char, c_int, c_long, c_uchar, c_uint, c_ulong, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::modes::{
    display_driver_data_for_window, with_x11_display, x11_get_global_content_scale,
    x11_get_pixel_format_from_visual_info, x11_get_visual_info_from_visual,
};
use super::settings::{
    SDL_XSETTINGS_GDK_UNSCALED_DPI, SDL_XSETTINGS_GDK_WINDOW_SCALING_FACTOR, SDL_XSETTINGS_XFT_DPI,
};
use super::sys::*;
use super::window::sdl_x11_set_window_title;
use super::x11dyn::{load_symbols, unload_symbols, X11Syms};
use super::xsettings_client::XSettingsClient;
use crate::error::{Error, Result};
use crate::events::WindowID;
use crate::hints;
use crate::video::messagebox::{
    MessageBoxButtonData, MessageBoxButtonFlags, MessageBoxColor, MessageBoxFlags,
    MESSAGEBOX_COLOR_COUNT,
};
use crate::video::surface::ScaleMode;
use crate::video::window::{PROP_WINDOW_X11_DISPLAY_POINTER, PROP_WINDOW_X11_WINDOW_NUMBER};
use crate::video::{Rect, Surface};

const SDL_SET_LOCALE: bool = true;
const SDL_GRAB: bool = true;

pub(crate) const SDL_TOOLKIT_X11_ELEMENT_PADDING: c_int = 4;
pub(crate) const SDL_TOOLKIT_X11_ELEMENT_PADDING_2: c_int = 12;
pub(crate) const SDL_TOOLKIT_X11_ELEMENT_PADDING_3: c_int = 8;
pub(crate) const SDL_TOOLKIT_X11_ELEMENT_PADDING_4: c_int = 16;
#[allow(dead_code)] // (defined but unused upstream)
pub(crate) const SDL_TOOLKIT_X11_ELEMENT_PADDING_5: c_int = 3;

const XK_Tab: KeySym = 0xff09;
const XK_Return: KeySym = 0xff0d;
const XK_Escape: KeySym = 0xff1b;
const XK_Left: KeySym = 0xff51;
const XK_Right: KeySym = 0xff53;
const XK_KP_Enter: KeySym = 0xff8d;

const XdbeUndefined: c_uchar = 0;

// The message box colors
const COLOR_BACKGROUND: usize = 0;
const COLOR_TEXT: usize = 1;
const COLOR_BUTTON_BORDER: usize = 2;
const COLOR_BUTTON_BACKGROUND: usize = 3;
const COLOR_BUTTON_SELECTED: usize = 4;

/// Translation of `SDL_ToolkitWindowModeX11`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // (the message boxes only make dialogs)
pub(crate) enum ToolkitWindowMode {
    Dialog,
    /// For embedding into a normal SDL_Window
    Child,
    Menu,
    Tooltip,
}

/// Translation of `SDL_ToolkitThaiEncodingX11`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // (only read by the libthai paths)
enum ThaiEncoding {
    None,
    /// -0
    Tis,
    /// -2
    TisWin,
    /// -1
    TisMac,
    Iso8859,
    Unicode,
}

/// Translation of `SDL_ToolkitThaiFontX11`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ThaiFont {
    Offset,
    Cell,
}

/// Translation of `SDL_ToolkitControlStateX11`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // (no control is disabled by the message boxes)
pub(crate) enum ControlState {
    Normal,
    Hover,
    /// Key/Button Up
    Pressed,
    /// Key/Button Down
    PressedHeld,
    Disabled,
}

/// Translation of `SDL_ToolkitTextTypeX11`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TextType {
    Generic,
    Thai,
}

/// Translation of `SDL_ToolkitTextElementX11`.
struct TextElement {
    type_: TextType,
    str: Vec<u8>,
    rect: Rect,
    font_h: c_int,
}

/// A button's callback (`cb` with its `cb_data`): given the window and the
/// button's control index.
pub(crate) type ButtonCallback = Box<dyn FnMut(&mut ToolkitWindow, usize)>;

/// The window's `cb_on_scale_change` (with its `cb_data`).
pub(crate) type ScaleChangeCallback = Box<dyn FnMut(&mut ToolkitWindow)>;

/// Translation of `SDL_ToolkitIconControlX11` (without the base).
struct IconControl {
    /* Icon type */
    flags: MessageBoxFlags,
    icon_char: u8,

    /* Font */
    icon_char_font: *mut XFontStruct,
    icon_char_x: c_int,
    icon_char_y: c_int,
    icon_char_a: c_int,
    icon_char_h: c_int,

    /* Colors */
    xcolor_black: XColor,
    xcolor_red: XColor,
    xcolor_red_darker: XColor,
    xcolor_white: XColor,
    xcolor_yellow: XColor,
    xcolor_blue: XColor,
    xcolor_bg_shadow: XColor,
}

/// Translation of `SDL_ToolkitButtonControlX11` (without the base).
struct ButtonControl {
    /* Data */
    data: MessageBoxButtonData,

    /* Text */
    text: Vec<TextElement>,
    text_rect: Rect,

    /* Callback */
    cb: Option<ButtonCallback>,
}

/// Translation of `SDL_ToolkitLabelControlLineX11`.
struct LabelLine {
    text: Vec<TextElement>,
    rect: Rect,
}

/// Translation of `SDL_ToolkitLabelControlX11` (without the base).
struct LabelControl {
    lines: Vec<LabelLine>,
}

/// The kind of a control, with its own data (the structures that embed
/// `SDL_ToolkitControlX11`).
enum ControlKind {
    Icon(IconControl),
    Button(ButtonControl),
    Label(LabelControl),
}

/// Translation of `SDL_ToolkitControlX11`.
pub(crate) struct Control {
    pub(crate) state: ControlState,
    pub(crate) rect: Rect,
    pub(crate) selected: bool,
    pub(crate) dynamic: bool,
    #[allow(dead_code)] // (set but never read upstream)
    pub(crate) is_default_enter: bool,
    pub(crate) is_default_esc: bool,
    pub(crate) do_size: bool,
    kind: ControlKind,
}

/// Translation of `SDL_ToolkitWindowX11`.
pub(crate) struct ToolkitWindow {
    x: Arc<X11Syms>,

    /* Locale */
    origlocale: Option<CString>,

    /* Mode */
    mode: ToolkitWindowMode,

    /* Display */
    display: *mut Display,
    screen: c_int,
    display_close: bool,

    /* Parent */
    parent: Option<WindowID>,
    /// The parent's X window (`parent->internal->xwindow`).
    parent_xwindow: Window,
    /// The toolkit parent's window (`tk_parent->window`).
    tk_parent_window: Option<Window>,

    /* Window */
    window: Window,
    drawable: Drawable,
    image: *mut XImage,
    /// (boxed: Xlib keeps its address)
    shm_info: Box<XShmSegmentInfo>,
    shm_bytes_per_line: c_int,

    /* Visuals and drawing */
    visual: *mut Visual,
    vi: XVisualInfo,
    cmap: Colormap,
    ctx: GC,
    depth: c_int,
    pixmap: bool,

    /* X11 extensions */
    buf: Drawable,
    /// Whether Xdbe is present or not
    xdbe: bool,
    /// Whether Xrandr is present or not
    xrandr: bool,
    shm: bool,
    shm_pixmap: Bool,
    utf8: bool,

    /* Atoms */
    wm_protocols: Atom,
    wm_delete_message: Atom,

    /* Window and pixmap sizes */
    /// Window width.
    window_width: c_int,
    /// Window height.
    window_height: c_int,
    pixmap_width: c_int,
    pixmap_height: c_int,
    window_x: c_int,
    window_y: c_int,

    /* XSettings and scaling */
    xsettings: Option<XSettingsClient>,
    xsettings_first_time: bool,
    pub(crate) iscale: c_int,
    scale: f32,

    /* Font */
    /// for UTF-8 systems
    font_set: XFontSet,
    /// Latin1 (ASCII) fallback.
    font_struct: *mut XFontStruct,
    thai_encoding: ThaiEncoding,
    thai_font: ThaiFont,

    /* Control colors */
    xcolor: [XColor; MESSAGEBOX_COLOR_COUNT],
    xcolor_bevel_l1: XColor,
    xcolor_bevel_l2: XColor,
    xcolor_bevel_d: XColor,
    xcolor_pressed: XColor,
    xcolor_disabled_text: XColor,

    /* Control list */
    has_focus: bool,
    focused_control: Option<usize>,
    fiddled_control: Option<usize>,
    pub(crate) controls: Vec<Control>,
    dyn_controls: Vec<usize>,

    /* User callbacks */
    /// `cb_on_scale_change` with its `cb_data`.
    pub(crate) cb_on_scale_change: Option<ScaleChangeCallback>,

    /* Event loop */
    previous_control: Option<usize>,
    key_control_esc: Option<usize>,
    key_control_enter: Option<usize>,
    last_key_pressed: KeySym,
    ev_scale: f32,
    ev_iscale: f32,
    draw: bool,
    close: bool,
    event_mask: c_long,

    pub(crate) flip_interface: bool,
}

/* Font for icon control */
const G_ICON_FONT: &str = "-*-*-bold-r-normal-*-%d-*-*-*-*-*-iso8859-1[33 88 105]";
const G_ICONFONT_SIZE: c_int = 22;

/* General UI font */
const G_TOOLKIT_FONT_LATIN1: &str = "-*-*-medium-r-normal--0-%d-*-*-p-0-iso8859-1";
const G_TOOLKIT_FONT_LATIN1_FALLBACK: &str = "-*-*-*-*-*--*-*-*-*-*-*-iso8859-1";

const G_TOOLKIT_FONT: &[&str] = &[
    "-*-*-medium-r-normal--*-%d-*-*-*-*-iso10646-1,*", // explicitly unicode (iso10646-1)
    "-*-*-medium-r-*--*-%d-*-*-*-*-iso10646-1,*",      // explicitly unicode (iso10646-1)
    "-misc-*-*-*-*--*-*-*-*-*-*-iso10646-1,*",         // misc unicode (fix for some systems)
    "-*-*-*-*-*--*-*-*-*-*-*-iso10646-1,*",            // just give me anything Unicode.
    "-*-*-medium-r-normal--*-%d-*-*-*-*-iso8859-1,*", // explicitly latin1, in case low-ASCII works out.
    "-*-*-medium-r-*--*-%d-*-*-*-*-iso8859-1,*", // explicitly latin1, in case low-ASCII works out.
    "-misc-*-*-*-*--*-*-*-*-*-*-iso8859-1,*",    // misc latin1 (fix for some systems)
    "-*-*-*-*-*--*-*-*-*-*-*-iso8859-1,*",       // just give me anything latin1.
];
const G_TOOLKITFONT_SIZE: c_int = 140;

#[rustfmt::skip]
const G_DEFAULT_COLORS: [MessageBoxColor; MESSAGEBOX_COLOR_COUNT] = [
    MessageBoxColor { r: 191, g: 184, b: 191 }, // SDL_MESSAGEBOX_COLOR_BACKGROUND,
    MessageBoxColor { r: 0, g: 0, b: 0 },       // SDL_MESSAGEBOX_COLOR_TEXT,
    MessageBoxColor { r: 127, g: 120, b: 127 }, // SDL_MESSAGEBOX_COLOR_BUTTON_BORDER,
    MessageBoxColor { r: 191, g: 184, b: 191 }, // SDL_MESSAGEBOX_COLOR_BUTTON_BACKGROUND,
    MessageBoxColor { r: 235, g: 235, b: 235 }, // SDL_MESSAGEBOX_COLOR_BUTTON_SELECTED,
];

#[cfg(not(target_os = "android"))]
const G_DEFAULT_COLORS_DARK: [MessageBoxColor; MESSAGEBOX_COLOR_COUNT] = [
    MessageBoxColor {
        r: 20,
        g: 20,
        b: 20,
    }, // SDL_MESSAGEBOX_COLOR_BACKGROUND,
    MessageBoxColor {
        r: 192,
        g: 192,
        b: 192,
    }, // SDL_MESSAGEBOX_COLOR_TEXT,
    MessageBoxColor {
        r: 12,
        g: 12,
        b: 12,
    }, // SDL_MESSAGEBOX_COLOR_BUTTON_BORDER,
    MessageBoxColor {
        r: 20,
        g: 20,
        b: 20,
    }, // SDL_MESSAGEBOX_COLOR_BUTTON_BACKGROUND,
    MessageBoxColor {
        r: 36,
        g: 36,
        b: 36,
    }, // SDL_MESSAGEBOX_COLOR_BUTTON_SELECTED,
];

/// The default colors for the system theme.
fn default_colors() -> [MessageBoxColor; MESSAGEBOX_COLOR_COUNT] {
    #[cfg(not(target_os = "android"))]
    {
        use crate::core::linux::system_theme;
        use crate::video::SystemTheme;
        let mut theme = SystemTheme::Light;
        if system_theme::init() {
            theme = system_theme::get();
        }
        if theme == SystemTheme::Dark {
            return G_DEFAULT_COLORS_DARK;
        }
    }
    G_DEFAULT_COLORS
}

/// A font name with its `%d` replaced (`SDL_asprintf(&font, fmt, size)`).
fn font_name(format: &str, size: c_int) -> CString {
    CString::new(format.replacen("%d", &size.to_string(), 1)).unwrap_or_default()
}

/// `SDL_clamp(v, 0, 65535)` for a color channel.
fn clamp_channel(v: i32) -> u16 {
    v.clamp(0, 65535) as u16
}

/// An `XColor` to allocate (`flags = DoRed|DoGreen|DoBlue`).
fn xcolor(red: i32, green: i32, blue: i32) -> XColor {
    XColor {
        pixel: 0,
        red: clamp_channel(red),
        green: clamp_channel(green),
        blue: clamp_channel(blue),
        flags: (DoRed | DoGreen | DoBlue) as c_char,
        pad: 0,
    }
}

static G_SHM_ERROR: AtomicBool = AtomicBool::new(false);
static G_OLD_ERROR_HANDLER: Mutex<XErrorHandler> = Mutex::new(Option::None);

/// Translation of `X11Toolkit_SharedMemoryErrorHandler()`.
unsafe extern "C" fn x11_toolkit_shared_memory_error_handler(
    d: *mut Display,
    e: *mut XErrorEvent,
) -> c_int {
    // SAFETY: Xlib passes a valid error event.
    let code = unsafe { (*e).error_code } as c_int;
    if code == BadAccess as c_int || code == BadRequest as c_int {
        G_SHM_ERROR.store(true, Ordering::Relaxed);
        return 0;
    }
    let handler = *G_OLD_ERROR_HANDLER
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    match handler {
        // SAFETY: the previous handler, called as Xlib would.
        Some(h) => unsafe { h(d, e) },
        Option::None => 0,
    }
}

/// Translation of `X11Toolkit_ShouldFlipUI()`.
fn x11_toolkit_should_flip_ui() -> bool {
    const RTL_LOCALES: &[(&str, Option<&str>)] = &[
        ("ar", Option::None),
        ("fa", Some("AF")),
        ("fa", Some("IR")),
        ("he", Option::None),
        ("iw", Option::None),
        ("yi", Option::None),
        ("ur", Option::None),
        ("ug", Option::None),
        ("kd", Option::None),
        ("pk", Some("PK")),
        ("ps", Option::None),
    ];

    let current_locales = crate::locale::preferred_locales().unwrap_or_default();
    let Some(first) = current_locales.first() else {
        return false;
    };
    for &(language, country) in RTL_LOCALES {
        if first.language.starts_with(language) {
            return match country {
                Option::None => true,
                Some(country) => first
                    .country
                    .as_deref()
                    .is_some_and(|c| c.starts_with(country)),
            };
        }
    }

    false
}

/// Translation of `X11Toolkit_GetTextWidthHeightForFont()`: width, height,
/// ascent and the font height.
fn x11_toolkit_get_text_width_height_for_font(
    x: &X11Syms,
    font: *mut XFontStruct,
    str: &[u8],
) -> (c_int, c_int, c_int, c_int) {
    let mut text_structure = XCharStruct::default();
    let mut font_direction: c_int = 0;
    let mut font_ascent: c_int = 0;
    let mut font_descent: c_int = 0;

    // SAFETY: the font was loaded on the display; the text has its length.
    unsafe {
        (x.XTextExtents)(
            font,
            str.as_ptr() as *const c_char,
            str.len() as c_int,
            &mut font_direction,
            &mut font_ascent,
            &mut font_descent,
            &mut text_structure,
        );
    }
    (
        text_structure.width as c_int,
        text_structure.ascent as c_int + text_structure.descent as c_int,
        text_structure.ascent as c_int,
        font_ascent + font_descent,
    )
}

impl ToolkitWindow {
    /// Translation of `X11Toolkit_InitWindowPixmap()`.
    fn init_window_pixmap(&mut self) {
        let x = self.x.clone();
        if self.pixmap {
            // SAFETY: the display is open and the window ours; the shared
            // memory calls are System V's, on a segment made here.
            unsafe {
                if self.shm_pixmap == 0 {
                    self.drawable = (x.XCreatePixmap)(
                        self.display,
                        self.window,
                        self.pixmap_width as c_uint,
                        self.pixmap_height as c_uint,
                        self.depth as c_uint,
                    );
                }
                if let (true, Some(shm)) = (self.shm, &x.shm) {
                    self.image = (shm.XShmCreateImage)(
                        self.display,
                        self.visual,
                        self.depth as c_uint,
                        ZPixmap,
                        std::ptr::null_mut(),
                        &mut *self.shm_info,
                        self.pixmap_width as c_uint,
                        self.pixmap_height as c_uint,
                    );
                    if !self.image.is_null() {
                        self.shm_bytes_per_line = (*self.image).bytes_per_line;

                        self.shm_info.shmid = libc::shmget(
                            libc::IPC_PRIVATE,
                            ((*self.image).bytes_per_line * (*self.image).height) as usize,
                            libc::IPC_CREAT | 0o777,
                        );
                        if self.shm_info.shmid < 0 {
                            XDestroyImage(self.image);
                            self.image = std::ptr::null_mut();
                            self.shm = false;
                            return;
                        }

                        self.shm_info.readOnly = False;
                        self.shm_info.shmaddr =
                            libc::shmat(self.shm_info.shmid, std::ptr::null(), 0) as *mut c_char;
                        (*self.image).data = self.shm_info.shmaddr;
                        if self.shm_info.shmaddr as isize == -1 {
                            // FIXME (upstream): the image is destroyed with
                            // its data pointing at (char *)-1.
                            (*self.image).data = std::ptr::null_mut();
                            XDestroyImage(self.image);
                            self.shm = false;
                            self.image = std::ptr::null_mut();
                            return;
                        }

                        G_SHM_ERROR.store(false, Ordering::Relaxed);
                        let previous =
                            (x.XSetErrorHandler)(Some(x11_toolkit_shared_memory_error_handler));
                        *G_OLD_ERROR_HANDLER
                            .lock()
                            .unwrap_or_else(|e| e.into_inner()) = previous;
                        (shm.XShmAttach)(self.display, &mut *self.shm_info);
                        (x.XSync)(self.display, False);
                        (x.XSetErrorHandler)(previous);
                        if G_SHM_ERROR.load(Ordering::Relaxed) {
                            // (the data is the shared memory: not freed by
                            // XDestroyImage of a shared memory image)
                            XDestroyImage(self.image);
                            libc::shmdt(self.shm_info.shmaddr as *const libc::c_void);
                            libc::shmctl(self.shm_info.shmid, libc::IPC_RMID, std::ptr::null_mut());
                            self.image = std::ptr::null_mut();
                            self.shm = false;
                            return;
                        }

                        if self.shm_pixmap != 0 {
                            self.drawable = (shm.XShmCreatePixmap)(
                                self.display,
                                self.window,
                                self.shm_info.shmaddr,
                                &mut *self.shm_info,
                                self.pixmap_width as c_uint,
                                self.pixmap_height as c_uint,
                                self.depth as c_uint,
                            );
                            if self.drawable == None {
                                self.shm_pixmap = False;
                            } else {
                                XDestroyImage(self.image);
                                self.image = std::ptr::null_mut();
                            }
                        }

                        libc::shmctl(self.shm_info.shmid, libc::IPC_RMID, std::ptr::null_mut());
                    } else {
                        self.shm = false;
                    }
                }
            }
        }
    }

    /// Translation of `X11Toolkit_InitWindowFonts()`.
    fn init_window_fonts(&mut self) {
        let x = self.x.clone();
        self.thai_encoding = ThaiEncoding::None;
        self.thai_font = ThaiFont::Cell;
        self.utf8 = true;
        self.font_set = std::ptr::null_mut();
        let mut load_traditional = x.utf8.is_none();
        if !load_traditional {
            let mut missing: *mut *mut c_char = std::ptr::null_mut();
            let mut num_missing: c_int = 0;
            self.font_struct = std::ptr::null_mut();
            for &format in G_TOOLKIT_FONT {
                if format.contains("%d") {
                    // FIXME (upstream): if no size of the font loads, iscale
                    // keeps decreasing (to negative sizes) forever.
                    loop {
                        let font = font_name(format, G_TOOLKITFONT_SIZE * self.iscale);
                        // SAFETY: the display is open; the out-parameters are valid.
                        self.font_set = unsafe {
                            (x.XCreateFontSet)(
                                self.display,
                                font.as_ptr(),
                                &mut missing,
                                &mut num_missing,
                                std::ptr::null_mut(),
                            )
                        };

                        if self.font_set.is_null() {
                            if self.scale != 0.0 && self.iscale > 0 {
                                self.iscale = self.scale.ceil() as c_int;
                                self.scale = 0.0;
                            } else {
                                self.iscale -= 1;
                            }
                            // (goto try_load_font, leaking `missing` as upstream)
                            continue;
                        }
                        break;
                    }
                } else {
                    let font = CString::new(format).unwrap_or_default();
                    // SAFETY: as above.
                    self.font_set = unsafe {
                        (x.XCreateFontSet)(
                            self.display,
                            font.as_ptr(),
                            &mut missing,
                            &mut num_missing,
                            std::ptr::null_mut(),
                        )
                    };
                }

                if !missing.is_null() {
                    // SAFETY: the list came from XCreateFontSet.
                    unsafe {
                        (x.XFreeStringList)(missing);
                    }
                    missing = std::ptr::null_mut();
                }

                if !self.font_set.is_null() {
                    break;
                }
            }

            if self.font_set.is_null() {
                load_traditional = true;
            } else {
                let mut font_structs: *mut *mut XFontStruct = std::ptr::null_mut();
                let mut font_names: *mut *mut c_char = std::ptr::null_mut();

                /* TODO: What to do the XFontSet happens to have more than one Thai font? */
                // SAFETY: the font set was just created; Xlib owns the lists.
                let font_sz = unsafe {
                    (x.XFontsOfFontSet)(self.font_set, &mut font_structs, &mut font_names)
                };
                for i in 0..font_sz.max(0) as usize {
                    // SAFETY: the font set has font_sz names.
                    let name = unsafe { CStr::from_ptr(*font_names.add(i)) };
                    let name_str = name.to_string_lossy();
                    let mut thai_encoding = ThaiEncoding::None;
                    if name_str.contains("tis620-0") {
                        thai_encoding = ThaiEncoding::Tis;
                    } else if name_str.contains("tis620-1") {
                        thai_encoding = ThaiEncoding::TisMac;
                    } else if name_str.contains("tis620-2") {
                        thai_encoding = ThaiEncoding::TisWin;
                    } else if name_str.contains("iso8859-11") {
                        thai_encoding = ThaiEncoding::Iso8859;
                    } else if name_str.contains("iso10646-1") {
                        thai_encoding = ThaiEncoding::Unicode;
                    }

                    /* TODO: Set encoding to none if the font does not actually have any Thai codepoints */
                    if thai_encoding != ThaiEncoding::None {
                        /* We have to load the font again because the font_struct supplied by FontsOfFontSet does not have the per_char member set */
                        // SAFETY: the display is open; the font is freed here.
                        unsafe {
                            let font_struct = (x.XLoadQueryFont)(self.display, name.as_ptr());
                            if !font_struct.is_null() {
                                let fs = &*font_struct;
                                if !fs.per_char.is_null() {
                                    let glyphs_sz = (fs.max_char_or_byte2 as c_int
                                        - fs.min_char_or_byte2 as c_int
                                        + 1)
                                        * (fs.max_byte1 as c_int - fs.min_byte1 as c_int + 1);
                                    for j in 0..glyphs_sz.max(0) as usize {
                                        if (*fs.per_char.add(j)).lbearing < 0 {
                                            self.thai_font = ThaiFont::Offset;
                                        }
                                    }
                                }
                                (x.XFreeFont)(self.display, font_struct);
                            }
                        }
                    }

                    self.thai_encoding = thai_encoding;
                }
            }
        }

        if load_traditional {
            // (load_font_traditional:)
            self.utf8 = false;
            loop {
                let font = font_name(G_TOOLKIT_FONT_LATIN1, G_TOOLKITFONT_SIZE * self.iscale);
                // SAFETY: the display is open.
                self.font_struct = unsafe { (x.XLoadQueryFont)(self.display, font.as_ptr()) };
                if self.font_struct.is_null() {
                    if self.iscale > 0 {
                        if self.scale != 0.0 {
                            self.iscale = self.scale.ceil() as c_int;
                            self.scale = 0.0;
                        } else {
                            self.iscale -= 1;
                        }
                        continue;
                    } else {
                        let fallback =
                            CString::new(G_TOOLKIT_FONT_LATIN1_FALLBACK).unwrap_or_default();
                        // SAFETY: as above.
                        self.font_struct =
                            unsafe { (x.XLoadQueryFont)(self.display, fallback.as_ptr()) };
                    }
                }
                break;
            }
        }
    }

    /// Translation of `X11Toolkit_SettingsNotify()`.
    fn settings_notify(&mut self, name: &str) {
        let x = self.x.clone();

        if self.xsettings_first_time {
            return;
        }

        if name == SDL_XSETTINGS_GDK_WINDOW_SCALING_FACTOR
            || name == SDL_XSETTINGS_GDK_UNSCALED_DPI
            || name == SDL_XSETTINGS_XFT_DPI
        {
            let mut dbe_already_setup = false;
            let mut pixmap_already_setup = false;

            if self.pixmap {
                pixmap_already_setup = true;
            } else {
                dbe_already_setup = true;
            }

            /* set scale vars */
            self.scale = x11_get_global_content_scale(&x, self.display, self.xsettings.as_ref());
            self.iscale = self.scale.ceil() as c_int;
            if self.scale < 1.0 {
                self.scale = 1.0;
            }
            if self.scale.round() == self.scale {
                self.scale = 0.0;
            }

            /* setup fonts */
            // SAFETY: the fonts were loaded on the display and are freed once.
            unsafe {
                if !self.font_set.is_null() {
                    (x.XFreeFontSet)(self.display, self.font_set);
                }
                if !self.font_struct.is_null() {
                    (x.XFreeFont)(self.display, self.font_struct);
                }
            }

            self.init_window_fonts();

            /* set up window */
            if self.scale != 0.0 {
                self.window_width =
                    ((self.window_width / self.iscale) as f32 * self.scale).round() as c_int;
                self.window_height =
                    ((self.window_height / self.iscale) as f32 * self.scale).round() as c_int;
                self.pixmap_width = self.window_width;
                self.pixmap_height = self.window_height;
                self.pixmap = true;
            } else {
                self.pixmap = false;
            }

            // SAFETY: the display is open; the drawables are ours.
            unsafe {
                if self.pixmap {
                    if !pixmap_already_setup {
                        if let (Some(xdbe), true) = (&x.xdbe, self.xdbe) {
                            (xdbe.XdbeDeallocateBackBufferName)(self.display, self.buf);
                        }
                    }
                    (x.XFreePixmap)(self.display, self.drawable);
                    self.init_window_pixmap();
                } else if !dbe_already_setup {
                    (x.XFreePixmap)(self.display, self.drawable);
                    if !self.image.is_null() {
                        XDestroyImage(self.image);
                        self.image = std::ptr::null_mut();
                    }
                    if let (Some(xdbe), true) = (&x.xdbe, self.xdbe) {
                        self.buf = (xdbe.XdbeAllocateBackBufferName)(
                            self.display,
                            self.window,
                            XdbeUndefined,
                        );
                        self.drawable = self.buf;
                    }
                }
            }

            /* notify controls */
            for i in 0..self.controls.len() {
                self.controls[i].do_size = true;

                self.control_on_scale_change(i);

                self.control_calc_size(i);

                self.controls[i].do_size = false;
            }

            /* notify cb */
            if let Some(mut cb) = self.cb_on_scale_change.take() {
                cb(self);
                self.cb_on_scale_change = Some(cb);
            }

            /* update ev scales */
            if !self.pixmap {
                self.ev_scale = 1.0;
                self.ev_iscale = 1.0;
            } else {
                self.ev_scale = self.scale;
                self.ev_iscale = self.iscale as f32;
            }
        }
    }

    /// Deliver the queued XSETTINGS notifications.
    fn deliver_settings_notifications(&mut self) {
        let notifications = match &mut self.xsettings {
            Some(client) => client.take_notifications(),
            Option::None => Vec::new(),
        };
        for (name, _action, _setting) in notifications {
            self.settings_notify(&name);
        }
    }

    /// Translation of `X11Toolkit_GetTextWidthHeight()`: width, height,
    /// ascent, descent and the font height.
    fn get_text_width_height(&self, str: &[u8]) -> (c_int, c_int, c_int, c_int, c_int) {
        let x = &self.x;
        if let (true, Some(utf8)) = (self.utf8, &x.utf8) {
            let mut overall_ink = XRectangle::default();
            let mut overall_logical = XRectangle::default();

            // SAFETY: the font set was created on the display; the text has
            // its length; the extents belong to the font set.
            unsafe {
                (utf8.Xutf8TextExtents)(
                    self.font_set,
                    str.as_ptr() as *const c_char,
                    str.len() as c_int,
                    &mut overall_ink,
                    &mut overall_logical,
                );
                let width = overall_logical.width as c_int;
                let height = overall_logical.height as c_int;
                let ascent = -(overall_logical.y as c_int);
                let descent = height - ascent;

                let extents = (utf8.XExtentsOfFontSet)(self.font_set);
                let font_height = (*extents).max_logical_extent.height as c_int;
                (width, height, ascent, descent, font_height)
            }
        } else {
            let mut text_structure = XCharStruct::default();
            let mut font_direction: c_int = 0;
            let mut font_ascent: c_int = 0;
            let mut font_descent: c_int = 0;
            // SAFETY: the font was loaded on the display.
            unsafe {
                (x.XTextExtents)(
                    self.font_struct,
                    str.as_ptr() as *const c_char,
                    str.len() as c_int,
                    &mut font_direction,
                    &mut font_ascent,
                    &mut font_descent,
                    &mut text_structure,
                );
            }
            (
                text_structure.width as c_int,
                text_structure.ascent as c_int + text_structure.descent as c_int,
                text_structure.ascent as c_int,
                text_structure.descent as c_int,
                font_ascent + font_descent,
            )
        }
    }

    /// Translation of `X11Toolkit_MakeTextElements()`: the runs of Thai and
    /// other text of `txt`.
    fn make_text_elements(&self, txt: &str) -> Vec<TextElement> {
        let mut list = Vec::new();
        let mut thai = false;
        let mut buffer = String::new();
        let mut chars = txt.chars();

        loop {
            let cp = chars.next().map_or(0, |c| c as u32);
            let cond = (0xe00..=0xe7f).contains(&cp);
            if cp == 0 || cond == !thai {
                list.push(TextElement {
                    type_: if thai {
                        TextType::Thai
                    } else {
                        TextType::Generic
                    },
                    str: std::mem::take(&mut buffer).into_bytes(),
                    rect: Rect::default(),
                    font_h: 0,
                });

                thai = !thai;
            }

            if cp == 0 {
                break;
            }

            buffer.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
        }

        list
    }

    /// Translation of `X11Toolkit_ShapeTextElements()`.
    fn shape_text_elements(&self, list: &mut [TextElement]) {
        /* Shape and calculate bounding box */
        for element in list.iter_mut() {
            if element.type_ == TextType::Thai {
                if self.thai_font == ThaiFont::Offset {
                    let (w, h, ascent, _descent, _font_h) =
                        self.get_text_width_height(&element.str);
                    element.rect.w = w;
                    element.rect.h = h;
                    element.rect.y = ascent;
                } else {
                    // (without libthai)
                    let (w, h, ascent, _descent, font_h) = self.get_text_width_height(&element.str);
                    element.rect.w = w;
                    element.rect.h = h;
                    element.rect.y = ascent;
                    element.font_h = font_h;
                }
            } else {
                let (w, h, ascent, _descent, font_h) = self.get_text_width_height(&element.str);
                element.rect.w = w;
                element.rect.h = h;
                element.rect.y = ascent;
                element.font_h = font_h;
            }
        }

        /* Add offsets */
        let mut prev: Option<Rect> = Option::None;
        for element in list.iter_mut() {
            element.rect.x = match prev {
                Some(prev) => prev.x + prev.w,
                Option::None => 0,
            };

            prev = Some(element.rect);
        }
    }

    /// Draw one string with the window's font (`Xutf8DrawString()` or
    /// `XDrawString()`).
    fn draw_string(&self, x: c_int, y: c_int, str: &[u8]) {
        let xs = &self.x;
        // SAFETY: the display is open; the drawable, GC and fonts are ours.
        unsafe {
            match (self.utf8, &xs.utf8) {
                (true, Some(utf8)) => (utf8.Xutf8DrawString)(
                    self.display,
                    self.drawable,
                    self.font_set,
                    self.ctx,
                    x,
                    y,
                    str.as_ptr() as *const c_char,
                    str.len() as c_int,
                ),
                _ => {
                    (xs.XDrawString)(
                        self.display,
                        self.drawable,
                        self.ctx,
                        x,
                        y,
                        str.as_ptr() as *const c_char,
                        str.len() as c_int,
                    );
                }
            }
        }
    }

    /// Translation of `X11Toolkit_DrawTextElements()`.
    fn draw_text_elements(&self, list: &[TextElement], x: c_int, y: c_int) {
        for element in list {
            if element.type_ == TextType::Thai {
                if self.thai_font == ThaiFont::Offset {
                    self.draw_string(x + element.rect.x, y + element.rect.y, &element.str);
                } else {
                    // (the cell font path needs libthai: nothing is drawn)
                }
            } else {
                self.draw_string(x + element.rect.x, y + element.rect.y, &element.str);
            }
        }
    }

    /// Translation of `X11Toolkit_CreateWindowStruct()`.
    pub(crate) fn create_window_struct(
        parent: Option<WindowID>,
        tkparent: Option<&ToolkitWindow>,
        mode: ToolkitWindowMode,
        colorhints: Option<&[MessageBoxColor; MESSAGEBOX_COLOR_COUNT]>,
        create_new_display: bool,
    ) -> Result<ToolkitWindow> {
        let Some(x) = load_symbols() else {
            return Err(Error::new("Couldn't load the X11 libraries"));
        };

        // This code could get called from multiple threads maybe?
        // SAFETY: XInitThreads has no preconditions.
        unsafe {
            (x.XInitThreads)();
        }

        let mut origlocale = Option::None;
        if SDL_SET_LOCALE && mode == ToolkitWindowMode::Dialog {
            // SAFETY: querying and setting the C locale.
            unsafe {
                let current = libc::setlocale(libc::LC_ALL, std::ptr::null());
                if !current.is_null() {
                    origlocale = Some(CStr::from_ptr(current).to_owned());
                    libc::setlocale(libc::LC_ALL, c"".as_ptr());
                }
            }
        }

        let restore_locale = |origlocale: &Option<CString>| {
            if let Some(l) = origlocale {
                // SAFETY: restoring the C locale.
                unsafe {
                    libc::setlocale(libc::LC_ALL, l.as_ptr());
                }
            }
        };

        // (the parent's display and window: the PROP_WINDOW_X11_* properties
        // of a window of the X11 device)
        let parent_x11 = parent.and_then(|p| {
            let props = crate::video::window::Window::from_raw(p)
                .properties()
                .ok()?;
            let display =
                props.get_number(PROP_WINDOW_X11_DISPLAY_POINTER)? as usize as *mut Display;
            let xwindow = props.get_number(PROP_WINDOW_X11_WINDOW_NUMBER)? as Window;
            Some((display, xwindow))
        });

        let display;
        let display_close;
        let open_display = || {
            // SAFETY: a NULL name opens $DISPLAY.
            unsafe { (x.XOpenDisplay)(std::ptr::null()) }
        };
        if create_new_display {
            display = open_display();
            display_close = true;
            if display.is_null() {
                restore_locale(&origlocale);
                unload_symbols();
                return Err(Error::new("Couldn't open X11 display"));
            }
        } else if let Some((parent_display, _)) = parent_x11 {
            display = parent_display;
            display_close = false;
        } else if let Some(tkparent) = tkparent {
            display = tkparent.display;
            display_close = false;
        } else {
            display = open_display();
            display_close = true;
            if display.is_null() {
                restore_locale(&origlocale);
                unload_symbols();
                return Err(Error::new("Couldn't open X11 display"));
            }
        }

        let mut xrandr = false;
        if let Some(rr) = &x.xrandr {
            let mut xrandr_event_base: c_int = 0;
            let mut xrandr_error_base: c_int = 0;
            // SAFETY: the display is open; the out-parameters are valid.
            xrandr = unsafe {
                (rr.XRRQueryExtension)(display, &mut xrandr_event_base, &mut xrandr_error_base) != 0
            };
        }

        let mut shm_pixmap: Bool = False;
        let shm = match &x.shm {
            // SAFETY: the display is open; the out-parameters are valid.
            Some(shm) => unsafe {
                let have = (shm.XShmQueryExtension)(display) != 0;
                if have {
                    let mut major: c_int = 0;
                    let mut minor: c_int = 0;
                    (shm.XShmQueryVersion)(display, &mut major, &mut minor, &mut shm_pixmap);
                    if shm_pixmap != 0 && (shm.XShmPixmapFormat)(display) != ZPixmap {
                        shm_pixmap = False;
                    }
                }
                have
            },
            Option::None => false,
        };

        // SAFETY: the display is open.
        let default_screen = unsafe { DefaultScreen(display) };

        let mut window = ToolkitWindow {
            x: x.clone(),
            origlocale,
            mode,
            display,
            screen: 0,
            display_close,
            parent,
            parent_xwindow: parent_x11.map_or(None, |(_, w)| w),
            tk_parent_window: tkparent.map(|t| t.window),
            window: None,
            drawable: None,
            image: std::ptr::null_mut(),
            shm_info: Box::default(),
            shm_bytes_per_line: 0,
            visual: std::ptr::null_mut(),
            // SAFETY: XVisualInfo is plain data.
            vi: unsafe { std::mem::zeroed() },
            cmap: 0,
            ctx: std::ptr::null_mut(),
            depth: 0,
            pixmap: false,
            buf: None,
            xdbe: false,
            xrandr,
            shm,
            shm_pixmap,
            utf8: false,
            wm_protocols: None,
            wm_delete_message: None,
            window_width: 0,
            window_height: 0,
            pixmap_width: 0,
            pixmap_height: 0,
            window_x: 0,
            window_y: 0,
            xsettings: Option::None,
            xsettings_first_time: true,
            iscale: 1,
            scale: 0.0,
            font_set: std::ptr::null_mut(),
            font_struct: std::ptr::null_mut(),
            thai_encoding: ThaiEncoding::None,
            thai_font: ThaiFont::Cell,
            xcolor: [XColor::default(); MESSAGEBOX_COLOR_COUNT],
            xcolor_bevel_l1: XColor::default(),
            xcolor_bevel_l2: XColor::default(),
            xcolor_bevel_d: XColor::default(),
            xcolor_pressed: XColor::default(),
            xcolor_disabled_text: XColor::default(),
            has_focus: false,
            focused_control: Option::None,
            fiddled_control: Option::None,
            controls: Vec::new(),
            dyn_controls: Vec::new(),
            cb_on_scale_change: Option::None,
            previous_control: Option::None,
            key_control_esc: Option::None,
            key_control_enter: Option::None,
            last_key_pressed: NoSymbol,
            ev_scale: 1.0,
            ev_iscale: 1.0,
            draw: false,
            close: false,
            event_mask: 0,
            flip_interface: false,
        };

        /* Scale/Xsettings */
        window.pixmap = false;
        window.xsettings_first_time = true;
        window.xsettings = Some(XSettingsClient::new(
            x.clone(),
            display,
            default_screen,
            true,
        ));
        window.deliver_settings_notifications();
        window.xsettings_first_time = false;
        window.scale = x11_get_global_content_scale(&x, display, window.xsettings.as_ref());
        if window.scale < 1.0 {
            window.scale = 1.0;
        }
        window.iscale = window.scale.ceil() as c_int;
        if window.scale.round() == window.scale {
            window.scale = 0.0;
        }

        /* Fonts */
        window.init_window_fonts();

        /* Color hints */
        let colorhints = colorhints.copied().unwrap_or_else(default_colors);

        /* Convert colors to 16 bpc XColor format */
        for (i, c) in colorhints.iter().enumerate() {
            window.xcolor[i] = xcolor(c.r as i32 * 257, c.g as i32 * 257, c.b as i32 * 257);
        }

        /* Generate bevel and pressed colors */
        let border = window.xcolor[COLOR_BUTTON_BORDER];
        window.xcolor_bevel_l1 = xcolor(
            border.red as i32 + 12500,
            border.green as i32 + 12500,
            border.blue as i32 + 12500,
        );
        window.xcolor_bevel_l2 = xcolor(
            border.red as i32 + 32500,
            border.green as i32 + 32500,
            border.blue as i32 + 32500,
        );
        window.xcolor_bevel_d = xcolor(
            border.red as i32 - 22500,
            border.green as i32 - 22500,
            border.blue as i32 - 22500,
        );
        let background = window.xcolor[COLOR_BUTTON_BACKGROUND];
        window.xcolor_pressed = xcolor(
            background.red as i32 - 12500,
            background.green as i32 - 12500,
            background.blue as i32 - 12500,
        );
        let text = window.xcolor[COLOR_TEXT];
        window.xcolor_disabled_text = xcolor(
            text.red as i32 + 19500,
            text.green as i32 + 19500,
            text.blue as i32 + 19500,
        );

        /* Screen */
        window.screen = match parent.and_then(display_driver_data_for_window) {
            Some(displaydata) => displaydata.screen,
            Option::None => default_screen,
        };

        /* Visuals */
        // (SDL_TOOLKIT_WINDOW_MODE_X11_CHILD takes the parent's visual and
        // colormap; the message boxes don't make child windows)
        // SAFETY: the display is open and the screen one of its screens.
        unsafe {
            window.visual = DefaultVisual(display, window.screen);
            window.cmap = DefaultColormap(display, window.screen);
            window.depth = DefaultDepth(display, window.screen);
        }
        if let Some(vi) = x11_get_visual_info_from_visual(&x, display, window.visual) {
            window.vi = vi;
        }

        /* Allocate colors */
        // SAFETY: the display is open; the colormap is the screen's.
        unsafe {
            for c in window.xcolor.iter_mut() {
                (x.XAllocColor)(display, window.cmap, c);
            }
            (x.XAllocColor)(display, window.cmap, &mut window.xcolor_bevel_l1);
            (x.XAllocColor)(display, window.cmap, &mut window.xcolor_bevel_l2);
            (x.XAllocColor)(display, window.cmap, &mut window.xcolor_bevel_d);
            (x.XAllocColor)(display, window.cmap, &mut window.xcolor_pressed);
            (x.XAllocColor)(display, window.cmap, &mut window.xcolor_disabled_text);
        }

        /* Interface direction */
        window.flip_interface = x11_toolkit_should_flip_ui();

        Ok(window)
    }

    /// Translation of `X11Toolkit_AddControlToWindow()`: the control's index.
    fn add_control_to_window(&mut self, control: Control) -> usize {
        /* Add to controls list */
        let index = self.controls.len();
        let dynamic = control.dynamic;
        let selected = control.selected;
        self.controls.push(control);

        /* If dynamic, add it to the dynamic controls list too */
        if dynamic {
            self.dyn_controls.push(index);
        }

        /* If selected, set currently focused control to it */
        if selected {
            self.focused_control = Some(index);
        }
        index
    }

    /// Translation of `X11Toolkit_CreateWindowRes()`.
    pub(crate) fn create_window_res(
        &mut self,
        w: c_int,
        h: c_int,
        cx: c_int,
        cy: c_int,
        title: &str,
    ) -> Result<()> {
        let x = self.x.clone();
        let display = self.display;
        let mut gcflags: c_ulong = (GCForeground | GCBackground) as c_ulong;
        let use_xrandr_by_default = true;

        if self.scale == 0.0 {
            self.window_width = w;
            self.window_height = h;
        } else {
            self.window_width = ((w / self.iscale) as f32 * self.scale).round() as c_int;
            self.window_height = ((h / self.iscale) as f32 * self.scale).round() as c_int;
            self.pixmap_width = w;
            self.pixmap_height = h;
            self.pixmap = true;
        }

        let has_parent = self.parent.is_some() && self.parent_xwindow != None;

        let mut valuemask: c_ulong = (CWEventMask | CWColormap) as c_ulong;
        self.event_mask = (ExposureMask
            | ButtonPressMask
            | ButtonReleaseMask
            | KeyPressMask
            | KeyReleaseMask
            | StructureNotifyMask
            | FocusChangeMask
            | PointerMotionMask) as c_long;
        // SAFETY: XSetWindowAttributes is plain data.
        let mut wnd_attr: XSetWindowAttributes = unsafe { std::mem::zeroed() };
        wnd_attr.event_mask = self.event_mask;
        wnd_attr.colormap = self.cmap;
        if self.mode == ToolkitWindowMode::Menu || self.mode == ToolkitWindowMode::Tooltip {
            valuemask |= (CWOverrideRedirect | CWSaveUnder) as c_ulong;
            wnd_attr.save_under = True;
            wnd_attr.override_redirect = True;
        }
        // SAFETY: the display is open.
        let root_win = unsafe { RootWindow(display, self.screen) };
        let parent_win = if self.mode == ToolkitWindowMode::Child {
            self.parent_xwindow
        } else {
            root_win
        };

        // SAFETY: the display is open; the attributes are valid.
        self.window = unsafe {
            (x.XCreateWindow)(
                display,
                parent_win,
                0,
                0,
                self.window_width as c_uint,
                self.window_height as c_uint,
                0,
                self.depth,
                InputOutput as c_uint,
                self.visual,
                valuemask,
                &mut wnd_attr,
            )
        };
        if self.window == None {
            return Err(Error::new("Couldn't create X window"));
        }

        let intern = |name: &str| super::video::intern_atom(&x, display, name, false);
        // SAFETY (for the property calls below): the display is open and
        // the window ours; the data are arrays of atoms.
        if has_parent && self.mode == ToolkitWindowMode::Dialog {
            let _NET_WM_STATE = intern("_NET_WM_STATE");
            // Set some message-boxy window states when attached to a parent window...
            // we skip the taskbar since this will pop to the front when the parent window is clicked in the taskbar, etc
            let stateatoms: [Atom; 4] = [
                intern("_NET_WM_STATE_SKIP_TASKBAR"),
                intern("_NET_WM_STATE_SKIP_PAGER"),
                intern("_NET_WM_STATE_FOCUSED"),
                intern("_NET_WM_STATE_MODAL"),
            ];
            // SAFETY: see above.
            unsafe {
                (x.XChangeProperty)(
                    display,
                    self.window,
                    _NET_WM_STATE,
                    XA_ATOM,
                    32,
                    PropModeReplace,
                    stateatoms.as_ptr() as *const c_uchar,
                    stateatoms.len() as c_int,
                );
            }
        }

        // SAFETY: see above.
        unsafe {
            if has_parent && self.mode != ToolkitWindowMode::Child {
                (x.XSetTransientForHint)(display, self.window, self.parent_xwindow);
            }

            if let Some(tk_parent_window) = self.tk_parent_window {
                (x.XSetTransientForHint)(display, self.window, tk_parent_window);
            }
        }

        let _ = sdl_x11_set_window_title(&x, display, self.window, title);

        // Let the window manager the type of the window
        let window_type = match self.mode {
            ToolkitWindowMode::Dialog => Some("_NET_WM_WINDOW_TYPE_DIALOG"),
            ToolkitWindowMode::Menu => Some("_NET_WM_WINDOW_TYPE_DROPDOWN_MENU"),
            ToolkitWindowMode::Tooltip => Some("_NET_WM_WINDOW_TYPE_TOOLTIP"),
            ToolkitWindowMode::Child => Option::None,
        };
        if let Some(window_type) = window_type {
            let _NET_WM_WINDOW_TYPE = intern("_NET_WM_WINDOW_TYPE");
            let value = intern(window_type);
            // SAFETY: see above.
            unsafe {
                (x.XChangeProperty)(
                    display,
                    self.window,
                    _NET_WM_WINDOW_TYPE,
                    XA_ATOM,
                    32,
                    PropModeReplace,
                    &value as *const Atom as *const c_uchar,
                    1,
                );
            }
        }

        // Allow the window to be deleted by the window manager
        self.wm_delete_message = intern("WM_DELETE_WINDOW");
        // SAFETY: see above.
        unsafe {
            (x.XSetWMProtocols)(display, self.window, &mut self.wm_delete_message, 1);
        }
        self.wm_protocols = intern("WM_PROTOCOLS");

        let (wx, wy) = if self.mode == ToolkitWindowMode::Menu
            || self.mode == ToolkitWindowMode::Tooltip
        {
            (cx, cy)
        } else if has_parent {
            // SAFETY: the display is open; the parent window exists; the
            // out-parameters are valid.
            unsafe {
                let mut attrib: XWindowAttributes = std::mem::zeroed();
                let mut dummy: Window = 0;

                (x.XGetWindowAttributes)(display, self.parent_xwindow, &mut attrib);
                let mut wx = attrib.x + (attrib.width - self.window_width) / 2;
                let mut wy = attrib.y + (attrib.height - self.window_height) / 3;
                let (sx, sy) = (wx, wy);
                (x.XTranslateCoordinates)(
                    display,
                    self.parent_xwindow,
                    RootWindow(display, self.screen),
                    sx,
                    sy,
                    &mut wx,
                    &mut wy,
                    &mut dummy,
                );
                (wx, wy)
            }
        } else {
            let first_display = crate::video::display::displays()
                .ok()
                .and_then(|d| d.first().copied());
            let from_device = first_display.and_then(|id| {
                with_x11_display(id, |dpy, dpydata| {
                    let mode = dpy.current_mode();
                    (
                        dpydata.x + ((mode.w - self.window_width) / 2),
                        dpydata.y + ((mode.h - self.window_height) / 3),
                    )
                })
            });
            match from_device {
                Some(pos) => pos,
                Option::None => {
                    let from_xrandr =
                        if hints::get_bool(hints::VIDEO_X11_XRANDR, use_xrandr_by_default)
                            && self.xrandr
                        {
                            self.xrandr_center(root_win)
                        } else {
                            Option::None
                        };
                    match from_xrandr {
                        Some(pos) => pos,
                        Option::None => {
                            // oh well. This will misposition on a multi-head setup. Init first next time.
                            // (NOXRANDR:)
                            // SAFETY: the display is open.
                            unsafe {
                                (
                                    (DisplayWidth(display, self.screen) - self.window_width) / 2,
                                    (DisplayHeight(display, self.screen) - self.window_height) / 3,
                                )
                            }
                        }
                    }
                }
            }
        };
        // (MOVEWINDOW:)
        // SAFETY: the display is open and the window ours.
        unsafe {
            (x.XMoveWindow)(display, self.window, wx, wy);
        }
        self.window_x = wx;
        self.window_y = wy;

        // SAFETY: the display is open and the window ours; the hints are
        // freed after use.
        unsafe {
            let sizehints = (x.XAllocSizeHints)();
            if !sizehints.is_null() {
                (*sizehints).flags = (USPosition | USSize | PMaxSize | PMinSize) as c_long;
                (*sizehints).x = wx;
                (*sizehints).y = wy;
                (*sizehints).width = self.window_width;
                (*sizehints).height = self.window_height;

                (*sizehints).min_width = self.window_width;
                (*sizehints).max_width = self.window_width;
                (*sizehints).min_height = self.window_height;
                (*sizehints).max_height = self.window_height;

                (x.XSetWMNormalHints)(display, self.window, sizehints);

                (x.XFree)(sizehints.cast());
            }

            (x.XMapRaised)(display, self.window);
        }

        self.drawable = self.window;
        // Initialise a back buffer for double buffering
        if let (Some(xdbe), false) = (&x.xdbe, self.pixmap) {
            let mut xdbe_major: c_int = 0;
            let mut xdbe_minor: c_int = 0;
            // SAFETY: the display is open and the window ours.
            unsafe {
                if (xdbe.XdbeQueryExtension)(display, &mut xdbe_major, &mut xdbe_minor) != 0 {
                    self.xdbe = true;
                    self.buf =
                        (xdbe.XdbeAllocateBackBufferName)(display, self.window, XdbeUndefined);
                    self.drawable = self.buf;
                } else {
                    self.xdbe = false;
                }
            }
        }

        self.init_window_pixmap();

        let mut ctx_vals = XGCValues {
            foreground: self.xcolor[COLOR_BACKGROUND].pixel,
            background: self.xcolor[COLOR_BACKGROUND].pixel,
            // SAFETY: XGCValues is plain data.
            ..unsafe { std::mem::zeroed() }
        };
        if !self.utf8 {
            gcflags |= GCFont as c_ulong;
            // SAFETY: the Latin-1 font was loaded (utf8 is false only then).
            ctx_vals.font = unsafe { (*self.font_struct).fid };
        }
        // SAFETY: the display is open; the drawable is ours.
        self.ctx = unsafe { (x.XCreateGC)(self.display, self.drawable, gcflags, &mut ctx_vals) };
        if self.ctx.is_null() {
            return Err(Error::new("Couldn't create graphics context"));
        }

        self.close = false;
        self.key_control_esc = Option::None;
        self.key_control_enter = Option::None;
        if !self.pixmap {
            self.ev_scale = 1.0;
            self.ev_iscale = 1.0;
        } else {
            self.ev_scale = self.scale;
            self.ev_iscale = self.iscale as f32;
        }

        if SDL_GRAB
            && (self.mode == ToolkitWindowMode::Menu || self.mode == ToolkitWindowMode::Tooltip)
        {
            // SAFETY: the display is open and the window ours.
            unsafe {
                (x.XGrabPointer)(
                    display,
                    self.window,
                    False,
                    (ButtonPressMask | ButtonReleaseMask | PointerMotionMask) as c_uint,
                    GrabModeAsync,
                    GrabModeAsync,
                    None,
                    None,
                    CurrentTime,
                );
                (x.XGrabKeyboard)(
                    display,
                    self.window,
                    False,
                    GrabModeAsync,
                    GrabModeAsync,
                    CurrentTime,
                );
            }
        }

        Ok(())
    }

    /// The XRandR part of positioning a dialog without a parent (the
    /// `FIRSTOUTPUTXRANDR` and `FIRSTCRTCXRANDR` paths of
    /// `X11Toolkit_CreateWindowRes()`); `None` for `NOXRANDR`.
    fn xrandr_center(&self, root_win: Window) -> Option<(c_int, c_int)> {
        let rr = self.x.xrandr.as_ref()?;
        let display = self.display;
        let (ww, wh) = (self.window_width, self.window_height);
        // SAFETY: the display is open; the XRandR structures are freed
        // after use, as upstream.
        unsafe {
            let screen_res = (rr.XRRGetScreenResourcesCurrent)(display, root_win);
            if screen_res.is_null() {
                return Option::None;
            }
            let res = &*screen_res;

            let crtc_of = |out_info: *mut XRROutputInfo| -> *mut XRRCrtcInfo {
                let o = &*out_info;
                if o.crtc != None {
                    (rr.XRRGetCrtcInfo)(display, screen_res, o.crtc)
                } else if o.ncrtc > 0 {
                    (rr.XRRGetCrtcInfo)(display, screen_res, *o.crtcs)
                } else {
                    std::ptr::null_mut()
                }
            };

            let default_out = (rr.XRRGetOutputPrimary)(display, root_win);
            if default_out != None {
                let out_info = (rr.XRRGetOutputInfo)(display, screen_res, default_out);
                // FIXME (upstream): the output info isn't checked for NULL
                // (C dereferences it; here NULL goes on as "not connected").
                if !out_info.is_null() && (*out_info).connection == RR_Connected {
                    let crtc_info = crtc_of(out_info);

                    if !crtc_info.is_null() {
                        let pos = (
                            ((*crtc_info).width as c_int - ww) / 2,
                            ((*crtc_info).height as c_int - wh) / 3,
                        );
                        (rr.XRRFreeOutputInfo)(out_info);
                        (rr.XRRFreeCrtcInfo)(crtc_info);
                        (rr.XRRFreeScreenResources)(screen_res);
                        return Some(pos);
                    } else {
                        (rr.XRRFreeOutputInfo)(out_info);
                        // FIXME (upstream): the screen resources are leaked
                        // on this path.
                        return Option::None;
                    }
                } else if !out_info.is_null() {
                    (rr.XRRFreeOutputInfo)(out_info);
                }
            }

            // (FIRSTOUTPUTXRANDR:)
            if res.noutput > 0 {
                let out_info = (rr.XRRGetOutputInfo)(display, screen_res, *res.outputs);
                if !out_info.is_null() {
                    let crtc_info = crtc_of(out_info);

                    if !crtc_info.is_null() {
                        let pos = (
                            ((*crtc_info).width as c_int - ww) / 2,
                            ((*crtc_info).height as c_int - wh) / 3,
                        );
                        (rr.XRRFreeOutputInfo)(out_info);
                        (rr.XRRFreeCrtcInfo)(crtc_info);
                        (rr.XRRFreeScreenResources)(screen_res);
                        return Some(pos);
                    }
                    (rr.XRRFreeOutputInfo)(out_info);
                }
            }

            // (FIRSTCRTCXRANDR:)
            if res.ncrtc == 0 {
                (rr.XRRFreeScreenResources)(screen_res);
                return Option::None;
            }

            let crtc_info = (rr.XRRGetCrtcInfo)(display, screen_res, *res.crtcs);
            if !crtc_info.is_null() {
                let pos = (
                    ((*crtc_info).width as c_int - ww) / 2,
                    ((*crtc_info).height as c_int - wh) / 3,
                );
                (rr.XRRFreeCrtcInfo)(crtc_info);
                (rr.XRRFreeScreenResources)(screen_res);
                Some(pos)
            } else {
                (rr.XRRFreeScreenResources)(screen_res);
                Option::None
            }
        }
    }

    /// Translation of `X11Toolkit_DrawWindow()`.
    fn draw_window(&mut self) {
        let x = self.x.clone();
        let display = self.display;

        // SAFETY: the display is open; the drawable and GC are ours.
        unsafe {
            if let (Some(xdbe), true, false) = (&x.xdbe, self.xdbe, self.pixmap) {
                (xdbe.XdbeBeginIdiom)(display);
            }

            (x.XSetForeground)(display, self.ctx, self.xcolor[COLOR_BACKGROUND].pixel);
            if self.pixmap {
                (x.XFillRectangle)(
                    display,
                    self.drawable,
                    self.ctx,
                    0,
                    0,
                    self.pixmap_width as c_uint,
                    self.pixmap_height as c_uint,
                );
            } else {
                (x.XFillRectangle)(
                    display,
                    self.drawable,
                    self.ctx,
                    0,
                    0,
                    self.window_width as c_uint,
                    self.window_height as c_uint,
                );
            }
        }

        for i in 0..self.controls.len() {
            self.control_draw(i);
        }

        // SAFETY: as above.
        unsafe {
            if let (Some(xdbe), true, false) = (&x.xdbe, self.xdbe, self.pixmap) {
                let mut swap_info = super::x11dyn::XdbeSwapInfo {
                    swap_window: self.window,
                    swap_action: XdbeUndefined,
                };
                (xdbe.XdbeSwapBuffers)(display, &mut swap_info, 1);
                (xdbe.XdbeEndIdiom)(display);
            }
        }

        if self.pixmap {
            let rect = Rect::new(0, 0, self.window_width, self.window_height);
            let format = x11_get_pixel_format_from_visual_info(&x, display, &self.vi);
            // SAFETY: the display is open; the images and the shared memory
            // are ours, with bytes_per_line * height bytes.
            unsafe {
                if self.shm {
                    if let Some(shm) = &x.shm {
                        if self.shm_pixmap != 0 {
                            (x.XFlush)(display);
                            (x.XSync)(display, False);
                            let len = (self.shm_bytes_per_line * self.pixmap_height) as usize;
                            let pixels = std::slice::from_raw_parts_mut(
                                self.shm_info.shmaddr as *mut u8,
                                len,
                            );
                            scale_in_place(
                                pixels,
                                self.pixmap_width,
                                self.pixmap_height,
                                format,
                                self.shm_bytes_per_line,
                                &rect,
                            );
                            (x.XCopyArea)(
                                display,
                                self.drawable,
                                self.window,
                                self.ctx,
                                0,
                                0,
                                self.window_width as c_uint,
                                self.window_height as c_uint,
                                0,
                                0,
                            );
                        } else {
                            (shm.XShmGetImage)(display, self.drawable, self.image, 0, 0, AllPlanes);
                            let len =
                                ((*self.image).bytes_per_line * (*self.image).height) as usize;
                            let pixels =
                                std::slice::from_raw_parts_mut((*self.image).data as *mut u8, len);
                            scale_in_place(
                                pixels,
                                self.pixmap_width,
                                self.pixmap_height,
                                format,
                                (*self.image).bytes_per_line,
                                &rect,
                            );
                            (shm.XShmPutImage)(
                                display,
                                self.window,
                                self.ctx,
                                self.image,
                                0,
                                0,
                                0,
                                0,
                                self.window_width as c_uint,
                                self.window_height as c_uint,
                                False,
                            );
                        }
                    }
                } else {
                    let image = (x.XGetImage)(
                        display,
                        self.drawable,
                        0,
                        0,
                        self.pixmap_width as c_uint,
                        self.pixmap_height as c_uint,
                        AllPlanes,
                        ZPixmap,
                    );
                    if !image.is_null() {
                        let len = ((*image).bytes_per_line * (*image).height) as usize;
                        let pixels = std::slice::from_raw_parts_mut((*image).data as *mut u8, len);
                        scale_in_place(
                            pixels,
                            self.pixmap_width,
                            self.pixmap_height,
                            format,
                            (*image).bytes_per_line,
                            &rect,
                        );
                        (x.XPutImage)(
                            display,
                            self.window,
                            self.ctx,
                            image,
                            0,
                            0,
                            0,
                            0,
                            self.window_width as c_uint,
                            self.window_height as c_uint,
                        );

                        XDestroyImage(image);
                    }
                }
            }
        }

        // SAFETY: the display is open.
        unsafe {
            (x.XFlush)(display);
        }
    }

    /// Translation of `X11Toolkit_GetControlMouseIsOn()`.
    fn get_control_mouse_is_on(&self, x: c_int, y: c_int) -> Option<usize> {
        self.controls.iter().position(|c| {
            let rect = &c.rect;
            (x >= rect.x) && (x <= (rect.x + rect.w)) && (y >= rect.y) && (y <= (rect.y + rect.h))
        })
    }

    /// The event coordinates in the control's space.
    fn ev_point(&self, x: c_int, y: c_int) -> (c_int, c_int) {
        (
            ((x as f32 / self.ev_scale) * self.ev_iscale).round() as c_int,
            ((y as f32 / self.ev_scale) * self.ev_iscale).round() as c_int,
        )
    }

    /// Translation of `X11Toolkit_ProcessWindowEvents()`.
    pub(crate) fn process_window_events(&mut self, e: &mut XEvent) {
        let x = self.x.clone();
        /* If X11_XFilterEvent returns True, then some input method has filtered the
        event, and the client should discard the event. */
        // SAFETY: the event is valid.
        if e.get_type() != Expose && unsafe { (x.XFilterEvent)(e, None) } != 0 {
            return;
        }

        self.draw = false;

        match e.get_type() {
            Expose => {
                self.draw = true;
            }
            ClientMessage => {
                let c = e.client();
                if c.message_type == self.wm_protocols
                    && c.format == 32
                    && c.data.l[0] as Atom == self.wm_delete_message
                {
                    self.close = true;
                }
            }
            FocusIn => {
                self.has_focus = true;
            }
            FocusOut => {
                self.has_focus = false;
                if let Some(f) = self.fiddled_control {
                    self.controls[f].selected = false;
                }
                self.fiddled_control = Option::None;
                for control in self.controls.iter_mut() {
                    control.state = ControlState::Normal;
                }
            }
            MotionNotify => {
                if self.has_focus {
                    self.previous_control = self.fiddled_control;
                    let b = *e.button();
                    let (px, py) = self.ev_point(b.x, b.y);
                    self.fiddled_control = self.get_control_mouse_is_on(px, py);
                    if let Some(p) = self.previous_control {
                        self.controls[p].state = ControlState::Normal;
                        self.control_on_state_change(p);
                        self.draw = true;
                    }
                    if let Some(f) = self.fiddled_control {
                        if self.controls[f].dynamic {
                            self.controls[f].state = ControlState::Hover;
                            self.control_on_state_change(f);
                            self.draw = true;
                        } else {
                            self.fiddled_control = Option::None;
                        }
                    }
                }
            }
            ButtonPress => {
                self.previous_control = self.fiddled_control;
                if let Some(p) = self.previous_control {
                    self.controls[p].state = ControlState::Normal;
                    self.control_on_state_change(p);
                    self.draw = true;
                }
                let b = *e.button();
                if b.button == Button1 {
                    let (px, py) = self.ev_point(b.x, b.y);
                    self.fiddled_control = self.get_control_mouse_is_on(px, py);
                    if let Some(f) = self.fiddled_control {
                        self.controls[f].state = ControlState::PressedHeld;
                        self.control_on_state_change(f);
                        self.draw = true;
                    }
                }
            }
            ButtonRelease => {
                let b = *e.button();
                if self.mode == ToolkitWindowMode::Menu || self.mode == ToolkitWindowMode::Tooltip {
                    let cx = b.x;
                    let cy = b.y;

                    if cy < 0 || cx < 0 {
                        self.close = true;
                    }

                    if cy > self.window_height || cx > self.window_width {
                        self.close = true;
                    }
                }

                if let (true, Some(f)) = (b.button == Button1, self.fiddled_control) {
                    let (px, py) = self.ev_point(b.x, b.y);
                    let control = self.get_control_mouse_is_on(px, py);
                    if Some(f) == control {
                        self.controls[f].state = ControlState::Pressed;
                        self.control_on_state_change(f);
                        if let Some(c) = self.controls.get_mut(f) {
                            c.state = ControlState::Normal;
                        }
                        self.draw = true;
                    }
                }
            }
            KeyPress => {
                // SAFETY: a key event.
                self.last_key_pressed = unsafe { (x.XLookupKeysym)(e.key_mut(), 0) };

                if self.last_key_pressed == XK_Escape {
                    for i in 0..self.controls.len() {
                        if self.controls[i].is_default_esc {
                            self.controls[i].state = ControlState::Pressed;
                            self.draw = true;
                            self.key_control_esc = Some(i);
                        }
                    }
                } else if (self.last_key_pressed == XK_Return)
                    || (self.last_key_pressed == XK_KP_Enter)
                {
                    for i in 0..self.controls.len() {
                        if self.controls[i].selected {
                            self.controls[i].state = ControlState::Pressed;
                            self.draw = true;
                            self.key_control_enter = Some(i);
                        }
                    }
                }
            }
            KeyRelease => {
                // SAFETY: a key event.
                let key = unsafe { (x.XLookupKeysym)(e.key_mut(), 0) };

                // If this is a key release for something we didn't get the key down for, then bail.
                if key != self.last_key_pressed {
                    // (break)
                } else if key == XK_Escape {
                    if let Some(c) = self.key_control_esc {
                        self.control_on_state_change(c);
                    }
                } else if (key == XK_Return) || (key == XK_KP_Enter) {
                    if let Some(c) = self.key_control_enter {
                        self.control_on_state_change(c);
                    }
                } else if key == XK_Tab || key == XK_Left || key == XK_Right {
                    if let Some(f) = self.focused_control {
                        self.controls[f].selected = false;
                    }
                    self.draw = true;
                    for ev_i in 0..self.dyn_controls.len() {
                        if Some(self.dyn_controls[ev_i]) == self.focused_control {
                            let mut next_index: isize = if key == XK_Left {
                                ev_i as isize - 1
                            } else {
                                ev_i as isize + 1
                            };
                            if (next_index >= self.dyn_controls.len() as isize) || (next_index < 0)
                            {
                                if key == XK_Right || key == XK_Left {
                                    next_index = ev_i as isize;
                                } else {
                                    next_index = 0;
                                }
                            }
                            let next = self.dyn_controls[next_index as usize];
                            self.focused_control = Some(next);
                            self.controls[next].selected = true;
                            break;
                        }
                    }
                }
            }
            _ => {}
        }

        if self.draw {
            self.draw_window();
        }
    }

    /// Translation of `X11Toolkit_DoWindowEventLoop()`.
    pub(crate) fn do_window_event_loop(&mut self) {
        let x = self.x.clone();
        while !self.close {
            let mut e = XEvent::zeroed();

            /* Process settings events */
            // SAFETY: the display is open; XPeekEvent blocks for an event.
            unsafe {
                (x.XPeekEvent)(self.display, &mut e);
            }
            if let Some(xsettings) = &mut self.xsettings {
                xsettings.process_event(&e);
            }
            self.deliver_settings_notifications();

            /* Do actual event loop */
            // SAFETY: the predicate reads this window's display and X window
            // (passed as a pair, alive for the call).
            unsafe {
                let mut target = (self.display, self.window);
                (x.XIfEvent)(
                    self.display,
                    &mut e,
                    Some(x11_toolkit_event_test),
                    &mut target as *mut (*mut Display, Window) as XPointer,
                );
            }
            self.process_window_events(&mut e);
        }
    }

    /// Translation of `X11Toolkit_ResizeWindow()`.
    pub(crate) fn resize_window(&mut self, w: c_int, h: c_int) {
        let x = self.x.clone();
        if !self.pixmap {
            self.window_width = w;
            self.window_height = h;
        } else {
            self.window_width = ((w / self.iscale) as f32 * self.scale).round() as c_int;
            self.window_height = ((h / self.iscale) as f32 * self.scale).round() as c_int;
            self.pixmap_width = w;
            self.pixmap_height = h;
            // SAFETY: the display is open; the pixmap is ours.
            unsafe {
                (x.XFreePixmap)(self.display, self.drawable);
            }
            self.init_window_pixmap();
        }

        // SAFETY: the display is open and the window ours.
        unsafe {
            (x.XResizeWindow)(
                self.display,
                self.window,
                self.window_width as c_uint,
                self.window_height as c_uint,
            );
        }
    }

    /// Translation of `X11Toolkit_SignalWindowClose()`.
    pub(crate) fn signal_window_close(&mut self) {
        self.close = true;
    }

    // * * * Generic controls ("virtual functions")

    /// `func_draw`.
    fn control_draw(&mut self, i: usize) {
        match &self.controls[i].kind {
            ControlKind::Icon(_) => self.draw_icon_control(i),
            ControlKind::Button(_) => self.draw_button_control(i),
            ControlKind::Label(_) => self.draw_label_control(i),
        }
    }

    /// `func_calc_size`.
    fn control_calc_size(&mut self, i: usize) {
        match &self.controls[i].kind {
            ControlKind::Icon(_) => self.calculate_icon_control(i),
            ControlKind::Button(_) => self.calculate_button_control(i),
            ControlKind::Label(_) => self.calculate_label_control(i),
        }
    }

    /// `func_on_scale_change`.
    fn control_on_scale_change(&mut self, i: usize) {
        if let ControlKind::Icon(_) = &self.controls[i].kind {
            self.on_icon_control_scale_change(i);
        }
    }

    /// `func_on_state_change`.
    fn control_on_state_change(&mut self, i: usize) {
        if let ControlKind::Button(_) = &self.controls[i].kind {
            self.on_button_control_state_change(i);
        }
    }

    /// `func_free`.
    fn control_free(&mut self, control: Control) {
        match control.kind {
            ControlKind::Icon(icon) => {
                // Translation of `X11Toolkit_DestroyIconControl()`.
                // SAFETY: the font was loaded on the display; freed once.
                unsafe {
                    (self.x.XFreeFont)(self.display, icon.icon_char_font);
                }
            }
            // (X11Toolkit_DestroyButtonControl() and
            // X11Toolkit_DestroyLabelControl() free their text)
            ControlKind::Button(_) | ControlKind::Label(_) => {}
        }
    }

    /// Translation of `X11Toolkit_NotifyControlOfSizeChange()`.
    pub(crate) fn notify_control_of_size_change(&mut self, i: usize) -> bool {
        // (every control has func_calc_size)
        self.control_calc_size(i);
        true
    }

    // * * * Icon control

    /// Translation of `X11Toolkit_DrawIconControl()`.
    fn draw_icon_control(&mut self, i: usize) {
        let x = self.x.clone();
        let iscale = self.iscale;
        let (display, ctx, drawable) = (self.display, self.ctx, self.drawable);
        let utf8 = self.utf8;
        let font_struct = self.font_struct;
        let control = &mut self.controls[i];
        let ControlKind::Icon(icon_control) = &control.kind else {
            return;
        };

        control.rect.w -= 2 * iscale;
        control.rect.h -= 2 * iscale;
        let r = control.rect;
        // SAFETY: the display is open; the drawable, GC and fonts are ours.
        unsafe {
            (x.XSetForeground)(display, ctx, icon_control.xcolor_bg_shadow.pixel);
            (x.XFillArc)(
                display,
                drawable,
                ctx,
                r.x + (2 * iscale),
                r.y + (2 * iscale),
                r.w as c_uint,
                r.h as c_uint,
                0,
                360 * 64,
            );

            let (outer, inner, glyph) = match icon_control.flags.0
                & (MessageBoxFlags::ERROR.0
                    | MessageBoxFlags::WARNING.0
                    | MessageBoxFlags::INFORMATION.0)
            {
                f if f == MessageBoxFlags::ERROR.0 => (
                    Some(icon_control.xcolor_red_darker.pixel),
                    icon_control.xcolor_red.pixel,
                    icon_control.xcolor_white.pixel,
                ),
                f if f == MessageBoxFlags::WARNING.0 => (
                    Some(icon_control.xcolor_black.pixel),
                    icon_control.xcolor_yellow.pixel,
                    icon_control.xcolor_black.pixel,
                ),
                f if f == MessageBoxFlags::INFORMATION.0 => (
                    Some(icon_control.xcolor_white.pixel),
                    icon_control.xcolor_blue.pixel,
                    icon_control.xcolor_white.pixel,
                ),
                _ => (Option::None, 0, 0),
            };
            if let Some(outer) = outer {
                (x.XSetForeground)(display, ctx, outer);
                (x.XFillArc)(
                    display,
                    drawable,
                    ctx,
                    r.x,
                    r.y,
                    r.w as c_uint,
                    r.h as c_uint,
                    0,
                    360 * 64,
                );
                (x.XSetForeground)(display, ctx, inner);
                (x.XFillArc)(
                    display,
                    drawable,
                    ctx,
                    r.x + iscale,
                    r.y + iscale,
                    (r.w - 2 * iscale) as c_uint,
                    (r.h - 2 * iscale) as c_uint,
                    0,
                    360 * 64,
                );
                (x.XSetForeground)(display, ctx, glyph);
            }
            (x.XSetFont)(display, ctx, (*icon_control.icon_char_font).fid);
            (x.XDrawString)(
                display,
                drawable,
                ctx,
                r.x + icon_control.icon_char_x,
                r.y + icon_control.icon_char_y,
                &icon_control.icon_char as *const u8 as *const c_char,
                1,
            );
            if !utf8 {
                (x.XSetFont)(display, ctx, (*font_struct).fid);
            }
        }

        control.rect.w += 2 * iscale;
        control.rect.h += 2 * iscale;
    }

    /// Translation of `X11Toolkit_CalculateIconControl()`.
    fn calculate_icon_control(&mut self, i: usize) {
        let x = self.x.clone();
        let iscale = self.iscale;
        let base_control = &mut self.controls[i];
        let ControlKind::Icon(control) = &mut base_control.kind else {
            return;
        };
        let (icon_char_w, icon_char_h, icon_char_a, _) = x11_toolkit_get_text_width_height_for_font(
            &x,
            control.icon_char_font,
            &[control.icon_char],
        );
        control.icon_char_h = icon_char_h;
        control.icon_char_a = icon_char_a;
        base_control.rect.w = icon_char_w;
        base_control.rect.h = control.icon_char_h;
        let icon_wh =
            icon_char_w.max(control.icon_char_h) + SDL_TOOLKIT_X11_ELEMENT_PADDING * 2 * iscale;
        base_control.rect.w = icon_wh;
        base_control.rect.h = icon_wh;
        base_control.rect.y = 0;
        base_control.rect.x = 0;
        control.icon_char_y = control.icon_char_a + (base_control.rect.h - control.icon_char_h) / 2;
        control.icon_char_x = (base_control.rect.w - icon_char_w) / 2;
        base_control.rect.w += 2 * iscale;
        base_control.rect.h += 2 * iscale;
    }

    /// Translation of `X11Toolkit_OnIconControlScaleChange()`.
    fn on_icon_control_scale_change(&mut self, i: usize) {
        let x = self.x.clone();
        let (display, iscale) = (self.display, self.iscale);
        let ControlKind::Icon(control) = &mut self.controls[i].kind else {
            return;
        };
        // SAFETY: the display is open; the old font is freed once.
        unsafe {
            (x.XFreeFont)(display, control.icon_char_font);
            let font = font_name(G_ICON_FONT, G_ICONFONT_SIZE * iscale);
            control.icon_char_font = (x.XLoadQueryFont)(display, font.as_ptr());
            if control.icon_char_font.is_null() {
                let font = font_name(G_TOOLKIT_FONT_LATIN1, G_TOOLKITFONT_SIZE * iscale);
                control.icon_char_font = (x.XLoadQueryFont)(display, font.as_ptr());
            }
        }
    }

    /// Translation of `X11Toolkit_CreateIconControl()`: the control's
    /// index, `None` without an icon (or its font).
    pub(crate) fn create_icon_control(&mut self, flags: MessageBoxFlags) -> Option<usize> {
        let x = self.x.clone();
        let display = self.display;

        /* Load font */
        // SAFETY: the display is open.
        let mut icon_char_font = unsafe {
            let font = font_name(G_ICON_FONT, G_ICONFONT_SIZE * self.iscale);
            (x.XLoadQueryFont)(display, font.as_ptr())
        };
        if icon_char_font.is_null() {
            // SAFETY: as above.
            icon_char_font = unsafe {
                let font = font_name(G_TOOLKIT_FONT_LATIN1, G_TOOLKITFONT_SIZE * self.iscale);
                (x.XLoadQueryFont)(display, font.as_ptr())
            };
            if icon_char_font.is_null() {
                return Option::None;
            }
        }

        let mut control = IconControl {
            flags,
            icon_char: 0,
            icon_char_font,
            icon_char_x: 0,
            icon_char_y: 0,
            icon_char_a: 0,
            icon_char_h: 0,
            xcolor_black: XColor::default(),
            xcolor_red: XColor::default(),
            xcolor_red_darker: XColor::default(),
            xcolor_white: XColor::default(),
            xcolor_yellow: XColor::default(),
            xcolor_blue: XColor::default(),
            xcolor_bg_shadow: XColor::default(),
        };

        /* Set colors */
        // SAFETY (for the XAllocColor calls): the display is open; the
        // colormap is the window's.
        match flags.0
            & (MessageBoxFlags::ERROR.0
                | MessageBoxFlags::WARNING.0
                | MessageBoxFlags::INFORMATION.0)
        {
            f if f == MessageBoxFlags::ERROR.0 => {
                control.icon_char = b'X';
                control.xcolor_white = xcolor(65535, 65535, 65535);
                control.xcolor_red = xcolor(65535, 0, 0);
                control.xcolor_red_darker = xcolor(40535, 0, 0);
                // SAFETY: see above.
                unsafe {
                    (x.XAllocColor)(display, self.cmap, &mut control.xcolor_white);
                    (x.XAllocColor)(display, self.cmap, &mut control.xcolor_red);
                    (x.XAllocColor)(display, self.cmap, &mut control.xcolor_red_darker);
                }
            }
            f if f == MessageBoxFlags::WARNING.0 => {
                control.icon_char = b'!';
                control.xcolor_black = xcolor(0, 0, 0);
                control.xcolor_yellow = xcolor(65535, 65535, 0);
                // SAFETY: see above.
                unsafe {
                    (x.XAllocColor)(display, self.cmap, &mut control.xcolor_black);
                    (x.XAllocColor)(display, self.cmap, &mut control.xcolor_yellow);
                }
            }
            f if f == MessageBoxFlags::INFORMATION.0 => {
                control.icon_char = b'i';
                control.xcolor_white = xcolor(65535, 65535, 65535);
                control.xcolor_blue = xcolor(0, 0, 65535);
                // SAFETY: see above.
                unsafe {
                    (x.XAllocColor)(display, self.cmap, &mut control.xcolor_white);
                    (x.XAllocColor)(display, self.cmap, &mut control.xcolor_blue);
                }
            }
            _ => {
                // SAFETY: the font was just loaded.
                unsafe {
                    (x.XFreeFont)(display, icon_char_font);
                }
                return Option::None;
            }
        }
        let bg = self.xcolor[COLOR_BACKGROUND];
        let shadow = |c: u16| -> i32 {
            if c > 32896 {
                c as i32 - 12500
            } else if c == 0 {
                c as i32 + 9000
            } else {
                c as i32 - 3000
            }
        };
        control.xcolor_bg_shadow = xcolor(shadow(bg.red), shadow(bg.green), shadow(bg.blue));
        // SAFETY: see above.
        unsafe {
            (x.XAllocColor)(display, self.cmap, &mut control.xcolor_bg_shadow);
        }

        /* Sizing and positioning */
        let index = self.add_control_to_window(Control {
            state: ControlState::Normal,
            rect: Rect::default(),
            selected: false,
            dynamic: false,
            is_default_enter: false,
            is_default_esc: false,
            do_size: false,
            kind: ControlKind::Icon(control),
        });
        self.calculate_icon_control(index);
        Some(index)
    }

    // * * * Button control

    /// Translation of `X11Toolkit_CalculateButtonControl()`.
    fn calculate_button_control(&mut self, i: usize) {
        let iscale = self.iscale;
        let control = &mut self.controls[i];
        let ControlKind::Button(button_control) = &mut control.kind else {
            return;
        };
        let (text_rect, _) = get_text_elements_rect(&button_control.text);
        button_control.text_rect = text_rect;
        if control.do_size {
            control.rect.w =
                SDL_TOOLKIT_X11_ELEMENT_PADDING_3 * 2 * iscale + button_control.text_rect.w;
            control.rect.h =
                SDL_TOOLKIT_X11_ELEMENT_PADDING_3 * 2 * iscale + button_control.text_rect.h;
        }
        button_control.text_rect.x = (control.rect.w - button_control.text_rect.w) / 2;
        button_control.text_rect.y = (control.rect.h - button_control.text_rect.h) / 2;
    }

    /// Translation of `X11Toolkit_DrawButtonControl()`.
    fn draw_button_control(&mut self, i: usize) {
        let x = self.x.clone();
        let (display, ctx, drawable, s) = (self.display, self.ctx, self.drawable, self.iscale);
        let control = &self.controls[i];
        let ControlKind::Button(button_control) = &control.kind else {
            return;
        };
        let r = control.rect;
        let fill = |color: c_ulong, dx: c_int, dy: c_int, dw: c_int, dh: c_int| {
            // SAFETY: the display is open; the drawable and GC are ours.
            unsafe {
                (x.XSetForeground)(display, ctx, color);
                (x.XFillRectangle)(
                    display,
                    drawable,
                    ctx,
                    r.x + dx,
                    r.y + dy,
                    (r.w - dw) as c_uint,
                    (r.h - dh) as c_uint,
                );
            }
        };
        let face = if control.state == ControlState::Hover {
            self.xcolor[COLOR_BUTTON_SELECTED].pixel
        } else {
            self.xcolor[COLOR_BUTTON_BACKGROUND].pixel
        };

        // SAFETY: as above.
        unsafe {
            (x.XSetForeground)(display, ctx, self.xcolor[COLOR_TEXT].pixel);
        }
        /* Draw bevel */
        if control.state == ControlState::Pressed || control.state == ControlState::PressedHeld {
            fill(self.xcolor_bevel_d.pixel, 0, 0, 0, 0);
            fill(self.xcolor_bevel_l2.pixel, 0, 0, s, s);
            fill(self.xcolor_bevel_l1.pixel, s, s, 3 * s, 2 * s);
            fill(self.xcolor[COLOR_BUTTON_BORDER].pixel, s, s, 3 * s, 3 * s);
            fill(self.xcolor_pressed.pixel, 2 * s, 2 * s, 4 * s, 4 * s);
        } else if control.selected {
            fill(self.xcolor_bevel_d.pixel, 0, 0, 0, 0);
            fill(self.xcolor_bevel_l2.pixel, s, s, 3 * s, 3 * s);
            fill(
                self.xcolor[COLOR_BUTTON_BORDER].pixel,
                2 * s,
                2 * s,
                4 * s,
                4 * s,
            );
            fill(self.xcolor_bevel_l1.pixel, 2 * s, 2 * s, 5 * s, 5 * s);
            fill(face, 3 * s, 3 * s, 6 * s, 6 * s);
        } else {
            fill(self.xcolor_bevel_d.pixel, 0, 0, 0, 0);
            fill(self.xcolor_bevel_l2.pixel, 0, 0, s, s);
            fill(self.xcolor[COLOR_BUTTON_BORDER].pixel, s, s, 2 * s, 2 * s);
            fill(self.xcolor_bevel_l1.pixel, s, s, 3 * s, 3 * s);
            fill(face, 2 * s, 2 * s, 4 * s, 4 * s);
        }

        // SAFETY: as above.
        unsafe {
            (x.XSetForeground)(display, ctx, self.xcolor[COLOR_TEXT].pixel);
        }
        self.draw_text_elements(
            &button_control.text,
            r.x + button_control.text_rect.x,
            r.y + button_control.text_rect.y,
        );
    }

    /// Translation of `X11Toolkit_OnButtonControlStateChange()`.
    fn on_button_control_state_change(&mut self, i: usize) {
        let control = &mut self.controls[i];
        let state = control.state;
        let ControlKind::Button(button_control) = &mut control.kind else {
            return;
        };
        if state == ControlState::Pressed {
            if let Some(mut cb) = button_control.cb.take() {
                cb(self, i);
                if let Some(Control {
                    kind: ControlKind::Button(b),
                    ..
                }) = self.controls.get_mut(i)
                {
                    b.cb = Some(cb);
                }
            }
        }
    }

    /// Translation of `X11Toolkit_CreateButtonControl()`: the control's index.
    pub(crate) fn create_button_control(&mut self, data: &MessageBoxButtonData) -> usize {
        let is_default_esc = data
            .flags
            .contains(MessageBoxButtonFlags::ESCAPEKEY_DEFAULT);
        let is_default_enter = data
            .flags
            .contains(MessageBoxButtonFlags::RETURNKEY_DEFAULT);
        let mut text = self.make_text_elements(&data.text);
        self.shape_text_elements(&mut text);

        let index = self.add_control_to_window(Control {
            state: ControlState::Normal,
            rect: Rect::default(),
            selected: is_default_enter,
            dynamic: true,
            is_default_enter,
            is_default_esc,
            do_size: true,
            kind: ControlKind::Button(ButtonControl {
                data: data.clone(),
                text,
                text_rect: Rect::default(),
                cb: Option::None,
            }),
        });
        self.calculate_button_control(index);
        self.controls[index].do_size = false;
        index
    }

    /// Translation of `X11Toolkit_RegisterCallbackForButtonControl()`.
    pub(crate) fn register_callback_for_button_control(&mut self, i: usize, cb: ButtonCallback) {
        if let ControlKind::Button(button_control) = &mut self.controls[i].kind {
            button_control.cb = Some(cb);
        }
    }

    /// Translation of `X11Toolkit_GetButtonControlData()`.
    pub(crate) fn get_button_control_data(&self, i: usize) -> Option<&MessageBoxButtonData> {
        match &self.controls[i].kind {
            ControlKind::Button(button_control) => Some(&button_control.data),
            _ => Option::None,
        }
    }

    // * * * Label control

    /// Translation of `X11Toolkit_DrawLabelControl()`.
    fn draw_label_control(&mut self, i: usize) {
        let control = &self.controls[i];
        let ControlKind::Label(label_control) = &control.kind else {
            return;
        };
        // SAFETY: the display is open; the GC is ours.
        unsafe {
            (self.x.XSetForeground)(self.display, self.ctx, self.xcolor[COLOR_TEXT].pixel);
        }
        for line in &label_control.lines {
            self.draw_text_elements(
                &line.text,
                control.rect.x + line.rect.x,
                control.rect.y + line.rect.y,
            );
        }
    }

    /// Translation of `X11Toolkit_CalculateLabelControl()`.
    fn calculate_label_control(&mut self, i: usize) {
        let base_control = &mut self.controls[i];
        let ControlKind::Label(control) = &mut base_control.kind else {
            return;
        };

        if base_control.do_size {
            base_control.rect.w = 0;
            base_control.rect.h = 0;
        }

        for j in 0..control.lines.len() {
            let (rect, font_h) = get_text_elements_rect(&control.lines[j].text);
            control.lines[j].rect = rect;

            if base_control.do_size {
                base_control.rect.w = base_control.rect.w.max(control.lines[j].rect.w);
            }

            if j > 0 {
                control.lines[j].rect.y = font_h + control.lines[j - 1].rect.y;
            } else {
                control.lines[j].rect.y = 0;
            }
        }

        // (the FriBidi paragraph directions: none without FriBidi)

        if base_control.do_size {
            if let Some(last) = control.lines.last() {
                base_control.rect.h = last.rect.y + last.rect.h;
            }
        }
    }

    /// Translation of `X11Toolkit_CreateLabelControl()`: the control's
    /// index, `None` for no text.
    pub(crate) fn create_label_control(&mut self, utf8: &str) -> Option<usize> {
        if utf8.is_empty() {
            return Option::None;
        }

        let sz = x11_toolkit_count_lines_of_text(utf8);
        let mut lines = Vec::with_capacity(sz);
        let mut rest = utf8;
        for _ in 0..sz {
            let lf = rest.find('\n');
            let length = lf.unwrap_or(rest.len());
            let mut line_sz = length;

            if lf.is_some() && length > 0 && rest.as_bytes()[length - 1] == b'\r' {
                line_sz -= 1;
            }

            let mut text = self.make_text_elements(&rest[..line_sz]);
            self.shape_text_elements(&mut text);
            lines.push(LabelLine {
                text,
                rect: Rect::default(),
            });

            if lf.is_none() {
                break;
            }
            rest = &rest[length + 1..];
        }

        let index = self.add_control_to_window(Control {
            state: ControlState::Normal,
            rect: Rect::default(),
            selected: false,
            dynamic: false,
            is_default_enter: false,
            is_default_esc: false,
            do_size: true,
            kind: ControlKind::Label(LabelControl { lines }),
        });
        self.calculate_label_control(index);
        self.controls[index].do_size = false;

        Some(index)
    }

    /// Translation of `X11Toolkit_GetLabelControlFirstLineHeight()`.
    pub(crate) fn get_label_control_first_line_height(&self, i: usize) -> c_int {
        match &self.controls[i].kind {
            ControlKind::Label(label_control) => {
                label_control.lines.first().map_or(0, |l| l.rect.h)
            }
            _ => 0,
        }
    }

    /// Translation of `X11Toolkit_DestroyWindow()`.
    pub(crate) fn destroy_window(mut self) {
        let x = self.x.clone();
        let display = self.display;

        // SAFETY: the display is open; every resource is the window's and
        // is freed once.
        unsafe {
            if SDL_GRAB
                && (self.mode == ToolkitWindowMode::Menu || self.mode == ToolkitWindowMode::Tooltip)
            {
                (x.XUngrabPointer)(display, CurrentTime);
                (x.XUngrabKeyboard)(display, CurrentTime);
            }

            for control in std::mem::take(&mut self.controls) {
                self.control_free(control);
            }
            self.dyn_controls.clear();

            if self.pixmap {
                (x.XFreePixmap)(display, self.drawable);
            }

            if self.pixmap && self.shm {
                if let Some(shm) = &x.shm {
                    (shm.XShmDetach)(display, &mut *self.shm_info);
                }
                if self.shm_pixmap == 0 {
                    XDestroyImage(self.image);
                }
                libc::shmdt(self.shm_info.shmaddr as *const libc::c_void);
            }

            if !self.font_set.is_null() {
                (x.XFreeFontSet)(display, self.font_set);
                self.font_set = std::ptr::null_mut();
            }

            if !self.font_struct.is_null() {
                (x.XFreeFont)(display, self.font_struct);
                self.font_struct = std::ptr::null_mut();
            }

            if let (Some(xdbe), true, false) = (&x.xdbe, self.xdbe, self.pixmap) {
                (xdbe.XdbeDeallocateBackBufferName)(display, self.buf);
            }

            // (xsettings_client_destroy())
            self.xsettings = Option::None;

            if !self.ctx.is_null() {
                (x.XFreeGC)(display, self.ctx);
            }

            if !display.is_null() {
                if self.window != None {
                    (x.XWithdrawWindow)(display, self.window, self.screen);
                    (x.XDestroyWindow)(display, self.window);
                    self.window = None;
                }

                if self.display_close {
                    (x.XCloseDisplay)(display);
                }
                self.display = std::ptr::null_mut();
            }

            if SDL_SET_LOCALE && self.mode == ToolkitWindowMode::Dialog {
                if let Some(l) = &self.origlocale {
                    libc::setlocale(libc::LC_ALL, l.as_ptr());
                }
            }
        }
        unload_symbols();
    }
}

/// Translation of `X11Toolkit_EventTest()`.
unsafe extern "C" fn x11_toolkit_event_test(
    _display: *mut Display,
    event: *mut XEvent,
    arg: XPointer,
) -> Bool {
    // SAFETY: `arg` is the (display, window) pair of the event loop; Xlib
    // passes a valid event.
    unsafe {
        let (display, window) = *(arg as *const (*mut Display, Window));
        let any = (*event).any();

        if any.display != display {
            return False;
        }

        if any.window == window {
            return True;
        }
    }

    False
}

/// Translation of `X11Toolkit_GetTextElementsRect()`: the size of the text
/// and its font height.
fn get_text_elements_rect(list: &[TextElement]) -> (Rect, c_int) {
    let mut ret = 0;
    let mut out = Rect::default();
    for element in list {
        out.w += element.rect.w;
        out.h = out.h.max(element.rect.h);
        ret = ret.max(element.font_h);
    }

    (out, ret)
}

/// Translation of `X11Toolkit_CountLinesOfText()`.
fn x11_toolkit_count_lines_of_text(text: &str) -> usize {
    let mut result = 0;
    let mut text = Some(text);
    while let Some(t) = text.filter(|t| !t.is_empty()) {
        result += 1; // even without an endline, this counts as a line.
        text = t.find('\n').map(|lf| &t[lf + 1..]);
    }
    result
}

/// `SDL_BlitSurfaceScaled(scale_surface, NULL, scale_surface, &rect,
/// SDL_SCALEMODE_LINEAR)` over pixel memory.
///
/// FIXME (upstream): the surface is blitted onto itself (overlapping
/// source and destination); the source is a copy here.
fn scale_in_place(
    pixels: &mut [u8],
    w: c_int,
    h: c_int,
    format: crate::video::PixelFormat,
    pitch: c_int,
    rect: &Rect,
) {
    if let Ok(mut surface) = Surface::from_pixels(w, h, format, pixels, pitch) {
        if let Ok(mut source) = surface.duplicate() {
            let _ = source.blit_scaled(Option::None, &mut surface, Some(rect), ScaleMode::Linear);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_of_text() {
        assert_eq!(x11_toolkit_count_lines_of_text(""), 0);
        assert_eq!(x11_toolkit_count_lines_of_text("a"), 1);
        assert_eq!(x11_toolkit_count_lines_of_text("a\nb"), 2);
        assert_eq!(x11_toolkit_count_lines_of_text("a\n"), 1);
        assert_eq!(
            font_name(G_ICON_FONT, 44).to_str().unwrap(),
            "-*-*-bold-r-normal-*-44-*-*-*-*-*-iso8859-1[33 88 105]"
        );
    }

    #[test]
    fn default_colors_follow_the_system_theme() {
        use crate::core::linux::dbus::test_bus::{self, Bus};
        use crate::core::linux::system_theme::serve_test_portal;
        let _l = crate::test_support::test_lock();
        let Some(bus) = Bus::start() else { return };
        for (scheme, colors) in [(1, G_DEFAULT_COLORS_DARK), (2, G_DEFAULT_COLORS)] {
            let portal = serve_test_portal(&bus, scheme);
            test_bus::use_as_session(&bus);
            assert_eq!(default_colors(), colors);
            test_bus::release_session();
            portal.stop();
        }
    }
}
