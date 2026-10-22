# Testing against the system and real hardware

Most of the test suite needs nothing but `cargo test`. The tests of the
platform backends, though, talk to whatever the machine offers: an X server,
a Wayland compositor, a PulseAudio or PipeWire server, libGL, a camera,
Windows' WASAPI endpoints, a macOS window server session. When a piece is missing, such a test **skips**:
it prints

```
note: skipping, pipewire unavailable: no server (Audio target 'pipewire' failed to initialize)
```

and passes. That keeps `cargo test` green on any machine, but it also means
a green run can hide checks that never ran. This page lists what each
check needs, how to make a missing piece fail the run instead, and how to
run the checks that need real hardware.

## Capabilities, `SDL3_TEST_REQUIRE` and the skip log

Every skip names one *capability* (the list is `CAPABILITIES` in
`sdl3/src/lib.rs`, module `test_support`; `skip()` there reports it):

| Capability | Platform | Provided by |
|---|---|---|
| `x11` | Linux | libX11 and the X extension libraries (libXext, libXi, libXrandr, libXfixes, libXcursor, libXss, libXtst) |
| `xvfb` | Linux | an X server: `Xvfb` in `PATH` (each X11 test starts its own), or `DISPLAY` |
| `glx` | Linux | libGL with GLX on that server (Mesa's llvmpipe is enough) |
| `egl` | Linux, Windows, macOS | libEGL and the GL/GLES libraries (Mesa on Linux; on Windows an EGL such as ANGLE's `libEGL.dll`, which Windows doesn't ship, or Mesa's (below) with `EGL_PLATFORM=windows`; on macOS ANGLE's `libEGL.dylib` and `libGLESv2.dylib`, from the directory `SDL3_TEST_ANGLE_DIR` names or the copy the Android emulator ships in `~/Library/Android/sdk/emulator/lib64/gles_angle`). The offscreen driver has EGL only on Linux and the other Unix systems, so the OpenGL ES 2 renderer's checks, which draw on it, skip on Windows and macOS, as the OpenGL renderer's checks on it do on macOS |
| `vulkan` | Linux, Windows, macOS | a Vulkan loader and a driver with the surface extensions, `VK_EXT_headless_surface` included for the tests of the Vulkan and GPU renderers and of the GPU API's swapchains on offscreen windows (Mesa's lavapipe is enough; on Windows those tests present to the Windows video driver's windows instead, since the GPU drivers there (NVIDIA's, say) have no `VK_EXT_headless_surface`, and only the offscreen driver's own headless-surface check skips; the tests of the GPU API's Vulkan backend and of the GPU renderer run under `VK_LAYER_KHRONOS_validation` when it is installed, and present to an X11 window too when `DISPLAY` is set); on macOS MoltenVK, by upstream's default names: `libMoltenVK.dylib` itself (in `/usr/local/lib`, say), which runs every check, or the Vulkan loader with it (the Cocoa driver's surface check also tries Homebrew's `/opt/homebrew/lib/libvulkan.1.dylib`), through which the Vulkan renderer's checks skip with `VK_ERROR_INCOMPATIBLE_DRIVER`: MoltenVK's devices then need portability enumeration, which upstream's renderer doesn't ask for. Under Metal's API validation, which the GPU API's Metal devices in debug mode turn on for the rest of the process (as upstream's do, setting `MTL_DEBUG_LAYER=1` unless it's set), MoltenVK misdraws the fills after another render state's in a pass in the GPU renderer's render-state test on the CI runners' Apple Paravirtual device (1.3.0 to 1.4.2, reporting no error), so with a MoltenVK device and `MTL_DEBUG_LAYER` set and not `0` those three checks skip (run the test again with `MTL_DEBUG_LAYER=0` for them; with discrete descriptor binding, `MVK_CONFIG_USE_METAL_ARGUMENT_BUFFERS=0`, more of the renderer's checks misdraw under the layer; Khronos' validation layer, with synchronization validation, finds nothing in them on lavapipe, and they pass on Metal under the same validation) |
| `wayland` | Linux | libwayland-client/-egl/-cursor, libxkbcommon and `sway` (or `weston`) in `PATH`; the input and clipboard tests need sway |
| `dbus` | Linux | libdbus and `dbus-daemon` (the tests start private buses) |
| `pulseaudio` | Linux | libpulse, and the `pulseaudio` and `pactl` binaries (the tests start a private server) |
| `pipewire` | Linux | libpipewire and a running PipeWire server with a session manager and at least one sink |
| `alsa` | Linux | libasound and its configuration (the tests use its `file` and `null` PCMs, no sound card) |
| `udev` | Linux | libudev and a working udev (netlink) |
| `uinput` | Linux | a writable `/dev/uinput`, and read access to the event node it creates |
| `hidapi` | Linux, Windows, macOS | the HID backend: hidraw with libudev, `hid.dll`, or IOKit's HID manager |
| `ipv6` | any | IPv6 sockets that can resolve and bind to the loopback address `::1` (`sdl3-net`'s IPv6 tests; containers often have IPv4 only) |
| `tzdata` | Linux | the time zone database in `/usr/share/zoneinfo` (the local time tests set `TZ`) |
| `locales` | Linux | glibc's `localedef` and its locale sources in `/usr/share/i18n/locales` (the tests compile the locales they read the date and time formats of) |
| `xinput` | Windows | an XInput DLL (`XInput1_4.dll` ships with Windows) |
| `gameinput` | Windows | a GameInput DLL with the v3 API (one that exports `GameInputInitialize`): the GameInput redistributable's `GameInputRedist.dll`, or a recent enough `GameInput.dll` from Windows (an older inbox one only has the v0 API) |
| `wgl` | Windows | `opengl32.dll` with a pixel format and context (GDI Generic's 1.1 is enough for most checks; Mesa's, below, for all of them); the hardware check wants the GPU's driver, not GDI Generic |
| `d3d11` | Windows | `d3d11.dll` and `dxgi.dll` with a Direct3D 11 device (feature level 11.0 or 11.1) and a swap chain on a window: a GPU driver, WARP, or Wine's d3d11 (wined3d on Mesa) |
| `d3d12` | Windows | `d3d12.dll` and `dxgi.dll` with a Direct3D 12 device (feature level 11.0) for the GPU API's backend: a GPU driver, WARP, or Wine's d3d12 (vkd3d on a Vulkan driver, with an X server for DXGI); the DXIL checks need shader model 6, which vkd3d under Wine 9 lacks (they skip there) |
| `dxc` | any | a current DXC (1.7.2212 or later, with the `-fspv-preserve-*` options) for `sdl3-shadercross`'s DXIL and HLSL-to-SPIR-V checks: `dxcompiler.dll` with `dxil.dll` (`libdxcompiler.so` elsewhere) next to the test executable or in `PATH`, from [DXC's releases](https://github.com/microsoft/DirectXShaderCompiler/releases) or the Vulkan SDK. The Windows SDK's DXC is found on its own and compiles DXIL, but can't do HLSL to SPIR-V (1.6 in SDK 10.0.22621 is too old for the SPIR-V options; the SDKs on the CI runners ship a DXC built without SPIR-V code generation): the test notes that and passes, unless `dxc` is required |
| `wasapi` | Windows | WASAPI with a default playback and a default recording endpoint |
| `mediafoundation` | Windows | Media Foundation (`mfplat.dll`, `mf.dll`, `mfreadwrite.dll`; missing on Windows N/Server without the Media Feature Pack) |
| `v4l2` | Linux | the V4L2 camera driver |
| `camera` | any | a camera (only the hardware checks need one) |
| `controller` | any | a connected game controller (only the hardware checks need one) |
| `desktop` | Windows | an interactive desktop the windows video driver can open windows on (and move the cursor on) |
| `cocoa` | macOS | a login session whose window server shows windows (Aqua), for the main-thread programs `cargo test -p sdl3 --test cocoa`, `--test cocoa_shell` and `--test cocoa_metal` (and `cargo test -p sdl3-shadercross --test metal`) |
| `coreaudio` | macOS | CoreAudio with a default playback and a default recording device |
| `metal` | macOS | a Metal device (`MTLCreateSystemDefaultDevice()`): the GPU API's Metal backend, the GPU renderer on it, and the Metal renderer |

Two environment variables control skips:

* `SDL3_TEST_REQUIRE`: `all`, or a comma-separated list of capabilities.
  A test that would skip for a listed capability panics instead
  (`required capability pipewire unavailable: ...`). An unknown name is an
  error, so typos don't go unnoticed. Use it to check that a machine you
  set up for testing really runs everything you expect.
* `SDL3_TEST_SKIP_LOG`: a file every skip appends a line to
  (`capability<TAB>test<TAB>reason`). CI lists it in the job summary.

Some notes aren't skips and stay plain notes: device counts, "no HID
devices", what an endpoint recorded, a missing 3.3 core profile.

## Hardware checks: `cargo test -- --ignored`

These tests are `#[ignore]`d, because they need hardware a CI runner
doesn't have, and print what they find (so run them with `--nocapture`):

| Test | Platform | What it checks |
|---|---|---|
| `camera::mediafoundation::tests::hardware_capture_from_the_first_camera` | Windows | lists the Media Foundation cameras and their formats; opens the first one and grabs ten frames: size and format match the opened spec, pixels are there, timestamps strictly advance; prints the frame rate |
| `camera::v4l2::tests::hardware_capture_from_the_first_camera` | Linux | the same through V4L2 (`/dev/video*`) |
| `camera::pipewire::tests::hardware_capture_from_the_first_camera` | Linux | the same through the PipeWire server's cameras |
| `video::drivers::windows::tests::hardware_gpu_gl_context_and_renderer` | Windows | a WGL context, printing `GL_VENDOR`, `GL_RENDERER`, `GL_VERSION` and whether framebuffer objects exist; it must not be GDI Generic. Then a window with the `opengl` 2D renderer (the default one without framebuffer objects), which must clear to a color that reads back |
| `render::direct3d11::tests::hardware_gpu_d3d11_renderer` | Windows | a Direct3D 11 renderer on the GPU, printing the adapter's description, vendor and device ids and video memory, the feature level and the swap chain flags (tearing support). Then a window with the default 2D renderer, which must be `direct3d11` and clear to a color that reads back, and an NV12 texture (which Wine's d3d11 can't sample) drawn as the software renderer draws it |
| `audio::drivers::wasapi::tests::hardware_default_playback_and_recording` | Windows | lists the WASAPI endpoints; plays half a second of a quiet 440 Hz tone on the default playback endpoint (it must drain), then records half a second from the default recording endpoint (it must arrive), printing formats, timing and the peak level |
| `joystick::windows::tests::hardware_joysticks_per_driver` | Windows | lists the joysticks and gamepads (name, GUID, gamepad type) each joystick driver finds: with the default drivers, with every driver enabled, and with each of HIDAPI, RawInput, DirectInput, XInput, Windows.Gaming.Input and GameInput alone (GameInput alone skips as `gameinput` where GameInput isn't usable). Zero controllers is fine |
| `joystick::windows::tests::hardware_controller_input` | Windows | opens the first controller through the default drivers and prints its name, driver, GUID, VID:PID, axes/buttons/hats and gamepad mapping; asks for a button press and release, then a full stick (or axis) movement, waiting up to 30 seconds for each; then sends a short rumble and prints whether the controller supports it. Run it on its own, as below |
| `audio::drivers::coreaudio::tests::hardware_default_playback_and_recording` | macOS | lists the CoreAudio devices; plays half a second of a quiet 440 Hz tone on the default playback device (it must drain), then records half a second from the default recording device, printing formats, timing and the peak level. A recording of pure silence (no microphone permission, see below) skips |
| `camera::coremedia::tests::hardware_capture_from_the_first_camera` | macOS | the camera check through AVFoundation |
| `joystick::darwin::tests::hardware_joysticks_per_driver` | macOS | lists the controllers the HIDAPI, IOKit and MFi (GameController) drivers see, together and each alone. Zero controllers is fine |
| `joystick::darwin::tests::hardware_controller_input` | macOS | the interactive controller check of Windows', through the default macOS drivers. Run it on its own |
| `joystick::apple::tests::hardware_mfi_controllers` | macOS | what GameController reports through the MFi driver alone: names, GUIDs, the generated gamepad mapping, SF Symbols names, sensors, LED, battery and rumble. GameController discovers controllers on the main run loop, which a test thread doesn't run, so only the ones it already knew when the driver started are listed |

When the hardware isn't there they skip like the other tests (as `camera`,
`wasapi`, `wgl`, ...), so add the capabilities to `SDL3_TEST_REQUIRE` to
make that a failure. `cargo test -- --ignored` runs only these tests;
`cargo test -- --include-ignored` runs everything.

The camera checks may need permission first: on Windows, Settings >
Privacy & security > Camera (and Microphone, for WASAPI recording) must
allow desktop apps; the tests wait 30 seconds for an answer. On macOS see
below.

## Linux (Ubuntu 24.04)

```sh
sudo apt-get install -y xvfb libgl1 libgl1-mesa-dri libegl1 libgles2 \
  mesa-vulkan-drivers libvulkan1 libxtst6 libwayland-client0 \
  libwayland-egl1 libwayland-cursor0 libxkbcommon0 sway \
  pulseaudio pulseaudio-utils pipewire pipewire-bin wireplumber \
  libudev1 libasound2t64 libdbus-1-3 dbus-daemon tzdata locales
```

On a desktop, PipeWire is usually already running. On a headless machine
(or to keep the tests off your real audio setup) start a private one, with
WirePlumber and a null sink and source:

```sh
export XDG_RUNTIME_DIR=$(mktemp -d)
tools/ci/start-pipewire.sh
```

Then:

```sh
cargo test --workspace
# fail instead of skipping (what CI requires):
SDL3_TEST_REQUIRE=x11,xvfb,glx,egl,vulkan,wayland,dbus,pulseaudio,pipewire,alsa,udev,hidapi,tzdata,locales \
  cargo test --workspace
# the OpenGL renderer's tests also run on GLX when DISPLAY is set:
xvfb-run -a cargo test --workspace
# the camera checks (V4L2 and PipeWire):
SDL3_TEST_REQUIRE=v4l2,camera cargo test --workspace -- --ignored --nocapture
```

The uinput joystick test needs `/dev/uinput` (`sudo modprobe uinput`) to be
writable and the event device it creates to be readable: run it as root, or
add a udev rule such as
`KERNEL=="uinput", MODE="0660", GROUP="input"` and be in the `input` group;
then require `uinput` too.

## Windows 11

Nothing needs installing for `desktop`, `wgl`, `d3d11`, `d3d12`, `vulkan`,
`wasapi`, `mediafoundation`, `xinput` and `hidapi`: they come with Windows
and the GPU driver (`vulkan` with a driver that has Vulkan, as NVIDIA's,
AMD's and Intel's do), given an interactive session (not a service, not a
remote desktop session without a GPU), speakers or headphones, and a
microphone (a webcam's counts). `gameinput` needs the GameInput
redistributable; `egl` needs an EGL implementation such as ANGLE's
`libEGL.dll` (or Mesa's, [below](#without-a-gpu-driver-mesa)) next to the
test executable or in `PATH`, so leave it out normally; `dxc` needs a
current DXC's `dxcompiler.dll` and `dxil.dll` next to the test executable
or in `PATH` (the Windows SDK's, found on its own, only compiles DXIL), so
leave it out unless you put one there.

In PowerShell:

```powershell
cargo test --workspace
# fail instead of skipping:
$env:SDL3_TEST_REQUIRE = "desktop,wgl,d3d11,d3d12,vulkan,wasapi,mediafoundation,xinput,hidapi"
cargo test --workspace
# the hardware checks (a webcam; a GPU OpenGL and Direct3D 11 driver; audio
# endpoints; controllers are only listed):
$env:SDL3_TEST_REQUIRE = "desktop,wgl,d3d11,wasapi,mediafoundation,camera"
cargo test --workspace -- --ignored --nocapture --skip hardware_controller_input
# with a controller connected, the interactive check (press a button, then
# move a stick, when it asks):
$env:SDL3_TEST_REQUIRE = "controller"
cargo test -p sdl3 --lib hardware_controller_input -- --ignored --nocapture
Remove-Item Env:SDL3_TEST_REQUIRE
```

Add `gameinput` to the lists when a GameInput with the v3 API is there.
The loader (Microsoft's, as upstream) picks the newest of the
redistributable's `GameInputRedist.dll` (in `System32`, or in the directory
that `RedistDir` under `HKLM\SOFTWARE\Microsoft\GameInput`, 32-bit view,
names), a `GameInputRedist.dll` next to the executable, and the
`GameInput.dll` Windows ships in `System32`. The inbox DLL is enough when
it's recent (it then exports `GameInputInitialize`); an old one (0.1908 on
Windows 11 21H2) only has the v0 API, and the tests skip with
`GameInputCreate failed: No such interface supported`. Installing the
redistributable fixes that.
Without GameInput the joystick listing leaves out "GameInput alone".

A plain `cargo test` only shows the output of failing tests, so the
`note: skipping` lines of the first two commands stay hidden. To see them,
set `SDL3_TEST_SKIP_LOG` to a file first and read it after each run:

```powershell
$env:SDL3_TEST_SKIP_LOG = "$env:TEMP\sdl3-skips.tsv"
```

### Without a GPU driver: Mesa

A machine without a GPU driver (a VM, the CI runners) gets OpenGL, EGL and
Vulkan from Mesa's software drivers, as the CI does: the MSVC build of
[mesa-dist-win](https://github.com/pal1000/mesa-dist-win/releases) (`x64`
folder) and a Vulkan loader (`vulkan-1.dll`, from the Vulkan SDK or
[built](https://github.com/KhronosGroup/Vulkan-Loader) from its SDK tag):

* copy Mesa's `opengl32.dll`, `libgallium_wgl.dll`, `libEGL.dll` and
  `libGLESv2.dll`, and `vulkan-1.dll`, next to the test executables
  (`target\debug\deps`, `target\release\deps`), where they're found
  before System32's (Mesa's per-application deployment);
* register lavapipe's ICD, `lvp_icd.x86_64.json`, under
  `HKLM\SOFTWARE\Khronos\Vulkan\Drivers` (a `REG_DWORD` 0 named by its
  path), or name it in `VK_DRIVER_FILES`, which the loader ignores in an
  elevated process;
* set `GALLIUM_DRIVER=llvmpipe`: on Microsoft's Basic Render Driver Mesa
  picks its d3d12 driver, whose `glReadPixels` ends the process (exit code
  `0x80070057`);
* set `EGL_PLATFORM=windows`: the build's default EGL display is
  surfaceless, which has no window surfaces (`EGL_BAD_NATIVE_WINDOW`).

Then `wgl` and `vulkan` are there (lavapipe has no
`VK_EXT_headless_surface` on Windows, so the offscreen driver's Vulkan
check skips) and `egl` too, for the windows video driver's EGL contexts.

### Running them from a Claude Code session on that machine

A session on the Windows machine (the Claude desktop app, or
`claude remote-control` in a terminal) can run these and report back.
Clone the repository, open the session in it, and ask it to follow this
section: run the commands above in order, then report:

* the test result lines and every `note: skipping` line of each run (from
  `SDL3_TEST_SKIP_LOG`, as above, for the runs without `--nocapture`);
* what the hardware checks printed: `GL_VENDOR`/`GL_RENDERER`/`GL_VERSION`,
  the Direct3D 11 adapter, feature level and default renderer's name, the
  WASAPI endpoints and peak level,
  the camera's formats and frame rate, the controllers each joystick driver
  lists, and the controller check's output (which driver claimed it, the
  gamepad mapping, the button and axis events, the rumble result);
* any failure with its full panic message and backtrace
  (`$env:RUST_BACKTRACE = "1"`).

It needs Rust 1.87 or newer (through `rustup`) and the MSVC build tools
(or the GNU toolchain); nothing else.

With an older GPU, `hardware_gpu_gl_context_and_renderer` prints the
driver's OpenGL version; anything with framebuffer objects (OpenGL 3.0, or
the `GL_ARB_framebuffer_object`/`GL_EXT_framebuffer_object` extensions) must
get the `opengl` renderer. A machine with only Microsoft's GDI Generic 1.1
(no GPU driver installed, a VM) skips as `wgl`.

## macOS

Nothing needs installing for `cocoa`, `coreaudio`, `metal` and `hidapi`:
they come with macOS, given a login session at the screen (not a plain SSH
session), speakers and a microphone. `vulkan` needs MoltenVK where
upstream's default names find it: `libMoltenVK.dylib` from
[MoltenVK's releases](https://github.com/KhronosGroup/MoltenVK/releases)
(`MoltenVK-macos.tar`, `MoltenVK/dynamic/dylib/macOS`) or Homebrew's
`molten-vk` copied or linked into `/usr/local/lib`, as the CI does, runs
every check; a loader (`brew install vulkan-loader molten-vk`) is found by
the Cocoa driver's check only, and the Vulkan renderer's checks skip
through it (see the table). After the Metal tests three render-state
checks skip `vulkan` (see the table too); `MTL_DEBUG_LAYER=0 cargo test
-p sdl3 --lib render::gpu::tests::gpu_render_states -- --exact` runs it.
`egl` needs ANGLE (see the table), and the offscreen driver's
checks skip it anyway, so leave that out.

```sh
cargo test --workspace
# fail instead of skipping:
SDL3_TEST_REQUIRE=cocoa,coreaudio,metal,hidapi,vulkan cargo test --workspace
# the hardware checks (the speakers and microphone, a camera; controllers
# are only listed):
SDL3_TEST_REQUIRE=coreaudio,camera cargo test --workspace -- --ignored --nocapture \
  --skip hardware_controller_input
# with a controller connected, the interactive check:
SDL3_TEST_REQUIRE=controller cargo test -p sdl3 --lib hardware_controller_input -- --ignored --nocapture
```

The Cocoa video driver only starts on the main thread (as upstream's), and
the test harness runs tests on other threads, so the tests that need it
are programs of their own (`harness = false`), run on the main thread by
`cargo test`: `tests/cocoa.rs` (displays, windows, fullscreen, text input,
the clipboard, cursors, the keymap, mouse warping, message boxes, OpenGL,
OpenGL ES, Metal views, Vulkan surfaces, the opengl renderer and window
surfaces), `tests/cocoa_shell.rs` (the tray, file panels, notifications)
and `tests/cocoa_metal.rs` (the Metal renderer, the GPU API's swapchains
and the GPU renderer on a window), and `sdl3-shadercross`'s
`tests/metal.rs`. They show small windows, an alert, a status item in the
menu bar and file panels for a moment (every panel and menu is answered
from the main queue, so nothing waits for a person), move the mouse
pointer, and set the clipboard, restoring the pointer and the clipboard
afterwards. Notifications need an application bundle, which a test binary
isn't: they skip. The user storage test makes a process-unique folder
under `~/Library/Application Support` and removes it.

Microphone and camera access is granted to the app the tests run in (the
terminal: Terminal, iTerm2, ...), in System Settings > Privacy & Security
> Microphone and Camera. macOS asks the first time; the camera check waits
30 seconds for an answer. When the microphone was denied (or the request
was answered for another app), CoreAudio still records, but pure
silence, and the recording check skips; reset the decision to be asked
again, for example for iTerm2:

```sh
tccutil reset Microphone com.googlecode.iterm2
```

## Windows tests under Wine (from Linux)

The Windows backends are also tested under Wine, which provides `desktop`
(with an X server), `wgl` (Mesa through Wine), `d3d11` (wined3d on Mesa),
`d3d12` (vkd3d on Mesa's lavapipe; its DXGI needs the X server too, and it
has shader model 5.1, so the DXIL checks skip),
`mediafoundation`, `xinput` and `hidapi`, but no audio endpoints, cameras or
EGL:

```sh
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine64 \
WINEDEBUG=-all xvfb-run -a cargo test --workspace --target x86_64-pc-windows-gnu
# fail instead of skipping the Direct3D 11 renderer's tests:
SDL3_TEST_REQUIRE=d3d11 CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine64 \
WINEDEBUG=-all xvfb-run -a cargo test -p sdl3 --lib --target x86_64-pc-windows-gnu direct3d11
```

Wine's `IDXGISwapChain1::SetRotation()` is a stub that fails, so the
Direct3D 11 renderer can't be made for an ordinary window there (as
upstream's can't: the default renderer falls back to `opengl`); its tests
draw into transparent windows, whose swap chains aren't rotated. Wine's
d3d11 can't sample NV12 textures either, so those checks only run on
Windows.

## Emscripten (under node)

The workspace's tests also run built for `wasm32-unknown-emscripten`,
under node, which `.cargo/config.toml` makes the runner (with the link
options the test programs need: growable memory up to 4 GB, an 8 MB
stack, node's environment for `SDL3_TEST_REQUIRE` and the hints). That
needs emsdk 6.0.13 (the version CI pins) and node 22 or newer:

```sh
rustup target add wasm32-unknown-emscripten
git clone https://github.com/emscripten-core/emsdk && cd emsdk
./emsdk install 6.0.13 && ./emsdk activate 6.0.13 && . ./emsdk_env.sh
cd -
CARGO_PROFILE_DEV_OPT_LEVEL=1 cargo test --workspace --target wasm32-unknown-emscripten
```

Unoptimized test programs run too (no function nears V8's limits of about
50,000 locals and 7.6 MB of code), but five or six times slower;
`CARGO_PROFILE_DEV_OPT_LEVEL=1` (as CI sets it) keeps the debug assertions
and overflow checks. `CARGO_PROFILE_DEV_DEBUG=line-tables-only` makes
the programs much smaller.

The programs see emscripten's in-memory file system (MEMFS), not the
host's, so they stay usable in a browser: test data is embedded with
`include_bytes!`, and what needs the host's files (a directory tree read
by path, as TiMidity's configurations; another crate's test data; the
checks that the embedded lists match the directories) is ignored there,
"host file system". The build is upstream's Emscripten configuration,
without threads, so the tests that need a thread (an audio device's, the
camera's, a timer's), a process, sockets, an executable name, or a
renderer or GPU backend the build doesn't have are `#[ignore]`d on
Emscripten with the reason, which the test run prints. Where
SDL has no SIMD blitters, as on wasm32, sdl3-image's reference results
of the frames composited with them are upstream's without them
(`sdl3-image/src/testdata/reference_nosimd.txt`). The skip log, which
the programs would write in their own file system, is made in CI from
the notes they print (`RUST_TEST_NOCAPTURE=1`).

The JavaScript layer, `sdl3-emscripten`, is tested by its own
integration tests (`sdl3-emscripten/tests/`): `em_js!` and `em_asm!`
round trips, the dependency table, the callback registry and the
`html5.h` layouts (the expected sizes and offsets are what a C program
built with emcc printed). Its build script links them with
`-sEXPORTED_FUNCTIONS=_main`, as an application with its own export list
is, which replaces the functions rustc would export. The crate's own
`--lib` tests can't link (in an executable crate rustc doesn't export the
`EM_JS` text) and are off. CI runs these tests once more in a release
build (`cargo test -p sdl3-emscripten --release --target
wasm32-unknown-emscripten`), where thin LTO and emcc's `-O3` change what
the program holds.

## CI

The ubuntu test jobs install the packages above, start the private
PipeWire server with `tools/ci/start-pipewire.sh` and set
`SDL3_TEST_REQUIRE=x11,xvfb,glx,egl,vulkan,wayland,dbus,pulseaudio,pipewire,alsa,udev,hidapi,tzdata,locales`,
so on Linux only the uinput test (the runner can't use `/dev/uinput`) and
the hardware checks may skip.

The Windows and macOS jobs install pinned GPU drivers (versions and
checksums in `.github/workflows/ci.yml`, cached between runs). On Windows
that's Mesa as [above](#without-a-gpu-driver-mesa), from mesa-dist-win's
release, with the Vulkan loader built from its SDK tag, and the jobs
require `wgl`. On macOS it's MoltenVK's release `libMoltenVK.dylib` in
`/usr/local/lib`, and the jobs run the render-state test whose checks
MoltenVK misdraws under the Metal debug layer (see the `vulkan` row above)
once more without it, requiring `vulkan`; the Cocoa programs find ANGLE in the
Android emulator the runner image ships. Both fail on any skip of `vulkan`
and `egl` but the ones by design: the offscreen driver's headless-surface
check on Windows, those render-state checks on macOS, and the OpenGL and
OpenGL ES 2 renderers' checks on the offscreen driver, which has no EGL
on either. What else a hosted runner offers (audio endpoints, GameInput,
a camera) isn't guaranteed, so nothing more is required. The
macOS clippy job also lints the x86-64 build for Intel Macs. The
Emscripten job runs the tests under node as [above](#emscripten-under-node),
with emsdk and emcc's sysroot libraries cached. Every job lists its skips
in the job summary (`tools/ci/list-skips.sh`).
