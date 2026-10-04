// Rust translation of the message box parts of src/video/SDL_video.c and
// include/SDL3/SDL_messagebox.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Modal message boxes.
//!
//! A message box is shown by the video driver if it can, otherwise by any
//! video backend that can show one without being initialized. The dummy and
//! offscreen drivers can't, so until a platform backend exists this fails
//! with "No message system available".

use std::sync::atomic::{AtomicI32, Ordering};

use crate::error::{Error, Result};
use crate::events::{keyboard, mouse};
use crate::hints;

use super::core::{self, driver, initialized};
use super::window::Window;

/// Message box flags. Translation of `SDL_MessageBoxFlags`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct MessageBoxFlags(pub u32);

impl MessageBoxFlags {
    pub const NONE: MessageBoxFlags = MessageBoxFlags(0);
    /// error dialog
    pub const ERROR: MessageBoxFlags = MessageBoxFlags(0x00000010);
    /// warning dialog
    pub const WARNING: MessageBoxFlags = MessageBoxFlags(0x00000020);
    /// informational dialog
    pub const INFORMATION: MessageBoxFlags = MessageBoxFlags(0x00000040);
    /// buttons placed left to right
    pub const BUTTONS_LEFT_TO_RIGHT: MessageBoxFlags = MessageBoxFlags(0x00000080);
    /// buttons placed right to left
    pub const BUTTONS_RIGHT_TO_LEFT: MessageBoxFlags = MessageBoxFlags(0x00000100);

    pub const fn contains(self, other: MessageBoxFlags) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for MessageBoxFlags {
    type Output = MessageBoxFlags;
    fn bitor(self, rhs: MessageBoxFlags) -> MessageBoxFlags {
        MessageBoxFlags(self.0 | rhs.0)
    }
}

/// Message box button flags. Translation of `SDL_MessageBoxButtonFlags`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct MessageBoxButtonFlags(pub u32);

impl MessageBoxButtonFlags {
    pub const NONE: MessageBoxButtonFlags = MessageBoxButtonFlags(0);
    /// Marks the default button when return is hit
    pub const RETURNKEY_DEFAULT: MessageBoxButtonFlags = MessageBoxButtonFlags(0x00000001);
    /// Marks the default button when escape is hit
    pub const ESCAPEKEY_DEFAULT: MessageBoxButtonFlags = MessageBoxButtonFlags(0x00000002);

    pub const fn contains(self, other: MessageBoxButtonFlags) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for MessageBoxButtonFlags {
    type Output = MessageBoxButtonFlags;
    fn bitor(self, rhs: MessageBoxButtonFlags) -> MessageBoxButtonFlags {
        MessageBoxButtonFlags(self.0 | rhs.0)
    }
}

/// A message box button. Translation of `SDL_MessageBoxButtonData`.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct MessageBoxButtonData {
    pub flags: MessageBoxButtonFlags,
    /// User defined button id (value returned via [`show_message_box`])
    pub button_id: i32,
    /// The UTF-8 button text
    pub text: String,
}

/// An RGB value used in a message box color scheme. Translation of
/// `SDL_MessageBoxColor`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct MessageBoxColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// The parts of a message box a color scheme colors. Translation of
/// `SDL_MessageBoxColorType`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MessageBoxColorType {
    Background,
    Text,
    ButtonBorder,
    ButtonBackground,
    ButtonSelected,
}

/// Translation of `SDL_MESSAGEBOX_COLOR_COUNT`.
pub const MESSAGEBOX_COLOR_COUNT: usize = 5;

/// A set of colors to use for message box dialogs, indexed by
/// [`MessageBoxColorType`]. Translation of `SDL_MessageBoxColorScheme`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct MessageBoxColorScheme {
    pub colors: [MessageBoxColor; MESSAGEBOX_COLOR_COUNT],
}

/// A modal message box. Translation of `SDL_MessageBoxData`.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct MessageBoxData {
    pub flags: MessageBoxFlags,
    /// Parent window, can be `None`
    pub window: Option<Window>,
    /// UTF-8 title
    pub title: String,
    /// UTF-8 message text
    pub message: String,
    pub buttons: Vec<MessageBoxButtonData>,
    /// `None` to use system settings
    pub color_scheme: Option<MessageBoxColorScheme>,
}

/// Translation of `SDL_messagebox_count`.
static MESSAGEBOX_COUNT: AtomicI32 = AtomicI32::new(0);

/// The number of message boxes being shown. Translation of
/// `SDL_GetMessageBoxCount()`.
pub(crate) fn message_box_count() -> i32 {
    MESSAGEBOX_COUNT.load(Ordering::Acquire)
}

/// Show a modal message box (blocking), returning the ID of the button
/// pressed. This works before video is initialized. Translation of
/// `SDL_ShowMessageBox()`.
pub fn show_message_box(messageboxdata: &MessageBoxData) -> Result<i32> {
    MESSAGEBOX_COUNT.fetch_add(1, Ordering::AcqRel);

    let current_window = keyboard::keyboard_focus();
    let _ = mouse::update_mouse_capture(false);
    let _ = mouse::set_relative_mouse_mode(false);
    let show_cursor_prev = mouse::cursor_visible();
    mouse::show_cursor();
    keyboard::reset_keyboard();

    let mut result = None;
    if let Some(r) = driver()
        .ok()
        .and_then(|d| d.show_message_box(messageboxdata))
    {
        result = Some(r);
    } else {
        // It's completely fine to call this function before video is initialized
        let bootstrap = core::video_bootstraps();
        match hints::get(hints::VIDEO_DRIVER).as_deref() {
            Some(driver_name) if !driver_name.is_empty() => {
                for driver_attempt in driver_name.split(',') {
                    if matches!(result, Some(Ok(_))) || driver_attempt.is_empty() {
                        break;
                    }
                    if let Some(show) = bootstrap
                        .iter()
                        .filter(|b| b.name.eq_ignore_ascii_case(driver_attempt))
                        .find_map(|b| b.show_message_box)
                    {
                        result = Some(show(messageboxdata));
                    }
                }
            }
            _ => {
                for show in bootstrap.iter().filter_map(|b| b.show_message_box) {
                    let r = show(messageboxdata);
                    let ok = r.is_ok();
                    result = Some(r);
                    if ok {
                        break;
                    }
                }
            }
        }
    }

    let result = result.unwrap_or_else(|| Err(Error::new("No message system available")));

    MESSAGEBOX_COUNT.fetch_sub(1, Ordering::AcqRel);

    if let Some(window) = current_window {
        if initialized() {
            let _ = Window::from_raw(window).raise();
        }
    }

    if !show_cursor_prev {
        mouse::hide_cursor();
    }
    let _ = mouse::update_relative_mouse_mode();
    let _ = mouse::update_mouse_capture(false);

    result
}

/// Show a simple modal message box with an OK button. Translation of
/// `SDL_ShowSimpleMessageBox()`.
pub fn show_simple_message_box(
    flags: MessageBoxFlags,
    title: &str,
    message: &str,
    window: Option<&Window>,
) -> Result<()> {
    let data = MessageBoxData {
        flags,
        title: title.to_owned(),
        message: message.to_owned(),
        buttons: vec![MessageBoxButtonData {
            flags: MessageBoxButtonFlags::RETURNKEY_DEFAULT
                | MessageBoxButtonFlags::ESCAPEKEY_DEFAULT,
            button_id: 0,
            text: "OK".to_owned(),
        }],
        window: window.copied(),
        color_scheme: None,
    };

    show_message_box(&data).map(|_| ())
}
