// Rust translation of src/video/x11/SDL_x11settings.c and
// SDL_x11settings.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Watching the XSETTINGS desktop settings for content scale changes.

use std::ffi::c_int;

use super::sys::*;
use super::video::X11Video;
use super::xsettings_client::{
    XSettingsAction, XSettingsClient, XSettingsNotification, XSettingsSetting, XSettingsValue,
};
use crate::video::display::{displays, set_display_content_scale};

pub(crate) const SDL_XSETTINGS_GDK_WINDOW_SCALING_FACTOR: &str = "Gdk/WindowScalingFactor";
pub(crate) const SDL_XSETTINGS_GDK_UNSCALED_DPI: &str = "Gdk/UnscaledDPI";
pub(crate) const SDL_XSETTINGS_XFT_DPI: &str = "Xft/DPI";

/// Translation of `SDLX11_SettingsData`.
#[derive(Default)]
pub(crate) struct SettingsData {
    pub(crate) xsettings: Option<XSettingsClient>,
}

impl X11Video {
    /// Translation of `UpdateContentScale()`.
    fn update_content_scale(&self) {
        let scale_factor = self.x11_get_global_content_scale_for_device();
        for display in displays().unwrap_or_default() {
            set_display_content_scale(display, scale_factor);
        }
    }

    /// Translation of `X11_XsettingsNotify()`.
    fn x11_xsettings_notify(
        &self,
        name: &str,
        _action: XSettingsAction,
        _setting: Option<&XSettingsSetting>,
    ) {
        if name == SDL_XSETTINGS_GDK_WINDOW_SCALING_FACTOR
            || name == SDL_XSETTINGS_GDK_UNSCALED_DPI
            || name == SDL_XSETTINGS_XFT_DPI
        {
            self.update_content_scale();
        }
    }

    /// Deliver queued notifications of the client (see
    /// [`XSettingsClient::take_notifications`]).
    fn deliver_xsettings_notifications(&self, notifications: Vec<XSettingsNotification>) {
        for (name, action, setting) in notifications {
            self.x11_xsettings_notify(&name, action, setting.as_ref());
        }
    }

    /// Translation of `X11_InitXsettings()`.
    pub(crate) fn x11_init_xsettings(&self) {
        // SAFETY: the display is open.
        let screen = unsafe { DefaultScreen(self.display) };
        let mut client = XSettingsClient::new(self.x.clone(), self.display, screen, true);
        // (upstream's notify runs inside xsettings_client_new(), before the
        // client is stored, so these see no client)
        self.deliver_xsettings_notifications(client.take_notifications());
        self.with_data(|d| d.xsettings_data.xsettings = Some(client));
    }

    /// Translation of `X11_QuitXsettings()`.
    pub(crate) fn x11_quit_xsettings(&self) {
        let client = self.with_data(|d| d.xsettings_data.xsettings.take());
        // (xsettings_client_destroy(): no watch function to tell)
        drop(client);
    }

    /// Translation of `X11_HandleXsettingsEvent()`.
    pub(crate) fn x11_handle_xsettings_event(&self, xevent: &XEvent) {
        let notifications = self.with_data(|d| {
            d.xsettings_data.xsettings.as_mut().map(|client| {
                client.process_event(xevent);
                client.take_notifications()
            })
        });
        if let Some(notifications) = notifications {
            self.deliver_xsettings_notifications(notifications);
        }
    }

    /// Run `f` with the XSETTINGS client, if there is one (inside the
    /// private data lock: `f` must not call back into SDL).
    pub(crate) fn with_xsettings_client<R>(
        &self,
        f: impl FnOnce(Option<&XSettingsClient>) -> R,
    ) -> R {
        self.with_data(|d| f(d.xsettings_data.xsettings.as_ref()))
    }

    /// Translation of `X11_GetXsettingsIntKey()`.
    #[allow(dead_code)] // (used by the message box toolkit upstream)
    pub(crate) fn x11_get_xsettings_int_key(&self, key: &str, fallback_value: c_int) -> c_int {
        self.with_xsettings_client(|client| {
            x11_get_xsettings_client_int_key(client, key, fallback_value)
        })
    }
}

/// Translation of `X11_GetXsettingsClientIntKey()`.
pub(crate) fn x11_get_xsettings_client_int_key(
    client: Option<&XSettingsClient>,
    key: &str,
    fallback_value: c_int,
) -> c_int {
    let mut res = fallback_value;

    if let Some(client) = client {
        if let Ok(setting) = client.get_setting(key) {
            if let XSettingsValue::Int(v_int) = setting.data {
                res = v_int;
            }
        }
    }

    res
}
