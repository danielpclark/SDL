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
> on) is complete and tested. Windowing, audio, rendering, input and the
> platform backends come next; see [docs/ROADMAP.md](docs/ROADMAP.md).

## What is translated so far

| Upstream | Rust module | Notes |
|---|---|---|
| `SDL_version.h` | `sdl3::version` | |
| `SDL_error.c` | `sdl3::error` | `Error`/`ErrorKind`/`Result`; SDL's messages, Rust's error handling |
| `SDL_guid.c` | `sdl3::guid` | |
| `SDL_hints.c` + all 276 `SDL_HINT_*` names | `sdl3::hints` | priorities, environment overrides, closure watchers with RAII tokens |
| `SDL_properties.c` | `sdl3::properties` | `Properties` handle + typed `Value` (`Any`/string/number/float/bool), lock guard |
| `SDL_log.c` | `sdl3::log` | `Category`/`Priority` enums, `SDL_LOGGING` hint parsing, prefixes, closure output, `log::warn!`… macros |
| `SDL_utils.c` (core helpers) | `sdl3::utils` | GCD, best-rational-approximation |
| `atomic/SDL_spinlock.c` | `sdl3::atomic` | `SpinLock<T>` with a guard |
| `thread/generic/SDL_sysmutex.c`, `SDL_InitState` | internal | recursive mutex, semaphore, init-state machine (used by hints/log/timers) |
| `timer/SDL_timer.c` | `sdl3::timer` | ticks, `delay`, `delay_precise`, threaded timer queue behind an RAII `Timer` |
| `time/SDL_time.c` | `sdl3::time` | `Time` newtype, `DateTime`, civil-date algorithms, `SystemTime` and Windows FILETIME conversions |
| `power/SDL_power.c` | `sdl3::power` | dispatcher (no platform backends yet) |
| `stdlib/SDL_crc16.c`, `SDL_crc32.c`, `SDL_murmur3.c`, `SDL_random.c` | `sdl3::stdlib` | bit-exact, verified against published vectors; `Rng` type |
| `video/SDL_rect.c`, `SDL_rect_impl.h` | `sdl3::video::rect` | methods on `Rect`/`FRect`; int and float variants via one macro, like the C double-include |
| `video/SDL_pixels.c`, `SDL_pixels.h` | `sdl3::video::pixels` | all pixel formats and colorspaces as typed constants, masks, details, `Palette`, RGB(A) mapping, colour matrices |

Roughly 7,000 lines of upstream C/headers are covered by about 6,500 lines of
Rust including tests. Upstream is ~624,000 lines, so this is about 1% by
volume, but it is the 1% that everything else includes.

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
