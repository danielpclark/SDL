// Rust translation of src/video/offscreen/SDL_offscreenvideo.c,
// SDL_offscreenwindow.c, SDL_offscreenevents.c and
// SDL_offscreenframebuffer.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The offscreen video driver: like the dummy driver, but meant to let
//! applications use some of the video functionality (notably context
//! creation) without a display, say for automated testing on a headless
//! machine. (Its EGL and Vulkan contexts come with those loaders.)

use std::sync::atomic::AtomicI32;
use std::sync::Arc;

use crate::error::Result;
use crate::events::window::send_window_event;
use crate::events::{DisplayID, EventType, WindowID};
use crate::hints;
use crate::properties::Properties;
use crate::video::core::with_window;
use crate::video::display::add_basic_video_display;
use crate::video::sysvideo::{
    DeviceCaps, DisplayMode, VideoBootStrap, VideoDriver, WINDOWPOS_UNDEFINED,
};
use crate::video::{PixelFormat, Rect, Surface};

const OFFSCREENVID_DRIVER_NAME: &str = "offscreen";

/// The offscreen window data. Translation of `SDL_WindowData`.
struct OffscreenWindow {
    #[allow(dead_code)]
    sdl_window: WindowID,
}

struct OffscreenVideo;

static FRAME_NUMBER: AtomicI32 = AtomicI32::new(0);

impl VideoDriver for OffscreenVideo {
    fn caps(&self) -> DeviceCaps {
        // TODO: Is this needed?
        DeviceCaps::SLOW_FRAMEBUFFER
    }

    fn video_init(&self) -> Result<()> {
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

        // We're done!
        Ok(())
    }

    fn video_quit(&self) {}

    fn set_display_mode(&self, _display: DisplayID, _mode: &DisplayMode) -> Option<Result<()>> {
        Some(Ok(()))
    }

    fn pump_events(&self) -> Option<()> {
        // do nothing.
        Some(())
    }

    fn create_window(&self, window: WindowID, _create_props: &Properties) -> Option<Result<()>> {
        Some(with_window(window, |w| {
            w.internal = Some(Box::new(OffscreenWindow { sdl_window: window }));

            if w.core.x == WINDOWPOS_UNDEFINED {
                w.core.x = 0;
            }

            if w.core.y == WINDOWPOS_UNDEFINED {
                w.core.y = 0;
            }
        }))
    }

    fn destroy_window(&self, window: WindowID) -> Option<()> {
        let _ = with_window(window, |w| w.internal = None);
        Some(())
    }

    fn set_window_size(&self, window: WindowID) -> Option<()> {
        let pending = with_window(window, |w| w.pending).ok()?;
        send_window_event(window, EventType::WINDOW_RESIZED, pending.w, pending.h);
        Some(())
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
            hints::VIDEO_OFFSCREEN_SAVE_FRAMES,
            &FRAME_NUMBER,
            window,
            surface,
        ))
    }

    fn destroy_window_framebuffer(&self, _window: WindowID) -> Option<()> {
        Some(())
    }
}

/// Translation of `OFFSCREEN_CreateDevice()`.
fn create_device() -> Option<Arc<dyn VideoDriver>> {
    if !super::dummy::available(OFFSCREENVID_DRIVER_NAME) {
        return None;
    }
    Some(Arc::new(OffscreenVideo))
}

/// Translation of `OFFSCREEN_bootstrap`.
pub(crate) static OFFSCREEN_BOOTSTRAP: VideoBootStrap = VideoBootStrap {
    name: OFFSCREENVID_DRIVER_NAME,
    desc: "SDL offscreen video driver",
    create: create_device,
    show_message_box: None, // no ShowMessageBox implementation
    is_preferred: false,
};
