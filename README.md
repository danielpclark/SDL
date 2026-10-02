# SDL in Rust

A **direct, pure-Rust translation of [Simple DirectMedia Layer](https://github.com/libsdl-org/SDL) 3**.

* 100% Rust. No C code, no `build.rs` compiling anything, no `bindgen`.
* The platform-independent core has **zero third-party dependencies**.
* The **implementation** is translated line by line: same algorithms, same
  constants and tables, same error strings, same quirks.
* The **API** is designed as a Rust library, not a mock-up of the C one:
  `Result`-based errors, typed values, closures instead of `void *userdata`,
  owned handles with `Drop`, methods on the types they belong to. Every item
  documents the C symbol it translates (`/// Translation of SDL_GetRectUnion()`).
* Tracks upstream `SDL-3.5.0`, commit `ea7a2dabfd1ab6b0a5720568ffd207f5825e47f1`
  (2026-10-01).

> **Status: early.** Phase 1 (the foundation every other subsystem is built
> on) is complete, and so is the first half of Phase 2: `SDL.c`, assertions,
> the events core, libm, the stdlib remainder, threads, CPU info, IO streams,
> async IO, the filesystem and storage. Audio, surfaces, rendering and the
> platform backends come next; see [docs/ROADMAP.md](docs/ROADMAP.md).

## What is translated so far

| Upstream | Rust module | Notes |
|---|---|---|
| `SDL.c` | `sdl3::init` | `init`/`quit`/`was_init` with `InitFlags`, subsystem refcounts, main-thread tracking, app metadata, platform/sandbox/form-factor queries |
| `SDL_version.h` | `sdl3::version` | |
| `SDL_error.c` | `sdl3::error` | `Error`/`ErrorKind`/`Result`; SDL's messages, Rust's error handling |
| `SDL_guid.c` | `sdl3::guid` | |
| `SDL_hints.c` + all 276 `SDL_HINT_*` names | `sdl3::hints` | priorities, environment overrides, closure watchers with RAII tokens |
| `SDL_properties.c` | `sdl3::properties` | `Properties` handle + typed `Value` (`Any`/string/number/float/bool), lock guard |
| `SDL_log.c` | `sdl3::log` | `Category`/`Priority` enums, `SDL_LOGGING` hint parsing, prefixes, closure output, `log::warn!`… macros |
| `SDL_utils.c` (core helpers) | `sdl3::utils` | GCD, best-rational-approximation |
| `atomic/SDL_spinlock.c` | `sdl3::atomic` | `SpinLock<T>` with a guard |
| `SDL_assert.c`, `SDL_assert.h` | `sdl3::assert` | `sdl_assert!`/`sdl_assert_release!`/`sdl_assert_paranoid!`/`sdl_assert_always!`, levels via Cargo features, closure handlers, the report, the `SDL_ASSERT` hint, breakpoints |
| `thread/SDL_thread.c`, generic sync primitives | `sdl3::thread` | `Thread`/`ThreadBuilder` (status, state, detach), `TlsId` with destructors, `Semaphore`, `ReentrantMutex<T>`, `InitState`, thread IDs |
| `cpuinfo/SDL_cpuinfo.c` | `sdl3::cpuinfo` | `CpuFeatures`, CPUID cache-line rules, core count, RAM, page size, the feature-mask hint, SIMD alignment |
| `libm/` (fdlibm) + `SDL_stdlib.c` math | `sdl3::stdlib::math` | `sin`…`pow`, `fmod`, `sqrt`, `floor`, `scalbn`, `modf`, f32 variants; bit-identical to SDL's own build, checked by hashing outputs against compiled upstream C |
| `stdlib/SDL_string.c` (UTF-8, case folding, number parsing), `SDL_iconv.c`, `SDL_getenv.c` | `sdl3::stdlib` | `step_utf8`/`codepoints`, `case_fold_unicode` (tables generated from upstream), `strcasecmp`, `strtol` family with upstream quirks, `Iconv` with all 29 encodings, `Environment` snapshots |
| `io/SDL_iostream.c` | `sdl3::io` | `IoStream` over an `IoInterface` trait: buffered files, memory, const memory, dynamic memory; endian helpers, `load_file`/`save_file`, `std::io` impls |
| `io/SDL_asyncio.c` + generic backend | `sdl3::io` | `AsyncIo`, `AsyncIoQueue<U>` (typed per-request data, buffers moved in and handed back), threadpool, `load_file_async` |
| `filesystem/SDL_filesystem.c`, POSIX fsops, Unix paths | `sdl3::filesystem` | create/remove/rename/copy, path info, enumeration with `ControlFlow`, glob (matcher checked against upstream C), base/pref/user folders via XDG |
| `storage/SDL_storage.c` + generic backend | `sdl3::storage` | `Storage` over a `StorageInterface` trait: title, user and file storage, driver hints, path validation, glob |
| `timer/SDL_timer.c` | `sdl3::timer` | ticks, `delay`, `delay_precise`, threaded timer queue behind an RAII `Timer` |
| `time/SDL_time.c` | `sdl3::time` | `Time` newtype, `DateTime`, civil-date algorithms, `SystemTime` and Windows FILETIME conversions |
| `power/SDL_power.c` | `sdl3::power` | dispatcher (no platform backends yet) |
| `stdlib/SDL_crc16.c`, `SDL_crc32.c`, `SDL_murmur3.c`, `SDL_random.c` | `sdl3::stdlib` | bit-exact, verified against published vectors; `Rng` type |
| `video/SDL_rect.c`, `SDL_rect_impl.h` | `sdl3::video::rect` | methods on `Rect`/`FRect`; int and float variants via one macro, like the C double-include |
| `video/SDL_pixels.c`, `SDL_pixels.h` | `sdl3::video::pixels` | all pixel formats and colorspaces as typed constants, masks, details, `Palette`, RGB(A) mapping, colour matrices |
| `events/SDL_events.c`, `SDL_eventwatch.c`, `SDL_events.h` | `sdl3::events` (`queue`) | `Event` enum (one variant per `SDL_Event` member, owned `String`s instead of temporary memory), `EventType` consts, `push`/`poll`/`wait`/`peek`/`flush`, filter + RAII watchers, enable/disable bitset, user event registration, poll sentinel, `run_on_main_thread`, event logging hint |
| `events/SDL_keyboard.c`, `SDL_keymap.c` | `sdl3::events::keyboard` | `Scancode`/`Keycode`/`Keymod` with every constant and name table, `Keymap` with shift-level lookup and the US QWERTY defaults, key state/modifiers/repeat/auto-release, focus, text input/editing/candidates, keycode-options hint |
| `events/SDL_mouse.c` | `sdl3::events::mouse` | devices, focus and position tracking, relative/absolute motion with scaling and integer mode, clicks and double-click counting, wheel accumulation, relative mode, capture, warping (incl. warp emulation), cursor state; all 14 mouse hints |
| `events/SDL_touch.c`, `SDL_pen.c` | `sdl3::events::touch`, `sdl3::events::pen` | touch devices and fingers, pinch; pen registry, axes, buttons, proximity (deferred proximity-out); touch⇄mouse and pen→mouse/touch emulation |
| `events/SDL_windowevents.c`, display/clipboard/drop/notification event sources | `sdl3::events::window` | `WindowFlags`, window state updates and superseded-event filtering, early/normal window watch lists, quit-on-last-window-close; the `VideoHooks` trait the video subsystem implements |

Roughly 38,000 lines of upstream C/headers are covered by about 34,000 lines
of Rust including tests. Upstream is ~624,000 lines, so this is about 6% by
volume, but it is the part that everything else includes.

Not yet translated from these files: the SIGINT/SIGTERM handlers of
`SDL_quit.c` (platform layer); cursor creation from surfaces and animated
cursors in `SDL_mouse.c` (they need `SDL_Surface`, which comes with the
software video phase); thread priorities (platform layer); the Windows
known-folder lookups and the io_uring/IoRing async backends. Parts of the C
stdlib that Rust already provides (`malloc`, `memcpy`, `qsort`, `snprintf`)
are intentionally not translated.

## Building

```sh
cargo build --release
cargo test
cargo doc --open
```

Requires Rust 1.85 or newer. There is nothing else to install.

## Using it

```rust
use std::time::Duration;
use sdl3::video::pixels::{Color, Palette, PixelFormat};
use sdl3::video::rect::Rect;
use sdl3::{hints, log, timer};

// Geometry: methods, Option/Result instead of out-parameters.
let a = Rect::new(0, 0, 10, 10);
let b = Rect::new(5, 5, 10, 10);
assert_eq!(a.intersection(&b), Some(Rect::new(5, 5, 5, 5)));

// Pixel formats: typed, with the C constants' names and values.
let fmt = PixelFormat::ARGB8888.details()?;
assert_eq!(fmt.map_rgba(None, Color::new(0x11, 0x22, 0x33, 0x44))?, 0x44112233);
let mut pal = Palette::new(256)?;
pal.dither();

// Hints, logging: closures and RAII tokens instead of callbacks + userdata.
hints::set(hints::LOGGING, "video=debug")?;
let _watch = hints::watch(hints::VIDEO_DRIVER, |change| {
    println!("video driver hint is now {:?}", change.new_value);
})?;
log::debug!(log::Category::Video, "hello {}", "world");

// Timers: a handle that cancels on drop (or `.detach()` to keep it running).
let timer = timer::Timer::new(Duration::from_millis(16), |interval| Some(interval))?;
timer::delay(Duration::from_millis(50));
drop(timer);
# Ok::<(), sdl3::Error>(())
```

[docs/TRANSLATION_GUIDE.md](docs/TRANSLATION_GUIDE.md) lists how each C
construct maps to Rust.

## License

zlib, the same as SDL. This is a derivative work (an "altered version") of
SDL and is marked as such; the original notice is preserved in
[LICENSE.txt](LICENSE.txt). [docs/LICENSING.md](docs/LICENSING.md) explains
what can and cannot change about the license of a translation.

## Satellite libraries

SDL_image, SDL_mixer, SDL_ttf, SDL_net, SDL_rtf and SDL_shadercross will be
translated the same way, each as its own crate in this workspace, after the
core they depend on exists. See the roadmap.
