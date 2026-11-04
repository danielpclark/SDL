// Rust translation of src/video/kmsdrm/SDL_kmsdrmopengles.c and
// SDL_kmsdrmopengles.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! EGL implementation of SDL OpenGL support on KMS/DRM, on
//! [`crate::video::egl`] (the GBM platform: the windows' EGL surfaces are
//! on their GBM surfaces, made in [`super::video`]), with the three ways
//! a swap reaches the screen: an atomic commit fenced both ways
//! (`EGL_ANDROID_native_fence_sync`: the default with atomic), a blocking
//! atomic commit (`SDL_VIDEO_DOUBLE_BUFFER`, or without the extension),
//! and the legacy `drmModeSetCrtc()`/`drmModePageFlip()` path (without
//! atomic: `SDL_KMSDRM_ATOMIC=0`).

use std::ffi::{c_int, c_void};
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::sys::*;
use super::video::{
    window_info, DisplayData, KmsdrmPlaneInfo, KmsdrmVideo, PlaneSel, SwapPath, VideoData,
    WindowData,
};
use crate::error::{Error, Result};
use crate::events::WindowID;
use crate::hints;
use crate::video::egl::{self, EGLSyncKHR, EGL_NONE, EGL_NO_SURFACE};
use crate::video::gl::{GlConfig, RawGlContext};
use crate::video::window::display_for_window;

/// `EGL_PLATFORM_GBM_KHR`.
const EGL_PLATFORM_GBM_KHR: i32 = 0x31D7;

/// `EGL_SYNC_NATIVE_FENCE_ANDROID`.
const EGL_SYNC_NATIVE_FENCE_ANDROID: u32 = 0x3144;

/// `EGL_SYNC_NATIVE_FENCE_FD_ANDROID`.
const EGL_SYNC_NATIVE_FENCE_FD_ANDROID: i32 = 0x3145;

/// `EGL_NO_NATIVE_FENCE_FD_ANDROID`.
const EGL_NO_NATIVE_FENCE_FD_ANDROID: c_int = -1;

// EGL implementation of SDL OpenGL support

/// Translation of `KMSDRM_GLES_SetDefaultProfileConfig()`.
pub(crate) fn kmsdrm_gles_set_default_profile_config(config: &mut GlConfig) {
    /* if SDL was _also_ built with the Raspberry Pi driver (so we're
    definitely a Pi device) or with the ROCKCHIP video driver
    (it's a ROCKCHIP device),  default to GLES2. */
    // (neither driver is translated)

    config.egl_platform = EGL_PLATFORM_GBM_KHR;
}

/// Translation of `KMSDRM_GLES_LoadLibrary()`.
pub(crate) fn kmsdrm_gles_load_library(_path: Option<&str>) -> Result<()> {
    /* Just pretend you do this here, but don't do it until KMSDRM_CreateWindow(),
    where we do the same library load we would normally do here.
    because this gets called by SDL_CreateWindow() before KMSDR_CreateWindow(),
    so gbm dev isn't yet created when this is called, AND we can't alter the
    call order in SDL_CreateWindow(). */
    Ok(())
}

/// Translation of `KMSDRM_GLES_UnloadLibrary()`.
pub(crate) fn kmsdrm_gles_unload_library() {
    /* As with KMSDRM_GLES_LoadLibrary(), we define our own "dummy" unloading function
    so we manually unload the library whenever we want. */
}

/// Translation of `KMSDRM_GLES_SetSwapInterval()`.
pub(crate) fn kmsdrm_gles_set_swap_interval(interval: i32) -> Result<()> {
    if !egl::is_loaded() {
        return Err(Error::new("EGL not initialized"));
    }

    if interval == 0 || interval == 1 {
        egl::set_stored_swap_interval(interval);
    } else {
        return Err(Error::new("Only swap intervals of 0 or 1 are supported"));
    }

    Ok(())
}

/// Translation of `create_fence()`.
fn create_fence(e: &egl::Egl, fd: c_int) -> EGLSyncKHR {
    let attrib_list = [EGL_SYNC_NATIVE_FENCE_FD_ANDROID, fd, EGL_NONE];

    let fence = e.create_sync_khr(EGL_SYNC_NATIVE_FENCE_ANDROID, &attrib_list);

    crate::sdl_assert!(!fence.is_null());
    fence
}

impl KmsdrmVideo {
    /// The window's `egl_surface` (`EGL_NO_SURFACE` without one).
    fn egl_surface_of(&self, window: WindowID) -> *mut c_void {
        self.with_state(|s| s.window(window).map(|w| w.egl_surface))
            .unwrap_or(EGL_NO_SURFACE)
    }

    /// Translation of `KMSDRM_GLES_CreateContext()` (`SDL_EGL_CreateContext_impl`).
    pub(crate) fn kmsdrm_gles_create_context(&self, window: WindowID) -> Result<RawGlContext> {
        egl::create_context(self.egl_surface_of(window))
    }

    /// Translation of `KMSDRM_GLES_MakeCurrent()` (`SDL_EGL_MakeCurrent_impl`).
    pub(crate) fn kmsdrm_gles_make_current(
        &self,
        window: Option<WindowID>,
        context: Option<RawGlContext>,
    ) -> Result<()> {
        let surface = window.map_or(EGL_NO_SURFACE, |w| self.egl_surface_of(w));
        egl::make_current(surface, context)
    }

    /// Add the pageflip of the window's next front buffer to the request
    /// list, and the modeset when one is pending (the part of the two
    /// atomic swaps after the framebuffer).
    #[allow(clippy::too_many_arguments)]
    fn atomic_flip_props(
        &self,
        vid: &VideoData,
        dispdata: &mut DisplayData,
        fb_id: u32,
        window_w: i32,
        window_h: i32,
        fenced: bool,
        modesetting: bool,
    ) {
        // SAFETY: the display's CRTC is libdrm's.
        let crtc_id = unsafe { (*dispdata.crtc.crtc).crtc_id };

        // Add the pageflip to the request list.
        let mut info = KmsdrmPlaneInfo::zeroed(PlaneSel::Display);
        info.crtc_id = crtc_id;
        info.fb_id = fb_id;
        info.src_w = window_w; // !!! FIXME: was windata->src_w in the original atomic patch
        info.src_h = window_h; // !!! FIXME: was windata->src_h in the original atomic patch
        info.crtc_w = dispdata.mode.hdisplay as i32; // !!! FIXME: was windata->output_w in the original atomic patch
        info.crtc_h = dispdata.mode.vdisplay as i32; // !!! FIXME: was windata->output_h in the original atomic patch
        info.crtc_x = 0; // !!! FIXME: was windata->output_x in the original atomic patch

        self.drm_atomic_set_plane_props(dispdata, &info);

        if fenced {
            /*****************************************************************/
            /* Tell the display (KMS) that it will have to wait on the fence */
            /* for the GPU-side FENCE.                                       */
            /*                                                               */
            /* Since KMS is a kernel thing, we have to pass an FD into       */
            /* the kernel, and get another FD out of the kernel.             */
            /*                                                               */
            /* 1) To pass the GPU-side fence into the kernel, we set the     */
            /* INPUT FD as the IN_FENCE_FD prop of the PRIMARY PLANE.        */
            /* This FD tells KMS (the kernel) to wait for the GPU-side fence.*/
            /*                                                               */
            /* 2) To get the KMS-side fence out of the kernel, we set the    */
            /* OUTPUT FD as the OUT_FEWNCE_FD prop of the CRTC.              */
            /* This FD will be later imported as a FENCE OBJECT which will be*/
            /* used to tell the GPU to wait for KMS to complete the changes  */
            /* requested in atomic_commit (the pageflip in this case).       */
            /*****************************************************************/
            if dispdata.kms_in_fence_fd != -1 {
                if let Some(plane) = dispdata.display_plane.as_deref() {
                    self.add_plane_property(
                        dispdata.atomic_req,
                        plane,
                        "IN_FENCE_FD",
                        dispdata.kms_in_fence_fd as i64 as u64,
                    );
                }
                // (the kernel writes the fence there during the commit)
                let out_fence_ptr =
                    std::ptr::addr_of_mut!(dispdata.kms_out_fence_fd) as usize as u64;
                self.add_crtc_property(
                    dispdata.atomic_req,
                    &dispdata.crtc,
                    "OUT_FENCE_PTR",
                    out_fence_ptr,
                );
            }
        }

        /* Do we have a pending modesetting? If so, set the necessary
        props so it's included in the incoming atomic commit. */
        if modesetting {
            let mut blob_id: u32 = 0;

            self.add_connector_property(
                dispdata.atomic_req,
                &dispdata.connector,
                "CRTC_ID",
                crtc_id as u64,
            );
            if let Some(atomic) = &self.syms.atomic {
                // SAFETY: the descriptor is the card's; the mode is read for
                // the call; an out-parameter.
                unsafe {
                    (atomic.drmModeCreatePropertyBlob)(
                        vid.drm_fd,
                        std::ptr::addr_of!(dispdata.mode).cast(),
                        std::mem::size_of::<drmModeModeInfo>(),
                        &mut blob_id,
                    )
                };
            }
            self.add_crtc_property(
                dispdata.atomic_req,
                &dispdata.crtc,
                "MODE_ID",
                blob_id as u64,
            );
            // FIXME (upstream): the CRTC's property is "ACTIVE"; "active"
            // isn't found (add_crtc_property() fails), so it isn't set.
            self.add_crtc_property(dispdata.atomic_req, &dispdata.crtc, "active", 1);
        }
    }

    /// The first swap's `drmModeSetCrtc()` ("On the first swap,
    /// immediately present the new front buffer").
    fn first_swap_set_crtc(
        &self,
        vid: &VideoData,
        dispdata: &mut DisplayData,
        fb_id: u32,
    ) -> Result<()> {
        /* On the first swap, immediately present the new front buffer. Before
        drmModePageFlip can be used the CRTC has to be configured to use
        the current connector and mode with drmModeSetCrtc */
        // SAFETY: the descriptor is the card's; the CRTC, connector and mode
        // are the display's.
        let ret = unsafe {
            let mut connector_id = (*dispdata.connector.connector).connector_id;
            (self.syms.drm.drmModeSetCrtc)(
                vid.drm_fd,
                (*dispdata.crtc.crtc).crtc_id,
                fb_id,
                0,
                0,
                &mut connector_id,
                1,
                &mut dispdata.mode,
            )
        };

        if ret != 0 {
            return Err(Error::new("Could not set videomode on CRTC."));
        }
        Ok(())
    }

    /// The window's, its display's and the device's data, locked, for a
    /// swap.
    fn with_swap_parts<R>(
        &self,
        window: WindowID,
        f: impl FnOnce(&mut VideoData, &mut DisplayData, &mut WindowData) -> Result<R>,
    ) -> Result<R> {
        let display = display_for_window(window)?;
        self.with_state(|s| match s.parts(display, window) {
            Some((vid, dispdata, windata)) => f(vid, dispdata, windata),
            None => Err(Error::new("Invalid window")),
        })
    }

    /***********************************************************************************/
    /* Comments about buffer access protection mechanism (=fences) are the ones boxed. */
    /* Also, DON'T remove the asserts: if a fence-related call fails, it's better that */
    /* program exits immediately, or we could leave KMS waiting for a failed/missing   */
    /* fence forever.                                                                  */
    /***********************************************************************************/
    /// Translation of `KMSDRM_GLES_SwapWindowFenced()`.
    fn kmsdrm_gles_swap_window_fenced(&self, window: WindowID) -> Result<()> {
        let modesetting =
            self.with_state(|s| s.window(window).is_some_and(|w| w.egl_surface_dirty));

        if modesetting {
            let _ = self.kmsdrm_create_surfaces(window);
        }

        let info = window_info(window)?;
        let Some(e) = egl::egl() else {
            return Err(Error::new("EGL not initialized"));
        };
        let gbm = &self.syms.gbm;

        self.with_swap_parts(window, |vid, dispdata, windata| {
            /******************************************************************/
            /* Create the GPU-side FENCE OBJECT. It will be inserted into the */
            /* GL CMDSTREAM exactly at the end of the gl commands that form a */
            /* frame.(KMS will have to wait on it before doing a pageflip.)   */
            /******************************************************************/
            dispdata.gpu_fence = create_fence(&e, EGL_NO_NATIVE_FENCE_FD_ANDROID);
            crate::sdl_assert!(!dispdata.gpu_fence.is_null());

            /******************************************************************/
            /* eglSwapBuffers flushes the fence down the GL CMDSTREAM, so we  */
            /* know for sure it's there now.                                  */
            /* Also it marks, at EGL level, the buffer that we want to become */
            /* the new front buffer. (Remember that won't really happen until */
            /* we request a pageflip at the KMS level and it completes.       */
            /******************************************************************/
            if e.swap_buffers_raw(windata.egl_surface) == 0 {
                return Err(e.set_error("Failed to swap EGL buffers", "eglSwapBuffers"));
            }

            /******************************************************************/
            /* EXPORT the GPU-side FENCE OBJECT to the fence INPUT FD, so we  */
            /* can pass it into the kernel. Atomic ioctl will pass the        */
            /* in-fence fd into the kernel, thus telling KMS that it has to   */
            /* wait for GPU to finish rendering the frame (remember where we  */
            /* put the fence in the GL CMDSTREAM) before doing the changes    */
            /* requested in the atomic ioct (the pageflip in this case).      */
            /* (We export the GPU-side FENCE OBJECT to the fence INPUT FD now,*/
            /* not sooner, because now we are sure that the GPU-side fence is */
            /* in the CMDSTREAM to be lifted when the CMDSTREAM to this point */
            /* is completed).                                                 */
            /******************************************************************/
            dispdata.kms_in_fence_fd = e.dup_native_fence_fd_android(dispdata.gpu_fence);

            e.destroy_sync_khr(dispdata.gpu_fence);
            crate::sdl_assert!(dispdata.kms_in_fence_fd != -1);

            /* Lock the buffer that is marked by eglSwapBuffers() to become the
            next front buffer (so it can not be chosen by EGL as back buffer
            to draw on), and get a handle to it to request the pageflip on it.
            REMEMBER that gbm_surface_lock_front_buffer() ALWAYS has to be
            called after eglSwapBuffers(). */
            // SAFETY: the surface is the window's GBM surface.
            windata.next_bo = unsafe { (gbm.gbm_surface_lock_front_buffer)(windata.gs) };
            if windata.next_bo.is_null() {
                return Err(Error::new("Failed to lock frontbuffer"));
            }
            let Some(fb_id) = self.kmsdrm_fb_from_bo(vid, windata.next_bo) else {
                return Err(Error::new("Failed to get a new framebuffer from BO"));
            };

            if windata.bo.is_null() {
                self.first_swap_set_crtc(vid, dispdata, fb_id)?;
            }

            self.atomic_flip_props(vid, dispdata, fb_id, info.w, info.h, true, modesetting);

            /*****************************************************************/
            /* Issue a non-blocking atomic commit: for triple buffering,     */
            /* this must not block so the game can start building another    */
            /* frame, even if the just-requested pageflip hasnt't completed. */
            /*****************************************************************/
            if self.drm_atomic_commit(vid, dispdata, false, modesetting) != 0 {
                return Err(Error::new("Failed to issue atomic commit on pageflip"));
            }

            /* Release the previous front buffer so EGL can chose it as back buffer
            and render on it again. */
            if !windata.bo.is_null() {
                // SAFETY: the buffer is the surface's, locked by the window.
                unsafe { (gbm.gbm_surface_release_buffer)(windata.gs, windata.bo) };
            }
            /* Take note of the buffer about to become front buffer, so next
            time we come here we can free it like we just did with the previous
            front buffer. */
            windata.bo = windata.next_bo;

            /****************************************************************/
            /* Import the KMS-side FENCE OUTPUT FD from the kernel to the   */
            /* KMS-side FENCE OBJECT so we can use use it to fence the GPU. */
            /****************************************************************/
            dispdata.kms_fence = create_fence(&e, dispdata.kms_out_fence_fd);
            crate::sdl_assert!(!dispdata.kms_fence.is_null());

            /****************************************************************/
            /* "Delete" the fence OUTPUT FD, because we already have the    */
            /* KMS FENCE OBJECT, the fence itself is away from us, on the   */
            /* kernel side.                                                 */
            /****************************************************************/
            dispdata.kms_out_fence_fd = -1;

            /*****************************************************************/
            /* Tell the GPU to wait on the fence for the KMS-side FENCE,     */
            /* which means waiting until the requested pageflip is completed.*/
            /*****************************************************************/
            e.wait_sync_khr(dispdata.kms_fence, 0);

            Ok(())
        })
    }

    /// Translation of `KMSDRM_GLES_SwapWindowDoubleBuffered()`.
    fn kmsdrm_gles_swap_window_double_buffered(&self, window: WindowID) -> Result<()> {
        let modesetting =
            self.with_state(|s| s.window(window).is_some_and(|w| w.egl_surface_dirty));

        if modesetting {
            let _ = self.kmsdrm_create_surfaces(window);
        }

        let info = window_info(window)?;
        let Some(e) = egl::egl() else {
            return Err(Error::new("EGL not initialized"));
        };
        let gbm = &self.syms.gbm;

        self.with_swap_parts(window, |vid, dispdata, windata| {
            /**********************************************************************************/
            /* In double-buffer mode, atomic_commit will always be synchronous/blocking (ie:  */
            /* won't return until the requested changes are really done).                     */
            /* Also, there's no need to fence KMS or the GPU, because we won't be entering    */
            /* game loop again (hence not building or executing a new cmdstring) until        */
            /* pageflip is done, so we don't need to protect the KMS/GPU access to the buffer.*/
            /**********************************************************************************/

            /* Mark, at EGL level, the buffer that we want to become the new front buffer.
            It won't really happen until we request a pageflip at the KMS level and it
            completes. */
            if e.swap_buffers_raw(windata.egl_surface) == 0 {
                return Err(e.set_error("Failed to swap EGL buffers", "eglSwapBuffers"));
            }
            /* Lock the buffer that is marked by eglSwapBuffers() to become the next front buffer
            (so it can not be chosen by EGL as back buffer to draw on), and get a handle to it,
            to request the pageflip on it. */
            // SAFETY: the surface is the window's GBM surface.
            windata.next_bo = unsafe { (gbm.gbm_surface_lock_front_buffer)(windata.gs) };
            if windata.next_bo.is_null() {
                return Err(Error::new("Failed to lock frontbuffer"));
            }
            let Some(fb_id) = self.kmsdrm_fb_from_bo(vid, windata.next_bo) else {
                return Err(Error::new("Failed to get a new framebuffer BO"));
            };

            if windata.bo.is_null() {
                self.first_swap_set_crtc(vid, dispdata, fb_id)?;
            }

            self.atomic_flip_props(vid, dispdata, fb_id, info.w, info.h, false, modesetting);

            /* Issue the one and only atomic commit where all changes will be requested!
            Blocking for double buffering: won't return until completed. */
            if self.drm_atomic_commit(vid, dispdata, true, modesetting) != 0 {
                return Err(Error::new("Failed to issue atomic commit on pageflip"));
            }

            // Release last front buffer so EGL can chose it as back buffer and render on it again.
            if !windata.bo.is_null() {
                // SAFETY: the buffer is the surface's, locked by the window.
                unsafe { (gbm.gbm_surface_release_buffer)(windata.gs, windata.bo) };
            }

            // Take note of current front buffer, so we can free it next time we come here.
            windata.bo = windata.next_bo;

            Ok(())
        })
    }

    /// Translation of `KMSDRM_GLES_SwapWindowLegacy()`.
    fn kmsdrm_gles_swap_window_legacy(&self, window: WindowID) -> Result<()> {
        let gbm = &self.syms.gbm;
        let drm = &self.syms.drm;

        /* Always wait for the previous issued flip before issuing a new one,
        even if you do async flips. */
        let mut flip_flags: u32 = DRM_MODE_PAGE_FLIP_EVENT;

        let (no_surface, dirty) = self.with_state(|s| {
            s.window(window).map_or((true, false), |w| {
                (w.egl_surface == EGL_NO_SURFACE, w.egl_surface_dirty)
            })
        });

        // Skip the swap if we've switched away to another VT
        if no_surface {
            // Wait a bit, throttling to ~100 FPS
            crate::timer::delay(Duration::from_millis(10));
            return Ok(());
        }

        // Recreate the GBM / EGL surfaces if the display mode has changed
        if dirty {
            let _ = self.kmsdrm_create_surfaces(window);
        }

        let Some(e) = egl::egl() else {
            return Err(Error::new("EGL not initialized"));
        };

        self.with_swap_parts(window, |vid, dispdata, windata| {
            /* Wait for confirmation that the next front buffer has been flipped, at which
            point the previous front buffer can be released */
            if !self.kmsdrm_wait_pageflip(vid, windata) {
                return Err(Error::new("Wait for previous pageflip failed"));
            }

            // Release the previous front buffer
            if !windata.bo.is_null() {
                // SAFETY: the buffer is the surface's, locked by the window.
                unsafe { (gbm.gbm_surface_release_buffer)(windata.gs, windata.bo) };
            }

            windata.bo = windata.next_bo;

            /* Mark a buffer to become the next front buffer.
            This won't happen until pageflip completes. */
            if e.swap_buffers_raw(windata.egl_surface) == 0 {
                return Err(Error::new("eglSwapBuffers failed"));
            }

            /* From the GBM surface, get the next BO to become the next front buffer,
            and lock it so it can't be allocated as a back buffer (to prevent EGL
            from drawing into it!) */
            // SAFETY: the surface is the window's GBM surface.
            windata.next_bo = unsafe { (gbm.gbm_surface_lock_front_buffer)(windata.gs) };
            if windata.next_bo.is_null() {
                return Err(Error::new("Could not lock front buffer on GBM surface"));
            }

            // Get an actual usable fb for the next front buffer.
            let Some(fb_id) = self.kmsdrm_fb_from_bo(vid, windata.next_bo) else {
                return Err(Error::new("Could not get a framebuffer"));
            };

            if windata.bo.is_null() {
                self.first_swap_set_crtc(vid, dispdata, fb_id)?;
            } else {
                /* On subsequent swaps, queue the new front buffer to be flipped during
                the next vertical blank

                Remember: drmModePageFlip() never blocks, it just issues the flip,
                which will be done during the next vblank, or immediately if
                we pass the DRM_MODE_PAGE_FLIP_ASYNC flag.
                Since calling drmModePageFlip() will return EBUSY if we call it
                without having completed the last issued flip, we must pass the
                DRM_MODE_PAGE_FLIP_ASYNC if we don't block on EGL (egl_swapinterval = 0).
                That makes it flip immediately, without waiting for the next vblank
                to do so, so even if we don't block on EGL, the flip will have completed
                when we get here again. */
                if egl::get_swap_interval().unwrap_or(0) == 0 && vid.async_pageflip_support {
                    flip_flags |= DRM_MODE_PAGE_FLIP_ASYNC;
                }

                // SAFETY: the descriptor is the card's; the user data is the
                // window's waiting_for_flip, alive while the window is (the
                // flip is waited for before the window's surfaces go).
                let ret = unsafe {
                    (drm.drmModePageFlip)(
                        vid.drm_fd,
                        (*dispdata.crtc.crtc).crtc_id,
                        fb_id,
                        flip_flags,
                        std::sync::Arc::as_ptr(&windata.waiting_for_flip) as *mut c_void,
                    )
                };

                if ret == 0 {
                    windata.waiting_for_flip.store(true, Ordering::SeqCst);
                } else {
                    crate::error!(
                        crate::log::Category::Video,
                        "Could not queue pageflip: {}",
                        ret
                    );
                }

                /* Wait immediately for vsync (as if we only had two buffers).
                Even if we are already doing a WaitPageflip at the beginning of this
                function, this is NOT redundant because here we wait immediately
                after submitting the image to the screen, reducing lag, and if
                we have waited here, there won't be a pending pageflip so the
                WaitPageflip at the beginning of this function will be a no-op.
                Just leave it here and don't worry.
                Run your SDL program with "SDL_VIDEO_DOUBLE_BUFFER=1 <program_name>"
                to enable this. */
                if windata.double_buffer && !self.kmsdrm_wait_pageflip(vid, windata) {
                    return Err(Error::new("Immediate wait for previous pageflip failed"));
                }
            }

            Ok(())
        })
    }

    /// Translation of `KMSDRM_GLES_SwapWindow()`.
    pub(crate) fn kmsdrm_gles_swap_window(&self, window: WindowID) -> Result<()> {
        let chosen = self.with_state(|s| {
            let is_atomic = s.vid.is_atomic;
            s.window(window).map(|w| (w.swap_window, is_atomic))
        });
        let Some((swap_window, is_atomic)) = chosen else {
            return Err(Error::new("Invalid window"));
        };

        let swap_window = match swap_window {
            Some(swap) => swap,
            None => {
                let swap = if is_atomic {
                    // We want the fenced version by default, but it needs extensions.
                    let has_fence = egl::egl().is_some_and(|e| {
                        egl::has_extension(
                            &e,
                            egl::ExtensionType::Display,
                            "EGL_ANDROID_native_fence_sync",
                        )
                    });
                    if hints::get_bool(hints::VIDEO_DOUBLE_BUFFER, false) || !has_fence {
                        SwapPath::DoubleBuffered
                    } else {
                        SwapPath::Fenced
                    }
                } else {
                    SwapPath::Legacy
                };
                self.with_state(|s| {
                    if let Some(w) = s.window(window) {
                        w.swap_window = Some(swap);
                    }
                });
                swap
            }
        };
        match swap_window {
            SwapPath::Fenced => self.kmsdrm_gles_swap_window_fenced(window),
            SwapPath::DoubleBuffered => self.kmsdrm_gles_swap_window_double_buffered(window),
            SwapPath::Legacy => self.kmsdrm_gles_swap_window_legacy(window),
        }
    }
}
