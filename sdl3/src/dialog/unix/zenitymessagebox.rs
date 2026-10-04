// Rust translation of src/dialog/unix/SDL_zenitymessagebox.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Message boxes shown with `zenity` (used by the X11 video driver).

use super::zenitydialog::zenity_version;
use crate::error::{Error, Result};
use crate::process::{ProcessBuilder, ProcessIo};
use crate::video::messagebox::{MessageBoxData, MessageBoxFlags};

// (ZENITY_VERSION_LEN, the number of bytes read from zenity --version:
// the whole output is read here.)

/// Maximum number of buttons supported
const MAX_BUTTONS: usize = 8;

/// The arguments of `SDL_Zenity_ShowMessageBox()` for a zenity of
/// version `zenity_major`.`zenity_minor`.
fn create_zenity_messagebox_args(
    messageboxdata: &MessageBoxData,
    zenity_major: i32,
    zenity_minor: i32,
) -> Vec<String> {
    let mut argv: Vec<String> = [
        "zenity",
        "--question",
        "--switch",
        "--no-wrap",
        "--no-markup",
    ]
    .into_iter()
    .map(String::from)
    .collect();

    /* https://gitlab.gnome.org/GNOME/zenity/-/commit/c686bdb1b45e95acf010efd9ca0c75527fbb4dea
     * This commit removed --icon-name without adding a deprecation notice.
     * We need to handle it gracefully, otherwise no message box will be shown.
     */
    argv.push(
        if zenity_major > 3 || (zenity_major == 3 && zenity_minor >= 90) {
            "--icon"
        } else {
            "--icon-name"
        }
        .into(),
    );
    let kind = messageboxdata.flags.0
        & (MessageBoxFlags::ERROR.0 | MessageBoxFlags::WARNING.0 | MessageBoxFlags::INFORMATION.0);
    argv.push(
        if kind == MessageBoxFlags::ERROR.0 {
            "dialog-error"
        } else if kind == MessageBoxFlags::WARNING.0 {
            "dialog-warning"
        } else {
            // (SDL_MESSAGEBOX_INFORMATION and the default)
            "dialog-information"
        }
        .into(),
    );

    if !messageboxdata.title.is_empty() {
        argv.push("--title".into());
        argv.push(messageboxdata.title.clone());
    } else {
        argv.push("--title=".into());
    }

    if !messageboxdata.message.is_empty() {
        argv.push("--text".into());
        argv.push(messageboxdata.message.clone());
    } else {
        argv.push("--text=".into());
    }

    for button in &messageboxdata.buttons {
        if !button.text.is_empty() {
            argv.push("--extra-button".into());
            argv.push(button.text.clone());
        } else {
            argv.push("--extra-button=".into());
        }
    }
    if messageboxdata.buttons.is_empty() {
        argv.push("--extra-button=OK".into());
    }
    argv
}

/// Show a message box with zenity, setting `button_id` to the ID of the
/// button chosen (and leaving it alone if none was). Translation of
/// `SDL_Zenity_ShowMessageBox()` (with a `buttonID`, as
/// `SDL_ShowMessageBox()` always passes one).
pub(crate) fn zenity_show_message_box(
    messageboxdata: &MessageBoxData,
    button_id: &mut i32,
) -> Result<()> {
    if messageboxdata.buttons.len() > MAX_BUTTONS {
        return Err(Error::new(format!(
            "Too many buttons ({MAX_BUTTONS} max allowed)"
        )));
    }

    // get zenity version so we know which arg to use
    let (zenity_major, zenity_minor) = zenity_version()?;

    let argv = create_zenity_messagebox_args(messageboxdata, zenity_major, zenity_minor);

    // If buttonID is set we need to wait and read the results
    let mut process = ProcessBuilder::new(argv).stdout(ProcessIo::App).spawn()?;
    let (output, exit_code) = process.read()?;
    if exit_code < 0 || exit_code == 255 {
        // (C returns false without setting an error)
        return Err(Error::new(format!("zenity failed: exit code {exit_code}")));
    }

    // It likes to add a newline...
    let output = match output.iter().rposition(|&b| b == b'\n') {
        Some(newline) => &output[..newline],
        None => &output[..],
    };

    // Check which button got pressed
    if let Some(button) = messageboxdata
        .buttons
        .iter()
        .find(|b| b.text.as_bytes() == output)
    {
        *button_id = button.button_id;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zenity_messagebox_arguments() {
        use crate::video::messagebox::{MessageBoxButtonData, MessageBoxData, MessageBoxFlags};

        let mut data = MessageBoxData {
            flags: MessageBoxFlags::WARNING,
            title: "Title".into(),
            message: "Text".into(),
            buttons: vec![
                MessageBoxButtonData {
                    button_id: 1,
                    text: "Yes".into(),
                    ..MessageBoxButtonData::default()
                },
                MessageBoxButtonData::default(),
            ],
            ..MessageBoxData::default()
        };
        assert_eq!(
            create_zenity_messagebox_args(&data, 3, 44),
            [
                "zenity",
                "--question",
                "--switch",
                "--no-wrap",
                "--no-markup",
                "--icon-name",
                "dialog-warning",
                "--title",
                "Title",
                "--text",
                "Text",
                "--extra-button",
                "Yes",
                "--extra-button=",
            ]
        );
        data.flags = MessageBoxFlags::ERROR | MessageBoxFlags::WARNING;
        data.title.clear();
        data.message.clear();
        data.buttons.clear();
        assert_eq!(
            create_zenity_messagebox_args(&data, 3, 90)[5..],
            [
                "--icon",
                "dialog-information",
                "--title=",
                "--text=",
                "--extra-button=OK",
            ]
        );
        data.buttons = vec![MessageBoxButtonData::default(); 9];
        let mut button_id = -1;
        assert_eq!(
            zenity_show_message_box(&data, &mut button_id)
                .unwrap_err()
                .to_string(),
            "Too many buttons (8 max allowed)"
        );
    }
}
