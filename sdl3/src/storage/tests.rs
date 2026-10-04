use super::*;
use crate::filesystem::PathType;
use crate::test_support::TempDir;
use crate::ErrorKind;
use std::ops::ControlFlow;

#[test]
fn path_validation() {
    for ok in ["", "a", "a/b", "a.b/c", ".hidden", "a/.x", "...", "a/.../b"] {
        assert!(validate_storage_path(ok).is_ok(), "{ok}");
    }
    for bad in [".", "..", "./a", "../a", "a/./b", "a/../b", "a/.", "a/.."] {
        assert_eq!(
            validate_storage_path(bad).unwrap_err().message(),
            "Relative paths not permitted",
            "{bad}"
        );
    }
    assert_eq!(
        validate_storage_path("a\\b").unwrap_err().message(),
        "Windows-style path separators ('\\') not permitted, use '/' instead."
    );
}

#[test]
fn file_storage() {
    let tmp = TempDir::new("storage");
    let storage = Storage::open_file(Some(&tmp.0)).unwrap();
    assert!(storage.ready());
    assert_eq!(storage.space_remaining().unwrap(), u64::MAX);

    storage.create_directory("saves/slot1").unwrap();
    storage
        .write_file("saves/slot1/game.dat", b"level 3")
        .unwrap();
    assert_eq!(storage.file_size("saves/slot1/game.dat").unwrap(), 7);
    assert_eq!(
        storage.load_file("saves/slot1/game.dat").unwrap(),
        b"level 3"
    );
    // Upstream only checks that the whole buffer was filled: a shorter
    // buffer reads a prefix, a longer one fails.
    let mut short = [0u8; 3];
    storage
        .read_file("saves/slot1/game.dat", &mut short)
        .unwrap();
    assert_eq!(&short, b"lev");
    let mut long = [0u8; 8];
    assert_eq!(
        storage
            .read_file("saves/slot1/game.dat", &mut long)
            .unwrap_err()
            .message(),
        "File length did not exactly match the destination length"
    );
    assert!(storage.read_file("../escape", &mut short).is_err());
    assert_eq!(
        storage.path_info("saves").unwrap().kind,
        PathType::Directory
    );

    storage
        .copy_file("saves/slot1/game.dat", "saves/backup.dat")
        .unwrap();
    storage
        .rename_path("saves/backup.dat", "saves/old.dat")
        .unwrap();

    // Enumeration reports directories relative to the storage root.
    let mut seen = Vec::new();
    storage
        .enumerate_directory(None, |dir, name| {
            seen.push(format!("{dir}{name}"));
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    assert_eq!(seen, ["saves"]);
    let mut seen = Vec::new();
    storage
        .enumerate_directory(Some("saves"), |dir, name| {
            seen.push(format!("{dir}{name}"));
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
    seen.sort();
    assert_eq!(seen, ["saves/old.dat", "saves/slot1"]);

    let mut all = storage
        .glob_directory(None, Some("*/*.dat"), GlobFlags::NONE)
        .unwrap();
    all.sort();
    assert_eq!(all, ["saves/old.dat"]);
    let mut all = storage
        .glob_directory(
            Some("saves"),
            Some("*/GAME.DAT"),
            GlobFlags::CASE_INSENSITIVE,
        )
        .unwrap();
    all.sort();
    assert_eq!(all, ["slot1/game.dat"]);

    storage.remove_path("saves/old.dat").unwrap();
    assert!(storage.path_info("saves/old.dat").is_err());
    storage.close().unwrap();

    // Relative base paths are made absolute from the current directory.
    let rel = Storage::open_file(Some("relative-dir")).unwrap();
    assert!(rel
        .path_info("x")
        .unwrap_err()
        .message()
        .starts_with("Can't stat"));
}

#[test]
fn title_storage_is_read_only() {
    let tmp = TempDir::new("title");
    std::fs::write(tmp.path("data.txt"), b"title data").unwrap();
    let title = Storage::open_title(Some(&tmp.0), None).unwrap();
    assert_eq!(title.load_file("data.txt").unwrap(), b"title data");
    for e in [
        title.write_file("x", b"").unwrap_err(),
        title.create_directory("d").unwrap_err(),
        title.remove_path("data.txt").unwrap_err(),
        title.rename_path("data.txt", "y").unwrap_err(),
        title.copy_file("data.txt", "y").unwrap_err(),
        title.space_remaining().unwrap_err(),
    ] {
        assert_eq!(e.kind(), ErrorKind::Unsupported);
    }
    assert!(
        Storage::open_title(None, None).is_ok(),
        "the base path exists"
    );
}

#[cfg(unix)]
#[test]
fn user_storage_and_driver_hints() {
    use crate::stdlib::{getenv, setenv_unsafe, unsetenv_unsafe};
    let _l = crate::test_support::test_lock();
    let tmp = TempDir::new("user");
    let saved = getenv("XDG_DATA_HOME");
    setenv_unsafe("XDG_DATA_HOME", &tmp.0, true).unwrap();

    let user = Storage::open_user(Some("Org"), "Game", None).unwrap();
    user.write_file("save.txt", b"1").unwrap();
    assert_eq!(std::fs::read(tmp.path("Org/Game/save.txt")).unwrap(), b"1");
    drop(user);

    hints::set(hints::STORAGE_USER_DRIVER, "nope,GENERIC").unwrap();
    assert!(Storage::open_user(None, "Game", None).is_ok());
    hints::set(hints::STORAGE_USER_DRIVER, "nope").unwrap();
    assert_eq!(
        Storage::open_user(None, "Game", None)
            .unwrap_err()
            .message(),
        "nope not available"
    );
    hints::reset(hints::STORAGE_USER_DRIVER);
    hints::set(hints::STORAGE_TITLE_DRIVER, "steam").unwrap();
    assert_eq!(
        Storage::open_title(None, None).unwrap_err().message(),
        "steam not available"
    );
    hints::reset(hints::STORAGE_TITLE_DRIVER);

    match saved {
        Some(v) => setenv_unsafe("XDG_DATA_HOME", &v, true).unwrap(),
        None => unsetenv_unsafe("XDG_DATA_HOME").unwrap(),
    }
}

#[test]
fn custom_interface() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    struct Readonly(Arc<AtomicBool>);
    impl StorageInterface for Readonly {
        fn close(&mut self) -> Result<()> {
            self.0.store(true, Ordering::SeqCst);
            Err(Error::new("flush failed"))
        }
        fn ready(&self) -> bool {
            false
        }
        fn info(&self, path: &str) -> Result<PathInfo> {
            Ok(PathInfo {
                kind: PathType::File,
                size: path.len() as u64,
                ..PathInfo::default()
            })
        }
    }

    let closed = Arc::new(AtomicBool::new(false));
    let s = Storage::new(Readonly(closed.clone()));
    assert!(!s.ready());
    assert_eq!(s.file_size("abcd").unwrap(), 4);
    assert_eq!(
        s.read_file("abcd", &mut [0; 4]).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(
        s.enumerate_directory(None, |_, _| Ok(ControlFlow::Continue(())))
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(s.close().unwrap_err().message(), "flush failed");
    assert!(closed.load(Ordering::SeqCst));

    // Dropping closes too.
    let closed = Arc::new(AtomicBool::new(false));
    drop(Storage::new(Readonly(closed.clone())));
    assert!(closed.load(Ordering::SeqCst));
}
