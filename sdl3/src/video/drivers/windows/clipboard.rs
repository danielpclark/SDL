// Rust translation of src/video/windows/SDL_windowsclipboard.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Windows clipboard: text (`CF_UNICODETEXT`), BMP (`CF_DIB`/`CF_DIBV5`)
//! and PNG images.

use std::sync::atomic::{AtomicU32, Ordering};

use windows_sys::Win32::Foundation::{GlobalFree, HANDLE, HWND};
use windows_sys::Win32::Graphics::Gdi::{
    BITMAPFILEHEADER, BITMAPINFOHEADER, BITMAPV5HEADER, BI_BITFIELDS, BI_RGB, RGBQUAD,
};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows_sys::Win32::System::Ole::{CF_DIB, CF_DIBV5, CF_TEXT, CF_UNICODETEXT};

use super::window::window_data;
use super::VideoData;
use crate::core::windows::{set_error, utf8_to_wide, wide_to_utf8};
use crate::error::{Error, Result};
use crate::events::window::send_clipboard_update;
use crate::video::clipboard::{
    clipboard_mime_types, has_internal_clipboard_data, internal_clipboard_data, is_text_mime_type,
};
use crate::video::core::with_device;

const BFT_BITMAP: u16 = 0x4d42; // 'BM'

// (the BMP fields are read and written as little-endian bytes)

/// Translation of `GetClipboardFormatPNG()`.
fn get_clipboard_format_png() -> u32 {
    static FORMAT: AtomicU32 = AtomicU32::new(0);

    let mut format = FORMAT.load(Ordering::Relaxed);
    if format == 0 {
        let name = utf8_to_wide("PNG");
        // SAFETY: the name is NUL-terminated.
        format = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
        FORMAT.store(format, Ordering::Relaxed);
    }
    format
}

/// The `HWND` of `_this->windows` (the most recently created window).
fn first_window_hwnd() -> HWND {
    let first = with_device(|v| v.windows.first().map(|w| w.core.id))
        .ok()
        .flatten();
    first
        .and_then(window_data)
        .map_or(std::ptr::null_mut(), |d| d.hwnd)
}

/// Translation of `WIN_OpenClipboard()`.
fn open_clipboard() -> bool {
    // Retry to open the clipboard in case another application has it open
    const MAX_ATTEMPTS: i32 = 3;

    let hwnd = first_window_hwnd();
    for _ in 0..MAX_ATTEMPTS {
        // SAFETY: hwnd is a window handle or null.
        if unsafe { OpenClipboard(hwnd) } != 0 {
            return true;
        }
        crate::timer::delay(std::time::Duration::from_millis(10));
    }
    false
}

/// Translation of `WIN_CloseClipboard()`.
fn close_clipboard() {
    // SAFETY: CloseClipboard has no preconditions.
    unsafe { CloseClipboard() };
}

fn u16_at(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}
fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// Copy `bytes` into a new movable global memory block.
fn global_from_bytes(bytes: &[&[u8]]) -> Result<HANDLE> {
    let size: usize = bytes.iter().map(|b| b.len()).sum();
    // SAFETY: GlobalAlloc has no preconditions; the block is locked, filled
    // with `size` bytes and unlocked.
    unsafe {
        let h_mem = GlobalAlloc(GMEM_MOVEABLE, size);
        if h_mem.is_null() {
            return Err(Error::out_of_memory());
        }
        let dst = GlobalLock(h_mem).cast::<u8>();
        if dst.is_null() {
            let e = set_error("GlobalLock()");
            GlobalFree(h_mem);
            return Err(e);
        }
        let mut off = 0;
        for b in bytes {
            std::ptr::copy_nonoverlapping(b.as_ptr(), dst.add(off), b.len());
            off += b.len();
        }
        GlobalUnlock(h_mem);
        Ok(h_mem)
    }
}

/// Translation of `WIN_ConvertBMPtoDIB()`: the DIB in a global memory
/// block, and its clipboard format.
fn convert_bmp_to_dib(bmp: Option<&[u8]>) -> Result<(HANDLE, u32)> {
    const FILE_HEADER: usize = size_of::<BITMAPFILEHEADER>();
    let Some(bmp) = bmp.filter(|b| b.len() > FILE_HEADER && u16_at(b, 0) == BFT_BITMAP) else {
        return Err(Error::new("Invalid BMP data"));
    };
    if bmp.len() < FILE_HEADER + size_of::<BITMAPINFOHEADER>() {
        // (upstream reads the header regardless)
        return Err(Error::new("Invalid BMP data"));
    }
    let bf_off_bits = u32_at(bmp, 10) as usize;
    let pbih = FILE_HEADER;
    let bi_size = u32_at(bmp, pbih) as usize;
    let bi_clr_used = u32_at(bmp, pbih + 32) as usize;
    let bi_size_image = u32_at(bmp, pbih + 20) as usize;
    let bih_size = bi_size + bi_clr_used * size_of::<RGBQUAD>();
    let pixels_size = bi_size_image;

    let format = if bi_size >= size_of::<BITMAPV5HEADER>() {
        CF_DIBV5 as u32
    } else {
        CF_DIB as u32
    };

    if bf_off_bits >= (FILE_HEADER + bih_size)
        && (bf_off_bits + pixels_size) <= bmp.len()
        && pbih + bih_size <= bmp.len()
    {
        let pixels = &bmp[bf_off_bits..bf_off_bits + pixels_size];
        let h_mem = global_from_bytes(&[&bmp[pbih..pbih + bih_size], pixels])?;
        Ok((h_mem, format))
    } else {
        Err(Error::new("Invalid BMP data"))
    }
}

/// Translation of `WIN_ConvertDIBtoBMP()`.
fn convert_dib_to_bmp(h_mem: HANDLE) -> Result<Vec<u8>> {
    // SAFETY: h_mem is a global memory handle from the clipboard.
    let mem_size = unsafe { GlobalSize(h_mem) };

    if mem_size <= size_of::<BITMAPINFOHEADER>() {
        return Err(Error::new("Invalid BMP data"));
    }
    // SAFETY: as above; the locked block holds mem_size bytes.
    let dib = unsafe { GlobalLock(h_mem) };
    if dib.is_null() {
        return Err(set_error("GlobalLock()"));
    }
    // SAFETY: as above.
    let dib = unsafe { std::slice::from_raw_parts(dib.cast::<u8>(), mem_size) };

    let bi_size = u32_at(dib, 0) as usize;
    let bi_bit_count = u16_at(dib, 14) as u32;
    let bi_compression = u32_at(dib, 16);
    let bi_size_image = u32_at(dib, 20) as usize;
    let bi_clr_used = u32_at(dib, 32) as usize;

    // https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-bitmapinfoheader#color-tables
    let color_table_size = match bi_compression {
        BI_RGB => {
            if bi_bit_count <= 8 {
                size_of::<RGBQUAD>()
                    * if bi_clr_used == 0 {
                        1 << bi_bit_count
                    } else {
                        bi_clr_used
                    }
            } else {
                0
            }
        }
        BI_BITFIELDS => 3 * 4,
        6 /* BI_ALPHABITFIELDS */ => {
            // https://learn.microsoft.com/en-us/previous-versions/windows/embedded/aa452885(v=msdn.10)
            4 * 4
        }
        // FOURCC
        _ => size_of::<RGBQUAD>() * bi_clr_used,
    };

    let bih_size = bi_size + color_table_size;
    let dib_size = bih_size + bi_size_image;
    let result = if dib_size <= mem_size {
        const FILE_HEADER: usize = size_of::<BITMAPFILEHEADER>();
        let bmp_size = FILE_HEADER + mem_size;
        let mut bmp = Vec::with_capacity(bmp_size);
        bmp.extend_from_slice(&BFT_BITMAP.to_le_bytes()); // bfType
        bmp.extend_from_slice(&(bmp_size as u32).to_le_bytes()); // bfSize
        bmp.extend_from_slice(&0u16.to_le_bytes()); // bfReserved1
        bmp.extend_from_slice(&0u16.to_le_bytes()); // bfReserved2
        bmp.extend_from_slice(&((FILE_HEADER + bi_size + color_table_size) as u32).to_le_bytes()); // bfOffBits
        bmp.extend_from_slice(dib);
        Ok(bmp)
    } else {
        Err(Error::new("Invalid BMP data"))
    };
    // SAFETY: locked above.
    unsafe { GlobalUnlock(h_mem) };
    result
}

/// Translation of `WIN_SetClipboardImage()`.
fn set_clipboard_image(mime_type: &str) -> Result<()> {
    let clipboard_data = internal_clipboard_data(mime_type);
    let (h_mem, format) = if mime_type == "image/bmp" {
        convert_bmp_to_dib(clipboard_data.as_deref())?
    } else if mime_type == "image/png" {
        let format = get_clipboard_format_png();
        let data = clipboard_data.unwrap_or_default();
        (global_from_bytes(&[&data])?, format)
    } else {
        return Err(Error::new("Unknown image format"));
    };
    // Save the image to the clipboard
    // SAFETY: the clipboard is open; it takes ownership of h_mem.
    if unsafe { SetClipboardData(format, h_mem) }.is_null() {
        return Err(set_error("Couldn't set clipboard data"));
    }
    Ok(())
}

/// Translation of `WIN_SetClipboardText()`.
fn set_clipboard_text(mime_type: &str) -> Result<()> {
    let clipboard_data = internal_clipboard_data(mime_type);
    let Some(clipboard_data) = clipboard_data.filter(|d| !d.is_empty()) else {
        return Ok(());
    };
    let Ok(utf16) = crate::stdlib::iconv::iconv_string("UTF-16LE", "UTF-8", &clipboard_data) else {
        return Err(Error::new("Couldn't convert text from UTF-8"));
    };
    let tstr: Vec<u16> = utf16
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect();

    // Copy the text over, adding carriage returns as necessary
    let mut dst: Vec<u16> = Vec::with_capacity(tstr.len() + 1);
    for (i, &c) in tstr.iter().enumerate() {
        if c == u16::from(b'\n') && (i == 0 || tstr[i - 1] != u16::from(b'\r')) {
            // We're going to insert a carriage return
            dst.push(u16::from(b'\r'));
        }
        dst.push(c);
    }
    dst.push(0);
    let bytes: Vec<u8> = dst.iter().flat_map(|c| c.to_le_bytes()).collect();

    // Save the data to the clipboard
    let h_mem = global_from_bytes(&[&bytes])?;
    // SAFETY: the clipboard is open; it takes ownership of h_mem.
    if unsafe { SetClipboardData(CF_UNICODETEXT as u32, h_mem) }.is_null() {
        return Err(set_error("Couldn't set clipboard data"));
    }
    Ok(())
}

/// Translation of `WIN_SetClipboardData()`.
pub(crate) fn set_clipboard_data(data: &VideoData) -> Result<()> {
    let mut result = Ok(());

    /* I investigated delayed clipboard rendering, and at least with text and image
     * formats you have to use an output window, not SDL_HelperWindow, and the system
     * requests them being rendered immediately, so there isn't any benefit.
     */

    if open_clipboard() {
        // SAFETY: the clipboard is open.
        unsafe { EmptyClipboard() };

        let mime_types = clipboard_mime_types().unwrap_or_default();

        // Set the clipboard text
        if let Some(mime_type) = mime_types.iter().find(|m| is_text_mime_type(m)) {
            if let Err(e) = set_clipboard_text(mime_type) {
                result = Err(e);
            }
            // Only set the first clipboard text
        }

        // Set the clipboard image
        if let Some(mime_type) = mime_types
            .iter()
            .find(|m| *m == "image/bmp" || *m == "image/png")
        {
            if let Err(e) = set_clipboard_image(mime_type) {
                result = Err(e);
            }
        }

        // SAFETY: GetClipboardSequenceNumber has no preconditions.
        let count = unsafe { GetClipboardSequenceNumber() };
        data.state.with(|s| s.clipboard_count = count);
        close_clipboard();
    } else {
        result = Err(set_error("Couldn't open clipboard"));
    }
    result
}

/// `IsClipboardFormatAvailable()`
fn format_available(format: u32) -> bool {
    // SAFETY: IsClipboardFormatAvailable has no preconditions.
    unsafe { IsClipboardFormatAvailable(format) != 0 }
}

/// Run `f` on the clipboard data of `format`, with the clipboard open.
fn with_clipboard_handle<R>(format: u32, f: impl FnOnce(HANDLE) -> Option<R>) -> Option<R> {
    if !open_clipboard() {
        return None;
    }
    // SAFETY: the clipboard is open.
    let h_mem = unsafe { GetClipboardData(format) };
    let result = if !h_mem.is_null() {
        f(h_mem)
    } else {
        let _ = set_error("Couldn't get clipboard data");
        None
    };
    close_clipboard();
    result
}

/// The bytes of a locked global memory block, `f` applied.
fn with_locked<R>(h_mem: HANDLE, f: impl FnOnce(*const u8, usize) -> R) -> Option<R> {
    // SAFETY: h_mem is a clipboard global memory handle; it's unlocked after use.
    unsafe {
        let size = GlobalSize(h_mem);
        let mem = GlobalLock(h_mem);
        if mem.is_null() {
            let _ = set_error("Couldn't lock clipboard data");
            return None;
        }
        let r = f(mem.cast(), size);
        GlobalUnlock(h_mem);
        Some(r)
    }
}

/// Translation of `WIN_GetClipboardData()`.
pub(crate) fn get_clipboard_data(mime_type: &str) -> Option<Vec<u8>> {
    if is_text_mime_type(mime_type) {
        let mut text: Option<String> = None;

        if format_available(CF_UNICODETEXT as u32) {
            text = with_clipboard_handle(CF_UNICODETEXT as u32, |h_mem| {
                with_locked(h_mem, |p, size| {
                    // SAFETY: the locked block holds `size` bytes of UTF-16.
                    let w = unsafe { std::slice::from_raw_parts(p.cast::<u16>(), size / 2) };
                    wide_to_utf8(w)
                })
            });
        } else if format_available(CF_TEXT as u32) {
            text = with_clipboard_handle(CF_TEXT as u32, |h_mem| {
                with_locked(h_mem, |p, size| {
                    // SAFETY: the locked block holds `size` bytes.
                    let b = unsafe { std::slice::from_raw_parts(p, size) };
                    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
                    String::from_utf8_lossy(&b[..end]).into_owned()
                })
            });
        }
        Some(text.unwrap_or_default().into_bytes())
    } else if mime_type == "image/bmp" {
        let format = if format_available(CF_DIBV5 as u32) {
            CF_DIBV5 as u32
        } else if format_available(CF_DIB as u32) {
            CF_DIB as u32
        } else {
            return None;
        };
        with_clipboard_handle(format, |h_mem| convert_dib_to_bmp(h_mem).ok())
    } else if mime_type == "image/png" {
        if format_available(get_clipboard_format_png()) {
            with_clipboard_handle(get_clipboard_format_png(), |h_mem| {
                with_locked(h_mem, |p, size| {
                    // SAFETY: the locked block holds `size` bytes.
                    unsafe { std::slice::from_raw_parts(p, size) }.to_vec()
                })
            })
        } else {
            None
        }
    } else {
        internal_clipboard_data(mime_type)
    }
}

/// Translation of `WIN_HasClipboardData()`.
pub(crate) fn has_clipboard_data(mime_type: &str) -> bool {
    if is_text_mime_type(mime_type) {
        if format_available(CF_UNICODETEXT as u32) || format_available(CF_TEXT as u32) {
            return true;
        }
    } else if mime_type == "image/bmp" {
        if format_available(CF_DIBV5 as u32) || format_available(CF_DIB as u32) {
            return true;
        }
    } else if mime_type == "image/png" && format_available(get_clipboard_format_png()) {
        return true;
    }
    has_internal_clipboard_data(mime_type)
}

/// Translation of `GetClipboardFormatMimeType()`.
fn get_clipboard_format_mime_type(format: u32) -> Option<&'static str> {
    match format {
        f if f == CF_TEXT as u32 => Some("text/plain"),
        f if f == CF_UNICODETEXT as u32 => Some("text/plain;charset=utf-8"),
        f if f == CF_DIB as u32 || f == CF_DIBV5 as u32 => Some("image/bmp"),
        f if f == get_clipboard_format_png() => Some("image/png"),
        _ => None,
    }
}

/// Translation of `GetMimeTypes()`.
fn get_mime_types() -> Option<Vec<String>> {
    if !open_clipboard() {
        return None;
    }
    let mut new_mime_types = Vec::new();
    let mut format = 0u32;
    let mut have_image_bmp = false;
    loop {
        // SAFETY: the clipboard is open.
        format = unsafe { EnumClipboardFormats(format) };
        if format == 0 {
            break;
        }

        if format == CF_DIB as u32 || format == CF_DIBV5 as u32 {
            if have_image_bmp {
                // We have already registered this format
                continue;
            }
            have_image_bmp = true;
        }

        if let Some(mime_type) = get_clipboard_format_mime_type(format) {
            new_mime_types.push(mime_type.to_owned());
        }
    }
    close_clipboard();
    Some(new_mime_types)
}

/// Translation of `WIN_CheckClipboardUpdate()`.
pub(crate) fn check_clipboard_update(data: &VideoData) {
    // SAFETY: GetClipboardSequenceNumber has no preconditions.
    let count = unsafe { GetClipboardSequenceNumber() };
    if count != data.state.with(|s| s.clipboard_count) {
        if count != 0 {
            if let Some(new_mime_types) = get_mime_types() {
                send_clipboard_update(false, new_mime_types);
            }
        }
        data.state.with(|s| s.clipboard_count = count);
    }
}
