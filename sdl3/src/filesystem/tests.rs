use super::*;
use crate::error::Error;
use crate::test_support::TempDir;

/// (pattern, string) pairs; [`WILDCARD_EXPECTED`] is the output of
/// upstream's `WildcardMatch()` over them, compiled from the C source:
/// two digits per case, `matched` then `matched_to_dir`.
const WILDCARD_CASES: &[(&str, &str)] = &[
    ("*", "abc"),
    ("a*c", "abc"),
    ("a*c", "abbbc"),
    ("a*c", "abd"),
    ("a?c", "abc"),
    ("a?c", "a/c"),
    ("*.txt", "x.txt"),
    ("*.txt", "dir/x.txt"),
    ("dir/*.txt", "dir/x.txt"),
    ("dir/*", "dir/sub/x"),
    ("*/x", "dir/x"),
    ("*", "dir/x"),
    ("d*r", "dir"),
    ("*/", "dir"),
    ("dir", "dir"),
    ("dir/", "dir"),
    ("a*b*c", "aXbYc"),
    ("a*b*c", "aXbY/c"),
    ("*a", "aaa"),
    ("**", "abc"),
    ("??", "ab"),
    ("??", "abc"),
    ("?", "/"),
    ("a/b", "a/b"),
    ("a/*/c", "a/b/c"),
    ("a/*/c", "a/b/d/c"),
    ("*x*", "axb"),
    ("*x*", "a/x"),
    ("", "abc"),
    ("abc", ""),
    ("*", ""),
    ("a*", "a"),
    ("*b", "ab/b"),
    ("x*y", "xay/xby"),
    ("*.*", "a.b.c"),
    ("sub*/*", "sub1/f"),
    ("sub*", "sub1"),
    ("ab*cd", "abXcdYcd"),
    ("a*?", "ab"),
    ("a*?", "a"),
];
const WILDCARD_EXPECTED: &str =
    "10101000100010001000100010011001101010101000001010001010000010100000101010101000";

#[test]
fn wildcard_matches_upstream() {
    let mut got = String::new();
    for &(pattern, s) in WILDCARD_CASES {
        let mut d = false;
        let m = wildcard_match(pattern.as_bytes(), s.as_bytes(), &mut d);
        got.push(if m { '1' } else { '0' });
        got.push(if d { '1' } else { '0' });
    }
    assert_eq!(got, WILDCARD_EXPECTED);
}

#[test]
fn case_folding_for_globs() {
    assert_eq!(
        case_fold_utf8_string("ABC/Straße".as_bytes()),
        "abc/strasse".as_bytes()
    );
    assert_eq!(
        case_fold_utf8_string("ΣΊΣΥΦΟΣ".as_bytes()),
        "σίσυφοσ".as_bytes()
    );
}

#[test]
fn file_operations() {
    let tmp = TempDir::new("fsops");

    // Nested directories, with and without a trailing separator.
    create_directory(&tmp.path("a/b/c/")).unwrap();
    create_directory(&tmp.path("a/b/c")).unwrap();
    assert_eq!(
        get_path_info(&tmp.path("a/b/c")).unwrap().kind,
        PathType::Directory
    );

    std::fs::write(tmp.path("a/file.txt"), b"hello").unwrap();
    let info = get_path_info(&tmp.path("a/file.txt")).unwrap();
    assert_eq!(info.kind, PathType::File);
    assert_eq!(info.size, 5);
    assert!(info.modify_time > Time::UNIX_EPOCH);
    assert!(create_directory(&tmp.path("a/file.txt")).is_err());
    assert!(create_directory(&tmp.path("a/file.txt/sub"))
        .unwrap_err()
        .message()
        .starts_with("Can't create directory: "));

    copy_file(&tmp.path("a/file.txt"), &tmp.path("a/b/copy.txt")).unwrap();
    assert_eq!(std::fs::read(tmp.path("a/b/copy.txt")).unwrap(), b"hello");
    assert!(copy_file(&tmp.path("missing"), &tmp.path("x")).is_err());

    rename_path(&tmp.path("a/b/copy.txt"), &tmp.path("a/b/moved.txt")).unwrap();
    assert_eq!(
        get_path_info(&tmp.path("a/b/copy.txt"))
            .unwrap_err()
            .message()
            .split(':')
            .next(),
        Some("Can't stat")
    );
    assert!(rename_path(&tmp.path("nope"), &tmp.path("nope2"))
        .unwrap_err()
        .message()
        .starts_with("Can't rename path: "));

    remove_path(&tmp.path("a/b/moved.txt")).unwrap();
    remove_path(&tmp.path("a/b/moved.txt")).unwrap(); // already gone is success
    remove_path(&tmp.path("a/b/c")).unwrap(); // empty directory
    assert!(remove_path(&tmp.path("a")).is_err(), "not empty");

    let cwd = get_current_directory().unwrap();
    assert!(cwd.ends_with(std::path::MAIN_SEPARATOR));
}

#[test]
fn enumerate_and_glob() {
    let tmp = TempDir::new("glob");
    for f in [
        "x.txt",
        "Y.TXT",
        "z.bin",
        "sub/inner.txt",
        "sub/deeper/far.txt",
        "other/o.txt",
    ] {
        let p = tmp.path(f);
        std::fs::create_dir_all(std::path::Path::new(&p).parent().unwrap()).unwrap();
        std::fs::write(&p, f).unwrap();
    }

    let mut seen = Vec::new();
    enumerate_directory(&format!("{}//", tmp.0), |dir, name| {
        assert_eq!(dir, format!("{}/", tmp.0));
        seen.push(name.to_owned());
        Ok(ControlFlow::Continue(()))
    })
    .unwrap();
    seen.sort();
    assert_eq!(seen, ["Y.TXT", "other", "sub", "x.txt", "z.bin"]);

    // Stopping early succeeds; an error from the callback is returned.
    let mut n = 0;
    enumerate_directory(&tmp.0, |_, _| {
        n += 1;
        Ok(ControlFlow::Break(()))
    })
    .unwrap();
    assert_eq!(n, 1);
    let e = enumerate_directory(&tmp.0, |_, _| Err(Error::new("stop"))).unwrap_err();
    assert_eq!(e.message(), "stop");
    assert!(
        enumerate_directory(&tmp.path("missing"), |_, _| Ok(ControlFlow::Continue(())))
            .unwrap_err()
            .message()
            .starts_with("Can't open directory: ")
    );

    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };
    assert_eq!(
        sorted(glob_directory(&tmp.0, Some("*.txt"), GlobFlags::NONE).unwrap()),
        ["x.txt"]
    );
    assert_eq!(
        sorted(glob_directory(&tmp.0, Some("*.txt"), GlobFlags::CASE_INSENSITIVE).unwrap()),
        ["Y.TXT", "x.txt"]
    );
    assert_eq!(
        sorted(glob_directory(&format!("{}/", tmp.0), Some("sub/*.txt"), GlobFlags::NONE).unwrap()),
        ["sub/inner.txt"]
    );
    assert_eq!(
        sorted(glob_directory(&tmp.0, Some("*/*/*.txt"), GlobFlags::NONE).unwrap()),
        ["sub/deeper/far.txt"]
    );
    assert_eq!(
        sorted(glob_directory(&tmp.0, None, GlobFlags::CASE_INSENSITIVE).unwrap()),
        [
            "Y.TXT",
            "other",
            "other/o.txt",
            "sub",
            "sub/deeper",
            "sub/deeper/far.txt",
            "sub/inner.txt",
            "x.txt",
            "z.bin"
        ]
    );
    assert!(glob_directory(&tmp.path("missing"), None, GlobFlags::NONE).is_err());
}

#[cfg(unix)]
#[test]
fn xdg_paths() {
    use crate::stdlib::{setenv_unsafe, unsetenv_unsafe};
    let _l = crate::test_support::test_lock();
    let tmp = TempDir::new("xdg");
    let saved: Vec<(&str, Option<String>)> = ["HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME"]
        .into_iter()
        .map(|n| (n, crate::stdlib::getenv(n)))
        .collect();

    setenv_unsafe("HOME", &tmp.0, true).unwrap();
    unsetenv_unsafe("XDG_DATA_HOME").unwrap();
    unsetenv_unsafe("XDG_CONFIG_HOME").unwrap();
    let pref = get_pref_path(Some("My Org"), "My App").unwrap();
    assert_eq!(pref, format!("{}/.local/share/My Org/My App/", tmp.0));
    assert!(std::path::Path::new(&pref).is_dir());
    setenv_unsafe("XDG_DATA_HOME", &format!("{}/data/", tmp.0), true).unwrap();
    assert_eq!(
        get_pref_path(None, "App").unwrap(),
        format!("{}/data/App/", tmp.0)
    );

    std::fs::create_dir_all(tmp.path(".config")).unwrap();
    std::fs::write(
        tmp.path(".config/user-dirs.dirs"),
        "# comment\n  XDG_MUSIC_DIR=\"$HOME/Mu\\\"sic\"\nXDG_VIDEOS_DIR = \"/abs/vids\"\nXDG_PICTURES_DIR=\"relative\"\n",
    )
    .unwrap();
    quit_filesystem();
    assert_eq!(
        get_user_folder(Folder::Home).unwrap(),
        format!("{}/", tmp.0)
    );
    assert_eq!(
        get_user_folder(Folder::Music).unwrap(),
        format!("{}/Mu\"sic/", tmp.0)
    );
    assert_eq!(get_user_folder(Folder::Videos).unwrap(), "/abs/vids/");
    assert_eq!(
        get_user_folder(Folder::Desktop).unwrap(),
        format!("{}/Desktop/", tmp.0)
    );
    assert_eq!(
        get_user_folder(Folder::Pictures).unwrap_err().message(),
        "XDG directory not available"
    );
    assert_eq!(
        get_user_folder(Folder::SavedGames).unwrap_err().message(),
        "Saved Games folder unavailable on XDG"
    );

    for (name, value) in saved {
        match value {
            Some(v) => setenv_unsafe(name, &v, true).unwrap(),
            None => unsetenv_unsafe(name).unwrap(),
        }
    }
    quit_filesystem();
}

#[test]
fn base_path_and_exe_name() {
    let base = get_base_path().unwrap();
    assert!(base.ends_with(std::path::MAIN_SEPARATOR) || base.ends_with('/'));
    let exe = get_exe_name().unwrap();
    assert!(!exe.is_empty() && !exe.contains('/'));
    assert!(std::path::Path::new(&format!("{base}{exe}")).exists());
}
