use super::*;
use crate::error::Error;
use crate::test_support::TempDir;

/// The path separator the backend puts in the paths it reports.
const SEP: &str = if cfg!(windows) { "\\" } else { "/" };

/// What the POSIX and the Windows operations call each failure.
fn expected_error(posix: &'static str, windows: &'static str) -> &'static str {
    if cfg!(windows) {
        windows
    } else {
        posix
    }
}

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
        .starts_with(expected_error(
            "Can't create directory: ",
            "Couldn't create directory: "
        )));

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
        .starts_with(expected_error(
            "Can't rename path: ",
            "Couldn't rename path: "
        )));

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
        assert_eq!(dir, format!("{}{SEP}", tmp.0));
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
            .starts_with(expected_error(
                "Can't open directory: ",
                "Failed to enumerate directory: "
            ))
    );

    // (with the backend's separators)
    let sorted = |v: Vec<String>| {
        let mut v: Vec<String> = v.into_iter().map(|p| p.replace(SEP, "/")).collect();
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

#[cfg(all(unix, not(target_vendor = "apple"), not(target_os = "emscripten")))]
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
#[cfg_attr(target_os = "emscripten", ignore = "no executable name on Emscripten")]
fn base_path_and_exe_name() {
    let base = get_base_path().unwrap();
    assert!(base.ends_with(std::path::MAIN_SEPARATOR) || base.ends_with('/'));
    let exe = get_exe_name().unwrap();
    assert!(!exe.is_empty() && !exe.contains('/'));
    assert!(std::path::Path::new(&format!("{base}{exe}")).exists());
}

/// What upstream's `filesystem/emscripten` reports under node, built with
/// emcc (the same calls, in the same order, on a fresh MEMFS): the pref
/// paths made under `/libsdl`, one that runs into a file, and `$HOME`.
#[cfg(all(target_os = "emscripten", not(feature = "emscripten-persistent-path")))]
#[test]
fn emscripten_paths() {
    use crate::stdlib::{setenv_unsafe, unsetenv_unsafe};
    let _l = crate::test_support::test_lock();
    quit_filesystem();
    assert_eq!(get_base_path().unwrap(), "/");
    assert_eq!(get_exe_name().unwrap_err().message(), "");

    assert_eq!(
        get_pref_path(Some("Org"), "App").unwrap(),
        "/libsdl/Org/App/"
    );
    assert_eq!(get_pref_path(None, "App2").unwrap(), "/libsdl/App2/");
    assert_eq!(get_pref_path(Some(""), "a/b").unwrap(), "/libsdl/a/b/");
    for dir in ["/libsdl", "/libsdl/Org/App", "/libsdl/App2", "/libsdl/a/b"] {
        let meta = std::fs::metadata(dir).unwrap();
        assert!(meta.is_dir(), "{dir}");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(meta.permissions().mode() & 0o777, 0o700, "{dir}");
    }
    // Again: everything is there already.
    assert_eq!(
        get_pref_path(Some("Org"), "App").unwrap(),
        "/libsdl/Org/App/"
    );
    // A file where a directory goes counts as made (EEXIST), until a
    // directory under it can't be.
    std::fs::write("/libsdl/file", b"").unwrap();
    assert_eq!(
        get_pref_path(Some("file"), "x").unwrap_err().message(),
        "Couldn't create directory '/libsdl/file/x': 'Not a directory'"
    );
    assert_eq!(
        get_pref_path(Some("Org"), "file2").unwrap(),
        "/libsdl/Org/file2/"
    );

    let saved = crate::stdlib::getenv("HOME");
    setenv_unsafe("HOME", "/home/web_user", true).unwrap();
    assert_eq!(get_user_folder(Folder::Home).unwrap(), "/home/web_user/");
    for folder in [Folder::Desktop, Folder::Documents, Folder::SavedGames] {
        assert_eq!(
            get_user_folder(folder).unwrap_err().message(),
            "Emscripten only supports the home folder"
        );
    }
    quit_filesystem();
    unsetenv_unsafe("HOME").unwrap();
    assert_eq!(
        get_user_folder(Folder::Home).unwrap_err().message(),
        "No $HOME environment variable available"
    );
    match saved {
        Some(home) => setenv_unsafe("HOME", &home, true).unwrap(),
        None => unsetenv_unsafe("HOME").unwrap(),
    }
    quit_filesystem();
    std::fs::remove_file("/libsdl/file").unwrap();
}

/// The SDL part of an error: on Windows, without the system's text for the
/// error code that follows it (it depends on the language, and on Wine or
/// Windows); elsewhere the whole message.
#[cfg(any(target_os = "linux", target_vendor = "apple", windows))]
fn sdl_part(e: &Error) -> &str {
    let m = e.message();
    if cfg!(windows) {
        m.rfind(": ").map_or(m, |i| &m[..i])
    } else {
        m
    }
}

/// What upstream's `SDL_sysfsops.c` and `SDL_filesystem.c` report for the
/// calls of `fsops_match_upstream` on a fresh tree: upstream's C built for
/// Linux and run there (and for macOS, which reports the same), and built
/// for Windows and run under Wine. Errors
/// are cut at the system's text on Windows; directory entries are sorted
/// case-insensitively.
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
const FSOPS_EXPECTED: &str = r"mkdir nested: ok
mkdir again: ok
save x: ok
save Y: ok
save inner: ok
mkdir under file: Can't create directory: File exists
mkdir on file: Can't create directory: File exists
info file: type 1 size 5 times 1
info dir: type 2 size 0 times 1
info missing: error Can't stat: No such file or directory
copy: ok
copy over: ok
info copied: type 1 size 2 times 1
copy missing: Couldn't open <root>/missing: No such file or directory
rename: ok
rename over: ok
rename missing: Can't rename path: No such file or directory
info renamed: type 1 size 2 times 1
remove empty dir: ok
remove gone: ok
remove missing parent: ok
remove non-empty: Can't remove path: Directory not empty
enumerate:
  enum [<root>/] [a]
  enum [<root>/] [x.txt]
  enum [<root>/] [Y.TXT]
enumerate done: ok
enumerate missing: Can't open directory: No such file or directory
glob *.txt 0: [x.txt]
glob *.txt 1: [Y.TXT] [x.txt]
glob a/* 0: [a/b]
glob a/b/*.txt 0: [a/b/inner.txt]
glob */*/*.TXT 1: [a/b/inner.txt]
glob ?.txt 0: [x.txt]
glob (null) 0: [Y.TXT] [a] [a/b] [a/b/inner.txt] [x.txt]
glob a/b/* 0: [a/b/inner.txt]
glob (null) 0: error Can't open directory: No such file or directory
";
#[cfg(windows)]
const FSOPS_EXPECTED: &str = r"mkdir nested: ok
mkdir again: ok
save x: ok
save Y: ok
save inner: ok
mkdir under file: Couldn't create directory
mkdir on file: Couldn't create directory
info file: type 1 size 5 times 1
info dir: type 2 size 0 times 1
info missing: error Can't stat
copy: ok
copy over: ok
info copied: type 1 size 2 times 1
copy missing: Couldn't copy path
rename: ok
rename over: ok
rename missing: Couldn't rename path
info renamed: type 1 size 2 times 1
remove empty dir: ok
remove gone: ok
remove missing parent: Couldn't get path's attributes
remove non-empty: Couldn't remove path
enumerate:
  enum [<root>\] [a]
  enum [<root>\] [x.txt]
  enum [<root>\] [Y.TXT]
enumerate done: ok
enumerate missing: Failed to enumerate directory
glob *.txt 0: [x.txt]
glob *.txt 1: [Y.TXT] [x.txt]
glob a/* 0: [a\b]
glob a\b\*.txt 0:
glob */*/*.TXT 1: [a\b\inner.txt]
glob ?.txt 0: [x.txt]
glob (null) 0: [Y.TXT] [a] [a\b] [a\b\inner.txt] [x.txt]
glob a/b/* 0: [a\b\inner.txt]
glob (null) 0: error Failed to enumerate directory
";

#[cfg(any(target_os = "linux", target_vendor = "apple", windows))]
#[test]
fn fsops_match_upstream() {
    let tmp = TempDir::new("fsopsref");
    // (paths are written with the platform's own separator, `b`, here and
    // there, as the C harness writes them)
    let (root, b) = if cfg!(windows) {
        (tmp.0.replace('/', "\\"), "\\")
    } else {
        (tmp.0.clone(), "/")
    };
    let mut out = String::new();
    let mut line = |s: String| {
        out.push_str(&s);
        out.push('\n');
    };
    let result = |r: Result<()>| match r {
        Ok(()) => "ok".to_owned(),
        Err(e) => sdl_part(&e).to_owned(),
    };
    let info = |path: &str| match get_path_info(path) {
        Ok(i) => format!(
            "type {} size {} times {}",
            i.kind as i32,
            i.size,
            (i.create_time > Time::UNIX_EPOCH
                && i.modify_time > Time::UNIX_EPOCH
                && i.access_time > Time::UNIX_EPOCH) as i32
        ),
        Err(e) => format!("error {}", sdl_part(&e)),
    };
    let glob = |root: &str, pattern: Option<&str>, flags: GlobFlags| {
        let mut s = format!("glob {} {}:", pattern.unwrap_or("(null)"), flags.0);
        match glob_directory(root, pattern, flags) {
            Ok(mut r) => {
                r.sort();
                for p in r {
                    s += &format!(" [{p}]");
                }
            }
            Err(e) => s += &format!(" error {}", sdl_part(&e)),
        }
        s
    };

    let r = &root;
    let nested = format!("{r}{b}a/b{b}c/");
    line(format!(
        "mkdir nested: {}",
        result(create_directory(&nested))
    ));
    line(format!(
        "mkdir again: {}",
        result(create_directory(&nested))
    ));
    let save = |p: &str, data: &[u8]| result(crate::io::save_file(p, data));
    line(format!("save x: {}", save(&format!("{r}/x.txt"), b"hello")));
    line(format!("save Y: {}", save(&format!("{r}/Y.TXT"), b"hi")));
    line(format!(
        "save inner: {}",
        save(&format!("{r}/a/b/inner.txt"), b"inner!")
    ));
    line(format!(
        "mkdir under file: {}",
        result(create_directory(&format!("{r}/x.txt/sub")))
    ));
    line(format!(
        "mkdir on file: {}",
        result(create_directory(&format!("{r}/x.txt")))
    ));
    line(format!("info file: {}", info(&format!("{r}/x.txt"))));
    line(format!("info dir: {}", info(&format!("{r}/a"))));
    line(format!("info missing: {}", info(&format!("{r}/missing"))));
    let (x, copy) = (format!("{r}/x.txt"), format!("{r}/a/copy.txt"));
    line(format!("copy: {}", result(copy_file(&x, &copy))));
    line(format!(
        "copy over: {}",
        result(copy_file(&format!("{r}/Y.TXT"), &copy))
    ));
    line(format!("info copied: {}", info(&copy)));
    line(format!(
        "copy missing: {}",
        result(copy_file(&format!("{r}/missing"), &copy))
    ));
    let moved = format!("{r}/a/moved.txt");
    line(format!("rename: {}", result(rename_path(&copy, &moved))));
    line(format!("rename over: {}", result(rename_path(&moved, &x))));
    line(format!(
        "rename missing: {}",
        result(rename_path(&moved, &x))
    ));
    line(format!("info renamed: {}", info(&x)));
    let c = format!("{r}/a/b/c");
    line(format!("remove empty dir: {}", result(remove_path(&c))));
    line(format!("remove gone: {}", result(remove_path(&c))));
    line(format!(
        "remove missing parent: {}",
        result(remove_path(&format!("{r}/nope/deeper")))
    ));
    line(format!(
        "remove non-empty: {}",
        result(remove_path(&format!("{r}/a")))
    ));

    line("enumerate:".to_owned());
    let mut entries = Vec::new();
    let done = enumerate_directory(&format!("{r}{b}{b}//"), |dir, name| {
        entries.push(format!("  enum [{dir}] [{name}]"));
        Ok(ControlFlow::Continue(()))
    });
    entries.sort_by_key(|e| e.to_lowercase());
    for e in entries {
        line(e);
    }
    line(format!("enumerate done: {}", result(done)));
    line(format!(
        "enumerate missing: {}",
        result(enumerate_directory(&format!("{r}/missing"), |_, _| Ok(
            ControlFlow::Continue(())
        )))
    ));

    line(glob(r, Some("*.txt"), GlobFlags::NONE));
    line(glob(r, Some("*.txt"), GlobFlags::CASE_INSENSITIVE));
    line(glob(r, Some("a/*"), GlobFlags::NONE));
    line(glob(r, Some(&format!("a{b}b{b}*.txt")), GlobFlags::NONE));
    line(glob(r, Some("*/*/*.TXT"), GlobFlags::CASE_INSENSITIVE));
    line(glob(r, Some("?.txt"), GlobFlags::NONE));
    line(glob(r, None, GlobFlags::NONE));
    line(glob(&format!("{r}{b}"), Some("a/b/*"), GlobFlags::NONE));
    line(glob(&format!("{r}/missing"), None, GlobFlags::NONE));

    assert_eq!(out.replace(r.as_str(), "<root>"), FSOPS_EXPECTED);
}

/// The drive letters, the known folders, the pref path and the current
/// directory hold together on any Windows (or Wine) installation.
#[cfg(windows)]
#[test]
fn windows_paths() {
    let _l = crate::test_support::test_lock();
    quit_filesystem();

    // An empty path lists the drives.
    let mut drives = Vec::new();
    enumerate_directory("", |dir, name| {
        assert_eq!(dir, "");
        drives.push(name.to_owned());
        Ok(ControlFlow::Continue(()))
    })
    .unwrap();
    assert!(!drives.is_empty());
    for d in &drives {
        let b = d.as_bytes();
        assert!(
            b.len() == 3 && b[0].is_ascii_uppercase() && &b[1..] == b":\\",
            "{d}"
        );
    }
    assert!(drives.windows(2).all(|w| w[0] < w[1]));

    let is_dir = |p: &str| std::path::Path::new(p).is_dir();
    let home = get_user_folder(Folder::Home).unwrap();
    let profile = std::env::var("USERPROFILE").unwrap();
    assert_eq!(home.to_lowercase(), format!("{profile}\\").to_lowercase());
    for folder in Folder::ALL {
        match get_user_folder(folder) {
            Ok(path) => {
                assert!(path.ends_with('\\') && is_dir(&path), "{folder:?}: {path}");
            }
            Err(e) if folder == Folder::PublicShare => {
                assert_eq!(e.message(), "Public share unavailable on Windows");
            }
            Err(e) => {
                // (Wine has no Screenshots folder, for one)
                assert!(
                    matches!(folder, Folder::SavedGames | Folder::Screenshots)
                        && sdl_part(&e) == "Couldn't get folder",
                    "{folder:?}: {e}"
                );
            }
        }
    }

    let appdata = std::env::var("APPDATA").unwrap();
    let pref = get_pref_path(Some("SDL3 Test Org"), "Test App").unwrap();
    assert_eq!(
        pref.to_lowercase(),
        format!("{appdata}\\SDL3 Test Org\\Test App\\").to_lowercase()
    );
    assert!(is_dir(&pref));
    let pref2 = get_pref_path(None, "SDL3 Test App 2").unwrap();
    assert_eq!(
        pref2.to_lowercase(),
        format!("{appdata}\\SDL3 Test App 2\\").to_lowercase()
    );
    let e = get_pref_path(Some(&"x".repeat(299)), "App").unwrap_err();
    assert_eq!(sdl_part(&e), "Path too long.");
    std::fs::remove_dir(&pref).unwrap();
    std::fs::remove_dir(format!("{appdata}\\SDL3 Test Org")).unwrap();
    std::fs::remove_dir(&pref2).unwrap();

    let cwd = get_current_directory().unwrap();
    let std_cwd = std::env::current_dir().unwrap();
    assert_eq!(
        cwd.trim_end_matches('\\'),
        std_cwd.to_string_lossy().trim_end_matches('\\')
    );
    assert!(cwd.ends_with('\\'));

    let base = get_base_path().unwrap();
    let exe = std::env::current_exe().unwrap();
    assert_eq!(
        format!("{base}{}", get_exe_name().unwrap()).to_lowercase(),
        exe.to_string_lossy().to_lowercase()
    );
    quit_filesystem();
}
