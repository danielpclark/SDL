# Translation guide

How SDL's C becomes Rust in this repository. The goal is a library that
**behaves** exactly like SDL and **reads** like Rust. The implementation is a
faithful translation; the API is not a mock-up of the C one.

## Two layers

1. **Implementation** — translated line by line from upstream: algorithms,
   control flow, constants, lookup tables, error strings, the comments and
   even the `FIXME`s. If upstream has a quirk, we have the quirk. A real
   upstream bug is the exception: it is marked `FIXME (upstream)` at first,
   then fixed in its own change (or tracked in an issue), with the expected
   test results regenerated from the C reference patched the same way.
   Where the translation can't have the bug at all (an owned value can't
   leak, a slice can't be read past its end), the comment says
   `Note (upstream)` instead: there's nothing left to fix here.
2. **API** — designed for Rust. Nothing in the public surface exists only
   because C needed it (`bool` returns, `void *userdata`, integer handles,
   out-parameters, `NULL` sentinels, global error strings).

## Layout and naming

* `sdl3/src/<dir>/` mirrors `upstream/src/<dir>/`; `SDL_foo.c` becomes
  `foo.rs` (or `foo/mod.rs`). Public-header content (enums, structs, inline
  functions, constants) lives in the same module as its `.c`.
* Drop the `SDL_` prefix. Types stay `CamelCase`; functions become
  `snake_case` *and* move onto the type they operate on:

  | C | Rust |
  |---|------|
  | `SDL_GetRectIntersection(&a, &b, &out)` | `a.intersection(&b) -> Option<Rect>` |
  | `SDL_GetPixelFormatDetails(fmt)` | `fmt.details() -> Result<PixelFormatDetails>` |
  | `SDL_MapRGBA(details, pal, r, g, b, a)` | `details.map_rgba(pal, Color) -> Result<u32>` |
  | `SDL_GetHint(SDL_HINT_VIDEO_DRIVER)` | `hints::get(hints::VIDEO_DRIVER)` |
  | `SDL_LogWarn(SDL_LOG_CATEGORY_VIDEO, fmt, ...)` | `log::warn!(Category::Video, fmt, ...)` |
  | `SDL_GetDaysInMonth(y, m)` | `time::days_in_month(y, m) -> Result<u8>` |

* Every public item's doc comment names the C symbol it translates.

## Type mapping

| C | Rust |
|---|------|
| `bool` return + `SDL_SetError()` | `Result<T, Error>`; the `Error` carries SDL's message and an `ErrorKind` |
| `Foo *` return, `NULL` on error | `Result<Foo>` (error has a reason) or `Option<Foo>` (absence is normal) |
| `T *out` parameter(s) | return value, tuple, or small struct (`PixelMasks`, `PowerInfo`) |
| optional `const T *` parameter | `Option<&T>` |
| `const char *` in / out | `&str` in, `String` or `&'static str` out |
| `void *userdata` + function pointer | `impl Fn(...) + Send + Sync + 'static` closure |
| `void *` payload + cleanup callback | `Arc<dyn Any + Send + Sync>`; cleanup is `Drop` |
| integer handle (`SDL_TimerID`, `SDL_PropertiesID`) | owned value with `Drop` (`Timer`, `Properties`); `.detach()` where C's "runs forever" default matters |
| `Uint64` nanoseconds / `Uint32` milliseconds | `std::time::Duration` (or a newtype like `time::Time` when the epoch matters) |
| `int` enum with reserved/custom ranges | `enum` with a `Custom(n)`/`Reserved(n)` variant and `to_raw`/`from_raw` |
| `Uint32` enum whose foreign values must round-trip (`SDL_PixelFormat`) | `#[repr(transparent)]` newtype with associated consts and methods |
| `SDL_Mutex` (recursive), `SDL_InitState`, `SDL_Semaphore` | internal (`thread` module); public types use `std::sync` |
| linked lists / `SDL_HashTable` | `Vec` / `HashMap` |
| C `static` state | `static` with `Mutex`/atomics/`LazyLock`; lazily initialized, so there is no mandatory `SDL_Init` for the core |

## Behaviour rules

* Keep upstream's checks that can still fail in Rust (ranges, empty names,
  overflow guards); drop the ones Rust makes impossible (null pointers).
* Never hold a `RefCell` borrow or a `std::sync::Mutex` guard across a call
  into user code. Collect, release, then call. Where upstream relies on its
  recursive mutex to let callbacks re-enter, keep that (internally).
* When ownership forces a restructuring, keep the observable behaviour
  identical and say so in a comment.
* Reproduce upstream's generated tables with `const fn`s and test them
  against the literal values in the C file.
* When a subsystem that isn't translated yet is reached through a pointer
  (`SDL_GetVideoDevice()`, `mouse->WarpMouse`, `SDL_Window *`), define a
  trait with default "not available" implementations and a registration
  function (`events::window::VideoHooks` / `set_video`), and refer to
  objects by their id (`WindowID`). The state the translated code reads and
  writes lives in a plain struct the later subsystem embeds (`WindowCore`).
  Upstream's `if (!mouse->X)` presence checks become `supports(Feature)`.
* Subsystem state that upstream keeps in a file-level `static struct` goes
  in a `static` guarded by `Mutex` (std) or `ReentrantMutex<RefCell<_>>`
  when callbacks may re-enter; the C "lock nothing, assume one thread"
  code becomes short locked scopes with the event push outside them.

## Emscripten's JavaScript

Upstream's `EM_ASM`/`EM_JS` blocks become Rust macros of the internal
`sdl3-emscripten` crate, with the JavaScript kept verbatim in raw strings
(no `.js` file):

* Prefer a named function, defined in `sdl3-emscripten` (in the module
  named after the upstream file) and called from `sdl3`: `em_js!` with
  JavaScript parameter names, or `em_js_dollar!` to paste an `EM_ASM`
  body that uses `$0`, `$1`, … as it is. Name it `sdl3_…`. It must be in
  `sdl3-emscripten`: in `sdl3`'s own test program rustc wouldn't export
  its text.
* The anonymous `em_asm!`/`em_asm_int!`/`em_asm_double!`/`em_asm_ptr!`
  (and `main_thread_…`) compile their text with `eval` at run time: for
  tests and one-off blocks only.
* Upstream's `#if` inside JavaScript becomes a `#[cfg]` between two
  functions. A library function the JavaScript needs goes in the one
  dependency table (`sdl3_emscripten::DEPS`), not a new table.
* A Rust function upstream's JavaScript calls by name (`EMSCRIPTEN_KEEPALIVE`,
  `_Emscripten_HandlePointerEnter`) is declared with `em_js_callback!`
  and registered when its subsystem initializes, not exported.
* The C API (`html5.h`, `emscripten.h`) is in `sdl3_emscripten::sys`;
  add what a backend needs there, with its layout test. A satellite crate
  without `unsafe` (`sdl3-image`) calls it through a safe wrapper there,
  in the module named after the upstream file (`sdl3_emscripten::image`
  for `IMG.c`).
* A Rust function the event loop calls (a main loop iteration, a
  timeout, an `html5.h` event callback, a function JavaScript calls by
  name) is `extern "C-unwind"`: `exit()` there (C's, or Rust's
  `std::process::exit()`) ends in a JavaScript exception, `ExitStatus`,
  that must leave it for the event loop, where a `"C"` function would
  abort. Without threads, and without Asyncify, nothing blocks: code that
  upstream runs "later" (timers, the main loop, `FS.syncfs` callbacks)
  runs only once control is back in the event loop.

## Apple mobile platforms

Upstream's Apple platform macros become these `cfg`s, spelled out where
they are used (there is no build script to make aliases):

| C | Rust |
|---|------|
| `SDL_PLATFORM_APPLE` | `target_vendor = "apple"` |
| `SDL_PLATFORM_MACOS` | `target_os = "macos"` |
| `SDL_PLATFORM_IOS`, `SDL_VIDEO_DRIVER_UIKIT` | `all(target_vendor = "apple", not(target_os = "macos"))` |
| CMake's `IOS` | `target_os = "ios"` |
| `SDL_PLATFORM_TVOS`, `SDL_PLATFORM_VISIONOS` | `target_os = "tvos"`, `target_os = "visionos"` |
| `TARGET_OS_SIMULATOR` | `target_abi = "sim"` |
| `TARGET_OS_MACCATALYST` | `target_abi = "macabi"` |

* `SDL_PLATFORM_IOS` is `TARGET_OS_IPHONE`: every UIKit platform, tvOS,
  visionOS and Mac Catalyst too, not just `target_os = "ios"`. Upstream's
  `#if` chains test it before `SDL_PLATFORM_TVOS`, so a tvOS branch after
  it is never taken; the translation keeps the order (`FIXME (upstream)`
  where that is a bug, as `SDL_GetPlatform()`'s "tvOS").
* The simulators are told by `target_abi`, not `target_env`, which the
  minimum supported Rust leaves empty there.
* What is built follows upstream's CMake build for the platform (as for
  macOS and Emscripten): on iOS the UIKit pieces, the Apple-wide ones
  (`core/apple`, the filesystem, locale, log, time and timer backends, the
  GPU API's Metal backend) and the dummy ones upstream uses there; a
  macOS-only backend (Cocoa, IOKit, LaunchServices) is
  `target_os = "macos"`, and so is an Apple-wide one until its UIKit half
  is translated (CoreAudio, MFi, the Metal renderer, CoreMedia, the
  notifications). Until then a subsystem falls back to what else is built,
  as upstream would without the piece (the offscreen and dummy video
  drivers, the dummy audio driver...). The UIKit pieces that backends
  other than the video driver share (UIDevice, UIApplication, the form
  factor, which upstream keeps in the video driver) are in
  `core/apple/uikit`, which links UIKit.
* `@available(macOS 11.0, iOS 14.0, tvOS 14.0, *)` is
  `available!(macos(11, 0), ios(14, 0), tvos(14, 0))`
  (`core::apple::foundation`), checked at run time; a platform it doesn't
  name passes. SDK guards (`__IPHONE_OS_VERSION_MAX_ALLOWED >= 170000`) are
  compiled in, as if upstream were built with the newest SDK.
* An Objective-C method declared at run time takes its type encoding
  through `objc::encoding()`, which writes `BOOL` as the platform's (`c`
  on x86-64 macOS and Mac Catalyst, `B` elsewhere, the x86-64 Simulator
  too).
* The frameworks are linked as upstream links them, but for those a
  system the program may start on can lack (GameController and
  CoreHaptics, UniformTypeIdentifiers), which are loaded at run time.
* `SDL_RunApp()` is `UIApplicationMain()`, which never returns: the main
  function, a closure on `run_app()`'s frame, is reached through a
  pointer in a thread-local variable, so it needs no `'static` bound. As
  upstream, the app keeps running when it returns (the main callbacks
  rely on that), and the process ends with `exit()`. The UIKit classes
  are declared at run time under upstream's names, as the Cocoa driver's
  are; their methods are `extern "C"`, so a panic in one (the main
  function included) aborts. The UIKit driver only starts in an app, on
  its main thread (`Note`): a program run as a plain process of a
  simulated device gets the next driver. Upstream's `@try` around
  loading the launch storyboard or nib becomes a check that the bundle
  has it (`Note (upstream)`): Objective-C exceptions can't be caught.

## Testing

Each module carries unit tests that pin behaviour to upstream's: known hash
vectors, the hex values from `SDL_pixels.h`, generated lookup tables, edge
cases taken from the C comments. Prefer tests that would fail if the
translation drifted from C over tests that only exercise the Rust API.
