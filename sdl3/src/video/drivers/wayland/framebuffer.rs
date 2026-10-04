// The window framebuffer of the Wayland video driver, in wl_shm buffers.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Window framebuffers (`SDL_GetWindowSurface()`) in `wl_shm` buffers.
//!
//! Upstream has no Wayland framebuffer: the video core draws the window
//! surface through a GLES texture (EGL), which isn't translated. This one
//! copies the window surface into a `wl_shm` buffer, attaches it and
//! commits; up to [`MAX_BUFFERS`] buffers are kept, each reused once the
//! compositor releases it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::client::{AsProxy, Proxy};
use super::protocols::wayland::*;
use super::shmbuffer::ShmPool;
use super::video::{VideoData, WaylandVideo};
use super::window::{ShellSurfaceStatus, WaylandWindowData};
use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::WindowID;
use crate::video::core::with_window;
use crate::video::{PixelFormat, Rect, Surface};

/// The number of buffers kept for reuse.
const MAX_BUFFERS: usize = 3;

/// A `wl_shm` buffer and its own pool.
struct FramebufferBuffer {
    /// Declared first, so that the buffer goes before its pool.
    buffer: Proxy<WlBuffer>,
    pool: ShmPool,
    /// Attached and not yet released by the compositor.
    busy: Arc<AtomicBool>,
}

/// The framebuffer of a window.
pub(crate) struct Framebuffer {
    format: PixelFormat,
    width: i32,
    height: i32,
    buffers: Vec<FramebufferBuffer>,
}

impl std::fmt::Debug for Framebuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Framebuffer")
            .field("format", &self.format)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("buffers", &self.buffers.len())
            .finish()
    }
}

/// The `wl_shm` format of a framebuffer format.
fn shm_format(format: PixelFormat) -> WlShmFormat {
    if format == PixelFormat::ARGB8888 {
        WlShmFormat::ARGB8888
    } else {
        WlShmFormat::XRGB8888
    }
}

/// Copy `height` rows of `row_len` bytes from `src` (rows `src_pitch` bytes
/// apart) to the packed `dst`.
fn copy_rows(dst: &mut [u8], src: &[u8], src_pitch: usize, row_len: usize, height: usize) {
    for (row, dst_row) in dst.chunks_exact_mut(row_len).take(height).enumerate() {
        let start = row * src_pitch;
        if let Some(src_row) = src.get(start..start + row_len) {
            dst_row.copy_from_slice(src_row);
        }
    }
}

impl WaylandVideo {
    /// Create the framebuffer of a window: the surface the app draws on.
    pub(crate) fn wayland_create_window_framebuffer(
        &self,
        window: WindowID,
        w: i32,
        h: i32,
    ) -> Result<Surface<'static>> {
        let flags = with_window(window, |w| w.flags())?;
        let format = if flags.contains(WindowFlags::TRANSPARENT) {
            PixelFormat::ARGB8888
        } else {
            PixelFormat::XRGB8888
        };
        let (w, h) = (w.max(1), h.max(1));
        let surface = Surface::new(w, h, format)?;

        self.with_data(|d| {
            let wd = d
                .window_mut(window)
                .ok_or_else(|| Error::new("Invalid window"))?;
            wd.framebuffer = Some(Framebuffer {
                format,
                width: w,
                height: h,
                buffers: Vec::new(),
            });
            Ok(surface)
        })
    }

    /// Show the framebuffer: copy it into a free buffer, attach it, damage
    /// `rects` (all of it when empty) and commit.
    pub(crate) fn wayland_update_window_framebuffer(
        &self,
        window: WindowID,
        surface: &Surface<'static>,
        rects: &[Rect],
    ) -> Result<()> {
        let pixels = surface
            .pixels()
            .ok_or_else(|| Error::new("The window surface has no pixels"))?;
        let pitch = surface.pitch() as usize;

        self.with_data(|d| {
            let VideoData { g, windows, .. } = d;
            let wd = windows
                .iter_mut()
                .find(|w| w.sdlwindow == window)
                .ok_or_else(|| Error::new("Invalid window"))?;
            let can_commit = wd.shell_surface_status != ShellSurfaceStatus::Hidden
                && wd.shell_surface_status != ShellSurfaceStatus::WaitingForConfigure
                && wd.shell_surface_status != ShellSurfaceStatus::ShowPending;
            let WaylandWindowData {
                surface: window_surface,
                framebuffer,
                ..
            } = wd;
            let wl_surface = window_surface.obj();
            let fb = framebuffer
                .as_mut()
                .ok_or_else(|| Error::new("The window has no framebuffer"))?;
            if fb.width != surface.width() || fb.height != surface.height() {
                return Err(Error::new(
                    "The window surface doesn't match the framebuffer",
                ));
            }
            let (width, height) = (fb.width, fb.height);
            let row_len = width as usize * 4;

            let index = match fb
                .buffers
                .iter()
                .position(|b| !b.busy.load(Ordering::Acquire))
            {
                Some(i) => i,
                None => {
                    let shm = g
                        .shm
                        .as_ref()
                        .ok_or_else(|| Error::new("The compositor has no wl_shm"))?;
                    let mut pool = ShmPool::alloc(shm.obj(), width * height * 4)?;
                    let (mut buffer, _) = pool
                        .alloc_buffer_with_format(width, height, shm_format(fb.format))
                        .ok_or_else(|| Error::new("Couldn't allocate a framebuffer buffer"))?;
                    let busy = Arc::new(AtomicBool::new(false));
                    let released = busy.clone();
                    buffer.listen(move |_, WlBufferEvent::Release| {
                        released.store(false, Ordering::Release);
                    });
                    if fb.buffers.len() >= MAX_BUFFERS {
                        // (all of them are in use: keep the new one instead of the oldest)
                        fb.buffers.remove(0);
                    }
                    fb.buffers.push(FramebufferBuffer { buffer, pool, busy });
                    fb.buffers.len() - 1
                }
            };

            let b = &mut fb.buffers[index];
            copy_rows(b.pool.memory_mut(), pixels, pitch, row_len, height as usize);

            if !can_commit {
                // (attaching a buffer before the first configure is a protocol error)
                return Ok(());
            }

            b.busy.store(true, Ordering::Release);
            wl_surface.attach(Some(b.buffer.obj()), 0, 0);
            if wl_surface.version() >= WlSurface::DAMAGE_BUFFER_SINCE_VERSION {
                if rects.is_empty() {
                    wl_surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
                }
                for r in rects {
                    wl_surface.damage_buffer(r.x, r.y, r.w, r.h);
                }
            } else {
                wl_surface.damage(0, 0, i32::MAX, i32::MAX);
            }
            wl_surface.commit();
            Ok(())
        })?;

        let _ = self.conn.flush();
        Ok(())
    }

    /// Destroy the framebuffer of a window.
    pub(crate) fn wayland_destroy_window_framebuffer(&self, window: WindowID) {
        let fb = self.with_data(|d| d.window_mut(window).and_then(|w| w.framebuffer.take()));
        drop(fb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_packed() {
        let src = [1u8, 2, 3, 4, 9, 9, 5, 6, 7, 8, 9, 9];
        let mut dst = [0u8; 8];
        copy_rows(&mut dst, &src, 6, 4, 2);
        assert_eq!(dst, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(shm_format(PixelFormat::ARGB8888), WlShmFormat::ARGB8888);
        assert_eq!(shm_format(PixelFormat::XRGB8888), WlShmFormat::XRGB8888);
    }
}
