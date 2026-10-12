// Tests of the macOS file dialogs.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use std::sync::{Arc, Mutex};

use super::*;

/// The patterns split as upstream's loop splits them, `*` anywhere meaning
/// all files; on macOS 11 and later each extension (after its last period)
/// becomes a UTType, unknown ones included as dynamic types.
#[test]
fn filters_to_allowed_types() {
    let filters = [
        DialogFileFilter::new("Images", "png;jpg;tar.gz"),
        DialogFileFilter::new("Text", "txt"),
    ];
    let (exts, all) = filter_extensions(&filters);
    assert_eq!(exts, ["png", "jpg", "tar.gz", "txt"]);
    assert!(!all);
    let (_, all) = filter_extensions(&[DialogFileFilter::new("All", "*")]);
    assert!(all);

    autoreleasepool(|| unsafe {
        let types: Id = msg_send![class!(NSMutableArray), array];
        for ext in &exts {
            add_file_extension_type(types, ext);
        }
        add_file_extension_type(types, "");
        let count: NSUInteger = msg_send![types, count];
        assert_eq!(count, 4);
        if !ut_type_class().is_null() {
            let png: Id = msg_send![types, objectAtIndex: 0 as NSUInteger];
            let id: Id = msg_send![png, identifier];
            assert_eq!(string_from_ns(id).as_deref(), Some("public.png"));
            // "tar.gz" is looked up as "gz"
            let gz: Id = msg_send![types, objectAtIndex: 2 as NSUInteger];
            let ext: Id = msg_send![gz, preferredFilenameExtension];
            assert_eq!(string_from_ns(ext).as_deref(), Some("gz"));
        }
    });
}

#[test]
fn dialog_driver_hint_is_refused() {
    let _l = crate::test_support::test_lock();
    crate::hints::set(crate::hints::FILE_DIALOG_DRIVER, "zenity").unwrap();
    let result = Arc::new(Mutex::new(None));
    let r2 = result.clone();
    show_file_dialog(
        FileDialogType::OpenFile,
        Box::new(move |r, f| {
            assert_eq!(f, None);
            *r2.lock().unwrap() = Some(r.map_err(|e| e.to_string()));
        }),
        FileDialogOptions::default(),
    );
    crate::hints::reset(crate::hints::FILE_DIALOG_DRIVER);
    assert_eq!(
        result.lock().unwrap().take(),
        Some(Err(
            "File dialog driver unsupported (don't set SDL_HINT_FILE_DIALOG_DRIVER)".to_owned()
        ))
    );
}
