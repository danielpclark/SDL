// Rust translation of src/core/linux/SDL_ime.c from Simple DirectMedia
// Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The input method layer the X11 (and KMS/DRM) video drivers use: Fcitx
//! when the environment asks for it, else IBus. Upstream picks function
//! pointers; here an [`Ime`] says which backend is active.
//!
//! Where upstream's backends translate the window's origin to root window
//! coordinates with `X11_XTranslateCoordinates()`, the X11 video driver
//! installs a [`WindowOriginHook`] instead.

use std::sync::Mutex;

use crate::events::WindowID;
use crate::video::rect::Rect;

use super::fcitx;
#[cfg(target_os = "linux")]
use super::ibus;

/// The active input method backend (upstream's `SDL_IME_*_Real` pointers).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ime {
    Fcitx,
    #[cfg(target_os = "linux")]
    IBus,
}

struct State {
    inited: bool,
    real: Option<Ime>,
}

static STATE: Mutex<State> = Mutex::new(State {
    inited: false,
    real: None,
});

fn real() -> Option<Ime> {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).real
}

/// Where a window's origin is in root window coordinates, if the video
/// driver knows better than the window's position (upstream asks the X11
/// server with `X11_XTranslateCoordinates()`).
pub(crate) type WindowOriginHook = fn(WindowID) -> Option<(i32, i32)>;

static WINDOW_ORIGIN_HOOK: Mutex<Option<WindowOriginHook>> = Mutex::new(None);

/// Install (or remove) the [`WindowOriginHook`]; the X11 video driver
/// installs one translating with `XTranslateCoordinates()`.
pub(crate) fn set_window_origin_hook(hook: Option<WindowOriginHook>) {
    *WINDOW_ORIGIN_HOOK.lock().unwrap_or_else(|e| e.into_inner()) = hook;
}

/// The input cursor of a window, in window coordinates: a square at the
/// text input cursor location; and the window's origin. `None` if the
/// window is gone. (The part of `SDL_IBus_UpdateTextInputArea()` and
/// `SDL_Fcitx_UpdateTextInputArea()` that reads the window.)
pub(super) fn window_input_cursor(window: WindowID) -> Option<(Rect, (i32, i32))> {
    let w = crate::video::Window::from_id(window).ok()?;
    let (text_input_rect, text_input_cursor) = w.text_input_area().ok()?;

    // We'll use a square at the text input cursor location for the cursor
    let cursor = Rect {
        x: text_input_rect.x + text_input_cursor,
        y: text_input_rect.y,
        w: text_input_rect.h,
        h: text_input_rect.h,
    };

    let mut origin = w.position().unwrap_or((0, 0));

    let hook = *WINDOW_ORIGIN_HOOK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(translated) = hook.and_then(|hook| hook(window)) {
        origin = translated;
    }

    Some((cursor, origin))
}

/// Translation of `InitIME()`.
fn init_ime(state: &mut State) {
    if state.inited {
        return;
    }

    state.inited = true;

    // See if fcitx IME support is being requested
    let im_module = crate::stdlib::getenv("SDL_IM_MODULE");
    let xmodifiers = crate::stdlib::getenv("XMODIFIERS");
    if state.real.is_none()
        && (im_module.as_deref() == Some("fcitx")
            || (im_module.is_none() && xmodifiers.is_some_and(|x| x.contains("@im=fcitx"))))
    {
        state.real = Some(Ime::Fcitx);
    }

    // default to IBus
    #[cfg(target_os = "linux")]
    if state.real.is_none() {
        state.real = Some(Ime::IBus);
    }
}

/// Start the input method; false if there is none (IME support is then
/// off). Translation of `SDL_IME_Init()`.
pub(crate) fn init() -> bool {
    let ime = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        init_ime(&mut state);
        state.real
    };

    if let Some(ime) = ime {
        let ok = match ime {
            Ime::Fcitx => fcitx::init(),
            #[cfg(target_os = "linux")]
            Ime::IBus => ibus::init(),
        };
        if ok {
            return true;
        }

        // uhoh, the IME implementation's init failed! Disable IME support.
        STATE.lock().unwrap_or_else(|e| e.into_inner()).real = None;
    }

    false
}

/// Translation of `SDL_IME_Quit()`.
pub(crate) fn quit() {
    match real() {
        Some(Ime::Fcitx) => fcitx::quit(),
        #[cfg(target_os = "linux")]
        Some(Ime::IBus) => ibus::quit(),
        None => {}
    }
}

/// Tell the input method about window focus changes. Translation of
/// `SDL_IME_SetFocus()`.
pub(crate) fn set_focus(focused: bool) {
    match real() {
        Some(Ime::Fcitx) => fcitx::set_focus(focused),
        #[cfg(target_os = "linux")]
        Some(Ime::IBus) => ibus::set_focus(focused),
        None => {}
    }
}

/// Close the candidate list and reset the text being composed.
/// Translation of `SDL_IME_Reset()`.
pub(crate) fn reset() {
    match real() {
        Some(Ime::Fcitx) => fcitx::reset(),
        #[cfg(target_os = "linux")]
        Some(Ime::IBus) => ibus::reset(),
        None => {}
    }
}

/// Send a key event (an X keysym and keycode) to the input method; true if
/// it used the event. [`pump_events`] delivers the text input and editing
/// it causes. Translation of `SDL_IME_ProcessKeyEvent()`.
pub(crate) fn process_key_event(keysym: u32, keycode: u32, down: bool) -> bool {
    match real() {
        Some(Ime::Fcitx) => fcitx::process_key_event(keysym, keycode, down),
        #[cfg(target_os = "linux")]
        Some(Ime::IBus) => ibus::process_key_event(keysym, keycode, down),
        None => false,
    }
}

/// Move the input method's candidate list to the window's text input
/// area. Translation of `SDL_IME_UpdateTextInputArea()`.
pub(crate) fn update_text_input_area(window: Option<WindowID>) {
    match real() {
        Some(Ime::Fcitx) => fcitx::update_text_input_area(window),
        #[cfg(target_os = "linux")]
        Some(Ime::IBus) => ibus::update_text_input_area(window),
        None => {}
    }
}

/// Dispatch the input method's messages: text input and editing events.
/// Translation of `SDL_IME_PumpEvents()`.
pub(crate) fn pump_events() {
    match real() {
        Some(Ime::Fcitx) => fcitx::pump_events(),
        #[cfg(target_os = "linux")]
        Some(Ime::IBus) => ibus::pump_events(),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_fcitx_from_the_environment() {
        let _l = crate::test_support::test_lock();
        let env = crate::stdlib::Environment::process();
        let saved = (env.get("SDL_IM_MODULE"), env.get("XMODIFIERS"));
        let pick = |im: Option<&str>, xmod: Option<&str>| {
            for (name, value) in [("SDL_IM_MODULE", im), ("XMODIFIERS", xmod)] {
                match value {
                    Some(v) => env.set(name, v, true).unwrap(),
                    None => env.unset(name).unwrap(),
                }
            }
            let mut state = State {
                inited: false,
                real: None,
            };
            init_ime(&mut state);
            state.real
        };
        let ibus = if cfg!(target_os = "linux") {
            pick(None, None)
        } else {
            None
        };
        assert_eq!(pick(Some("fcitx"), None), Some(Ime::Fcitx));
        assert_eq!(pick(None, Some("@im=fcitx")), Some(Ime::Fcitx));
        assert_eq!(pick(Some("ibus"), Some("@im=fcitx")), ibus);
        assert_eq!(pick(None, Some("@im=ibus")), ibus);
        for (name, value) in [("SDL_IM_MODULE", saved.0), ("XMODIFIERS", saved.1)] {
            match value {
                Some(v) => env.set(name, &v, true).unwrap(),
                None => env.unset(name).unwrap(),
            }
        }
    }
}
