// Rust translation of src/notification/SDL_notification.c,
// SDL_notification_c.h, src/notification/dummy/SDL_dummynotification.c and
// include/SDL3/SDL_notification.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! System notifications.
//!
//! Only the dummy backend exists so far: showing a notification reports
//! that the operation is unsupported. The platform backends (D-Bus,
//! Windows toasts, the macOS notification center) arrive with the platform
//! layer.

use crate::error::{Error, Result};
use crate::video::surface::Surface;

/// The identifier for a system notification. Translation of `SDL_NotificationID`.
pub type NotificationID = u32;

/// The header icon for notifications (a path), read when the first
/// notification is shown.
/// Translation of `SDL_PROP_GLOBAL_NOTIFICATION_HEADER_ICON_STRING`.
pub const PROP_GLOBAL_NOTIFICATION_HEADER_ICON_STRING: &str = "SDL.notification.header_icon";

/// Notification priorities. Translation of `SDL_NotificationPriority`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum NotificationPriority {
    /// Lowest priority.
    Low = -1,
    /// Normal/medium priority.
    #[default]
    Normal = 0,
    /// High/important priority.
    High = 1,
    /// Highest/critical priority. Note that this may override any "Do Not
    /// Disturb" settings and wake the screen.
    Critical = 2,
}

/// An action users can take on a notification; activations are reported
/// with `NOTIFICATION_ACTION_INVOKED` events. Translation of
/// `SDL_NotificationAction`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotificationAction {
    /// A button with a localized text label, which generates feedback when
    /// activated (`SDL_NOTIFICATION_ACTION_TYPE_BUTTON`).
    Button {
        /// The identifier string for the button. 'default' is a reserved
        /// identifier and must not be used.
        action_id: String,
        /// The localized label for the button associated with the action.
        action_label: String,
    },
}

/// A notification to show. Translation of the `SDL_PROP_NOTIFICATION_*`
/// properties of `SDL_ShowNotificationWithProperties()`.
#[derive(Debug, Default)]
pub struct Notification<'a> {
    /// The title; required (`SDL_PROP_NOTIFICATION_TITLE_STRING`).
    pub title: Option<String>,
    /// The message body (`SDL_PROP_NOTIFICATION_MESSAGE_STRING`).
    pub message: Option<String>,
    /// An image to show (`SDL_PROP_NOTIFICATION_IMAGE_POINTER`).
    pub image: Option<&'a Surface<'a>>,
    /// The actions offered (`SDL_PROP_NOTIFICATION_ACTIONS_POINTER`).
    pub actions: Vec<NotificationAction>,
    /// The priority (`SDL_PROP_NOTIFICATION_PRIORITY_NUMBER`).
    pub priority: NotificationPriority,
    /// A notification this one replaces (`SDL_PROP_NOTIFICATION_REPLACES_NUMBER`).
    pub replaces: Option<NotificationID>,
    /// A sound to play (`SDL_PROP_NOTIFICATION_SOUND_STRING`).
    pub sound: Option<String>,
    /// Whether the notification shouldn't persist
    /// (`SDL_PROP_NOTIFICATION_TRANSIENT_BOOLEAN`).
    pub transient: bool,
}

/// Request permission to show notifications (needed on some platforms).
/// Translation of `SDL_RequestNotificationPermission()`.
pub fn request_notification_permission() -> Result<()> {
    sys_request_notification_permission()
}

/// Show a notification. Translation of `SDL_ShowNotificationWithProperties()`.
pub fn show_notification_with(notification: &Notification<'_>) -> Result<NotificationID> {
    // The title property is required.
    if notification.title.is_none() {
        return Err(Error::new("Notifications must have a title"));
    }

    sys_show_notification(notification)
}

/// Show a notification with a title, an optional message, image and
/// actions. Translation of `SDL_ShowNotification()`.
pub fn show_notification(
    title: Option<&str>,
    message: Option<&str>,
    image: Option<&Surface<'_>>,
    actions: &[NotificationAction],
) -> Result<NotificationID> {
    let Some(title) = title else {
        return Err(Error::new("Notifications must have a title"));
    };
    let notification = Notification {
        title: Some(title.to_string()),
        message: message.map(str::to_string),
        image,
        actions: actions.to_vec(),
        ..Notification::default()
    };
    show_notification_with(&notification)
}

/// Remove a notification. Translation of `SDL_RemoveNotification()`.
pub fn remove_notification(notification: NotificationID) -> Result<()> {
    sys_remove_notification(notification)
}

/// Translation of `SDL_RequestNotificationPermission()` (dummy).
fn sys_request_notification_permission() -> Result<()> {
    Err(Error::unsupported())
}

/// Translation of `SDL_SYS_ShowNotification()` (dummy).
fn sys_show_notification(_notification: &Notification<'_>) -> Result<NotificationID> {
    Err(Error::unsupported())
}

/// Translation of `SDL_RemoveNotification()` (dummy).
fn sys_remove_notification(_notification: NotificationID) -> Result<()> {
    Err(Error::unsupported())
}

/// Free the notification state at shutdown. Translation of
/// `SDL_CleanupNotifications()` (dummy: nothing to do).
pub(crate) fn cleanup_notifications() {
    // Nothing to do.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dummy_notifications() {
        assert_eq!(
            show_notification(None, Some("m"), None, &[])
                .unwrap_err()
                .to_string(),
            "Notifications must have a title"
        );
        assert_eq!(
            show_notification(Some("t"), None, None, &[])
                .unwrap_err()
                .to_string(),
            "That operation is not supported"
        );
        assert!(remove_notification(1).is_err());
        assert!(request_notification_permission().is_err());
    }
}
