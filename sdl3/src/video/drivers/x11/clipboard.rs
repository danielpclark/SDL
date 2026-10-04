// Rust translation of src/video/x11/SDL_x11clipboard.c and
// SDL_x11clipboard.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The clipboard and the primary selection: owning a selection (with an
//! unmapped window made for it) and converting another owner's selection,
//! including the INCR protocol for large transfers.

use std::ffi::{c_int, c_uchar, c_ulong};
use std::sync::Arc;

use super::sys::*;
use super::video::X11Video;
use crate::error::{Error, Result};
use crate::video::clipboard::{
    clipboard_mime_types, clipboard_sequence, current_clipboard_callback,
    has_internal_clipboard_data, ClipboardDataCallback,
};

const TEXT_MIME_TYPES: [&str; 5] = [
    "UTF8_STRING",
    "text/plain;charset=utf-8",
    "text/plain",
    "TEXT",
    "STRING",
];

/// The data of a selection we own. Translation of `SDLX11_ClipboardData`
/// (the callback carries its `userdata`; dropping the data frees it).
#[derive(Default)]
pub(crate) struct ClipboardData {
    pub(crate) callback: Option<ClipboardDataCallback>,
    pub(crate) mime_types: Vec<String>,
    pub(crate) sequence: u32,
}

/// `SDL_ClipboardTextCallback` over a copy of `text` (`None`: no text).
fn clipboard_text_callback(text: Option<String>) -> ClipboardDataCallback {
    Arc::new(move |_mime_type: &str| text.as_ref().map(|t| t.clone().into_bytes()))
}

/// The text mime types, as `Vec<String>`.
fn text_mime_types() -> Vec<String> {
    TEXT_MIME_TYPES.iter().map(|m| (*m).to_owned()).collect()
}

/// Translation of `X11_GetTextMimeTypes()`.
pub(crate) fn x11_get_text_mime_types() -> Vec<String> {
    text_mime_types()
}

/// Translation of `CloneDataBuffer()`: a copy of the data (none if empty).
fn clone_data_buffer(buffer: &[u8]) -> Option<Vec<u8>> {
    if !buffer.is_empty() {
        // (upstream adds 4 zero bytes after the data)
        Some(buffer.to_vec())
    } else {
        Option::None
    }
}

impl X11Video {
    /// Get any application owned window handle for clipboard association.
    /// Translation of `GetWindow()`.
    pub(crate) fn get_window(&self) -> Window {
        let existing = self.with_data(|d| d.clipboard_window);

        /* We create an unmapped window that exists just to manage the clipboard,
        since X11 selection data is tied to a specific window and dies with it.
        We create the window on demand, so apps that don't use the clipboard
        don't have to keep an unnecessary resource around. */
        if existing == None {
            let dpy = self.display;
            // SAFETY: the display is open; xattr is unused (no value mask).
            let window = unsafe {
                let parent = RootWindow(dpy, DefaultScreen(dpy));
                let mut xattr: XSetWindowAttributes = std::mem::zeroed();
                let window = (self.x.XCreateWindow)(
                    dpy,
                    parent,
                    -10,
                    -10,
                    1,
                    1,
                    0,
                    CopyFromParent as c_int,
                    InputOnly as _,
                    std::ptr::null_mut(),
                    0,
                    &mut xattr,
                );

                (self.x.XSelectInput)(dpy, window, PropertyChangeMask);
                (self.x.XFlush)(dpy);
                window
            };
            self.with_data(|d| d.clipboard_window = window);
            return window;
        }

        existing
    }

    /// Translation of `SetSelectionData()`.
    fn set_selection_data(
        &self,
        selection: Atom,
        callback: Option<ClipboardDataCallback>,
        mime_types: Vec<String>,
        sequence: u32,
    ) -> Result<()> {
        let display = self.display;

        let window = self.get_window();
        if window == None {
            return Err(Error::new("Couldn't find a window to own the selection"));
        }

        // SAFETY: the display is open.
        let clipboard_owner = unsafe { (self.x.XGetSelectionOwner)(display, selection) } == window;

        let old = self.with_data(|d| {
            let clipboard = if selection == XA_PRIMARY {
                &mut d.primary_selection
            } else {
                &mut d.clipboard
            };

            // If we are canceling our own data we need to clean it up
            // (dropping the old data does, whatever the owner: the
            // callback's userdata belongs to it)
            let _ = clipboard_owner;

            std::mem::replace(
                clipboard,
                ClipboardData {
                    callback,
                    mime_types,
                    sequence,
                },
            )
        });
        drop(old);

        // SAFETY: the display is open; the window is the clipboard window.
        unsafe {
            (self.x.XSetSelectionOwner)(display, selection, window, CurrentTime);
        }
        Ok(())
    }

    /// Translation of `WaitForSelection()`: pump events until `flag` (a
    /// flag of the device data) is cleared, for up to a second.
    fn wait_for_selection(
        &self,
        selection_type: Atom,
        flag: fn(&mut super::video::VideoData) -> &mut bool,
    ) -> bool {
        let wait_start = crate::timer::ticks_ms();
        self.with_data(|d| *flag(d) = true);
        while self.with_data(|d| *flag(d)) {
            self.x11_pump_events();
            let wait_elapsed = crate::timer::ticks_ms() - wait_start;
            // Wait one second for a selection response.
            if wait_elapsed > 1000 {
                self.with_data(|d| *flag(d) = false);
                // (SDL_SetError("Selection timeout"): the callers return no data)
                /* We need to set the selection text so that next time we won't
                timeout, otherwise we will hang on every call to this function. */
                let _ = self.set_selection_data(
                    selection_type,
                    Some(clipboard_text_callback(Option::None)),
                    text_mime_types(),
                    0,
                );
                return false;
            }
        }

        true
    }

    /// Translation of `GetSelectionData()`.
    fn get_selection_data(&self, selection_type: Atom, mime_type: &str) -> Option<Vec<u8>> {
        let display = self.display;
        let x = &self.x;
        let atoms = self.atoms();
        let mut seln_type: Atom = 0;
        let mut seln_format: c_int = 0;
        let mut count: c_ulong = 0;
        let mut overflow: c_ulong = 0;

        let mut data: Option<Vec<u8>> = Option::None;
        let mut src: *mut c_uchar = std::ptr::null_mut();
        let mut incr_success = false;
        let xa_mime = self.intern_atom(mime_type, false);

        // Get the window that holds the selection
        let window = self.get_window();
        // SAFETY: the display is open.
        let owner = unsafe { (x.XGetSelectionOwner)(display, selection_type) };
        if owner == None {
            // This requires a fallback to ancient X10 cut-buffers. We will just skip those for now
            data = Option::None;
        } else if owner == window {
            // (owner = DefaultRootWindow(display), unused afterwards)
            let callback = self.with_data(|d| {
                if selection_type == XA_PRIMARY {
                    d.primary_selection.callback.clone()
                } else {
                    d.clipboard.callback.clone()
                }
            });

            if let Some(callback) = callback {
                let clipboard_data = callback(mime_type);
                data = clipboard_data.as_deref().and_then(clone_data_buffer);
            }
        } else {
            // Request that the selection owner copy the data to our window
            let owner = window;
            let selection = atoms.SDL_SELECTION;
            // SAFETY: the display is open; the window is ours.
            unsafe {
                (x.XConvertSelection)(
                    display,
                    selection_type,
                    xa_mime,
                    selection,
                    owner,
                    CurrentTime,
                );
            }

            if !self.wait_for_selection(selection_type, |d| &mut d.selection_waiting) {
                data = Option::None;
            }

            // SAFETY: the display is open; the out-parameters are valid.
            let status = unsafe {
                (x.XGetWindowProperty)(
                    display,
                    owner,
                    selection,
                    0,
                    (c_int::MAX / 4) as _,
                    False,
                    xa_mime,
                    &mut seln_type,
                    &mut seln_format,
                    &mut count,
                    &mut overflow,
                    &mut src,
                )
            };
            if status == Success {
                let bytes = |src: *mut c_uchar, count: c_ulong| -> &[u8] {
                    if src.is_null() {
                        &[]
                    } else {
                        // SAFETY: format-8 property data of `count` bytes,
                        // freed only after the bytes are copied.
                        unsafe { std::slice::from_raw_parts(src, count as usize) }
                    }
                };
                if seln_type == xa_mime {
                    data = clone_data_buffer(bytes(src, count));
                } else if seln_type == atoms.INCR {
                    loop {
                        // Only delete the property after being done with the previous "chunk".
                        // SAFETY: the display is open; the window is ours.
                        unsafe {
                            (x.XDeleteProperty)(display, owner, selection);
                            (x.XFlush)(display);
                        }

                        if !self
                            .wait_for_selection(selection_type, |d| &mut d.selection_incr_waiting)
                        {
                            break;
                        }

                        // SAFETY: as above; the previous chunk is freed first.
                        let status = unsafe {
                            (x.XFree)(src.cast());
                            (x.XGetWindowProperty)(
                                display,
                                owner,
                                selection,
                                0,
                                (c_int::MAX / 4) as _,
                                False,
                                xa_mime,
                                &mut seln_type,
                                &mut seln_format,
                                &mut count,
                                &mut overflow,
                                &mut src,
                            )
                        };
                        if status != Success {
                            break;
                        }

                        if count == 0 {
                            incr_success = true;
                            break;
                        }

                        match &mut data {
                            Option::None => data = clone_data_buffer(bytes(src, count)),
                            Some(buffer) => buffer.extend_from_slice(bytes(src, count)),
                        }

                        if data.is_none() {
                            break;
                        }
                    }

                    if !incr_success {
                        data = Option::None;
                    }
                }
                // SAFETY: the property data came from Xlib (XFree accepts NULL).
                unsafe {
                    (x.XFree)(src.cast());
                }
            }
        }
        data
    }

    /// Translation of `X11_SetClipboardData()`: own the clipboard with the
    /// video core's current data.
    pub(crate) fn x11_set_clipboard_data(&self) -> Result<()> {
        let atoms = self.atoms();
        let mime_types = clipboard_mime_types().unwrap_or_default();
        self.set_selection_data(
            atoms.CLIPBOARD,
            current_clipboard_callback(),
            mime_types,
            clipboard_sequence(),
        )
    }

    /// Translation of `X11_GetClipboardData()`.
    pub(crate) fn x11_get_clipboard_data(&self, mime_type: &str) -> Option<Vec<u8>> {
        if !has_internal_clipboard_data(mime_type) {
            // This mime type wasn't advertised by the last selection owner.
            // The atom might still have data, but it's stale, so ignore it.
            return Option::None;
        }
        self.get_selection_data(self.atoms().CLIPBOARD, mime_type)
    }

    /// Translation of `X11_HasClipboardData()`.
    pub(crate) fn x11_has_clipboard_data(&self, mime_type: &str) -> bool {
        let data = self.x11_get_clipboard_data(mime_type);
        data.is_some_and(|d| !d.is_empty())
    }

    /// Translation of `X11_SetPrimarySelectionText()`.
    pub(crate) fn x11_set_primary_selection_text(&self, text: &str) -> Result<()> {
        self.set_selection_data(
            XA_PRIMARY,
            Some(clipboard_text_callback(Some(text.to_owned()))),
            text_mime_types(),
            0,
        )
    }

    /// Translation of `X11_GetPrimarySelectionText()`.
    pub(crate) fn x11_get_primary_selection_text(&self) -> String {
        let text = self.get_selection_data(XA_PRIMARY, TEXT_MIME_TYPES[0]);
        match text {
            // (the data is used as a C string)
            Some(bytes) => {
                let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
                String::from_utf8_lossy(&bytes[..end]).into_owned()
            }
            Option::None => String::new(),
        }
    }

    /// Translation of `X11_HasPrimarySelectionText()`.
    pub(crate) fn x11_has_primary_selection_text(&self) -> bool {
        let text = self.x11_get_primary_selection_text();
        !text.is_empty()
    }

    /// Translation of `X11_QuitClipboard()`.
    pub(crate) fn x11_quit_clipboard(&self) {
        let (primary, clipboard) = self.with_data(|d| {
            (
                std::mem::take(&mut d.primary_selection),
                std::mem::take(&mut d.clipboard),
            )
        });
        // (dropping the data frees the userdata of sequence 0 data; the
        // video core's data is freed by the core)
        drop(primary);
        drop(clipboard);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_callback() {
        let cb = clipboard_text_callback(Some("hi".into()));
        assert_eq!(cb("text/plain").as_deref(), Some(&b"hi"[..]));
        assert!(clipboard_text_callback(Option::None)("TEXT").is_none());
        assert_eq!(x11_get_text_mime_types()[0], "UTF8_STRING");
        assert!(clone_data_buffer(b"").is_none());
    }
}
