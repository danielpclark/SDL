// Rust translation of src/video/x11/SDL_x11messagebox.c and
// SDL_x11messagebox.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! X11 message boxes: zenity if it is there, otherwise a dialog drawn with
//! the toolkit of [`super::toolkit`].
//!
//! As upstream's default (`SDL_FORK_MESSAGEBOX` 0), the dialog runs in this
//! process.

use std::cell::Cell;
use std::ffi::c_int;
use std::rc::Rc;

use super::toolkit::{
    ToolkitWindow, ToolkitWindowMode, SDL_TOOLKIT_X11_ELEMENT_PADDING_2,
    SDL_TOOLKIT_X11_ELEMENT_PADDING_3, SDL_TOOLKIT_X11_ELEMENT_PADDING_4,
};
use crate::error::Result;
use crate::video::messagebox::{MessageBoxData, MessageBoxFlags};

/// Translation of `SDL_MessageBoxX11`: the controls of a message box, by
/// their index in the toolkit window (the window itself, the data and the
/// button ID are passed along instead).
#[derive(Clone)]
struct MessageBoxX11 {
    icon: Option<usize>,
    message: Option<usize>,
    buttons: Vec<usize>,
    /// `messageboxdata->flags`
    flags: MessageBoxFlags,
}

/// Translation of `X11_PositionMessageBox()`: place the controls, and the
/// window's size.
fn x11_position_message_box(
    controls: &MessageBoxX11,
    window: &mut ToolkitWindow,
) -> (c_int, c_int) {
    let iscale = window.iscale;
    let numbuttons = controls.buttons.len();

    /* window size */
    // (window_width and window_height start at 1 upstream, and are set
    // below before being read)

    /* rtl */
    let rtl = if controls
        .flags
        .contains(MessageBoxFlags::BUTTONS_RIGHT_TO_LEFT)
    {
        true
    } else if controls
        .flags
        .contains(MessageBoxFlags::BUTTONS_LEFT_TO_RIGHT)
    {
        false
    } else {
        window.flip_interface
    };

    /* first line */
    let mut first_line_width = 0;
    let mut first_line_height = 0;
    match (controls.icon, controls.message) {
        (Some(icon), Some(message)) => {
            window.controls[icon].rect.y = 0;

            first_line_width = window.controls[icon].rect.w
                + SDL_TOOLKIT_X11_ELEMENT_PADDING_2 * iscale
                + window.controls[message].rect.w;

            if !window.flip_interface {
                window.controls[message].rect.x =
                    window.controls[icon].rect.w + SDL_TOOLKIT_X11_ELEMENT_PADDING_2 * iscale;
                window.controls[icon].rect.x = 0;
            } else {
                window.controls[message].rect.x = 0;
                window.controls[icon].rect.x =
                    window.controls[message].rect.w + SDL_TOOLKIT_X11_ELEMENT_PADDING_2 * iscale;
            }

            if window.controls[message].rect.h > window.controls[icon].rect.h {
                window.controls[message].rect.y = (window.controls[icon].rect.h
                    - window.get_label_control_first_line_height(message))
                    / 2;
                first_line_height =
                    window.controls[message].rect.y + window.controls[message].rect.h;
            } else {
                window.controls[message].rect.y =
                    (window.controls[icon].rect.h - window.controls[message].rect.h) / 2;
                first_line_height = window.controls[icon].rect.h;
            }
        }
        (Option::None, Some(message)) => {
            first_line_width = window.controls[message].rect.w;
            first_line_height = window.controls[message].rect.h;
            window.controls[message].rect.x = 0;
            window.controls[message].rect.y = 0;
        }
        (Some(icon), Option::None) => {
            first_line_width = window.controls[icon].rect.w;
            first_line_height = window.controls[icon].rect.h;
            window.controls[icon].rect.x = 0;
            window.controls[icon].rect.y = 0;
        }
        (Option::None, Option::None) => {}
    }

    /* second line */
    let mut max_button_width = 50;
    let mut max_button_height = 0;
    let mut second_line_width = 0;
    let mut second_line_height = 0;

    for &b in &controls.buttons {
        max_button_width = max_button_width.max(window.controls[b].rect.w);
        max_button_height = max_button_height.max(window.controls[b].rect.h);
        window.controls[b].rect.x = 0;
        window.controls[b].rect.y = 0;
    }

    let mut place_button = |window: &mut ToolkitWindow, i: usize, previous: Option<usize>| {
        let b = controls.buttons[i];
        window.controls[b].rect.w = max_button_width;
        window.controls[b].rect.h = max_button_height;
        window.notify_control_of_size_change(b);

        if first_line_height != 0 {
            window.controls[b].rect.y =
                first_line_height + SDL_TOOLKIT_X11_ELEMENT_PADDING_4 * iscale;
            second_line_height = max_button_height + SDL_TOOLKIT_X11_ELEMENT_PADDING_4 * iscale;
        } else {
            second_line_height = max_button_height;
        }

        if let Some(p) = previous {
            let p = controls.buttons[p];
            window.controls[b].rect.x = window.controls[p].rect.x
                + window.controls[p].rect.w
                + (SDL_TOOLKIT_X11_ELEMENT_PADDING_3 * iscale);
        }
    };
    if rtl {
        for i in (0..numbuttons).rev() {
            let previous = (i + 1 < numbuttons).then_some(i + 1);
            place_button(window, i, previous);
        }
    } else {
        for i in 0..numbuttons {
            let previous = i.checked_sub(1);
            place_button(window, i, previous);
        }
    }

    if numbuttons != 0 {
        let last = if rtl {
            controls.buttons[0]
        } else {
            controls.buttons[numbuttons - 1]
        };
        second_line_width = window.controls[last].rect.x + window.controls[last].rect.w;
    }

    /* center lines */
    if second_line_width > first_line_width {
        let pad = (second_line_width - first_line_width) / 2;
        if let Some(message) = controls.message {
            window.controls[message].rect.x += pad;
        }
        if let Some(icon) = controls.icon {
            window.controls[icon].rect.x += pad;
        }
    } else {
        let pad = (first_line_width - second_line_width) / 2;
        for &b in &controls.buttons {
            window.controls[b].rect.x += pad;
        }
    }

    /* window size and final padding */
    let window_width =
        first_line_width.max(second_line_width) + SDL_TOOLKIT_X11_ELEMENT_PADDING_2 * 2 * iscale;
    let window_height =
        first_line_height + second_line_height + SDL_TOOLKIT_X11_ELEMENT_PADDING_2 * 2 * iscale;
    let padding = SDL_TOOLKIT_X11_ELEMENT_PADDING_2 * iscale;
    for i in controls
        .message
        .iter()
        .chain(&controls.icon)
        .chain(&controls.buttons)
    {
        window.controls[*i].rect.x += padding;
        window.controls[*i].rect.y += padding;
    }
    (window_width, window_height)
}

/// Translation of `X11_ShowMessageBoxImpl()`: the toolkit dialog, setting
/// `button_id` to the ID of the button chosen.
pub(crate) fn x11_show_message_box_impl(
    messageboxdata: &MessageBoxData,
    button_id: &mut i32,
) -> Result<()> {
    /* Color scheme */
    let colorhints = messageboxdata
        .color_scheme
        .as_ref()
        .map(|scheme| &scheme.colors);

    /* Create window */
    let mut parent_window = Option::None;
    if let Some(window) = &messageboxdata.window {
        if crate::video::core::current_video_driver().ok() == Some("x11") {
            // Only use the window as a parent if it is from the X11 driver.
            parent_window = Some(window.id());
        }
    }
    let mut window = ToolkitWindow::create_window_struct(
        parent_window,
        Option::None,
        ToolkitWindowMode::Dialog,
        colorhints,
        false,
    )?;

    /* Create controls */
    let chosen = Rc::new(Cell::new(Option::None));
    let mut controls = MessageBoxX11 {
        icon: window.create_icon_control(messageboxdata.flags),
        message: window.create_label_control(&messageboxdata.message),
        buttons: Vec::with_capacity(messageboxdata.buttons.len()),
        flags: messageboxdata.flags,
    };
    for button in &messageboxdata.buttons {
        let b = window.create_button_control(button);
        let chosen = chosen.clone();
        // (translation of `X11_MessageBoxButtonCallback()`)
        window.register_callback_for_button_control(
            b,
            Box::new(move |window: &mut ToolkitWindow, control: usize| {
                if let Some(data) = window.get_button_control_data(control) {
                    chosen.set(Some(data.button_id));
                }
                window.signal_window_close();
            }),
        );
        controls.buttons.push(b);
    }

    // (translation of `X11_OnMessageBoxScaleChange()`)
    let scale_controls = controls.clone();
    window.cb_on_scale_change = Some(Box::new(move |window: &mut ToolkitWindow| {
        let (w, h) = x11_position_message_box(&scale_controls, window);
        window.resize_window(w, h);
    }));

    /* Positioning */
    let (w, h) = x11_position_message_box(&controls, &mut window);

    /* Actually create window, do event loop, cleanup */
    // FIXME (upstream): the result isn't checked: when the window can't be
    // created, the event loop waits for its events forever.
    let _ = window.create_window_res(w, h, 0, 0, &messageboxdata.title);
    window.do_window_event_loop();
    window.destroy_window();
    if let Some(id) = chosen.get() {
        *button_id = id;
    }
    Ok(())
}

/// Display an x11 message box. Translation of `X11_ShowMessageBox()` (the
/// bootstrap's `ShowMessageBox`): the ID of the button chosen, or -1 if
/// none was (C leaves the caller's `buttonID` alone then).
pub(crate) fn x11_show_message_box(messageboxdata: &MessageBoxData) -> Result<i32> {
    let mut button_id = -1;

    #[cfg(not(any(target_os = "android", target_os = "haiku")))]
    if crate::dialog::zenity_show_message_box(messageboxdata, &mut button_id).is_ok() {
        return Ok(button_id);
    }

    x11_show_message_box_impl(messageboxdata, &mut button_id)?;
    Ok(button_id)
}
