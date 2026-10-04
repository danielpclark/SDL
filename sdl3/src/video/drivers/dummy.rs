// Rust translation of src/video/dummy/SDL_nullvideo.c, SDL_nullevents.c and
// SDL_nullframebuffer.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The dummy video driver: just enough to make an SDL-based application
//! THINK it's got a working video driver, for applications that initialize
//! video when they don't need it, and as a collection of stubs when porting
//! SDL to a new platform.
//!
//! This is also a great way to determine bottlenecks: if you think that SDL
//! is a performance problem for a given platform, enable this driver, and
//! then see if your application runs faster without video overhead.
//!
//! (The `evdev` variant, which reads input devices, comes with the Linux
//! input layer.)

use std::sync::atomic::AtomicI32;
use std::sync::Arc;

use crate::error::Result;
use crate::events::mouse::MouseFeature;
use crate::events::window::send_window_event;
use crate::events::{EventType, WindowID};
use crate::hints;
use crate::video::core::with_window;
use crate::video::display::add_basic_video_display;
use crate::video::sysvideo::{DisplayMode, VideoBootStrap, VideoDriver};
use crate::video::{PixelFormat, Rect, Surface};

const DUMMYVID_DRIVER_NAME: &str = "dummy";

/// Translation of `DUMMY_Available()`: the driver must be asked for.
pub(super) fn available(drivername: &str) -> bool {
    hints::get(hints::VIDEO_DRIVER).is_some_and(|hint| hint.contains(drivername))
}

struct DummyVideo;

/// Translation of `DUMMY_VideoInitCommon()`.
pub(super) fn video_init_common() -> Result<()> {
    // Use a fake 32-bpp desktop mode
    let mode = DisplayMode {
        format: PixelFormat::XRGB8888,
        w: 1024,
        h: 768,
        ..Default::default()
    };
    if add_basic_video_display(Some(&mode)) == 0 {
        return Err(crate::video::core::uninitialized_video());
    }
    Ok(())
}

static FRAME_NUMBER: AtomicI32 = AtomicI32::new(0);

impl VideoDriver for DummyVideo {
    fn video_init(&self) -> Result<()> {
        video_init_common()
    }

    fn video_quit(&self) {}

    fn pump_events(&self) -> Option<()> {
        // do nothing.
        Some(())
    }

    fn set_window_size(&self, window: WindowID) -> Option<()> {
        let pending = with_window(window, |w| w.pending).ok()?;
        send_window_event(window, EventType::WINDOW_RESIZED, pending.w, pending.h);
        Some(())
    }

    fn set_window_position(&self, window: WindowID) -> Option<Result<()>> {
        let pending = with_window(window, |w| w.pending).ok()?;
        send_window_event(window, EventType::WINDOW_MOVED, pending.x, pending.y);
        Some(Ok(()))
    }

    fn implements_window_framebuffer(&self) -> bool {
        true
    }

    fn create_window_framebuffer(
        &self,
        _window: WindowID,
        w: i32,
        h: i32,
    ) -> Option<Result<Surface<'static>>> {
        // Create a new framebuffer
        Some(super::create_xrgb8888_framebuffer(w, h))
    }

    fn update_window_framebuffer(
        &self,
        window: WindowID,
        surface: &Surface<'static>,
        _rects: &[Rect],
    ) -> Option<Result<()>> {
        Some(super::maybe_save_frame(
            hints::VIDEO_DUMMY_SAVE_FRAMES,
            &FRAME_NUMBER,
            window,
            surface,
        ))
    }

    fn destroy_window_framebuffer(&self, _window: WindowID) -> Option<()> {
        Some(())
    }

    fn set_relative_mouse_mode(&self, _enabled: bool) -> Option<Result<()>> {
        Some(Ok(()))
    }

    fn implements_mouse_feature(&self, feature: MouseFeature) -> bool {
        feature == MouseFeature::SetRelativeMouseMode
    }
}

/// Translation of `DUMMY_CreateDevice()`.
fn create_device() -> Option<Arc<dyn VideoDriver>> {
    if !available(DUMMYVID_DRIVER_NAME) {
        return None;
    }
    Some(Arc::new(DummyVideo))
}

/// Translation of `DUMMY_bootstrap`.
pub(crate) static DUMMY_BOOTSTRAP: VideoBootStrap = VideoBootStrap {
    name: DUMMYVID_DRIVER_NAME,
    desc: "SDL dummy video driver",
    create: create_device,
    show_message_box: None, // no ShowMessageBox implementation
    is_preferred: false,
};
