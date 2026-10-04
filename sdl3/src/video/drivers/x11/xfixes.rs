// Rust translation of src/video/x11/SDL_x11xfixes.c and SDL_x11xfixes.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! XFixes: clipboard owner change notifications and pointer barriers (for
//! `Window::set_mouse_rect`).

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use super::sys::*;
use super::video::X11Video;
use super::window::with_x11_window;
use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::WindowID;
use crate::video::window::Window as SdlWindow;
use crate::video::Rect;

pub(crate) const X11_BARRIER_HANDLED_BY_EVENT: i32 = 1;

// FIXME (upstream): xfixes_initialized starts out true, so the barrier
// functions run even without XFixes (where upstream would call NULL
// function pointers; here they report "unsupported").
static XFIXES_INITIALIZED: AtomicBool = AtomicBool::new(true);
static XFIXES_SELECTION_NOTIFY_EVENT: AtomicI32 = AtomicI32::new(0);

impl X11Video {
    /// Translation of `query_xfixes_version()`.
    fn query_xfixes_version(&self, mut major: i32, mut minor: i32) -> i32 {
        if let Some(xfixes) = &self.x.xfixes {
            // We don't care if this fails, so long as it sets major/minor on it's way out the door.
            // SAFETY: the display is open; the out-parameters are valid.
            unsafe {
                (xfixes.XFixesQueryVersion)(self.display, &mut major, &mut minor);
            }
        }
        (major * 1000) + minor
    }

    /// Translation of `X11_InitXfixes()`.
    pub(crate) fn x11_init_xfixes(&self) {
        let mut event = 0;
        let mut error = 0;
        let mut fixes_opcode = 0;

        let xa_clipboard = self.atoms().CLIPBOARD;

        let Some(xfixes) = &self.x.xfixes else {
            return;
        };
        // SAFETY: the display is open; the out-parameters are valid.
        if unsafe {
            (self.x.XQueryExtension)(
                self.display,
                c"XFIXES".as_ptr(),
                &mut fixes_opcode,
                &mut event,
                &mut error,
            )
        } == 0
        {
            return;
        }

        // Selection tracking is available in all versions of XFixes
        XFIXES_SELECTION_NOTIFY_EVENT.store(event + XFixesSelectionNotify, Ordering::Relaxed);
        // SAFETY: the display is open.
        unsafe {
            (xfixes.XFixesSelectSelectionInput)(
                self.display,
                DefaultRootWindow(self.display),
                xa_clipboard,
                XFixesSetSelectionOwnerNotifyMask,
            );
            (xfixes.XFixesSelectSelectionInput)(
                self.display,
                DefaultRootWindow(self.display),
                XA_PRIMARY,
                XFixesSetSelectionOwnerNotifyMask,
            );
        }

        // We need at least 5.0 for barriers.
        let version = self.query_xfixes_version(5, 0);
        if !xfixes_version_atleast(version, 5, 0) {
            return; // X server does not support the version we want at all.
        }

        XFIXES_INITIALIZED.store(true, Ordering::Relaxed);
    }

    /// Translation of `X11_SetWindowMouseRect()`.
    pub(crate) fn x11_set_window_mouse_rect(&self, window: WindowID) -> Result<()> {
        let (mouse_rect, flags) = with_x11_window(window, |w, _| (w.core.mouse_rect, w.flags()))?;
        if mouse_rect.is_empty() {
            self.x11_confine_cursor_with_flags(window, Option::None, 0)?;
        } else if flags.contains(WindowFlags::INPUT_FOCUS) {
            self.x11_confine_cursor_with_flags(window, Some(&mouse_rect), 0)?;
        } else {
            // Save the state for when we get focus again
            with_x11_window(window, |_, wdata| {
                wdata.barrier_rect = mouse_rect;

                wdata.pointer_barrier_active = true;
            })?;
        }

        Ok(())
    }

    /// Translation of `X11_ConfineCursorWithFlags()`.
    pub(crate) fn x11_confine_cursor_with_flags(
        &self,
        window: WindowID,
        rect: Option<&Rect>,
        flags: i32,
    ) -> Result<()> {
        /* Yaakuro: For some reason Xfixes when confining inside a rect where the
         * edges exactly match, a rectangle the cursor 'slips' out of the barrier.
         * To prevent that the lines for the barriers will span the whole screen.
         */
        if !x11_xfixes_is_initialized() {
            return Err(Error::unsupported());
        }
        let Some(xfixes) = &self.x.xfixes else {
            return Err(Error::unsupported());
        };

        // If there is already a set of barriers active, disable them.
        if let Some(active) = self.with_data(|d| d.active_cursor_confined_window) {
            self.x11_destroy_pointer_barrier(Some(active));
        }

        /* If user did not specify an area to confine, destroy the barrier that was/is assigned to
         * this window it was assigned */
        if let Some(rect) = rect {
            let sdl_window = SdlWindow::from_raw(window);
            let (bx, by) = sdl_window.position()?;
            let (bw, bh) = sdl_window.size()?;

            // Negative values are not allowed. Clip values relative to the specified window.
            let x1 = bx + rect.x.max(0);
            let y1 = by + rect.y.max(0);
            let x2 = (bx + rect.x + rect.w).min(bx + bw);
            let y2 = (by + rect.y + rect.h).min(by + bh);

            // Use the display bounds to ensure the barriers don't have corner gaps
            let bounds = crate::video::window::display_for_window(window)
                .and_then(crate::video::display::display_bounds)
                .unwrap_or_default();

            let display = self.display;
            with_x11_window(window, |_, wdata| {
                if wdata.barrier_rect != *rect {
                    wdata.barrier_rect = *rect;
                }

                // SAFETY: the display is open and the window one of ours.
                unsafe {
                    // Create the left barrier
                    wdata.barrier[0] = (xfixes.XFixesCreatePointerBarrier)(
                        display,
                        wdata.xwindow,
                        x1,
                        bounds.y,
                        x1,
                        bounds.y + bounds.h,
                        BarrierPositiveX,
                        0,
                        std::ptr::null_mut(),
                    );
                    // Create the right barrier
                    wdata.barrier[1] = (xfixes.XFixesCreatePointerBarrier)(
                        display,
                        wdata.xwindow,
                        x2,
                        bounds.y,
                        x2,
                        bounds.y + bounds.h,
                        BarrierNegativeX,
                        0,
                        std::ptr::null_mut(),
                    );
                    // Create the top barrier
                    wdata.barrier[2] = (xfixes.XFixesCreatePointerBarrier)(
                        display,
                        wdata.xwindow,
                        bounds.x,
                        y1,
                        bounds.x + bounds.w,
                        y1,
                        BarrierPositiveY,
                        0,
                        std::ptr::null_mut(),
                    );
                    // Create the bottom barrier
                    wdata.barrier[3] = (xfixes.XFixesCreatePointerBarrier)(
                        display,
                        wdata.xwindow,
                        bounds.x,
                        y2,
                        bounds.x + bounds.w,
                        y2,
                        BarrierNegativeY,
                        0,
                        std::ptr::null_mut(),
                    );

                    (self.x.XFlush)(display);
                }

                /* User activated the confinement for this window. We use this later to reactivate
                 * the confinement if it got deactivated by FocusOut or UnmapNotify */
                wdata.pointer_barrier_active = true;
            })?;

            // Lets remember current active confined window.
            self.with_data(|d| d.active_cursor_confined_window = Some(window));
        } else {
            self.x11_destroy_pointer_barrier(Some(window));

            // Only set barrier inactive when user specified NULL and not handled by focus out.
            if flags != X11_BARRIER_HANDLED_BY_EVENT {
                with_x11_window(window, |_, wdata| wdata.pointer_barrier_active = false)?;
            }
        }
        Ok(())
    }

    /// Translation of `X11_DestroyPointerBarrier()`.
    pub(crate) fn x11_destroy_pointer_barrier(&self, window: Option<WindowID>) {
        if let (Some(window), Some(xfixes)) = (window, &self.x.xfixes) {
            let display = self.display;
            let _ = with_x11_window(window, |_, wdata| {
                for barrier in wdata.barrier.iter_mut() {
                    if *barrier > 0 {
                        // SAFETY: the display is open; the barrier is ours.
                        unsafe {
                            (xfixes.XFixesDestroyPointerBarrier)(display, *barrier);
                        }
                        *barrier = 0;
                    }
                }
            });
            // SAFETY: the display is open.
            unsafe {
                (self.x.XFlush)(display);
            }
        }
        self.with_data(|d| d.active_cursor_confined_window = Option::None);
    }
}

/// Translation of `xfixes_version_atleast()`.
fn xfixes_version_atleast(version: i32, wantmajor: i32, wantminor: i32) -> bool {
    version >= ((wantmajor * 1000) + wantminor)
}

/// Translation of `X11_XfixesIsInitialized()`.
pub(crate) fn x11_xfixes_is_initialized() -> bool {
    XFIXES_INITIALIZED.load(Ordering::Relaxed)
}

/// Translation of `X11_GetXFixesSelectionNotifyEvent()`.
pub(crate) fn x11_get_xfixes_selection_notify_event() -> i32 {
    XFIXES_SELECTION_NOTIFY_EVENT.load(Ordering::Relaxed)
}
