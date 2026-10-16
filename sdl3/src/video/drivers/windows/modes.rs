// Rust translation of src/video/windows/SDL_windowsmodes.c and
// SDL_windowsmodes.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Displays (monitors) and their modes: `EnumDisplayMonitors()`,
//! `EnumDisplaySettings()`, `ChangeDisplaySettingsEx()`, the friendly
//! monitor names of the display configuration API and the per-monitor DPI.
//!
//! Upstream keeps each mode's `DEVMODEW` in `SDL_DisplayMode::internal`;
//! here the display's [`DisplayData`] keeps them next to the modes they
//! belong to. As in upstream's build with `dxgi.h` and `dxgi1_6.h`, the
//! refresh rates are refined by the display's DXGI output (found through
//! the driver's `IDXGIFactory`, [`VideoData::dxgi_factory`]) and the HDR
//! properties come from `IDXGIOutput6::GetDesc1()` and the display
//! configuration's SDR white level; without `dxgi.dll` the refresh rates
//! come from `EnumDisplaySettings()` alone and no HDR is reported.

use windows_sys::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_DEVICE_INFO_TYPE, DISPLAYCONFIG_MODE_INFO,
    DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME,
    QDC_ONLY_ACTIVE_PATHS,
};
use windows_sys::Win32::Foundation::{
    ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, LPARAM, RECT, S_OK,
};
use windows_sys::Win32::Graphics::Gdi::{
    ChangeDisplaySettingsExW, CreateCompatibleBitmap, CreateDCW, DeleteDC, DeleteObject,
    EnumDisplayDevicesW, EnumDisplayMonitors, EnumDisplaySettingsW, GetDC, GetDIBits,
    GetDeviceCaps, GetMonitorInfoW, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER, BI_BITFIELDS, BI_RGB,
    CDS_FULLSCREEN, DEVMODEW, DIB_RGB_COLORS, DISPLAY_DEVICEW, DISP_CHANGE_BADFLAGS,
    DISP_CHANGE_BADMODE, DISP_CHANGE_BADPARAM, DISP_CHANGE_FAILED, DISP_CHANGE_SUCCESSFUL,
    DMDO_180, DMDO_270, DMDO_90, DMDO_DEFAULT, DM_BITSPERPEL, DM_DISPLAYFLAGS, DM_DISPLAYFREQUENCY,
    DM_PELSHEIGHT, DM_PELSWIDTH, ENUM_CURRENT_SETTINGS, HDC, HMONITOR, LOGPIXELSX, MONITORINFO,
    MONITORINFOEXW, RGBQUAD,
};
use windows_sys::Win32::UI::HiDpi::MDT_EFFECTIVE_DPI;
use windows_sys::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

use super::{VideoData, USER_DEFAULT_SCREEN_DPI};
use crate::core::windows::wide_to_utf8;
use crate::error::{Error, Result};
use crate::events::window::send_display_event;
use crate::events::{DisplayID, EventType};
use crate::loadso::SharedObject;
use crate::render::direct3d11::d3d::{
    out, DxgiOutput, IDXGIFactory1Vtbl, IDXGIOutput6Vtbl, ModeDesc, OutputDesc1,
    PfnCreateDxgiFactory, Rational, DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020,
    DXGI_FORMAT_R8G8B8A8_UNORM, IID_IDXGIFACTORY1, IID_IDXGIOUTPUT6,
};
use crate::video::core::{with_device, with_display};
use crate::video::display::{
    add_fullscreen_display_mode, add_video_display, cmpmodes, del_video_display,
    display_properties, finalize_display_mode, reset_fullscreen_display_modes,
    set_desktop_display_mode, set_display_content_scale, set_display_hdr_properties,
    PROP_DISPLAY_WINDOWS_HMONITOR_POINTER,
};
use crate::video::sysvideo::{DisplayMode, DisplayOrientation, HdrOutputProperties, VideoDisplay};
use crate::video::{PixelFormat, PixelMasks, Rect};

// #define DEBUG_MODES
// #define HIGHDPI_DEBUG_VERBOSE

/// The SDR white level of scRGB content (`SCRGB_NITS`).
const SCRGB_NITS: f32 = 80.0;

/// Translation of `WIN_DisplayState`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DisplayState {
    Unchanged,
    Added,
    Removed,
}

/// The driver data of a display. Translation of `struct SDL_DisplayData`
/// (plus the `SDL_DisplayModeData` of its modes).
pub(crate) struct DisplayData {
    pub(crate) device_name: [u16; 32],
    pub(crate) monitor_handle: HMONITOR,
    pub(crate) state: DisplayState,
    pub(crate) bounds: Rect,
    /// The `DEVMODEW` of the desktop mode (`desktop_mode.internal`).
    desktop_device_mode: DEVMODEW,
    /// The fullscreen modes with their `DEVMODEW`s (the `internal` of the
    /// display's `fullscreen_modes`), as finalized by the video core.
    modes: Vec<(DisplayMode, DEVMODEW)>,
}

// SAFETY: the monitor handle is a process-wide token, usable from any thread.
unsafe impl Send for DisplayData {}

/// Run `f` on a display's driver data.
pub(crate) fn with_display_data<R>(
    display: DisplayID,
    f: impl FnOnce(&mut DisplayData) -> R,
) -> Option<R> {
    with_display(display, |d| {
        d.internal
            .as_mut()
            .and_then(|i| i.downcast_mut::<DisplayData>())
            .map(f)
    })
    .flatten()
}

/// The driver data of a display in the device's list.
fn display_data_of(display: &mut VideoDisplay) -> Option<&mut DisplayData> {
    display
        .internal
        .as_mut()
        .and_then(|i| i.downcast_mut::<DisplayData>())
}

/// `SDL_wcscmp(a, b) == 0` for NUL-terminated arrays.
pub(crate) fn wide_eq(a: &[u16], b: &[u16]) -> bool {
    let end = |s: &[u16]| s.iter().position(|&c| c == 0).unwrap_or(s.len());
    a[..end(a)] == b[..end(b)]
}

/// Fill in the format of a mode. Translation of `WIN_UpdateDisplayMode()`.
fn update_display_mode(
    device_name: &[u16; 32],
    index: u32,
    mode: &mut DisplayMode,
    device_mode: &mut DEVMODEW,
) {
    device_mode.dmFields =
        DM_BITSPERPEL | DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY | DM_DISPLAYFLAGS;

    // SAFETY: device_name is NUL-terminated (from the system).
    let hdc: HDC = if index == ENUM_CURRENT_SETTINGS {
        unsafe {
            CreateDCW(
                device_name.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            )
        }
    } else {
        std::ptr::null_mut()
    };
    if !hdc.is_null() {
        #[repr(C)]
        struct BmiData {
            header: BITMAPINFOHEADER,
            colors: [RGBQUAD; 256],
        }
        // SAFETY: BmiData is plain data; all zeroes is valid.
        let mut bmi_data: BmiData = unsafe { std::mem::zeroed() };
        bmi_data.header.biSize = size_of::<BITMAPINFOHEADER>() as u32;

        // SAFETY: hdc is a live DC; bmi_data has room for the header and a
        // full color table, which is what GetDIBits fills in.
        unsafe {
            let bmi = (&mut bmi_data as *mut BmiData).cast::<BITMAPINFO>();
            let hbm = CreateCompatibleBitmap(hdc, 1, 1);
            GetDIBits(hdc, hbm, 0, 1, std::ptr::null_mut(), bmi, DIB_RGB_COLORS);
            GetDIBits(hdc, hbm, 0, 1, std::ptr::null_mut(), bmi, DIB_RGB_COLORS);
            DeleteObject(hbm);
            DeleteDC(hdc);
        }
        if bmi_data.header.biCompression == BI_BITFIELDS {
            let first_mask = u32::from_le_bytes([
                bmi_data.colors[0].rgbBlue,
                bmi_data.colors[0].rgbGreen,
                bmi_data.colors[0].rgbRed,
                bmi_data.colors[0].rgbReserved,
            ]);
            match first_mask {
                0x00FF0000 => mode.format = PixelFormat::XRGB8888,
                0x000000FF => mode.format = PixelFormat::XBGR8888,
                0xF800 => mode.format = PixelFormat::RGB565,
                0x7C00 => mode.format = PixelFormat::XRGB1555,
                _ => {}
            }
        } else if bmi_data.header.biCompression == BI_RGB {
            if bmi_data.header.biBitCount == 24 {
                mode.format = PixelFormat::RGB24;
            } else if bmi_data.header.biBitCount == 8 {
                mode.format = PixelFormat::INDEX8;
            } else if bmi_data.header.biBitCount == 4 {
                mode.format = PixelFormat::INDEX4LSB;
            }
        }
    } else if mode.format == PixelFormat::UNKNOWN {
        // FIXME: Can we tell what this will be?
        if (device_mode.dmFields & DM_BITSPERPEL) == DM_BITSPERPEL {
            match device_mode.dmBitsPerPel {
                32 => mode.format = PixelFormat::XRGB8888,
                24 => mode.format = PixelFormat::RGB24,
                16 => mode.format = PixelFormat::RGB565,
                15 => mode.format = PixelFormat::XRGB1555,
                8 => mode.format = PixelFormat::INDEX8,
                4 => mode.format = PixelFormat::INDEX4LSB,
                _ => {}
            }
        }
    }
}

/// The DXGI output of a display device, when the driver has a DXGI
/// factory and one of its adapters' outputs has that name. Translation of
/// `WIN_GetDXGIOutput()` (released when dropped: `WIN_ReleaseDXGIOutput()`).
fn get_dxgi_output(videodata: &VideoData, device_name: &[u16]) -> Option<DxgiOutput> {
    let factory = videodata
        .dxgi_factory
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let factory = factory.as_ref()?;

    let mut result = None;
    let mut n_adapter = 0;
    while result.is_none() {
        let Ok(adapter) = factory.enum_adapters(n_adapter) else {
            break;
        };
        let mut n_output = 0;
        while result.is_none() {
            let Ok(output) = adapter.enum_outputs(n_output) else {
                break;
            };
            if let Ok(output_desc) = output.desc() {
                if wide_eq(&output_desc.device_name, device_name) {
                    result = Some(output);
                }
            }
            // (an output that isn't the result is released here)
            n_output += 1;
        }
        n_adapter += 1;
    }
    result
}

/// The orientation of a display when unrotated. Translation of
/// `WIN_GetNaturalOrientation()`.
fn get_natural_orientation(mode: &DEVMODEW) -> DisplayOrientation {
    let mut width = mode.dmPelsWidth as i32;
    let mut height = mode.dmPelsHeight as i32;

    // SAFETY: display modes use the display (Anonymous2) member of the union.
    let orientation = unsafe { mode.Anonymous1.Anonymous2.dmDisplayOrientation };
    // Use unrotated width/height to guess orientation
    if orientation == DMDO_90 || orientation == DMDO_270 {
        std::mem::swap(&mut width, &mut height);
    }

    if width >= height {
        DisplayOrientation::Landscape
    } else {
        DisplayOrientation::Portrait
    }
}

/// Translation of `WIN_GetDisplayOrientation()`.
fn get_display_orientation(mode: &DEVMODEW) -> DisplayOrientation {
    // SAFETY: display modes use the display (Anonymous2) member of the union.
    let orientation = unsafe { mode.Anonymous1.Anonymous2.dmDisplayOrientation };
    if get_natural_orientation(mode) == DisplayOrientation::Landscape {
        match orientation {
            DMDO_DEFAULT => DisplayOrientation::Landscape,
            DMDO_90 => DisplayOrientation::Portrait,
            DMDO_180 => DisplayOrientation::LandscapeFlipped,
            DMDO_270 => DisplayOrientation::PortraitFlipped,
            _ => DisplayOrientation::Unknown,
        }
    } else {
        match orientation {
            DMDO_DEFAULT => DisplayOrientation::Portrait,
            DMDO_90 => DisplayOrientation::LandscapeFlipped,
            DMDO_180 => DisplayOrientation::PortraitFlipped,
            DMDO_270 => DisplayOrientation::Landscape,
            _ => DisplayOrientation::Unknown,
        }
    }
}

/// The refresh rate of a mode as a fraction, from the display's DXGI
/// output's closest mode when there is one. Translation of
/// `WIN_GetRefreshRate()`.
fn get_refresh_rate(dxgi_output: Option<&DxgiOutput>, mode: &DEVMODEW) -> (i32, i32) {
    // We're not currently using DXGI to query display modes, so fake NTSC timings
    let (mut numerator, mut denominator) = match mode.dmDisplayFrequency {
        119 | 59 | 29 => ((mode.dmDisplayFrequency as i32 + 1) * 1000, 1001),
        _ => (mode.dmDisplayFrequency as i32, 1),
    };

    if let Some(output) = dxgi_output {
        let mode_to_match = ModeDesc {
            width: mode.dmPelsWidth,
            height: mode.dmPelsHeight,
            refresh_rate: Rational {
                numerator: numerator as u32,
                denominator: denominator as u32,
            },
            format: DXGI_FORMAT_R8G8B8A8_UNORM,
            ..Default::default()
        };

        if let Ok(closest_match) = output.find_closest_matching_mode(&mode_to_match) {
            numerator = closest_match.refresh_rate.numerator as i32;
            denominator = closest_match.refresh_rate.denominator as i32;
        }
    }
    (numerator, denominator)
}

/// The content scale of a monitor (its DPI over 96). Translation of
/// `WIN_GetContentScale()`.
fn get_content_scale(videodata: &VideoData, h_monitor: HMONITOR) -> f32 {
    let mut dpi: i32 = 0;

    if let Some(get_dpi_for_monitor) = videodata.get_dpi_for_monitor {
        let mut hdpi_uint: u32 = 0;
        let mut vdpi_uint: u32 = 0;
        // SAFETY: the function was loaded with this signature.
        if unsafe {
            get_dpi_for_monitor(h_monitor, MDT_EFFECTIVE_DPI, &mut hdpi_uint, &mut vdpi_uint)
        } == S_OK
        {
            dpi = hdpi_uint as i32;
        }
    }
    if dpi == 0 {
        // Window 8.0 and below: same DPI for all monitors
        // SAFETY: the screen DC is released after the query.
        unsafe {
            let hdc = GetDC(std::ptr::null_mut());
            if !hdc.is_null() {
                dpi = GetDeviceCaps(hdc, LOGPIXELSX as i32);
                ReleaseDC(std::ptr::null_mut(), hdc);
            }
        }
    }
    if dpi == 0 {
        // Safe default
        dpi = USER_DEFAULT_SCREEN_DPI as i32;
    }
    dpi as f32 / USER_DEFAULT_SCREEN_DPI as f32
}

/// A mode of a display: the mode, its `DEVMODEW` and the orientations.
/// Translation of `WIN_GetDisplayMode()`.
fn get_display_mode(
    dxgi_output: Option<&DxgiOutput>,
    device_name: &[u16; 32],
    index: u32,
) -> Option<(
    DisplayMode,
    DEVMODEW,
    DisplayOrientation,
    DisplayOrientation,
)> {
    let mut devmode = DEVMODEW {
        dmSize: size_of::<DEVMODEW>() as u16,
        dmDriverExtra: 0,
        ..Default::default()
    };
    // SAFETY: device_name is NUL-terminated; devmode is sized.
    if unsafe { EnumDisplaySettingsW(device_name.as_ptr(), index, &mut devmode) } == 0 {
        return None;
    }

    let mut data = devmode;
    let (num, den) = get_refresh_rate(dxgi_output, &data);
    let mut mode = DisplayMode {
        format: PixelFormat::UNKNOWN,
        w: data.dmPelsWidth as i32,
        h: data.dmPelsHeight as i32,
        refresh_rate_numerator: num,
        refresh_rate_denominator: den,
        ..Default::default()
    };

    // Fill in the mode information
    update_display_mode(device_name, index, &mut mode, &mut data);

    Some((
        mode,
        data,
        get_natural_orientation(&devmode),
        get_display_orientation(&devmode),
    ))
}

/// The friendly name of a monitor, from the display configuration API.
/// Translation of `WIN_GetDisplayNameVista()`.
fn get_display_name_vista(videodata: &VideoData, device_name: &[u16; 32]) -> Option<String> {
    let (Some(get_buffer_sizes), Some(query_display_config), Some(get_device_info)) = (
        videodata.get_display_config_buffer_sizes,
        videodata.query_display_config,
        videodata.display_config_get_device_info,
    ) else {
        return None;
    };

    let mut paths: Vec<DISPLAYCONFIG_PATH_INFO>;
    let mut modes: Vec<DISPLAYCONFIG_MODE_INFO>;
    let mut path_count: u32 = 0;
    let mut mode_count: u32 = 0;
    let mut result = None;

    loop {
        // SAFETY: the functions were loaded with these signatures; the
        // buffers hold the counts we pass.
        let rc = unsafe {
            let rc = get_buffer_sizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count);
            if rc as u32 != ERROR_SUCCESS {
                return None; // WIN_GetDisplayNameVista_failed
            }

            paths = vec![std::mem::zeroed(); path_count as usize];
            modes = vec![std::mem::zeroed(); mode_count as usize];

            query_display_config(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                paths.as_mut_ptr(),
                &mut mode_count,
                modes.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        };
        if rc as u32 != ERROR_INSUFFICIENT_BUFFER {
            if rc as u32 == ERROR_SUCCESS {
                for path in paths.iter().take(path_count as usize) {
                    // SAFETY: plain data; all zeroes is valid.
                    let mut source_name: DISPLAYCONFIG_SOURCE_DEVICE_NAME =
                        unsafe { std::mem::zeroed() };
                    source_name.header.adapterId = path.targetInfo.adapterId;
                    source_name.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
                    source_name.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
                    source_name.header.id = path.sourceInfo.id;
                    // SAFETY: the header heads a sized source name request.
                    let rc = unsafe { get_device_info(&mut source_name.header) };
                    if rc as u32 != ERROR_SUCCESS {
                        break;
                    } else if !wide_eq(device_name, &source_name.viewGdiDeviceName) {
                        continue;
                    }

                    // SAFETY: plain data; all zeroes is valid.
                    let mut target_name: DISPLAYCONFIG_TARGET_DEVICE_NAME =
                        unsafe { std::mem::zeroed() };
                    target_name.header.adapterId = path.targetInfo.adapterId;
                    target_name.header.id = path.targetInfo.id;
                    target_name.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
                    target_name.header.size = size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
                    // SAFETY: the header heads a sized target name request.
                    let rc = unsafe { get_device_info(&mut target_name.header) };
                    if rc as u32 == ERROR_SUCCESS {
                        let name = wide_to_utf8(&target_name.monitorFriendlyDeviceName);
                        /* if we got an empty string, treat it as failure so we'll fallback
                        to getting the generic name. */
                        if !name.is_empty() {
                            result = Some(name);
                        }
                    }
                    break;
                }
            }
            break;
        }
    }
    result
}

/// The `DXGI_OUTPUT_DESC1` of a monitor, from a DXGI 1.1 factory made for
/// the call (`dxgi.dll` is loaded and unloaded here, as upstream does).
/// Translation of `WIN_GetMonitorDESC1()`.
fn get_monitor_desc1(h_monitor: HMONITOR) -> Option<OutputDesc1> {
    let h_dxgi_mod = SharedObject::load("dxgi.dll").ok();
    // SAFETY: CreateDXGIFactory1 has this signature (dxgi.h).
    let create_dxgi_factory1: Option<PfnCreateDxgiFactory> =
        unsafe { super::load_function(h_dxgi_mod.as_ref(), "CreateDXGIFactory1") };

    let mut found = None;
    if let Some(create_dxgi_factory1) = create_dxgi_factory1 {
        // SAFETY: the function stores an owned IDXGIFactory1, the interface
        // asked for.
        let dxgi_factory =
            unsafe { out::<IDXGIFactory1Vtbl>(|o| create_dxgi_factory1(&IID_IDXGIFACTORY1, o)) };
        if let Ok(dxgi_factory) = dxgi_factory {
            let mut adapter = 0;
            while found.is_none() {
                let Ok(dxgi_adapter) = dxgi_factory.enum_adapters1(adapter) else {
                    break;
                };
                let mut output = 0;
                while found.is_none() {
                    let Ok(dxgi_output) = dxgi_adapter.enum_outputs(output) else {
                        break;
                    };
                    if let Ok(dxgi_output6) =
                        dxgi_output.query::<IDXGIOutput6Vtbl>(&IID_IDXGIOUTPUT6)
                    {
                        if let Ok(desc) = dxgi_output6.desc1() {
                            if desc.monitor == h_monitor {
                                found = Some(desc);
                            }
                        }
                    }
                    output += 1;
                }
                adapter += 1;
            }
        }
    }
    // (the factory, adapters and outputs are released before h_dxgi_mod is
    // unloaded, when it goes out of scope)
    found
}

/// The display configuration path of a monitor. Translation of
/// `WIN_GetMonitorPathInfo()`.
fn get_monitor_path_info(
    videodata: &VideoData,
    h_monitor: HMONITOR,
) -> Option<DISPLAYCONFIG_PATH_INFO> {
    let (Some(get_buffer_sizes), Some(query_display_config), Some(get_device_info)) = (
        videodata.get_display_config_buffer_sizes,
        videodata.query_display_config,
        videodata.display_config_get_device_info,
    ) else {
        return None;
    };

    let mut view_info = MONITORINFOEXW::default();
    view_info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: view_info is a sized MONITORINFOEXW.
    if unsafe { GetMonitorInfoW(h_monitor, (&mut view_info as *mut MONITORINFOEXW).cast()) } == 0 {
        return None;
    }

    let mut path_infos: Vec<DISPLAYCONFIG_PATH_INFO>;
    let mut mode_infos: Vec<DISPLAYCONFIG_MODE_INFO>;
    let mut num_path_array_elements: u32 = 0;
    let mut num_mode_info_array_elements: u32 = 0;
    let result = loop {
        // SAFETY: the functions were loaded with these signatures; the
        // buffers hold the counts we pass.
        let result = unsafe {
            if get_buffer_sizes(
                QDC_ONLY_ACTIVE_PATHS,
                &mut num_path_array_elements,
                &mut num_mode_info_array_elements,
            ) as u32
                != ERROR_SUCCESS
            {
                return None;
            }

            path_infos = vec![std::mem::zeroed(); num_path_array_elements as usize];
            mode_infos = vec![std::mem::zeroed(); num_mode_info_array_elements as usize];

            query_display_config(
                QDC_ONLY_ACTIVE_PATHS,
                &mut num_path_array_elements,
                path_infos.as_mut_ptr(),
                &mut num_mode_info_array_elements,
                mode_infos.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        };
        if result as u32 != ERROR_INSUFFICIENT_BUFFER {
            break result;
        }
    };

    if result as u32 == ERROR_SUCCESS {
        for path_info in path_infos.iter().take(num_path_array_elements as usize) {
            // SAFETY: plain data; all zeroes is valid.
            let mut device_name: DISPLAYCONFIG_SOURCE_DEVICE_NAME = unsafe { std::mem::zeroed() };
            device_name.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            device_name.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            device_name.header.adapterId = path_info.sourceInfo.adapterId;
            device_name.header.id = path_info.sourceInfo.id;
            // SAFETY: the header heads a sized source name request.
            if unsafe { get_device_info(&mut device_name.header) } as u32 == ERROR_SUCCESS
                && wide_eq(&view_info.szDevice, &device_name.viewGdiDeviceName)
            {
                return Some(*path_info);
            }
        }
    }

    None
}

/// The SDR white level of a monitor, relative to 80 nits (1.0 when it
/// can't be read). Translation of `WIN_GetSDRWhitePoint()`.
fn get_sdr_white_point(videodata: &VideoData, h_monitor: HMONITOR) -> f32 {
    let mut sdr_white_level = 1.0f32;

    if let Some(path_info) = get_monitor_path_info(videodata, h_monitor) {
        /* workarounds for https://github.com/libsdl-org/SDL/issues/11193 */
        #[repr(C)]
        struct SdlDisplayconfigSdrWhiteLevel {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
            sdr_white_level: u32,
        }
        const DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL: DISPLAYCONFIG_DEVICE_INFO_TYPE = 11;

        // SAFETY: plain data; all zeroes is valid.
        let mut white_level: SdlDisplayconfigSdrWhiteLevel = unsafe { std::mem::zeroed() };
        white_level.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL;
        white_level.header.size = size_of::<SdlDisplayconfigSdrWhiteLevel>() as u32;
        white_level.header.adapterId = path_info.targetInfo.adapterId;
        white_level.header.id = path_info.targetInfo.id;
        // WIN_GetMonitorPathInfo() succeeded: DisplayConfigGetDeviceInfo is not NULL
        let get_device_info = videodata
            .display_config_get_device_info
            .expect("get_monitor_path_info() checked it");
        // SAFETY: the header heads a sized white level request.
        if unsafe { get_device_info(&mut white_level.header) } as u32 == ERROR_SUCCESS
            && white_level.sdr_white_level > 0
        {
            sdr_white_level = white_level.sdr_white_level as f32 / 1000.0;
        }
    }
    sdr_white_level
}

/// The HDR properties of a monitor: the headroom and SDR white level of
/// one in HDR mode (its output's color space is PQ), else none.
/// Translation of `WIN_GetHDRProperties()`.
fn get_hdr_properties(videodata: &VideoData, h_monitor: HMONITOR) -> HdrOutputProperties {
    let mut hdr = HdrOutputProperties::default();

    if let Some(desc) = get_monitor_desc1(h_monitor) {
        if desc.color_space == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020 {
            hdr.sdr_white_level = get_sdr_white_point(videodata, h_monitor);
            hdr.hdr_headroom = (desc.max_luminance / SCRGB_NITS) / hdr.sdr_white_level;
        }
    }
    hdr
}

/// Bounds from `GetMonitorInfo()`'s monitor rectangle (`WIN_GetDisplayBounds()`
/// for a monitor handle).
fn monitor_bounds(h_monitor: HMONITOR) -> Result<Rect> {
    let mut minfo = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: minfo is sized.
    let rc = unsafe { GetMonitorInfoW(h_monitor, &mut minfo) };

    if rc == 0 {
        return Err(Error::new("Couldn't find monitor data"));
    }

    Ok(Rect::new(
        minfo.rcMonitor.left,
        minfo.rcMonitor.top,
        minfo.rcMonitor.right - minfo.rcMonitor.left,
        minfo.rcMonitor.bottom - minfo.rcMonitor.top,
    ))
}

/// Add a monitor, or refresh it if it's already known (moving it to
/// `display_index` in the display list). Translation of `WIN_AddDisplay()`.
fn add_display(
    videodata: &VideoData,
    h_monitor: HMONITOR,
    info: &MONITORINFOEXW,
    display_index: &mut usize,
) {
    let index = *display_index;
    let content_scale = get_content_scale(videodata, h_monitor);

    let dxgi_output = get_dxgi_output(videodata, &info.szDevice);
    let found = get_display_mode(dxgi_output.as_ref(), &info.szDevice, ENUM_CURRENT_SETTINGS);
    drop(dxgi_output); // WIN_ReleaseDXGIOutput()
    let Some((mode, devmode, natural_orientation, current_orientation)) = found else {
        return;
    };

    // Prevent adding duplicate displays. Do this after we know the display is
    // ready to be added to allow any displays that we can't fully query to be
    // removed
    enum Found {
        /// Not in the list yet
        New,
        /// `goto cleanup`
        Cleanup,
        /// An existing display, with whether it moved
        Existing(DisplayID, bool, Rect),
    }
    let found = with_device(|v| {
        let num_displays = v.displays.len();
        for i in 0..num_displays {
            let Some(internal) = display_data_of(&mut v.displays[i]) else {
                continue;
            };
            if wide_eq(&internal.device_name, &info.szDevice) {
                let moved = index != i;

                if internal.state != DisplayState::Removed {
                    // We've already enumerated this display, don't move it
                    return Found::Cleanup;
                }

                if index >= num_displays {
                    // This should never happen due to the check above, but just in case...
                    return Found::Cleanup;
                }

                let mut i = i;
                if moved {
                    v.displays.swap(index, i);
                    i = index;
                }

                let id = v.displays[i].id;
                let internal = display_data_of(&mut v.displays[i]).expect("display data");
                internal.monitor_handle = h_monitor;
                internal.state = DisplayState::Unchanged;
                return Found::Existing(id, moved, internal.bounds);
            }
        }
        Found::New
    })
    .unwrap_or(Found::Cleanup);

    match found {
        Found::Cleanup => {}
        Found::Existing(id, moved, old_bounds) => {
            if let Ok(props) = display_properties(id) {
                let _ = props.set(PROP_DISPLAY_WINDOWS_HMONITOR_POINTER, h_monitor as i64);
            }

            if !with_device(|v| v.setting_display_mode).unwrap_or(false) {
                let mut changed_bounds = false;

                reset_fullscreen_display_modes(id);
                with_display_data(id, |d| d.modes.clear());
                set_desktop_display_mode(id, &mode);
                // The mode is owned by the video subsystem
                with_display_data(id, |d| d.desktop_device_mode = devmode);
                if let Ok(bounds) = monitor_bounds(h_monitor) {
                    if old_bounds != bounds {
                        changed_bounds = true;
                        with_display_data(id, |d| d.bounds = bounds);
                    }
                }
                if moved || changed_bounds {
                    send_display_event(id, EventType::DISPLAY_MOVED, 0, 0);
                }
                send_display_event(
                    id,
                    EventType::DISPLAY_ORIENTATION,
                    current_orientation as i32,
                    0,
                );
                set_display_content_scale(id, content_scale);
                let hdr = get_hdr_properties(videodata, h_monitor);
                set_display_hdr_properties(id, hdr);
            }
            *display_index += 1;
        }
        Found::New => {
            let mut display = VideoDisplay::new();
            display.name = get_display_name_vista(videodata, &info.szDevice);
            if display.name.is_none() {
                let mut device = DISPLAY_DEVICEW {
                    cb: size_of::<DISPLAY_DEVICEW>() as u32,
                    ..Default::default()
                };
                // SAFETY: szDevice is NUL-terminated; device is sized.
                if unsafe { EnumDisplayDevicesW(info.szDevice.as_ptr(), 0, &mut device, 0) } != 0 {
                    display.name = Some(wide_to_utf8(&device.DeviceString));
                }
            }

            display.desktop_mode = mode;
            display.natural_orientation = natural_orientation;
            display.current_orientation = current_orientation;
            display.content_scale = content_scale;
            let bounds = monitor_bounds(h_monitor).unwrap_or_default();
            display.hdr = get_hdr_properties(videodata, h_monitor);
            display.internal = Some(Box::new(DisplayData {
                device_name: info.szDevice,
                monitor_handle: h_monitor,
                state: DisplayState::Added,
                bounds,
                desktop_device_mode: devmode,
                modes: Vec::new(),
            }));
            let display_id = add_video_display(display, false);
            if display_id != 0 {
                // The mode is owned by the video subsystem
                if let Ok(props) = display_properties(display_id) {
                    let _ = props.set(PROP_DISPLAY_WINDOWS_HMONITOR_POINTER, h_monitor as i64);
                }
            }
            *display_index += 1;
        }
    }
}

/// Translation of `WIN_AddDisplaysData`.
struct AddDisplaysData<'a> {
    video_data: &'a VideoData,
    display_index: usize,
    want_primary: bool,
}

/// Translation of `WIN_AddDisplaysCallback()`.
unsafe extern "system" fn add_displays_callback(
    h_monitor: HMONITOR,
    _hdc_monitor: HDC,
    _lprc_monitor: *mut RECT,
    dw_data: LPARAM,
) -> i32 {
    // SAFETY: dw_data is the AddDisplaysData add_displays() passed, alive
    // for the duration of EnumDisplayMonitors().
    let data = unsafe { &mut *(dw_data as *mut AddDisplaysData<'_>) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;

    // SAFETY: info is a sized MONITORINFOEXW.
    if unsafe { GetMonitorInfoW(h_monitor, (&mut info as *mut MONITORINFOEXW).cast()) } != 0 {
        let is_primary = (info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) == MONITORINFOF_PRIMARY;

        if is_primary == data.want_primary {
            add_display(data.video_data, h_monitor, &info, &mut data.display_index);
        }
    }

    // continue enumeration
    1
}

/// Enumerate the monitors, the primary one first. Translation of
/// `WIN_AddDisplays()`.
fn add_displays(videodata: &VideoData) {
    let mut callback_data = AddDisplaysData {
        video_data: videodata,
        display_index: 0,
        want_primary: true,
    };

    // SAFETY: the callback gets a pointer to callback_data, which outlives
    // both enumerations.
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(add_displays_callback),
            &mut callback_data as *mut AddDisplaysData<'_> as LPARAM,
        );
    }

    callback_data.want_primary = false;
    // SAFETY: as above.
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(add_displays_callback),
            &mut callback_data as *mut AddDisplaysData<'_> as LPARAM,
        );
    }
}

/// Translation of `WIN_InitModes()`.
pub(crate) fn init_modes(videodata: &VideoData) -> Result<()> {
    add_displays(videodata);

    if with_device(|v| v.displays.is_empty()).unwrap_or(true) {
        return Err(Error::new("No displays available"));
    }
    Ok(())
}

/// Translation of `WIN_GetDisplayBounds()`.
pub(crate) fn get_display_bounds(display: DisplayID) -> Result<Rect> {
    let h_monitor = with_display_data(display, |d| d.monitor_handle)
        .ok_or_else(|| Error::new("Couldn't find monitor data"))?;
    monitor_bounds(h_monitor)
}

/// Translation of `WIN_GetDisplayUsableBounds()`.
pub(crate) fn get_display_usable_bounds(display: DisplayID) -> Result<Rect> {
    let h_monitor = with_display_data(display, |d| d.monitor_handle)
        .ok_or_else(|| Error::new("Couldn't find monitor data"))?;
    let mut minfo = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: minfo is sized.
    let rc = unsafe { GetMonitorInfoW(h_monitor, &mut minfo) };

    if rc == 0 {
        return Err(Error::new("Couldn't find monitor data"));
    }

    Ok(Rect::new(
        minfo.rcWork.left,
        minfo.rcWork.top,
        minfo.rcWork.right - minfo.rcWork.left,
        minfo.rcWork.bottom - minfo.rcWork.top,
    ))
}

/// List a display's modes. Translation of `WIN_GetDisplayModes()`.
pub(crate) fn get_display_modes(videodata: &VideoData, display: DisplayID) {
    let Some(device_name) = with_display_data(display, |d| d.device_name) else {
        return;
    };

    let dxgi_output = get_dxgi_output(videodata, &device_name);

    // Make sure we add the current mode to the list in case it's a custom mode that doesn't enumerate
    let mut i = ENUM_CURRENT_SETTINGS;
    while let Some((mode, devmode, _, _)) = get_display_mode(dxgi_output.as_ref(), &device_name, i)
    {
        i = i.wrapping_add(1);
        if mode.format.is_indexed() {
            // We don't support palettized modes now
            continue;
        }
        if mode.format != PixelFormat::UNKNOWN && add_fullscreen_display_mode(display, &mode) {
            let mut finalized = mode;
            finalized.display_id = display;
            finalize_display_mode(&mut finalized);
            with_display_data(display, |d| d.modes.push((finalized, devmode)));
        }
    }

    drop(dxgi_output); // WIN_ReleaseDXGIOutput()
}

/// Switch a display to a mode (its desktop mode resets the display).
/// Translation of `WIN_SetDisplayMode()`.
pub(crate) fn set_display_mode(
    _videodata: &VideoData,
    display: DisplayID,
    mode: &DisplayMode,
) -> Result<()> {
    let desktop_mode =
        with_display(display, |d| d.desktop_mode).ok_or_else(|| Error::new("Invalid display"))?;
    // (upstream compares the modes' internal pointers: the desktop mode is
    // the one the video core passes from display->desktop_mode)
    let is_desktop = *mode == desktop_mode;
    let found = with_display_data(display, |d| {
        let device_mode = if is_desktop {
            Some(d.desktop_device_mode)
        } else {
            d.modes
                .iter()
                .find(|(m, _)| m == mode || cmpmodes(m, mode) == 0)
                .map(|(_, dm)| *dm)
        };
        (d.device_name, device_mode)
    });
    let Some((device_name, Some(mut device_mode))) = found else {
        return Err(Error::new("Invalid display mode"));
    };

    /* High-DPI notes:

    - ChangeDisplaySettingsEx always takes pixels.
    - e.g. if the display is set to 2880x1800 with 200% scaling in Display Settings
      - calling ChangeDisplaySettingsEx with a dmPelsWidth/Height other than 2880x1800 will
        change the monitor DPI to 96. (100% scaling)
      - calling ChangeDisplaySettingsEx with a dmPelsWidth/Height of 2880x1800 (or a NULL DEVMODE*) will
        reset the monitor DPI to 192. (200% scaling)

    NOTE: these are temporary changes in DPI, not modifications to the Control Panel setting. */
    // SAFETY: device_name is NUL-terminated; device_mode is a full DEVMODEW.
    let status = unsafe {
        if is_desktop {
            ChangeDisplaySettingsExW(
                device_name.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                CDS_FULLSCREEN,
                std::ptr::null(),
            )
        } else {
            ChangeDisplaySettingsExW(
                device_name.as_ptr(),
                &device_mode,
                std::ptr::null_mut(),
                CDS_FULLSCREEN,
                std::ptr::null(),
            )
        }
    };
    if status != DISP_CHANGE_SUCCESSFUL {
        let reason = match status {
            DISP_CHANGE_BADFLAGS => "DISP_CHANGE_BADFLAGS",
            DISP_CHANGE_BADMODE => "DISP_CHANGE_BADMODE",
            DISP_CHANGE_BADPARAM => "DISP_CHANGE_BADPARAM",
            DISP_CHANGE_FAILED => "DISP_CHANGE_FAILED",
            _ => "Unknown reason",
        };
        return Err(Error::new(format!(
            "ChangeDisplaySettingsEx() failed: {reason}"
        )));
    }

    // SAFETY: device_name is NUL-terminated; device_mode is sized.
    unsafe {
        EnumDisplaySettingsW(
            device_name.as_ptr(),
            ENUM_CURRENT_SETTINGS,
            &mut device_mode,
        )
    };
    // (upstream also updates the format of the mode it was given; the
    // video core's copy of the mode isn't reachable from here)
    let mut updated = *mode;
    update_display_mode(
        &device_name,
        ENUM_CURRENT_SETTINGS,
        &mut updated,
        &mut device_mode,
    );
    with_display_data(display, |d| {
        if is_desktop {
            d.desktop_device_mode = device_mode;
        } else if let Some(entry) = d
            .modes
            .iter_mut()
            .find(|(m, _)| m == mode || cmpmodes(m, mode) == 0)
        {
            entry.1 = device_mode;
        }
    });
    Ok(())
}

/// Re-enumerate the monitors: add new ones, drop the ones that went away.
/// Translation of `WIN_RefreshDisplays()`.
pub(crate) fn refresh_displays(videodata: &VideoData) {
    // Mark all displays as potentially invalid to detect
    // entries that have actually been removed
    let _ = with_device(|v| {
        for display in &mut v.displays {
            if let Some(internal) = display_data_of(display) {
                internal.state = DisplayState::Removed;
            }
        }
    });

    // Enumerate displays to add any new ones and mark still
    // connected entries as valid
    add_displays(videodata);

    // Delete any entries still marked as invalid, iterate
    // in reverse as each delete takes effect immediately
    let states: Vec<(DisplayID, Option<DisplayState>)> = with_device(|v| {
        v.displays
            .iter_mut()
            .map(|d| (d.id, display_data_of(d).map(|i| i.state)))
            .collect()
    })
    .unwrap_or_default();
    for &(id, state) in states.iter().rev() {
        if state == Some(DisplayState::Removed) {
            del_video_display(id, true);
        }
    }

    // Send events for any newly added displays
    for &(id, state) in &states {
        if state == Some(DisplayState::Added) {
            send_display_event(id, EventType::DISPLAY_ADDED, 0, 0);
        }
    }
}

/// Translation of `WIN_UpdateDisplayUsableBounds()`.
pub(crate) fn update_display_usable_bounds() {
    // This almost never happens, so just go ahead and send update events for all displays
    let ids = crate::video::display::displays().unwrap_or_default();
    for id in ids {
        send_display_event(id, EventType::DISPLAY_USABLE_BOUNDS_CHANGED, 0, 0);
    }
}

/// Translation of `WIN_QuitModes()`.
pub(crate) fn quit_modes(_videodata: &VideoData) {
    // All fullscreen windows should have restored modes by now
}

/// The masks of a `BI_BITFIELDS` bitmap info, as a format (for the
/// framebuffer's `SDL_GetPixelFormatForMasks()`).
pub(crate) fn format_for_masks(bpp: u32, masks: [u32; 3]) -> PixelFormat {
    PixelFormat::from_masks(PixelMasks {
        bpp,
        r: masks[0],
        g: masks[1],
        b: masks[2],
        a: 0,
    })
    .unwrap_or(PixelFormat::UNKNOWN)
}
