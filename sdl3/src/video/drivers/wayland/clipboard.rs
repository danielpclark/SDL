// Rust translation of src/video/wayland/SDL_waylandclipboard.c and
// SDL_waylandclipboard.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The clipboard (`wl_data_device`) and the primary selection
//! (`zwp_primary_selection_device_v1`) entry points.

use std::sync::Arc;

use super::datamanager::*;
use super::video::{VideoData, WaylandVideo};
use crate::error::{Error, Result};
use crate::video::clipboard::{
    clipboard_mime_types, clipboard_sequence, current_clipboard_callback,
    has_internal_clipboard_data, internal_clipboard_data, ClipboardDataCallback,
};

/// The text mime types (`text_mime_types`).
const TEXT_MIME_TYPES: [&str; 5] = [TEXT_MIME, "text/plain", "TEXT", "UTF8_STRING", "STRING"];

/// Translation of `Wayland_GetTextMimeTypes()`.
pub(crate) fn wayland_get_text_mime_types() -> &'static [&'static str] {
    &TEXT_MIME_TYPES
}

/// `SDL_ClipboardTextCallback` over a copy of `text`.
fn clipboard_text_callback(text: String) -> ClipboardDataCallback {
    Arc::new(move |_mime_type: &str| Some(text.clone().into_bytes()))
}

/// The seat to own a selection: the one of the last implicit grab, or if no
/// implicit grab is available yet, the first available seat.
fn selection_seat(video_data: &VideoData) -> Option<u32> {
    video_data
        .last_implicit_grab_seat
        .or_else(|| video_data.seat_list.first().map(|s| s.registry_id))
}

impl WaylandVideo {
    /// Translation of `Wayland_SetClipboardData()`.
    pub(crate) fn wayland_set_clipboard_data(&self) -> Result<()> {
        let callback = current_clipboard_callback();
        let mime_types = clipboard_mime_types().unwrap_or_default();
        let sequence = clipboard_sequence();

        // The sources taken out of the devices, destroyed with nothing borrowed.
        let mut old_sources = Vec::new();
        let result = self.with_data(|video_data| {
            // If no implicit grab is available yet, just attach it to the first available seat.
            let seat = selection_seat(video_data);

            video_data.current_data_offer_seat = seat;

            let Some(seat) =
                seat.filter(|&s| video_data.seat(s).is_some_and(|s| s.data_device.is_some()))
            else {
                return false;
            };

            /* Clear references to the clipboard held by other seats, as they are about to become invalid.
             * For the target seat, the new source data is set before clearing the old source, as some
             * clipboard managers prefer this behavior.
             */
            for s in video_data.seat_list.iter_mut() {
                if let Some(dd) = s.data_device.as_mut() {
                    if s.registry_id != seat {
                        old_sources.extend(dd.selection_source.take());
                    }

                    dd.selection_offer = None;
                }
            }

            if let (Some(callback), false) = (callback, mime_types.is_empty()) {
                let Some(source) = self.wayland_data_source_create(video_data) else {
                    return false;
                };
                wayland_data_source_set_callback(&source, Some(callback), sequence);

                let data_device = video_data
                    .seat_mut(seat)
                    .and_then(|s| s.data_device.as_mut());
                match wayland_data_device_set_selection_source(data_device, source, &mime_types) {
                    Ok(old) => {
                        old_sources.extend(old);
                        true
                    }
                    Err((_, source, old)) => {
                        old_sources.extend(old);
                        old_sources.push(source);
                        false
                    }
                }
            } else {
                if let Some(dd) = video_data
                    .seat_mut(seat)
                    .and_then(|s| s.data_device.as_mut())
                {
                    old_sources.extend(dd.selection_source.take());
                }
                true
            }
        });

        for source in old_sources {
            wayland_data_source_destroy(source);
        }

        if result {
            Ok(())
        } else {
            // (upstream returns false, leaving any error set along the way)
            Err(Error::new("Couldn't set the clipboard data"))
        }
    }

    /// Translation of `Wayland_GetClipboardData()`.
    pub(crate) fn wayland_get_clipboard_data(&self, mime_type: &str) -> Option<Vec<u8>> {
        enum Source {
            Internal,
            Offer(Option<Vec<u8>>),
        }

        let source = self.with_data(|video_data| {
            let seat = video_data.current_data_offer_seat?;
            let data_device = video_data.seat(seat)?.data_device.as_ref()?;
            if data_device.selection_source.is_some() {
                Some(Source::Internal)
            } else if wayland_data_offer_has_mime(data_device.selection_offer.as_ref(), mime_type) {
                let data = self
                    .wayland_data_offer_receive(
                        data_device.selection_offer.as_ref(),
                        mime_type,
                        true,
                    )
                    .ok()
                    .flatten();
                Some(Source::Offer(data))
            } else {
                None
            }
        })?;

        match source {
            Source::Internal => internal_clipboard_data(mime_type),
            Source::Offer(data) => data,
        }
    }

    /// Translation of `Wayland_HasClipboardData()`.
    pub(crate) fn wayland_has_clipboard_data(&self, mime_type: &str) -> bool {
        let internal = self.with_data(|video_data| {
            let seat = video_data.current_data_offer_seat?;
            let data_device = video_data.seat(seat)?.data_device.as_ref()?;
            if data_device.selection_source.is_some() {
                Some(None)
            } else {
                Some(Some(wayland_data_offer_has_mime(
                    data_device.selection_offer.as_ref(),
                    mime_type,
                )))
            }
        });

        match internal {
            Some(None) => has_internal_clipboard_data(mime_type),
            Some(Some(result)) => result,
            None => false,
        }
    }

    /// Translation of `Wayland_SetPrimarySelectionText()`.
    pub(crate) fn wayland_set_primary_selection_text(&self, text: &str) -> Result<()> {
        let mut old_sources = Vec::new();
        let result = self.with_data(|video_data| {
            // If no implicit grab is available yet, just attach it to the first available seat.
            let seat = selection_seat(video_data);

            video_data.current_primary_selection_seat = seat;

            let Some(seat) = seat.filter(|&s| {
                video_data
                    .seat(s)
                    .is_some_and(|s| s.primary_selection_device.is_some())
            }) else {
                return Err(Error::new("Primary selection not supported"));
            };

            if !text.is_empty() {
                let Some(source) = self.wayland_primary_selection_source_create(video_data) else {
                    return Err(Error::new("Primary selection not supported"));
                };
                wayland_primary_selection_source_set_callback(
                    &source,
                    clipboard_text_callback(text.to_owned()),
                );

                let device = video_data
                    .seat_mut(seat)
                    .and_then(|s| s.primary_selection_device.as_mut());
                match wayland_primary_selection_device_set_selection(
                    device,
                    source,
                    &TEXT_MIME_TYPES,
                ) {
                    Ok(old) => {
                        old_sources.extend(old);
                        Ok(())
                    }
                    Err((e, source)) => {
                        old_sources.push(source);
                        Err(e)
                    }
                }
            } else {
                if let Some(device) = video_data
                    .seat_mut(seat)
                    .and_then(|s| s.primary_selection_device.as_mut())
                {
                    old_sources.extend(device.selection_source.take());
                }
                Ok(())
            }
        });

        // (Wayland_PrimarySelectionSourceDestroy(): dropping a source destroys it
        // and its callback)
        drop(old_sources);
        result
    }

    /// Translation of `Wayland_GetPrimarySelectionText()`.
    pub(crate) fn wayland_get_primary_selection_text(&self) -> String {
        let text = self.with_data(|video_data| {
            let seat = video_data.current_primary_selection_seat?;
            let device = video_data.seat(seat)?.primary_selection_device.as_ref()?;
            if device.selection_source.is_some() {
                wayland_primary_selection_source_get_data(
                    device.selection_source.as_ref(),
                    TEXT_MIME,
                )
                .ok()
                .flatten()
            } else {
                for mime_type in TEXT_MIME_TYPES {
                    if wayland_primary_selection_offer_has_mime(
                        device.selection_offer.as_ref(),
                        mime_type,
                    ) {
                        return self
                            .wayland_primary_selection_offer_receive(
                                device.selection_offer.as_ref(),
                                mime_type,
                            )
                            .ok()
                            .flatten();
                    }
                }
                None
            }
        });

        text.map(|t| String::from_utf8_lossy(&t).into_owned())
            .unwrap_or_default()
    }

    /// Translation of `Wayland_HasPrimarySelectionText()`.
    pub(crate) fn wayland_has_primary_selection_text(&self) -> bool {
        self.with_data(|video_data| {
            let Some(device) = video_data
                .current_primary_selection_seat
                .and_then(|seat| video_data.seat(seat))
                .and_then(|s| s.primary_selection_device.as_ref())
            else {
                return false;
            };
            if device.selection_source.is_some() {
                true
            } else {
                wayland_get_text_mime_types().iter().any(|mime_type| {
                    wayland_primary_selection_offer_has_mime(
                        device.selection_offer.as_ref(),
                        mime_type,
                    )
                })
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_mime_types_start_with_utf8() {
        assert_eq!(wayland_get_text_mime_types()[0], "text/plain;charset=utf-8");
        assert_eq!(wayland_get_text_mime_types().len(), 5);
        assert_eq!(
            clipboard_text_callback("hi".into())("TEXT"),
            Some(b"hi".to_vec())
        );
    }
}
