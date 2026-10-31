// Rust translation of src/video/android/SDL_androidevents.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The activity's lifecycle as SDL's application events: SDL's event pump
//! on Android ([`pump_events`], whatever the video driver) takes the
//! lifecycle events the activity queues (`core::android`'s
//! `send_lifecycle_event()`) and sends `SDL_EVENT_WILL_ENTER_BACKGROUND`
//! and `SDL_EVENT_DID_ENTER_BACKGROUND` for a pause,
//! `SDL_EVENT_WILL_ENTER_FOREGROUND` and `SDL_EVENT_DID_ENTER_FOREGROUND`
//! for a resume, `SDL_EVENT_LOW_MEMORY`, and `SDL_EVENT_QUIT` and
//! `SDL_EVENT_TERMINATING` when the activity is destroyed. While the app
//! is paused, with `SDL_HINT_ANDROID_BLOCK_ON_PAUSE` (the default) the
//! pump blocks until the app resumes (and the audio is paused); without
//! it, each pump waits up to 100 ms for the next lifecycle event.
//!
//! When the app pauses, the driver's OpenGL window's EGL context is kept
//! and released ([`android_egl_context_backup`]), so that the activity's
//! surface can be freed; when it resumes, it's made current again with
//! the new surface, or, lost, replaced (with
//! `SDL_EVENT_RENDER_DEVICE_RESET`: [`android_egl_context_restore`]).
//! The audio drivers' devices (AAudio's and OpenSL ES's) are paused with
//! the block on pause, and resumed with the app.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::core::android::{self as core_android, LifecycleEvent};
use crate::events::window::WindowFlags;
use crate::events::{Event, EventType, RenderEvent, WindowID};
use crate::video::gl;

/// Whether `window` is an OpenGL window.
fn is_opengl(window: WindowID) -> bool {
    crate::video::core::with_window(window, |w| w.flags().contains(WindowFlags::OPENGL))
        .unwrap_or(false)
}

/// Make the window's kept EGL context current again (with the activity's
/// new surface), or, when it's no longer valid, a new one, telling the app
/// (`SDL_EVENT_RENDER_DEVICE_RESET`); and its swap interval. Translation
/// of `android_egl_context_restore()`.
fn android_egl_context_restore(window: WindowID) {
    if is_opengl(window) {
        let egl_context = super::window::with_window_data(|_, data| data.egl_context).flatten();
        let _ = gl::make_current_raw(Some(window), None);
        if gl::make_current_raw(Some(window), egl_context).is_err() {
            // The context is no longer valid, create a new one (owned by
            // nothing, as upstream's)
            let egl_context =
                gl::gl_create_context(&crate::video::window::Window::from_raw(window))
                    .ok()
                    .map(|context| {
                        let raw = context.raw();
                        std::mem::forget(context);
                        raw
                    });
            super::window::with_window_data(|_, data| data.egl_context = egl_context);
            let _ = gl::make_current_raw(Some(window), egl_context);
            let _ = crate::events::push(Event::Render(RenderEvent {
                event_type: EventType::RENDER_DEVICE_RESET,
                window_id: window,
                ..Default::default()
            }));
        }
        let swap_interval = super::window::with_window_data(|_, data| {
            data.backup_done = false;
            data.swap_interval
        })
        .unwrap_or(0);

        let _ = gl::gl_set_swap_interval(swap_interval);
    }
}

/// Keep the window's current EGL context (and its swap interval), and
/// release it so the EGL surface can be freed. Translation of
/// `android_egl_context_backup()`.
fn android_egl_context_backup(window: WindowID) {
    if is_opengl(window) {
        // Keep a copy of the EGL Context so we can try to restore it when we resume
        let egl_context = gl::gl_current_context().ok().flatten();
        // Save/Restore the swap interval / vsync
        let interval = gl::gl_get_swap_interval();
        super::window::with_window_data(|_, data| {
            data.egl_context = egl_context;
            if let Ok(interval) = interval {
                data.has_swap_interval = true;
                data.swap_interval = interval;
            }
        });

        // We need to do this so the EGLSurface can be freed
        let _ = gl::make_current_raw(Some(window), None);
        super::window::with_window_data(|_, data| data.backup_done = true);
    }
}

/// `Android_Window && !Android_Window->external_graphics_context`: the
/// driver's window, when SDL manages its graphics context.
fn window_with_sdl_context() -> Option<WindowID> {
    let window = super::window()?;
    let external =
        crate::video::core::with_window(window, |w| w.external_graphics_context).unwrap_or(true);
    (!external).then_some(window)
}

/*
 * Android_ResumeSem and Android_PauseSem are signaled from Java_org_libsdl_app_SDLActivity_nativePause and Java_org_libsdl_app_SDLActivity_nativeResume
 */
/// `Android_EventsInitialized`.
static EVENTS_INITIALIZED: AtomicBool = AtomicBool::new(false);
/// `Android_BlockOnPause`.
static BLOCK_ON_PAUSE: AtomicBool = AtomicBool::new(true);
/// `Android_Paused`.
static PAUSED: AtomicBool = AtomicBool::new(false);
/// `Android_PausedAudio`.
static PAUSED_AUDIO: AtomicBool = AtomicBool::new(false);
/// `Android_Destroyed`.
static DESTROYED: AtomicBool = AtomicBool::new(false);

/// Translation of `Android_InitEvents()` (from `SDL_InitEvents()`).
pub(crate) fn init_events() {
    if !EVENTS_INITIALIZED.load(Ordering::Acquire) {
        BLOCK_ON_PAUSE.store(
            crate::hints::get_bool(crate::hints::ANDROID_BLOCK_ON_PAUSE, true),
            Ordering::Release,
        );
        PAUSED.store(false, Ordering::Release);
        DESTROYED.store(false, Ordering::Release);
        EVENTS_INITIALIZED.store(true, Ordering::Release);
    }
}

/// Translation of `Android_PauseAudio()`.
fn pause_audio() {
    crate::audio::drivers::opensles::pause_devices();
    crate::audio::drivers::aaudio::pause_devices();
    PAUSED_AUDIO.store(true, Ordering::Release);
}

/// Translation of `Android_ResumeAudio()`.
fn resume_audio() {
    if PAUSED_AUDIO.load(Ordering::Acquire) {
        crate::audio::drivers::opensles::resume_devices();
        crate::audio::drivers::aaudio::resume_devices();
        PAUSED_AUDIO.store(false, Ordering::Release);
    }
}

/// Translation of `Android_OnPause()`.
fn on_pause() {
    crate::video::on_application_will_enter_background();
    crate::video::on_application_did_enter_background();

    /* The semantics are that as soon as the enter background event
     * has been queued, the app will block. The application should
     * do any life cycle handling in an event filter while the event
     * was being queued.
     */
    if let Some(window) = window_with_sdl_context() {
        core_android::lock_activity_mutex();
        android_egl_context_backup(window);
        core_android::unlock_activity_mutex();
    }

    if BLOCK_ON_PAUSE.load(Ordering::Acquire) {
        // We're blocking, also pause audio
        pause_audio();
    }

    PAUSED.store(true, Ordering::Release);
}

/// Translation of `Android_OnResume()`.
fn on_resume() {
    PAUSED.store(false, Ordering::Release);

    crate::video::on_application_will_enter_foreground();

    resume_audio();

    // Restore the GL Context from here, as this operation is thread dependent
    if let Some(window) = window_with_sdl_context() {
        if !crate::events::has_event(EventType::QUIT) {
            core_android::lock_activity_mutex();
            android_egl_context_restore(window);
            core_android::unlock_activity_mutex();
        }
    }

    crate::video::on_application_did_enter_foreground();
}

/// Translation of `Android_OnLowMemory()`.
fn on_low_memory() {
    crate::events::queue::send_app_event(EventType::LOW_MEMORY);
}

/// Translation of `Android_OnDestroy()`.
fn on_destroy() {
    // Make sure we unblock any audio processing before we quit
    resume_audio();

    /* Discard previous events. The user should have handled state storage
     * in SDL_EVENT_WILL_ENTER_BACKGROUND. After nativeSendQuit() is called, no
     * events other than SDL_EVENT_QUIT and SDL_EVENT_TERMINATING should fire */
    crate::events::flush_events(EventType::FIRST, EventType::LAST);
    crate::events::send_quit();
    crate::events::queue::send_app_event(EventType::TERMINATING);

    DESTROYED.store(true, Ordering::Release);
}

/// Translation of `Android_HandleLifecycleEvent()`.
fn handle_lifecycle_event(event: LifecycleEvent) {
    match event {
        LifecycleEvent::Wake => {
            // Nothing to do, just return
        }
        LifecycleEvent::Pause => on_pause(),
        LifecycleEvent::Resume => on_resume(),
        LifecycleEvent::LowMemory => on_low_memory(),
        LifecycleEvent::Destroy => on_destroy(),
    }
}

/// How long to wait for a lifecycle event: as long as it takes while
/// paused with the block on pause, at least 100 ms while paused without
/// it, else `timeout`. Translation of `GetLifecycleEventTimeout()`. FIXME
/// (upstream): its `paused` parameter is unused (it reads
/// `Android_Paused`), so after a pause [`pump_events`] blocks at once,
/// where its comment says it returns "to process events one more time
/// before blocking".
fn get_lifecycle_event_timeout(_paused: bool, timeout: Option<Duration>) -> Option<Duration> {
    let mut timeout = timeout;
    if PAUSED.load(Ordering::Acquire) {
        if BLOCK_ON_PAUSE.load(Ordering::Acquire) {
            timeout = None;
        } else if timeout == Some(Duration::ZERO) {
            timeout = Some(Duration::from_millis(100));
        }
    }
    timeout
}

/// Handle the lifecycle events the activity has queued, waiting up to
/// `timeout` (`None`: as long as it takes) for the first, then for the
/// others as [`get_lifecycle_event_timeout`] says. Translation of
/// `Android_PumpEvents()`.
pub(crate) fn pump_events(timeout: Option<Duration>) {
    let mut timeout = timeout;
    let mut paused = PAUSED.load(Ordering::Acquire);

    while !DESTROYED.load(Ordering::Acquire) {
        let Some(event) =
            core_android::wait_lifecycle_event(get_lifecycle_event_timeout(paused, timeout))
        else {
            break;
        };
        handle_lifecycle_event(event);

        match event {
            LifecycleEvent::Wake => {
                // Finish handling events quickly if we're not paused
                timeout = Some(Duration::ZERO);
            }
            LifecycleEvent::Pause => {
                // Finish handling events at the current timeout and return to process events one more time before blocking.
            }
            LifecycleEvent::Resume => {
                // Finish handling events at the resume state timeout
                paused = false;
            }
            _ => {}
        }
    }
}

/// Wait until the app isn't paused, then lock the activity mutex; an
/// error once the activity is destroyed. Translation of
/// `Android_WaitActiveAndLockActivity()` (which the driver's windows use).
pub(crate) fn wait_active_and_lock_activity() -> crate::error::Result<()> {
    /* Make sure we have pumped all events so that Android_Paused state is correct */
    while !DESTROYED.load(Ordering::Acquire) {
        let Some(event) = core_android::wait_lifecycle_event(Some(Duration::ZERO)) else {
            break;
        };
        handle_lifecycle_event(event);
    }

    while PAUSED.load(Ordering::Acquire) && !DESTROYED.load(Ordering::Acquire) {
        pump_events(None);
    }

    if DESTROYED.load(Ordering::Acquire) {
        return Err(crate::error::Error::new(
            "Android activity has been destroyed",
        ));
    }

    core_android::lock_activity_mutex();
    Ok(())
}

/// Translation of `Android_QuitEvents()` (from `SDL_QuitEvents()`).
pub(crate) fn quit_events() {
    EVENTS_INITIALIZED.store(false, Ordering::Release);
}
