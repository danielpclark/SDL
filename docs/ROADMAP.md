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
| `haptic/`, `camera/`, `dialog/`, `tray/`, `notification/`, `locale/`, `misc/`, `process/`, `main/` front-ends | ~13,000 | **Done** (`sdl3::haptic`, `sdl3::camera`, `sdl3::dialog`, `sdl3::tray`, `sdl3::notification`, `sdl3::locale`, `sdl3::misc`, `sdl3::process`, `sdl3::app`): the front ends with their dummy backends, plus the Unix backends that need only `std` (zenity dialogs, `LANG` locales, `xdg-open`, POSIX processes). `main/` is `sdl3::app` since `main` is reserved for binaries. The camera drivers are Phase 4 (V4L2, PipeWire and Media Foundation are done); the HIDAPI and Windows DirectInput haptic drivers and the D-Bus tray, notification and portal dialog backends are done (see Phase 4). (`filesystem/` is done too: `sdl3::filesystem`, with the POSIX operations and Unix paths.) |
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
| `video/` backends | ~210,000 | dummy & offscreen (**done**, with the video core, `SDL_video.c`, checked against upstream C, and the offscreen driver's EGL contexts, `SDL_offscreenopengles.c`, tested with Mesa's llvmpipe, and its headless Vulkan surfaces, `SDL_offscreenvulkan.c`, tested with Mesa's lavapipe) → Wayland (**done**: `video/wayland/` including EGL (`SDL_waylandopengles.c`, on the OpenGL front end and `SDL_egl.c`), with protocol bindings generated from the XML by `tools/gen_wayland_protocols.py`, a `wl_shm` window framebuffer in place of the GLES one, and the D-Bus IME, system theme and taskbar progress hooked up; tested against headless sway and weston, EGL with Mesa's software rasterizer) → X11 (**mostly done**: `video/x11/` including GLX/EGL (`SDL_x11opengl.c`, `SDL_x11opengles.c`, on the OpenGL front end and `SDL_egl.c`, tested with Mesa's llvmpipe on Xvfb); the D-Bus screensaver inhibition, system theme and taskbar progress are hooked up, and text input uses XIM as upstream; tested against Xvfb) → Windows (**done**: `video/windows/` with `core/windows/SDL_hid.c` and WGL/EGL (`SDL_windowsopengl.c`, `SDL_windowsopengles.c`, tested with Mesa's llvmpipe under Wine), tested under Wine and on the Windows CI runners, with GameInput keyboards and mice (`SDL_windowsgameinput.cpp`) **done**; deferred: DXGI, the TSF input method UI) → Cocoa → UIKit → Android → KMS/DRM → Emscripten → others |
| `render/` GPU backends | ~77,000 | OpenGL / GLES2 (OpenGL **done**: `render/opengl/`, `SDL_render_gl.c` and `SDL_shaders_gl.c`, the "opengl" driver first in the driver list as upstream; tested on Mesa's llvmpipe through offscreen EGL, GLX on Xvfb and WGL under Wine against the software renderer's output. GLES2 **done**: `render/opengles2/`, the "opengles2" driver through the OpenGL front end, tested against the software renderer on the offscreen driver's EGL with Mesa's llvmpipe; not yet: wrapping existing GL textures, the Emscripten VBO path and the OpenGL ES 3 shader variants) → Vulkan (**done**: `render/vulkan/`, `SDL_render_vulkan.c` and `SDL_shaders_vulkan.c` with the SPIR-V converted by `tools/gen_vulkan_shaders.py`, the "vulkan" driver after "opengles2" as upstream, tested against the software renderer on Mesa's lavapipe through the offscreen driver's headless surfaces; not yet: the Android hardware buffer textures and YCbCr pipelines, the creation options for existing Vulkan objects) → Direct3D 11 (**done**: `render/direct3d11/`, `SDL_render_d3d11.c` and `SDL_shaders_d3d11.c` with the DXBC converted by `tools/gen_d3d11_shaders.py`, the "direct3d11" driver first on Windows as upstream, with the COM declarations checked against a mingw-w64 C harness; tested against the software renderer under Wine's d3d11 on Mesa, and on a GPU by a hardware check; not yet: the creation options for existing textures) → `SDL_GPU`-based renderer (**done**: `render/gpu/`, `SDL_render_gpu.c`, `SDL_pipeline_gpu.c` and `SDL_shaders_gpu.c` with the SPIR-V converted by `tools/gen_gpu_render_shaders.py`, the "gpu" driver after "vulkan" as upstream, on the GPU API's Vulkan backend; tested against the software renderer on Mesa's lavapipe through the offscreen driver's headless surfaces, under the validation layer; not yet: the GPU render states, the creation options for an existing device, shader formats and textures, and the DXIL/MSL shaders, which come with the GPU API's Direct3D 12 and Metal backends) → Direct3D 12 → Metal |
| `gpu/` | ~41,800 | front end (**done**: `sdl3::gpu`, `SDL_gpu.c` and `SDL_sysgpu.h`, with the debug-mode validation, the backend interface and the shared blit helpers; the format tables checked against upstream's C, the dispatch and validation over a mock backend; not yet: the OpenXR functions) → Vulkan (**done**: `gpu/vulkan/`, the "vulkan" driver: devices, the memory allocator with defragmentation, buffers, transfer buffers, textures, samplers, SPIR-V shaders, descriptor layouts and pools, graphics and compute pipelines, command buffers, render, compute and copy passes, blits and mipmaps, swapchains on the video drivers' Vulkan surfaces, submission and fences, tested on Mesa's lavapipe under the validation layer; not yet: the OpenXR parts) → D3D12 (**done**: `gpu/d3d12/`, the "direct3d12" driver before "vulkan" on Windows: devices with the debug layers, the format tables, descriptor heaps, buffers, transfer and uniform buffers, textures, samplers, DXBC/DXIL shaders, root signatures, graphics and compute pipelines, the blit shaders from `tools/gen_d3d12_shaders.py`, command buffers, barriers, render, compute and copy passes, blits and mipmaps, DXGI swapchains, submission and fences; checked against a mingw-w64 C harness, tested under Wine's vkd3d with DXBC shaders (DXIL pipelines where there is shader model 6, as on WARP); not yet: the OpenXR and Xbox parts) → Metal |
| `audio/` drivers | ~15,000 | dummy & disk (**done**) → ALSA, PulseAudio, PipeWire, WASAPI (**done**) → CoreAudio → AAudio → others |
| `joystick/` drivers + `hidapi/` | ~60,000 | Linux evdev (**done**: udev/inotify/polling discovery, classic `js` nodes, calibration, hats, balls, sensors, rumble, the generated gamepad mapping; the Linux haptic driver with it) → HIDAPI (**done**: `sdl3::hidapi` with the Linux hidraw and Windows backends, the HIDAPI joystick framework with rumble thread, combined Joy-Cons and report descriptors, and the Xbox 360/360W/360 Big Button/One, GIP, PS3, PS4, PS5, Switch/Joy-Con/classic, Switch 2 (without its libusb setup), Wii, GameCube, Luna, SHIELD, Stadia, 8BitDo, ZUIKI, Flydigi, SInput, GameSir, Logitech wheels (lg4ff) and Valve (Steam Controller, Steam HORI, Steam Deck, Steam Triton) drivers, and the HIDAPI haptic driver for the Logitech wheels; next: the libusb/macOS backends) → Windows (the `SDL_windowsjoystick.c` frame, XInput, DirectInput with its haptic driver, RawInput, WGI and GameInput (`gdk/SDL_gameinputjoystick.cpp`) **done**) → Darwin IOKit/MFI → Android |
| `core/` per-platform glue | ~26,000 | Linux (D-Bus with the menu export, the system theme, taskbar progress, the IME layer with IBus and Fcitx, thread priorities, evdev capabilities, udev and the evdev event timestamps **done**; the console evdev keyboard/mouse reader with the KMS/DRM video driver, `SDL_ubuntu_touch.c`), Windows (COM, IMMDevice, XInput, DirectInput (`SDL_directx.h`) and `SDL_hid.c` (device notifications and the HID DLL) and GameInput (`SDL_gameinput.cpp` with Microsoft's loader) **done**), Android JNI, Apple, Haiku, Emscripten, GDK, PS2/PSP/Vita/3DS/N-Gage, OpenHarmony |
| `power/`, `locale/`, `filesystem/`, `dialog/`, `tray/`, `notification/`, `camera/`, `sensor/`, `haptic/`, `misc/`, `process/`, `time/`, `timer/`, `loadso/`, `main/` backends | ~30,000 | alongside the platform they belong to (haptic: Linux, Windows DirectInput and HIDAPI (Logitech wheels) **done**; camera: V4L2, PipeWire and Media Foundation **done**, CoreMedia, Android, Emscripten, Vita and OpenHarmony not translated; the Unix D-Bus ones **done**: the FileChooser portal dialogs, the StatusNotifierItem tray with its dbusmenu, `org.freedesktop.Notifications` and the notification portal) |
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
