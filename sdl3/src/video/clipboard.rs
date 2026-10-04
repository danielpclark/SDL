// Rust translation of src/video/SDL_clipboard.c and
// include/SDL3/SDL_clipboard.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The clipboard and the primary selection.
//!
//! The app offers clipboard data with [`set_clipboard_data`]: a callback
//! that produces the data for each of its mime types on request. Dropping
//! the callback replaces upstream's cleanup callback; it happens when the
//! data is replaced or the clipboard is lost to another application.
//! Without a backend clipboard (as with the dummy driver), the data stays
//! within the app.

use std::sync::{Arc, Mutex};

use crate::error::{Error, Result};
use crate::events::window::send_clipboard_update;

use super::core::{driver, initialized, uninitialized_video};

/// Produces the clipboard data for a mime type (or `None` if it can't).
/// Translation of `SDL_ClipboardDataCallback` (the closure replaces its
/// `userdata`; dropping it replaces `SDL_ClipboardCleanupCallback`).
pub type ClipboardDataCallback = Arc<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>;

/// The clipboard state of the video device (`clipboard_*`,
/// `primary_selection_text`).
#[derive(Default)]
struct ClipboardState {
    sequence: u32,
    callback: Option<ClipboardDataCallback>,
    mime_types: Vec<String>,
    primary_selection_text: Option<String>,
}

static CLIPBOARD: Mutex<ClipboardState> = Mutex::new(ClipboardState {
    sequence: 0,
    callback: None,
    mime_types: Vec::new(),
    primary_selection_text: None,
});

fn state() -> std::sync::MutexGuard<'static, ClipboardState> {
    CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `SDL_FreeClipboardMimeTypes()`.
fn free_clipboard_mime_types(st: &mut ClipboardState) {
    st.mime_types.clear();
}

/// Drop the app's clipboard data (if `sequence` is 0 or still current).
/// Translation of `SDL_CancelClipboardData()`.
pub(crate) fn cancel_clipboard_data(sequence: u32) {
    if !initialized() {
        return;
    }

    let callback = {
        let mut st = state();
        if sequence != 0 && sequence != st.sequence {
            // This clipboard data was already canceled
            return;
        }

        free_clipboard_mime_types(&mut st);
        st.callback.take()
    };
    // (the cleanup callback: drop the data callback with nothing locked)
    drop(callback);
}

/// The clipboard was taken by another application offering `mime_types`
/// (`SDL_SendClipboardUpdate()` with `owner` false).
pub(crate) fn clipboard_lost(mime_types: &[String]) {
    cancel_clipboard_data(0);
    let _ = save_clipboard_mime_types(mime_types);
}

/// Translation of `SDL_SaveClipboardMimeTypes()`.
pub(crate) fn save_clipboard_mime_types(mime_types: &[String]) -> Result<()> {
    if !initialized() {
        return Err(uninitialized_video());
    }

    let mut st = state();
    free_clipboard_mime_types(&mut st);
    st.mime_types = mime_types.to_vec();
    Ok(())
}

/// The sequence number of the current clipboard data (for backends).
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn clipboard_sequence() -> u32 {
    state().sequence
}

/// The callback of the current clipboard data (`clipboard_callback` and
/// `clipboard_userdata`, for backends).
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn current_clipboard_callback() -> Option<ClipboardDataCallback> {
    state().callback.clone()
}

/// Offer data in the clipboard: `callback` will be asked for the data in
/// any of `mime_types` when someone pastes. `None` and no mime types clear
/// the clipboard. Translation of `SDL_SetClipboardData()`.
pub fn set_clipboard_data(
    callback: Option<ClipboardDataCallback>,
    mime_types: &[&str],
) -> Result<()> {
    if !initialized() {
        return Err(uninitialized_video());
    }

    // Parameter validation
    if callback.is_some() == mime_types.is_empty() {
        return Err(Error::new("Invalid parameters"));
    }

    cancel_clipboard_data(0);

    let mime_types: Vec<String> = mime_types.iter().map(|m| (*m).to_owned()).collect();
    {
        let mut st = state();
        st.sequence = st.sequence.wrapping_add(1);
        if st.sequence == 0 {
            st.sequence = 1;
        }
        st.callback = callback.clone();
    }

    save_clipboard_mime_types(&mime_types)?;

    let driver = driver()?;
    match driver.set_clipboard_data() {
        Some(result) => result?,
        None if driver.implements_clipboard_text() => {
            let mut text = None;
            for mime_type in &mime_types {
                if is_text_mime_type(mime_type) {
                    if let Some(data) = callback.as_ref().and_then(|cb| cb(mime_type)) {
                        text = Some(String::from_utf8_lossy(&data).into_owned());
                        break;
                    }
                }
            }
            if let Some(result) = driver.set_clipboard_text(text.as_deref().unwrap_or("")) {
                result?;
            }
        }
        None => {}
    }

    send_clipboard_update(true, mime_types);
    Ok(())
}

/// Clear the clipboard data. Translation of `SDL_ClearClipboardData()`.
pub fn clear_clipboard_data() -> Result<()> {
    set_clipboard_data(None, &[])
}

/// The app's own clipboard data for a mime type. Translation of
/// `SDL_GetInternalClipboardData()`.
pub(crate) fn internal_clipboard_data(mime_type: &str) -> Option<Vec<u8>> {
    let callback = {
        let st = state();
        let callback = st.callback.clone()?;
        if !st.mime_types.iter().any(|m| m == mime_type) {
            // The callback hasn't advertised that it can handle this mime type
            return None;
        }
        callback
    };
    callback(mime_type)
}

/// The clipboard data for a mime type, if there is any. Translation of
/// `SDL_GetClipboardData()`.
pub fn clipboard_data(mime_type: &str) -> Result<Option<Vec<u8>>> {
    if !initialized() {
        return Err(uninitialized_video());
    }

    let driver = driver()?;
    if let Some(data) = driver.clipboard_data(mime_type) {
        return Ok(data);
    }
    if is_text_mime_type(mime_type) {
        if let Some(text) = driver.clipboard_text() {
            return Ok(text.filter(|t| !t.is_empty()).map(String::into_bytes));
        }
    }
    Ok(internal_clipboard_data(mime_type))
}

/// Whether the app's own clipboard data offers a mime type. Translation of
/// `SDL_HasInternalClipboardData()`.
pub(crate) fn has_internal_clipboard_data(mime_type: &str) -> bool {
    state().mime_types.iter().any(|m| m == mime_type)
}

/// Whether there is clipboard data in a mime type. Translation of
/// `SDL_HasClipboardData()`.
pub fn has_clipboard_data(mime_type: &str) -> Result<bool> {
    if !initialized() {
        return Err(uninitialized_video());
    }

    let driver = driver()?;
    if let Some(has) = driver.has_clipboard_data(mime_type) {
        return Ok(has);
    }
    if is_text_mime_type(mime_type) {
        if let Some(has) = driver.has_clipboard_text() {
            return Ok(has);
        }
    }
    Ok(has_internal_clipboard_data(mime_type))
}

/// The mime types the clipboard currently offers. Translation of
/// `SDL_GetClipboardMimeTypes()`.
pub fn clipboard_mime_types() -> Result<Vec<String>> {
    if !initialized() {
        return Err(uninitialized_video());
    }
    Ok(state().mime_types.clone())
}

// Clipboard text

/// Whether a mime type is a text type. Translation of `SDL_IsTextMimeType()`.
pub(crate) fn is_text_mime_type(mime_type: &str) -> bool {
    mime_type.starts_with("text")
}

/// The mime types clipboard text is offered as. Translation of
/// `SDL_GetTextMimeTypes()`.
fn text_mime_types() -> Vec<String> {
    driver()
        .ok()
        .and_then(|d| d.text_mime_types())
        .unwrap_or_else(|| vec!["text/plain;charset=utf-8".to_owned()])
}

/// Put UTF-8 text into the clipboard (an empty string clears it).
/// Translation of `SDL_SetClipboardText()`.
pub fn set_clipboard_text(text: &str) -> Result<()> {
    if !initialized() {
        return Err(uninitialized_video());
    }

    if !text.is_empty() {
        let mime_types = text_mime_types();
        let mime_refs: Vec<&str> = mime_types.iter().map(String::as_str).collect();

        let text = text.to_owned();
        let callback: ClipboardDataCallback =
            Arc::new(move |_mime_type: &str| Some(text.clone().into_bytes()));
        return set_clipboard_data(Some(callback), &mime_refs);
    }
    clear_clipboard_data()
}

/// The UTF-8 text in the clipboard (empty if there is none). Translation
/// of `SDL_GetClipboardText()`.
pub fn clipboard_text() -> Result<String> {
    if !initialized() {
        return Err(uninitialized_video());
    }

    for mime_type in text_mime_types() {
        if let Ok(Some(data)) = clipboard_data(&mime_type) {
            return Ok(String::from_utf8_lossy(&data).into_owned());
        }
    }
    Ok(String::new())
}

/// Whether the clipboard has text. Translation of `SDL_HasClipboardText()`.
pub fn has_clipboard_text() -> Result<bool> {
    if !initialized() {
        return Err(uninitialized_video());
    }

    for mime_type in text_mime_types() {
        if has_clipboard_data(&mime_type)? {
            return Ok(true);
        }
    }
    Ok(false)
}

// Primary selection text

/// Put UTF-8 text into the primary selection. Translation of
/// `SDL_SetPrimarySelectionText()`.
pub fn set_primary_selection_text(text: &str) -> Result<()> {
    if !initialized() {
        return Err(uninitialized_video());
    }

    match driver()?.set_primary_selection_text(text) {
        Some(result) => result?,
        None => state().primary_selection_text = Some(text.to_owned()),
    }

    let mime_types = state().mime_types.clone();
    send_clipboard_update(true, mime_types);
    Ok(())
}

/// The UTF-8 text in the primary selection (empty if there is none).
/// Translation of `SDL_GetPrimarySelectionText()`.
pub fn primary_selection_text() -> Result<String> {
    if !initialized() {
        return Err(uninitialized_video());
    }

    match driver()?.primary_selection_text() {
        Some(text) => Ok(text),
        None => Ok(state().primary_selection_text.clone().unwrap_or_default()),
    }
}

/// Whether the primary selection has text. Translation of
/// `SDL_HasPrimarySelectionText()`.
pub fn has_primary_selection_text() -> Result<bool> {
    if !initialized() {
        return Err(uninitialized_video());
    }

    match driver()?.has_primary_selection_text() {
        Some(has) => Ok(has),
        None => Ok(state()
            .primary_selection_text
            .as_deref()
            .is_some_and(|t| !t.is_empty())),
    }
}

/// Forget the clipboard state when video quits (upstream frees the device
/// that holds it).
pub(crate) fn quit_clipboard() {
    let old = std::mem::take(&mut *state());
    drop(old);
}
