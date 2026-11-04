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
//! On Linux, its `evdev` variant reads the keyboards, mice and touch
//! devices through the console input reader
//! ([`crate::core::linux::evdev`]).

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
#[cfg(target_os = "linux")]
const DUMMYVID_DRIVER_EVDEV_NAME: &str = "evdev";

/// Translation of `DUMMY_Available()`: the driver must be asked for.
pub(crate) fn available(drivername: &str) -> bool {
    hints::get(hints::VIDEO_DRIVER).is_some_and(|hint| hint.contains(drivername))
}

/// The driver; `evdev` for `DUMMY_evdev_bootstrap`'s, whose
/// `VideoInit`, `VideoQuit` and `PumpEvents` are the evdev ones.
struct DummyVideo {
    #[cfg(target_os = "linux")]
    evdev: bool,
}

impl DummyVideo {
    /// Whether this is the `evdev` variant.
    fn evdev(&self) -> bool {
        #[cfg(target_os = "linux")]
        return self.evdev;
        #[cfg(not(target_os = "linux"))]
        false
    }
}

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
        #[cfg(target_os = "linux")]
        if self.evdev {
            // Translation of DUMMY_EVDEV_VideoInit()
            video_init_common()?;

            let _ = crate::core::linux::evdev::init();
            return Ok(());
        }
        video_init_common()
    }

    fn video_quit(&self) {
        // Translation of DUMMY_EVDEV_VideoQuit()
        #[cfg(target_os = "linux")]
        if self.evdev {
            crate::core::linux::evdev::quit();
        }
    }

    fn pump_events(&self) -> Option<()> {
        // Translation of DUMMY_EVDEV_Poll()
        #[cfg(target_os = "linux")]
        if self.evdev {
            crate::core::linux::evdev::poll();
            return Some(());
        }
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
        // (DUMMY_VideoInit()'s; DUMMY_EVDEV_VideoInit() leaves it to
        // SDL_EVDEV_Init())
        feature == MouseFeature::SetRelativeMouseMode && !self.evdev()
    }
}

/// Translation of `DUMMY_InternalCreateDevice()`.
fn internal_create_device(enable_hint: &str) -> Option<DummyVideo> {
    if !available(enable_hint) {
        return None;
    }
    Some(DummyVideo {
        #[cfg(target_os = "linux")]
        evdev: false,
    })
}

/// Translation of `DUMMY_CreateDevice()`.
fn create_device() -> Option<Arc<dyn VideoDriver>> {
    Some(Arc::new(internal_create_device(DUMMYVID_DRIVER_NAME)?))
}

/// Translation of `DUMMY_bootstrap`.
pub(crate) static DUMMY_BOOTSTRAP: VideoBootStrap = VideoBootStrap {
    name: DUMMYVID_DRIVER_NAME,
    desc: "SDL dummy video driver",
    create: create_device,
    show_message_box: None, // no ShowMessageBox implementation
    is_preferred: false,
};

/// Translation of `DUMMY_EVDEV_CreateDevice()`.
#[cfg(target_os = "linux")]
fn evdev_create_device() -> Option<Arc<dyn VideoDriver>> {
    let mut device = internal_create_device(DUMMYVID_DRIVER_EVDEV_NAME)?;
    device.evdev = true;
    Some(Arc::new(device))
}

/// Translation of `DUMMY_evdev_bootstrap`.
#[cfg(target_os = "linux")]
pub(crate) static DUMMY_EVDEV_BOOTSTRAP: VideoBootStrap = VideoBootStrap {
    name: DUMMYVID_DRIVER_EVDEV_NAME,
    desc: "SDL dummy video driver with evdev",
    create: evdev_create_device,
    show_message_box: None, // no ShowMessageBox implementation
    is_preferred: false,
};
