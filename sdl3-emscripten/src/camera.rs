// Rust translation of the JavaScript in src/camera/emscripten/SDL_camera_emscripten.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The JavaScript of the MediaStream camera driver,
//! `camera/emscripten/SDL_camera_emscripten.c`.
//!
//! Each function is upstream's `MAIN_THREAD_EM_ASM` body, verbatim (in the
//! braces emscripten puts around an `EM_ASM`'s code): without threads the
//! main thread is the only one. The state is in `Module['SDL3'].camera`:
//! once the camera is opened and allowed, its `stream` (from
//! `navigator.mediaDevices.getUserMedia()`), the hidden `video` element
//! playing it, the hidden `canvas` and its 2D context (`ctx2d`) the frames
//! are drawn into and read back from, the size and frame rate the stream
//! has (`width`, `height`, `fps`, `fpsincrms`) and the time the next frame
//! is due (`next_frame_time`, in `performance.now()` milliseconds).
//!
//! The JavaScript calls back two Rust functions by name, through the
//! callback registry: [`SDL_EMSCRIPTEN_CAMERA_PERMISSION_OUTCOME`] once the
//! user (or the browser) has answered, and [`SDL_EMSCRIPTEN_THREAD_ITERATE`]
//! for each frame due (from `requestAnimationFrame()`). Where upstream
//! passes them the `SDL_Camera *`, the driver passes the camera's instance
//! ID, which they look the device up by.

crate::em_js_callback!(
    /// `Module._SDLEmscriptenCameraPermissionOutcome(device, approved, w, h,
    /// fps)`, which [`sdl3_camera_open_device`]'s JavaScript calls when
    /// `getUserMedia()` answers: 1 and the stream's size and frame rate, or
    /// 0 when it failed; the result says whether the camera is approved (1).
    pub static SDL_EMSCRIPTEN_CAMERA_PERMISSION_OUTCOME = "_SDLEmscriptenCameraPermissionOutcome";
);

crate::em_js_callback!(
    /// `Module._SDLEmscriptenThreadIterate(device)`, which the frame loop of
    /// [`sdl3_camera_open_device`] calls for each frame due: one iteration
    /// of the camera's "thread" (`SDL_CameraThreadIterate()`).
    pub static SDL_EMSCRIPTEN_THREAD_ITERATE = "_SDLEmscriptenThreadIterate";
);

// EMSCRIPTENCAMERA_AcquireFrame()'s MAIN_THREAD_EM_ASM_INT, verbatim.
crate::em_js_dollar!(
    /// Draws the video's current frame into the canvas at `w` × `h` and
    /// copies the canvas's RGBA pixels to `rgba` (`w` × `h` × 4 bytes);
    /// 0 (and nothing copied) when there's no `Module['SDL3'].camera.ctx2d`
    /// (the camera isn't streaming), else 1.
    pub fn sdl3_camera_acquire_frame(w: i32, h: i32, rgba: *mut u8) -> i32,
    r#"{
        const w = $0;
        const h = $1;
        const rgba = $2;
        const SDL3 = Module['SDL3'];
        if ((typeof(SDL3) === 'undefined') || (typeof(SDL3.camera) === 'undefined') || (typeof(SDL3.camera.ctx2d) === 'undefined')) {
            return 0;  // don't have something we need, oh well.
        }

        SDL3.camera.ctx2d.drawImage(SDL3.camera.video, 0, 0, w, h);
        const imgrgba = SDL3.camera.ctx2d.getImageData(0, 0, w, h).data;
        HEAPU8.set(imgrgba, rgba);

        return 1;
    }"#
);

// EMSCRIPTENCAMERA_CloseDevice()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Stops the stream's tracks and empties `Module['SDL3'].camera`;
    /// nothing when there's no stream (the camera never streamed, or was
    /// closed, or the driver is gone).
    pub fn sdl3_camera_close_device(),
    r#"{
            const SDL3 = Module['SDL3'];
            if ((typeof(SDL3) === 'undefined') || (typeof(SDL3.camera) === 'undefined') || (typeof(SDL3.camera.stream) === 'undefined')) {
                return;  // camera was closed and/or subsystem was shut down, we're already done.
            }
            SDL3.camera.stream.getTracks().forEach(track => track.stop());  // stop all recording.
            SDL3.camera = {};  // dump our references to everything.
        }"#
);

// EMSCRIPTENCAMERA_OpenDevice()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Asks `navigator.mediaDevices.getUserMedia()` for video: any (`w` or
    /// `h` not positive) or `w` × `h` as the ideal size, and the frame rate
    /// `framerate_numerator / framerate_denominator` as the ideal one (when
    /// both are positive). When it answers, calls
    /// [`SDL_EMSCRIPTEN_CAMERA_PERMISSION_OUTCOME`] with `device` and the
    /// stream's settings (or a failure), and once that approves, plays the
    /// stream in a hidden video element; from its `loadedmetadata` event
    /// on, a `requestAnimationFrame()` loop calls
    /// [`SDL_EMSCRIPTEN_THREAD_ITERATE`] with `device` for each frame due
    /// at the stream's frame rate, until the stream is gone from
    /// `Module['SDL3'].camera`. Both callbacks must be registered.
    pub fn sdl3_camera_open_device(
        device: u32,
        w: i32,
        h: i32,
        framerate_numerator: i32,
        framerate_denominator: i32
    ),
    r#"{
        // Since we can't get actual specs until we make a move that prompts the user for
        // permission, we don't list any specs for the device and wrangle it during device open.
        const device = $0;
        const w = $1;
        const h = $2;
        const framerate_numerator = $3;
        const framerate_denominator = $4;
        const outcome = Module._SDLEmscriptenCameraPermissionOutcome;
        const iterate = Module._SDLEmscriptenThreadIterate;

        const constraints = {};
        if ((w <= 0) || (h <= 0)) {
            constraints.video = true;   // didn't ask for anything, let the system choose.
        } else {
            constraints.video = {};  // asked for a specific thing: request it as "ideal" but take closest hardware will offer.
            constraints.video.width = w;
            constraints.video.height = h;
        }

        if ((framerate_numerator > 0) && (framerate_denominator > 0)) {
            var fps = framerate_numerator / framerate_denominator;
            constraints.video.frameRate = { ideal: fps };
        }

        function grabNextCameraFrame() {  // !!! FIXME: this (currently) runs as a requestAnimationFrame callback, for lack of a better option.
            const SDL3 = Module['SDL3'];
            if ((typeof(SDL3) === 'undefined') || (typeof(SDL3.camera) === 'undefined') || (typeof(SDL3.camera.stream) === 'undefined')) {
                return;  // camera was closed and/or subsystem was shut down, stop iterating here.
            }

            // time for a new frame from the camera?
            const nextframems = SDL3.camera.next_frame_time;
            const now = performance.now();
            if (now >= nextframems) {
                iterate(device);  // calls SDL_CameraThreadIterate, which will call our AcquireFrame implementation.

                // bump ahead but try to stay consistent on timing, in case we dropped frames.
                while (SDL3.camera.next_frame_time < now) {
                    SDL3.camera.next_frame_time += SDL3.camera.fpsincrms;
                }
            }

            requestAnimationFrame(grabNextCameraFrame);  // run this function again at the display framerate.  (!!! FIXME: would this be better as requestIdleCallback?)
        }

        navigator.mediaDevices.getUserMedia(constraints)
            .then((stream) => {
                const settings = stream.getVideoTracks()[0].getSettings();
                const actualw = settings.width;
                const actualh = settings.height;
                const actualfps = settings.frameRate;
                console.log("Camera is opened! Actual spec: (" + actualw + "x" + actualh + "), fps=" + actualfps);

                if (outcome(device, 1, actualw, actualh, actualfps)) {
                    const video = document.createElement("video");
                    video.width = actualw;
                    video.height = actualh;
                    video.style.display = 'none';    // we need to attach this to a hidden video node so we can read it as pixels.
                    video.srcObject = stream;

                    const canvas = document.createElement("canvas");
                    canvas.width = actualw;
                    canvas.height = actualh;
                    canvas.style.display = 'none';    // we need to attach this to a hidden video node so we can read it as pixels.

                    const ctx2d = canvas.getContext('2d');

                    const SDL3 = Module['SDL3'];
                    SDL3.camera.width = actualw;
                    SDL3.camera.height = actualh;
                    SDL3.camera.fps = actualfps;
                    SDL3.camera.fpsincrms = 1000.0 / actualfps;
                    SDL3.camera.stream = stream;
                    SDL3.camera.video = video;
                    SDL3.camera.canvas = canvas;
                    SDL3.camera.ctx2d = ctx2d;
                    SDL3.camera.next_frame_time = performance.now();

                    video.play();
                    video.addEventListener('loadedmetadata', () => {
                        grabNextCameraFrame();  // start this loop going.
                    });
                }
            })
            .catch((err) => {
                console.error("Tried to open camera but it threw an error! " + err.name + ": " +  err.message);
                outcome(device, 0, 0, 0, 0);   // we call this a permission error, because it probably is.
            });
    }"#
);

// EMSCRIPTENCAMERA_Deinitialize()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Drops `Module['SDL3'].camera` (which stops a frame loop still
    /// running).
    pub fn sdl3_camera_deinitialize(),
    r#"{
        if (typeof(Module['SDL3']) !== 'undefined') {
            Module['SDL3'].camera = undefined;
        }
    }"#
);

// EMSCRIPTENCAMERA_DetectDevices()'s MAIN_THREAD_EM_ASM_INT, verbatim.
crate::em_js_dollar!(
    /// Whether there's `navigator.mediaDevices` (1): not under node, nor
    /// in a page that isn't a secure context.
    pub fn sdl3_camera_supported() -> i32,
    r#"{ return (navigator.mediaDevices === undefined) ? 0 : 1; }"#
);

// EMSCRIPTENCAMERA_Init()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Makes `Module['SDL3'].camera` (empty); `Module['SDL3']` must be
    /// there (SDL's initialization makes it).
    pub fn sdl3_camera_init(),
    r#"{
        Module['SDL3'].camera = {};
    }"#
);
