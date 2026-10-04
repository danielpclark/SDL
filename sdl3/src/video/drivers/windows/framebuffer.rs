// Rust translation of src/video/windows/SDL_windowsframebuffer.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The window framebuffer of the Windows video driver: a GDI DIB section
//! selected into a memory DC, blitted to the window.
//!
//! Upstream hands the app the DIB section's own pixels. Here the window
//! surface owns its pixels (the front end keeps it behind a lock the app
//! shares), so [`update_window_framebuffer`] first copies the updated
//! rectangles into the DIB section, then blits them like upstream.

use std::ffi::c_void;

use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
    GetDIBits, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_BITFIELDS, BI_RGB, DIB_RGB_COLORS,
    RGBQUAD, SRCCOPY,
};

use super::modes::format_for_masks;
use super::window::{window_data, window_flags};
use crate::core::windows::set_error;
use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::WindowID;
use crate::video::{PixelFormat, Rect, Surface};

/// A `BITMAPINFO` with room for 256 colors (or the bitfield masks).
#[repr(C)]
struct BitmapInfo256 {
    header: BITMAPINFOHEADER,
    colors: [RGBQUAD; 256],
}

/// Translation of `WIN_CreateWindowFramebuffer()`.
pub(crate) fn create_window_framebuffer(
    window: WindowID,
    w: i32,
    h: i32,
) -> Result<Surface<'static>> {
    let Some(data) = window_data(window) else {
        return Err(Error::new("Invalid window"));
    };

    // Free the old framebuffer surface
    data.state.with(|s| {
        // SAFETY: the DC and bitmap were created below for this window.
        unsafe {
            if !s.mdc.is_null() {
                DeleteDC(s.mdc);
            }
            if !s.hbm.is_null() {
                DeleteObject(s.hbm);
            }
        }
        s.mdc = std::ptr::null_mut();
        s.hbm = std::ptr::null_mut();
        s.bits = std::ptr::null_mut();
    });

    // Find out the format of the screen
    // SAFETY: BITMAPINFOHEADER and RGBQUAD are plain data; all zeroes is valid.
    let mut info: BitmapInfo256 = unsafe { std::mem::zeroed() };
    info.header.biSize = size_of::<BITMAPINFOHEADER>() as u32;

    // The second call to GetDIBits() fills in the bitfields
    // SAFETY: data.hdc is the window's DC; info has room for the color table.
    unsafe {
        let hbm = CreateCompatibleBitmap(data.hdc, 1, 1);
        let pinfo = (&mut info as *mut BitmapInfo256).cast::<BITMAPINFO>();
        GetDIBits(
            data.hdc,
            hbm,
            0,
            0,
            std::ptr::null_mut(),
            pinfo,
            DIB_RGB_COLORS,
        );
        GetDIBits(
            data.hdc,
            hbm,
            0,
            0,
            std::ptr::null_mut(),
            pinfo,
            DIB_RGB_COLORS,
        );
        DeleteObject(hbm);
    }

    // Check if a transparent channel is required
    let need_alpha = window_flags(window).contains(WindowFlags::TRANSPARENT);

    let masks = |info: &BitmapInfo256| -> [u32; 4] {
        // (the masks follow the header, in the color table)
        let c = |i: usize| {
            let q = info.colors[i];
            u32::from_le_bytes([q.rgbBlue, q.rgbGreen, q.rgbRed, q.rgbReserved])
        };
        [c(0), c(1), c(2), c(3)]
    };

    let mut format = PixelFormat::UNKNOWN;
    if info.header.biCompression == BI_BITFIELDS {
        let bpp = info.header.biPlanes as u32 * info.header.biBitCount as u32;
        let m = masks(&info);
        format = format_for_masks(bpp, [m[0], m[1], m[2]]);
    }
    if format == PixelFormat::UNKNOWN || need_alpha {
        // We'll use RGB or BGRA32 format for now
        format = if need_alpha {
            PixelFormat::BGRA32
        } else {
            PixelFormat::XRGB8888
        };

        // Create a new one
        // SAFETY: plain data; all zeroes is valid.
        info = unsafe { std::mem::zeroed() };
        info.header.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.header.biPlanes = 1;
        info.header.biBitCount = 32;
        info.header.biCompression = if need_alpha { BI_BITFIELDS } else { BI_RGB };

        if need_alpha {
            let m = PixelFormat::BGRA32.masks()?;
            for (i, mask) in [m.r, m.g, m.b, m.a].into_iter().enumerate() {
                let b = mask.to_le_bytes();
                info.colors[i] = RGBQUAD {
                    rgbBlue: b[0],
                    rgbGreen: b[1],
                    rgbRed: b[2],
                    rgbReserved: b[3],
                };
            }
        }
    }

    // Fill in the size information
    let pitch = ((w * format.bytes_per_pixel() as i32) + 3) & !3;
    info.header.biWidth = w;
    info.header.biHeight = -h; // negative for topdown bitmap
    info.header.biSizeImage = h as u32 * pitch as u32;

    let mut pixels: *mut c_void = std::ptr::null_mut();
    // SAFETY: data.hdc is the window's DC; info describes the bitmap.
    let (mdc, hbm) = unsafe {
        let mdc = CreateCompatibleDC(data.hdc);
        let hbm = CreateDIBSection(
            data.hdc,
            (&info as *const BitmapInfo256).cast::<BITMAPINFO>(),
            DIB_RGB_COLORS,
            &mut pixels,
            std::ptr::null_mut(),
            0,
        );
        (mdc, hbm)
    };
    data.state.with(|s| {
        s.mdc = mdc;
        s.hbm = hbm;
    });

    if hbm.is_null() {
        return Err(set_error("Unable to create DIB"));
    }
    // SAFETY: both were just created.
    unsafe { SelectObject(mdc, hbm) };
    data.state.with(|s| {
        s.bits = pixels;
        s.bits_pitch = pitch as usize;
        s.bits_h = h.max(0) as usize;
    });

    Surface::from_vec(
        w,
        h,
        format,
        vec![0; pitch as usize * h.max(0) as usize],
        pitch,
    )
}

/// Translation of `WIN_UpdateWindowFramebuffer()`.
pub(crate) fn update_window_framebuffer(
    window: WindowID,
    surface: &Surface<'static>,
    rects: &[Rect],
) -> Result<()> {
    let Some(data) = window_data(window) else {
        return Err(Error::new("Invalid window"));
    };

    data.state.with(|s| {
        // (copy the updated pixels into the DIB section, see the module docs)
        if let Some(src) = surface.pixels() {
            if !s.bits.is_null() {
                let src_pitch = surface.pitch() as usize;
                let row_bytes = s.bits_pitch.min(src_pitch);
                let bpp = surface.format().bytes_per_pixel() as usize;
                let dst_len = s.bits_pitch * s.bits_h;
                // SAFETY: the DIB section holds bits_pitch * bits_h bytes.
                let dst = unsafe { std::slice::from_raw_parts_mut(s.bits.cast::<u8>(), dst_len) };
                let bounds = Rect {
                    x: 0,
                    y: 0,
                    w: surface.width(),
                    h: surface.height().min(s.bits_h as i32),
                };
                for rect in rects {
                    let Some(r) = rect.intersection(&bounds) else {
                        continue;
                    };
                    let x0 = (r.x as usize * bpp).min(row_bytes);
                    let x1 = ((r.x + r.w) as usize * bpp).min(row_bytes);
                    for y in r.y as usize..(r.y + r.h) as usize {
                        dst[y * s.bits_pitch + x0..y * s.bits_pitch + x1]
                            .copy_from_slice(&src[y * src_pitch + x0..y * src_pitch + x1]);
                    }
                }
            }
        }

        for rect in rects {
            // SAFETY: the window's DC and the memory DC with the DIB selected.
            unsafe {
                BitBlt(
                    data.hdc, rect.x, rect.y, rect.w, rect.h, s.mdc, rect.x, rect.y, SRCCOPY,
                );
            }
        }
    });
    Ok(())
}

/// Translation of `WIN_DestroyWindowFramebuffer()`.
pub(crate) fn destroy_window_framebuffer(window: WindowID) {
    let Some(data) = window_data(window) else {
        // The window wasn't fully initialized
        return;
    };

    data.state.with(|s| {
        // SAFETY: the DC and bitmap were created for this window.
        unsafe {
            if !s.mdc.is_null() {
                DeleteDC(s.mdc);
                s.mdc = std::ptr::null_mut();
            }
            if !s.hbm.is_null() {
                DeleteObject(s.hbm);
                s.hbm = std::ptr::null_mut();
            }
        }
        s.bits = std::ptr::null_mut();
    });
}
