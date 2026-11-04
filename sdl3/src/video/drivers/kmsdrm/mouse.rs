// Rust translation of src/video/kmsdrm/SDL_kmsdrmmouse.c and
// SDL_kmsdrmmouse.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The cursor: on a GBM buffer object of the display's cursor size
//! (`DRM_CAP_CURSOR_WIDTH`/`HEIGHT`, premultiplied ARGB8888), shown with
//! `drmModeSetCursor()`/`drmModeSetCursor2()` and moved with
//! `drmModeMoveCursor()` (upstream's atomic cursor plane is turned off:
//! `USE_ATOMIC_CURSOR` is 0, and its code is translated for it), and the
//! warps. The relative mode is the evdev reader's (it sets the mouse's
//! hook), not the driver's.

use super::sys::*;
use super::video::{KmsdrmPlaneInfo, KmsdrmVideo, PlaneSel};
use crate::error::{Error, Result};
use crate::events::default_cursor::{
    DEFAULT_CDATA, DEFAULT_CHEIGHT, DEFAULT_CHOTX, DEFAULT_CHOTY, DEFAULT_CMASK, DEFAULT_CWIDTH,
};
use crate::events::mouse::{self, Cursor, GLOBAL_MOUSE_ID};
use crate::events::{DisplayID, WindowID};
use crate::video::display::displays;
use crate::video::window::display_for_window;
use crate::video::{PixelFormat, Surface};

// !!! FIXME: atomic cursors are broken right now.
const USE_ATOMIC_CURSOR: bool = false;

/// Translation of `MAX_CURSOR_W`.
#[allow(dead_code)] // (as upstream: defined, unused)
pub(crate) const MAX_CURSOR_W: i32 = 512;
/// Translation of `MAX_CURSOR_H`.
#[allow(dead_code)] // (as upstream: defined, unused)
pub(crate) const MAX_CURSOR_H: i32 = 512;

/// Translation of `struct SDL_CursorData`; `KMSDRM_FreeCursor()` is its
/// drop.
#[derive(Debug)]
pub(crate) struct CursorData {
    pub(crate) hot_x: i32,
    pub(crate) hot_y: i32,
    pub(crate) w: i32,
    pub(crate) h: i32,

    /* The buffer where we store the mouse bitmap ready to be used.
    We get it ready and filled in CreateCursor(), and copy it
    to a GBM BO in ShowCursor().*/
    pub(crate) buffer: Vec<u32>,
    #[allow(dead_code)] // (as upstream: kept, unused)
    pub(crate) buffer_size: usize,
    #[allow(dead_code)] // (as upstream: kept, unused)
    pub(crate) buffer_pitch: usize,
}

/**************************************************************************************/
// BEFORE CODING ANYTHING MOUSE/CURSOR RELATED, REMEMBER THIS.
// How does SDL manage cursors internally? First, mouse != cursor. The mouse can have
// many cursors in mouse->cursors.
// -SDL tells us to create a cursor with KMSDRM_CreateCursor(). It can create many
// cursors with this, not only one.
// -SDL stores those cursors in a cursors array, in mouse->cursors.
// -Whenever it wants (or the programmer wants) takes a cursor from that array
// and shows it on screen with KMSDRM_ShowCursor().
// KMSDRM_ShowCursor() simply shows or hides the cursor it receives: it does NOT
// mind if it's mouse->cur_cursor, etc.
// -If KMSDRM_ShowCursor() returns successfully, that cursor becomes
// mouse->cur_cursor and mouse->cursor_visible is 1.
/**************************************************************************************/

/// Translation of `KMSDRM_CreateDefaultCursor()`.
fn kmsdrm_create_default_cursor() -> Option<Cursor> {
    mouse::create_cursor(
        &DEFAULT_CDATA,
        &DEFAULT_CMASK,
        DEFAULT_CWIDTH,
        DEFAULT_CHEIGHT,
        DEFAULT_CHOTX,
        DEFAULT_CHOTY,
    )
    .ok()
}

/// This simply gets the cursor soft-buffer ready.
/// We don't copy it to a GBO BO until ShowCursor() because the cusor GBM BO (living
/// in dispata) is destroyed and recreated when we recreate windows, etc.
/// Translation of `KMSDRM_CreateCursor()`.
pub(crate) fn kmsdrm_create_cursor(
    surface: &Surface<'_>,
    hot_x: i32,
    hot_y: i32,
) -> Result<Cursor> {
    let (w, h) = (surface.width(), surface.height());

    /* Configure the cursor buffer info.
    This buffer has the original size of the cursor surface we are given. */
    let buffer_pitch = w as usize;
    let buffer_size = w as usize * h as usize * 4;
    let mut buffer = vec![0u32; w as usize * h as usize];

    /* All code below assumes ARGB8888 format for the cursor surface,
    like other backends do. Also, the GBM BO pixels have to be
    alpha-premultiplied, but the SDL surface we receive has
    straight-alpha pixels, so we always have to convert. */
    {
        // SAFETY: u32 has no padding; the bytes are the buffer's.
        let dst = unsafe {
            std::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast::<u8>(), buffer_size)
        };
        let src = surface.pixels().unwrap_or(&[]);
        // (upstream ignores the result)
        let _ = crate::video::surface::premultiply_alpha(
            w,
            h,
            surface.format(),
            src,
            surface.pitch(),
            PixelFormat::ARGB8888,
            dst,
            w * 4,
            true,
        );
    }

    // hox_x and hot_y are the coordinates of the "tip of the cursor" from it's base.
    Ok(Cursor::with_internal(CursorData {
        hot_x,
        hot_y,
        w,
        h,
        buffer,
        buffer_size,
        buffer_pitch,
    }))
}

impl KmsdrmVideo {
    /// Given a display's internal, destroy the cursor BO for it.
    /// To be called from KMSDRM_DestroyWindow(), as that's where we
    /// destroy the internal for the window's display.
    /// Translation of `KMSDRM_DestroyCursorBO()`.
    pub(crate) fn kmsdrm_destroy_cursor_bo(&self, display: DisplayID) {
        self.with_state(|s| {
            let Some((vid, dispdata)) = s.display_parts(display) else {
                return;
            };

            // Destroy the curso GBM BO.
            if !dispdata.cursor_bo.is_null() {
                if USE_ATOMIC_CURSOR && vid.is_atomic && dispdata.cursor_plane.is_some() {
                    // Unset the the cursor BO from the cursor plane.
                    let info = KmsdrmPlaneInfo::zeroed(PlaneSel::Cursor);
                    self.drm_atomic_set_plane_props(dispdata, &info);
                    // Wait until the cursor is unset from the cursor plane before destroying it's BO.
                    if self.drm_atomic_commit(vid, dispdata, true, false) != 0 {
                        // (SDL_SetError("Failed atomic commit in KMSDRM_DenitMouse."))
                    }
                    // Free the cursor plane, on which the cursor was being shown.
                    self.free_plane(&mut dispdata.cursor_plane);
                }

                // SAFETY: the buffer is GBM's, not used after this.
                unsafe { (self.syms.gbm.gbm_bo_destroy)(dispdata.cursor_bo) };
                dispdata.cursor_bo = std::ptr::null_mut();
                dispdata.cursor_bo_drm_fd = -1;
            }
        });
    }

    /// Given a display's internal, create the cursor BO for it.
    /// To be called from KMSDRM_CreateWindow(), as that's where we
    /// build a window and assign a display to it.
    /// Translation of `KMSDRM_CreateCursorBO()`.
    pub(crate) fn kmsdrm_create_cursor_bo(&self, display: DisplayID) -> Result<()> {
        let gbm = &self.syms.gbm;
        let drm = &self.syms.drm;
        self.with_state(|s| {
            let Some((vid, dispdata)) = s.display_parts(display) else {
                return Err(Error::new("Invalid display"));
            };

            if USE_ATOMIC_CURSOR && vid.is_atomic {
                if let Ok(plane) = self.setup_plane(vid, dispdata, DRM_PLANE_TYPE_CURSOR) {
                    dispdata.cursor_plane = Some(plane);
                }
            }

            // SAFETY: the device is GBM's; the descriptor is the card's;
            // out-parameters.
            unsafe {
                if (gbm.gbm_device_is_format_supported)(
                    vid.gbm_dev,
                    GBM_FORMAT_ARGB8888,
                    GBM_BO_USE_CURSOR | GBM_BO_USE_WRITE,
                ) == 0
                {
                    return Err(Error::new("Unsupported pixel format for cursor"));
                }

                if (drm.drmGetCap)(vid.drm_fd, DRM_CAP_CURSOR_WIDTH, &mut dispdata.cursor_w) != 0
                    || (drm.drmGetCap)(vid.drm_fd, DRM_CAP_CURSOR_HEIGHT, &mut dispdata.cursor_h)
                        != 0
                {
                    return Err(Error::new("Could not get the recommended GBM cursor size"));
                }

                if dispdata.cursor_w == 0 || dispdata.cursor_h == 0 {
                    return Err(Error::new("Could not get an usable GBM cursor size"));
                }

                dispdata.cursor_bo = (gbm.gbm_bo_create)(
                    vid.gbm_dev,
                    dispdata.cursor_w as u32,
                    dispdata.cursor_h as u32,
                    GBM_FORMAT_ARGB8888,
                    GBM_BO_USE_CURSOR | GBM_BO_USE_WRITE | GBM_BO_USE_LINEAR,
                );
            }

            if dispdata.cursor_bo.is_null() {
                return Err(Error::new("Could not create GBM cursor BO"));
            }

            dispdata.cursor_bo_drm_fd = vid.drm_fd;
            Ok(())
        })
    }

    /// Remove a cursor buffer from a display's DRM cursor BO. Translation
    /// of `KMSDRM_RemoveCursorFromBO()`.
    fn kmsdrm_remove_cursor_from_bo(&self, display: DisplayID) -> Result<()> {
        self.with_state(|s| {
            let Some((vid, dispdata)) = s.display_parts(display) else {
                return Ok(());
            };

            if USE_ATOMIC_CURSOR && vid.is_atomic {
                if dispdata.cursor_plane.is_some() {
                    let info = KmsdrmPlaneInfo::zeroed(PlaneSel::Cursor);
                    // The rest of the members are zeroed, so this takes away the cursor from the cursor plane.
                    self.drm_atomic_set_plane_props(dispdata, &info);
                    if self.drm_atomic_commit(vid, dispdata, true, false) != 0 {
                        return Err(Error::new("Failed atomic commit in KMSDRM_ShowCursor."));
                    }
                }
                Ok(())
            } else {
                // SAFETY: the descriptor is the card's; the CRTC the
                // display's.
                let rc = unsafe {
                    (self.syms.drm.drmModeSetCursor)(
                        vid.drm_fd,
                        (*dispdata.crtc.crtc).crtc_id,
                        0,
                        0,
                        0,
                    )
                };
                if rc < 0 {
                    return Err(Error::new(format!(
                        "drmModeSetCursor() failed: {}",
                        crate::core::linux::input::errno_string(-rc)
                    )));
                }
                Ok(())
            }
        })
    }

    /// Dump a cursor buffer to a display's DRM cursor BO. Translation of
    /// `KMSDRM_DumpCursorToBO()`.
    fn kmsdrm_dump_cursor_to_bo(
        &self,
        display: DisplayID,
        mouse_xy: (f32, f32),
        cursor: &Cursor,
    ) -> Result<()> {
        let gbm = &self.syms.gbm;
        let drm = &self.syms.drm;
        let curdata = cursor.internal::<CursorData>();
        self.with_state(|s| {
            let Some((vid, dispdata)) = s.display_parts(display) else {
                return Err(Error::new("Cursor or display not initialized properly."));
            };
            let Some(curdata) = curdata.filter(|_| !dispdata.cursor_bo.is_null()) else {
                return Err(Error::new("Cursor or display not initialized properly."));
            };

            /* Prepare a buffer we can dump to our GBM BO (different
            size, alpha premultiplication...) */
            // SAFETY: the buffer is GBM's.
            let bo_stride = unsafe { (gbm.gbm_bo_get_stride)(dispdata.cursor_bo) } as usize;
            let bufsize = bo_stride * dispdata.cursor_h as usize;

            let mut ready_buffer = vec![0u8; bufsize];

            // Copy from the cursor buffer to a buffer that we can dump to the GBM BO.
            // SAFETY: u32 has no padding; the bytes are the buffer's.
            let src = unsafe {
                std::slice::from_raw_parts(
                    curdata.buffer.as_ptr().cast::<u8>(),
                    curdata.buffer.len() * 4,
                )
            };
            let row = 4 * curdata.w as usize;
            for i in 0..curdata.h as usize {
                // Note (upstream): a cursor wider or taller than the BO
                // writes past the buffer there; the copy is cut here.
                let dst_start = i * bo_stride;
                if dst_start >= bufsize {
                    break;
                }
                let n = row.min(bufsize - dst_start);
                ready_buffer[dst_start..dst_start + n].copy_from_slice(&src[i * row..i * row + n]);
            }

            // Dump the cursor buffer to our GBM BO.
            // SAFETY: the buffer holds bufsize bytes.
            if unsafe {
                (gbm.gbm_bo_write)(dispdata.cursor_bo, ready_buffer.as_ptr().cast(), bufsize)
            } != 0
            {
                return Err(Error::new("Could not write to GBM cursor BO"));
            }

            if USE_ATOMIC_CURSOR && vid.is_atomic {
                // Get the fb_id for the GBM BO so we can show it on the cursor plane.
                let Some(fb_id) = self.kmsdrm_fb_from_bo(vid, dispdata.cursor_bo) else {
                    return Err(Error::new("Failed to get cursor FB from BO"));
                };

                // Show the GBM BO buffer on the cursor plane.
                let mut info = KmsdrmPlaneInfo::zeroed(PlaneSel::Cursor);
                // SAFETY: the display's CRTC is libdrm's.
                info.crtc_id = unsafe { (*dispdata.crtc.crtc).crtc_id };
                info.fb_id = fb_id;
                info.src_w = dispdata.cursor_w as i32;
                info.src_h = dispdata.cursor_h as i32;
                info.crtc_x = mouse_xy.0.round() as i32 - curdata.hot_x;
                info.crtc_y = mouse_xy.1.round() as i32 - curdata.hot_y;
                info.crtc_w = curdata.w;
                info.crtc_h = curdata.h;
                self.drm_atomic_set_plane_props(dispdata, &info);
                if self.drm_atomic_commit(vid, dispdata, true, false) != 0 {
                    return Err(Error::new("Failed atomic commit in KMSDRM_ShowCursor."));
                }
            } else {
                // Put the GBM BO buffer on screen using the DRM interface.
                // SAFETY: the buffer is GBM's; the descriptor is the card's;
                // the CRTC the display's.
                let rc = unsafe {
                    let bo_handle = (gbm.gbm_bo_get_handle)(dispdata.cursor_bo).u32_;
                    let crtc_id = (*dispdata.crtc.crtc).crtc_id;
                    if curdata.hot_x == 0 && curdata.hot_y == 0 {
                        (drm.drmModeSetCursor)(
                            vid.drm_fd,
                            crtc_id,
                            bo_handle,
                            dispdata.cursor_w as u32,
                            dispdata.cursor_h as u32,
                        )
                    } else {
                        (drm.drmModeSetCursor2)(
                            vid.drm_fd,
                            crtc_id,
                            bo_handle,
                            dispdata.cursor_w as u32,
                            dispdata.cursor_h as u32,
                            curdata.hot_x,
                            curdata.hot_y,
                        )
                    }
                };
                if rc < 0 {
                    return Err(Error::new(format!(
                        "Failed to set DRM cursor: {}",
                        crate::core::linux::input::errno_string(-rc)
                    )));
                }
            }

            Ok(())
        })
    }

    /// Show the specified cursor, or hide if cursor is NULL or has no
    /// focus. Translation of `KMSDRM_ShowCursor()`.
    pub(crate) fn kmsdrm_show_cursor(&self, cursor: Option<&Cursor>) -> Result<()> {
        let mut result = Ok(());

        // Get the mouse focused window, if any.
        let window = mouse::mouse_focus();

        match (window, cursor) {
            (Some(window), Some(cursor)) => {
                if let Ok(display) = display_for_window(window) {
                    /* Dump the cursor to the display DRM cursor BO so it becomes visible
                    on that display. */
                    result = self.kmsdrm_dump_cursor_to_bo(display, mouse::position(), cursor);
                }
            }
            _ => {
                /* If no window is focused by mouse or cursor is NULL,
                since we have no window (no mouse->focus) and hence
                we have no display, we simply hide mouse on all displays.
                This happens on video quit, where we get here after
                the mouse focus has been unset, yet SDL wants to
                restore the system default cursor (makes no sense here). */
                // (upstream's loop is `for (i = 0; i < displays[i]; i++)`,
                // which, the IDs being above their indices, is every display)
                for display in displays().unwrap_or_default() {
                    // Iterate on the displays, hiding the cursor.
                    result = self.kmsdrm_remove_cursor_from_bo(display);
                }
            }
        }

        result
    }

    /// Translation of `drm_atomic_movecursor()`.
    fn drm_atomic_movecursor(&self, display: DisplayID, curdata: &CursorData, x: u16, y: u16) {
        self.with_state(|s| {
            let Some(dispdata) = s.display(display) else {
                return;
            };
            if let Some(plane) = dispdata.cursor_plane.as_deref() {
                // We can't move a non-existing cursor, but that's ok.
                // Do we have a set of changes already in the making? If not, allocate a new one.
                if dispdata.atomic_req.is_null() {
                    if let Some(atomic) = &self.syms.atomic {
                        // SAFETY: drmModeAtomicAlloc has no preconditions.
                        dispdata.atomic_req = unsafe { (atomic.drmModeAtomicAlloc)() };
                    }
                }
                let req = dispdata.atomic_req;
                self.add_plane_property(
                    req,
                    plane,
                    "CRTC_X",
                    (x as i32 - curdata.hot_x) as i64 as u64,
                );
                self.add_plane_property(
                    req,
                    plane,
                    "CRTC_Y",
                    (y as i32 - curdata.hot_y) as i64 as u64,
                );
            }
        });
    }

    /// `drmModeMoveCursor()` on the display's cursor BO, if it has one.
    fn move_cursor_on(&self, display: DisplayID, x: f32, y: f32) -> Result<()> {
        self.with_state(|s| {
            let Some(dispdata) = s.display(display) else {
                return Err(Error::new("Cursor not initialized properly."));
            };
            if dispdata.cursor_bo.is_null() {
                return Err(Error::new("Cursor not initialized properly."));
            }
            // SAFETY: the descriptor is the cursor's card's; the CRTC the
            // display's.
            let rc = unsafe {
                (self.syms.drm.drmModeMoveCursor)(
                    dispdata.cursor_bo_drm_fd,
                    (*dispdata.crtc.crtc).crtc_id,
                    x as i32,
                    y as i32,
                )
            };
            if rc < 0 {
                return Err(Error::new(format!(
                    "drmModeMoveCursor() failed: {}",
                    crate::core::linux::input::errno_string(-rc)
                )));
            }
            Ok(())
        })
    }

    /// Translation of `KMSDRM_WarpMouseGlobal()`.
    pub(crate) fn kmsdrm_warp_mouse_global(&self, x: f32, y: f32) -> Result<()> {
        let (Some(cur_cursor), Some(focus)) = (mouse::cursor(), mouse::mouse_focus()) else {
            return Err(Error::new("No mouse or current cursor."));
        };
        let display = display_for_window(focus)?;

        // Update internal mouse position.
        mouse::send_mouse_motion(
            std::time::Duration::ZERO,
            Some(focus),
            GLOBAL_MOUSE_ID,
            false,
            x,
            y,
        );

        // And now update the cursor graphic position on screen.
        let has_bo =
            self.with_state(|s| s.display(display).is_some_and(|d| !d.cursor_bo.is_null()));
        if has_bo {
            if USE_ATOMIC_CURSOR && self.with_state(|s| s.vid.is_atomic) {
                if let Some(curdata) = cur_cursor.internal::<CursorData>() {
                    self.drm_atomic_movecursor(display, curdata, x as i32 as u16, y as i32 as u16);
                }
            } else {
                self.move_cursor_on(display, x, y)?;
            }
        } else {
            return Err(Error::new("Cursor not initialized properly."));
        }

        Ok(())
    }

    /// Translation of `KMSDRM_WarpMouse()`.
    pub(crate) fn kmsdrm_warp_mouse(&self, _window: WindowID, x: f32, y: f32) -> Result<()> {
        // Only one global/fullscreen window is supported
        self.kmsdrm_warp_mouse_global(x, y)
    }

    /// Translation of `KMSDRM_InitMouse()` (the mouse's entry points are
    /// the driver's from now on: [`mouse_init`](super::video::VideoData)).
    pub(crate) fn kmsdrm_init_mouse(&self, display: DisplayID) {
        let first = self.with_state(|s| {
            s.vid.mouse_init = true;

            /* Only create the default cursor for this display if we haven't done so before,
            we don't want several cursors to be created for the same display. */
            match s.display(display) {
                Some(d) if !d.default_cursor_init => {
                    d.default_cursor_init = true;
                    true
                }
                _ => false,
            }
        });
        if first {
            mouse::set_default_cursor(kmsdrm_create_default_cursor());
        }
    }

    /// Translation of `KMSDRM_QuitMouse()`.
    #[allow(dead_code)] // (as upstream: declared, never called)
    pub(crate) fn kmsdrm_quit_mouse(&self) {
        // TODO: ?
    }

    /// This is called when a mouse motion event occurs. Translation of
    /// `KMSDRM_MoveCursor()`.
    pub(crate) fn kmsdrm_move_cursor(&self, _cursor: &Cursor) -> Result<()> {
        /* We must NOT call SDL_SendMouseMotion() here or we will enter recursivity!
        That's why we move the cursor graphic ONLY. */
        if let (Some(cur_cursor), Some(window)) = (mouse::cursor(), mouse::mouse_focus()) {
            let display = display_for_window(window)?;
            let (x, y) = mouse::position();

            if USE_ATOMIC_CURSOR && self.with_state(|s| s.vid.is_atomic) {
                /* !!! FIXME: Some programs expect cursor movement even while they don't do SwapWindow() calls,
                and since we ride on the atomic_commit() in SwapWindow() for cursor movement,
                cursor won't move in these situations. We could do an atomic_commit() here
                for each cursor movement request, but it cripples the movement to 30FPS,
                so a future solution is needed. SDLPoP "QUIT?" menu is an example of this
                situation. */
                if let Some(curdata) = cur_cursor.internal::<CursorData>() {
                    self.drm_atomic_movecursor(display, curdata, x as i32 as u16, y as i32 as u16);
                }
            } else {
                self.move_cursor_on(display, x, y)?;
            }
        }
        Ok(())
    }
}
