// A test of SDL_RunApp()'s persistent storage on Emscripten.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! With the `emscripten-persistent-path` feature, `run_app()` mounts
//! IndexedDB storage (IDBFS) at `SDL_EMSCRIPTEN_PERSISTENT_PATH` and calls
//! the main function once it's loaded, from the event loop, never
//! returning itself. Under node there is no IndexedDB: the load fails with
//! upstream's warning, and the main function runs anyway on the mounted
//! (in-memory) directory. Run it with
//!
//! ```sh
//! SDL_EMSCRIPTEN_PERSISTENT_PATH=/persistent cargo test -p sdl3 \
//!     --target wasm32-unknown-emscripten --features emscripten-persistent-path \
//!     --test emscripten_persistent_path
//! ```

#[cfg(not(target_os = "emscripten"))]
fn main() {}

#[cfg(target_os = "emscripten")]
fn main() {
    use std::sync::atomic::{AtomicBool, Ordering};

    static RETURNED: AtomicBool = AtomicBool::new(false);
    let path = env!("SDL_EMSCRIPTEN_PERSISTENT_PATH");
    // The page's URL, as a browser has it (run_app() reads it first).
    // SAFETY: plain JavaScript.
    unsafe { sdl3_emscripten::em_asm!("globalThis.window = { location: { search: '' } };") };
    let code = sdl3::app::run_app(move |args| {
        assert_eq!(args.len(), 1);
        assert!(!RETURNED.load(Ordering::SeqCst));
        assert!(std::path::Path::new(path).is_dir(), "{path}");
        let file = format!("{path}/save.txt");
        std::fs::write(&file, b"saved").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"saved");
        println!("main function: {path} is mounted: ok");
        std::process::exit(0)
    });
    RETURNED.store(true, Ordering::SeqCst);
    unreachable!("run_app() returned {code}");
}
