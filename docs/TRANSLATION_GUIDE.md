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

iOS, tvOS and visionOS share upstream's UIKit code and most of its Apple
code; this is how their C becomes Rust here.

### Platform macros

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
* Mac Catalyst and watchOS aren't targets here. Upstream's
  `TARGET_OS_MACCATALYST` branches are kept as `cfg`s, unbuilt.
* An Objective-C method declared at run time takes its type encoding
  through `objc::encoding()`, which writes `BOOL` as the platform's (`c`
  on x86-64 macOS and Mac Catalyst, `B` elsewhere, the x86-64 Simulator
  too).
* `aarch64-apple-ios`, `aarch64-apple-ios-sim` and `x86_64-apple-ios` are
  tier 2 targets, also for the minimum supported Rust, which checks iOS.
  The tvOS and visionOS targets (`aarch64-apple-tvos`, `-tvos-sim`,
  `aarch64-apple-visionos`, `-visionos-sim`) are tier 2 targets of the
  pinned toolchain but tier 3 for the minimum supported Rust, which has
  no standard library for them, so tvOS and visionOS aren't checked with
  it. `x86_64-apple-tvos` (the Intel Macs' tvOS Simulator) is tier 3,
  built with `-Zbuild-std` (in CI, linted on the pinned toolchain with
  `RUSTC_BOOTSTRAP=1`); there's no x86-64 visionOS Simulator.

### What is built

What is built follows upstream's CMake build for the platform, as for
macOS and Emscripten:

| | iOS | tvOS | visionOS |
|---|---|---|---|
| video | UIKit | UIKit, no orientations, status bar or pasteboard | UIKit, flat windows on one fake display |
| OpenGL ES (EAGL), the OpenGL ES 2 renderer | yes | yes (below) | no |
| Metal renderer, the GPU API's Metal backend | yes | yes | yes |
| Vulkan (MoltenVK) | yes | yes | built; nothing loads (below) |
| audio | CoreAudio with the audio session | the same | the same |
| joysticks | HIDAPI (off by default), MFi, virtual | the same, with the Siri Remote | MFi only (below) |
| sensors | CoreMotion | dummy | CoreMotion |
| camera | CoreMedia | none | none |
| power | UIDevice's battery | no battery | UIDevice's battery |
| filesystem | Cocoa, the pref path in Application Support | Cocoa, the pref path in Caches, no user folders | Cocoa, the pref path in Application Support |
| process | POSIX | dummy (CMake finds no `posix_spawn()`) | POSIX |
| haptic, dialog, tray | dummy (HIDAPI's haptics on iOS and tvOS) | dummy | dummy |
| notification | not translated yet (dummy) | none (dummy), as upstream | not translated yet (dummy) |

The Apple-wide pieces (`core/apple`, the filesystem, locale, log, time
and timer backends, the GPU API's Metal backend) are the same as on
macOS. A macOS-only backend (Cocoa, IOKit, LaunchServices) is
`target_os = "macos"`, and so is an Apple-wide one whose UIKit half
isn't translated yet (the notifications: UserNotifications on iOS and
visionOS); a subsystem then falls back to what else is built, as
upstream would without the piece (the dummy notification backend). The
UIKit pieces that backends other than the video driver share (UIDevice,
UIApplication, the form factor, which upstream keeps in the video
driver) are in `core/apple/uikit`, which links UIKit.

Where upstream's Xcode project (`SDL_build_config_ios.h`) and its CMake
build differ, the translation picks one, marked `Note` in the code:

* OpenGL ES on tvOS: the Xcode project's. EAGL and the OpenGL ES 2
  renderer are built on iOS and tvOS (`all(any(target_os = "ios",
  target_os = "tvos"), not(target_abi = "macabi"))`); the CMake build
  leaves them out on tvOS. The C references of the tvOS tests are
  upstream's CMake build with OpenGL ES allowed on tvOS.
* The virtual joystick on visionOS: the CMake build's. CMake's
  `SDL_VIRTUAL_JOYSTICK` needs `SDL_HIDAPI`, which visionOS doesn't
  build, so the virtual joystick functions fail there with upstream's
  "SDL not built with virtual-joystick support" (`Note (upstream)`:
  the Xcode project builds it).
* The CoreMedia camera on iOS only: CMake's `MACOS OR IOS`, which
  `SDL_build_config_ios.h` agrees with.
* CoreBluetooth loaded at run time (below), where the CMake build links
  it.

### Frameworks and run-time checks

* The frameworks are linked as upstream links them, but for those a
  system the program may start on can lack (GameController and
  CoreHaptics, UniformTypeIdentifiers), and CoreBluetooth (below), which
  are loaded at run time (`SharedObject::load_framework()`: without the
  version check upstream's `SDL_LoadObject()` makes on UIKit, which a
  link doesn't go through). Symbols newer than the deployment target
  (`UITextContentTypeOneTimeCode`, the camera's device types) are looked
  up at run time, as weakly linked symbols would be.
* `@available(macOS 11.0, iOS 14.0, tvOS 14.0, *)` is
  `available!(macos(11, 0), ios(14, 0), tvos(14, 0))`
  (`core::apple::foundation`), checked at run time; a platform it doesn't
  name passes. SDK guards (`__IPHONE_OS_VERSION_MAX_ALLOWED >= 170000`) are
  compiled in, as if upstream were built with the newest SDK.
* A check translated from code that was macOS-only here must name every
  platform upstream's `@available` names: `available!(macos(11, 0))`
  alone passes on iOS and tvOS (as the MFi driver's did before its UIKit
  half: its checks are upstream's macOS, iOS and tvOS versions now).
* `SDL_LoadObject()` (`SharedObject::load()`) refuses to load anything
  before iOS 8 on UIKit, as upstream's, which reads visionOS's version as
  iOS's, so nothing loads on visionOS (`FIXME (upstream)`): there the
  Vulkan library isn't loaded, upstream's C failing the same way.

### The app and the UIKit video driver

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
* `SDL_SetiOSAnimationCallback(window, interval, callback, param)` is
  `Window::set_ios_animation_callback(interval, Some(Box::new(closure)))`
  (`None` stops it). The closure isn't `Send`, so the call must be made
  on the main thread (`Note`), where the display link calls it. As the
  other drivers' pointer properties, `SDL_PROP_WINDOW_UIKIT_WINDOW_POINTER`
  and `SDL_PROP_WINDOW_CREATE_WINDOWSCENE_POINTER` hold numbers.
* The view's touch handlers enumerate the touches through the set's
  `objectEnumerator` (as `for ... in` does), and a finger's ID is its
  `UITouch`'s address, as upstream's. The methods upstream's view calls
  only from its own handlers (`touchLocation:shouldNormalize:`,
  `indirectPointerPressed:fromEvent:`, ...) are Rust functions, not
  methods of the class. The game controllers' mice (`GCMouse`) are
  GameController's, loaded at run time: without it there's no mouse to
  follow, and no relative mode (`SDL_GetMouse()->SetRelativeMouseMode`
  is set only while they're followed, as upstream's).
* The keyboard (`SDL_IPHONE_KEYBOARD`, on every UIKit platform) is the
  view controller's: its text field (`SDLUITextField`, declared at run
  time) and the `UITextFieldDelegate` methods, the keyboard's
  notifications, the key commands and `startTextInput`/`stopTextInput`
  are as upstream's, the instance variables in the view controller's
  Rust state. The text field's changes go through `NSString`'s UTF-16
  code units as upstream's do: each one, surrogates too, is a key press
  (`keyboard::send_keyboard_unicode_code()`, which takes upstream's
  `Uint32`; `send_keyboard_unicode_key()` takes a `char`). The key
  commands' inputs are compared with UIKit's constant strings by
  address, as upstream does. The game controllers' keyboards
  (`GCKeyboard`) are GameController's, loaded at run time. When the text
  field can't become the first responder, starting text input fails
  with an error of its own (`Note`: upstream fails without one).
* The clipboard is the general `UIPasteboard`'s text (the driver has
  upstream's `SetClipboardText`, `GetClipboardText` and
  `HasClipboardText`, not the data entry points, so the clipboard front
  end gives the pasteboard the text among the app's data and keeps the
  rest to the app); `SDL_UIKitVideoData` is the driver's `VideoData`,
  whose notification block (`UIPasteboardChangedNotification`) holds an
  `Arc` of it. A message box is upstream's `UIAlertController`, its
  actions' handlers Rust closures in blocks, run in a nested run loop on
  the main thread (`SDL_RunOnMainThread()`, waiting, from another one);
  the bootstrap's message box, as the driver, only shows one in an app,
  on its main thread (`Note`), and fails with upstream's "Could not show
  message box." elsewhere.

### Graphics

* Metal views are upstream's `SDL_uikitmetalview`, a subclass of the
  run-time `SDL_uikitview` (so it becomes the window's view as any of
  them does). The Metal renderer and the GPU API's blit shaders take
  upstream's precompiled libraries for the platform, generated from its
  headers (`tools/gen_metal_render_shaders.py PLATFORM`,
  `tools/gen_metal_shaders.py PLATFORM`) into a file each, the module
  chosen by `cfg` (`#[path]`) as upstream's `#if` chain chooses the
  header: the renderer's tests macOS, then tvOS (simulator, device), then
  the rest (simulator, device), while `Metal_Blit.h` tests
  `SDL_PLATFORM_IOS` first, so tvOS takes the iOS blit libraries and its
  own are never used (`FIXME (upstream)`; they aren't generated).
  visionOS takes the iOS libraries of both (upstream has none of its
  own). The renderer's other UIKit branches are `cfg`s too
  (`CONSTANT_ALIGN()`, 256 on macOS, the simulators and Mac Catalyst, the
  type's alignment on the devices; vsync, which only Mac layers can turn
  off; the iOS and tvOS feature sets' maximum texture size; no EDR on
  tvOS), and so are visionOS's RealityKit paths (a curved window's
  texture, through the Swift bridge's stand-ins: see visionOS below).
* OpenGL ES (EAGL) is built on iOS and tvOS (see "What is built").
  OpenGLES.framework is linked there, as upstream links it.
  `SDLEAGLContext` is an `EAGLContext` subclass declared at run time
  whose `sdlView` (strong) is an instance variable released in its
  `dealloc`, as ARC would; `SDL_uikitopenglview`'s instance variables
  are a Rust structure the view points to, its `context` a weak
  reference made with the runtime's `objc_storeWeak()` and
  `objc_loadWeakRetained()` (what ARC's `weak` compiles to), so a view
  outliving its context sees nil as upstream's does. The context handle
  owns the reference `alloc`/`init` gave (`CFBridgingRetain`).
  `UIKit_GL_GetProcAddress()` is `dlsym(RTLD_DEFAULT, ...)` as upstream;
  the OpenGL ES 2 renderer looks its functions up through it on UIKit too,
  where upstream's `__SDL_NOGETPROCADDR__` takes them from the linked
  library (`Note`: the same functions; a missing one is an error, not a
  link failure). With OpenGL ES contexts, a UIKit window that asks for
  no graphics API gets the OpenGL flag, as upstream's
  (`SDL_DefaultGraphicsBackends()`), and on visionOS and Mac Catalyst the
  Metal one. The swaps check UIDevice's battery monitoring
  (`SDL_UIKit_UpdateBatteryMonitoring()`), whose assertion fails on the
  iOS Simulator three seconds after a power query (`FIXME (upstream)`,
  in `power/uikit.rs`). The OpenGL ES 1 renderer (`SDL_render_gles.c`),
  which upstream builds there, isn't translated yet.
* Vulkan (`video/drivers/uikit/vulkan.rs`) is MoltenVK's (or a Vulkan
  loader's): the library `SDL_HINT_VULKAN_LIBRARY` names, else the
  program's own `vkGetInstanceProcAddr`, else `libvulkan.dylib` (upstream's
  only default name on UIKit, though `SDL_vulkan.h` says
  `libMoltenVK.dylib`: a `Note (upstream)`), with surfaces on Metal views.
  An app bundles MoltenVK's framework (`Frameworks/MoltenVK.framework`)
  and names it in the hint. On visionOS nothing loads (above).

### Audio, controllers, sensors and the camera

* CoreAudio's UIKit half (upstream's `!MACOSX_COREAUDIO` parts) is
  `audio/drivers/coreaudio/session.rs`: the app's `AVAudioSession`, its
  category and options from the hints and what is open, and
  `SDLInterruptionListener`, declared at run time. There are no HAL
  devices, only the default ones, as upstream's. AVFoundation is linked.
* The MFi joystick driver (`joystick/apple`, `SDL_IOS_JoystickDriver`) is
  built on every Apple platform, as upstream's: on iOS and tvOS after the
  HIDAPI driver, which leaves it the controllers HIDAPI doesn't take (as
  on macOS), and on visionOS, which upstream builds without HIDAPI, first
  (there's no IOKit driver on any of them). Its iOS and tvOS paths are
  upstream's: the windows' game controller interaction set when it
  starts and cleared when it quits (`UIKit_SetGameControllerInteraction()`
  in the video driver), and on tvOS the Siri
  Remote's micro gamepad with `SDL_HINT_APPLE_TV_REMOTE_ALLOW_ROTATION`,
  the home button left to the system, the menu button through the pause
  handler, the remotes opened as joysticks counted for the view's presses
  (`SDL_AppleTVRemoteOpenedAsJoystick`), and the root view controller a
  `GCEventViewController` with `SDL_HINT_APPLE_TV_CONTROLLER_UI_EVENTS`.
  GameController and CoreHaptics are loaded at run time (CoreHaptics,
  which upstream links weakly, may be missing: before iOS 13 and tvOS 14
  there's no rumble). Rumble is CoreHaptics' on every platform, as
  upstream's; the haptic subsystem stays the dummy driver there.
* HIDAPI (`SDL_hidapi.c`, and the HIDAPI joystick and haptic drivers) is
  built on iOS and tvOS, not visionOS, as upstream's CMake builds it, with
  the backend of `hidapi/ios/hid.m` (`hidapi/ios.rs`, and its report
  handling in `hidapi/ble.rs`): Valve's Steam Controllers over Bluetooth
  LE. The HIDAPI joystick driver is off by default there
  (`SDL_HIDAPI_DEFAULT`), and so is the Steam driver
  (`SDL_HINT_JOYSTICK_HIDAPI_STEAM`'s default), and the backend only
  starts Bluetooth (a `CBCentralManager`, which asks the user, and needs
  `NSBluetoothAlwaysUsageDescription` in the app's Info.plist) when
  `SDL_HINT_JOYSTICK_HIDAPI_STEAM` is set as it's first used. Its classes
  are declared at run time under upstream's names (`HIDBLEManager`, a
  singleton, and `HIDBLEDevice`), with the properties' getters the tests
  read; the methods only the backend calls are Rust functions.
  CoreBluetooth is loaded at run time (`Note`: upstream's CMake build
  links it), so a program that doesn't start Bluetooth needn't have it,
  upstream's reason for `SDL_HIDAPI_DISABLED` there; without it its
  classes are nil, as a weakly linked framework's. The device map and the
  devices' state are locked (`Note`: upstream uses them from Bluetooth's
  queue and the caller's thread without a lock), and the reports are
  copied within their buffers (`FIXME (upstream)`: upstream copies whole
  reports into shorter buffers, and sends a byte past a feature report).
* The CoreMotion sensor driver (`sensor/coremotion.rs`) is built on iOS
  and visionOS, as upstream's CMake builds it, first in the sensor driver
  list (no dummy driver there); CoreMotion is linked.
* The CoreMedia camera driver (`camera/coremedia.rs`) is built on macOS
  and iOS (see "What is built"), before the dummy driver as upstream; in
  it `SDL_PLATFORM_IOS` and `USE_UIKIT_DEVICE_ROTATION` are
  `target_os = "ios"`. The device types the discovery session asks for
  are looked up at run time on both. A frame's rotation is upstream's
  tables, the interface's orientation from the UIKit video driver
  (`get_interface_orientation()`) and the device's from `UIDevice`, whose
  orientation notifications are on while a camera is open; an interface
  orientation outside the tables (unknown: upstream reads before them)
  leaves the frame unrotated (`FIXME (upstream)`), and the tables' rows
  are labelled by their values (`Note (upstream)`: upstream's comments swap
  the two landscape rows). The permission check's `@available(macOS 14, *)`
  is always taken on iOS, as upstream's.

### tvOS

tvOS (`target_os = "tvos"`) builds what upstream's tvOS build has: the
UIKit pieces less what tvOS hasn't, each upstream's `SDL_PLATFORM_TVOS`
branch a `cfg`. The video driver has no orientations, status bar, home
indicator, pinch gesture, pointer or Pencil (the pen's code is built and
never called, as upstream's), and observes no keyboard notifications;
its root view controller is a `GCEventViewController` (above), its
view's four `UISwipeGestureRecognizer`s turn the Siri Remote's swipes
into arrow keys (`swipeGesture:`), and its presses are the remote's
buttons, both only while no remote is opened as a joystick. tvOS has no
pasteboard: setting the clipboard's text fails with upstream's error, and
there's none to get. The pref path is in Caches (the system may empty
it: an Apple TV app has no other place to keep files) and there are no
user folders; the power state is "no battery". The Metal renderer takes
upstream's tvOS libraries, and the GPU API its iOS ones (above).

### visionOS

visionOS (`target_os = "visionos"`, flat windows only) builds what
upstream's CMake visionOS build has: the UIKit pieces, each upstream
`SDL_PLATFORM_VISIONOS` branch a `cfg`: no `UIScreen`, so one fake
1280x720-point display, `SDL_XR_SCREENWIDTH` by `SDL_XR_SCREENHEIGHT`,
of density 2 at 90 Hz; a window the size it asks for, the window scene
asked for that size through a `UIWindowSceneGeometryPreferencesVision`
when the window is made and resized; no status bar, interface
orientation or screen notifications; GCMouse scrolls clamped to a tick;
no depth clip mode in the GPU API (always clipped, as upstream asserts);
and every renderer's output colorspace linear sRGB
(`SDL_SetupRendererColorspace()`, `render::setup_renderer_colorspace()`),
so the software renderer can't be made there, as upstream's can't.

Upstream's Swift code (curved windows and their RealityKit texture, the
pointer mode, the settings and HDR headroom sliders, the immersive
spaces) isn't translated: SwiftUI and RealityKit have no Objective-C
interface. Its bridge (`SDL_UIKitBridge.m`) is
`video/drivers/uikit/bridge.rs`, whose functions the driver and the
Metal renderer call are stand-ins with what upstream's answer without a
curved window (`SDL_UIKit_HasCurvedWindow()` is false, the curved
texture nil, `initializeVisionOSCurvedUI` does nothing; `Note`), so a
window's settings (`SDL_PROP_WINDOW_CREATE_VISIONOS_SETTINGS_STRING`)
are kept and given back, but every window is a flat one; the bridge's
functions only the Swift code calls aren't translated.

## Android

Android builds and lints, and the tests run on the Android emulator as
spawned processes and as apps (`tools/android_run.py`: "Android emulator"
in docs/HARDWARE_TESTING.md). Its platform layer is begun: the JNI layer,
the app's start and lifecycle, the event pump's Android parts, the log,
and the small backends: the filesystem paths, the assets and
`content://` files, the locale, URLs, the battery, toasts and permissions
(below); the Android drivers (video, audio, input, ...) aren't
translated yet.

A test that reads its package's files does so at run time, through
`CARGO_MANIFEST_DIR` (falling back on `env!("CARGO_MANIFEST_DIR")`), which
the Android runner points to its copy of the workspace's test data on the
device; the same for `CARGO_TARGET_TMPDIR` and `CARGO_BIN_EXE_NAME`.

### Platform macros

| C | Rust |
|---|------|
| `SDL_PLATFORM_ANDROID`, `SDL_VIDEO_DRIVER_ANDROID` | `target_os = "android"` |
| `SDL_PLATFORM_LINUX` (not defined on Android) | `target_os = "linux"` |
| `__linux__` (defined on Android) | `any(target_os = "linux", target_os = "android")` |
| `SDL_PLATFORM_UNIX` (defined on Android) | `all(unix, not(target_vendor = "apple"))` |
| `__ANDROID_API__ >= N` | evaluated at upstream's floor, API 21 (so `HAVE_NL_LANGINFO` and `HAVE_ST_MTIM`, 26, are off) |

* The desktop Unix code (X11, Wayland, `core/linux`: D-Bus, the IME,
  udev, evdev) is `all(unix, not(target_vendor = "apple"), not(target_os
  = "android"), not(target_os = "emscripten"))`, as CMake's
  `UNIX AND NOT ANDROID` branches; `core/unix` isn't built either, as
  CMake's Android branch doesn't build it.
* The four ABIs are tier 2 targets, for the minimum supported Rust too:
  `aarch64-linux-android`, `x86_64-linux-android`, and the 32-bit
  `armv7-linux-androideabi` and `i686-linux-android`, where `time_t` and
  `c_long` are 32 bits, as upstream's 32-bit builds have them. `c_char` is
  unsigned on the ARM ABIs. There is no native thread-local storage
  (emulated TLS), which makes clippy misreport the `const` initializers
  of `thread_local!` (allowed per crate on Android).
* Where the libc crate lacks a constant of Bionic's headers (`SCHED_OTHER`
  is `SCHED_NORMAL`, the `RTMGRP_*` groups), it is defined where it is
  used, with its header's value.

### The Java, the app and its lifecycle

* Upstream's Java (android-project's `org.libsdl.app`) is translated to
  Rust too: `core/android/java/`, a module per file (`SDLActivity.java` is
  `java::activity`, `SDL.java` `java::sdl`), its calls to the framework
  JNI calls (`core::android::env`), its calls to upstream's natives and
  between its classes Rust calls, its statics a module's state. The
  framework still needs upstream's classes in the app's DEX file (the
  activity the manifest names, its handler, its listeners): they are
  declared in `core/android/classes.rs`, with upstream's names,
  superclasses and interfaces, and every method the framework calls (an
  override: `onCreate()`, `dispatchKeyEvent()`, `handleMessage()`, ...)
  and every one of upstream's natives is `native`, implemented by the
  Rust function the table names, which `JNI_OnLoad()` registers;
  `super.f()` is `CallNonvirtual*Method()`. To add one, add its entry,
  then write `classes_dex.rs` again (`SDL3_ANDROID_WRITE_DEX=1 cargo test
  -p sdl3 --lib core::android::dex`); the table's types are checked
  against the Rust function's at compile time.
* Upstream's anonymous classes (`new Runnable() {...}`, dialog listeners,
  the `OnBackInvokedCallback`) can't keep their compiler's names
  (`SDLActivity$1`): they are two classes of the DEX file carrying a Rust
  closure, `SDLNativeRunnable` (a `Runnable`, run once) and
  `SDLNativeListener` (the invocation handler of a
  `java.lang.reflect.Proxy` of the interface): `java::anonymous`.
* An instance field upstream's constructor initializes
  (`SDLActivity.commandHandler`) is set at the start of `onCreate()`
  instead (the constructors have no code of their own here). An app's
  Java can't extend `SDLActivity`, so the methods upstream's subclasses
  override (`getLibraries()`, `getArguments()`, `onUnhandledMessage()`)
  are fixed: `Note`s.
* What a later driver owns is stubbed where upstream calls it: the
  activity's surface (`java::surface`, null until `SDLSurface`), the
  video driver's window (`video::drivers::android::window()`, none), the
  keyboard, the audio drivers' pause, and the controller, HID, clipboard
  and audio managers (comments where upstream calls them).
* The app's `main` runs on the activity's "SDLThread" thread
  (`nativeRunMain()`, through the generic `SDL_RunApp()`; the main
  callbacks are the generic ones too), which isn't SDL's main thread until
  SDL is initialized there (`nativeSetupJNI()` makes the activity's thread
  the main one, as upstream's does). The activity passes its lifecycle to
  SDL's thread through the lifecycle event queue
  (`core::android::send_lifecycle_event()`), which SDL's event pump and
  `events::wait*()` read on Android, whatever the video driver
  (`video/drivers/android/events.rs`, SDL_androidevents.c), with
  `SDL_HINT_ANDROID_BLOCK_ON_PAUSE`'s blocking pause. SDL's threads attach
  themselves to the Java VM when they start (`Android_JNI_SetupThread()`).
* SDL_system.h's Android functions are the `sdl3::android` module
  (`jni_env()`, `activity()`, `sdk_version()`, `is_chromebook()`,
  `is_dex_mode()`, `send_back_button()`, `send_message()`,
  `internal_storage_path()`, `external_storage_state()`,
  `external_storage_path()`, `cache_path()`, `request_permission()`,
  `show_toast()`); a program that isn't an app has no Java VM, where they
  give `None`, false or the error `Failed, there is no JavaVM` (upstream
  dereferences a null environment). Where upstream returns false without
  setting an error (a toast without an activity, a URL no activity views),
  the error's message is empty: `Note`s.
* The app package's assets (SDL_android.c's APK tree and
  `AAssetManager` I/O) are `core::android::assets`, linked to
  `libandroid` as upstream links it: `IoStream::from_file()` opens an
  absolute path on the filesystem, a `content://` URI through the content
  resolver, a relative one in the internal storage, and otherwise (and
  `assets://` paths) the asset; the file operations take relative paths in
  the internal storage, and fall back to the assets for path info (an
  absolute path's too), as upstream's `SDL_sysfsops.c` does. A program
  that isn't an app has no assets, and no internal storage: its relative
  paths are the current directory's, as on the other Unix platforms (a
  `Note`: upstream needs an app there).
* The log's default output writes to Android's log too
  (`__android_log_write()` of liblog, the tag `SDL/CATEGORY`), then to
  stderr, as upstream's does.

### What is built

As upstream's CMake build for Android, except where Android's own backend
isn't translated yet, where the dummy one (or none, as upstream without
it) stands in:

| | Upstream | Here |
|---|---|---|
| video | android, offscreen, dummy | offscreen and dummy (of the android driver, its events and the system theme, whatever the driver: the rest later) |
| OpenGL ES, EGL | GLES 1 and 2 through EGL (`libEGL.so`, `libGLESv2.so`, `libGLESv1_CM.so`), no desktop OpenGL | the same (the offscreen driver's EGL) |
| Vulkan | `libvulkan.so` | the same (the offscreen driver's: `FIXME (upstream)`, which tries `libvulkan.so.1`) |
| renderers | OpenGL ES 2, OpenGL ES, Vulkan, GPU, software | OpenGL ES 2, Vulkan, GPU, software (the OpenGL ES 1 renderer isn't translated anywhere) |
| audio | AAudio, OpenSL ES | dummy and disk (later) |
| joysticks | android, HIDAPI (off by default), virtual | HIDAPI with no HID backend (the Android one: later), virtual |
| haptic | android, HIDAPI | dummy (the android driver: later), HIDAPI |
| power, locale, URL, filesystem paths, assets | Android's | the same |
| sensor, camera, dialog | Android's | dummy, or no backend (later) |
| file operations, threads, time, timer, `loadso` | POSIX, pthread, Unix, Unix, `dlopen` | the same |
| process, tray, notification | dummy, dummy, none | the same |

## Testing

Each module carries unit tests that pin behaviour to upstream's: known hash
vectors, the hex values from `SDL_pixels.h`, generated lookup tables, edge
cases taken from the C comments. Prefer tests that would fail if the
translation drifted from C over tests that only exercise the Rust API.
