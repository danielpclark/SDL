// Rust translation of src/video/windows/SDL_windowsmessagebox.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Message boxes on Windows: a Task Dialog (comctl32.dll, loaded at run
//! time), or a dialog built from an in-memory template where that's not
//! available.

use std::sync::atomic::{AtomicI32, Ordering};

use windows_sys::Win32::Foundation::{FreeLibrary, HWND, LPARAM, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateFontIndirectW, DeleteDC, DrawTextW, GetDC, GetDeviceCaps,
    GetTextExtentPoint32A, GetTextMetricsW, ReleaseDC, SelectObject, DT_CALCRECT, DT_EDITCONTROL,
    DT_LEFT, DT_NOPREFIX, LOGPIXELSX, LOGPIXELSY, TEXTMETRICW,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::WindowsProgramming::MulDiv;
use windows_sys::Win32::UI::Controls::{TASKDIALOGCONFIG, TASKDIALOG_BUTTON};
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    DPI_AWARENESS_CONTEXT_UNAWARE,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DialogBoxIndirectParamW, EndDialog, GetDlgItem, GetSystemMetrics, PostMessageW,
    SystemParametersInfoA, SystemParametersInfoW, DLGTEMPLATE, GWLP_USERDATA, IDI_ERROR,
    IDI_INFORMATION, IDI_WARNING, NONCLIENTMETRICSA, NONCLIENTMETRICSW, SM_CXICON, SM_CYICON,
    SPI_GETNONCLIENTMETRICS, WM_COMMAND, WM_INITDIALOG, WM_NEXTDLGCTL, WM_SETFOCUS, WS_CAPTION,
    WS_CHILD, WS_GROUP, WS_TABSTOP, WS_VISIBLE,
};

use super::window::window_data;
use super::{get_window_long_ptr, set_window_long_ptr, USER_DEFAULT_SCREEN_DPI};
use crate::core::windows::utf8_to_wide;
use crate::error::{Error, Result};
use crate::video::messagebox::{MessageBoxButtonFlags, MessageBoxData, MessageBoxFlags};

const SS_EDITCONTROL: u32 = 0x2000;
const SS_LEFT: u32 = 0x0000;
const SS_NOPREFIX: u32 = 0x0080;
const SS_ICON: u32 = 0x0003;
const BS_PUSHBUTTON: u32 = 0x0000;
const BS_DEFPUSHBUTTON: u32 = 0x0001;
const DS_CENTER: u32 = 0x0800;
/// `DS_SETFONT | DS_FIXEDSYS`
const DS_SHELLFONT: u32 = 0x0040 | 0x0008;

const IDOK: usize = 1;
const IDCANCEL: usize = 2;

// Custom dialog return codes
const IDCLOSED: isize = 20;
const IDINVALPTRINIT: isize = 50;
const IDINVALPTRCOMMAND: isize = 51;
const IDINVALPTRSETFOCUS: isize = 52;
const IDINVALPTRDLGITEM: isize = 53;
// First button ID
const IDBUTTONINDEX0: isize = 100;

const DLGITEMTYPEBUTTON: u16 = 0x0080;
const DLGITEMTYPESTATIC: u16 = 0x0082;

/* Windows only sends the lower 16 bits of the control ID when a button
 * gets clicked. There are also some predefined and custom IDs that lower
 * the available number further. 2^16 - 101 buttons should be enough for
 * everyone, no need to make the code more complex.
 */
const MAX_BUTTONS: usize = 0xffff - 100;

// Display a Windows message box

const TDF_SIZE_TO_CONTENT: i32 = 0x01000000; // used by ShellMessageBox to emulate MessageBox sizing behavior

/// `MAKEINTRESOURCEW(-1)`
const TD_WARNING_ICON: *const u16 = (-1isize) as u16 as usize as *const u16;
/// `MAKEINTRESOURCEW(-2)`
const TD_ERROR_ICON: *const u16 = (-2isize) as u16 as usize as *const u16;
/// `MAKEINTRESOURCEW(-3)`
const TD_INFORMATION_ICON: *const u16 = (-3isize) as u16 as usize as *const u16;

/// `sizeof(DLGTEMPLATEEX)` (packed)
const DLGTEMPLATEEX_SIZE: usize = 26;
/// The offset of `cDlgItems` in `DLGTEMPLATEEX`
const DLGTEMPLATEEX_CDLGITEMS: usize = 16;
/// `sizeof(DLGITEMTEMPLATEEX)` (packed)
const DLGITEMTEMPLATEEX_SIZE: usize = 24;

/// Translation of `WIN_DialogData` (`lpDialog` is the start of `data`; the
/// `Vec` grows as upstream's buffer does).
struct DialogData {
    data: Vec<u8>,
    /// `size`, the allocated size
    size: usize,
    numbuttons: u16,
}

/// Translation of `GetButtonIndex()`.
fn get_button_index(
    messageboxdata: &MessageBoxData,
    flags: MessageBoxButtonFlags,
) -> Option<usize> {
    messageboxdata
        .buttons
        .iter()
        .position(|b| (b.flags.0 & flags.0) != 0)
}

/// Translation of `MessageBoxDialogProc()`.
unsafe extern "system" fn message_box_dialog_proc(
    h_dlg: HWND,
    i_message: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> isize {
    // SAFETY: GWLP_USERDATA holds the MessageBoxData pointer passed to
    // DialogBoxIndirectParam(), which outlives the dialog.
    let userdata =
        || unsafe { (get_window_long_ptr(h_dlg, GWLP_USERDATA) as *const MessageBoxData).as_ref() };
    // SAFETY: the dialog APIs are called on the dialog's own window.
    unsafe {
        match i_message {
            WM_INITDIALOG => {
                if l_param == 0 {
                    EndDialog(h_dlg, IDINVALPTRINIT);
                    return 1;
                }
                let messageboxdata = &*(l_param as *const MessageBoxData);
                set_window_long_ptr(h_dlg, GWLP_USERDATA, l_param);

                if let Some(buttonindex) =
                    get_button_index(messageboxdata, MessageBoxButtonFlags::RETURNKEY_DEFAULT)
                {
                    // Focus on the first default return-key button
                    let buttonctl =
                        GetDlgItem(h_dlg, (IDBUTTONINDEX0 + buttonindex as isize) as i32);
                    if buttonctl.is_null() {
                        EndDialog(h_dlg, IDINVALPTRDLGITEM);
                    }
                    PostMessageW(h_dlg, WM_NEXTDLGCTL, buttonctl as WPARAM, 1);
                } else {
                    // Give the focus to the dialog window instead
                    SetFocus(h_dlg);
                }
                0
            }
            WM_SETFOCUS => {
                let Some(messageboxdata) = userdata() else {
                    EndDialog(h_dlg, IDINVALPTRSETFOCUS);
                    return 1;
                };

                // Let the default button be focused if there is one. Otherwise, prevent any initial focus.
                if get_button_index(messageboxdata, MessageBoxButtonFlags::RETURNKEY_DEFAULT)
                    .is_some()
                {
                    return 0;
                }
                1
            }
            WM_COMMAND => {
                let Some(messageboxdata) = userdata() else {
                    EndDialog(h_dlg, IDINVALPTRCOMMAND);
                    return 1;
                };

                // Return the ID of the button that was pushed
                if w_param == IDOK {
                    if let Some(buttonindex) =
                        get_button_index(messageboxdata, MessageBoxButtonFlags::RETURNKEY_DEFAULT)
                    {
                        EndDialog(h_dlg, IDBUTTONINDEX0 + buttonindex as isize);
                    }
                } else if w_param == IDCANCEL {
                    if let Some(buttonindex) =
                        get_button_index(messageboxdata, MessageBoxButtonFlags::ESCAPEKEY_DEFAULT)
                    {
                        EndDialog(h_dlg, IDBUTTONINDEX0 + buttonindex as isize);
                    } else {
                        // Closing of window was requested by user or system. It would be rude not to comply.
                        EndDialog(h_dlg, IDCLOSED);
                    }
                } else if w_param >= IDBUTTONINDEX0 as usize
                    && (w_param as isize - IDBUTTONINDEX0) < messageboxdata.buttons.len() as isize
                {
                    EndDialog(h_dlg, w_param as isize);
                }
                1
            }
            _ => 0,
        }
    }
}

/// Translation of `ExpandDialogSpace()`.
fn expand_dialog_space(dialog: &mut DialogData, space: usize) -> Result<()> {
    // Growing memory in 64 KiB steps.
    const SIZESTEP: usize = 0x10000;
    let used = dialog.data.len();
    let mut size = dialog.size;

    if size == 0 {
        // Start with 4 KiB or a multiple of 64 KiB to fit the data.
        size = 0x1000;
        if usize::MAX - SIZESTEP < space {
            size = space;
        } else if space > size {
            size = (space + SIZESTEP) & !(SIZESTEP - 1);
        }
    } else if usize::MAX - used < space {
        return Err(Error::out_of_memory());
    } else if usize::MAX - (used + space) < SIZESTEP {
        // Close to the maximum.
        size = used + space;
    } else if size < used + space {
        // Round up to the next 64 KiB block.
        size = used + space;
        size += SIZESTEP - size % SIZESTEP;
    }

    if size > dialog.size {
        dialog.data.reserve_exact(size - used);
        dialog.size = size;
    }
    Ok(())
}

/// Translation of `AlignDialogData()`.
fn align_dialog_data(dialog: &mut DialogData, size: usize) -> Result<()> {
    // FIXME (upstream): the padding is `used % size`, not the bytes up to the next multiple of `size` (right only for 0 or size/2).
    let padding = dialog.data.len() % size;

    expand_dialog_space(dialog, padding)?;

    let new_len = dialog.data.len() + padding;
    dialog.data.resize(new_len, 0);

    Ok(())
}

/// Translation of `AddDialogData()`.
fn add_dialog_data(dialog: &mut DialogData, data: &[u8]) -> Result<()> {
    expand_dialog_space(dialog, data.len())?;

    dialog.data.extend_from_slice(data);

    Ok(())
}

/// Translation of `AddDialogString()`.
fn add_dialog_string(dialog: &mut DialogData, string: Option<&str>) -> Result<()> {
    let string = string.unwrap_or("");

    // (the characters, including the null terminator)
    let wstring = utf8_to_wide(string);
    let count = wstring
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(wstring.len())
        + 1;

    let bytes: Vec<u8> = wstring[..count]
        .iter()
        .flat_map(|c| c.to_le_bytes())
        .collect();
    add_dialog_data(dialog, &bytes)
}

static BASE_UNITS_X: AtomicI32 = AtomicI32::new(0);
static BASE_UNITS_Y: AtomicI32 = AtomicI32::new(0);

/// Translation of `Vec2ToDLU()`.
fn vec2_to_dlu(x: &mut i16, y: &mut i16) {
    let base_x = BASE_UNITS_X.load(Ordering::Relaxed);
    let base_y = BASE_UNITS_Y.load(Ordering::Relaxed);
    crate::sdl_assert!(base_x != 0); // we init in WIN_ShowMessageBox(), which is the only public function...

    // SAFETY: MulDiv is a pure function.
    unsafe {
        *x = MulDiv(*x as i32, 4, base_x) as i16;
        *y = MulDiv(*y as i32, 8, base_y) as i16;
    }
}

/// Translation of `AddDialogControl()`.
#[allow(clippy::too_many_arguments)]
fn add_dialog_control(
    dialog: &mut DialogData,
    ty: u16,
    style: u32,
    ex_style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: i32,
    caption: Option<&str>,
    ordinal: u16,
) -> Result<()> {
    let marker: u16 = 0xFFFF;
    let extra_data: u16 = 0;

    let (mut ix, mut iy, mut icx, mut icy) = (x as i16, y as i16, w as i16, h as i16);
    vec2_to_dlu(&mut ix, &mut iy);
    vec2_to_dlu(&mut icx, &mut icy);

    // DLGITEMTEMPLATEEX: helpID, exStyle, style, x, y, cx, cy, id
    let mut item = Vec::with_capacity(DLGITEMTEMPLATEEX_SIZE);
    item.extend_from_slice(&0u32.to_le_bytes());
    item.extend_from_slice(&ex_style.to_le_bytes());
    item.extend_from_slice(&style.to_le_bytes());
    item.extend_from_slice(&ix.to_le_bytes());
    item.extend_from_slice(&iy.to_le_bytes());
    item.extend_from_slice(&icx.to_le_bytes());
    item.extend_from_slice(&icy.to_le_bytes());
    item.extend_from_slice(&(id as u32).to_le_bytes());

    align_dialog_data(dialog, 4)?;
    add_dialog_data(dialog, &item)?;
    add_dialog_data(dialog, &marker.to_le_bytes())?;
    add_dialog_data(dialog, &ty.to_le_bytes())?;
    if ty == DLGITEMTYPEBUTTON || (ty == DLGITEMTYPESTATIC && caption.is_some()) {
        add_dialog_string(dialog, caption)?;
    } else {
        add_dialog_data(dialog, &marker.to_le_bytes())?;
        add_dialog_data(dialog, &ordinal.to_le_bytes())?;
    }
    add_dialog_data(dialog, &extra_data.to_le_bytes())?;
    if ty == DLGITEMTYPEBUTTON {
        dialog.numbuttons += 1;
    }
    let at = DLGTEMPLATEEX_CDLGITEMS;
    let count = u16::from_le_bytes([dialog.data[at], dialog.data[at + 1]]) + 1;
    dialog.data[at..at + 2].copy_from_slice(&count.to_le_bytes());

    Ok(())
}

/// Translation of `AddDialogStaticText()`.
fn add_dialog_static_text(
    dialog: &mut DialogData,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    text: &str,
) -> Result<()> {
    let style = WS_VISIBLE | WS_CHILD | SS_LEFT | SS_NOPREFIX | SS_EDITCONTROL | WS_GROUP;
    add_dialog_control(
        dialog,
        DLGITEMTYPESTATIC,
        style,
        0,
        x,
        y,
        w,
        h,
        -1,
        Some(text),
        0,
    )
}

/// Translation of `AddDialogStaticIcon()`.
fn add_dialog_static_icon(
    dialog: &mut DialogData,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    ordinal: u16,
) -> Result<()> {
    let style = WS_VISIBLE | WS_CHILD | SS_ICON | WS_GROUP;
    add_dialog_control(
        dialog,
        DLGITEMTYPESTATIC,
        style,
        0,
        x,
        y,
        w,
        h,
        -2,
        None,
        ordinal,
    )
}

/// Translation of `AddDialogButton()`.
#[allow(clippy::too_many_arguments)]
fn add_dialog_button(
    dialog: &mut DialogData,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    text: &str,
    id: i32,
    is_default: bool,
) -> Result<()> {
    let mut style = WS_VISIBLE | WS_CHILD | WS_TABSTOP;
    if is_default {
        style |= BS_DEFPUSHBUTTON;
    } else {
        style |= BS_PUSHBUTTON;
    }
    // The first button marks the start of the group.
    if dialog.numbuttons == 0 {
        style |= WS_GROUP;
    }
    add_dialog_control(
        dialog,
        DLGITEMTYPEBUTTON,
        style,
        0,
        x,
        y,
        w,
        h,
        id,
        Some(text),
        0,
    )
}

/// Translation of `CreateDialogData()`.
fn create_dialog_data(w: i32, h: i32, caption: &str) -> Result<DialogData> {
    let (mut cx, mut cy) = (w as i16, h as i16);
    vec2_to_dlu(&mut cx, &mut cy);

    // DLGTEMPLATEEX: dlgVer, signature, helpID, exStyle, style, cDlgItems, x, y, cx, cy
    let mut dialog_template = Vec::with_capacity(DLGTEMPLATEEX_SIZE);
    dialog_template.extend_from_slice(&1u16.to_le_bytes());
    dialog_template.extend_from_slice(&0xffffu16.to_le_bytes());
    dialog_template.extend_from_slice(&0u32.to_le_bytes());
    dialog_template.extend_from_slice(&0u32.to_le_bytes());
    dialog_template.extend_from_slice(&(WS_CAPTION | DS_CENTER | DS_SHELLFONT).to_le_bytes());
    dialog_template.extend_from_slice(&0u16.to_le_bytes());
    dialog_template.extend_from_slice(&0i16.to_le_bytes());
    dialog_template.extend_from_slice(&0i16.to_le_bytes());
    dialog_template.extend_from_slice(&cx.to_le_bytes());
    dialog_template.extend_from_slice(&cy.to_le_bytes());

    let mut dialog = DialogData {
        data: Vec::new(),
        size: 0,
        numbuttons: 0,
    };

    add_dialog_data(&mut dialog, &dialog_template)?;

    // No menu
    let mut word_to_pass: u16 = 0;
    add_dialog_data(&mut dialog, &word_to_pass.to_le_bytes())?;

    // No custom class
    add_dialog_data(&mut dialog, &word_to_pass.to_le_bytes())?;

    // title
    add_dialog_string(&mut dialog, Some(caption))?;

    // Font stuff
    {
        /*
         * We want to use the system messagebox font.
         */
        // SAFETY: NONCLIENTMETRICSA is plain data; SystemParametersInfoA fills it.
        let mut ncm: NONCLIENTMETRICSA = unsafe { std::mem::zeroed() };
        ncm.cbSize = size_of::<NONCLIENTMETRICSA>() as u32;
        // SAFETY: as above.
        unsafe {
            SystemParametersInfoA(
                SPI_GETNONCLIENTMETRICS,
                0,
                (&mut ncm as *mut NONCLIENTMETRICSA).cast(),
                0,
            )
        };

        // Font size - convert to logical font size for dialog parameter.
        {
            // SAFETY: the screen DC is released after use.
            unsafe {
                let screen_dc = GetDC(std::ptr::null_mut());
                let mut logical_pixels_y = GetDeviceCaps(screen_dc, LOGPIXELSY as i32);
                if logical_pixels_y == 0 {
                    logical_pixels_y = 72; // This can happen if the application runs out of GDI handles
                }

                word_to_pass = (-72 * ncm.lfMessageFont.lfHeight / logical_pixels_y) as u16;
                ReleaseDC(std::ptr::null_mut(), screen_dc);
            }
        }

        add_dialog_data(&mut dialog, &word_to_pass.to_le_bytes())?;

        // Font weight
        word_to_pass = ncm.lfMessageFont.lfWeight as u16;
        add_dialog_data(&mut dialog, &word_to_pass.to_le_bytes())?;

        // italic?
        add_dialog_data(&mut dialog, &[ncm.lfMessageFont.lfItalic])?;

        // charset?
        add_dialog_data(&mut dialog, &[ncm.lfMessageFont.lfCharSet])?;

        // font typeface.
        // FIXME (upstream): lfFaceName is in the ANSI code page, but is converted as UTF-8.
        let face: Vec<u8> = ncm
            .lfMessageFont
            .lfFaceName
            .iter()
            .map(|&c| c as u8)
            .take_while(|&c| c != 0)
            .collect();
        add_dialog_string(&mut dialog, Some(&String::from_utf8_lossy(&face)))?;
    }

    Ok(dialog)
}

/* Escaping ampersands is necessary to disable mnemonics in dialog controls.
 */
/// Translation of `EscapeAmpersands()` (the work buffer is the returned
/// string).
fn escape_ampersands(src: &str) -> String {
    // The escape character is the ampersand itself.
    src.replace('&', "&&")
}

/// Translation of `WIN_GetContentScale()`.
fn get_content_scale() -> f32 {
    let mut dpi = 0;

    // (upstream's per-monitor query is disabled: we don't know what monitor the dialog will be shown on)
    if dpi == 0 {
        // Window 8.0 and below: same DPI for all monitors
        // SAFETY: the screen DC is released after use.
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

/// The `HWND` of the message box's parent window (or null).
fn parent_hwnd(messageboxdata: &MessageBoxData) -> HWND {
    messageboxdata
        .window
        .as_ref()
        .and_then(|w| window_data(w.id()))
        .map_or(std::ptr::null_mut(), |d| d.hwnd)
}

// This function is called if a Task Dialog is unsupported.
/// Translation of `WIN_ShowOldMessageBox()`.
fn show_old_message_box(messageboxdata: &MessageBoxData) -> Result<i32> {
    let roundf = crate::stdlib::math::roundf;
    let mut defbuttoncount: u16 = 0;
    let mut icon: u16 = 0;

    let scale = get_content_scale();
    let button_width = roundf(88.0 * scale) as i32;
    let button_height = roundf(26.0 * scale) as i32;
    let text_margin = roundf(16.0 * scale) as i32;
    let button_margin = roundf(12.0 * scale) as i32;
    // SAFETY: GetSystemMetrics has no preconditions.
    let (icon_width, icon_height) =
        unsafe { (GetSystemMetrics(SM_CXICON), GetSystemMetrics(SM_CYICON)) };
    let icon_margin = roundf(20.0 * scale) as i32;

    let numbuttons = messageboxdata.buttons.len();
    if numbuttons > MAX_BUTTONS {
        return Err(Error::new(format!(
            "Number of buttons exceeds limit of {MAX_BUTTONS}"
        )));
    }

    let kind = messageboxdata.flags.0
        & (MessageBoxFlags::ERROR.0 | MessageBoxFlags::WARNING.0 | MessageBoxFlags::INFORMATION.0);
    if kind == MessageBoxFlags::ERROR.0 {
        icon = IDI_ERROR as usize as u16;
    } else if kind == MessageBoxFlags::WARNING.0 {
        icon = IDI_WARNING as usize as u16;
    } else if kind == MessageBoxFlags::INFORMATION.0 {
        icon = IDI_INFORMATION as usize as u16;
    }

    /* Jan 25th, 2013 - dant@fleetsa.com
     *
     * I've tried to make this more reasonable, but I've run in to a lot
     * of nonsense.
     *
     * The original issue is the code was written in pixels and not
     * dialog units (DLUs). All DialogBox functions use DLUs, which
     * vary based on the selected font (yay).
     *
     * According to MSDN, the most reliable way to convert is via
     * MapDialogUnits, which requires an HWND, which we don't have
     * at time of template creation.
     *
     * We do however have:
     *  The system font (DLU width 8 for me)
     *  The font we select for the dialog (DLU width 6 for me)
     *
     * Based on experimentation, *neither* of these return the value
     * actually used. Stepping in to MapDialogUnits(), the conversion
     * is fairly clear, and uses 7 for me.
     *
     * As a result, some of this is hacky to ensure the sizing is
     * somewhat correct.
     *
     * Honestly, a long term solution is to use CreateWindow, not CreateDialog.
     *
     * In order to get text dimensions we need to have a DC with the desired font.
     * I'm assuming a dialog box in SDL is rare enough we can to the create.
     */
    let mut text_size = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: the DC and font are created, selected and used in order.
    unsafe {
        let font_dc = CreateCompatibleDC(std::ptr::null_mut());

        let dialog_font = {
            // Create a duplicate of the font used in system message boxes.
            let mut ncm: NONCLIENTMETRICSW = std::mem::zeroed();
            ncm.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
            SystemParametersInfoW(
                SPI_GETNONCLIENTMETRICS,
                0,
                (&mut ncm as *mut NONCLIENTMETRICSW).cast(),
                0,
            );
            let lf = ncm.lfMessageFont;
            CreateFontIndirectW(&lf)
        };
        // FIXME (upstream): DialogFont is never deleted.

        // Select the font in to our DC
        SelectObject(font_dc, dialog_font);

        {
            // Get the metrics to try and figure our DLU conversion.
            let mut tm: TEXTMETRICW = std::mem::zeroed();
            GetTextMetricsW(font_dc, &mut tm);

            /* Calculation from the following documentation:
             * https://support.microsoft.com/en-gb/help/125681/how-to-calculate-dialog-base-units-with-non-system-based-font
             * This fixes bug 2137, dialog box calculation with a fixed-width system font
             */
            {
                let mut extent = SIZE { cx: 0, cy: 0 };
                GetTextExtentPoint32A(
                    font_dc,
                    c"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"
                        .as_ptr()
                        .cast(),
                    52,
                    &mut extent,
                );
                BASE_UNITS_X.store((extent.cx / 26 + 1) / 2, Ordering::Relaxed);
            }
            // s_BaseUnitsX = TM.tmAveCharWidth + 1;
            BASE_UNITS_Y.store(tm.tmHeight, Ordering::Relaxed);
        }

        /* Measure the *pixel* size of the string. */
        let wmessage = utf8_to_wide(&messageboxdata.message);
        DrawTextW(
            font_dc,
            wmessage.as_ptr(),
            -1,
            &mut text_size,
            DT_CALCRECT | DT_LEFT | DT_NOPREFIX | DT_EDITCONTROL,
        );

        // Done with the DC, and the string
        DeleteDC(font_dc);
    }

    // Add margins and some padding for hangs, etc.
    text_size.left += text_margin;
    text_size.right += text_margin + 2;
    text_size.top += text_margin;
    text_size.bottom += text_margin + 2;

    // Increase the size of the dialog by some border spacing around the text.
    let mut size = SIZE {
        cx: text_size.right - text_size.left,
        cy: text_size.bottom - text_size.top,
    };
    size.cx += text_margin * 2;
    size.cy += text_margin * 2;

    // Make dialog wider and shift text over for the icon.
    if icon != 0 {
        size.cx += icon_margin + icon_width;
        text_size.left += icon_margin + icon_width;
        text_size.right += icon_margin + icon_width;
    }

    // Ensure the size is wide enough for all of the buttons.
    let n = numbuttons as i32;
    if size.cx < n * (button_width + button_margin) + button_margin {
        size.cx = n * (button_width + button_margin) + button_margin;
    }

    // Reset the height to the icon size if it is actually bigger than the text.
    if icon != 0 && size.cy < icon_margin * 2 + icon_height {
        size.cy = icon_margin * 2 + icon_height;
    }

    // Add vertical space for the buttons and border.
    size.cy += button_height + text_margin;

    let mut dialog = create_dialog_data(size.cx, size.cy, &messageboxdata.title)?;

    if icon != 0 {
        add_dialog_static_icon(
            &mut dialog,
            icon_margin,
            icon_margin,
            icon_width,
            icon_height,
            icon,
        )?;
    }

    add_dialog_static_text(
        &mut dialog,
        text_size.left,
        text_size.top,
        text_size.right - text_size.left,
        text_size.bottom - text_size.top,
        &messageboxdata.message,
    )?;

    // Align the buttons to the right/bottom.
    let mut x = size.cx - (button_width + button_margin) * n;
    let y = size.cy - button_height - button_margin;
    for i in 0..numbuttons {
        let mut isdefault = false;

        /* We always have to create the dialog buttons from left to right
         * so that the tab order is correct.  Select the info to use
         * depending on which order was requested. */
        let index = if messageboxdata
            .flags
            .contains(MessageBoxFlags::BUTTONS_LEFT_TO_RIGHT)
        {
            i
        } else {
            numbuttons - 1 - i
        };
        let sdl_button = &messageboxdata.buttons[index];

        if sdl_button
            .flags
            .contains(MessageBoxButtonFlags::RETURNKEY_DEFAULT)
        {
            defbuttoncount += 1;
            if defbuttoncount == 1 {
                isdefault = true;
            }
        }

        let buttontext = escape_ampersands(&sdl_button.text);
        /* Make sure to provide the correct ID to keep buttons indexed in the
         * same order as how they are in messageboxdata. */
        add_dialog_button(
            &mut dialog,
            x,
            y,
            button_width,
            button_height,
            &buttontext,
            IDBUTTONINDEX0 as i32 + index as i32,
            isdefault,
        )?;

        x += button_width + button_margin;
    }

    /* If we have a parent window, get the Instance and HWND for them
     * so that our little dialog gets exclusive focus at all times. */
    let parent_window = parent_hwnd(messageboxdata);

    // (the template is copied to u32s: it must be DWORD-aligned)
    let template: Vec<u32> = dialog
        .data
        .chunks(4)
        .map(|c| {
            let mut b = [0u8; 4];
            b[..c.len()].copy_from_slice(c);
            u32::from_le_bytes(b)
        })
        .collect();
    // SAFETY: the template is a complete DLGTEMPLATEEX; the dialog procedure
    // reads messageboxdata, which outlives the modal dialog.
    let rc = unsafe {
        DialogBoxIndirectParamW(
            std::ptr::null_mut(),
            template.as_ptr().cast::<DLGTEMPLATE>(),
            parent_window,
            Some(message_box_dialog_proc),
            messageboxdata as *const MessageBoxData as LPARAM,
        )
    };
    if rc >= IDBUTTONINDEX0 && rc - IDBUTTONINDEX0 < numbuttons as isize {
        Ok(messageboxdata.buttons[(rc - IDBUTTONINDEX0) as usize].button_id)
    } else if rc == IDCLOSED {
        // Dialog window closed by user or system.
        // This could use a special return code.
        Ok(-1)
    } else if rc == 0 {
        Err(Error::new("Invalid parent window handle"))
    } else if rc == -1 {
        Err(Error::new("The message box encountered an error."))
    } else if rc == IDINVALPTRINIT || rc == IDINVALPTRSETFOCUS || rc == IDINVALPTRCOMMAND {
        Err(Error::new(
            "Invalid message box pointer in dialog procedure",
        ))
    } else if rc == IDINVALPTRDLGITEM {
        Err(Error::new(
            "Couldn't find dialog control of the default enter-key button",
        ))
    } else {
        Err(Error::new("An unknown error occurred"))
    }
}

/* TaskDialogIndirect procedure
 * This is because SDL targets Windows XP (0x501), so this is not defined in the platform SDK.
 */
type TaskDialogIndirectProc =
    unsafe extern "system" fn(*const TASKDIALOGCONFIG, *mut i32, *mut i32, *mut i32) -> i32;
type SetThreadDpiAwarenessContextFn =
    unsafe extern "system" fn(DPI_AWARENESS_CONTEXT) -> DPI_AWARENESS_CONTEXT;

/// Show a message box; the ID of the button pressed (-1 if closed).
/// Translation of `WIN_ShowMessageBox()`.
pub(crate) fn show_message_box(messageboxdata: &MessageBoxData) -> Result<i32> {
    let user32 = utf8_to_wide("user32.dll");
    // SAFETY: the name is NUL-terminated; the function has this signature.
    let set_thread_dpi_awareness_context: Option<SetThreadDpiAwarenessContextFn> = unsafe {
        let h_user32 = GetModuleHandleW(user32.as_ptr());
        GetProcAddress(h_user32, c"SetThreadDpiAwarenessContext".as_ptr().cast())
            .map(|f| std::mem::transmute::<_, SetThreadDpiAwarenessContextFn>(f))
    };
    let mut previous_context = DPI_AWARENESS_CONTEXT_UNAWARE;
    if let Some(f) = set_thread_dpi_awareness_context {
        // SAFETY: a valid awareness context.
        previous_context = unsafe { f(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    }

    let result = show_task_dialog(messageboxdata);

    if let Some(f) = set_thread_dpi_awareness_context {
        // SAFETY: the context SetThreadDpiAwarenessContext() returned.
        unsafe { f(previous_context) };
    }
    result
}

/// The part of `WIN_ShowMessageBox()` between setting and restoring the
/// thread's DPI awareness.
fn show_task_dialog(messageboxdata: &MessageBoxData) -> Result<i32> {
    // If we cannot load comctl32.dll use the old messagebox!
    let name = utf8_to_wide("comctl32.dll");
    // SAFETY: the name is NUL-terminated.
    let h_comctl32 = unsafe { LoadLibraryW(name.as_ptr()) };
    if h_comctl32.is_null() {
        return show_old_message_box(messageboxdata);
    }

    /* If TaskDialogIndirect doesn't exist use the old messagebox!
      This will fail prior to Windows Vista.
      The manifest file in the application may require targeting version 6 of comctl32.dll, even
      when we use LoadLibrary here!
      If you don't want to bother with manifests, put this #pragma in your app's source code somewhere:
      #pragma comment(linker,"\"/manifestdependency:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0'  processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'\"")
    */
    // SAFETY: the module is loaded; the function has this signature.
    let task_dialog_indirect: Option<TaskDialogIndirectProc> = unsafe {
        GetProcAddress(h_comctl32, c"TaskDialogIndirect".as_ptr().cast())
            .map(|f| std::mem::transmute::<_, TaskDialogIndirectProc>(f))
    };
    let Some(task_dialog_indirect) = task_dialog_indirect else {
        // SAFETY: loaded above.
        unsafe { FreeLibrary(h_comctl32) };
        return show_old_message_box(messageboxdata);
    };

    /* If we have a parent window, get the Instance and HWND for them
    so that our little dialog gets exclusive focus at all times. */
    let parent_window = parent_hwnd(messageboxdata);

    let wmessage = utf8_to_wide(&messageboxdata.message);
    let wtitle = utf8_to_wide(&messageboxdata.title);

    let mut task_config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: parent_window,
        dwFlags: TDF_SIZE_TO_CONTENT,
        pszWindowTitle: wtitle.as_ptr(),
        ..Default::default()
    };
    let flags = messageboxdata.flags;
    task_config.Anonymous1.pszMainIcon = if flags.contains(MessageBoxFlags::ERROR) {
        TD_ERROR_ICON
    } else if flags.contains(MessageBoxFlags::WARNING) {
        TD_WARNING_ICON
    } else if flags.contains(MessageBoxFlags::INFORMATION) {
        TD_INFORMATION_ICON
    } else {
        std::ptr::null()
    };

    let numbuttons = messageboxdata.buttons.len();
    task_config.pszContent = wmessage.as_ptr();
    task_config.cButtons = numbuttons as u32;
    let mut buttons = vec![
        TASKDIALOG_BUTTON {
            nButtonID: 0,
            pszButtonText: std::ptr::null(),
        };
        numbuttons
    ];
    let mut button_texts: Vec<Vec<u16>> = Vec::with_capacity(numbuttons);
    task_config.nDefaultButton = 0;
    let mut n_cancel_button = 0;
    for (i, button) in messageboxdata.buttons.iter().enumerate() {
        let slot = if flags.contains(MessageBoxFlags::BUTTONS_LEFT_TO_RIGHT) {
            i
        } else {
            numbuttons - 1 - i
        };
        let p_button = &mut buttons[slot];
        if button
            .flags
            .contains(MessageBoxButtonFlags::ESCAPEKEY_DEFAULT)
        {
            n_cancel_button = button.button_id;
            p_button.nButtonID = IDCANCEL as i32;
        } else {
            p_button.nButtonID = IDBUTTONINDEX0 as i32 + i as i32;
        }
        let buttontext = escape_ampersands(&button.text);
        let wtext = utf8_to_wide(&buttontext);
        p_button.pszButtonText = wtext.as_ptr();
        button_texts.push(wtext);
        if button
            .flags
            .contains(MessageBoxButtonFlags::RETURNKEY_DEFAULT)
        {
            task_config.nDefaultButton = p_button.nButtonID;
        }
    }
    task_config.pButtons = buttons.as_ptr();

    // Show the Task Dialog
    let mut n_button: i32 = 0;
    // SAFETY: the config and everything it points to live until the call returns.
    let hr = unsafe {
        task_dialog_indirect(
            &task_config,
            &mut n_button,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };

    // Free everything
    // SAFETY: loaded above.
    unsafe { FreeLibrary(h_comctl32) };
    drop(button_texts);

    // Check the Task Dialog was successful and give the result
    if hr >= 0 {
        if n_button == IDCANCEL as i32 {
            Ok(n_cancel_button)
        } else if n_button >= IDBUTTONINDEX0 as i32
            && n_button < IDBUTTONINDEX0 as i32 + numbuttons as i32
        {
            Ok(messageboxdata.buttons[(n_button - IDBUTTONINDEX0 as i32) as usize].button_id)
        } else {
            Ok(-1)
        }
    } else {
        // We failed showing the Task Dialog, use the old message box!
        show_old_message_box(messageboxdata)
    }
}
