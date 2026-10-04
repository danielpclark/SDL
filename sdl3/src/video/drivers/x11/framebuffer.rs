// Rust translation of src/video/x11/SDL_x11framebuffer.c and
// SDL_x11framebuffer.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Window framebuffers: an `XImage` in a MIT-SHM shared memory segment when
//! the server is local, or a client-side image sent with `XPutImage()`.
//!
//! The shared memory segment is a [`ShmSegment`], held by both the window
//! data and the framebuffer surface (whose pixels it is); it is detached
//! from this process when both are gone. Without shared memory the
//! surface owns its pixels and the image is pointed at them on each update.

use std::cell::UnsafeCell;
use std::ffi::{c_char, c_int, c_uint};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::modes::{x11_get_pixel_format_from_visual_info, x11_get_visual_info_from_visual};
use super::sys::*;
use super::video::X11Video;
use super::window::with_x11_window;
use crate::error::{Error, Result};
use crate::events::WindowID;
use crate::video::window::window_size_in_pixels;
use crate::video::{PixelFormat, Rect, Surface};

/// A shared memory segment attached to this process (`XShmSegmentInfo`
/// with the mapping), detached (`shmdt()`) when dropped.
pub(crate) struct ShmSegment {
    /// The segment info; Xlib keeps its address (in the image) and writes
    /// it (XShmAttach), hence the cell.
    info: UnsafeCell<XShmSegmentInfo>,
    len: usize,
}

// SAFETY: the info is only touched by Xlib calls made under the window's
// data (while the driver holds the window), and the mapping is plain memory
// reached through the framebuffer surface.
unsafe impl Send for ShmSegment {}
// SAFETY: as above.
unsafe impl Sync for ShmSegment {}

impl ShmSegment {
    /// The segment info, for Xlib.
    fn info(&self) -> *mut XShmSegmentInfo {
        self.info.get()
    }

    /// The start of the mapping.
    fn addr(&self) -> *mut c_char {
        // SAFETY: the address is set before the segment is shared and never
        // changes.
        unsafe { (*self.info.get()).shmaddr }
    }
}

impl Drop for ShmSegment {
    fn drop(&mut self) {
        // SAFETY: the mapping came from shmat() and is detached once, after
        // its last user (the window data and the surface) is gone.
        unsafe {
            libc::shmdt(self.addr() as *const libc::c_void);
        }
    }
}

// Shared memory error handler routine
static SHM_ERROR: AtomicBool = AtomicBool::new(false);
static X_HANDLER: Mutex<XErrorHandler> = Mutex::new(Option::None);

/// Translation of `shm_errhandler()`.
unsafe extern "C" fn shm_errhandler(d: *mut Display, e: *mut XErrorEvent) -> c_int {
    // SAFETY: Xlib passes a valid error event.
    let code = unsafe { (*e).error_code } as c_int;
    if code == BadAccess as c_int || code == BadRequest as c_int {
        SHM_ERROR.store(true, Ordering::Relaxed);
        return 0;
    }
    let handler = *X_HANDLER.lock().unwrap_or_else(|e| e.into_inner());
    match handler {
        // SAFETY: the previous handler, called as Xlib would.
        Some(h) => unsafe { h(d, e) },
        Option::None => 0,
    }
}

impl X11Video {
    /// Translation of `have_mitshm()`.
    fn have_mitshm(&self, dpy: *mut Display) -> bool {
        // Only use shared memory on local X servers
        match &self.x.shm {
            // SAFETY: the display is open.
            Some(shm) => unsafe { (shm.XShmQueryExtension)(dpy) != 0 },
            Option::None => false,
        }
    }

    /// Translation of `X11_CreateWindowFramebuffer()`: the surface to draw
    /// into.
    pub(crate) fn x11_create_window_framebuffer(
        &self,
        window: WindowID,
    ) -> Result<Surface<'static>> {
        let display = self.display;
        let x = &self.x;
        let (w, h) = window_size_in_pixels(window)?;

        // Free the old framebuffer surface
        self.x11_destroy_window_framebuffer(window);

        let (xwindow, visual) = with_x11_window(window, |_, data| (data.xwindow, data.visual))?;

        // Create the graphics context for drawing
        // SAFETY: XGCValues is plain data.
        let mut gcv: XGCValues = unsafe { std::mem::zeroed() };
        gcv.graphics_exposures = False;
        // SAFETY: the display is open and the window ours.
        let gc = unsafe { (x.XCreateGC)(display, xwindow, GCGraphicsExposures as _, &mut gcv) };
        if gc.is_null() {
            return Err(Error::new("Couldn't create graphics context"));
        }
        with_x11_window(window, |_, data| data.gc = gc)?;

        // Find out the pixel format and depth
        let Some(vinfo) = x11_get_visual_info_from_visual(x, display, visual) else {
            return Err(Error::new("Couldn't get window visual information"));
        };

        let format = x11_get_pixel_format_from_visual_info(x, display, &vinfo);
        if format == PixelFormat::UNKNOWN {
            return Err(Error::new("Unknown window pixel format"));
        }

        // Calculate pitch
        let pitch = ((w * format.bytes_per_pixel() as i32) + 3) & !3;
        let size = h as usize * pitch as usize;

        let byte_order = if cfg!(target_endian = "big") {
            MSBFirst
        } else {
            LSBFirst
        };

        // Create the actual image
        if let (true, Some(shm)) = (self.have_mitshm(display), &x.shm) {
            let mut info = XShmSegmentInfo {
                // SAFETY: plain System V shared memory calls.
                shmid: unsafe { libc::shmget(libc::IPC_PRIVATE, size, libc::IPC_CREAT | 0o777) },
                ..XShmSegmentInfo::default()
            };
            let mut segment: Option<Arc<ShmSegment>> = Option::None;
            if info.shmid >= 0 {
                // SAFETY: attaching the segment just created.
                info.shmaddr =
                    unsafe { libc::shmat(info.shmid, std::ptr::null(), 0) } as *mut c_char;
                info.readOnly = False;
                if info.shmaddr as isize != -1 {
                    let seg = Arc::new(ShmSegment {
                        info: UnsafeCell::new(info),
                        len: size,
                    });
                    SHM_ERROR.store(false, Ordering::Relaxed);
                    // SAFETY: the handler is a valid extern fn; the display
                    // is open; the segment info lives in the Arc (a stable
                    // address).
                    unsafe {
                        let previous = (x.XSetErrorHandler)(Some(shm_errhandler));
                        *X_HANDLER.lock().unwrap_or_else(|e| e.into_inner()) = previous;
                        (shm.XShmAttach)(display, seg.info());
                        (x.XSync)(display, False);
                        (x.XSetErrorHandler)(previous);
                    }
                    if !SHM_ERROR.load(Ordering::Relaxed) {
                        segment = Some(seg);
                    }
                    // (otherwise dropping the segment detaches it: shmdt())
                } else {
                    SHM_ERROR.store(true, Ordering::Relaxed);
                }
                // SAFETY: marking the segment for removal once detached.
                unsafe {
                    libc::shmctl(info.shmid, libc::IPC_RMID, std::ptr::null_mut());
                }
            } else {
                SHM_ERROR.store(true, Ordering::Relaxed);
            }
            if let (false, Some(seg)) = (SHM_ERROR.load(Ordering::Relaxed), segment) {
                // SAFETY: the display is open; the segment is attached and
                // has room for h rows of pitch bytes.
                let ximage = unsafe {
                    (shm.XShmCreateImage)(
                        display,
                        visual,
                        vinfo.depth as c_uint,
                        ZPixmap,
                        seg.addr(),
                        seg.info(),
                        w as c_uint,
                        h as c_uint,
                    )
                };
                if ximage.is_null() {
                    // SAFETY: the display is open; the segment is attached.
                    unsafe {
                        (shm.XShmDetach)(display, seg.info());
                        (x.XSync)(display, False);
                    }
                    // (dropping the segment detaches it: shmdt())
                } else {
                    // Done!
                    // SAFETY: the image was just created.
                    unsafe {
                        (*ximage).byte_order = byte_order;
                    }
                    let owner: Arc<dyn std::any::Any + Send + Sync> = seg.clone();
                    // SAFETY: the mapping is `len` bytes, alive while the
                    // segment (the owner) lives; the surface is the only Rust
                    // access to it.
                    let surface = unsafe {
                        Surface::from_external(
                            w,
                            h,
                            format,
                            seg.addr() as *mut u8,
                            seg.len,
                            pitch,
                            owner,
                        )
                    };
                    with_x11_window(window, |_, data| {
                        data.ximage = ximage;
                        data.use_mitshm = true;
                        data.shminfo = Some(seg);
                    })?;
                    return surface;
                }
            }
        }

        let mut surface = Surface::from_vec(w, h, format, vec![0u8; size], pitch)?;
        let pixels = surface
            .pixels_mut()
            .map_or(std::ptr::null_mut(), |p| p.as_mut_ptr());

        // SAFETY: the display is open; the pixels (the surface's, h rows of
        // pitch bytes) are attached to the image on each update and
        // detached before it is destroyed.
        let ximage = unsafe {
            (x.XCreateImage)(
                display,
                visual,
                vinfo.depth as c_uint,
                ZPixmap,
                0,
                pixels as *mut c_char,
                w as c_uint,
                h as c_uint,
                32,
                0,
            )
        };
        if ximage.is_null() {
            return Err(Error::new("Couldn't create XImage"));
        }
        // SAFETY: the image was just created.
        unsafe {
            (*ximage).byte_order = byte_order;
            // (the pixels belong to the surface; they are set before each
            // update)
            (*ximage).data = std::ptr::null_mut();
        }
        with_x11_window(window, |_, data| data.ximage = ximage)?;
        Ok(surface)
    }

    /// Translation of `X11_UpdateWindowFramebuffer()`.
    pub(crate) fn x11_update_window_framebuffer(
        &self,
        window: WindowID,
        surface: &Surface<'static>,
        rects: &[Rect],
    ) -> Result<()> {
        let display = self.display;
        let x = &self.x;

        let (window_w, window_h) = window_size_in_pixels(window)?;
        let (xwindow, gc, ximage, use_mitshm) = with_x11_window(window, |_, data| {
            (data.xwindow, data.gc, data.ximage, data.use_mitshm)
        })?;
        if ximage.is_null() {
            return Err(Error::new("Couldn't update window framebuffer"));
        }

        /// The rectangle clipped to the window (`None`: clipped away).
        fn clip(rect: &Rect, window_w: i32, window_h: i32) -> Option<(c_int, c_int, c_int, c_int)> {
            let mut x = rect.x;
            let mut y = rect.y;
            let mut w = rect.w;
            let mut h = rect.h;

            if w <= 0 || h <= 0 || (x + w) <= 0 || (y + h) <= 0 {
                // Clipped?
                return Option::None;
            }
            // FIXME (upstream): a rectangle starting left of (above) the
            // window is moved to its right (bottom) edge, x += w, instead of
            // being clipped to 0.
            if x < 0 {
                x += w;
                w += rect.x;
            }
            if y < 0 {
                y += h;
                h += rect.y;
            }
            if x + w > window_w {
                w = window_w - x;
            }
            if y + h > window_h {
                h = window_h - y;
            }
            Some((x, y, w, h))
        }

        if let (true, Some(shm)) = (use_mitshm, &x.shm) {
            for rect in rects {
                let Some((rx, ry, rw, rh)) = clip(rect, window_w, window_h) else {
                    continue;
                };

                // SAFETY: the display is open; the image is the window's
                // shared memory image.
                unsafe {
                    (shm.XShmPutImage)(
                        display,
                        xwindow,
                        gc,
                        ximage,
                        rx,
                        ry,
                        rx,
                        ry,
                        rw as c_uint,
                        rh as c_uint,
                        False,
                    );
                }
            }
        } else {
            // (point the image at the surface's pixels)
            let Some(pixels) = surface.pixels() else {
                return Err(Error::new("Couldn't update window framebuffer"));
            };
            // SAFETY: the image is the window's; the pixels have the
            // image's size (the surface was created for it) and outlive the
            // XPutImage() calls below, after which they are detached again.
            unsafe {
                if (pixels.len() as c_int) < (*ximage).bytes_per_line * (*ximage).height {
                    return Err(Error::new("Couldn't update window framebuffer"));
                }
                (*ximage).data = pixels.as_ptr() as *mut c_char;
            }
            for rect in rects {
                let Some((rx, ry, rw, rh)) = clip(rect, window_w, window_h) else {
                    continue;
                };

                // SAFETY: as above.
                unsafe {
                    (x.XPutImage)(
                        display,
                        xwindow,
                        gc,
                        ximage,
                        rx,
                        ry,
                        rx,
                        ry,
                        rw as c_uint,
                        rh as c_uint,
                    );
                }
            }
            // SAFETY: as above.
            unsafe {
                (*ximage).data = std::ptr::null_mut();
            }
        }

        self.x11_handle_present(window);

        // SAFETY: the display is open.
        unsafe {
            (x.XSync)(display, False);
        }

        Ok(())
    }

    /// Translation of `X11_DestroyWindowFramebuffer()`.
    pub(crate) fn x11_destroy_window_framebuffer(&self, window: WindowID) {
        let display = self.display;
        let x = &self.x;

        // (no window data: the window wasn't fully initialized)
        let _ = with_x11_window(window, |_, data| {
            if !data.ximage.is_null() {
                // SAFETY: the image is ours; without shared memory its data
                // pointer is NULL (the pixels are the surface's), so only the
                // image is freed.
                unsafe {
                    XDestroyImage(data.ximage);
                }

                if data.use_mitshm {
                    if let (Some(shm), Some(seg)) = (&x.shm, &data.shminfo) {
                        // SAFETY: the display is open; the segment is attached.
                        unsafe {
                            (shm.XShmDetach)(display, seg.info());
                            (x.XSync)(display, False);
                        }
                    }
                    // (shmdt() happens when the surface is gone too)
                    data.shminfo = Option::None;
                    data.use_mitshm = false;
                }

                data.ximage = std::ptr::null_mut();
            }
            if !data.gc.is_null() {
                // SAFETY: the display is open; the GC is ours.
                unsafe {
                    (x.XFreeGC)(display, data.gc);
                }
                data.gc = std::ptr::null_mut();
            }
        });
    }
}
