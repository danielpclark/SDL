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
| `SDL.c`, `SDL_assert.c` | ~1,400 | `init`/`quit` with subsystem refcounts, app metadata, assertion handler. |
| `stdlib/` remainder | ~16,000 | Only what has SDL-specific semantics: `SDL_iconv` (UTF conversions), `SDL_qsort` (unused in Rust: `sort_unstable`), string helpers whose exact behaviour callers rely on (`SDL_strlcpy`, `SDL_utf8strlen`, `SDL_StepUTF8`, `SDL_UCS4ToUTF8`). `SDL_malloc.c` (dlmalloc) is replaced by Rust's allocator. |
| `libm/` | ~3,300 | fdlibm routines; translate so results are bit-identical to SDL's own `SDL_sin` etc. rather than deferring to the platform libm. |
| `thread/` generic | ~6,000 | RWLock, condition variable, TLS, thread creation/priority wrappers over `std::thread`. |
| `cpuinfo/` | ~1,300 | `std::arch` feature detection; cache line size, RAM. |
| `events/` core | ~12,800 | Event queue, event types/structs, keyboard/mouse/touch/pen state machines, scancode tables, keymaps. Pure logic; the biggest Phase 2 item. |
| `io/`, `storage/` | ~5,500 | `SDL_IOStream` over `std::fs`/memory, async IO queue, storage interface. |
| `audio/` core | ~24,400 | Audio spec, format conversion (`SDL_audiotypecvt`), resampler, channel mixing, audio streams, WAVE loader, mixer. Platform drivers (ALSA, Pulse, WASAPI, CoreAudio…) are Phase 4. |
| `video/` software | ~60,000 of 270,000 | Surfaces, blitters (`SDL_blit_*`, incl. the generated `SDL_blit_auto.c`), fill, stretch, RLE, YUV conversion, BMP load/save, clipboard/surface helpers. |
| `render/software/` | ~10,000 | Software renderer + the renderer front-end (`SDL_render.c`, `SDL_yuv_sw.c`). |
| `joystick/` core | ~15,000 of 60,000 | Joystick/gamepad front-end, mapping database parser, virtual joystick, steam virtual gamepad. HIDAPI drivers need USB access → Phase 4. |
| `haptic/`, `sensor/`, `camera/`, `dialog/`, `tray/`, `notification/`, `filesystem/`, `locale/`, `misc/`, `process/`, `main/` front-ends | ~15,000 | Interface + dummy backends. |
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
