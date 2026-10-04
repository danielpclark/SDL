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
| `thread/` generic | ~6,000 | **Done** (`sdl3::thread`): threads, TLS, semaphore, recursive mutex, init state over `std::thread`/`std::sync`; priorities on pthreads, Linux (with the RealtimeKit fallback over D-Bus) and Windows. |
| `cpuinfo/` | ~1,300 | **Done** (`sdl3::cpuinfo`): `std::arch` feature detection, cache line size, cores, RAM, page size. |
| `events/` core | ~12,800 | **Done** (`sdl3::events`): event queue, event types/structs, keyboard/mouse/touch/pen state machines, scancode tables, keymaps, window/display/clipboard/drop/notification event sources. Window state is reached through the `events::window::VideoHooks` trait (with `WindowCore` holding the `SDL_Window` fields the event code touches), which the video subsystem implements later. `SDL_quit.c` with its SIGINT/SIGTERM handlers. Deferred: the platform scancode tables (`scancodes_*.h`, `SDL_keysym_to_*.c`, with their backends). |
| `io/`, `storage/` | ~5,500 | **Done** (`sdl3::io`, `sdl3::storage`): `IoStream` over files/memory, async IO with the generic threadpool backend, storage with the generic backends. io_uring/IoRing and Steam storage are Phase 4. |
| `audio/` core | ~24,400 | **Done** (`sdl3::audio`): format conversion, channel converters (generated from upstream's table by `tools/gen_audio_channel_converters.py`), the resampler (in its SIMD-build configuration), audio streams and queue, devices, binding, device threads, mixer, WAVE loader, dummy and disk drivers; verified against upstream compiled C. Platform drivers (ALSA, Pulse, WASAPI, CoreAudio…) are Phase 4. |
| `video/` software | ~33,000 of 270,000 | **Done** (`sdl3::video::surface`, `sdl3::video`): surfaces, every blitter (incl. the generated `SDL_blit_auto.c`, with the x86 SIMD kernels computed in portable code), fill, stretch, RLE, rotation, YUV conversion, BMP load/save, and the bundled stb_image PNG/JPEG decoders and miniz PNG writer (`SDL_stb.c`, including MJPG conversion); all checked bit for bit against upstream C. Deferred: the NEON/LSX/AltiVec kernels. The video core (`SDL_video.c`, `SDL_clipboard.c`) is done too, with the dummy and offscreen drivers (see Phase 4). |
| `render/software/` | ~18,000 | **Done** (`sdl3::render`): the renderer front end (`SDL_render.c`, `SDL_sysrender.h`, `SDL_yuv_sw.c`, the debug font) and the software renderer with its drawing primitives, over an owned `Surface`; checked bit for bit against upstream C. Renderers for windows are done too, with event coordinate conversion and window shapes, checked against upstream C on the dummy video driver. Deferred: the window texture (the GPU-texture window framebuffer, which also carries vsync for window surfaces) and the GPU render state, which come with `gpu/` and the GPU backends. |
| `joystick/` core | ~15,000 of 60,000 | **Done** (`sdl3::joystick`, `sdl3::gamepad`): the joystick and gamepad front ends, the controller tables and mapping database (generated from upstream), mapping parsing and generation (incl. the HIDAPI/RAWINPUT/WGI generators), the Steam virtual gamepad file, the virtual and dummy drivers; checked against upstream C with a scripted virtual-joystick session. The HIDAPI and platform drivers need device access → Phase 4. |
| `sensor/` front end | ~1,000 | **Done** (`sdl3::sensor`) with the dummy driver. |
| `haptic/`, `camera/`, `dialog/`, `tray/`, `notification/`, `locale/`, `misc/`, `process/`, `main/` front-ends | ~13,000 | **Done** (`sdl3::haptic`, `sdl3::camera`, `sdl3::dialog`, `sdl3::tray`, `sdl3::notification`, `sdl3::locale`, `sdl3::misc`, `sdl3::process`, `sdl3::app`): the front ends with their dummy backends, plus the Unix backends that need only `std` (zenity dialogs, `LANG` locales, `xdg-open`, POSIX processes). `main/` is `sdl3::app` since `main` is reserved for binaries. The HIDAPI haptic driver and the camera drivers (V4L2, PipeWire…) are Phase 4; the D-Bus tray, notification and portal dialog backends are done (see Phase 4). (`filesystem/` is done too: `sdl3::filesystem`, with the POSIX operations and Unix paths.) |
| `dynapi/` | ~3,450 | Not applicable in Rust (no runtime ABI jump table); documented as intentionally omitted. |

## Phase 3 — Platform layer design ✅

A pure-Rust SDL still has to talk to the OS. "No C code" is kept by using
Rust's FFI declarations against the operating system's own libraries,
exactly as the C code links to them; nothing ever links to or loads the C
SDL or its bundled libraries, whose code is all translated. The OS is reached
through the pure-Rust declaration crates `libc` (Unix) and `windows-sys`
(Windows). Libraries upstream loads at run time (X11, Wayland, ALSA,
PulseAudio, PipeWire, libdbus, libudev, ...) are declared by hand in the
backend that uses them and loaded with `sdl3::loadso`, like upstream's
`*_dyn.c` files, so a binary still starts on a system missing them. Only
`rustc` is needed to build.

Done so far: `loadso/`, `core/unix/` (`SDL_poll.c`, `SDL_appid.c`),
`core/windows/SDL_windows.c`, `core/linux/SDL_dbus.c` (with the dbusmenu
export) and `SDL_threadprio.c`, the `SDL_quit.c` signal handlers and thread
priorities.

## Phase 4 — Platform backends (largest volume)

| Area | Lines | Order |
|---|---|---|
| `video/` backends | ~210,000 | dummy & offscreen (**done**, with the video core, `SDL_video.c`, checked against upstream C) → Wayland → X11 (**mostly done**: `video/x11/` except GLX/EGL (`SDL_x11opengl.c`, `SDL_x11opengles.c`, waiting for the OpenGL front end) and the IBus/Fcitx input methods, screensaver inhibition and system theme over D-Bus (next); tested against Xvfb) → Windows (**done**: `video/windows/` with `core/windows/SDL_hid.c`, tested under Wine and on the Windows CI runners; deferred: WGL/EGL OpenGL contexts, DXGI, GameInput, the TSF input method UI) → Cocoa → UIKit → Android → KMS/DRM → Emscripten → others |
| `render/` GPU backends | ~77,000 | OpenGL / GLES2 → Vulkan → Direct3D 11/12 → Metal → `SDL_GPU`-based renderer |
| `gpu/` | ~41,800 | Vulkan → D3D12 → Metal |
| `audio/` drivers | ~15,000 | dummy & disk (**done**) → ALSA, PulseAudio, PipeWire, WASAPI (**done**) → CoreAudio → AAudio → others |
| `joystick/` drivers + `hidapi/` | ~60,000 | Linux evdev (**done**: udev/inotify/polling discovery, classic `js` nodes, calibration, hats, balls, sensors, rumble, the generated gamepad mapping; the Linux haptic driver with it) → HIDAPI (pure-Rust hid transport) → Windows (the `SDL_windowsjoystick.c` frame, XInput, DirectInput with its haptic driver, RawInput and WGI **done**; GameInput next) → Darwin IOKit/MFI → Android |
| `core/` per-platform glue | ~26,000 | Linux (D-Bus with the menu export, the system theme, taskbar progress, the IME layer with IBus and Fcitx, thread priorities, evdev capabilities, udev and the evdev event timestamps **done**; the console evdev keyboard/mouse reader with the KMS/DRM video driver, `SDL_ubuntu_touch.c`), Windows (COM, IMMDevice, XInput, DirectInput (`SDL_directx.h`) and `SDL_hid.c` (device notifications and the HID DLL) **done**; GameInput), Android JNI, Apple, Haiku, Emscripten, GDK, PS2/PSP/Vita/3DS/N-Gage, OpenHarmony |
| `power/`, `locale/`, `filesystem/`, `dialog/`, `tray/`, `notification/`, `camera/`, `sensor/`, `haptic/`, `misc/`, `process/`, `time/`, `timer/`, `loadso/`, `main/` backends | ~30,000 | alongside the platform they belong to (haptic: Linux and Windows DirectInput **done**; the Unix D-Bus ones **done**: the FileChooser portal dialogs, the StatusNotifierItem tray with its dbusmenu, `org.freedesktop.Notifications` and the notification portal) |
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
* Carry upstream comments and `FIXME`s across; fix upstream bugs in their
  own changes (see the translation guide).
* Pin behaviour with tests derived from upstream constants and comments.
* `cargo test`, `cargo clippy --all-targets` and `cargo doc` stay clean.
* Platform code is `#[cfg]`-gated and the crate always builds on every tier-1
  Rust target even when a backend is missing (it reports "not supported",
  exactly like an SDL built without that backend).
