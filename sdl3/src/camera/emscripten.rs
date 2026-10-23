// Rust translation of src/camera/emscripten/SDL_camera_emscripten.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Emscripten camera driver: the browser's camera through
//! `navigator.mediaDevices.getUserMedia()`.
//!
//! Where the page can ask for media (`navigator.mediaDevices`: a browser,
//! in a secure context) the driver reports one camera, "Web browser's
//! camera", with no formats: what the browser has is only known once the
//! user allows it, and asking at initialization would show the browser's
//! permission prompt then. Opening the camera asks `getUserMedia()` for
//! video (the requested size as the ideal one, and the frame rate when the
//! request has one); the camera's permission stays pending until the
//! browser answers. When it does, the stream's settings become the camera's
//! actual format (RGBA32 at the stream's size and frame rate, which the
//! frames are converted and scaled from as the app asked), and an answer
//! that isn't a stream is a denial.
//!
//! As upstream's build has no threads, the driver provides its own
//! "thread" (`ProvidesOwnCallbackThread`): the JavaScript plays the stream
//! in a hidden video element and, once it has its metadata, runs a
//! `requestAnimationFrame()` loop that calls the camera's iteration
//! (`SDL_CameraThreadIterate()`) whenever a frame is due at the stream's
//! frame rate; each iteration draws the video's current frame into a
//! hidden canvas and reads its pixels back (`getImageData()`), timestamped
//! with [`ticks_ns`](crate::timer::ticks_ns) when it's taken.
//!
//! The JavaScript is upstream's, in the `sdl3-emscripten` crate
//! (`sdl3_emscripten::camera`), and calls back the two functions upstream
//! marks `EMSCRIPTEN_KEEPALIVE` by their names, through the callback
//! registry. Where upstream gives it the `SDL_Camera *` to call them with,
//! this gives it the camera's instance ID, which they look the device up
//! by (as the CoreMedia and PipeWire drivers' callbacks do).
//!
//! Without `navigator.mediaDevices` (under node, where `navigator` has no
//! media devices) the driver starts, as upstream's does, and reports no
//! camera.
//!
//! Note (upstream), what the driver does, kept as it is:
//!
//! * a requested frame rate without a size is dropped (`constraints.video`
//!   is `true` then, and setting a property on it does nothing);
//! * the browser's frame rate becomes the camera's as an integer (the
//!   callback's `int`), but the JavaScript paces the frames at its exact
//!   value, and a stream whose settings have no frame rate gets one frame
//!   (and the format 0/1 frames per second);
//! * a frame is taken when the loop, which runs at the display's rate,
//!   finds one due (`performance.now()` at or past the next frame's time):
//!   a loop that's late takes one frame and skips the times it missed, and
//!   one that runs exactly at the next frame's time doesn't move it, so it
//!   takes another frame at its next run;
//! * the loop stops when it finds no stream in `Module['SDL3'].camera`: a
//!   camera closed and opened again, and approved, before the loop's next
//!   run (in the same display frame) keeps the old loop, which goes on
//!   with the new stream next to the new one's;
//! * once approved, a stream whose surfaces couldn't be prepared (out of
//!   memory) is denied but not stopped, and so is a stream for a camera
//!   that was closed meanwhile (see below).
//!
//! Note (upstream), what upstream does that can't be done here or is done
//! as upstream means:
//!
//! * upstream's `OpenDevice` never sets `device->hidden`, which the camera
//!   core takes for "the device is open": so closing the camera
//!   (`SDL_CloseCamera()`, or `SDL_Quit()`) does nothing: it doesn't stop
//!   the stream, the frame loop goes on, and the camera stays approved with
//!   its surfaces; and opening it again asks for a second stream, whose
//!   loop runs next to the first one's, and prepares the surfaces again
//!   (see the next point). Here the core keeps the backend
//!   [`CameraDriverImpl::open_device`] returns, and closing the camera
//!   stops its stream and its loop, as upstream means;
//! * a requested spec with a format makes the core prepare the surfaces
//!   when the camera opens (for the requested spec itself, the only one a
//!   camera without formats can match), and the browser's answer prepares
//!   them again for the stream's format: in a debug build upstream's
//!   assertion that `SDL_PrepareCameraSurfaces()` isn't called twice fails,
//!   and a release build leaks the first ones. Here the first ones are
//!   dropped before;
//! * an answer for a camera that's no longer open (closed while the
//!   browser asked the user) would approve the closed camera and start its
//!   frame loop, and after `SDL_Quit()` it uses the freed `SDL_Camera *`.
//!   Here it's refused (the callback returns 0, so the JavaScript doesn't
//!   play the stream; the stream itself isn't stopped).

use std::ffi::c_int;
use std::sync::{Arc, Once};

use sdl3_emscripten::camera as js;

use super::{
    add_camera, camera_permission_outcome, camera_thread_iterate, find_physical_camera_by_callback,
    prepare_camera_surfaces, AcquiredFrame, CameraBackend, CameraBootStrap, CameraDevice,
    CameraDriverImpl, CameraFrameResult, CameraID, CameraPosition, CameraSpec,
};
use crate::error::Result;
use crate::log::Category;
use crate::video::PixelFormat;

/// The driver (`EMSCRIPTENCAMERA_Init()`'s implementation).
struct EmscriptenCamera;

/// An opened camera. Upstream's has no `SDL_PrivateCameraData` (see the
/// module's documentation): the state is the JavaScript's.
struct EmscriptenCameraDevice;

/// The camera with this instance ID (where upstream's JavaScript has the
/// `SDL_Camera *`).
fn find_device(devid: CameraID) -> Option<Arc<CameraDevice>> {
    find_physical_camera_by_callback(|d| d.instance_id == devid).ok()
}

impl CameraBackend for EmscriptenCameraDevice {
    /// Translation of `EMSCRIPTENCAMERA_WaitDevice()`.
    fn wait_device(&self, _device: &CameraDevice) -> bool {
        crate::sdl_assert!(!"This shouldn't be called"); // we aren't using SDL's internal thread.
        false
    }

    /// Translation of `EMSCRIPTENCAMERA_AcquireFrame()`.
    fn acquire_frame(&self, device: &CameraDevice) -> CameraFrameResult {
        let spec = device.actual_spec();
        let len = (spec.width.max(0) as usize) * (spec.height.max(0) as usize) * 4;
        let mut rgba = Vec::new();
        if rgba.try_reserve_exact(len).is_err() {
            return CameraFrameResult::Error;
        }
        rgba.resize(len, 0);

        let timestamp_ns = crate::timer::ticks_ns(); // best we can do here.

        // SAFETY: the JavaScript writes `width` × `height` × 4 bytes at
        // `rgba`, which holds them.
        let rc =
            unsafe { js::sdl3_camera_acquire_frame(spec.width, spec.height, rgba.as_mut_ptr()) };

        if rc == 0 {
            return CameraFrameResult::Error; // something went wrong, maybe shutting down; just don't return a frame.
        }

        CameraFrameResult::Ready(AcquiredFrame {
            pixels: rgba,
            pitch: spec.width * 4,
            timestamp_ns,
            rotation: 0.0,
        })
    }

    /// Translation of `EMSCRIPTENCAMERA_ReleaseFrame()`.
    fn release_frame(&self, _device: &CameraDevice, pixels: Vec<u8>) {
        drop(pixels);
    }

    /// Translation of `EMSCRIPTENCAMERA_CloseDevice()`.
    fn close_device(&self, _device: &CameraDevice) {
        // SAFETY: plain JavaScript.
        unsafe { js::sdl3_camera_close_device() };
        // (the core drops this, `device->hidden`)
    }
}

/// Translation of `SDLEmscriptenCameraPermissionOutcome()`, which the
/// JavaScript calls (`Module._SDLEmscriptenCameraPermissionOutcome`) with
/// the camera's instance ID when the browser has answered.
extern "C-unwind" fn permission_outcome(
    devid: CameraID,
    approved: c_int,
    w: c_int,
    h: c_int,
    fps: c_int,
) -> c_int {
    // (Note (upstream): a camera that's gone, or not open anymore, isn't
    // approved; see the module's documentation)
    let Some(device) = find_device(devid) else {
        return 0;
    };
    if device.lock().borrow().hidden.is_none() {
        return 0;
    }

    let mut approved = approved != 0;
    if approved {
        {
            let guard = device.lock();
            let mut st = guard.borrow_mut();
            st.actual_spec.format = PixelFormat::RGBA32;
            st.actual_spec.width = w;
            st.actual_spec.height = h;
            st.actual_spec.framerate_numerator = fps;
            st.actual_spec.framerate_denominator = 1;

            // (Note (upstream): the surfaces the core prepared at open for a
            // requested format go first; see the module's documentation)
            st.acquire_surface = None;
            st.conversion_surface = None;
            st.output_surfaces.clear();
            st.empty_output_surfaces.clear();
            st.filled_output_surfaces.clear();
        }

        if let Err(e) = prepare_camera_surfaces(&device) {
            // uhoh, we're in trouble. Probably ran out of memory.
            crate::error!(
                Category::Error,
                "Camera could not prepare surfaces: {} ... revoking approval!",
                e.message()
            );
            approved = false; // disconnecting the SDL camera might not be safe here, just mark it as denied by user.
        }
    }

    camera_permission_outcome(&device, approved);
    approved as c_int
}

/// Translation of `SDLEmscriptenThreadIterate()`, which the JavaScript's
/// frame loop calls (`Module._SDLEmscriptenThreadIterate`) with the
/// camera's instance ID for each frame due.
extern "C-unwind" fn thread_iterate(devid: CameraID) -> bool {
    match find_device(devid) {
        Some(device) => camera_thread_iterate(&device),
        None => false,
    }
}

impl CameraDriverImpl for EmscriptenCamera {
    /// Translation of `EMSCRIPTENCAMERA_DetectDevices()`.
    fn detect_devices(&self) {
        // `navigator.mediaDevices` is not defined if unsupported or not in a secure context!
        // SAFETY: plain JavaScript.
        let supported = unsafe { js::sdl3_camera_supported() } != 0;

        // if we have support at all, report a single generic camera with no specs.
        //  We'll find out if there really _is_ a camera when we try to open it, but querying it for real here
        //  will pop up a user permission dialog warning them we're trying to access the camera, and we generally
        //  don't want that during SDL_Init().
        if supported {
            add_camera(
                "Web browser's camera",
                CameraPosition::Unknown,
                &[],
                Box::new(0x1usize),
            );
        }
    }

    /// Translation of `EMSCRIPTENCAMERA_OpenDevice()`.
    fn open_device(
        &self,
        device: &Arc<CameraDevice>,
        spec: &CameraSpec,
    ) -> Result<Arc<dyn CameraBackend>> {
        // SAFETY: plain JavaScript; it calls back the two functions
        // registered at initialization, with the instance ID.
        unsafe {
            js::sdl3_camera_open_device(
                device.instance_id,
                spec.width,
                spec.height,
                spec.framerate_numerator,
                spec.framerate_denominator,
            )
        };

        Ok(Arc::new(EmscriptenCameraDevice)) // the real work waits until the user approves a camera.
    }

    /// Translation of `EMSCRIPTENCAMERA_FreeDeviceHandle()`.
    fn free_device_handle(&self, _device: &CameraDevice) {
        // no-op.
    }

    /// Translation of `EMSCRIPTENCAMERA_Deinitialize()`.
    fn deinitialize(&self) {
        // SAFETY: plain JavaScript.
        unsafe { js::sdl3_camera_deinitialize() };
    }

    fn provides_own_callback_thread(&self) -> bool {
        true
    }
}

/// Makes the functions upstream marks `EMSCRIPTEN_KEEPALIVE` in this file
/// callable from JavaScript (see `sdl3_emscripten::Callback`), once.
fn register_callbacks() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: extern "C-unwind" functions with the parameters the
        // JavaScript passes; they live as long as the program.
        unsafe {
            js::SDL_EMSCRIPTEN_CAMERA_PERMISSION_OUTCOME.register(permission_outcome as *const ());
            js::SDL_EMSCRIPTEN_THREAD_ITERATE.register(thread_iterate as *const ());
        }
    });
}

/// Translation of `EMSCRIPTENCAMERA_Init()`.
fn emscriptencamera_init() -> Result<Arc<dyn CameraDriverImpl>> {
    register_callbacks();

    // SAFETY: plain JavaScript; SDL's initialization made Module['SDL3'].
    unsafe { js::sdl3_camera_init() };

    Ok(Arc::new(EmscriptenCamera))
}

/// Translation of `EMSCRIPTENCAMERA_bootstrap`.
pub(super) static EMSCRIPTENCAMERA_BOOTSTRAP: CameraBootStrap = CameraBootStrap {
    name: "emscripten",
    desc: "SDL Emscripten MediaStream camera driver",
    init: emscriptencamera_init,
    demand_only: false,
};

#[cfg(test)]
mod tests;
