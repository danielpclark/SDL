// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// SDLSurface.java from Simple DirectMedia Layer (part: what SDLActivity
// calls).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `class SDLSurface extends SurfaceView`, the activity's view: so far
//! the methods `SDLActivity`'s state machine calls on it (`mSurface`),
//! which do nothing, as there's no surface yet. The class (the surface's
//! lifecycle, its metrics, insets, touch, keys and sensors) comes with the
//! android video driver, with what `SDLActivity` does with it (its
//! layout, `createSDLSurface()`, the orientation, the window style, the
//! system UI's visibility).

use super::super::env::{Env, JniResult, Obj};

/// Translation of `mIsSurfaceReady`.
pub(crate) fn is_surface_ready(_env: Env<'_>, _surface: Obj<'_>) -> bool {
    false
}

/// Translation of `handlePause()`.
pub(crate) fn handle_pause<'a>(_env: Env<'a>, _surface: Obj<'_>) -> JniResult<'a, ()> {
    Ok(())
}

/// Translation of `handleResume()`.
pub(crate) fn handle_resume<'a>(_env: Env<'a>, _surface: Obj<'_>) -> JniResult<'a, ()> {
    Ok(())
}

/// Translation of `enableSensor()`.
pub(crate) fn enable_sensor<'a>(
    _env: Env<'a>,
    _surface: Obj<'_>,
    _sensortype: i32,
    _enabled: bool,
) -> JniResult<'a, ()> {
    Ok(())
}
