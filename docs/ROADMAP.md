# Roadmap

Upstream SDL (`SDL-3.5.0`, 2026-10-01) is 1,169 C/H files and ~624,000 lines
under `src/`. This is how the translation is sequenced. Each phase produces a
building, tested crate; nothing is left half-translated across a commit.

Line counts are upstream C for the directory, to show relative size.

## Phase 1 — Foundation ✅ (this commit)

Everything that the rest of SDL `#include`s: error reporting, hints,
properties, logging, atomics, the recursive mutex and init-state machine,
timers/ticks, time, GUIDs, CRC/murmur/random, rectangles and pixel formats.

## Phase 2 — Core runtime (pure Rust, no OS APIs)

| Directory | Lines | Plan |
|---|---|---|
| `SDL.c`, `SDL_assert.c` | ~1,400 | **Done** (`sdl3::init`, `sdl3::assert`): `init`/`quit` with subsystem refcounts, app metadata, main-thread tracking; the assertion macros, handler and report. |
| `stdlib/` remainder | ~16,000 | **Done** (`sdl3::stdlib`): UTF-8 stepping, case folding (tables generated from upstream), string comparison and number parsing with upstream quirks, `iconv` with all encodings, environment snapshots. `SDL_malloc.c` (dlmalloc), `mem*`, `qsort` and `snprintf` are replaced by Rust's own. |
| `libm/` | ~3,300 | **Done** (`sdl3::stdlib::math`): fdlibm routines, bit-identical to SDL's own `SDL_sin` etc. (verified by hashing outputs against compiled upstream C). |
| `thread/` generic | ~6,000 | **Done** (`sdl3::thread`): threads, TLS, semaphore, recursive mutex, init state over `std::thread`/`std::sync`. Priorities need the platform layer. |
| `cpuinfo/` | ~1,300 | **Done** (`sdl3::cpuinfo`): `std::arch` feature detection, cache line size, cores, RAM, page size. |
| `events/` core | ~12,800 | **Done** (`sdl3::events`): event queue, event types/structs, keyboard/mouse/touch/pen state machines, scancode tables, keymaps, window/display/clipboard/drop/notification event sources. Window state is reached through the `events::window::VideoHooks` trait (with `WindowCore` holding the `SDL_Window` fields the event code touches), which the video subsystem implements later. Deferred: `SDL_quit.c` signal handlers (platform layer), cursor creation from surfaces (needs `SDL_Surface`), the platform scancode tables (`scancodes_*.h`, `SDL_keysym_to_*.c`, with their backends). |
| `io/`, `storage/` | ~5,500 | **Done** (`sdl3::io`, `sdl3::storage`): `IoStream` over files/memory, async IO with the generic threadpool backend, storage with the generic backends. io_uring/IoRing and Steam storage are Phase 4. |
| `audio/` core | ~24,400 | **Done** (`sdl3::audio`): format conversion, channel converters (generated from upstream's table by `tools/gen_audio_channel_converters.py`), the resampler (in its SIMD-build configuration), audio streams and queue, devices, binding, device threads, mixer, WAVE loader, dummy and disk drivers; verified against upstream compiled C. Platform drivers (ALSA, Pulse, WASAPI, CoreAudio…) are Phase 4. |
| `video/` software | ~33,000 of 270,000 | **Done** (`sdl3::video::surface`, `sdl3::video`): surfaces, every blitter (incl. the generated `SDL_blit_auto.c`, with the x86 SIMD kernels computed in portable code), fill, stretch, RLE, rotation, YUV conversion, BMP load/save; all checked bit for bit against upstream C. Deferred: the stb_image/miniz PNG and JPEG codecs (`SDL_stb.c` reports "SDL not built with STB image support" like a build without them), the NEON/LSX/AltiVec kernels, and `SDL_clipboard.c`/`SDL_video.c`, which belong to the video subsystem with its backends. |
| `render/software/` | ~18,000 | **Done** (`sdl3::render`): the renderer front end (`SDL_render.c`, `SDL_sysrender.h`, `SDL_yuv_sw.c`, the debug font) and the software renderer with its drawing primitives, over an owned `Surface`; checked bit for bit against upstream C. Deferred: renderers for windows (with the window texture, vsync and event coordinate conversion), which come with the video subsystem, and the GPU render state, which comes with `gpu/`. |
| `joystick/` core | ~15,000 of 60,000 | **Done** (`sdl3::joystick`, `sdl3::gamepad`): the joystick and gamepad front ends, the controller tables and mapping database (generated from upstream), mapping parsing and generation (incl. the HIDAPI/RAWINPUT/WGI generators), the Steam virtual gamepad file, the virtual and dummy drivers; checked against upstream C with a scripted virtual-joystick session. The HIDAPI and platform drivers need device access → Phase 4. |
| `sensor/` front end | ~1,000 | **Done** (`sdl3::sensor`) with the dummy driver. |
| `haptic/`, `camera/`, `dialog/`, `tray/`, `notification/`, `locale/`, `misc/`, `process/`, `main/` front-ends | ~13,000 | **Done** (`sdl3::haptic`, `sdl3::camera`, `sdl3::dialog`, `sdl3::tray`, `sdl3::notification`, `sdl3::locale`, `sdl3::misc`, `sdl3::process`, `sdl3::app`): the front ends with their dummy backends, plus the Unix backends that need only `std` (zenity dialogs, `LANG` locales, `xdg-open`, POSIX processes). `main/` is `sdl3::app` since `main` is reserved for binaries. The HIDAPI haptic driver, the camera drivers (V4L2, PipeWire…) and the D-Bus tray/notification/portal backends are Phase 4. (`filesystem/` is done too: `sdl3::filesystem`, with the POSIX operations and Unix paths.) |
| `dynapi/` | ~3,450 | Not applicable in Rust (no runtime ABI jump table); documented as intentionally omitted. |

## Phase 3 — Platform layer design

A pure-Rust SDL still has to talk to the OS. "No C code" is kept by using
Rust's FFI declarations (`extern "system"` / `extern "C"` blocks) against the
operating system's own libraries, exactly as the C code links to them. No C
is compiled or vendored; raw declarations are written in Rust, optionally via
the pure-Rust binding crates (`windows-sys`, `libc`, `wayland-sys`, `x11-dl`,
`objc2`, `ndk-sys`) which are themselves pure Rust. The repository will
decide per backend whether to use a binding crate or hand-written
declarations; either way nothing but `rustc` is needed to build.

## Phase 4 — Platform backends (largest volume)

| Area | Lines | Order |
|---|---|---|
| `video/` backends | ~210,000 | dummy & offscreen → Wayland → X11 → Windows → Cocoa → UIKit → Android → KMS/DRM → Emscripten → others |
| `render/` GPU backends | ~77,000 | OpenGL / GLES2 → Vulkan → Direct3D 11/12 → Metal → `SDL_GPU`-based renderer |
| `gpu/` | ~41,800 | Vulkan → D3D12 → Metal |
| `audio/` drivers | ~15,000 | dummy & disk → PipeWire/Pulse/ALSA → WASAPI → CoreAudio → AAudio → others |
| `joystick/` drivers + `hidapi/` | ~60,000 | Linux evdev → HIDAPI (pure-Rust hid transport) → Windows (RawInput/XInput/WGI/GameInput) → Darwin IOKit/MFI → Android |
| `core/` per-platform glue | ~26,000 | Linux (D-Bus, evdev, udev, ibus, fcitx), Windows (COM, GameInput, XInput), Android JNI, Apple, Haiku, Emscripten, GDK, PS2/PSP/Vita/3DS/N-Gage, OpenHarmony |
| `power/`, `locale/`, `filesystem/`, `dialog/`, `tray/`, `notification/`, `camera/`, `sensor/`, `haptic/`, `misc/`, `process/`, `time/`, `timer/`, `loadso/`, `main/` backends | ~30,000 | alongside the platform they belong to |
| `test/` (`SDL_test_*`) | ~6,200 | the test framework, as a `sdl3-test` crate |

## Phase 5 — Satellite libraries (same approach, each its own crate)

In dependency order, after the core each one needs exists:

1. **SDL_image** — needs surfaces + IOStream. Decoders (BMP, PNG, JPEG, GIF, WebP, AVIF, TIFF, SVG, QOI, …) translated from the upstream single-file decoders; no system `libpng`/`libjpeg`.
2. **SDL_ttf** — needs surfaces, renderer, GPU. Includes a FreeType and HarfBuzz translation or pure-Rust equivalents; the largest satellite by far.
3. **SDL_mixer** — needs audio streams. Decoders for WAV, MP3 (minimp3), OGG/Vorbis (stb_vorbis), FLAC (dr_flac), Opus, MOD/XM (libxmp), MIDI (Timidity/FluidSynth).
4. **SDL_net** — needs the event/timer core; sockets via the platform layer.
5. **SDL_rtf** — needs SDL_ttf.
6. **SDL_shadercross** — needs `gpu/`; SPIRV-Cross/DXC glue.

## Rules for every phase

* Translate whole files; keep upstream's function order inside a module.
* Carry upstream comments and `FIXME`s across.
* Pin behaviour with tests derived from upstream constants and comments.
* `cargo test`, `cargo clippy --all-targets` and `cargo doc` stay clean.
* Platform code is `#[cfg]`-gated and the crate always builds on every tier-1
  Rust target even when a backend is missing (it reports "not supported",
  exactly like an SDL built without that backend).
