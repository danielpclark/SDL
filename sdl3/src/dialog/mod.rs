// Rust translation of src/dialog/SDL_dialog.c, SDL_dialog.h,
// SDL_dialog_utils.c, SDL_dialog_utils.h, src/dialog/unix/SDL_unixdialog.c,
// SDL_zenitydialog.c (and the version check of SDL_zenitymessagebox.c),
// src/dialog/dummy/SDL_dummydialog.c and include/SDL3/SDL_dialog.h from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! File dialogs: asynchronous "open file", "save file" and "open folder"
//! dialogs, whose result is passed to a callback.
//!
//! On Unix systems other than Apple's, Android and Haiku, the dialogs are
//! shown with `zenity` (the XDG desktop portal needs D-Bus, which arrives
//! with the platform layer); elsewhere the dummy backend reports that the
//! operation is unsupported.

use crate::error::{Error, Result};
use crate::events::WindowID;

/// An entry for filters for file dialogs. Translation of `SDL_DialogFileFilter`.
///
/// `name` is a user-readable label for the filter (for example, "Office
/// document"); `pattern` is a semicolon-separated list of file extensions
/// (for example, "doc;docx"). File extensions may only contain alphanumeric
/// characters, hyphens, underscores and periods. Alternatively, the whole
/// string can be a single asterisk ("*"), which serves as an "All files"
/// filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogFileFilter {
    pub name: String,
    pub pattern: String,
}

impl DialogFileFilter {
    /// A filter named `name` matching the extensions in `pattern`.
    pub fn new(name: impl Into<String>, pattern: impl Into<String>) -> DialogFileFilter {
        DialogFileFilter {
            name: name.into(),
            pattern: pattern.into(),
        }
    }
}

/// Various types of file dialogs. Translation of `SDL_FileDialogType`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FileDialogType {
    OpenFile,
    SaveFile,
    OpenFolder,
}

/// The options of a file dialog. Translation of the
/// `SDL_PROP_FILE_DIALOG_*` properties of `SDL_ShowFileDialogWithProperties()`.
#[derive(Clone, Debug, Default)]
pub struct FileDialogOptions {
    /// The filters (`SDL_PROP_FILE_DIALOG_FILTERS_POINTER`); ignored for
    /// folder dialogs.
    pub filters: Vec<DialogFileFilter>,
    /// The window the dialog should be modal for (`SDL_PROP_FILE_DIALOG_WINDOW_POINTER`).
    pub window: Option<WindowID>,
    /// The default folder or file to start the dialog at
    /// (`SDL_PROP_FILE_DIALOG_LOCATION_STRING`).
    pub location: Option<String>,
    /// Whether the user can select more than one entry
    /// (`SDL_PROP_FILE_DIALOG_MANY_BOOLEAN`).
    pub many: bool,
    /// The title for the dialog (`SDL_PROP_FILE_DIALOG_TITLE_STRING`).
    pub title: Option<String>,
    /// The label for the accept button (`SDL_PROP_FILE_DIALOG_ACCEPT_STRING`).
    pub accept: Option<String>,
    /// The label for the cancel button (`SDL_PROP_FILE_DIALOG_CANCEL_STRING`).
    pub cancel: Option<String>,
}

/// The callback of a file dialog: the selected paths (empty if the user
/// canceled) and the index of the selected filter (`None` if unknown or
/// unsupported), or the error that prevented showing the dialog.
/// Translation of `SDL_DialogFileCallback`; it may run on another thread.
pub type DialogFileCallback = Box<dyn FnOnce(Result<Vec<String>>, Option<usize>) + Send>;

/// Create and launch a file dialog with the specified options. This is an
/// asynchronous function; it returns immediately, and the result is
/// passed to the callback. Translation of `SDL_ShowFileDialogWithProperties()`.
pub fn show_file_dialog(
    dialog_type: FileDialogType,
    options: FileDialogOptions,
    callback: impl FnOnce(Result<Vec<String>>, Option<usize>) + Send + 'static,
) {
    let callback: DialogFileCallback = Box::new(callback);

    if let Some(msg) = validate_filters(&options.filters) {
        callback(
            Err(Error::new(format!("Invalid dialog file filters: {msg}"))),
            None,
        );
        return;
    }

    sys::show_file_dialog(dialog_type, callback, options);
}

/// Display a dialog that lets the user select a file on their filesystem.
/// Translation of `SDL_ShowOpenFileDialog()`.
pub fn show_open_file_dialog(
    callback: impl FnOnce(Result<Vec<String>>, Option<usize>) + Send + 'static,
    window: Option<WindowID>,
    filters: &[DialogFileFilter],
    default_location: Option<&str>,
    allow_many: bool,
) {
    let options = FileDialogOptions {
        filters: filters.to_vec(),
        window,
        location: default_location.map(str::to_string),
        many: allow_many,
        ..FileDialogOptions::default()
    };
    show_file_dialog(FileDialogType::OpenFile, options, callback);
}

/// Display a dialog that lets the user choose a new or existing file on
/// their filesystem. Translation of `SDL_ShowSaveFileDialog()`.
pub fn show_save_file_dialog(
    callback: impl FnOnce(Result<Vec<String>>, Option<usize>) + Send + 'static,
    window: Option<WindowID>,
    filters: &[DialogFileFilter],
    default_location: Option<&str>,
) {
    let options = FileDialogOptions {
        filters: filters.to_vec(),
        window,
        location: default_location.map(str::to_string),
        ..FileDialogOptions::default()
    };
    show_file_dialog(FileDialogType::SaveFile, options, callback);
}

/// Display a dialog that lets the user select a folder on their filesystem.
/// Translation of `SDL_ShowOpenFolderDialog()`.
pub fn show_open_folder_dialog(
    callback: impl FnOnce(Result<Vec<String>>, Option<usize>) + Send + 'static,
    window: Option<WindowID>,
    default_location: Option<&str>,
    allow_many: bool,
) {
    let options = FileDialogOptions {
        window,
        location: default_location.map(str::to_string),
        many: allow_many,
        ..FileDialogOptions::default()
    };
    show_file_dialog(FileDialogType::OpenFolder, options, callback);
}

// The following are utility functions to help implementations. They are
// ordered by scope largeness, decreasing. All implementations should use
// them, as they check for invalid filters. Where they are unused, the
// validate_* function further down below should be used.

/// Transform the name given in argument into something viable for the
/// engine. Useful if there are special characters to avoid on certain
/// platforms (such as "|" with Zenity). Translation of `NameTransform`.
pub(crate) type NameTransform = fn(&str) -> String;

/// Convert all the filters into a single string:
/// `<prefix>[filter]{<separator>[filter]...}<suffix>`.
/// Translation of `convert_filters()`.
#[allow(dead_code, clippy::too_many_arguments)] // (used by the platform backends)
pub(crate) fn convert_filters(
    filters: &[DialogFileFilter],
    ntf: Option<NameTransform>,
    prefix: &str,
    separator: &str,
    suffix: &str,
    filt_prefix: &str,
    filt_separator: &str,
    filt_suffix: &str,
    ext_prefix: &str,
    ext_separator: &str,
    ext_suffix: &str,
    anycase: bool,
) -> Result<String> {
    let mut combined = prefix.to_string();

    for (i, f) in filters.iter().enumerate() {
        let converted = convert_filter(
            f,
            ntf,
            filt_prefix,
            filt_separator,
            filt_suffix,
            ext_prefix,
            ext_separator,
            ext_suffix,
            anycase,
        )?;

        let terminator = if i + 1 < filters.len() {
            separator
        } else {
            suffix
        };
        combined.push_str(&converted);
        combined.push_str(terminator);
    }

    // FIXME (upstream): the suffix is appended after the last filter's terminator too
    combined.push_str(suffix);

    Ok(combined)
}

/// Convert one filter into a single string:
/// `<prefix>[filter name]<separator>[filter extension list]<suffix>`.
/// Translation of `convert_filter()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn convert_filter(
    filter: &DialogFileFilter,
    ntf: Option<NameTransform>,
    prefix: &str,
    separator: &str,
    suffix: &str,
    ext_prefix: &str,
    ext_separator: &str,
    ext_suffix: &str,
    anycase: bool,
) -> Result<String> {
    let list = convert_ext_list(
        &filter.pattern,
        ext_prefix,
        ext_separator,
        ext_suffix,
        anycase,
    )?;

    let name_filtered = match ntf {
        Some(ntf) => ntf(&filter.name),
        // Useless strdup, but easier to read and maintain code this way
        None => filter.name.clone(),
    };

    Ok(format!("{prefix}{name_filtered}{separator}{list}{suffix}"))
}

fn is_pattern_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b'.'
}

/// Convert the extension list of a filter into a single string:
/// `<prefix>[extension]{<separator>[extension]...}<suffix>`.
/// Translation of `convert_ext_list()`.
pub(crate) fn convert_ext_list(
    list: &str,
    prefix: &str,
    separator: &str,
    suffix: &str,
    anycase: bool,
) -> Result<String> {
    let mut converted = prefix.to_string();

    /* Some platforms may prefer to handle the asterisk manually, but this
    function offers to handle it for ease of use. */
    let bytes = list.as_bytes();
    if list == "*" {
        converted.push('*');
    } else {
        for (i, &c) in bytes.iter().enumerate() {
            if anycase && c.is_ascii_alphabetic() {
                converted.push('[');
                converted.push(c as char);
                converted.push((c ^ 0x20) as char); // ASCII case toggle
                converted.push(']');
            } else if is_pattern_char(c) {
                converted.push(c as char);
            } else if c == b';' {
                if i == 0 || bytes[i - 1] == b';' {
                    return Err(Error::new("Empty pattern not allowed"));
                }

                converted.push_str(separator);
            } else {
                // (the first byte of a non-ASCII character)
                let ch = list[i..].chars().next().unwrap_or(c as char);
                return Err(Error::new(format!(
                    "Invalid character '{ch}' in pattern (Only [a-zA-Z0-9_.-] allowed, or a single *)"
                )));
            }
        }
    }

    // FIXME (upstream): an empty list reads the byte before the string
    if list.ends_with(';') {
        return Err(Error::new("Empty pattern not allowed"));
    }

    converted.push_str(suffix);

    Ok(converted)
}

/// Check the filters; an error message if there's a problem.
/// Translation of `validate_filters()`.
pub(crate) fn validate_filters(filters: &[DialogFileFilter]) -> Option<&'static str> {
    filters.iter().find_map(|f| validate_list(&f.pattern))
}

/// Check an extension list; an error message if there's a problem.
/// Translation of `validate_list()`.
pub(crate) fn validate_list(list: &str) -> Option<&'static str> {
    if list == "*" {
        return None;
    }
    let bytes = list.as_bytes();
    for (i, &c) in bytes.iter().enumerate() {
        if is_pattern_char(c) {
            continue;
        } else if c == b';' {
            if i == 0 || bytes[i - 1] == b';' {
                return Some("Empty pattern not allowed");
            }
        } else {
            return Some(
                "Invalid character in pattern (Only [a-zA-Z0-9_.-] allowed, or a single *)",
            );
        }
    }

    // FIXME (upstream): an empty list reads the byte before the string
    if list.ends_with(';') {
        return Some("Empty pattern not allowed");
    }

    None
}

#[cfg(all(
    unix,
    not(any(target_vendor = "apple", target_os = "android", target_os = "haiku"))
))]
mod sys {
    //! Translation of `SDL_unixdialog.c` and `SDL_zenitydialog.c`.

    use std::sync::Mutex;

    use super::{convert_filter, DialogFileCallback, FileDialogOptions, FileDialogType};
    use crate::error::{Error, Result};
    use crate::hints;
    use crate::process::{Process, ProcessBuilder, ProcessIo};
    use crate::stdlib::Environment;

    type ShowFn = fn(FileDialogType, DialogFileCallback, FileDialogOptions);

    /// Translation of `detected_function`.
    static DETECTED_FUNCTION: Mutex<Option<ShowFn>> = Mutex::new(None);
    static HINT_WATCH: Mutex<Option<hints::Callback>> = Mutex::new(None);

    /// Translation of `set_callback()`.
    fn set_callback() {
        let mut watch = HINT_WATCH.lock().unwrap_or_else(|e| e.into_inner());
        if watch.is_none() {
            // (translation of `hint_callback()`; the first call happens now,
            // with the current value)
            *watch = hints::watch(hints::FILE_DIALOG_DRIVER, |change| {
                let _ = detect_available_methods(change.new_value);
            })
            .ok();
        }
    }

    /// Translation of `detect_available_methods()`.
    fn detect_available_methods(value: Option<&str>) -> Result<()> {
        let driver = match value {
            Some(v) => Some(v.to_string()),
            None => hints::get(hints::FILE_DIALOG_DRIVER),
        };

        if HINT_WATCH.try_lock().is_ok() {
            set_callback();
        }

        // ("portal": the XDG desktop portal needs D-Bus, which arrives with
        // the platform layer, and isn't detected.)

        if (driver.is_none() || driver.as_deref() == Some("zenity")) && zenity_detect() {
            *DETECTED_FUNCTION.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(zenity_show_file_dialog);
            return Ok(());
        }

        Err(Error::new(
            "File dialog driver unsupported (supported values for SDL_HINT_FILE_DIALOG_DRIVER are 'zenity' and 'portal')",
        ))
    }

    /// Translation of `SDL_SYS_ShowFileDialogWithProperties()` (Unix).
    pub(super) fn show_file_dialog(
        dialog_type: FileDialogType,
        callback: DialogFileCallback,
        options: FileDialogOptions,
    ) {
        // Call detect_available_methods() again each time in case the situation changed
        let detected = *DETECTED_FUNCTION.lock().unwrap_or_else(|e| e.into_inner());
        let function = match detected {
            Some(function) => function,
            None => match detect_available_methods(None) {
                Ok(()) => match *DETECTED_FUNCTION.lock().unwrap_or_else(|e| e.into_inner()) {
                    Some(function) => function,
                    None => {
                        return callback(Err(Error::new("File dialog driver unsupported")), None)
                    }
                },
                Err(e) => return callback(Err(e), None),
            },
        };

        function(dialog_type, callback, options);
    }

    /// Translation of `zenity_clean_name()`.
    fn zenity_clean_name(name: &str) -> String {
        /* Filter out "|", which Zenity considers a special character. Let's hope
        there aren't others. TODO: find something better. */
        // Zenity doesn't support escaping with '\'
        name.replace('|', "/")
    }

    /// Translation of `parse_zenity_version()`.
    pub(super) fn parse_zenity_version(version: &str) -> Result<(i32, i32)> {
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
    fn zenity_version() -> Result<(i32, i32)> {
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
    fn zenity_show_file_dialog(
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
    fn zenity_detect() -> bool {
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
}

#[cfg(not(all(
    unix,
    not(any(target_vendor = "apple", target_os = "android", target_os = "haiku"))
)))]
mod sys {
    use super::{DialogFileCallback, FileDialogOptions, FileDialogType};

    /// Translation of `SDL_SYS_ShowFileDialogWithProperties()` (dummy).
    pub(super) fn show_file_dialog(
        _dialog_type: FileDialogType,
        callback: DialogFileCallback,
        _options: FileDialogOptions,
    ) {
        callback(Err(crate::error::Error::unsupported()), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_conversion() {
        assert_eq!(validate_list("*"), None);
        assert_eq!(validate_list("png;jpg"), None);
        assert_eq!(validate_list("png;;jpg"), Some("Empty pattern not allowed"));
        assert_eq!(validate_list(";png"), Some("Empty pattern not allowed"));
        assert_eq!(validate_list("png;"), Some("Empty pattern not allowed"));
        assert_eq!(
            validate_list("p ng"),
            Some("Invalid character in pattern (Only [a-zA-Z0-9_.-] allowed, or a single *)")
        );

        let f = DialogFileFilter::new("Images | pics", "png;JPG");
        assert_eq!(
            convert_filter(&f, None, "--file-filter=", " | ", "", "*.", " *.", "", true).unwrap(),
            "--file-filter=Images | pics | *.[pP][nN][gG] *.[Jj][Pp][Gg]"
        );
        assert_eq!(convert_ext_list("*", "(", ",", ")", false).unwrap(), "(*)");
        assert_eq!(
            convert_ext_list("a/b", "", "", "", false)
                .unwrap_err()
                .to_string(),
            "Invalid character '/' in pattern (Only [a-zA-Z0-9_.-] allowed, or a single *)"
        );
        let filters = [
            DialogFileFilter::new("A", "a"),
            DialogFileFilter::new("B", "b;c"),
        ];
        assert_eq!(
            convert_filters(&filters, None, "[", "|", "]", "<", ":", ">", "", ",", "", false)
                .unwrap(),
            "[<A:a>|<B:b,c>]]"
        );
    }

    #[test]
    fn invalid_filters_reach_the_callback() {
        let (tx, rx) = std::sync::mpsc::channel();
        show_open_file_dialog(
            move |result, filter| {
                tx.send((result.map_err(|e| e.to_string()), filter))
                    .unwrap()
            },
            None,
            &[DialogFileFilter::new("Bad", "a;;b")],
            None,
            false,
        );
        assert_eq!(
            rx.recv().unwrap(),
            (
                Err("Invalid dialog file filters: Empty pattern not allowed".to_string()),
                None
            )
        );
    }

    #[cfg(all(
        unix,
        not(any(target_vendor = "apple", target_os = "android", target_os = "haiku"))
    ))]
    #[test]
    fn zenity_arguments() {
        assert_eq!(sys::parse_zenity_version("3.44.0\n").unwrap(), (3, 44));
        assert_eq!(sys::parse_zenity_version("4\n").unwrap(), (4, 0));
        assert_eq!(
            sys::parse_zenity_version("x").unwrap_err().to_string(),
            "failed to get zenity major version number"
        );
        assert_eq!(sys::split_output(b"/a\n/b\n"), ["/a", "/b"]);
        assert_eq!(sys::split_output(b"/a"), ["/a"]);
        // A canceled dialog prints nothing: no files were chosen
        assert_eq!(sys::split_output(b""), Vec::<String>::new());
        assert_eq!(sys::split_output(b"\n"), Vec::<String>::new());

        let options = FileDialogOptions {
            filters: vec![DialogFileFilter::new("Text|Docs", "txt")],
            location: Some("/tmp".into()),
            many: true,
            title: Some("Pick".into()),
            ..FileDialogOptions::default()
        };
        assert_eq!(
            sys::create_zenity_args(FileDialogType::SaveFile, &options).unwrap(),
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
