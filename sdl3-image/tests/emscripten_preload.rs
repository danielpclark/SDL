// Tests of SDL_image's preloaded images on Emscripten (src/IMG.c).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `IMG_Load()` and `IMG_LoadTyped_IO()` on Emscripten first take the image
//! the browser decoded when the file was preloaded (emscripten's
//! `emscripten_get_preloaded_image_data()` and its `_from_FILE` variant:
//! the canvas emscripten's image plugin keeps in `Browser.preloadedImages`
//! under the file's absolute path), as an RGBA32 surface. A program of its
//! own (`harness = false`), which asks `tools/emscripten_run.py` for the
//! browser (the custom section below), with two checks, each a trace of
//! steps against upstream's C trace of the same steps on the same page
//! (SDL_image's `IMG.c` built with emcc, over upstream's C SDL):
//!
//! * `standins`, under node and in the browser: stand-in canvases (what
//!   emscripten reads of one: its size and `getImageData()`) put in
//!   `Browser.preloadedImages` as the plugin would, then [`load`] by
//!   absolute and relative paths, a preloaded path with no file (which
//!   `load` takes and a file stream can't open), a preloaded file whose
//!   contents are no image (taken through its stream's descriptor by
//!   [`load_io`], with a type by [`load_typed_io`], through
//!   [`IoStream::from_std_file`] too, and not through a memory stream of
//!   the same bytes), an empty canvas, and a file that wasn't preloaded,
//!   which is decoded as usual.
//! * `plugin`, in the browser only (under node it skips as `browser`):
//!   emscripten's own image plugin, through `FS.preloadFile()` as its file
//!   packager calls it for `--preload-file` with `--use-preload-plugins`,
//!   on a PNG and a 24-bit BMP of a known opaque image (decoded by the
//!   browser into a canvas, exactly) and on a PNG preloaded without its
//!   file; their pixels must be the image's, from the path and from a
//!   stream, and still after the file is overwritten.
//!
//! `SDL3_TEST_TRACE=1` prints the traces. `SDL3_EMSCRIPTEN_BROWSER=1
//! SDL3_TEST_REQUIRE=browser cargo test -p sdl3-image --target
//! wasm32-unknown-emscripten --test emscripten_preload` (with `CHROME`
//! naming the browser) runs it as CI does: see "Emscripten (browser)" in
//! docs/HARDWARE_TESTING.md.
//!
//! [`load`]: sdl3_image::load
//! [`load_io`]: sdl3_image::load_io
//! [`load_typed_io`]: sdl3_image::load_typed_io
//! [`IoStream::from_std_file`]: sdl3::io::IoStream::from_std_file

#[cfg(not(target_os = "emscripten"))]
fn main() {}

#[cfg(target_os = "emscripten")]
fn main() {
    preload::main()
}

/// The program's request for the browser: `tools/emscripten_run.py` runs a
/// program whose .wasm has this section in headless Chromium.
#[cfg(target_os = "emscripten")]
#[used]
#[link_section = "sdl3_runner"]
static RUNNER: [u8; 7] = *b"browser";

#[cfg(target_os = "emscripten")]
mod preload {
    use std::cell::RefCell;
    use std::fs::File;
    use std::panic::{self, AssertUnwindSafe};
    use std::time::{Duration, Instant};

    use sdl3::error::Result;
    use sdl3::io::IoStream;
    use sdl3::stdlib::crc32::crc32;
    use sdl3::video::{PixelFormat, Surface};
    use sdl3_emscripten::{em_asm, em_asm_int, sys};

    /// How long the browser may take to decode the preloaded images.
    const PATIENCE: Duration = Duration::from_secs(30);

    /// Upstream's C trace of `standins` (`IMG.c` built with emcc, run
    /// under node and in headless Chromium alike).
    const STANDINS_TRACE: &str = "\
load /standin.png: 5x3 pitch 20 format 0x16762004 crc cf6af009
load standin.png: 5x3 pitch 20 format 0x16762004 crc cf6af009
load /tmp/../standin.png: 5x3 pitch 20 format 0x16762004 crc cf6af009
load_io file /standin.png: error Couldn't open /standin.png: No such file or directory
load /garbage.png: 4x2 pitch 16 format 0x16762004 crc fcf9997e
load_io file /garbage.png: 4x2 pitch 16 format 0x16762004 crc fcf9997e
load_typed_io file /garbage.png TGA: 4x2 pitch 16 format 0x16762004 crc fcf9997e
load_io fp /garbage.png: 4x2 pitch 16 format 0x16762004 crc fcf9997e
load_io mem (its bytes): error Unsupported image format
load /empty.png: 0x0 pitch 0 format 0x16762004 crc 00000000
load /plain.bmp: 5x3 pitch 16 format 0x17401803 crc 29e7da99
load_io file /plain.bmp: 5x3 pitch 16 format 0x17401803 crc 29e7da99
";

    /// Upstream's C trace of `plugin` in headless Chromium.
    const PLUGIN_TRACE: &str = "\
preloaded /real.png: HTMLCanvasElement 5x3
preloaded /real.bmp: HTMLCanvasElement 5x3
preloaded /nofile.png: HTMLCanvasElement 5x3
load /real.png: 5x3 pitch 20 format 0x16762004 crc d6c14138
load_io file /real.png: 5x3 pitch 20 format 0x16762004 crc d6c14138
load /real.bmp: 5x3 pitch 20 format 0x16762004 crc d6c14138
load /nofile.png: 5x3 pitch 20 format 0x16762004 crc d6c14138
load_io file /nofile.png: error Couldn't open /nofile.png: No such file or directory
load /real.png (overwritten): 5x3 pitch 20 format 0x16762004 crc d6c14138
load_io file /real.png (overwritten): 5x3 pitch 20 format 0x16762004 crc d6c14138
load_io mem (its bytes): error Unsupported image format
";

    /// Whether `SDL3_TEST_REQUIRE` makes `capability` mandatory.
    fn required(capability: &str) -> bool {
        std::env::var("SDL3_TEST_REQUIRE")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .any(|c| c == "all" || c == capability)
    }

    /// A missing capability: a note (the CI skip list reads it), or a
    /// failure when it's required.
    fn skip(capability: &str, reason: &str) {
        if required(capability) {
            panic!("required capability {capability} unavailable: {reason}");
        }
        println!("note: skipping, {capability} unavailable: {reason}");
    }

    /// Whether this is a page in a browser (not node).
    fn in_browser() -> bool {
        // SAFETY: plain JavaScript.
        unsafe {
            em_asm_int!(
                r#"
                var node = typeof process === 'object' && !!(process.versions && process.versions.node);
                return !node && typeof window === 'object' && typeof document === 'object' ? 1 : 0;
            "#
            ) != 0
        }
    }

    /// A check's `test` line, libtest's way, after its trace if it failed
    /// (or with `SDL3_TEST_TRACE`).
    fn report(name: &str, check: impl FnOnce()) -> bool {
        let ok = panic::catch_unwind(AssertUnwindSafe(check)).is_ok();
        println!("test {name} ... {}", if ok { "ok" } else { "FAILED" });
        ok
    }

    /// Compares a trace with upstream's, line by line.
    fn compare(name: &str, trace: &str, expected: &str) {
        if std::env::var_os("SDL3_TEST_TRACE").is_some() || trace != expected {
            println!("{name}: the trace:\n{trace}");
        }
        for (i, (got, want)) in trace.lines().zip(expected.lines()).enumerate() {
            assert_eq!(got, want, "{name}: line {} differs from upstream's", i + 1);
        }
        assert_eq!(
            trace.lines().count(),
            expected.lines().count(),
            "{name}: not upstream's number of lines"
        );
    }

    /// A trace line: the surface's size, pitch, format and the CRC-32 of
    /// its pixels (`SDL_crc32()` of `pitch * h` bytes), or the error.
    fn describe(trace: &mut String, step: &str, surface: Result<Surface<'_>>) {
        let line = match surface {
            Ok(s) => format!(
                "{step}: {}x{} pitch {} format {:#010x} crc {:08x}",
                s.width(),
                s.height(),
                s.pitch(),
                s.format().0,
                s.pixels().map_or(0, |p| crc32(0, p))
            ),
            Err(e) => format!("{step}: error {e}"),
        };
        trace.push_str(&line);
        trace.push('\n');
    }

    fn file_io(path: &str) -> Result<Surface<'static>> {
        sdl3_image::load_io(&mut IoStream::from_file(path, "rb")?)
    }

    /// The bytes of a file that is no image.
    const GARBAGE: &[u8] = b"not an image";

    /// `standins`: the canvases emscripten's image plugin would keep, stood
    /// in for.
    fn standins() -> String {
        // SAFETY: plain JavaScript (emscripten's `Browser`, which the
        // preloaded image functions bring into the program).
        unsafe {
            em_asm!(
                r#"
                function standin(w, h, seed) {
                    var data = new Uint8ClampedArray(w * h * 4);
                    for (var i = 0; i < w * h; i++) {
                        data[4 * i] = (i * 37 + seed * 11) & 255;
                        data[4 * i + 1] = (i * 59 + seed * 3) & 255;
                        data[4 * i + 2] = (i * 13 + seed * 101) & 255;
                        data[4 * i + 3] = (i * 7 + seed) & 255;
                    }
                    return { width: w, height: h, getContext: (type) => ({
                        getImageData: (x, y, cw, ch) => ({ width: cw, height: ch, data: data }),
                    }) };
                }
                Browser.preloadedImages['/standin.png'] = standin(5, 3, 1);
                Browser.preloadedImages['/garbage.png'] = standin(4, 2, 2);
                Browser.preloadedImages['/empty.png'] = standin(0, 0, 3);
            "#
            )
        };
        let mut t = String::new();
        describe(
            &mut t,
            "load /standin.png",
            sdl3_image::load("/standin.png"),
        );
        describe(&mut t, "load standin.png", sdl3_image::load("standin.png"));
        describe(
            &mut t,
            "load /tmp/../standin.png",
            sdl3_image::load("/tmp/../standin.png"),
        );
        describe(&mut t, "load_io file /standin.png", file_io("/standin.png"));

        std::fs::write("/garbage.png", GARBAGE).unwrap();
        describe(
            &mut t,
            "load /garbage.png",
            sdl3_image::load("/garbage.png"),
        );
        describe(&mut t, "load_io file /garbage.png", file_io("/garbage.png"));
        let typed = IoStream::from_file("/garbage.png", "rb")
            .and_then(|mut io| sdl3_image::load_typed_io(&mut io, Some("TGA")));
        describe(&mut t, "load_typed_io file /garbage.png TGA", typed);
        let mut fp = IoStream::from_std_file(File::open("/garbage.png").unwrap(), "rb", true);
        describe(
            &mut t,
            "load_io fp /garbage.png",
            sdl3_image::load_io(&mut fp),
        );
        let mem = sdl3_image::load_io(&mut IoStream::from_const_mem(GARBAGE));
        describe(&mut t, "load_io mem (its bytes)", mem);
        describe(&mut t, "load /empty.png", sdl3_image::load("/empty.png"));

        let mut plain = pattern().convert(PixelFormat::RGB24).unwrap();
        plain.save_bmp("/plain.bmp").unwrap();
        describe(&mut t, "load /plain.bmp", sdl3_image::load("/plain.bmp"));
        describe(&mut t, "load_io file /plain.bmp", file_io("/plain.bmp"));
        t
    }

    /// The known image: 5×3, opaque, every pixel different.
    fn pattern() -> Surface<'static> {
        let mut s = Surface::new(5, 3, PixelFormat::RGBA32).unwrap();
        let pitch = s.pitch() as usize;
        let pixels = s.pixels_mut().unwrap();
        for y in 0..3 {
            for x in 0..5 {
                let i = y * pitch + x * 4;
                let rgba = [40 * x + 10, 70 * y + 20, (13 * x * y + 100) & 255, 255];
                for (p, c) in pixels[i..i + 4].iter_mut().zip(rgba) {
                    *p = c as u8;
                }
            }
        }
        s
    }

    fn png_and_bmp() -> (Vec<u8>, Vec<u8>) {
        let mut png = IoStream::from_dynamic_mem();
        pattern().save_png_io(&mut png).unwrap();
        let mut bmp = IoStream::from_dynamic_mem();
        let mut rgb = pattern().convert(PixelFormat::RGB24).unwrap();
        rgb.save_bmp_io(&mut bmp).unwrap();
        (
            png.dynamic_memory().unwrap().to_vec(),
            bmp.dynamic_memory().unwrap().to_vec(),
        )
    }

    /// `plugin`'s start: the files preloaded through emscripten's own
    /// `FS.preloadFile()`, as its file packager does (the full path, no
    /// name, readable and writable, `dontCreateFile`, `canOwn`); the
    /// iterations wait for it.
    fn plugin_start() {
        let (png, bmp) = png_and_bmp();
        // SAFETY: plain JavaScript, reading the two buffers.
        unsafe {
            em_asm!(
                r#"
                var png = HEAPU8.slice($0, $0 + $1);
                var bmp = HEAPU8.slice($2, $2 + $3);
                Module['sdl3_test_preloaded'] = 0;
                Promise.all([
                    FS.preloadFile('/real.png', null, png, true, true, false, true),
                    FS.preloadFile('/real.bmp', null, bmp, true, true, false, true),
                    FS.preloadFile('/nofile.png', null, png.slice(), true, true, true, true),
                ]).then(() => { Module['sdl3_test_preloaded'] = 1; },
                        (e) => { console.log('preloading failed: ' + e); Module['sdl3_test_preloaded'] = -1; });
            "#,
                png.as_ptr(),
                png.len() as i32,
                bmp.as_ptr(),
                bmp.len() as i32
            )
        };
    }

    /// What `Browser.preloadedImages` holds for the three files, a line
    /// each.
    fn preloaded(t: &mut String) {
        let mut buf = [0u8; 512];
        // SAFETY: plain JavaScript, writing at most the buffer's length.
        let len = unsafe {
            em_asm_int!(
                r#"
                var s = '';
                for (var path of ['/real.png', '/real.bmp', '/nofile.png']) {
                    var c = Browser.preloadedImages[path];
                    s += 'preloaded ' + path + ': ' + (c ? c.constructor.name + ' ' + c.width + 'x' + c.height : 'none') + '|';
                }
                var bytes = new TextEncoder().encode(s).subarray(0, $1);
                HEAPU8.set(bytes, $0);
                return bytes.length;
            "#,
                buf.as_mut_ptr(),
                buf.len() as i32
            )
        };
        for line in String::from_utf8_lossy(&buf[..len as usize]).split_terminator('|') {
            t.push_str(line);
            t.push('\n');
        }
    }

    /// `plugin`, once the browser has decoded the files.
    fn plugin() -> String {
        let mut t = String::new();
        preloaded(&mut t);
        describe(&mut t, "load /real.png", sdl3_image::load("/real.png"));
        describe(&mut t, "load_io file /real.png", file_io("/real.png"));
        describe(&mut t, "load /real.bmp", sdl3_image::load("/real.bmp"));
        describe(&mut t, "load /nofile.png", sdl3_image::load("/nofile.png"));
        describe(&mut t, "load_io file /nofile.png", file_io("/nofile.png"));
        std::fs::write("/real.png", GARBAGE).unwrap();
        describe(
            &mut t,
            "load /real.png (overwritten)",
            sdl3_image::load("/real.png"),
        );
        describe(
            &mut t,
            "load_io file /real.png (overwritten)",
            file_io("/real.png"),
        );
        let mem = sdl3_image::load_io(&mut IoStream::from_const_mem(GARBAGE));
        describe(&mut t, "load_io mem (its bytes)", mem);

        // The browser's decodings are the image, exactly.
        let image = pattern();
        for path in ["/real.png", "/real.bmp", "/nofile.png"] {
            let loaded = sdl3_image::load(path).unwrap();
            assert_eq!(
                loaded.pixels(),
                image.pixels(),
                "{path} as the browser decoded it"
            );
        }
        t
    }

    struct Waiting {
        start: Instant,
        ok: bool,
    }

    thread_local! {
        static WAITING: RefCell<Option<Waiting>> = const { RefCell::new(None) };
    }

    /// Ends the program with libtest's summary.
    fn finish(ok: bool) -> ! {
        println!("\ntest result: {}.", if ok { "ok" } else { "FAILED" });
        // SAFETY: plain calls; exit() leaves through the event loop.
        unsafe { sys::emscripten_cancel_main_loop() };
        std::process::exit(if ok { 0 } else { 101 })
    }

    /// The iterations, until the browser has decoded the files.
    unsafe extern "C-unwind" fn iterate() {
        // SAFETY: plain JavaScript.
        let state = unsafe { em_asm_int!("return Module['sdl3_test_preloaded'];") };
        let (start, ok) = WAITING.with_borrow(|w| {
            let w = w.as_ref().unwrap();
            (w.start, w.ok)
        });
        if state == 0 && start.elapsed() < PATIENCE {
            return;
        }
        let ok = report("plugin", || {
            assert_eq!(state, 1, "the files weren't preloaded");
            compare("plugin", &plugin(), PLUGIN_TRACE);
        }) && ok;
        finish(ok)
    }

    pub(super) fn main() {
        let mut ok = report("standins", || {
            compare("standins", &standins(), STANDINS_TRACE)
        });
        if !in_browser() {
            ok = report("plugin", || {
                skip(
                    "browser",
                    "not in a browser (under node: run it with SDL3_EMSCRIPTEN_BROWSER=1, or with CHROME naming Chrome or Chromium)",
                )
            }) && ok;
            println!("\ntest result: {}.", if ok { "ok" } else { "FAILED" });
            std::process::exit(if ok { 0 } else { 101 });
        }
        plugin_start();
        WAITING.set(Some(Waiting {
            start: Instant::now(),
            ok,
        }));
        // SAFETY: a callback that runs from the browser's event loop; main
        // returns to it.
        unsafe { sys::emscripten_set_main_loop(Some(iterate), 0, false) };
    }
}
