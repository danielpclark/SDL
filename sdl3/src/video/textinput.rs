// Rust translation of the text input and screen keyboard parts of
// src/video/SDL_video.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Text input (with input methods) and the on-screen keyboard.

use crate::error::Result;
use crate::events::{keyboard, Event, EventType, WindowID};
use crate::hints;
use crate::properties::Properties;
use crate::video::Rect;

use super::core::{self, driver, with_device, with_window};
use super::window::Window;

/// The type of text being input (for on-screen keyboards). Translation of
/// `SDL_TextInputType`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum TextInputType {
    /// The input is text
    #[default]
    Text = 0,
    /// The input is a person's name
    TextName,
    /// The input is an e-mail address
    TextEmail,
    /// The input is a username
    TextUsername,
    /// The input is a secure password that is hidden
    TextPasswordHidden,
    /// The input is a secure password that is visible
    TextPasswordVisible,
    /// The input is a number
    Number,
    /// The input is a secure PIN that is hidden
    NumberPasswordHidden,
    /// The input is a secure PIN that is visible
    NumberPasswordVisible,
}

impl TextInputType {
    fn from_i64(v: i64) -> TextInputType {
        use TextInputType::*;
        match v {
            1 => TextName,
            2 => TextEmail,
            3 => TextUsername,
            4 => TextPasswordHidden,
            5 => TextPasswordVisible,
            6 => Number,
            7 => NumberPasswordHidden,
            8 => NumberPasswordVisible,
            _ => Text,
        }
    }
}

/// Auto capitalization type. Translation of `SDL_Capitalization`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Capitalization {
    /// No auto-capitalization will be done
    #[default]
    None = 0,
    /// The first letter of sentences will be capitalized
    Sentences,
    /// The first letter of words will be capitalized
    Words,
    /// All letters will be capitalized
    Letters,
}

/// Translation of `SDL_PROP_TEXTINPUT_TYPE_NUMBER`.
pub const PROP_TEXTINPUT_TYPE_NUMBER: &str = "SDL.textinput.type";
/// Translation of `SDL_PROP_TEXTINPUT_CAPITALIZATION_NUMBER`.
pub const PROP_TEXTINPUT_CAPITALIZATION_NUMBER: &str = "SDL.textinput.capitalization";
/// Translation of `SDL_PROP_TEXTINPUT_AUTOCORRECT_BOOLEAN`.
pub const PROP_TEXTINPUT_AUTOCORRECT_BOOLEAN: &str = "SDL.textinput.autocorrect";
/// Translation of `SDL_PROP_TEXTINPUT_MULTILINE_BOOLEAN`.
pub const PROP_TEXTINPUT_MULTILINE_BOOLEAN: &str = "SDL.textinput.multiline";
/// Translation of `SDL_PROP_TEXTINPUT_TITLE_STRING`.
pub const PROP_TEXTINPUT_TITLE_STRING: &str = "SDL.textinput.title";
/// Translation of `SDL_PROP_TEXTINPUT_PLACEHOLDER_STRING`.
pub const PROP_TEXTINPUT_PLACEHOLDER_STRING: &str = "SDL.textinput.placeholder";
/// Translation of `SDL_PROP_TEXTINPUT_DEFAULT_TEXT_STRING`.
pub const PROP_TEXTINPUT_DEFAULT_TEXT_STRING: &str = "SDL.textinput.default_text";
/// Translation of `SDL_PROP_TEXTINPUT_MAX_LENGTH_NUMBER`.
pub const PROP_TEXTINPUT_MAX_LENGTH_NUMBER: &str = "SDL.textinput.max_length";

/// Translation of `SDL_GetTextInputType()`.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn text_input_type(props: Option<&Properties>) -> TextInputType {
    TextInputType::from_i64(
        props
            .and_then(|p| p.get_number(PROP_TEXTINPUT_TYPE_NUMBER))
            .unwrap_or(TextInputType::Text as i64),
    )
}

/// Translation of `SDL_GetTextInputCapitalization()`.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn text_input_capitalization(props: Option<&Properties>) -> Capitalization {
    if let Some(v) = props.and_then(|p| p.get_number(PROP_TEXTINPUT_CAPITALIZATION_NUMBER)) {
        return match v {
            1 => Capitalization::Sentences,
            2 => Capitalization::Words,
            3 => Capitalization::Letters,
            _ => Capitalization::None,
        };
    }

    match text_input_type(props) {
        TextInputType::Text => Capitalization::Sentences,
        TextInputType::TextName => Capitalization::Words,
        _ => Capitalization::None,
    }
}

/// Translation of `SDL_GetTextInputAutocorrect()`.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn text_input_autocorrect(props: Option<&Properties>) -> bool {
    props
        .and_then(|p| p.get_bool(PROP_TEXTINPUT_AUTOCORRECT_BOOLEAN))
        .unwrap_or(true)
}

/// Translation of `SDL_GetTextInputMultiline()`.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn text_input_multiline(props: Option<&Properties>) -> bool {
    if let Some(multiline) = props.and_then(|p| p.get_bool(PROP_TEXTINPUT_MULTILINE_BOOLEAN)) {
        return multiline;
    }

    !hints::get_bool(hints::RETURN_KEY_HIDES_IME, false)
}

/// Translation of `AutoShowingScreenKeyboard()`.
fn auto_showing_screen_keyboard() -> bool {
    let mut hint = hints::get(hints::ENABLE_SCREEN_KEYBOARD);
    if hint.is_none() {
        // This hint is currently only used by the X11 video driver
        if core::current_video_driver().ok() == Some("x11") {
            hint = hints::get(hints::ENABLE_STEAM_SCREEN_KEYBOARD);
        }
    }
    let auto = hint
        .as_deref()
        .is_none_or(|h| h.eq_ignore_ascii_case("auto"));
    (auto && !keyboard::has_keyboard()) || hints::string_to_bool(hint.as_deref(), false)
}

/// The keyboard focus moved to a window with text input active: restart
/// the backend's text input (`video->StartTextInput` from `SDL_SetKeyboardFocus()`).
pub(crate) fn backend_start_text_input(window: WindowID) {
    let props = with_window(window, |w| w.text_input_props.clone())
        .ok()
        .flatten();
    if let Ok(driver) = driver() {
        let _ = driver.start_text_input(window, props.as_ref());
    }
}

/// The keyboard focus left a window with text input active: commit any
/// composition (`video->StopTextInput` from `SDL_SetKeyboardFocus()`).
pub(crate) fn backend_stop_text_input(window: WindowID) {
    if let Ok(driver) = driver() {
        let _ = driver.stop_text_input(window);
    }
}

/// Whether the platform has on-screen keyboard support. Translation of
/// `SDL_HasScreenKeyboardSupport()`.
pub fn has_screen_keyboard_support() -> bool {
    driver()
        .ok()
        .and_then(|d| d.has_screen_keyboard_support())
        .unwrap_or(false)
}

/// Translation of `SDL_SendScreenKeyboardShown()`.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn send_screen_keyboard_shown() {
    let changed = with_device(|v| {
        if v.screen_keyboard_shown {
            false
        } else {
            v.screen_keyboard_shown = true;
            true
        }
    })
    .unwrap_or(false);
    if changed && crate::events::event_enabled(EventType::SCREEN_KEYBOARD_SHOWN) {
        let _ = crate::events::push(Event::simple(
            EventType::SCREEN_KEYBOARD_SHOWN,
            std::time::Duration::ZERO,
        ));
    }
}

/// Translation of `SDL_SendScreenKeyboardHidden()`.
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn send_screen_keyboard_hidden() {
    let changed = with_device(|v| {
        if v.screen_keyboard_shown {
            v.screen_keyboard_shown = false;
            true
        } else {
            false
        }
    })
    .unwrap_or(false);
    if changed && crate::events::event_enabled(EventType::SCREEN_KEYBOARD_HIDDEN) {
        let _ = crate::events::push(Event::simple(
            EventType::SCREEN_KEYBOARD_HIDDEN,
            std::time::Duration::ZERO,
        ));
    }
}

impl Window {
    /// Start accepting Unicode text input events (`TEXT_INPUT`,
    /// `TEXT_EDITING`) in the window. Translation of `SDL_StartTextInput()`.
    pub fn start_text_input(&self) -> Result<()> {
        self.start_text_input_with_properties(None)
    }

    /// Start text input with properties for the on-screen keyboard and IME
    /// (`PROP_TEXTINPUT_*`). Translation of `SDL_StartTextInputWithProperties()`.
    pub fn start_text_input_with_properties(&self, props: Option<&Properties>) -> Result<()> {
        let window = self.id();
        let copy = match props {
            Some(props) => {
                let copy = Properties::new();
                copy.copy_from(props)?;
                Some(copy)
            }
            None => None,
        };
        with_window(window, |w| w.text_input_props = copy)?;

        let driver = driver()?;
        let _ = driver.set_text_input_properties(window, props);

        if !with_window(window, |w| w.core.text_input_active)? {
            // Start the text input system
            if let Some(Err(e)) = driver.start_text_input(window, props) {
                return Err(e);
            }
            with_window(window, |w| w.core.text_input_active = true)?;
        }

        // Show the on-screen keyboard, if desired
        if auto_showing_screen_keyboard() && !self.screen_keyboard_shown()? {
            let _ = driver.show_screen_keyboard(window, props);
        }
        Ok(())
    }

    /// Whether text input is active in the window. Translation of
    /// `SDL_TextInputActive()`.
    pub fn text_input_active(&self) -> Result<bool> {
        with_window(self.id(), |w| w.core.text_input_active)
    }

    /// Stop receiving text input events in the window. Translation of
    /// `SDL_StopTextInput()`.
    pub fn stop_text_input(&self) -> Result<()> {
        let window = self.id();
        let driver = driver()?;
        if with_window(window, |w| w.core.text_input_active)? {
            // Stop the text input system
            let _ = driver.stop_text_input(window);
            with_window(window, |w| w.core.text_input_active = false)?;
        }

        // Hide the on-screen keyboard, if desired
        if auto_showing_screen_keyboard() && self.screen_keyboard_shown()? {
            let _ = driver.hide_screen_keyboard(window);
        }
        Ok(())
    }

    /// Set the area used to type Unicode text input (`None`: clear it), with
    /// the cursor's offset in it. Translation of `SDL_SetTextInputArea()`.
    pub fn set_text_input_area(&self, rect: Option<&Rect>, cursor: i32) -> Result<()> {
        let window = self.id();
        with_window(window, |w| match rect {
            Some(rect) => {
                w.text_input_rect = *rect;
                w.text_input_cursor = cursor;
            }
            None => {
                w.text_input_rect = Rect::default();
                w.text_input_cursor = 0;
            }
        })?;

        if let Ok(driver) = driver() {
            if let Some(Err(e)) = driver.update_text_input_area(window) {
                return Err(e);
            }
        }
        Ok(())
    }

    /// The area used to type Unicode text input, and the cursor offset.
    /// Translation of `SDL_GetTextInputArea()`.
    pub fn text_input_area(&self) -> Result<(Rect, i32)> {
        with_window(self.id(), |w| (w.text_input_rect, w.text_input_cursor))
    }

    /// Dismiss the composition window/IME without disabling the subsystem.
    /// Translation of `SDL_ClearComposition()`.
    pub fn clear_composition(&self) -> Result<()> {
        with_window(self.id(), |_| ())?;
        driver()?.clear_composition(self.id()).unwrap_or(Ok(()))
    }

    /// Whether the on-screen keyboard is shown. Translation of
    /// `SDL_ScreenKeyboardShown()`.
    pub fn screen_keyboard_shown(&self) -> Result<bool> {
        with_window(self.id(), |_| ())?;
        with_device(|v| v.screen_keyboard_shown)
    }
}
