// Rust translation of src/video/x11/SDL_x11mouse.c and SDL_x11mouse.h from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The mouse: cursors (Xcursor images, with a pixmap fallback, and the
//! system cursors from the cursor theme or the cursor font), showing and
//! hiding them, warping, capturing the pointer and the global mouse state.

use std::ffi::{c_char, c_int, c_uint, CString};
use std::sync::Arc;

use super::modes::display_driver_data;
use super::sys::*;
use super::video::{X11Display, X11Video};
use super::window::with_x11_window;
use super::xinput2::x11_xinput2_is_initialized;
use crate::error::{Error, Result};
use crate::events::mouse::{self, Cursor, CursorFrame, MouseButtonFlags, SystemCursor};
use crate::events::WindowID;
use crate::video::core::{css_cursor_name, update_window_grab, window_ids};
use crate::video::sysvideo::HitTestResult;
use crate::video::{PixelFormat, Surface};

/// An X cursor resource (`Cursor` of Xlib).
type XCursorId = super::sys::Cursor;

/// The backend data of a cursor. Translation of `struct SDL_CursorData`;
/// the cursor is freed (`X11_FreeCursor()`) when the last reference goes.
pub(crate) struct CursorData {
    pub(crate) cursor: XCursorId,
    conn: Arc<X11Display>,
}

impl Drop for CursorData {
    /// Translation of `X11_FreeCursor()`.
    fn drop(&mut self) {
        let x11_cursor = self.cursor;

        if x11_cursor != None {
            // SAFETY: the cursor was created on this display, which is still
            // open (the cursor holds a reference to it).
            unsafe {
                (self.conn.x.XFreeCursor)(self.conn.display, x11_cursor);
            }
        }
    }
}

/// The cursors of the device (the statics of `SDL_x11mouse.c`).
pub(crate) struct MouseData {
    // FIXME: Find a better place to put this...
    pub(crate) x11_empty_cursor: XCursorId,
    pub(crate) x11_cursor_visible: bool,
    /// `sys_cursors`, indexed by `SDL_HitTestResult`.
    pub(crate) sys_cursors: [Option<Cursor>; HitTestResult::ResizeLeft as usize + 1],
}

impl Default for MouseData {
    fn default() -> Self {
        MouseData {
            x11_empty_cursor: None,
            x11_cursor_visible: true,
            sys_cursors: Default::default(),
        }
    }
}

/// Translation of `GetLegacySystemCursorShape()`.
fn get_legacy_system_cursor_shape(id: SystemCursor) -> c_uint {
    use SystemCursor as C;
    match id {
        // X Font Cursors reference:
        // http://tronche.com/gui/x/xlib/appendix/b/
        C::Default => XC_left_ptr,
        C::Text => XC_xterm,
        C::Wait => XC_watch,
        C::Crosshair => XC_tcross,
        C::Progress => XC_watch,
        C::NwseResize => XC_top_left_corner,
        C::NeswResize => XC_top_right_corner,
        C::EwResize => XC_sb_h_double_arrow,
        C::NsResize => XC_sb_v_double_arrow,
        C::Move => XC_fleur,
        C::NotAllowed => XC_pirate,
        C::Pointer => XC_hand2,
        C::NwResize => XC_top_left_corner,
        C::NResize => XC_top_side,
        C::NeResize => XC_top_right_corner,
        C::EResize => XC_right_side,
        C::SeResize => XC_bottom_right_corner,
        C::SResize => XC_bottom_side,
        C::SwResize => XC_bottom_left_corner,
        C::WResize => XC_left_side,
        C::ContextMenu => XC_hand2,
        C::Help => XC_question_arrow,
        C::Cell => XC_cross,
        C::VerticalText => XC_xterm,
        C::Alias => XC_hand2,
        C::Copy => XC_hand2,
        C::NoDrop => XC_pirate,
        C::Grab => XC_hand2,
        C::Grabbing => XC_hand2,
        C::ColResize => XC_sb_h_double_arrow,
        C::RowResize => XC_sb_v_double_arrow,
        C::AllScroll => XC_fleur,
        C::ZoomIn => XC_hand2,
        C::ZoomOut => XC_hand2,
    }
}

/// The ARGB8888 pixels of a cursor surface, row by row (`pitch` bytes).
fn cursor_pixels<'a>(surface: &'a Surface<'_>) -> &'a [u8] {
    surface.pixels().unwrap_or(&[])
}

impl X11Video {
    /// Translation of `X11_CreateEmptyCursor()`.
    fn x11_create_empty_cursor(&self) -> XCursorId {
        let existing = self.with_data(|d| d.mouse.x11_empty_cursor);
        if existing != None {
            return existing;
        }
        let display = self.display;
        let data: [c_char; 1] = [0];
        let mut color = XColor {
            red: 0,
            green: 0,
            blue: 0,
            ..XColor::default()
        };
        let mut cursor: XCursorId = None;
        // SAFETY: the display is open; the bitmap data is 1x1; the pixmap
        // is freed after use.
        unsafe {
            let pixmap = (self.x.XCreateBitmapFromData)(
                display,
                DefaultRootWindow(display),
                data.as_ptr(),
                1,
                1,
            );
            if pixmap != 0 {
                cursor = (self.x.XCreatePixmapCursor)(
                    display, pixmap, pixmap, &mut color, &mut color, 0, 0,
                );
                (self.x.XFreePixmap)(display, pixmap);
            }
        }
        self.with_data(|d| d.mouse.x11_empty_cursor = cursor);
        cursor
    }

    /// Translation of `X11_DestroyEmptyCursor()`.
    fn x11_destroy_empty_cursor(&self) {
        let cursor = self.with_data(|d| std::mem::replace(&mut d.mouse.x11_empty_cursor, None));
        if cursor != None {
            // SAFETY: the display is open; the cursor is ours.
            unsafe {
                (self.x.XFreeCursor)(self.display, cursor);
            }
        }
    }

    /// Translation of `X11_CreateCursorAndData()`.
    fn x11_create_cursor_and_data(&self, x11_cursor: XCursorId) -> Cursor {
        Cursor::with_internal(CursorData {
            cursor: x11_cursor,
            conn: self.conn.clone(),
        })
    }

    /// Translation of `X11_CreateXCursorCursor()`.
    fn x11_create_xcursor_cursor(
        &self,
        surface: &Surface<'_>,
        hot_x: i32,
        hot_y: i32,
    ) -> XCursorId {
        let Some(xcursor) = &self.x.xcursor else {
            return None;
        };
        let display = self.display;

        // SAFETY: XcursorImageCreate allocates a w*h image (or fails).
        let image = unsafe { (xcursor.XcursorImageCreate)(surface.width(), surface.height()) };
        if image.is_null() {
            let _ = Error::out_of_memory();
            return None;
        }
        // SAFETY: the image was just created with room for w*h pixels; the
        // surface is ARGB8888 with a pitch of w*4 (asserted, as upstream).
        unsafe {
            (*image).xhot = hot_x as XcursorDim;
            (*image).yhot = hot_y as XcursorDim;
            (*image).delay = 0;

            crate::sdl_assert!(surface.format() == PixelFormat::ARGB8888);
            crate::sdl_assert!(surface.pitch() == surface.width() * 4);
            let pixels = cursor_pixels(surface);
            let len = (surface.height() as usize * surface.pitch() as usize).min(pixels.len());
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), (*image).pixels as *mut u8, len);

            let cursor = (xcursor.XcursorImageLoadCursor)(display, image);

            (xcursor.XcursorImageDestroy)(image);

            cursor
        }
    }

    /// Translation of `X11_CreateAnimatedXCursorCursor()`.
    fn x11_create_animated_xcursor_cursor(
        &self,
        frames: &[CursorFrame<'_>],
        hot_x: i32,
        hot_y: i32,
    ) -> XCursorId {
        let Some(xcursor) = &self.x.xcursor else {
            return None;
        };
        let display = self.display;
        let mut cursor: XCursorId = None;

        // SAFETY: XcursorImagesCreate allocates room for the frames.
        let images = unsafe { (xcursor.XcursorImagesCreate)(frames.len() as c_int) };
        if images.is_null() {
            let _ = Error::out_of_memory();
            return None;
        }

        let mut complete = true;
        for (i, frame) in frames.iter().enumerate() {
            // SAFETY: as in x11_create_xcursor_cursor(); `images` has room
            // for every frame.
            unsafe {
                let image =
                    (xcursor.XcursorImageCreate)(frame.surface.width(), frame.surface.height());
                if image.is_null() {
                    let _ = Error::out_of_memory();
                    complete = false;
                    break;
                }
                (*image).xhot = hot_x as XcursorDim;
                (*image).yhot = hot_y as XcursorDim;
                (*image).delay = frame.duration;

                crate::sdl_assert!(frame.surface.format() == PixelFormat::ARGB8888);
                crate::sdl_assert!(frame.surface.pitch() == frame.surface.width() * 4);
                let pixels = cursor_pixels(frame.surface);
                let len = (frame.surface.height() as usize * frame.surface.pitch() as usize)
                    .min(pixels.len());
                std::ptr::copy_nonoverlapping(pixels.as_ptr(), (*image).pixels as *mut u8, len);

                *(*images).images.add(i) = image;
                (*images).nimage += 1;
            }
        }

        // SAFETY: the images are complete (or the load is skipped); they are
        // destroyed once.
        unsafe {
            if complete {
                cursor = (xcursor.XcursorImagesLoadCursor)(display, images);
            }

            (xcursor.XcursorImagesDestroy)(images);
        }
        cursor
    }

    /// Translation of `X11_CreatePixmapCursor()`.
    fn x11_create_pixmap_cursor(&self, surface: &Surface<'_>, hot_x: i32, hot_y: i32) -> XCursorId {
        let display = self.display;
        let mut fg = XColor::default();
        let mut bg = XColor::default();
        let w = surface.width().max(0) as usize;
        let h = surface.height().max(0) as usize;
        let width_bytes = ((w + 7) & !7usize) / 8;

        let mut data_bits = vec![0u8; h * width_bytes];
        let mut mask_bits = vec![0u8; h * width_bytes];

        // Code below assumes ARGB pixel format
        crate::sdl_assert!(surface.format() == PixelFormat::ARGB8888);

        let (mut rfg, mut gfg, mut bfg, mut rbg, mut gbg, mut bbg, mut fg_bits, mut bg_bits) =
            (0u32, 0u32, 0u32, 0u32, 0u32, 0u32, 0u32, 0u32);
        let pixels = cursor_pixels(surface);
        let pitch = surface.pitch().max(0) as usize;
        for y in 0..h {
            for x in 0..w {
                let offset = y * pitch + x * 4;
                let Some(bytes) = pixels.get(offset..offset + 4) else {
                    continue;
                };
                let ptr = u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                let alpha = (ptr >> 24) & 0xff;
                let red = (ptr >> 16) & 0xff;
                let green = (ptr >> 8) & 0xff;
                let blue = ptr & 0xff;
                if alpha > 25 {
                    mask_bits[y * width_bytes + x / 8] |= 0x01 << (x % 8);

                    if (red + green + blue) > 0x40 {
                        fg_bits += 1;
                        rfg += red;
                        gfg += green;
                        bfg += blue;
                        data_bits[y * width_bytes + x / 8] |= 0x01 << (x % 8);
                    } else {
                        bg_bits += 1;
                        rbg += red;
                        gbg += green;
                        bbg += blue;
                    }
                }
            }
        }

        // (0 without foreground bits)
        fg.red = (rfg * 257).checked_div(fg_bits).unwrap_or(0) as u16;
        fg.green = (gfg * 257).checked_div(fg_bits).unwrap_or(0) as u16;
        fg.blue = (bfg * 257).checked_div(fg_bits).unwrap_or(0) as u16;

        // (0 without background bits)
        bg.red = (rbg * 257).checked_div(bg_bits).unwrap_or(0) as u16;
        bg.green = (gbg * 257).checked_div(bg_bits).unwrap_or(0) as u16;
        bg.blue = (bbg * 257).checked_div(bg_bits).unwrap_or(0) as u16;

        // SAFETY: the display is open; the bitmaps have h rows of
        // width_bytes bytes; the pixmaps are freed after use.
        unsafe {
            let data_pixmap = (self.x.XCreateBitmapFromData)(
                display,
                DefaultRootWindow(display),
                data_bits.as_ptr() as *const c_char,
                w as c_uint,
                h as c_uint,
            );
            let mask_pixmap = (self.x.XCreateBitmapFromData)(
                display,
                DefaultRootWindow(display),
                mask_bits.as_ptr() as *const c_char,
                w as c_uint,
                h as c_uint,
            );
            let cursor = (self.x.XCreatePixmapCursor)(
                display,
                data_pixmap,
                mask_pixmap,
                &mut fg,
                &mut bg,
                hot_x as c_uint,
                hot_y as c_uint,
            );
            (self.x.XFreePixmap)(display, data_pixmap);
            (self.x.XFreePixmap)(display, mask_pixmap);

            cursor
        }
    }

    /// Translation of `X11_CreateCursor()`.
    pub(crate) fn x11_create_cursor(
        &self,
        surface: &Surface<'_>,
        hot_x: i32,
        hot_y: i32,
    ) -> Result<Cursor> {
        let mut x11_cursor: XCursorId = None;

        if self.x.xcursor.is_some() {
            x11_cursor = self.x11_create_xcursor_cursor(surface, hot_x, hot_y);
        }
        if x11_cursor == None {
            x11_cursor = self.x11_create_pixmap_cursor(surface, hot_x, hot_y);
        }
        Ok(self.x11_create_cursor_and_data(x11_cursor))
    }

    /// Translation of `X11_CreateAnimatedCursor()`.
    pub(crate) fn x11_create_animated_cursor(
        &self,
        frames: &[CursorFrame<'_>],
        hot_x: i32,
        hot_y: i32,
    ) -> Result<Cursor> {
        let mut x11_cursor: XCursorId = None;

        if self.x.xcursor.is_some() {
            x11_cursor = self.x11_create_animated_xcursor_cursor(frames, hot_x, hot_y);
        }
        if x11_cursor == None {
            let Some(first) = frames.first() else {
                return Err(Error::invalid_param("frames"));
            };
            x11_cursor = self.x11_create_pixmap_cursor(first.surface, hot_x, hot_y);
        }

        Ok(self.x11_create_cursor_and_data(x11_cursor))
    }

    /// Translation of `X11_CreateSystemCursor()`.
    pub(crate) fn x11_create_system_cursor(&self, id: SystemCursor) -> Option<Cursor> {
        let dpy = self.display;
        let mut x11_cursor: XCursorId = None;

        if let Some(xcursor) = &self.x.xcursor {
            let name = CString::new(css_cursor_name(id).0).unwrap_or_default();
            // SAFETY: the display is open; the name is NUL-terminated.
            x11_cursor = unsafe { (xcursor.XcursorLibraryLoadCursor)(dpy, name.as_ptr()) };
        }

        if x11_cursor == None {
            // SAFETY: the display is open.
            x11_cursor =
                unsafe { (self.x.XCreateFontCursor)(dpy, get_legacy_system_cursor_shape(id)) };
        }

        if x11_cursor != None {
            return Some(self.x11_create_cursor_and_data(x11_cursor));
        }

        Option::None
    }

    /// Translation of `X11_CreateDefaultCursor()`.
    fn x11_create_default_cursor(&self) -> Option<Cursor> {
        let id = mouse::default_system_cursor();
        self.x11_create_system_cursor(id)
    }

    /// Translation of `X11_ShowCursor()`.
    pub(crate) fn x11_show_cursor(&self, cursor: Option<&Cursor>) -> Result<()> {
        let x11_cursor = match cursor {
            Some(cursor) => cursor.internal::<CursorData>().map_or(None, |d| d.cursor),
            Option::None => self.x11_create_empty_cursor(),
        };

        // FIXME: Is there a better way than this?
        {
            let display = self.display;

            self.with_data(|d| d.mouse.x11_cursor_visible = cursor.is_some());

            for window in window_ids() {
                if let Ok(xwindow) = with_x11_window(window, |_, data| data.xwindow) {
                    // SAFETY: the display is open and the window ours.
                    unsafe {
                        if x11_cursor != None {
                            (self.x.XDefineCursor)(display, xwindow, x11_cursor);
                        } else {
                            (self.x.XUndefineCursor)(display, xwindow);
                        }
                    }
                }
            }
            // SAFETY: the display is open.
            unsafe {
                (self.x.XFlush)(display);
            }
        }
        Ok(())
    }

    /// Translation of `X11_WarpMouseInternal()`.
    fn x11_warp_mouse_internal(&self, xwindow: Window, x: f32, y: f32) {
        let display = self.display;
        let mut warp_hack = false;

        // XWayland will only warp the cursor if it is hidden, so this workaround is required.
        if self.is_xwayland && self.with_data(|d| d.mouse.x11_cursor_visible) {
            warp_hack = true;
        }

        if warp_hack {
            let _ = self.x11_show_cursor(Option::None);
        }
        let mut deviceid: c_int = 0;
        if x11_xinput2_is_initialized() {
            /* It seems XIWarpPointer() doesn't work correctly on multi-head setups:
             * https://developer.blender.org/rB165caafb99c6846e53d11c4e966990aaffc06cea
             */
            // SAFETY: the display is open.
            if unsafe { ScreenCount(display) } == 1 {
                if let Some(xi) = &self.x.xinput2 {
                    // SAFETY: the display is open; deviceid is an out-parameter.
                    unsafe {
                        (xi.XIGetClientPointer)(display, None, &mut deviceid);
                    }
                }
            }
        }
        match (&self.x.xinput2, deviceid != 0) {
            (Some(xi), true) => {
                // SAFETY: the display is open; the window is ours or the root.
                unsafe {
                    (xi.XIWarpPointer)(
                        display, deviceid, None, xwindow, 0.0, 0.0, 0, 0, x as f64, y as f64,
                    );
                }
            }
            _ => {
                // SAFETY: as above.
                unsafe {
                    (self.x.XWarpPointer)(
                        display, None, xwindow, 0, 0, 0, 0, x as c_int, y as c_int,
                    );
                }
            }
        }

        if warp_hack {
            let _ = self.x11_show_cursor(mouse::cursor().as_ref());
        }
        // SAFETY: the display is open.
        unsafe {
            (self.x.XSync)(display, False);
        }
        self.with_data(|d| d.global_mouse_changed = true);
    }

    /// Translation of `X11_WarpMouse()`.
    pub(crate) fn x11_warp_mouse(&self, window: WindowID, x: f32, y: f32) -> Result<()> {
        if self.x11_warp_mouse_xtest(Some(window), x, y) {
            return Ok(());
        }

        // If we have no barrier, we need to warp
        let (xwindow, barrier_active) = with_x11_window(window, |_, data| {
            (data.xwindow, data.pointer_barrier_active)
        })?;
        if !barrier_active {
            self.x11_warp_mouse_internal(xwindow, x, y);
        }
        Ok(())
    }

    /// Translation of `X11_WarpMouseGlobal()`.
    pub(crate) fn x11_warp_mouse_global(&self, x: f32, y: f32) -> Result<()> {
        if self.x11_warp_mouse_xtest(Option::None, x, y) {
            return Ok(());
        }

        // SAFETY: the display is open.
        let root = unsafe { DefaultRootWindow(self.display) };
        self.x11_warp_mouse_internal(root, x, y);
        Ok(())
    }

    /// Translation of `X11_SetRelativeMouseMode()`.
    pub(crate) fn x11_set_relative_mouse_mode(&self, _enabled: bool) -> Result<()> {
        if !x11_xinput2_is_initialized() {
            return Err(Error::unsupported());
        }
        Ok(())
    }

    /// Translation of `X11_CaptureMouse()`.
    pub(crate) fn x11_capture_mouse(&self, window: Option<WindowID>) -> Result<()> {
        let display = self.display;
        let mouse_focus = mouse::mouse_focus();

        if let Some(window) = window {
            let (xinput2_mouse_enabled, mouse_grabbed, xwindow) =
                with_x11_window(window, |_, data| {
                    (data.xinput2_mouse_enabled, data.mouse_grabbed, data.xwindow)
                })?;

            /* If XInput2 is handling the pointer input, non-confinement grabs will always fail with 'AlreadyGrabbed',
             * since the pointer is being grabbed by XInput2.
             */
            if !xinput2_mouse_enabled || mouse_grabbed {
                let mask =
                    (ButtonPressMask | ButtonReleaseMask | PointerMotionMask | FocusChangeMask)
                        as c_uint;
                let confined = if mouse_grabbed { xwindow } else { None };
                // SAFETY: the display is open and the window ours.
                let rc = unsafe {
                    (self.x.XGrabPointer)(
                        display,
                        xwindow,
                        False,
                        mask,
                        GrabModeAsync,
                        GrabModeAsync,
                        confined,
                        None,
                        CurrentTime,
                    )
                };
                if rc != GrabSuccess {
                    return Err(Error::new("X server refused mouse capture"));
                }

                if mouse_grabbed {
                    // XGrabPointer can warp the cursor when confining, so update the coordinates.
                    self.with_data(|d| d.global_mouse_changed = true);
                }
            }
        } else if let Some(mouse_focus) = mouse_focus {
            update_window_grab(mouse_focus);
        } else {
            // SAFETY: the display is open.
            unsafe {
                (self.x.XUngrabPointer)(display, CurrentTime);
            }
        }

        // SAFETY: the display is open.
        unsafe {
            (self.x.XSync)(display, False);
        }

        Ok(())
    }

    /// Translation of `X11_GetGlobalMouseState()`.
    pub(crate) fn x11_get_global_mouse_state(&self) -> (f32, f32, MouseButtonFlags) {
        let display = self.display;

        // !!! FIXME: should we XSync() here first?

        if !x11_xinput2_is_initialized() {
            self.with_data(|d| d.global_mouse_changed = true);
        }

        // check if we have this cached since XInput last saw the mouse move.
        // !!! FIXME: can we just calculate this from XInput's events?
        if self.with_data(|d| d.global_mouse_changed) {
            if let Ok(displays) = crate::video::display::displays() {
                for display_id in displays {
                    let Some(data) = display_driver_data(display_id) else {
                        continue;
                    };
                    let mut root: Window = 0;
                    let mut child: Window = 0;
                    let mut rootx: c_int = 0;
                    let mut rooty: c_int = 0;
                    let mut winx: c_int = 0;
                    let mut winy: c_int = 0;
                    let mut mask: c_uint = 0;
                    // SAFETY: the display is open; the out-parameters are valid.
                    let ok = unsafe {
                        (self.x.XQueryPointer)(
                            display,
                            RootWindow(display, data.screen),
                            &mut root,
                            &mut child,
                            &mut rootx,
                            &mut rooty,
                            &mut winx,
                            &mut winy,
                            &mut mask,
                        )
                    } != 0;
                    if ok {
                        let mut buttons = MouseButtonFlags::NONE;
                        if mask & Button1Mask as c_uint != 0 {
                            buttons |= MouseButtonFlags::LMASK;
                        }
                        if mask & Button2Mask as c_uint != 0 {
                            buttons |= MouseButtonFlags::MMASK;
                        }
                        if mask & Button3Mask as c_uint != 0 {
                            buttons |= MouseButtonFlags::RMASK;
                        }
                        // Use the SDL state for the extended buttons - it's better than nothing
                        let state = mouse::mouse_state().2;
                        buttons = MouseButtonFlags(
                            buttons.0
                                | (state.0
                                    & (MouseButtonFlags::X1MASK.0 | MouseButtonFlags::X2MASK.0)),
                        );
                        /* SDL_DisplayData->x,y point to screen origin, and adding them to mouse coordinates relative to root window doesn't do the right thing
                         * (observed on dual monitor setup with primary display being the rightmost one - mouse was offset to the right).
                         *
                         * Adding root position to root-relative coordinates seems to be a better way to get absolute position. */
                        // SAFETY: the display is open; root_attrs is an out-parameter.
                        let root_attrs = unsafe {
                            let mut root_attrs: XWindowAttributes = std::mem::zeroed();
                            (self.x.XGetWindowAttributes)(display, root, &mut root_attrs);
                            root_attrs
                        };
                        self.with_data(|d| {
                            d.global_mouse_position.x = root_attrs.x + rootx;
                            d.global_mouse_position.y = root_attrs.y + rooty;
                            d.global_mouse_buttons = buttons.0;
                            d.global_mouse_changed = false;
                        });
                        break;
                    }
                }
            }
        }

        let (changed, position, buttons) = self.with_data(|d| {
            (
                d.global_mouse_changed,
                d.global_mouse_position,
                d.global_mouse_buttons,
            )
        });
        crate::sdl_assert!(!changed); // The pointer wasn't on any X11 screen?!

        (
            position.x as f32,
            position.y as f32,
            MouseButtonFlags(buttons),
        )
    }

    /// Translation of `X11_InitMouse()` (the entry points are the
    /// `VideoDriver` implementation).
    pub(crate) fn x11_init_mouse(&self) {
        use SystemCursor as C;
        let mut sys_cursors: [Option<Cursor>; HitTestResult::ResizeLeft as usize + 1] =
            Default::default();
        for (r, slot) in sys_cursors.iter_mut().enumerate() {
            let id = match r {
                // SDL_HITTEST_NORMAL, SDL_HITTEST_DRAGGABLE
                0 | 1 => C::Default,
                2 => C::NwResize,
                3 => C::NResize,
                4 => C::NeResize,
                5 => C::EResize,
                6 => C::SeResize,
                7 => C::SResize,
                8 => C::SwResize,
                _ => C::WResize,
            };
            *slot = self.x11_create_system_cursor(id);
        }
        self.with_data(|d| d.mouse.sys_cursors = sys_cursors);

        mouse::set_default_cursor(self.x11_create_default_cursor());
    }

    /// Translation of `X11_QuitMouse()`.
    pub(crate) fn x11_quit_mouse(&self) {
        let sys_cursors = self.with_data(|d| std::mem::take(&mut d.mouse.sys_cursors));
        // (dropping the cursors frees them)
        drop(sys_cursors);

        self.x11_destroy_empty_cursor();
    }

    /// Translation of `X11_SetHitTestCursor()`.
    pub(crate) fn x11_set_hit_test_cursor(&self, rc: HitTestResult) {
        if rc == HitTestResult::Normal || rc == HitTestResult::Draggable {
            mouse::redraw_cursor();
        } else {
            let cursor = self.with_data(|d| d.mouse.sys_cursors[rc as usize].clone());
            let _ = self.x11_show_cursor(cursor.as_ref());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_shapes() {
        assert_eq!(
            get_legacy_system_cursor_shape(SystemCursor::Default),
            XC_left_ptr
        );
        assert_eq!(get_legacy_system_cursor_shape(SystemCursor::Text), XC_xterm);
        assert_eq!(
            get_legacy_system_cursor_shape(SystemCursor::NoDrop),
            XC_pirate
        );
    }
}
