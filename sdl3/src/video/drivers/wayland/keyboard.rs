// Rust translation of src/video/wayland/SDL_waylandkeyboard.c and
// SDL_waylandkeyboard.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Text input through the text-input-unstable-v3 protocol, and the screen
//! keyboard query.

use super::protocols::text_input_unstable_v3::*;
use super::video::WaylandVideo;
use crate::error::Result;
use crate::events::keyboard::Scancode;
use crate::events::WindowID;
use crate::properties::Properties;
use crate::video::core::with_window;
use crate::video::textinput::{
    text_input_autocorrect, text_input_capitalization, text_input_multiline, text_input_type,
    Capitalization, TextInputType,
};
use crate::video::Rect;

/// The text input properties of a window (`text_input_props`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TextInputProps {
    pub(crate) hint: ZwpTextInputV3ContentHint,
    pub(crate) purpose: ZwpTextInputV3ContentPurpose,
    pub(crate) active: bool,
}

/// The text input area of a window, scaled to surface coordinates.
fn scaled_text_input_area(rect: Rect, cursor: i32, pointer_scale: (f64, f64)) -> (Rect, i32) {
    let scaled_rect = Rect {
        x: (rect.x as f64 / pointer_scale.0).floor() as i32,
        y: (rect.y as f64 / pointer_scale.1).floor() as i32,
        w: (rect.w as f64 / pointer_scale.0).ceil() as i32,
        h: (rect.h as f64 / pointer_scale.1).ceil() as i32,
    };
    let scaled_cursor = (cursor as f64 / pointer_scale.0).floor() as i32;
    (scaled_rect, scaled_cursor)
}

impl WaylandVideo {
    /// Translation of `Wayland_InitKeyboard()`.
    pub(crate) fn wayland_init_keyboard(&self) {
        if self.with_data(|d| d.g.text_input_manager.is_none()) {
            crate::core::linux::ime::init();
        }
        let _ = Scancode::APPLICATION.set_name(Some("Menu"));
    }

    /// Translation of `Wayland_QuitKeyboard()`.
    pub(crate) fn wayland_quit_keyboard(&self) {
        if self.with_data(|d| d.g.text_input_manager.is_none()) {
            crate::core::linux::ime::quit();
        }
    }

    /// Translation of `Wayland_SeatUpdateTextInput()`.
    pub(crate) fn wayland_seat_update_text_input(&self, seat: u32) {
        let Some(focus) = self.with_data(|d| {
            let s = d.seat(seat)?;
            s.text_input.zwp_text_input.as_ref()?;
            Some(s.keyboard.focus)
        }) else {
            return;
        };

        // The text input area lives in the window (read before borrowing the device data).
        let area =
            focus.and_then(|w| with_window(w, |w| (w.text_input_rect, w.text_input_cursor)).ok());

        self.with_data(|d| {
            let props_and_scale = focus
                .and_then(|w| d.window(w))
                .filter(|w| w.text_input_props.active)
                .map(|w| (w.text_input_props, (w.pointer_scale.x, w.pointer_scale.y)));
            let Some(s) = d.seat_mut(seat) else {
                return;
            };
            let Some(ti) = s.text_input.zwp_text_input.as_ref() else {
                return;
            };

            if let (Some((props, pointer_scale)), Some((rect, cursor))) = (props_and_scale, area) {
                // Enabling will reset all state, so don't do it redundantly.
                if !s.text_input.enabled {
                    s.text_input.enabled = true;
                    ti.enable();

                    // Now that it's enabled, set the input properties
                    ti.set_content_type(props.hint, props.purpose);
                    if !rect.is_empty() {
                        let (scaled_rect, scaled_cursor) =
                            scaled_text_input_area(rect, cursor, pointer_scale);

                        s.text_input.text_input_rect = scaled_rect;
                        s.text_input.text_input_cursor = scaled_cursor;

                        // Clamp the x value so it doesn't run too far past the end of the text input area.
                        ti.set_cursor_rectangle(
                            (scaled_rect.x + scaled_cursor).min(scaled_rect.x + scaled_rect.w),
                            scaled_rect.y,
                            1,
                            scaled_rect.h,
                        );
                    }
                    ti.commit();

                    if let Some(compose_state) = &s.keyboard.xkb.compose_state {
                        // Reset compose state so composite and dead keys don't carry over
                        compose_state.reset();
                    }
                }
            } else {
                if s.text_input.enabled {
                    s.text_input.enabled = false;
                    s.text_input.text_input_rect = Rect::default();
                    s.text_input.text_input_cursor = 0;
                    ti.disable();
                    ti.commit();
                }

                if let Some(compose_state) = &s.keyboard.xkb.compose_state {
                    // Reset compose state so composite and dead keys don't carry over
                    compose_state.reset();
                }
            }
        });
    }

    /// The seats whose keyboard focus is `window`.
    fn seats_focused_on(&self, window: WindowID) -> Vec<u32> {
        self.with_data(|d| {
            d.seat_list
                .iter()
                .filter(|s| s.keyboard.focus == Some(window))
                .map(|s| s.registry_id)
                .collect()
        })
    }

    /// Translation of `Wayland_StartTextInput()`.
    pub(crate) fn wayland_start_text_input(
        &self,
        window: WindowID,
        props: Option<&Properties>,
    ) -> Result<()> {
        if self.with_data(|d| d.g.text_input_manager.is_some()) {
            type Hint = ZwpTextInputV3ContentHint;
            type Purpose = ZwpTextInputV3ContentPurpose;

            let mut hint = Hint::NONE;
            let purpose = match text_input_type(props) {
                TextInputType::Text => Purpose::NORMAL,
                TextInputType::TextName => Purpose::NAME,
                TextInputType::TextEmail => Purpose::EMAIL,
                TextInputType::TextUsername => {
                    hint |= Hint::SENSITIVE_DATA;
                    Purpose::NORMAL
                }
                TextInputType::TextPasswordHidden => {
                    hint |= Hint::HIDDEN_TEXT | Hint::SENSITIVE_DATA;
                    Purpose::PASSWORD
                }
                TextInputType::TextPasswordVisible => {
                    hint |= Hint::SENSITIVE_DATA;
                    Purpose::PASSWORD
                }
                TextInputType::Number => Purpose::NUMBER,
                TextInputType::NumberPasswordHidden => {
                    hint |= Hint::HIDDEN_TEXT | Hint::SENSITIVE_DATA;
                    Purpose::PIN
                }
                TextInputType::NumberPasswordVisible => {
                    hint |= Hint::SENSITIVE_DATA;
                    Purpose::PIN
                }
            };

            match text_input_capitalization(props) {
                Capitalization::None => {}
                Capitalization::Letters => hint |= Hint::UPPERCASE,
                Capitalization::Words => hint |= Hint::TITLECASE,
                Capitalization::Sentences => hint |= Hint::AUTO_CAPITALIZATION,
            }

            if text_input_autocorrect(props) {
                hint |= Hint::COMPLETION | Hint::SPELLCHECK;
            }
            if text_input_multiline(props) {
                hint |= Hint::MULTILINE;
            }

            self.with_data(|d| {
                if let Some(wind) = d.window_mut(window) {
                    wind.text_input_props = TextInputProps {
                        hint,
                        purpose,
                        active: true,
                    };
                }
            });

            for seat in self.seats_focused_on(window) {
                self.wayland_seat_update_text_input(seat);
            }
        }

        /* Always return true, even if the text input protocol isn't supported, as basic
         * text can still be obtained from keysyms and the composition system.
         */
        Ok(())
    }

    /// Translation of `Wayland_StopTextInput()`.
    pub(crate) fn wayland_stop_text_input(&self, window: WindowID) -> Result<()> {
        if self.with_data(|d| d.g.text_input_manager.is_some()) {
            self.with_data(|d| {
                if let Some(wind) = d.window_mut(window) {
                    wind.text_input_props.active = false;
                }
            });

            for seat in self.seats_focused_on(window) {
                self.wayland_seat_update_text_input(seat);
            }
        } else {
            crate::core::linux::ime::reset();
        }

        Ok(())
    }

    /// Translation of `Wayland_UpdateTextInputArea()`.
    pub(crate) fn wayland_update_text_input_area(&self, window: WindowID) -> Result<()> {
        if self.with_data(|d| d.g.text_input_manager.is_some()) {
            let (rect, cursor) = with_window(window, |w| (w.text_input_rect, w.text_input_cursor))?;
            self.with_data(|d| {
                let Some(pointer_scale) = d
                    .window(window)
                    .map(|w| (w.pointer_scale.x, w.pointer_scale.y))
                else {
                    return;
                };
                for s in d.seat_list.iter_mut() {
                    if s.keyboard.focus != Some(window) {
                        continue;
                    }
                    let Some(ti) = s.text_input.zwp_text_input.as_ref() else {
                        continue;
                    };
                    let (scaled_rect, scaled_cursor) =
                        scaled_text_input_area(rect, cursor, pointer_scale);

                    if scaled_rect != s.text_input.text_input_rect
                        || scaled_cursor != s.text_input.text_input_cursor
                    {
                        s.text_input.text_input_rect = scaled_rect;
                        s.text_input.text_input_cursor = scaled_cursor;

                        // Clamp the x value so it doesn't run too far past the end of the text input area.
                        ti.set_cursor_rectangle(
                            (scaled_rect.x + scaled_cursor).min(scaled_rect.x + scaled_rect.w),
                            scaled_rect.y,
                            1,
                            scaled_rect.h,
                        );
                        ti.commit();
                    }
                }
            });
        } else {
            crate::core::linux::ime::update_text_input_area(Some(window));
        }
        Ok(())
    }

    /// Translation of `Wayland_HasScreenKeyboardSupport()`.
    pub(crate) fn wayland_has_screen_keyboard_support(&self) -> bool {
        /* In reality, we just want to return true when the screen keyboard is the
         * _only_ way to get text input. So, in addition to checking for the text
         * input protocol, make sure we don't have any physical keyboards either.
         */
        self.with_data(|d| {
            let hastextmanager = d.g.text_input_manager.is_some();

            // Check for at least one keyboard object on one seat.
            let haskeyboard = d.seat_list.iter().any(|s| s.keyboard.wl_keyboard.is_some());

            !haskeyboard && hastextmanager
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_input_area_is_scaled_to_surface_units() {
        let (r, c) = scaled_text_input_area(
            Rect {
                x: 11,
                y: 21,
                w: 31,
                h: 41,
            },
            5,
            (2.0, 2.0),
        );
        assert_eq!(
            r,
            Rect {
                x: 5,
                y: 10,
                w: 16,
                h: 21
            }
        );
        assert_eq!(c, 2);
    }
}
