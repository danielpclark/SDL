// Rust translation of src/dialog/unix/SDL_zenitydialog.c (and the version
// check of SDL_zenitymessagebox.c) from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! File dialogs shown with `zenity`.

use std::sync::Mutex;

use super::super::{convert_filter, DialogFileCallback, FileDialogOptions, FileDialogType};
use crate::error::{Error, Result};
use crate::process::{Process, ProcessBuilder, ProcessIo};
use crate::stdlib::Environment;

/// Translation of `zenity_clean_name()`.
fn zenity_clean_name(name: &str) -> String {
    /* Filter out "|", which Zenity considers a special character. Let's hope
    there aren't others. TODO: find something better. */
    // Zenity doesn't support escaping with '\'
    name.replace('|', "/")
}

/// Translation of `parse_zenity_version()`.
pub(crate) fn parse_zenity_version(version: &str) -> Result<(i32, i32)> {
    /* We expect the version string is in the form of MAJOR.MINOR.MICRO
     * as described in meson.build. We'll ignore everything after that.
     */
    let (major, used) = crate::stdlib::string::strtol(version, 10);
    if major == 0 && used == 0 {
        return Err(Error::new("failed to get zenity major version number"));
    }

    let rest = &version.as_bytes()[used..];
    let minor = if rest.first() == Some(&b'.') {
        // skip the dot
        let (minor, used) = crate::stdlib::string::strtol(&rest[1..], 10);
        if minor == 0 && used == 0 {
            return Err(Error::new("failed to get zenity minor version number"));
        }
        minor as i32
    } else {
        0
    };
    Ok((major as i32, minor))
}

/// Translation of `SDL_get_zenity_version()`.
pub(super) fn zenity_version() -> Result<(i32, i32)> {
    let mut process = Process::create(&["zenity", "--version"], true)?;
    let (output, _) = process.read()?;
    parse_zenity_version(&String::from_utf8_lossy(&output))
}

/// Exec call format:
///
/// ```text
/// zenity --file-selection --separator=\n [--multiple]
///                     [--directory] [--save --confirm-overwrite]
///                     [--filename FILENAME] [--modal --attach 0x11w1nd0w]
///                     [--title TITLE] [--ok-label ACCEPT]
///                     [--cancel-label CANCEL]
///                     [--file-filter=Filter Name | *.filt *.fn ...]...
/// ```
///
/// Translation of `create_zenity_args()`.
pub(super) fn create_zenity_args(
    dialog_type: FileDialogType,
    options: &FileDialogOptions,
) -> Result<Vec<String>> {
    let (_zenity_major, _zenity_minor) = zenity_version().unwrap_or((0, 0));

    // ARGV PASS
    let mut argv: Vec<String> = vec![
        "zenity".into(),
        "--file-selection".into(),
        "--separator=\n".into(),
    ];

    if options.many {
        argv.push("--multiple".into());
    }

    match dialog_type {
        FileDialogType::OpenFile => {}
        FileDialogType::SaveFile => {
            argv.push("--save".into());
            /* Asking before overwriting while saving seems like a sane default */
            argv.push("--confirm-overwrite".into());
        }
        FileDialogType::OpenFolder => argv.push("--directory".into()),
    }

    if let Some(filename) = &options.location {
        argv.push("--filename".into());
        argv.push(filename.clone());
    }

    // (`--modal --attach` with the window's X11 handle arrives with the
    // video subsystem's X11 backend, for zenity 3.6 and later.)

    if let Some(title) = &options.title {
        argv.push("--title".into());
        argv.push(title.clone());
    }

    if let Some(accept) = &options.accept {
        argv.push("--ok-label".into());
        argv.push(accept.clone());
    }

    if let Some(cancel) = &options.cancel {
        argv.push("--cancel-label".into());
        argv.push(cancel.clone());
    }

    for filter in &options.filters {
        argv.push(convert_filter(
            filter,
            Some(zenity_clean_name),
            "--file-filter=",
            " | ",
            "",
            "*.",
            " *.",
            "",
            true,
        )?);
    }

    Ok(argv)
}

// TODO: Zenity survives termination of the parent

/// Translation of `run_zenity()`.
fn run_zenity(callback: DialogFileCallback, argv: Vec<String>) {
    let run = || -> Result<(Vec<String>, i32)> {
        let env = Environment::new(true);

        /* Recent versions of Zenity have different exit codes, but picks up
        different codes from the environment */
        env.set("ZENITY_OK", "0", true)?;
        env.set("ZENITY_CANCEL", "1", true)?;
        env.set("ZENITY_ESC", "1", true)?;
        env.set("ZENITY_EXTRA", "2", true)?;
        env.set("ZENITY_ERROR", "2", true)?;
        env.set("ZENITY_TIMEOUT", "2", true)?;

        let mut process = ProcessBuilder::new(argv)
            .environment(env)
            .stdin(ProcessIo::Null)
            .stdout(ProcessIo::App)
            .stderr(ProcessIo::Null)
            .spawn()?;

        let (container, status) = process.read()?;
        Ok((split_output(&container), status))
    };

    match run() {
        // 0 = the user chose one or more files, 1 = the user canceled the dialog
        Ok((files, 0 | 1)) => callback(Ok(files), None),
        Ok((_, status)) => callback(
            Err(Error::new(format!(
                "Could not run zenity: exit code {status}"
            ))),
            None,
        ),
        Err(e) => callback(Err(e), None),
    }
}

/// The paths in zenity's output, one per line. Reading from a process
/// often leaves a trailing `\n`, so the last one is ignored.
///
/// Upstream always reports the text before the first `\n` as a path, so
/// an empty output (a canceled dialog) gives one empty path; here it
/// gives none, the documented result for a canceled dialog.
pub(super) fn split_output(container: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(container);
    let text = text.strip_suffix('\n').unwrap_or(&text);
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n').map(str::to_string).collect()
}

/// Translation of `SDL_Zenity_ShowFileDialogWithProperties()`.
pub(super) fn zenity_show_file_dialog(
    dialog_type: FileDialogType,
    callback: DialogFileCallback,
    options: FileDialogOptions,
) {
    let argv = match create_zenity_args(dialog_type, &options) {
        Ok(argv) => argv,
        Err(e) => return callback(Err(e), None),
    };

    let slot = std::sync::Arc::new(Mutex::new(Some(callback)));
    let thread_slot = slot.clone();
    let spawned = std::thread::Builder::new()
        .name("SDL_ZenityFileDialog".into())
        .spawn(move || {
            let callback = thread_slot.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(callback) = callback {
                run_zenity(callback, argv);
            }
        });

    if let Err(e) = spawned {
        let callback = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(callback) = callback {
            callback(
                Err(Error::new(format!("Couldn't create thread: {e}"))),
                None,
            );
        }
    }
}

/// Translation of `SDL_Zenity_detect()`.
pub(super) fn zenity_detect() -> bool {
    let process = ProcessBuilder::new(["zenity", "--version"])
        .stdin(ProcessIo::Null)
        .stdout(ProcessIo::Null)
        .stderr(ProcessIo::Null)
        .spawn();
    match process {
        Ok(mut process) => process.wait(true).ok().flatten() == Some(0),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialog::DialogFileFilter;

    #[test]
    fn zenity_arguments() {
        assert_eq!(parse_zenity_version("3.44.0\n").unwrap(), (3, 44));
        assert_eq!(parse_zenity_version("4\n").unwrap(), (4, 0));
        assert_eq!(
            parse_zenity_version("x").unwrap_err().to_string(),
            "failed to get zenity major version number"
        );
        assert_eq!(split_output(b"/a\n/b\n"), ["/a", "/b"]);
        assert_eq!(split_output(b"/a"), ["/a"]);
        // A canceled dialog prints nothing: no files were chosen
        assert_eq!(split_output(b""), Vec::<String>::new());
        assert_eq!(split_output(b"\n"), Vec::<String>::new());

        let options = FileDialogOptions {
            filters: vec![DialogFileFilter::new("Text|Docs", "txt")],
            location: Some("/tmp".into()),
            many: true,
            title: Some("Pick".into()),
            ..FileDialogOptions::default()
        };
        assert_eq!(
            create_zenity_args(FileDialogType::SaveFile, &options).unwrap(),
            [
                "zenity",
                "--file-selection",
                "--separator=\n",
                "--multiple",
                "--save",
                "--confirm-overwrite",
                "--filename",
                "/tmp",
                "--title",
                "Pick",
                "--file-filter=Text/Docs | *.[tT][xX][tT]",
            ]
        );
    }
}
