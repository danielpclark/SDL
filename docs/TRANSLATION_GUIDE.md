# Translation guide

How SDL's C becomes Rust in this repository. The goal is a library that
**behaves** exactly like SDL and **reads** like Rust. The implementation is a
faithful translation; the API is not a mock-up of the C one.

## Two layers

1. **Implementation** — translated line by line from upstream: algorithms,
   control flow, constants, lookup tables, error strings, the comments and
   even the `FIXME`s. If upstream has a quirk, we have the quirk.
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

## Testing

Each module carries unit tests that pin behaviour to upstream's: known hash
vectors, the hex values from `SDL_pixels.h`, generated lookup tables, edge
cases taken from the C comments. Prefer tests that would fail if the
translation drifted from C over tests that only exercise the Rust API.
