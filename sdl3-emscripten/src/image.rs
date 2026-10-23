// Rust translation of the Emscripten parts of src/IMG.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What SDL_image's `IMG.c` asks emscripten for on Emscripten: the image
//! the browser decoded when a file was preloaded, which `IMG_Load()` (by
//! the file's path) and `IMG_LoadTyped_IO()` (by the stream's `FILE *`)
//! take before decoding the file themselves. For the `sdl3-image` crate,
//! whose own code has no `unsafe`: [`PreloadedImage`] owns the pixels
//! emscripten allocates and frees them with `free()`.
//!
//! A file is preloaded with emcc's `--preload-file` and
//! `--use-preload-plugins` (emscripten's file packager then calls
//! `FS.preloadFile()` for it, which a page may call itself): emscripten's
//! image plugin has the browser decode a `.jpg`, `.jpeg`, `.png`, `.bmp` or
//! `.webp` file (an `Image` drawn into a canvas, kept in
//! `Browser.preloadedImages` under the file's absolute path) as the file is
//! put in the file system. Without that (and under node, where nothing
//! decodes images) there's no preloaded image.

use core::ffi::{c_char, c_int, CStr};
use core::fmt;
use core::ptr::NonNull;

use crate::sys;

/// The pixels of a preloaded image: RGBA, 4 bytes a pixel, rows of
/// `width * 4` bytes, as the browser's canvas gives them
/// (`getImageData()`, not premultiplied), in a buffer emscripten allocated
/// with `malloc()`, freed with `free()` when this is dropped (upstream's
/// `free(data); /* This should NOT be SDL_free() */`).
pub struct PreloadedImage {
    data: NonNull<u8>,
    width: c_int,
    height: c_int,
}

impl PreloadedImage {
    /// The image preloaded as the file `path` (resolved against the current
    /// directory, so a relative path finds it too), or `None`:
    /// `emscripten_get_preloaded_image_data()`, as `IMG_Load()` calls it.
    pub fn of_path(path: &CStr) -> Option<PreloadedImage> {
        let (mut w, mut h) = (0, 0);
        // SAFETY: a NUL-terminated path, and the two ints emscripten sets
        // when it returns pixels.
        let data =
            unsafe { sys::emscripten_get_preloaded_image_data(path.as_ptr(), &mut w, &mut h) };
        PreloadedImage::new(data, w, h)
    }

    /// The image preloaded as the file the descriptor `fd` is open on, or
    /// `None`: `emscripten_get_preloaded_image_data_from_FILE()`, as
    /// `IMG_LoadTyped_IO()` calls it with its stream's `FILE *`, here with
    /// a stdio stream over a duplicate of `fd`, closed afterwards
    /// (emscripten looks the image up by the descriptor's path, which the
    /// duplicate shares).
    pub fn of_fd(fd: c_int) -> Option<PreloadedImage> {
        // SAFETY: dup() takes any number; it fails or gives a new
        // descriptor, ours.
        let dup = unsafe { sys::dup(fd) };
        if dup < 0 {
            return None;
        }
        // SAFETY: our descriptor and a NUL-terminated mode; the stream owns
        // the descriptor once it's made.
        let fp = unsafe { sys::fdopen(dup, c"rb".as_ptr()) };
        if fp.is_null() {
            // SAFETY: still ours.
            unsafe { sys::close(dup) };
            return None;
        }
        let (mut w, mut h) = (0, 0);
        // SAFETY: an open stream, and the two ints emscripten sets when it
        // returns pixels.
        let data =
            unsafe { sys::emscripten_get_preloaded_image_data_from_FILE(fp, &mut w, &mut h) };
        // SAFETY: the stream made above, closed once (and its descriptor
        // with it).
        unsafe { sys::fclose(fp) };
        PreloadedImage::new(data, w, h)
    }

    fn new(data: *mut c_char, width: c_int, height: c_int) -> Option<PreloadedImage> {
        NonNull::new(data.cast()).map(|data| PreloadedImage {
            data,
            width,
            height,
        })
    }

    /// The image's width, in pixels (the canvas's `width`).
    pub fn width(&self) -> i32 {
        self.width
    }

    /// The image's height, in pixels (the canvas's `height`).
    pub fn height(&self) -> i32 {
        self.height
    }

    /// The pixels: `width * height * 4` bytes of RGBA. (Empty if that size
    /// doesn't fit in the 32-bit size emscripten allocated, which a canvas,
    /// whose area browsers limit far below it, never comes near.)
    pub fn pixels(&self) -> &[u8] {
        let len = usize::try_from(self.width)
            .ok()
            .zip(usize::try_from(self.height).ok())
            .and_then(|(w, h)| w.checked_mul(h)?.checked_mul(4))
            .filter(|&len| isize::try_from(len).is_ok())
            .unwrap_or(0);
        // SAFETY: emscripten allocated `width * height * 4` bytes and
        // filled them (`HEAPU8.set()` of the canvas's data), and they live
        // until this is dropped.
        unsafe { core::slice::from_raw_parts(self.data.as_ptr(), len) }
    }
}

impl Drop for PreloadedImage {
    fn drop(&mut self) {
        // SAFETY: emscripten's malloc() made it, and nothing else frees it.
        unsafe { sys::free(self.data.as_ptr().cast()) }; /* This should NOT be SDL_free() */
    }
}

impl fmt::Debug for PreloadedImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreloadedImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}
