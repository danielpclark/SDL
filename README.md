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
> on) is complete, and so is most of the platform-independent Phase 2:
> `SDL.c`, assertions, the events core, libm, the stdlib remainder, threads,
> CPU info, IO streams, async IO, the filesystem, storage, the audio core
> (with the dummy and disk drivers), software video (surfaces, every
> blitter, RLE, rotation, YUV conversion, BMP files), the 2D renderer
> with its software backend, and the joystick, gamepad and sensor front ends
> (with the virtual joystick driver). The remaining front ends and the
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
| `SDL_utils.c` (core helpers) | `sdl3::utils` | GCD, best-rational-approximation, device names |
| `atomic/SDL_spinlock.c` | `sdl3::atomic` | `SpinLock<T>` with a guard |
| `SDL_assert.c`, `SDL_assert.h` | `sdl3::assert` | `sdl_assert!`/`sdl_assert_release!`/`sdl_assert_paranoid!`/`sdl_assert_always!`, levels via Cargo features, closure handlers, the report, the `SDL_ASSERT` hint, breakpoints |
| `thread/SDL_thread.c`, generic sync primitives | `sdl3::thread` | `Thread`/`ThreadBuilder` (status, state, detach), `TlsId` with destructors, `Semaphore`, `ReentrantMutex<T>`, `InitState`, thread IDs |
| `cpuinfo/SDL_cpuinfo.c` | `sdl3::cpuinfo` | `CpuFeatures`, CPUID cache-line rules, core count, RAM, page size, the feature-mask hint, SIMD alignment |
| `libm/` (fdlibm) + `SDL_stdlib.c` math | `sdl3::stdlib::math` | `sin`…`pow`, `fmod`, `sqrt`, `floor`, `scalbn`, `modf`, f32 variants; bit-identical to SDL's own build, checked by hashing outputs against compiled upstream C |
| `stdlib/SDL_string.c` (UTF-8, case folding, number parsing), `SDL_iconv.c`, `SDL_getenv.c` | `sdl3::stdlib` | `step_utf8`/`codepoints`, `case_fold_unicode` (tables generated from upstream), `strcasecmp`, `strtol` family with upstream quirks, `Iconv` with all 29 encodings, `Environment` snapshots |
| `io/SDL_iostream.c` | `sdl3::io` | `IoStream` over an `IoInterface` trait: buffered files, memory, const memory, dynamic memory; endian helpers, `load_file`/`save_file`, `std::io` impls |
| `io/SDL_asyncio.c` + generic backend | `sdl3::io` | `AsyncIo`, `AsyncIoQueue<U>` (typed per-request data, buffers moved in and handed back), threadpool, `load_file_async` |
| `filesystem/SDL_filesystem.c`, POSIX fsops, Unix paths | `sdl3::filesystem` | create/remove/rename/copy, path info, enumeration with `ControlFlow`, glob (matcher checked against upstream C), base/pref/user folders via XDG |
| `audio/` core: `SDL_audio.c`, `SDL_audiocvt.c`, `SDL_audioqueue.c`, `SDL_audioresample.c`, `SDL_audiotypecvt.c`, `SDL_audio_channel_converters.h`, `SDL_mixer.c`, `SDL_wave.c`, dummy and disk drivers | `sdl3::audio` | `AudioFormat`/`AudioSpec`, `AudioStream` (conversion, resampling, gain, frequency ratio, channel maps, callbacks, no-copy and planar input), physical and logical devices with binding, device threads, postmix, default-device migration, `mix_audio`, `load_wav` (PCM, float, A-law, µ-law, MS and IMA ADPCM). Conversion, resampling, streams, mixing and WAVE decoding are checked bit-for-bit against upstream's C compiled for x86-64 |
| `storage/SDL_storage.c` + generic backend | `sdl3::storage` | `Storage` over a `StorageInterface` trait: title, user and file storage, driver hints, path validation, glob |
| `timer/SDL_timer.c` | `sdl3::timer` | ticks, `delay`, `delay_precise`, threaded timer queue behind an RAII `Timer` |
| `time/SDL_time.c` | `sdl3::time` | `Time` newtype, `DateTime`, civil-date algorithms, `SystemTime` and Windows FILETIME conversions |
| `power/SDL_power.c` | `sdl3::power` | dispatcher (no platform backends yet) |
| `stdlib/SDL_crc16.c`, `SDL_crc32.c`, `SDL_murmur3.c`, `SDL_random.c` | `sdl3::stdlib` | bit-exact, verified against published vectors; `Rng` type |
| `video/SDL_rect.c`, `SDL_rect_impl.h` | `sdl3::video::rect` | methods on `Rect`/`FRect`; int and float variants via one macro, like the C double-include |
| `video/SDL_pixels.c`, `SDL_pixels.h` | `sdl3::video::pixels` | all pixel formats and colorspaces as typed constants, masks, details, `Palette`, RGB(A) mapping, colour matrices |
| `video/SDL_surface.c`, `SDL_blit.c`, `SDL_blit_0.c`, `SDL_blit_1.c`, `SDL_blit_A.c`, `SDL_blit_N.c`, `SDL_blit_auto.c`, `SDL_blit_copy.c`, `SDL_blit_slow.c`, `SDL_fillrect.c`, `SDL_stretch.c`, `SDL_RLEaccel.c`, `SDL_rotate.c` | `sdl3::video::surface` | `Surface<'a>` owning or borrowing its pixels, shared palettes, lock guard, alternate images; blits (plain, scaled, tiled, 9-grid), conversion between all formats and colorspaces, fill, stretch, RLE acceleration, rotation, premultiplication; every specialized blitter and the generated `SDL_blit_auto.c` table. Upstream's x86 SIMD kernels (MMX/SSE/SSE2/SSE4.1/AVX2) are computed in portable code and chosen by the same CPU checks, so results match the C bit for bit on any architecture |
| `video/SDL_yuv.c`, `yuv2rgb/` | `sdl3::video` (`convert_pixels*`, YUV surfaces) | YUV⇄RGB for all 11 FOURCC formats in every supported colorspace (portable and SSE2 kernels), YUV⇄YUV repacking |
| `video/SDL_video.c`, `SDL_sysvideo.h`, `SDL_clipboard.c`, `dummy/`, `offscreen/` | `sdl3::video` (`window`, `display`, `clipboard`, `messagebox`, `textinput`, `gl`, `vulkan`) | the video core: driver selection, displays and modes (closest-mode search, mode switching, desktop bounds), `Window` handles checked against the window list, creation from properties or a `WindowBuilder`, position/size with limits and aspect ratio, show/hide with child windows, fullscreen (desktop and exclusive) across displays, grabs, window surfaces, popups and parents, text input, the clipboard with closure data callbacks, message boxes, and the OpenGL/Vulkan/Metal front ends (no loaders yet, like a build without them); the dummy and offscreen drivers. Checked against upstream's C by comparing the trace of a scripted session on both drivers |
| `video/SDL_bmp.c`, the image front end of `SDL_stb.c` | `Surface::load_bmp`/`save_bmp`/`load`, `is_bmp`/`is_png`/`is_jpg` | BMP load (1–32 bpp, bitfields, RLE4/RLE8, v1–v5 headers) and save (8-bit, 24-bit, 32-bit v5); the stb_image PNG/JPEG codecs are not translated yet |
| `render/SDL_render.c`, `SDL_sysrender.h`, `SDL_yuv_sw.c`, `SDL_render_debug_font.h`, `render/software/` (`SDL_render_sw.c`, draw/blend point, line and fill, `SDL_triangle.c`) | `sdl3::render` | `Renderer` (software renderer over an owned `Surface`) with the command queue, logical presentation, viewport, clip, scale, points/lines/rects, texture copies (plain, rotated, affine, tiled, 9-grid), geometry (incl. the software renderer's quad-to-rect path), debug text, read-back; `Texture` handles checked against the renderer and their generation; streaming, static and target textures, YUV and indexed textures through native textures, palettes, `lock_texture`/`lock_texture_to_surface` guards that upload on drop. Checked against upstream's C with randomized sessions on 16 output formats |
| `joystick/SDL_joystick.c`, `SDL_gamepad.c`, `SDL_gamepad_db.h`, `controller_type.c`, `controller_list.h`, `usb_ids.h`, `SDL_steam_virtual_gamepad.c`, `virtual/`, `dummy/` | `sdl3::joystick`, `sdl3::gamepad` | `Joystick` and `Gamepad` handles (closed on drop), the joystick state machine (axis initial-value detection, focus filtering, rumble expiry and resend, sensor fusion, player indexes), GUIDs and device classification, VID/PID lists with their hints, the mapping database (tables generated from upstream by `tools/gen_joystick_tables.py` and `tools/gen_gamepad_db.py`), mapping parsing and generation, binding to gamepad events, the virtual joystick driver with closure callbacks. Checked against upstream's C by replaying a scripted virtual-joystick session and comparing the event trace and the whole mapping database |
| `sensor/SDL_sensor.c`, `dummy/` | `sdl3::sensor` | `Sensor` handles, the driver interface, sensor events |
| `haptic/SDL_haptic.c`, `dummy/` | `sdl3::haptic` | `Haptic` handles (closed on drop), typed `HapticEffect` variants instead of the C union, effect slots, gain/autocenter/pause, the simple rumble API, the joystick haptic-axes hint |
| `camera/SDL_camera.c`, `SDL_syscamera.h`, `dummy/` | `sdl3::camera` | `Camera` handles, the device thread, spec sorting and best-match selection, frame queues with zero-copy hand-off (frames are owned buffers that return to the backend when the `CameraFrame` is dropped), conversion and scaling, permission states, zombie devices, hotplug events |
| `dialog/SDL_dialog.c`, `SDL_dialog_utils.c`, `unix/SDL_zenitydialog.c` | `sdl3::dialog` | open/save/folder dialogs with closure callbacks, filter validation and conversion, the zenity backend on Unix (the D-Bus portal comes with the platform layer) |
| `tray/SDL_tray_utils.c`, `notification/SDL_notification.c`, their `dummy/` backends | `sdl3::tray`, `sdl3::notification` | the tray and notification front ends; tray bookkeeping for quit-on-last-window-close |
| `locale/SDL_locale.c`, `unix/` | `sdl3::locale` | `Locale` list, the CSV parser (byte-exact with upstream, checked against its C), `LANG`/`LANGUAGE` on Unix |
| `misc/SDL_url.c`, `unix/` | `sdl3::misc` | `open_url` via `xdg-open` |
| `process/SDL_process.c`, `posix/` | `sdl3::process` | `Process`/`ProcessBuilder` (arguments, environment, working directory, redirections, background), pipes with non-blocking output, `kill`, `wait`, over `std::process` |
| `main/SDL_main_callbacks.c`, `generic/SDL_sysmain_callbacks.c`, `SDL_runapp.c` | `sdl3::app` | the main-callbacks driver (`AppCallbacks` trait: init/iterate/event/quit), the callback-rate hint, `run_app` |
| `events/SDL_events.c`, `SDL_eventwatch.c`, `SDL_events.h` | `sdl3::events` (`queue`) | `Event` enum (one variant per `SDL_Event` member, owned `String`s instead of temporary memory), `EventType` consts, `push`/`poll`/`wait`/`peek`/`flush`, filter + RAII watchers, enable/disable bitset, user event registration, poll sentinel, `run_on_main_thread`, event logging hint |
| `events/SDL_keyboard.c`, `SDL_keymap.c` | `sdl3::events::keyboard` | `Scancode`/`Keycode`/`Keymod` with every constant and name table, `Keymap` with shift-level lookup and the US QWERTY defaults, key state/modifiers/repeat/auto-release, focus, text input/editing/candidates, keycode-options hint |
| `events/SDL_mouse.c` | `sdl3::events::mouse` | devices, focus and position tracking, relative/absolute motion with scaling and integer mode, clicks and double-click counting, wheel accumulation, relative mode, capture, warping (incl. warp emulation), cursor state; all 14 mouse hints |
| `events/SDL_touch.c`, `SDL_pen.c` | `sdl3::events::touch`, `sdl3::events::pen` | touch devices and fingers, pinch; pen registry, axes, buttons, proximity (deferred proximity-out); touch⇄mouse and pen→mouse/touch emulation |
| `events/SDL_windowevents.c`, display/clipboard/drop/notification event sources | `sdl3::events::window` | `WindowFlags`, window state updates and superseded-event filtering, early/normal window watch lists, quit-on-last-window-close; the `VideoHooks` trait the video subsystem implements |

Roughly 129,000 lines of upstream C/headers are covered by about 91,000
lines of Rust including tests. Upstream is ~624,000 lines, so this is about
21% by volume, but it is the part that everything else includes. The audio
conversions, every blit, conversion, fill, stretch, RLE, rotation, YUV and
BMP path, and the renderer are checked against upstream's C (compiled with
its SIMD kernels on and off) by hashing the results of large randomized
scenarios; the joystick and gamepad front ends by comparing the event trace
of a scripted virtual-joystick session with upstream's. Upstream bugs found this way are kept and marked
`FIXME (upstream)`; where the C code would read or write out of bounds, the
Rust code returns an error instead.

Not yet translated from these files: the SIGINT/SIGTERM handlers of
`SDL_quit.c` (platform layer); cursor creation from surfaces and animated
cursors in `SDL_mouse.c`; renderers for windows, vsync and
`SDL_ConvertEventToRenderCoordinates()`, and the GPU render state; the
OpenGL/EGL and Vulkan loaders and the GPU-texture window framebuffer; thread priorities (platform layer); the Windows
known-folder lookups and the io_uring/IoRing async backends; the platform
audio, haptic and camera drivers and the tray, notification and portal
dialog backends; the PNG/JPEG codecs (stb_image, miniz) and the NEON, LSX and
AltiVec kernels. Parts of the C
stdlib that Rust already provides (`malloc`, `memcpy`, `qsort`, `snprintf`)
are intentionally not translated.

## Building

```sh
cargo build --release
cargo test
cargo doc --open
```

Requires Rust 1.87 or newer. There is nothing else to install.

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
