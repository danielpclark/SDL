// Rust translation of src/video/windows/SDL_windowsmouse.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Cursors (including animated cursors built as in-memory `.ani` files),
//! warping, capture and relative motion scaling for the Windows video driver.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use windows_sys::Win32::Foundation::{ERROR_SUCCESS, POINT};
use windows_sys::Win32::Graphics::Gdi::{ClientToScreen, BITMAPINFOHEADER, BI_RGB};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, ReleaseCapture, SetCapture, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON, VK_XBUTTON1,
    VK_XBUTTON2,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconFromResourceEx, DestroyCursor, GetCursorPos, GetSystemMetrics, LoadCursorW,
    SetCursor, SetCursorPos, SystemParametersInfoW, HCURSOR, IDC_APPSTARTING, IDC_ARROW, IDC_CROSS,
    IDC_HAND, IDC_HELP, IDC_IBEAM, IDC_NO, IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE,
    IDC_SIZEWE, IDC_WAIT, SM_REMOTESESSION, SM_SWAPBUTTON, SPI_GETMOUSE, SPI_GETMOUSESPEED,
};

use super::window::window_data;
use super::{current_is_per_monitor_v2_dpi_aware, is_per_monitor_v2_dpi_aware, rawinput};
use super::{VideoData, USER_DEFAULT_SCREEN_DPI};
use crate::core::windows::{is_windows_8_or_greater, utf8_to_wide};
use crate::error::{Error, Result};
use crate::events::mouse::{
    self, Cursor, CursorFrame, MouseButtonFlags, MouseID, SystemCursor, GLOBAL_MOUSE_ID,
};
use crate::events::WindowID;
use crate::hints;
use crate::video::surface::{PROP_SURFACE_HOTSPOT_X_NUMBER, PROP_SURFACE_HOTSPOT_Y_NUMBER};
use crate::video::{display, window as video_window, PixelFormat, Surface};

/// Translation of `RIFF_FOURCC()`.
const fn riff_fourcc(c0: u8, c1: u8, c2: u8, c3: u8) -> u32 {
    (c0 as u32) | ((c1 as u32) << 8) | ((c2 as u32) << 16) | ((c3 as u32) << 24)
}

const ANI_FLAG_ICON: u32 = 0x1;

/// `sizeof(CURSORICONFILEDIRENTRY)` (packed)
const CURSORICONFILEDIRENTRY_SIZE: usize = 16;
/// `sizeof(CURSORICONFILEDIR)` (packed)
const CURSORICONFILEDIR_SIZE: usize = 6;
/// `sizeof(ANIHEADER)` = 36 bytes.
const ANIHEADER_SIZE: u32 = 36;

/// Translation of `CURSORICONFILEDIRENTRY`.
#[derive(Clone, Copy, Default)]
struct CursorIconFileDirEntry {
    b_width: u8,
    b_height: u8,
    b_color_count: u8,
    b_reserved: u8,
    x_hotspot: u16,
    y_hotspot: u16,
    dw_image_size: u32,
    dw_image_offset: u32,
}

impl CursorIconFileDirEntry {
    /// The packed little-endian bytes.
    fn bytes(&self) -> [u8; CURSORICONFILEDIRENTRY_SIZE] {
        let mut b = [0u8; CURSORICONFILEDIRENTRY_SIZE];
        b[0] = self.b_width;
        b[1] = self.b_height;
        b[2] = self.b_color_count;
        b[3] = self.b_reserved;
        b[4..6].copy_from_slice(&self.x_hotspot.to_le_bytes());
        b[6..8].copy_from_slice(&self.y_hotspot.to_le_bytes());
        b[8..12].copy_from_slice(&self.dw_image_size.to_le_bytes());
        b[12..16].copy_from_slice(&self.dw_image_offset.to_le_bytes());
        b
    }
}

/// A Win32 handle that may move between threads (it's a process-wide token).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Handle(HCURSOR);
// SAFETY: cursor handles are process-wide tokens.
unsafe impl Send for Handle {}
// SAFETY: as above.
unsafe impl Sync for Handle {}

/// The mutable part of `SDL_CursorData`.
struct CursorCache {
    /// `CachedCursor` entries, newest first (`scale`, `cursor`)
    cache: Vec<(f32, Handle)>,
}

/// Translation of `struct SDL_CursorData`.
struct CursorData {
    cursor: Handle,
    cache: Mutex<CursorCache>,
    hot_x: i32,
    hot_y: i32,
    /// `frames` (`num_frames` is its length)
    frames: Mutex<Vec<(Surface<'static>, u32)>>,
}

impl Drop for CursorData {
    /// Translation of `WIN_FreeCursor()` (the surfaces drop with the frames).
    fn drop(&mut self) {
        let cache = self.cache.get_mut().unwrap_or_else(|e| e.into_inner());
        for (_, entry) in cache.cache.drain(..) {
            if !entry.0.is_null() {
                // SAFETY: the cursor was created by CreateIconFromResourceEx.
                unsafe { DestroyCursor(entry.0) };
            }
        }
        if !self.cursor.0.is_null() {
            // SAFETY: a cursor handle from LoadCursor.
            unsafe { DestroyCursor(self.cursor.0) };
        }
    }
}

/// Translation of `WIN_MouseData`.
struct MouseData {
    xs: [u64; 5],
    ys: [u64; 5],
    #[allow(dead_code)] // (only used by commented-out code upstream)
    residual: [i64; 2],
    dpiscale: u32,
    dpidenom: u32,
    last_node: i32,
    enhanced: bool,
    dpiaware: bool,
}

/// Translation of `SDL_last_warp_time`.
pub(crate) static LAST_WARP_TIME: AtomicU32 = AtomicU32::new(0);
/// Translation of `SDL_cursor` (as a number).
static SDL_CURSOR: AtomicUsize = AtomicUsize::new(0);
/// Translation of `SDL_blank_cursor`.
static BLANK_CURSOR: Mutex<Option<Cursor>> = Mutex::new(None);
/// Translation of `WIN_system_scale_data`.
static SYSTEM_SCALE_DATA: Mutex<MouseData> = Mutex::new(MouseData {
    xs: [0; 5],
    ys: [0; 5],
    residual: [0; 2],
    dpiscale: 0,
    dpidenom: 0,
    last_node: 0,
    enhanced: false,
    dpiaware: false,
});

/// The cursor shown in client areas (`SDL_cursor`).
pub(crate) fn current_cursor() -> HCURSOR {
    SDL_CURSOR.load(Ordering::Relaxed) as HCURSOR
}

/// Translation of `WIN_CreateCursorAndData()`.
fn create_cursor_and_data(hcursor: HCURSOR) -> Option<Cursor> {
    if hcursor.is_null() {
        return None;
    }

    Some(Cursor::with_internal(CursorData {
        cursor: Handle(hcursor),
        cache: Mutex::new(CursorCache { cache: Vec::new() }),
        hot_x: 0,
        hot_y: 0,
        frames: Mutex::new(Vec::new()),
    }))
}

/// A copy of `surface` with its alternate images (upstream takes a
/// reference, or duplicates `SDL_SURFACE_PREALLOCATED` surfaces).
fn retain_surface(surface: &Surface<'_>) -> Result<Surface<'static>> {
    let mut copy = surface.duplicate()?;
    for image in surface.alternate_images() {
        copy.add_alternate_image(image.duplicate()?);
    }
    Ok(copy)
}

/// Translation of `WIN_CreateAnimatedCursorAndData()`.
fn create_animated_cursor_and_data(
    frames: &[CursorFrame<'_>],
    hot_x: i32,
    hot_y: i32,
) -> Result<Cursor> {
    // Dynamically generate cursors at the appropriate DPI
    let mut kept = Vec::with_capacity(frames.len());
    for frame in frames {
        // (SDL_SURFACE_PREALLOCATED or not, the surface is kept as a copy)
        kept.push((retain_surface(frame.surface)?, frame.duration));
    }
    Ok(Cursor::with_internal(CursorData {
        cursor: Handle(std::ptr::null_mut()),
        cache: Mutex::new(CursorCache { cache: Vec::new() }),
        hot_x,
        hot_y,
        frames: Mutex::new(kept),
    }))
}

/// An in-memory `SDL_IOStream` (`SDL_IOFromDynamicMem()`) for the writers below.
struct DynamicMem {
    data: Vec<u8>,
    pos: usize,
}

impl DynamicMem {
    fn tell(&self) -> i64 {
        self.pos as i64
    }
    fn seek(&mut self, offset: i64) -> bool {
        if offset < 0 {
            return false;
        }
        self.pos = offset as usize;
        true
    }
    fn write(&mut self, bytes: &[u8]) -> bool {
        let end = self.pos + bytes.len();
        if self.data.len() < end {
            self.data.resize(end, 0);
        }
        self.data[self.pos..end].copy_from_slice(bytes);
        self.pos = end;
        true
    }
    fn write_u32_le(&mut self, v: u32) -> bool {
        self.write(&v.to_le_bytes())
    }
}

/// Translation of `SaveChunkSize()`.
fn save_chunk_size(dst: &mut DynamicMem, offset: i64) -> bool {
    let here = dst.tell();
    if here < 0 {
        return false;
    }
    if !dst.seek(offset) {
        return false;
    }

    let size = (here - (offset + 4)) as u32;
    if !dst.write_u32_le(size) {
        return false;
    }
    dst.seek(here)
}

/// Translation of `FillIconEntry()`.
fn fill_icon_entry(
    entry: &mut CursorIconFileDirEntry,
    surface: &Surface<'_>,
    mut hot_x: i32,
    mut hot_y: i32,
    dw_image_size: u32,
    dw_image_offset: u32,
) -> bool {
    if let Some(props) = surface.props.as_ref() {
        hot_x = props
            .get_number(PROP_SURFACE_HOTSPOT_X_NUMBER)
            .unwrap_or(hot_x as i64) as i32;
        hot_y = props
            .get_number(PROP_SURFACE_HOTSPOT_Y_NUMBER)
            .unwrap_or(hot_y as i64) as i32;
    }
    hot_x = hot_x.clamp(0, surface.width() - 1);
    hot_y = hot_y.clamp(0, surface.height() - 1);

    *entry = CursorIconFileDirEntry::default();
    entry.b_width = if surface.width() < 256 {
        surface.width() as u8
    } else {
        0
    }; // 0 means a width of 256
    entry.b_height = if surface.height() < 256 {
        surface.height() as u8
    } else {
        0
    }; // 0 means a height of 256
    entry.x_hotspot = hot_x as u16;
    entry.y_hotspot = hot_y as u16;
    entry.dw_image_size = dw_image_size;
    entry.dw_image_offset = dw_image_offset;
    true
}

/* For info on the expected mask format see:
 * https://devblogs.microsoft.com/oldnewthing/20101018-00/?p=12513
 */
/// Translation of `CreateIconMask()`.
fn create_icon_mask(surface: &Surface<'_>) -> Vec<u8> {
    let w = ((surface.width() + 7) / 8) as usize;
    let pad = if !w.is_multiple_of(4) { 4 - (w % 4) } else { 0 };
    let pitch = w + pad;
    let size = pitch * surface.height() as usize;
    const MASKS: [u8; 8] = [0x80, 0x40, 0x20, 0x10, 0x8, 0x4, 0x2, 0x1];

    // Make the mask completely transparent.
    let mut mask = vec![0xffu8; size];
    let mut dst = 0usize;
    for y in (0..surface.height()).rev() {
        for x in 0..surface.width() {
            let a = surface.read_pixel(x, y).map_or(0, |c| c.a);

            if a != 0 {
                // Reset bit of an opaque pixel.
                mask[dst + (x as usize >> 3)] &= !MASKS[x as usize & 7];
            }
        }
        dst += pitch;
    }
    mask
}

/// Translation of `WriteIconSurface()`.
fn write_icon_surface(dst: &mut DynamicMem, surface: &Surface<'_>) -> bool {
    let temp;
    let surface = if surface.format() != PixelFormat::ARGB8888 {
        match surface.convert(PixelFormat::ARGB8888) {
            Ok(s) => {
                temp = s;
                &temp
            }
            Err(_) => return false,
        }
    } else {
        surface
    };

    // Cursor data is double height (DIB and mask), stored bottom-up
    let mut ok = true;
    let mask = create_icon_mask(surface);
    let mask_size = mask.len();

    let row_size = surface.width() as u32 * 4;
    let bmih = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: surface.width(),
        biHeight: surface.height() * 2,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        biSizeImage: surface.height() as u32 * row_size + mask_size as u32,
        biXPelsPerMeter: 0,
        biYPelsPerMeter: 0,
        biClrUsed: 0,
        biClrImportant: 0,
    };
    let mut header = Vec::with_capacity(40);
    header.extend_from_slice(&bmih.biSize.to_le_bytes());
    header.extend_from_slice(&bmih.biWidth.to_le_bytes());
    header.extend_from_slice(&bmih.biHeight.to_le_bytes());
    header.extend_from_slice(&bmih.biPlanes.to_le_bytes());
    header.extend_from_slice(&bmih.biBitCount.to_le_bytes());
    header.extend_from_slice(&bmih.biCompression.to_le_bytes());
    header.extend_from_slice(&bmih.biSizeImage.to_le_bytes());
    header.extend_from_slice(&bmih.biXPelsPerMeter.to_le_bytes());
    header.extend_from_slice(&bmih.biYPelsPerMeter.to_le_bytes());
    header.extend_from_slice(&bmih.biClrUsed.to_le_bytes());
    header.extend_from_slice(&bmih.biClrImportant.to_le_bytes());
    ok &= dst.write(&header);

    let Some(pixels) = surface.pixels() else {
        return false;
    };
    let pitch = surface.pitch() as usize;
    for i in (0..surface.height() as usize).rev() {
        ok &= dst.write(&pixels[i * pitch..i * pitch + row_size as usize]);
    }
    ok &= dst.write(&mask);

    ok
}

/// Translation of `WriteIconFrame()`.
fn write_icon_frame(
    dst: &mut DynamicMem,
    surface: &mut Surface<'static>,
    hot_x: i32,
    hot_y: i32,
    scale: f32,
) -> bool {
    let image = surface.image_for_scale(scale);
    let surface: &Surface<'_> = match &image {
        crate::video::surface::SurfaceImage::Original(s) => s,
        crate::video::surface::SurfaceImage::Alternate(s) => s,
        crate::video::surface::SurfaceImage::Scaled(s) => s,
    };

    let count = 1usize;

    // Raymond Chen has more insight into this format at:
    // https://devblogs.microsoft.com/oldnewthing/20101018-00/?p=12513
    let mut ok = true;
    ok &= dst.write_u32_le(riff_fourcc(b'i', b'c', b'o', b'n'));
    let icon_size_offset = dst.tell();
    ok &= dst.write_u32_le(0);
    let base_offset = icon_size_offset + 4;

    let mut dir = [0u8; CURSORICONFILEDIR_SIZE];
    dir[0..2].copy_from_slice(&0u16.to_le_bytes()); // idReserved
    dir[2..4].copy_from_slice(&2u16.to_le_bytes()); // idType: Cursor
    dir[4..6].copy_from_slice(&(count as u16).to_le_bytes()); // idCount
    ok &= dst.write(&dir);

    let mut entries = vec![CursorIconFileDirEntry::default(); count];
    let entry_bytes = |entries: &[CursorIconFileDirEntry]| -> Vec<u8> {
        entries.iter().flat_map(|e| e.bytes()).collect()
    };
    ok &= dst.write(&entry_bytes(&entries));

    let mut image_offset = dst.tell();
    for entry in entries.iter_mut() {
        ok &= write_icon_surface(dst, surface);

        let next_offset = dst.tell();
        let dw_image_size = (next_offset - image_offset) as u32;
        let dw_image_offset = (image_offset - base_offset) as u32;

        ok &= fill_icon_entry(entry, surface, hot_x, hot_y, dw_image_size, dw_image_offset);

        image_offset = next_offset;
    }

    // Now that we have the icon entries filled out, rewrite them
    ok &= dst.seek(base_offset + CURSORICONFILEDIR_SIZE as i64);
    ok &= dst.write(&entry_bytes(&entries));
    ok &= dst.seek(image_offset);

    ok &= save_chunk_size(dst, icon_size_offset);

    ok
}

/* Windows doesn't have an API to easily create animated cursors from a sequence of images,
 * so we have to build an animated cursor resource file in memory and load it.
 */
/// Translation of `WIN_CreateAnimatedCursorInternal()`.
fn create_animated_cursor_internal(
    frames: &mut [(Surface<'static>, u32)],
    hot_x: i32,
    hot_y: i32,
    scale: f32,
) -> Result<HCURSOR> {
    let w = crate::stdlib::math::roundf(frames[0].0.width() as f32 * scale) as i32;
    let h = crate::stdlib::math::roundf(frames[0].0.height() as f32 * scale) as i32;

    let data = write_animated_cursor(frames, hot_x, hot_y, scale)?;

    let size = data.len() as u32;
    // SAFETY: the buffer holds `size` bytes of the resource.
    let hcursor = unsafe { CreateIconFromResourceEx(data.as_ptr(), size, 0, 0x00030000, w, h, 0) };
    if hcursor.is_null() {
        return Err(Error::new("CreateIconFromResource failed"));
    }

    Ok(hcursor)
}

/// The animated cursor resource `WIN_CreateAnimatedCursorInternal()`
/// writes in memory (split out so it can be checked byte by byte).
fn write_animated_cursor(
    frames: &mut [(Surface<'static>, u32)],
    hot_x: i32,
    hot_y: i32,
    scale: f32,
) -> Result<Vec<u8>> {
    let mut dst = DynamicMem {
        data: Vec::new(),
        pos: 0,
    };
    let frame_count = frames.len() as u32;

    let mut ok = true;
    // RIFF header
    ok &= dst.write_u32_le(riff_fourcc(b'R', b'I', b'F', b'F'));
    let riff_size_offset = dst.tell();
    ok &= dst.write_u32_le(0);
    ok &= dst.write_u32_le(riff_fourcc(b'A', b'C', b'O', b'N'));

    // anih header chunk
    ok &= dst.write_u32_le(riff_fourcc(b'a', b'n', b'i', b'h'));
    ok &= dst.write_u32_le(ANIHEADER_SIZE);

    // ANIHEADER: cbSizeof, frames, steps, width, height, bpp, planes, jifRate, fl
    let anih = [
        ANIHEADER_SIZE,
        frame_count,
        frame_count,
        0,
        0,
        0,
        0,
        1,
        ANI_FLAG_ICON,
    ];
    for v in anih {
        ok &= dst.write_u32_le(v);
    }

    // Rate chunk
    ok &= dst.write_u32_le(riff_fourcc(b'r', b'a', b't', b'e'));
    ok &= dst.write_u32_le(4 * frame_count);
    for frame in frames.iter() {
        // Animated Win32 cursors are in jiffy units, and one jiffy is 1/60 of a second.
        const WIN32_JIFFY: f64 = 1000.0 / 60.0;
        let duration = if frame.1 != 0 {
            crate::stdlib::math::lround(frame.1 as f64 / WIN32_JIFFY) as u32
        } else {
            0xFFFFFFFF
        };
        ok &= dst.write_u32_le(duration);
    }

    // Frame list
    ok &= dst.write_u32_le(riff_fourcc(b'L', b'I', b'S', b'T'));
    let frame_list_size_offset = dst.tell();
    ok &= dst.write_u32_le(0);
    ok &= dst.write_u32_le(riff_fourcc(b'f', b'r', b'a', b'm'));

    for frame in frames.iter_mut() {
        ok &= write_icon_frame(&mut dst, &mut frame.0, hot_x, hot_y, scale);
    }
    ok &= save_chunk_size(&mut dst, frame_list_size_offset);

    // All done!
    ok &= save_chunk_size(&mut dst, riff_size_offset);
    if !ok {
        // (upstream: the error has been set above)
        return Err(Error::new("Couldn't write the animated cursor"));
    }

    Ok(dst.data)
}

/// Translation of `WIN_CreateCursor()`.
pub(crate) fn create_cursor(surface: &Surface<'_>, hot_x: i32, hot_y: i32) -> Result<Cursor> {
    let frame = CursorFrame {
        surface,
        duration: 0,
    };
    create_animated_cursor_and_data(&[frame], hot_x, hot_y)
}

/// Translation of `WIN_CreateAnimatedCursor()`.
pub(crate) fn create_animated_cursor(
    frames: &[CursorFrame<'_>],
    hot_x: i32,
    hot_y: i32,
) -> Result<Cursor> {
    create_animated_cursor_and_data(frames, hot_x, hot_y)
}

/// Translation of `WIN_CreateBlankCursor()`.
fn create_blank_cursor() -> Option<Cursor> {
    let surface = Surface::new(32, 32, PixelFormat::ARGB8888).ok()?;
    create_cursor(&surface, 0, 0).ok()
}

/// Translation of `WIN_CreateSystemCursor()`.
pub(crate) fn create_system_cursor(id: SystemCursor) -> Result<Cursor> {
    use SystemCursor::*;
    let name = match id {
        Default => IDC_ARROW,
        Text | VerticalText => IDC_IBEAM,
        Wait => IDC_WAIT,
        Crosshair | Cell => IDC_CROSS,
        Progress => IDC_APPSTARTING,
        NwseResize | NwResize | SeResize => IDC_SIZENWSE,
        NeswResize | NeResize | SwResize => IDC_SIZENESW,
        EwResize | EResize | WResize | ColResize => IDC_SIZEWE,
        NsResize | NResize | SResize | RowResize => IDC_SIZENS,
        Move | AllScroll => IDC_SIZEALL,
        NotAllowed | NoDrop => IDC_NO,
        Pointer | ContextMenu | Alias | Copy | Grab | Grabbing | ZoomIn | ZoomOut => IDC_HAND,
        Help => IDC_HELP,
    };
    // SAFETY: a predefined cursor of the system.
    let hcursor = unsafe { LoadCursorW(std::ptr::null_mut(), name) };
    create_cursor_and_data(hcursor).ok_or_else(|| Error::new("LoadCursor() failed"))
}

/// Translation of `WIN_CreateDefaultCursor()`.
fn create_default_cursor() -> Option<Cursor> {
    let id = mouse::default_system_cursor();
    create_system_cursor(id).ok()
}

/// Translation of `GetCachedCursor()`.
fn get_cached_cursor(data: &CursorData) -> HCURSOR {
    let mut scale = 1.0f32;
    if hints::get_bool(hints::MOUSE_DPI_SCALE_CURSORS, false) {
        scale = mouse::mouse_focus()
            .and_then(|w| video_window::display_for_window(w).ok())
            .and_then(|d| display::display_content_scale(d).ok())
            .unwrap_or(0.0);
        if scale == 0.0 {
            scale = 1.0;
        }
    }
    {
        let cache = data.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, entry)) = cache.cache.iter().find(|(s, _)| *s == scale) {
            return entry.0;
        }
    }

    // Need to create a cursor for this content scale
    let mut frames = data.frames.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(hcursor) = create_animated_cursor_internal(&mut frames, data.hot_x, data.hot_y, scale)
    else {
        return std::ptr::null_mut();
    };

    let mut cache = data.cache.lock().unwrap_or_else(|e| e.into_inner());
    cache.cache.insert(0, (scale, Handle(hcursor)));

    hcursor
}

/// Translation of `WIN_ShowCursor()`.
pub(crate) fn show_cursor(cursor: Option<&Cursor>) -> Result<()> {
    let blank;
    let mut cursor = cursor;
    if cursor.is_none() {
        // SAFETY: GetSystemMetrics has no preconditions.
        if unsafe { GetSystemMetrics(SM_REMOTESESSION) } != 0 {
            // Use a blank cursor so we continue to get relative motion over RDP
            blank = BLANK_CURSOR
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            cursor = blank.as_ref();
        }
    }
    let hcursor = match cursor.and_then(|c| c.internal::<CursorData>()) {
        Some(data) => {
            let num_frames = data.frames.lock().unwrap_or_else(|e| e.into_inner()).len();
            if num_frames > 0 {
                get_cached_cursor(data)
            } else {
                data.cursor.0
            }
        }
        None => std::ptr::null_mut(),
    };
    SDL_CURSOR.store(hcursor as usize, Ordering::Relaxed);
    if mouse::mouse_focus().is_some() {
        // SAFETY: a cursor handle, or NULL to hide the cursor.
        unsafe { SetCursor(hcursor) };
    }
    Ok(())
}

/// Translation of `WIN_SetCursorPos()`.
pub(crate) fn set_cursor_pos(x: i32, y: i32) {
    // We need to jitter the value because otherwise Windows will occasionally inexplicably ignore the SetCursorPos() or SendInput()
    // SAFETY: SetCursorPos has no preconditions.
    unsafe {
        SetCursorPos(x, y);
        SetCursorPos(x + 1, y);
        SetCursorPos(x, y);
    }

    // Flush any mouse motion prior to or associated with this warp
    // SAFETY: GetTickCount has no preconditions.
    let mut time = unsafe { GetTickCount() };
    if time == 0 {
        time = 1;
    }
    LAST_WARP_TIME.store(time, Ordering::Relaxed);
}

/// Translation of `WIN_WarpMouse()`.
pub(crate) fn warp_mouse(window: WindowID, x: f32, y: f32) -> Result<()> {
    let Some(data) = window_data(window) else {
        return Ok(());
    };
    let hwnd = data.hwnd;

    // Don't warp the mouse while we're doing a modal interaction
    if data
        .state
        .with(|s| s.in_title_click || s.focus_click_pending != 0)
    {
        return Ok(());
    }

    let mut pt = POINT {
        x: crate::stdlib::math::roundf(x) as i32,
        y: crate::stdlib::math::roundf(y) as i32,
    };
    // SAFETY: hwnd is the window's handle.
    unsafe { ClientToScreen(hwnd, &mut pt) };
    set_cursor_pos(pt.x, pt.y);

    // Send the exact mouse motion associated with this warp
    mouse::send_mouse_motion(Duration::ZERO, Some(window), GLOBAL_MOUSE_ID, false, x, y);
    Ok(())
}

/// Translation of `WIN_WarpMouseGlobal()`.
pub(crate) fn warp_mouse_global(x: f32, y: f32) -> Result<()> {
    let pt = POINT {
        x: crate::stdlib::math::roundf(x) as i32,
        y: crate::stdlib::math::roundf(y) as i32,
    };
    // SAFETY: SetCursorPos has no preconditions.
    unsafe { SetCursorPos(pt.x, pt.y) };
    Ok(())
}

/// Translation of `WIN_SetRelativeMouseMode()`.
pub(crate) fn set_relative_mouse_mode(data: &VideoData, enabled: bool) -> Result<()> {
    rawinput::set_raw_mouse_enabled(data, enabled)
}

/// Translation of `WIN_CaptureMouse()`.
pub(crate) fn capture_mouse(window: Option<WindowID>) -> Result<()> {
    if let Some(window) = window {
        if let Some(data) = window_data(window) {
            // SAFETY: the window's handle.
            unsafe { SetCapture(data.hwnd) };
        }
    } else {
        if let Some(focus_window) = mouse::mouse_focus() {
            if let Some(data) = window_data(focus_window) {
                if !data.state.with(|s| s.mouse_tracked) {
                    mouse::set_mouse_focus(None);
                }
            }
        }
        // SAFETY: ReleaseCapture has no preconditions.
        unsafe { ReleaseCapture() };
    }

    Ok(())
}

/// Translation of `WIN_GetGlobalMouseState()`.
pub(crate) fn get_global_mouse_state() -> (f32, f32, MouseButtonFlags) {
    let mut result = 0u32;
    let mut pt = POINT { x: 0, y: 0 };
    // SAFETY: GetSystemMetrics, GetCursorPos and GetAsyncKeyState have no
    // preconditions beyond a valid out pointer.
    unsafe {
        let swap_buttons = GetSystemMetrics(SM_SWAPBUTTON) != 0;

        GetCursorPos(&mut pt);

        let down = |vk: u16| (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0;
        if down(if !swap_buttons {
            VK_LBUTTON
        } else {
            VK_RBUTTON
        }) {
            result |= MouseButtonFlags::LMASK.0;
        }
        if down(if !swap_buttons {
            VK_RBUTTON
        } else {
            VK_LBUTTON
        }) {
            result |= MouseButtonFlags::RMASK.0;
        }
        if down(VK_MBUTTON) {
            result |= MouseButtonFlags::MMASK.0;
        }
        if down(VK_XBUTTON1) {
            result |= MouseButtonFlags::X1MASK.0;
        }
        if down(VK_XBUTTON2) {
            result |= MouseButtonFlags::X2MASK.0;
        }
    }

    (pt.x as f32, pt.y as f32, MouseButtonFlags(result))
}

/// Translation of `WIN_ApplySystemScale()`.
pub(crate) fn apply_system_scale(
    _videodata: &VideoData,
    _timestamp: Duration,
    window: Option<WindowID>,
    _mouse_id: MouseID,
    x: f32,
    y: f32,
) -> (f32, f32) {
    let display = match window {
        Some(window) => video_window::display_for_window(window).ok(),
        None => display::primary_display().ok(),
    };
    let content_scale = display.and_then(|d| display::display_content_scale(d).ok());

    let mut data = SYSTEM_SCALE_DATA.lock().unwrap_or_else(|e| e.into_inner());

    let mut ix = (x as i64).wrapping_mul(65536);
    let mut iy = (y as i64).wrapping_mul(65536);
    let mut dpi = match content_scale {
        Some(scale) => (scale * USER_DEFAULT_SCREEN_DPI as f32) as u32,
        None => USER_DEFAULT_SCREEN_DPI,
    };

    if !data.enhanced {
        // early return if flat scale
        dpi = data.dpiscale.wrapping_mul(if data.dpiaware {
            dpi
        } else {
            USER_DEFAULT_SCREEN_DPI
        });
        ix = ix.wrapping_mul(dpi as i64);
        iy = iy.wrapping_mul(dpi as i64);
        ix /= USER_DEFAULT_SCREEN_DPI as i64;
        iy /= USER_DEFAULT_SCREEN_DPI as i64;
        ix /= 32;
        iy /= 32;
        // data->residual[0] += ix;
        // data->residual[1] += iy;
        // ix = 65536 * (data->residual[0] / 65536);
        // iy = 65536 * (data->residual[1] / 65536);
        // data->residual[0] -= ix;
        // data->residual[1] -= iy;
        return (ix as f32 / 65536.0, iy as f32 / 65536.0);
    }

    let xs = data.xs;
    let ys = data.ys;
    // FIXME (upstream): SDL_abs() takes an int, so the 64-bit motion is truncated first.
    let absx = (ix as i32).wrapping_abs() as i64 as u64;
    let absy = (iy as i32).wrapping_abs() as i64 as u64;
    let speed = absx.min(absy).wrapping_add(absx.max(absy) << 1); // super cursed approximation used by Windows
    if speed == 0 {
        return (x, y);
    }

    let mut i = 1usize;
    let mut j = 1usize;
    while i < 5 {
        j = i;
        if speed < xs[j] {
            break;
        }
        i += 1;
    }
    i -= 1;
    j -= 1;
    let k = data.last_node as usize;
    data.last_node = j as i32;

    // FIXME (upstream): the slopes mix signed and unsigned 64-bit arithmetic (C's usual conversions, reproduced here).
    let mut denom = data.dpidenom;
    let mut scale: i64 = 0;
    let mut xdiff = xs[j + 1].wrapping_sub(xs[j]) as i64;
    let mut ydiff = ys[j + 1].wrapping_sub(ys[j]) as i64;
    if xdiff != 0 {
        let slope = ydiff / xdiff;
        let inter = (slope as u64).wrapping_mul(xs[i]).wrapping_sub(ys[i]) as i64;
        scale =
            (scale as u64).wrapping_add((slope as u64).wrapping_sub((inter as u64) / speed)) as i64;
    }

    if j > k {
        denom <<= 1;
        xdiff = xs[k + 1].wrapping_sub(xs[k]) as i64;
        ydiff = ys[k + 1].wrapping_sub(ys[k]) as i64;
        if xdiff != 0 {
            let slope = ydiff / xdiff;
            let inter = (slope as u64).wrapping_mul(xs[k]).wrapping_sub(ys[k]) as i64;
            scale = (scale as u64).wrapping_add((slope as u64).wrapping_sub((inter as u64) / speed))
                as i64;
        }
    }

    scale = scale.wrapping_mul(dpi as i64);
    ix = ix.wrapping_mul(scale);
    iy = iy.wrapping_mul(scale);
    ix /= denom as i64;
    iy /= denom as i64;
    // data->residual[0] += ix;
    // data->residual[1] += iy;
    // ix = 65536 * (data->residual[0] / 65536);
    // iy = 65536 * (data->residual[1] / 65536);
    // data->residual[0] -= ix;
    // data->residual[1] -= iy;
    (ix as f32 / 65536.0, iy as f32 / 65536.0)
}

/// Translation of `WIN_InitMouse()` (the `mouse->` entry points are the
/// driver's `VideoDriver` methods).
pub(crate) fn init_mouse(data: &VideoData) {
    mouse::set_default_cursor(create_default_cursor());

    *BLANK_CURSOR.lock().unwrap_or_else(|e| e.into_inner()) = create_blank_cursor();

    update_mouse_system_scale_for(is_per_monitor_v2_dpi_aware(data));
}

/// Translation of `WIN_QuitMouse()`.
pub(crate) fn quit_mouse(_data: &VideoData) {
    let blank = BLANK_CURSOR
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    drop(blank);
}

/// Translation of `ReadMouseCurve()`.
fn read_mouse_curve(v: i32, xs: &mut [u64; 5], ys: &mut [u64; 5]) {
    let win8 = is_windows_8_or_greater();
    let mut xbuff: [u32; 10] = [
        0x00000000, 0, 0x00006e15, 0, 0x00014000, 0, 0x0003dc29, 0, 0x00280000, 0,
    ];
    let mut ybuff: [u32; 10] = [
        0x00000000,
        0,
        if win8 { 0x000111fd } else { 0x00015eb8 },
        0,
        if win8 { 0x00042400 } else { 0x00054ccd },
        0,
        if win8 { 0x0012fc00 } else { 0x00184ccd },
        0,
        if win8 { 0x01bbc000 } else { 0x02380000 },
        0,
    ];
    let mut xsize = size_of_val(&xbuff) as u32;
    let mut ysize = size_of_val(&ybuff) as u32;
    let key = utf8_to_wide("Control Panel\\Mouse");
    let xname = utf8_to_wide("SmoothMouseXCurve");
    let yname = utf8_to_wide("SmoothMouseYCurve");
    let mut open_handle: HKEY = std::ptr::null_mut();
    // SAFETY: the names are NUL-terminated and the buffers hold the sizes passed.
    unsafe {
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            0,
            KEY_READ,
            &mut open_handle,
        ) == ERROR_SUCCESS
        {
            RegQueryValueExW(
                open_handle,
                xname.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                xbuff.as_mut_ptr().cast(),
                &mut xsize,
            );
            RegQueryValueExW(
                open_handle,
                yname.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                ybuff.as_mut_ptr().cast(),
                &mut ysize,
            );
            RegCloseKey(open_handle);
        }
    }
    xs[0] = 0; // first node must always be origin
    ys[0] = 0; // first node must always be origin
    for i in 1..5 {
        xs[i] = 7 * xbuff[i * 2] as u64;
        ys[i] = ((v as u64).wrapping_mul(ybuff[i * 2] as u64)) << 17;
    }
}

/// Translation of `WIN_UpdateMouseSystemScale()`.
pub(crate) fn update_mouse_system_scale() {
    update_mouse_system_scale_for(current_is_per_monitor_v2_dpi_aware());
}

/// `WIN_UpdateMouseSystemScale()` with the result of
/// `WIN_IsPerMonitorV2DPIAware()`.
fn update_mouse_system_scale_for(dpiaware: bool) {
    // (mouse->system_scale_data is the static SYSTEM_SCALE_DATA)

    // always reinitialize to valid defaults, whether fetch was successful or not.
    let mut data = SYSTEM_SCALE_DATA.lock().unwrap_or_else(|e| e.into_inner());
    data.residual = [0, 0];
    data.dpiscale = 32;
    data.dpidenom = (10 * if is_windows_8_or_greater() { 120 } else { 150 }) << 16;
    data.dpiaware = dpiaware;
    data.enhanced = false;

    let mut v: i32 = 10;
    // SAFETY: SPI_GETMOUSESPEED writes an int.
    if unsafe { SystemParametersInfoW(SPI_GETMOUSESPEED, 0, (&mut v as *mut i32).cast(), 0) } != 0 {
        v = v.clamp(1, 20);
        data.dpiscale = v.max((v - 2) * 4).max((v - 6) * 8) as u32;
    }

    let mut params = [0i32; 3];
    // SAFETY: SPI_GETMOUSE writes three ints.
    if unsafe { SystemParametersInfoW(SPI_GETMOUSE, 0, params.as_mut_ptr().cast(), 0) } != 0 {
        data.enhanced = params[2] != 0;
        if params[2] != 0 {
            let (mut xs, mut ys) = (data.xs, data.ys);
            read_mouse_curve(v, &mut xs, &mut ys);
            data.xs = xs;
            data.ys = ys;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32_at(b: &[u8], off: usize) -> u32 {
        u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
    }
    fn u16_at(b: &[u8], off: usize) -> u16 {
        u16::from_le_bytes([b[off], b[off + 1]])
    }

    #[test]
    fn animated_cursor_resource_layout() {
        let mut a = Surface::new(4, 2, PixelFormat::ARGB8888).unwrap();
        a.fill_rect(None, 0xFF112233).unwrap();
        let b = Surface::new(4, 2, PixelFormat::ARGB8888).unwrap();
        let mut frames = vec![(a, 100), (b, 0)];
        let data = write_animated_cursor(&mut frames, 3, 1, 1.0).unwrap();

        // RIFF ACON with the chunk sizes filled in
        assert_eq!(&data[0..4], b"RIFF");
        assert_eq!(u32_at(&data, 4) as usize, data.len() - 8);
        assert_eq!(&data[8..12], b"ACON");
        assert_eq!(&data[12..16], b"anih");
        assert_eq!(u32_at(&data, 16), 36);
        assert_eq!(u32_at(&data, 20), 36); // cbSizeof
        assert_eq!((u32_at(&data, 24), u32_at(&data, 28)), (2, 2)); // frames, steps
        assert_eq!(u32_at(&data, 48), 1); // jifRate
        assert_eq!(u32_at(&data, 52), ANI_FLAG_ICON);
        assert_eq!(&data[56..60], b"rate");
        assert_eq!(u32_at(&data, 60), 8);
        assert_eq!(u32_at(&data, 64), 6); // 100ms in jiffies
        assert_eq!(u32_at(&data, 68), 0xFFFFFFFF); // forever
        assert_eq!(&data[72..76], b"LIST");
        assert_eq!(u32_at(&data, 76) as usize, data.len() - 80);
        assert_eq!(&data[80..84], b"fram");

        // The first icon chunk: a cursor directory of one entry with the hotspot
        let icon = 84;
        assert_eq!(&data[icon..icon + 4], b"icon");
        let icon_size = u32_at(&data, icon + 4) as usize;
        let dir = icon + 8;
        assert_eq!(
            (
                u16_at(&data, dir),
                u16_at(&data, dir + 2),
                u16_at(&data, dir + 4)
            ),
            (0, 2, 1)
        );
        let entry = dir + CURSORICONFILEDIR_SIZE;
        assert_eq!((data[entry], data[entry + 1]), (4, 2));
        assert_eq!((u16_at(&data, entry + 4), u16_at(&data, entry + 6)), (3, 1));
        let image_size = u32_at(&data, entry + 8) as usize;
        let image_offset = u32_at(&data, entry + 12) as usize;
        assert_eq!(
            image_offset,
            CURSORICONFILEDIR_SIZE + CURSORICONFILEDIRENTRY_SIZE
        );
        // BITMAPINFOHEADER, the bottom-up pixels, then a 4-byte-per-row mask
        assert_eq!(image_size, 40 + 4 * 2 * 4 + 4 * 2);
        assert_eq!(icon_size, image_offset + image_size);
        let bmp = dir + image_offset;
        assert_eq!(u32_at(&data, bmp), 40);
        assert_eq!((u32_at(&data, bmp + 4), u32_at(&data, bmp + 8)), (4, 4)); // double height
        assert_eq!(u32_at(&data, bmp + 40), 0xFF112233);
        // Opaque pixels clear their mask bits
        assert_eq!(
            &data[bmp + 40 + 32..bmp + 40 + 36],
            &[0x0F, 0xFF, 0xFF, 0xFF]
        );

        // The second frame is fully transparent
        let icon2 = icon + 8 + icon_size;
        assert_eq!(&data[icon2..icon2 + 4], b"icon");
        let bmp2 = icon2 + 8 + image_offset;
        assert_eq!(&data[bmp2 + 40 + 32..bmp2 + 40 + 36], &[0xFF; 4]);
        assert_eq!(icon2 + 8 + icon_size, data.len());
    }
}
