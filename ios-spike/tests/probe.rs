// libtest under `xcrun simctl spawn` (as a cargo target runner).

use std::path::Path;

#[test]
fn passes() {
    println!("passes: stdout");
    eprintln!("passes: stderr");
}

#[test]
fn fails_when_asked() {
    if std::env::var_os("SPIKE_FAIL").is_some() {
        panic!("failing on purpose (SPIKE_FAIL is set)");
    }
}

#[test]
#[ignore = "ignored on purpose"]
fn ignored() {}

#[test]
fn environment_and_cwd() {
    println!("cwd = {:?}", std::env::current_dir());
    println!("exe = {:?}", std::env::current_exe());
    for (k, v) in std::env::vars() {
        println!("env {k}={v}");
    }
}

#[test]
fn manifest_dir_readable() {
    // env! is the build host's path: is it visible from the simulator?
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let toml = dir.join("Cargo.toml");
    match std::fs::read_to_string(&toml) {
        Ok(s) => println!("read {} ({} bytes): OK", toml.display(), s.len()),
        Err(e) => panic!("can't read {}: {e}", toml.display()),
    }
    // and CARGO_TARGET_TMPDIR (integration tests only)
    let tmp = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let f = tmp.join("spike-write-test.txt");
    match std::fs::write(&f, b"x") {
        Ok(()) => println!("wrote {}: OK", f.display()),
        Err(e) => println!("can't write {}: {e}", f.display()),
    }
}

#[test]
fn temp_dir_writable() {
    let t = std::env::temp_dir();
    let f = t.join("spike-temp.txt");
    std::fs::write(&f, b"x").unwrap();
    println!("temp_dir = {} (writable)", t.display());
    std::fs::remove_file(&f).unwrap();
}

#[test]
fn subprocess_of_self() {
    // sdl3-shadercross's cli tests run other binaries: can a simulator
    // process spawn one?
    let exe = std::env::current_exe().unwrap();
    let r = std::process::Command::new(&exe).arg("--list").output();
    match r {
        Ok(o) => println!(
            "spawned self --list: status {:?}, {} bytes stdout",
            o.status,
            o.stdout.len()
        ),
        Err(e) => println!("can't spawn self: {e}"),
    }
    let r = std::process::Command::new("/bin/sh").arg("-c").arg("echo sh-ok").output();
    match r {
        Ok(o) => println!("spawned /bin/sh: {:?} {:?}", o.status, String::from_utf8_lossy(&o.stdout)),
        Err(e) => println!("can't spawn /bin/sh: {e}"),
    }
}

#[test]
fn threads() {
    let h: Vec<_> = (0..8).map(|i| std::thread::spawn(move || i * 2)).collect();
    let s: i32 = h.into_iter().map(|h| h.join().unwrap()).sum();
    assert_eq!(s, 56);
}
