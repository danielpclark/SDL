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
| `render/software/` | ~18,000 | **Done** (`sdl3::render`): the renderer front end (`SDL_render.c`, `SDL_sysrender.h`, `SDL_yuv_sw.c`, the debug font) and the software renderer with its drawing primitives, over an owned `Surface`; checked bit for bit against upstream C. Renderers for windows are done too, with event coordinate conversion and window shapes, checked against upstream C on the dummy video driver. The GPU render state is done with the GPU renderer (see the GPU backends). Deferred: the window texture (the GPU-texture window framebuffer, which also carries vsync for window surfaces), which comes with `gpu/` and the GPU backends. |
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
| `render/` GPU backends | ~77,000 | OpenGL / GLES2 (OpenGL **done**: `render/opengl/`, `SDL_render_gl.c` and `SDL_shaders_gl.c`, the "opengl" driver first in the driver list as upstream; tested on Mesa's llvmpipe through offscreen EGL, GLX on Xvfb and WGL under Wine against the software renderer's output. GLES2 **done**: `render/opengles2/`, the "opengles2" driver through the OpenGL front end, tested against the software renderer on the offscreen driver's EGL with Mesa's llvmpipe; not yet: wrapping existing GL textures, the Emscripten VBO path and the OpenGL ES 3 shader variants) → Vulkan (**done**: `render/vulkan/`, `SDL_render_vulkan.c` and `SDL_shaders_vulkan.c` with the SPIR-V converted by `tools/gen_vulkan_shaders.py`, the "vulkan" driver after "opengles2" as upstream, tested against the software renderer on Mesa's lavapipe through the offscreen driver's headless surfaces; not yet: the Android hardware buffer textures and YCbCr pipelines, the creation options for existing Vulkan objects) → Direct3D 11 (**done**: `render/direct3d11/`, `SDL_render_d3d11.c` and `SDL_shaders_d3d11.c` with the DXBC converted by `tools/gen_d3d11_shaders.py`, the "direct3d11" driver first on Windows as upstream, with the COM declarations checked against a mingw-w64 C harness; tested against the software renderer under Wine's d3d11 on Mesa, and on a GPU by a hardware check; not yet: the creation options for existing textures) → `SDL_GPU`-based renderer (**done**: `render/gpu/`, `SDL_render_gpu.c`, `SDL_pipeline_gpu.c` and `SDL_shaders_gpu.c` with the SPIR-V and DXIL converted by `tools/gen_gpu_render_shaders.py`, the "gpu" driver after "vulkan" as upstream, on the GPU API's Vulkan backend and on Windows its Direct3D 12 backend; tested against the software renderer on Mesa's lavapipe through the offscreen driver's headless surfaces, under the validation layer, and on Direct3D 12 on the Windows video driver where there is shader model 6, as on WARP; with the GPU render states (custom fragment shaders with their bindings and uniforms), renderers on an existing device or with the application's shader formats, and textures wrapping existing GPU textures, tested against expected pixels with SPIR-V and DXIL test shaders; not yet: GPU renderers without a window, and the MSL shaders, which come with the GPU API's Metal backend) → Direct3D 12 (**done**: `render/direct3d12/`, `SDL_render_d3d12.c` and `SDL_shaders_d3d12.c` with the DXIL and root signatures converted by `tools/gen_d3d12_render_shaders.py`, the "direct3d12" driver after "direct3d11" on Windows as upstream, on the GPU API's Direct3D 12 declarations; tested against the software renderer on Direct3D 12 devices with shader model 6, as WARP on Windows CI (Wine's vkd3d has no DXIL: skipped there); not yet: the Xbox (GDK) parts and the creation options for existing textures) → Metal |
| `gpu/` | ~41,800 | front end (**done**: `sdl3::gpu`, `SDL_gpu.c` and `SDL_sysgpu.h`, with the debug-mode validation, the backend interface and the shared blit helpers; the format tables checked against upstream's C, the dispatch and validation over a mock backend; not yet: the OpenXR functions) → Vulkan (**done**: `gpu/vulkan/`, the "vulkan" driver: devices, the memory allocator with defragmentation, buffers, transfer buffers, textures, samplers, SPIR-V shaders, descriptor layouts and pools, graphics and compute pipelines, command buffers, render, compute and copy passes, blits and mipmaps, swapchains on the video drivers' Vulkan surfaces, submission and fences, tested on Mesa's lavapipe under the validation layer; not yet: the OpenXR parts) → D3D12 (**done**: `gpu/d3d12/`, the "direct3d12" driver before "vulkan" on Windows: devices with the debug layers, the format tables, descriptor heaps, buffers, transfer and uniform buffers, textures, samplers, DXBC/DXIL shaders, root signatures, graphics and compute pipelines, the blit shaders from `tools/gen_d3d12_shaders.py`, command buffers, barriers, render, compute and copy passes, blits and mipmaps, DXGI swapchains, submission and fences; checked against a mingw-w64 C harness, tested under Wine's vkd3d with DXBC shaders (DXIL pipelines where there is shader model 6, as on WARP); not yet: the OpenXR and Xbox parts) → Metal |
| `audio/` drivers | ~15,000 | dummy & disk (**done**) → ALSA, PulseAudio, PipeWire, WASAPI (**done**) → CoreAudio → AAudio → others |
| `joystick/` drivers + `hidapi/` | ~60,000 | Linux evdev (**done**: udev/inotify/polling discovery, classic `js` nodes, calibration, hats, balls, sensors, rumble, the generated gamepad mapping; the Linux haptic driver with it) → HIDAPI (**done**: `sdl3::hidapi` with the Linux hidraw and Windows backends, the HIDAPI joystick framework with rumble thread, combined Joy-Cons and report descriptors, and the Xbox 360/360W/360 Big Button/One, GIP, PS3, PS4, PS5, Switch/Joy-Con/classic, Switch 2 (without its libusb setup), Wii, GameCube, Luna, SHIELD, Stadia, 8BitDo, ZUIKI, Flydigi, SInput, GameSir, Logitech wheels (lg4ff) and Valve (Steam Controller, Steam HORI, Steam Deck, Steam Triton) drivers, and the HIDAPI haptic driver for the Logitech wheels; next: the libusb/macOS backends) → Windows (the `SDL_windowsjoystick.c` frame, XInput, DirectInput with its haptic driver, RawInput, WGI and GameInput (`gdk/SDL_gameinputjoystick.cpp`) **done**) → Darwin IOKit/MFI → Android |
| `core/` per-platform glue | ~26,000 | Linux (D-Bus with the menu export, the system theme, taskbar progress, the IME layer with IBus and Fcitx, thread priorities, evdev capabilities, udev and the evdev event timestamps **done**; the console evdev keyboard/mouse reader with the KMS/DRM video driver, `SDL_ubuntu_touch.c`), Windows (COM, IMMDevice, XInput, DirectInput (`SDL_directx.h`) and `SDL_hid.c` (device notifications and the HID DLL) and GameInput (`SDL_gameinput.cpp` with Microsoft's loader) **done**), Android JNI, Apple, Haiku, Emscripten, GDK, PS2/PSP/Vita/3DS/N-Gage, OpenHarmony |
| `power/`, `locale/`, `filesystem/`, `dialog/`, `tray/`, `notification/`, `camera/`, `sensor/`, `haptic/`, `misc/`, `process/`, `time/`, `timer/`, `loadso/`, `main/` backends | ~30,000 | alongside the platform they belong to (haptic: Linux, Windows DirectInput and HIDAPI (Logitech wheels) **done**; camera: V4L2, PipeWire and Media Foundation **done**, CoreMedia, Android, Emscripten, Vita and OpenHarmony not translated; the Unix D-Bus ones **done**: the FileChooser portal dialogs, the StatusNotifierItem tray with its dbusmenu, `org.freedesktop.Notifications` and the notification portal) |
| `test/` (`SDL_test_*`) | ~6,200 | **Done**: the test framework, as the `sdl3-test` crate (assert, common, compare, crc32, font, fuzzer, harness, log, md5, memory, with the memory tracker as a global allocator); the fuzzer, execution keys, test order and logs checked against upstream's C, and upstream's `testautomation_sdltest.c` run through the harness. Not yet: the D3D9/DXGI indices of `--info modes` |

## Phase 5 — Satellite libraries (same approach, each its own crate)

In dependency order, after the core each one needs exists:

1. **SDL_image** — needs surfaces + IOStream. Decoders (BMP, PNG, JPEG, GIF, WebP, AVIF, TIFF, SVG, QOI, …) translated from the upstream single-file decoders; no system `libpng`/`libjpeg`.
   **Mostly done** (`sdl3-image`, SDL_image 3.5.0): the front end
   (`IMG.c`: `IMG_Version`, `IMG_Load*`, `IMG_LoadTyped_IO`,
   `IMG_LoadTexture*`, `IMG_Save`/`IMG_SaveTyped_IO`,
   `IMG_GetClipboardImage`, `IMG_LoadAnimation*`, `IMG_SaveAnimation*`,
   `IMG_CreateAnimatedCursor`), every `IMG_is*` detector, the AVIF
   (`IMG_avif.c` over a translation of libavif 1.1.1's decoder as
   SDL_image builds it: the ISOBMFF/HEIF parser with items, grids, alpha
   items and tracks, the plain-C YUV to RGB conversion and alpha, the
   libyuv plane scaler libavif bundles, and its dav1d codec), BMP/ICO/CUR,
   GIF, LBM, PCX, PNM, QOI
   (`qoi.h`), SVG (the bundled nanosvg parser and rasterizer, also at a
   chosen size), TGA, WebP (`IMG_webp.c` over a translation of libwebp
   1.3.2's VP8 and VP8L decoders, alpha plane decoder, plain-C DSP
   functions and demuxer, with `xmlman.c` for the XMP metadata, and of its
   VP8 and VP8L encoders, alpha plane encoder, muxer and animation encoder
   for `IMG_SaveWEBP_IO()` and the WebP animation encoder, whose output is
   byte-identical to libwebp's plain-C build), TIFF
   (`IMG_tif.c` over a translation of libtiff 4.7.2's reading path:
   directories, strips and tiles, `TIFFReadRGBAImageOriented()` with every
   photometric interpretation it takes, and the codecs SDL_image builds
   libtiff with: PackBits, LZW and the predictor, CCITT RLE/RLEW/Group 3/
   Group 4, ThunderScan, NeXT and SGI LogL/LogLuv), JPEG XL (`IMG_jxl.c`
   over a translation of libjxl 0.7.3's decoder: headers, ANS and prefix
   codes, modular with every transform, VarDCT with every transform size,
   patches, splines, noise, the render pipeline with upsampling, the loop
   filters, blending, color conversion and orientation), XCF, XPM
   (with its color table, also from arrays) and XV decoders, PNG and JPEG through the stb_image translation in `sdl3`
   (`IMG_stb.c`), the BMP, ICO, CUR, GIF (LZW, octree quantizer), PNG
   (miniz), TGA, JPEG (`tiny_jpeg.h`) and WebP savers, the animation API
   (`IMG_anim_decoder.c`, `IMG_anim_encoder.c`: frame-by-frame decoders and
   encoders with timebases and metadata) with GIF, ANI cursor and WebP
   animations and AVIF image sequences (decoding),
   and `IMG_gpu.c` (GPU textures through a copy pass); checked against
   upstream's C on its test images and synthetic ones (detection, loading,
   truncated and corrupted input, saving, animation decoding and encoding).
   AVIF's AV1 decoder is a translation of dav1d 1.2.1 (the
   revision SDL_image's external/dav1d pins; plain-C paths, one thread,
   a frame delay of one), bit-exact with dav1d's C on AV1 streams from
   libaom, SVT-AV1 and rav1e and their truncated and corrupted variants;
   AVIF decoding is bit-exact with libavif and dav1d's C on synthetic
   images covering its bit depths, layouts, matrices, alpha, grids,
   transformations, scaling and sequences, truncated and corrupted.
   JPEG XL is a translation of libjxl 0.7.3's decoder (the revision
   SDL_image's external/libjxl pins; Highway's scalar target, one
   thread), bit-exact with SDL_image built with libjxl on synthetic
   images covering modular and VarDCT, every sample type, alpha, the
   image features, orientations, color encodings, progressive passes,
   recompressed JPEGs and animations, truncated and corrupted.
   APNG animations are a translation of `IMG_libpng.c`'s decoder and
   encoder over libpng 1.6.59's sequential reader and writer and zlib
   1.3.1's inflate and deflate (the revisions SDL_image's external/
   pins), byte-identical to SDL_image built with them on synthetic APNGs
   covering every color type and bit depth, the dispose and blend
   operations, offsets, interlacing, delays and metadata, truncated and
   corrupted, and on every test image re-encoded.
   Not yet: AVIF saving and the AVIF animation encoder (an AV1 encoder);
   still PNGs and JPEG stay on stb_image (and PNG saving on miniz) as
   upstream's stb backend.
2. **SDL_ttf** — needs surfaces, renderer, GPU. Includes a FreeType and HarfBuzz translation or pure-Rust equivalents; the largest satellite by far.
   **Done** (`sdl3-ttf`, SDL_ttf 3.2.2 with its bundled FreeType
   2.13.2 and HarfBuzz 8.5.0, without PlutoSVG): all of `SDL_ttf.c`
   (fonts, sizes, styles, outlines, hinting, kerning, text shaping,
   metrics, fallback fonts, measuring, wrapping, rendering in every mode,
   glyph images, text objects), the surface, renderer (with
   `stb_rect_pack.h`) and GPU text engines, and the FreeType modules for
   TrueType and OpenType/CFF fonts: the base layer (with `ftglyph.c`, `ftstroke.c`,
   `ftbitmap.c`, `ftlcdfil.c`, `ftadvanc.c`), `sfnt` (with WOFF, color
   tables, embedded bitmaps), `truetype` (the v40 bytecode interpreter,
   GX/OpenType variations), `cff` (CFF, CFF2 variable and bare CFF fonts),
   `psaux` (its CFF parts and Adobe's CFF engine), `pshinter` (its global
   hints), `psnames`, `autofit` (with the CJK and Indic
   writing systems), `smooth`, `raster`, `sdf` (signed distance fields
   from outlines and bitmaps), `gzip` (with its zlib), and the drivers
   of FreeType's other formats: `pfr` (PFR outlines, bitmaps and
   kerning), `winfonts` (Windows FNT and FON), `pcf` (with `lzw` and the
   `bzip2` stub SDL_ttf builds) and `bdf`. The
   FreeType License (`sdl3-ttf/FTL.TXT`) applies to `src/freetype/`.
   **Part 2 done**: HarfBuzz as SDL_ttf's build compiles it
   (`src/harfbuzz/`, under HarfBuzz's Old MIT license in
   `sdl3-ttf/HARFBUZZ-COPYING`, its Unicode data tables under the Unicode
   License V3): the buffer, the UCD Unicode functions, the OpenType
   layout engine (GSUB, GPOS, GDEF, feature variations, the `kern`
   table), script/language tags and feature selection, the OpenType
   shaper with every shaper it selects (default, Arabic with fallback
   shaping, Hebrew, Indic, Khmer, Myanmar and Zawgyi, Thai/Lao, Hangul,
   USE) with normalization and fallback positioning, `hb-ft`, and the
   OpenType font functions FreeType's auto-hinter uses (SDL_ttf builds
   its FreeType with HarfBuzz, so the auto-hinter finds the glyphs of
   OpenType features through it); the
   generated tables come from HarfBuzz's via
   `tools/gen_harfbuzz_tables.py`. SDL_ttf's `TTF_USE_HARFBUZZ` paths are
   on, as upstream's default build has them (font and text direction,
   script and language, `TTF_GetGlyphScript`). Checked against upstream
   (SDL_ttf with HarfBuzz, and HarfBuzz itself on FreeType) on subsets of
   Noto fonts: shaped glyphs and positions, rendered surfaces and text
   layouts for Latin ligatures and kerning, Arabic, Hebrew, mixed
   direction, Devanagari, Thai, Hangul, Khmer, Myanmar and Sinhala, and
   fonts with corrupted or truncated GSUB/GPOS/GDEF, and unhinted fonts
   (the auto-hinter with HarfBuzz); HarfBuzz alone was
   also checked on more fonts and scripts (the other Indic scripts,
   Tibetan, Balinese, Javanese, Mongolian, N'Ko, Syriac, Tai Tham,
   Tifinagh, Lao), and on byte flips of every layout-table byte of the
   test fonts.
   Checked against upstream's C (built the same way) on subsets of DejaVu
   fonts made by `tools/gen_sdl_ttf_testdata.py` (and CFF, CFF2 and
   bare CFF versions of one, hinted by the AFDKO's otfautohint): metrics, every render and
   hinting mode, styles, outlines, wrapping, SDF rendering, text objects
   drawn by the three engines (the GPU one on a Vulkan device), and truncated fonts and
   fonts with flipped bytes; FreeType alone was also checked against
   upstream's on more fonts and scripts, and on every truncation and byte
   flip of two small fonts. Not yet: HarfBuzz's AAT layout (`morx`,
   `kerx`, `trak`) and state-machine `kern` subtables, and FreeType's other
   font drivers (`type1`, `cid`, `type42`, with the Type 1 parts of
   `psaux` and the hinter of `pshinter` that only their old interpreters
   use) and its `svg` renderer (which needs PlutoSVG).
3. **SDL_mixer** — needs audio streams. Decoders for WAV, MP3 (minimp3), OGG/Vorbis (stb_vorbis), FLAC (dr_flac), Opus, MOD/XM (libxmp), MIDI (Timidity/FluidSynth).
   **Mostly done** (`sdl3-mixer`, SDL_mixer 3.3.0): the mixer
   (`SDL_mixer.c`: mixers on devices or generating into buffers, audio
   loading and predecoding, tracks with gains, frequency ratios, fades,
   loops, stereo and 3D positioning (`SDL_mixer_spatialization.c`, VBAP),
   tags, groups and every callback, `MIX_AudioDecoder`), the metadata
   parsers (`SDL_mixer_metadata_tags.c`: ID3v1/v2, APE, Lyrics3,
   MusicMatch, Ogg comments with loop points), and the self-contained
   decoders: WAV (PCM, float, mu-law, a-law, MS and IMA ADPCM, `smpl`
   loops), AIFF/AIFF-C, VOC, AU, raw PCM, the sine wave, MP3 (`dr_mp3.h`
   with minimp3), Ogg Vorbis (`stb_vorbis.h`, with SDL_mixer's patches)
   and FLAC and Ogg FLAC (`dr_flac.h`); checked against upstream's C on
   test audio from `tools/gen_sdl_mixer_testdata.py` (decoding in three
   formats, truncated and corrupted input, Ogg packets corrupted in intact
   pages, mixing with loops and seeks, and the mixer features).
   MIDI through the bundled TiMidity (`decoder_timidity.c`, behind the
   `timidity` feature, on by default as upstream's SDLMIXER_MIDI_TIMIDITY
   is) is done: `src/timidity/` is translated in its own crate,
   `sdl3-mixer-timidity`, which keeps TiMidity's license
   (`Artistic-1.0-Perl OR LGPL-2.1-only`; `default-features = false`
   leaves it out for a purely zlib-licensed `sdl3-mixer`). It reads
   `timidity.cfg` files, GUS patches and SoundFonts, and is checked
   bit-for-bit against upstream's C on synthetic patches, a SoundFont and
   MIDI files from `tools/gen_timidity_testdata.py` (eight output formats,
   seeks, truncated and corrupted files and broken patches, and through
   the mixer). Not translated, as they need an external library: Opus
   (libopusfile), libxmp, FluidSynth, WavPack, libgme, and the libmpg123,
   libvorbisfile and libFLAC decoders.
4. **SDL_net** — needs the event/timer core; sockets via the platform layer.
   **Done** (`sdl3-net`, SDL_net 3.2.0): all of `SDL_net.c` — `NET_Init`/
   `NET_Quit`, hostname resolution on resolver threads (a pool of two to
   ten, with simulated resolution loss), reference-counted addresses with
   their strings, bytes and comparison, stream sockets (clients connecting
   without blocking, servers accepting, writes queued and pumped, reads,
   disconnects, simulated lag), datagram sockets (unicast, broadcast and its
   IPv6 multicast stand-in, queued sends, the recent-sender address cache,
   simulated loss), `NET_WaitUntilInputAvailable` (with upstream's
   `select()`-based poll on Windows), and the network interface list with
   change monitoring (netlink on Linux and Android, `getifaddrs` and
   `PF_ROUTE` on the BSDs and Apple platforms, `GetAdaptersAddresses` and
   `NotifyIpInterfaceChange` on Windows), over BSD sockets through `libc`
   and WinSock through `windows-sys`. Tested over loopback only (IPv4, and
   IPv6 where the system has it, else skipped as `ipv6`), on Linux and
   under Wine, with the system's error strings, status codes and address
   formats checked against upstream's C (SDL_net 3.2.0 with SDL3) on Linux;
   upstream's resolve-hostnames, get-local-addrs, echo-server and datagram
   examples run as tests. Not translated: Haiku's interface monitor (which
   upstream keeps outside `SDL_net.c`; Haiku lists interfaces without
   noticing changes) and the PS Vita support upstream added after 3.2.0.
5. **SDL_rtf** — needs SDL_ttf.
   **Done** (`sdl3-rtf`, SDL_rtf 3.0.0 from its `main` branch at
   0bdba67b48e67f2c7d80a08720dcce8b54b01018, which has no SDL3 release
   tag; upstream plans no further development): all of `SDL_rtf.c`
   (`RTF_CreateContext`, `RTF_Load`/`RTF_Load_IO`, `RTF_GetTitle`,
   `RTF_GetSubject`, `RTF_GetAuthor`, `RTF_GetHeight`, `RTF_Render`,
   `RTF_FreeContext` as `Drop`), `SDL_rtfreadr.c` (reflowing with word
   wrapping, indents, tab stops and alignment; rendering to a renderer
   rectangle with a scroll offset), and the RTF reader SDL_rtf adapted
   from the sample code of Microsoft's RTF specification (`rtfreadr.c`,
   `rtfactn.c`, `rtftype.h`, `rtfdecl.h`: groups, keyword and property
   tables, destinations, the font and color tables, document
   information, hex and binary data). The `RTF_FontEngine` callbacks are
   the `FontEngine` trait; `TtfFontEngine` is the font engine of
   upstream's `examples/showrtf.c` over `sdl3-ttf`, and the example
   itself is `examples/showrtf.rs`. Upstream's NULL dereference of an
   empty text buffer (an unnamed first font, an empty title before any
   text), its unbounded keyword and parameter buffers, and its color
   table freed twice when a context loads a second document are worked
   around (FIXME (upstream)). Checked bit for bit against upstream's C
   over SDL_ttf (with FreeType and HarfBuzz) and SDL3 on test documents
   written for it (fonts, colors, styles, alignment, indents, tabs,
   escapes, binary data, skipped destinations, a table, out-of-range
   values, malformed documents), truncated and with flipped bytes: the
   font engine's calls, title, subject and author, heights, and the
   rendered pixels at several widths, offsets and rectangles.
6. **SDL_shadercross** — needs `gpu/`; SPIRV-Cross/DXC glue.
   **Part 1 done** (`sdl3-shadercross`, SDL_shadercross 3.0.0 from its
   `main` branch at 1ff05bec573988a98ef9e0260b4da44f512b8367, with the
   SPIRV-Cross its `external/SPIRV-Cross` submodule pins,
   1a6169566c73d3da552748fc372fe2bbb856e46e): all of
   `SDL_shadercross.c` (`init`/`quit`, the shader formats, SPIR-V
   reflection of graphics shaders and compute pipelines, SPIR-V to MSL
   with SDL_GPU's Metal resource indices, to HLSL and, through Windows'
   `d3dcompiler_47.dll` loaded as upstream does, to DXBC;
   `compile_graphics_shader_from_spirv` and
   `compile_compute_pipeline_from_spirv` over `sdl3::gpu`) and `cli.c`
   (the `shadercross` binary). SPIRV-Cross is translated in its hidden
   `spirv_cross` module, under its Apache-2.0 OR MIT license: the parser,
   the parsed IR, the CFG and the analysis and reflection of
   `spirv_cross.cpp`, and the GLSL, MSL and HLSL backends in full, with
   the subset of the C API SDL_shadercross calls. Upstream's bugs are kept
   where output depends on them and marked FIXME (upstream) (among them
   SDL_shadercross's uninitialized MSL buffer index for unused compute
   bindings, which the translation zeroes, and the CLI's property
   mix-ups); where C++ reads out of bounds, recurses without limit,
   loops forever or allocates for impossible sizes on malformed SPIR-V
   (member indices past any struct, vectors of more than 16 components,
   block chains that come back around), the translation returns an
   error. Checked byte for byte against SDL_shadercross and SPIRV-Cross's C++ on
   13 glslang-built shaders (tools/gen_shadercross_testdata.py): GLSL
   (450, ES 310, Vulkan), MSL (1.2 and 2.1), HLSL (SM 6.0, 5.1 and 5.0
   with PSSL) and the reflection, and on malformed variants of them
   (truncated, broken headers and word counts, out-of-range IDs, flipped
   bytes; the ones that crash upstream or make it allocate gigabytes are
   checked to fail cleanly); in release mode, on every truncation of them
   and three byte flips at every position (about 285,000 variants through
   reflection, MSL, HLSL and GLSL) for panics, aborts and hangs, which
   found the guards above. The test shaders also become `sdl3::gpu`
   shaders and compute pipelines on a Vulkan device (lavapipe; skipped
   without one), and DXBC through `d3dcompiler_47.dll` on Windows (Wine's
   builtin one doesn't implement shader model 5.1, so under Wine only the
   failure is checked). Left for part 2: the DXC paths
   (`compile_dxil_from_hlsl`, `compile_spirv_from_hlsl`,
   `compile_dxil_from_spirv` and the DXIL choice of the format
   selection), which fail with upstream's "not built with DXC" messages
   (DXC's `dxcompiler` library is not a system library), and the vkd3d
   DXBC path upstream takes outside Windows.

## Rules for every phase

* Translate whole files; keep upstream's function order inside a module.
* Carry upstream comments and `FIXME`s across; fix upstream bugs in their
  own changes (see the translation guide).
* Pin behaviour with tests derived from upstream constants and comments.
* `cargo test`, `cargo clippy --all-targets` and `cargo doc` stay clean.
* Platform code is `#[cfg]`-gated and the crate always builds on every tier-1
  Rust target even when a backend is missing (it reports "not supported",
  exactly like an SDL built without that backend).
