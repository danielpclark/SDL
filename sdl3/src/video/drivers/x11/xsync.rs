// Rust translation of src/video/x11/SDL_x11xsync.c and SDL_x11xsync.h from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The SYNC extension: `_NET_WM_SYNC_REQUEST` resize synchronization with
//! the window manager (enabled with `SDL_VIDEO_X11_ENABLE_XSYNC_EXT`).

use std::ffi::c_uchar;
use std::sync::atomic::{AtomicBool, Ordering};

use super::sys::*;
use super::video::X11Video;
use super::window::with_x11_window;
use crate::error::{Error, Result};
use crate::events::WindowID;

static XSYNC_INITIALIZED: AtomicBool = AtomicBool::new(false);

impl X11Video {
    /// Translation of `query_xsync_version()`.
    fn query_xsync_version(&self, mut major: i32, mut minor: i32) -> i32 {
        if let Some(xsync) = &self.x.xsync {
            // We don't care if this fails, so long as it sets major/minor on it's way out the door.
            // SAFETY: the display is open; the out-parameters are valid.
            unsafe {
                (xsync.XSyncInitialize)(self.display, &mut major, &mut minor);
            }
        }
        (major * 1000) + minor
    }

    /// Translation of `X11_InitXsync()`.
    pub(crate) fn x11_init_xsync(&self) {
        let mut event = 0;
        let mut error = 0;
        let mut sync_opcode = 0;

        // SAFETY: the display is open; the out-parameters are valid.
        if self.x.xsync.is_none()
            || unsafe {
                (self.x.XQueryExtension)(
                    self.display,
                    c"SYNC".as_ptr(),
                    &mut sync_opcode,
                    &mut event,
                    &mut error,
                )
            } == 0
        {
            return;
        }

        // We need at least 5.0 for barriers.
        let version = self.query_xsync_version(5, 0);
        if !xsync_version_atleast(version, 3, 0) {
            return; // X server does not support the version we want at all.
        }

        XSYNC_INITIALIZED.store(true, Ordering::Relaxed);
    }

    /// Translation of `X11_InitResizeSync()`.
    pub(crate) fn x11_init_resize_sync(&self, window: WindowID) -> Result<()> {
        let counter_prop = self.atoms()._NET_WM_SYNC_REQUEST_COUNTER;

        if !x11_xsync_is_initialized() {
            return Err(Error::unsupported());
        }
        let Some(xsync) = &self.x.xsync else {
            return Err(Error::unsupported());
        };

        let display = self.display;
        with_x11_window(window, |_, data| {
            // SAFETY: the display is open.
            let counter =
                unsafe { (xsync.XSyncCreateCounter)(display, XSyncValue { hi: 0, lo: 0 }) };
            data.resize_counter = counter;
            data.resize_id.lo = 0;
            data.resize_id.hi = 0;
            data.resize_in_progress = false;

            if counter == None {
                return Err(Error::unsupported());
            }

            let counter_id: u32 = counter as u32;
            // FIXME (upstream): the property is set from a 4-byte CARD32,
            // but Xlib reads format-32 data as longs (8 bytes on LP64);
            // a long is passed here.
            let counter_long = counter_id as std::ffi::c_long;
            // SAFETY: the display is open and the window ours; format-32
            // property data is passed as longs.
            unsafe {
                (self.x.XChangeProperty)(
                    display,
                    data.xwindow,
                    counter_prop,
                    XA_CARDINAL,
                    32,
                    PropModeReplace,
                    &counter_long as *const std::ffi::c_long as *const c_uchar,
                    1,
                );
            }

            Ok(())
        })?
    }

    /// Translation of `X11_TermResizeSync()`.
    pub(crate) fn x11_term_resize_sync(&self, window: WindowID) {
        let counter_prop = self.atoms()._NET_WM_SYNC_REQUEST_COUNTER;
        let display = self.display;
        let _ = with_x11_window(window, |_, data| {
            let counter = data.resize_counter;

            // SAFETY: the display is open and the window ours.
            unsafe {
                (self.x.XDeleteProperty)(display, data.xwindow, counter_prop);
                if counter != None {
                    if let Some(xsync) = &self.x.xsync {
                        (xsync.XSyncDestroyCounter)(display, counter);
                    }
                }
            }
        });
    }

    /// Translation of `X11_HandleSyncRequest()`.
    pub(crate) fn x11_handle_sync_request(&self, window: WindowID, event: &XClientMessageEvent) {
        let _ = with_x11_window(window, |_, data| {
            data.resize_id.lo = event.data.l[2] as u32;
            data.resize_id.hi = event.data.l[3] as i32;
            data.resize_in_progress = false;
        });
    }

    /// Translation of `X11_HandleConfigure()`.
    pub(crate) fn x11_handle_configure(&self, window: WindowID, _event: &XConfigureEvent) {
        let _ = with_x11_window(window, |_, data| {
            if data.resize_id.lo != 0 || data.resize_id.hi != 0 {
                data.resize_in_progress = true;
            }
        });
    }

    /// Translation of `X11_HandlePresent()`.
    pub(crate) fn x11_handle_present(&self, window: WindowID) {
        let display = self.display;
        let _ = with_x11_window(window, |_, data| {
            let counter = data.resize_counter;

            if counter == None || !data.resize_in_progress {
                return;
            }

            if let Some(xsync) = &self.x.xsync {
                // SAFETY: the display is open and the counter ours.
                unsafe {
                    (xsync.XSyncSetCounter)(display, counter, data.resize_id);
                }
            }

            data.resize_id.lo = 0;
            data.resize_id.hi = 0;
            data.resize_in_progress = false;
        });
    }
}

/// Translation of `xsync_version_atleast()`.
fn xsync_version_atleast(version: i32, wantmajor: i32, wantminor: i32) -> bool {
    version >= ((wantmajor * 1000) + wantminor)
}

/// Translation of `X11_XsyncIsInitialized()`.
pub(crate) fn x11_xsync_is_initialized() -> bool {
    XSYNC_INITIALIZED.load(Ordering::Relaxed)
}
