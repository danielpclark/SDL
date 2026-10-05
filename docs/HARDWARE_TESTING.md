# Testing against the system and real hardware

Most of the test suite needs nothing but `cargo test`. The tests of the
platform backends, though, talk to whatever the machine offers: an X server,
a Wayland compositor, a PulseAudio or PipeWire server, libGL, a camera,
Windows' WASAPI endpoints. When a piece is missing, such a test **skips**:
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
| `egl` | Linux, Windows | libEGL and the GL/GLES libraries (Mesa on Linux; on Windows an EGL such as ANGLE's `libEGL.dll`, which Windows doesn't ship) |
| `vulkan` | Linux | a Vulkan loader and a driver with the surface extensions, `VK_EXT_headless_surface` included for the tests of the Vulkan renderer and of the GPU API's swapchains on offscreen windows (Mesa's lavapipe is enough; the GPU API's Vulkan backend tests run under `VK_LAYER_KHRONOS_validation` when it is installed, and present to an X11 window too when `DISPLAY` is set) |
| `wayland` | Linux | libwayland-client/-egl/-cursor, libxkbcommon and `sway` (or `weston`) in `PATH`; the input and clipboard tests need sway |
| `dbus` | Linux | libdbus and `dbus-daemon` (the tests start private buses) |
| `pulseaudio` | Linux | libpulse, and the `pulseaudio` and `pactl` binaries (the tests start a private server) |
| `pipewire` | Linux | libpipewire and a running PipeWire server with a session manager and at least one sink |
| `alsa` | Linux | libasound and its configuration (the tests use its `file` and `null` PCMs, no sound card) |
| `udev` | Linux | libudev and a working udev (netlink) |
| `uinput` | Linux | a writable `/dev/uinput`, and read access to the event node it creates |
| `hidapi` | Linux, Windows | the HID backend: hidraw with libudev, or `hid.dll` |
| `xinput` | Windows | an XInput DLL (`XInput1_4.dll` ships with Windows) |
| `gameinput` | Windows | a GameInput DLL with the v3 API (one that exports `GameInputInitialize`): the GameInput redistributable's `GameInputRedist.dll`, or a recent enough `GameInput.dll` from Windows (an older inbox one only has the v0 API) |
| `wgl` | Windows | `opengl32.dll` with a pixel format and context; the hardware check wants the GPU's driver, not GDI Generic |
| `d3d11` | Windows | `d3d11.dll` and `dxgi.dll` with a Direct3D 11 device (feature level 11.0 or 11.1) and a swap chain on a window: a GPU driver, WARP, or Wine's d3d11 (wined3d on Mesa) |
| `wasapi` | Windows | WASAPI with a default playback and a default recording endpoint |
| `mediafoundation` | Windows | Media Foundation (`mfplat.dll`, `mf.dll`, `mfreadwrite.dll`; missing on Windows N/Server without the Media Feature Pack) |
| `v4l2` | Linux | the V4L2 camera driver |
| `camera` | any | a camera (only the hardware checks need one) |
| `controller` | any | a connected game controller (only the hardware checks need one) |
| `desktop` | Windows | an interactive desktop the windows video driver can open windows on (and move the cursor on) |

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

When the hardware isn't there they skip like the other tests (as `camera`,
`wasapi`, `wgl`, ...), so add the capabilities to `SDL3_TEST_REQUIRE` to
make that a failure. `cargo test -- --ignored` runs only these tests;
`cargo test -- --include-ignored` runs everything.

The camera checks may need permission first: on Windows, Settings >
Privacy & security > Camera (and Microphone, for WASAPI recording) must
allow desktop apps; the tests wait 30 seconds for an answer.

## Linux (Ubuntu 24.04)

```sh
sudo apt-get install -y xvfb libgl1 libgl1-mesa-dri libegl1 libgles2 \
  mesa-vulkan-drivers libvulkan1 libxtst6 libwayland-client0 \
  libwayland-egl1 libwayland-cursor0 libxkbcommon0 sway \
  pulseaudio pulseaudio-utils pipewire pipewire-bin wireplumber \
  libudev1 libasound2t64 libdbus-1-3 dbus-daemon
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
SDL3_TEST_REQUIRE=x11,xvfb,glx,egl,vulkan,wayland,dbus,pulseaudio,pipewire,alsa,udev,hidapi \
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

Nothing needs installing for `desktop`, `wgl`, `d3d11`, `wasapi`,
`mediafoundation`, `xinput` and `hidapi`: they come with Windows and the GPU
driver, given an
interactive session (not a service, not a remote desktop session without a
GPU), speakers or headphones, and a microphone (a webcam's counts).
`gameinput` needs the GameInput redistributable; `egl` needs an EGL
implementation such as ANGLE's `libEGL.dll` next to the test executable or
in `PATH`, so leave it out normally.

In PowerShell:

```powershell
cargo test --workspace
# fail instead of skipping:
$env:SDL3_TEST_REQUIRE = "desktop,wgl,d3d11,wasapi,mediafoundation,xinput,hidapi"
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

## Windows tests under Wine (from Linux)

The Windows backends are also tested under Wine, which provides `desktop`
(with an X server), `wgl` (Mesa through Wine), `d3d11` (wined3d on Mesa),
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

## CI

The ubuntu test jobs install the packages above, start the private
PipeWire server with `tools/ci/start-pipewire.sh` and set
`SDL3_TEST_REQUIRE=x11,xvfb,glx,egl,vulkan,wayland,dbus,pulseaudio,pipewire,alsa,udev,hidapi`,
so on Linux only the uinput test (the runner can't use `/dev/uinput`) and
the hardware checks may skip. The Windows jobs require nothing, since what
a hosted runner offers (a desktop, audio endpoints) isn't guaranteed. Every
job lists its skips in the job summary.
