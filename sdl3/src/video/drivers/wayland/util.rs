// Rust translation of src/video/wayland/SDL_waylandutil.c and
// SDL_waylandutil.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Activation tokens for other applications (opening URLs).

use std::sync::{Arc, Mutex, Weak};

use super::client::AsProxy;
use super::protocols::xdg_activation_v1::*;
use super::video::WaylandVideo;
use crate::video::window::PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_EXPORT_HANDLE_STRING;

const WAYLAND_HANDLE_PREFIX: &str = "wayland:";

/// The Wayland device, while one exists (for `SDL_GetVideoDevice()` from
/// outside the video subsystem).
static CURRENT_DEVICE: Mutex<Weak<WaylandVideo>> = Mutex::new(Weak::new());

/// Remember the device made by `Wayland_CreateDevice()`.
pub(crate) fn set_current_device(device: Weak<WaylandVideo>) {
    *CURRENT_DEVICE.lock().unwrap_or_else(|e| e.into_inner()) = device;
}

/// The current video device, if it is the Wayland one (`SDL_GetVideoDevice()`
/// with a `vid->name` of "wayland").
fn current_device() -> Option<Arc<WaylandVideo>> {
    if crate::video::core::current_video_driver().ok()? != super::video::WAYLANDVID_DRIVER_NAME {
        return None;
    }
    CURRENT_DEVICE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .upgrade()
}

/// Wayland requires an activation token for another application to take
/// focus: the token and the exported window handle (`wayland:<handle>`) to
/// pass along, if any. Translation of `GetActivationToken()` of
/// SDL_sysurl.c.
pub(crate) fn get_activation_token() -> (Option<String>, Option<String>) {
    match current_device() {
        Some(device) => device.wayland_get_activation_token_for_export(),
        None => (None, None),
    }
}

impl WaylandVideo {
    /// Translation of `Wayland_GetActivationTokenForExport()`: the token
    /// and the window ID (each `None` if there isn't one).
    pub(crate) fn wayland_get_activation_token_for_export(
        &self,
    ) -> (Option<String>, Option<String>) {
        let (seat, focus) = self.with_data(|viddata| {
            let seat = viddata.last_implicit_grab_seat;
            let focus = seat
                .and_then(|s| viddata.seat(s))
                .and_then(|s| s.keyboard.focus.or(s.pointer.focus));
            (seat, focus)
        });

        let mut token = None;
        let xdg_activation_token = crate::stdlib::getenv("XDG_ACTIVATION_TOKEN")
            .or_else(crate::notification::notification_activation_token);
        if let Some(t) = xdg_activation_token {
            token = Some(t);

            // Unset the envvar after claiming the token.
            let _ = crate::stdlib::unsetenv_unsafe("XDG_ACTIVATION_TOKEN");
        } else if self.with_data(|d| d.g.activation_manager.is_some()) {
            let Some(queue) = self
                .conn
                .create_queue(c"SDL Activation Token Generation Queue")
            else {
                return (None, None);
            };

            let done: Arc<Mutex<Option<String>>> = Arc::default();
            let mut activation_token = self.with_data(|viddata| {
                let manager = viddata.g.activation_manager.as_ref()?;
                let wrapper = manager.obj().create_wrapper(&queue);
                let mut activation_token = wrapper.get_activation_token();
                drop(wrapper);

                let result = done.clone();
                activation_token.listen(move |_, XdgActivationTokenV1Event::Done { token }| {
                    *result.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(token.to_string_lossy().into_owned());
                });

                let focus_data = focus.and_then(|f| viddata.window(f));
                if let Some(wd) = focus_data {
                    // This specifies the surface from which the activation request is originating, not the activation target surface.
                    activation_token.set_surface(wd.surface());
                }
                if let Some(s) = seat.and_then(|s| viddata.seat(s)) {
                    activation_token.set_serial(s.last_implicit_grab_serial, s.wl_seat.obj());
                }
                if let Some(app_id) = focus_data.map(|wd| wd.app_id.as_str()) {
                    // Set the app ID for external use.
                    activation_token.set_app_id(app_id);
                }
                activation_token.commit();
                Some(activation_token)
            });

            while activation_token.is_some()
                && done.lock().unwrap_or_else(|e| e.into_inner()).is_none()
            {
                // FIXME (upstream): this spins forever if the connection
                // fails before the token arrives (kept as upstream).
                let _ = self.conn.dispatch_queue(&queue);
            }
            // (handle_xdg_activation_done() destroys the token)
            activation_token.take();
            drop(queue);

            token = done.lock().unwrap_or_else(|e| e.into_inner()).take();
            if token.is_none() {
                return (None, None);
            }
        }

        let mut window_id = None;
        if let Some(focus) = focus {
            let id = crate::video::core::with_window(focus, |w| {
                w.properties()
                    .get_string(PROP_WINDOW_WAYLAND_XDG_TOPLEVEL_EXPORT_HANDLE_STRING)
            })
            .ok()
            .flatten();
            if let Some(id) = id {
                window_id = Some(format!("{WAYLAND_HANDLE_PREFIX}{id}"));
            }
        }

        (token, window_id)
    }
}
