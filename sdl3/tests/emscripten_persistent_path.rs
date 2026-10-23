// A test of SDL_RunApp()'s persistent storage on Emscripten.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! With the `emscripten-persistent-path` feature, `run_app()` mounts
//! IndexedDB storage (IDBFS, persisted automatically) at
//! `SDL_EMSCRIPTEN_PERSISTENT_PATH` and calls the main function once what
//! it holds is loaded, from the event loop, never returning itself; the
//! pref paths are made in it.
//!
//! In a browser (the program asks the runner for one) the page loads
//! twice: the first time the storage is empty, and the main function writes
//! a file in it and one in its pref path, then waits until both have
//! reached IndexedDB (read there directly) and reloads the page; the
//! second time they're loaded before the main function runs. Under node
//! there is no IndexedDB: the load fails with upstream's warning, and the
//! main function runs anyway on the mounted (in-memory) directory. Each
//! load's lines are compared with upstream's C SDL built with the same path
//! (`SDL_EMSCRIPTEN_PERSISTENT_PATH_STRING`) and run the same way. Run it
//! with
//!
//! ```sh
//! SDL_EMSCRIPTEN_PERSISTENT_PATH=/persistent SDL3_EMSCRIPTEN_BROWSER=1 cargo test -p sdl3 \
//!     --target wasm32-unknown-emscripten --features emscripten-persistent-path \
//!     --test emscripten_persistent_path
//! ```
//!
//! (`SDL3_EMSCRIPTEN_BROWSER=0` for node); the expected lines are for the
//! path `/persistent`.

#[cfg(not(target_os = "emscripten"))]
fn main() {}

/// The program's request for the browser (see `emscripten_browser.rs`).
#[cfg(target_os = "emscripten")]
#[used]
#[link_section = "sdl3_runner"]
static RUNNER: [u8; 7] = *b"browser";

#[cfg(target_os = "emscripten")]
fn main() {
    use std::sync::atomic::{AtomicBool, Ordering};

    use sdl3_emscripten::{em_asm, em_asm_int, sys};

    static RETURNED: AtomicBool = AtomicBool::new(false);
    let path = env!("SDL_EMSCRIPTEN_PERSISTENT_PATH");
    assert_eq!(
        path, "/persistent",
        "the expected lines are for /persistent"
    );
    // SAFETY: plain JavaScript.
    let browser = unsafe {
        em_asm_int!(
            r#"
            var node = typeof process === 'object' && !!(process.versions && process.versions.node);
            return !node && typeof window === 'object' && typeof document === 'object' ? 1 : 0;
        "#
        ) != 0
    };
    if !browser {
        // The page's URL, as a browser has it (run_app() reads it first).
        // SAFETY: plain JavaScript.
        unsafe { em_asm!("globalThis.window = { location: { search: '' } };") };
    }
    let code = sdl3::app::run_app(move |args| {
        assert!(!RETURNED.load(Ordering::SeqCst));
        // (which load of the page this is, in the tab's sessionStorage)
        let load = if browser {
            // SAFETY: plain JavaScript.
            unsafe {
                em_asm_int!("return Number(sessionStorage.getItem('sdl3TestLoad') || 0) + 1;")
            }
        } else {
            1
        };
        let contents = |file: &str| match std::fs::read(file) {
            Ok(data) => String::from_utf8_lossy(&data).into_owned(),
            Err(_) => "(none)".into(),
        };
        let is_dir = |dir: &str| std::path::Path::new(dir).is_dir() as i32;
        let mut lines = vec![
            format!("load {load}: argc {} mounted {}", args.len(), is_dir(path)),
            format!(
                "load {load}: save.txt [{}]",
                contents(&format!("{path}/save.txt"))
            ),
        ];
        let pref = sdl3::filesystem::get_pref_path(Some("org"), "app").unwrap();
        lines.push(format!("load {load}: pref {pref} dir {}", is_dir(&pref)));
        let pref_file = format!("{pref}pref.txt");
        lines.push(format!("load {load}: pref.txt [{}]", contents(&pref_file)));
        if load == 1 {
            std::fs::write(format!("{path}/save.txt"), b"saved").unwrap();
            std::fs::write(&pref_file, b"pref").unwrap();
            lines.push(format!(
                "load 1: written [{}] [{}]",
                contents(&format!("{path}/save.txt")),
                contents(&pref_file)
            ));
        }
        for line in &lines {
            println!("{line}");
        }
        let expected: Vec<String> = EXPECTED
            .lines()
            .filter(|l| l.starts_with(&format!("load {load}: ")) && !l.contains("IndexedDB"))
            .map(str::to_owned)
            .collect();
        assert_eq!(lines, expected, "upstream's C (above: here)");
        if !browser || load == 2 {
            println!("main function: {path} is mounted and loaded: ok");
            // (emscripten_force_exit(): the runtime is kept alive since
            // run_app() returned to the event loop, and exit() would leave
            // it running, waiting in the browser)
            // SAFETY: ends the program.
            unsafe { sys::emscripten_force_exit(0) };
        }
        // (the storage saves itself, autoPersist: once IndexedDB has both
        // files, the page again)
        // SAFETY: plain JavaScript.
        unsafe {
            em_asm!(
                r#"
                var tries = 0; var poll = setInterval(() => {
                  if (++tries > 500) { clearInterval(poll); throw new Error('the files never reached IndexedDB'); }
                  var open = indexedDB.open('/persistent');
                  open.onsuccess = () => {
                    var db = open.result;
                    if (!db.objectStoreNames.contains('FILE_DATA')) { db.close(); return; }
                    var keys = db.transaction(['FILE_DATA'], 'readonly').objectStore('FILE_DATA').getAllKeys();
                    keys.onsuccess = () => {
                      db.close();
                      var k = keys.result;
                      if (poll === null || !k.includes('/persistent/save.txt') || !k.includes('/persistent/org/app/pref.txt')) return;
                      clearInterval(poll); poll = null;
                      console.log('load 1: in IndexedDB ' + JSON.stringify(k.sort()));
                      sessionStorage.setItem('sdl3TestLoad', '1');
                      location.reload();
                    };
                  };
                }, 20);
            "#
            )
        };
        0
    });
    RETURNED.store(true, Ordering::SeqCst);
    unreachable!("run_app() returned {code}");
}

/// Upstream's C SDL (3.5.0, emcc 6.0.13) with
/// `SDL_EMSCRIPTEN_PERSISTENT_PATH_STRING` `"/persistent"`, in headless
/// Chromium (under node, the first load's lines and upstream's warning):
/// the same program (scratchpad `pr14/cref/persist.c` when this was
/// written). The IndexedDB line is the page's JavaScript.
#[cfg(target_os = "emscripten")]
const EXPECTED: &str = "\
load 1: argc 1 mounted 1
load 1: save.txt [(none)]
load 1: pref /persistent/org/app/ dir 1
load 1: pref.txt [(none)]
load 1: written [saved] [pref]
load 1: in IndexedDB [\"/persistent/org\",\"/persistent/org/app\",\"/persistent/org/app/pref.txt\",\"/persistent/save.txt\"]
load 2: argc 1 mounted 1
load 2: save.txt [saved]
load 2: pref /persistent/org/app/ dir 1
load 2: pref.txt [pref]
";
