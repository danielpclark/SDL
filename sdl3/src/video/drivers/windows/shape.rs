// Rust translation of src/video/windows/SDL_windowsshape.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Shaped windows: a window region built from the shape's alpha channel.

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, HRGN, RGN_OR,
};

use super::window::{adjust_window_rect_for_hwnd, window_data, window_flags};
use crate::core::windows::set_error;
use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::WindowID;
use crate::video::core::with_window;
use crate::video::surface::ScaleMode;
use crate::video::{PixelFormat, Surface};

/// `SDL_ALPHA_TRANSPARENT`
const ALPHA_TRANSPARENT: u8 = 0;

/// Translation of `AddRegion()`.
fn add_region(mask: &mut HRGN, x1: i32, y1: i32, x2: i32, y2: i32) {
    // SAFETY: the regions are GDI objects owned here (or by the window
    // once SetWindowRgn() succeeds).
    unsafe {
        let region = CreateRectRgn(x1, y1, x2, y2);
        if !mask.is_null() {
            CombineRgn(*mask, *mask, region, RGN_OR);
            DeleteObject(region);
        } else {
            *mask = region;
        }
    }
}

/// Translation of `GenerateSpanListRegion()`.
fn generate_span_list_region(shape: &Surface<'_>, offset_x: i32, offset_y: i32) -> HRGN {
    let mut mask: HRGN = std::ptr::null_mut();
    let mut span_start = -1;

    let Some(pixels) = shape.pixels() else {
        return mask;
    };
    let pitch = shape.pitch() as usize;
    for y in 0..shape.height() {
        let row = &pixels[y as usize * pitch..];
        let mut x = 0;
        while x < shape.width() {
            // (the first byte of an ARGB32 pixel is its alpha)
            let a = row[x as usize * 4];
            if a == ALPHA_TRANSPARENT {
                if span_start != -1 {
                    add_region(
                        &mut mask,
                        offset_x + span_start,
                        offset_y + y,
                        offset_x + x,
                        offset_y + y + 1,
                    );
                    span_start = -1;
                }
            } else if span_start == -1 {
                span_start = x;
            }
            x += 1;
        }
        if span_start != -1 {
            // Add the final span
            add_region(
                &mut mask,
                offset_x + span_start,
                offset_y + y,
                offset_x + x,
                offset_y + y + 1,
            );
            span_start = -1;
        }
    }
    mask
}

/// Translation of `WIN_UpdateWindowShape()`.
pub(crate) fn update_window_shape(
    window: WindowID,
    shape: Option<&Surface<'static>>,
) -> Result<()> {
    let Some(data) = window_data(window) else {
        return Err(Error::new("Invalid window"));
    };
    let mut mask: HRGN = std::ptr::null_mut();

    // Generate a set of spans for the region
    if let Some(shape) = shape {
        let (w, h) = with_window(window, |w| (w.core.w, w.core.h))?;
        let stretched;
        let shape: &Surface<'_> = if shape.width() != w || shape.height() != h {
            let mut s = Surface::new(w, h, PixelFormat::ARGB32)?;
            let mut src = shape.duplicate()?;
            src.stretch(None, &mut s, None, ScaleMode::Linear)?;
            stretched = s;
            &stretched
        } else {
            shape
        };

        let mut rect = RECT {
            top: 0,
            left: 0,
            bottom: 0,
            right: 0,
        };
        let borderless = window_flags(window).contains(WindowFlags::BORDERLESS);
        if !borderless {
            let _ = adjust_window_rect_for_hwnd(data.hwnd, &mut rect, 0);
        }

        mask = generate_span_list_region(shape, -rect.left, -rect.top);

        if !borderless {
            let (sw, sh) = (shape.width(), shape.height());
            // Add the window borders
            // top
            add_region(
                &mut mask,
                0,
                0,
                -rect.left + sw + rect.right + 1,
                -rect.top + 1,
            );
            // left
            add_region(&mut mask, 0, -rect.top, -rect.left + 1, -rect.top + sh + 1);
            // right
            add_region(
                &mut mask,
                -rect.left + sw,
                -rect.top,
                -rect.left + sw + rect.right + 1,
                -rect.top + sh + 1,
            );
            // bottom
            add_region(
                &mut mask,
                0,
                -rect.top + sh,
                -rect.left + sw + rect.right + 1,
                -rect.top + sh + rect.bottom + 1,
            );
        }
    }
    // SAFETY: the window's handle; on success the window owns the region.
    if unsafe { SetWindowRgn(data.hwnd, mask, 1) } == 0 {
        // SAFETY: the region is still ours.
        unsafe { DeleteObject(mask) };
        return Err(set_error("SetWindowRgn failed"));
    }
    Ok(())
}
