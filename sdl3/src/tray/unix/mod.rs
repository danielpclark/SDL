// Rust translation of src/tray/unix/SDL_tray.c and SDL_unixtray.h from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Unix tray front end: picks the tray driver (the D-Bus
//! StatusNotifierItem one is the only driver) and updates the trays.

use std::sync::{Arc, Mutex};

use super::{TrayBackend, TrayOptions};
use crate::error::{Error, Result};

mod dbustray;

/// A tray driver. Translation of the driver-wide part of `SDL_TrayDriver`
/// (the per-tray functions are [`TrayBackend`]).
#[derive(Debug)]
pub(super) struct TrayDriver {
    #[allow(dead_code)] // (as upstream: for debugging)
    name: &'static str,

    /// The number of trays created, for unique service names.
    count: u32,
}

/// Translation of `driver`.
static DRIVER: Mutex<Option<TrayDriver>> = Mutex::new(None);

fn driver() -> std::sync::MutexGuard<'static, Option<TrayDriver>> {
    DRIVER.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `SDL_UpdateTrays()`.
pub(super) fn update_trays() {
    let active_trays = super::active_tray_count();
    if active_trays == 0 {
        return;
    }

    let trays = super::valid_trays();
    debug_assert_eq!(trays.len() as i32, active_trays);
    for tray in trays {
        // (still valid: a callback may have destroyed it meanwhile)
        if super::is_tray_valid(&tray) {
            tray.update();
        }
    }
}

/// Translation of `SDL_CreateTrayWithProperties()`.
pub(super) fn create_tray(options: TrayOptions<'_>) -> Result<Arc<dyn TrayBackend>> {
    if !crate::init::is_main_thread() {
        return Err(Error::new(
            "This function should be called on the main thread",
        ));
    }

    create_tray_on_any_thread(options)
}

/// [`create_tray`] without the main thread check.
fn create_tray_on_any_thread(options: TrayOptions<'_>) -> Result<Arc<dyn TrayBackend>> {
    let mut driver = driver();
    if driver.is_none() {
        *driver = Some(dbustray::create_dbus_driver()?);
    }

    match driver.as_mut() {
        // (the caller registers the tray)
        Some(d) => dbustray::create_tray(d, options),
        None => Err(Error::new("Unable to create tray")),
    }
}

/// Destroy the driver once the last tray is gone (the tail of
/// `SDL_DestroyTray()`).
pub(super) fn destroy_driver() {
    let d = driver().take();
    if let Some(d) = d {
        dbustray::destroy_driver(d);
    }
}
