// Tests of OpenGL ES on the UIKit video driver: its contexts and their
// views, and the OpenGL ES 2 renderer on its windows.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The UIKit video driver only starts in an app, so this is a program of
//! its own (`harness = false`), run as an app on the iOS Simulator by
//! `tools/ios_run.py` (the `__DATA,__sdl3_runner` section below), as the
//! other `uikit_*` programs are. Its main function traces:
//!
//! * the OpenGL attributes' defaults (OpenGL ES 2.0), and window D, which
//!   asks for no graphics API and gets the OpenGL flag;
//! * contexts on D for OpenGL ES 2.0, 3.0 and 1.1 (and 3.1 and 4.0, which
//!   EAGL hasn't): what is current (SDL's and EAGL's context, its API),
//!   the GL strings, the attributes as SDL reads them back, the window's
//!   `SDL_PROP_WINDOW_UIKIT_OPENGL_*_NUMBER` properties against the
//!   bindings, the view (an `SDL_uikitopenglview` that becomes the view
//!   controller's view, its `CAEAGLLayer`'s scale and drawable
//!   properties, its renderbuffer's size), a clear read back, a swap, the
//!   swap interval (which UIKit doesn't set), and the context destroyed
//!   (the window's own view comes back);
//! * contexts sharing a share group (a texture of one is the other's), one
//!   that doesn't, and the first made current again (its view, which is
//!   still among the window's, comes back once the others are destroyed,
//!   as upstream's `setSDLWindow:` does nothing for a view already in the
//!   window);
//! * the framebuffer's formats: sRGB (asked for, and forced either way by
//!   `SDL_HINT_OPENGL_FORCE_SRGB_FRAMEBUFFER`), RGB565, depth and
//!   stencil, a drawable that isn't retained, and multisampling with
//!   OpenGL ES 2.0 and 3.0 (its draw framebuffer, resolved at each swap);
//! * the current context restored: by the event pump and before the
//!   animation callbacks (`UIKit_GL_RestoreCurrentContext()`), and by
//!   raising the window, after another EAGL context was made current;
//! * the battery monitoring the swaps turn off: a power query turns it on,
//!   and the first swap more than 3 s later turns it off; the simulator's
//!   UIDevice keeps it off whatever it's told, so upstream's assertion
//!   that it was on fails there (`FIXME (upstream)` in
//!   `power/uikit.rs`). C and Rust both run with an assertion handler
//!   that counts the failures and ignores them (the runner's
//!   `SDL_ASSERT=abort` is the default handler's);
//! * window H (OpenGL, resizable, high pixel density): an OpenGL ES 3.0
//!   context's renderbuffers following the window as it turns in-process
//!   to landscape right and back to portrait
//!   (`requestGeometryUpdateWithPreferences:`, iOS 16);
//! * the OpenGL ES 2 renderer on H: its name, output size, maximum texture
//!   size, texture formats, vsync, its context and view; the scenes the
//!   other renderers' tests draw (`uikit_support::scenes`, as
//!   `uikit_metal` draws them) and partial reads, each read back and
//!   hashed; vsync 1, 0, 2 and -1 (UIKit sets no swap interval); the
//!   output following the window as it turns; and the renderer destroyed;
//! * the OpenGL ES 2 renderer on Metal window M, which is made again with
//!   the OpenGL flag (UIKit can't reconfigure a window), and the default
//!   renderer (Metal) on window W, which asks for no graphics API: it is
//!   made again with the Metal flag.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! run the same way on the same simulator (an iPhone 16 with iOS 18.5):
//! the GL strings are the simulator's ("Apple Software Renderer"), and the
//! hashes are of the same pixels. Each scene is also drawn by the software
//! renderer and compared with it (within the tolerances the other
//! renderers' tests allow), and the custom blend mode's pixels are
//! checked. The mice's device events are left out. Upstream's renderer
//! frees a program cache entry twice when it's destroyed after these
//! scenes (its cache list loses its tail: the `Note (upstream)` on the
//! Rust renderer's cache, a `Vec`), and malloc aborts, so the C program is
//! built with that list's two missing updates added. If OpenGL ES is
//! missing (Apple deprecated it), the program skips (capability `gles`),
//! unless `SDL3_TEST_REQUIRE` names it.
//!
//! It runs on the tvOS Simulator too (an Apple TV 4K (3rd generation) with
//! tvOS 18.5), where nothing turns (`turn` only traces that), against
//! upstream's C there, built with OpenGL ES as the Xcode project builds it
//! on tvOS (upstream's CMake build leaves it out there). Elsewhere this
//! program does nothing.

#[cfg(not(any(target_os = "ios", target_os = "tvos")))]
fn main() {}

/// The runner's words: run as an app.
#[cfg(any(target_os = "ios", target_os = "tvos"))]
#[used]
#[link_section = "__DATA,__sdl3_runner"]
static RUNNER: [u8; 4] = *b"app\0";

#[cfg(any(target_os = "ios", target_os = "tvos"))]
mod uikit_support;

#[cfg(any(target_os = "ios", target_os = "tvos"))]
fn main() {
    ios::main()
}

#[cfg(any(target_os = "ios", target_os = "tvos"))]
mod ios {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event, WindowID};
    use sdl3::init::{self, InitFlags};
    use sdl3::render::Renderer;
    use sdl3::video::gl::{self, GlAttr, GlContext};
    use sdl3::video::{self, Rect, Window};

    use super::uikit_support::objc::{self, Id};
    use super::uikit_support::scenes::{
        check_custom_blend, clear_and_fill, hash_read, scene, scenes, t, MISMATCHES, RH, RW, TRACE,
    };
    use super::uikit_support::{check_trace, checked, elapsed, event_name, exit};

    /// Upstream's trace (the C program, on an iPhone 16 simulator with
    /// iOS 18.5).
    #[cfg(target_os = "ios")]
    const EXPECTED: &str = "\
init video: ok
video driver: uikit
application state: 0
defaults: attributes: double=1 accelerated=1 retained=1 major=2 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0
D: create 320x480: ok
D: created: events: D:WINDOW_SAFE_AREA_CHANGED(0,0), D:WINDOW_MOUSE_ENTER(0,0), D:WINDOW_FOCUS_GAINED(0,0), D:WINDOW_SHOWN(0,0), D:WINDOW_PIXEL_SIZE_CHANGED(393,852)
D: created: size=393x852 pixels=393x852 density=1 flags=0x602
D: created: root view: SDL_uikitview frame=0,0 393x852
D: es 2.0: create context: ok
D: es 2.0: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=2
D: es 2.0: version=OpenGL ES 2.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 1.00
D: es 2.0: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=2 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: es 2.0: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
D: es 2.0: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 393x852 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormat8888
D: es 2.0: renderbuffer: 393x852 format=0x8058 pixels=393x852
D: es 2.0: samples=0 max_samples=4 error=0x0
D: es 2.0: clear: pixel=[64, 128, 191, 255] error=0x0; swap: ok, error=0x0
D: es 2.0: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: es 2.0: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 1 1 0
D: es 3.0: create context: ok
D: es 3.0: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=3
D: es 3.0: version=OpenGL ES 3.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 3.00
D: es 3.0: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=3 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: es 3.0: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
D: es 3.0: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 393x852 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormat8888
D: es 3.0: renderbuffer: 393x852 format=0x8058 pixels=393x852
D: es 3.0: samples=0 max_samples=9 error=0x0
D: es 3.0: clear: pixel=[64, 128, 191, 255] error=0x0; swap: ok, error=0x0
D: es 3.0: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: es 3.0: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 1 1 0
D: es 1.1: create context: ok
D: es 1.1: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=1
D: es 1.1: version=OpenGL ES-CM 1.1 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=-
D: es 1.1: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=1 minor=1 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: es 1.1: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
D: es 1.1: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 393x852 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormat8888
D: es 1.1: renderbuffer: 393x852 format=0x8058 pixels=393x852
D: es 1.1: samples=0 max_samples=4 error=0x0
D: es 1.1: clear: pixel=[64, 128, 191, 255] error=0x0; swap: ok, error=0x0
D: es 1.1: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: es 1.1: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 1 1 0
D: es 3.1: create context: error: OpenGL ES 3.1 context could not be created
D: es 4.0: create context: error: OpenGL ES 4.0 context could not be created
D: versions: events: D:WINDOW_SAFE_AREA_CHANGED(0,0)
D: context A: ok
D: context B (shared): same group=yes texture=yes current=yes root view is A's=no
D: context C (not shared): same group=no texture=no
D: A current again: ok texture=yes root view is A's=no eagl=yes
D: B and C destroyed: A current=yes root view is A's=yes
D: pumped after another context: ours=yes
D: animation callbacks after another context: ours=yes, yes
D: raised after another context: ok ours=yes
D: power: state=Unknown percent=None
D: battery: assertions: swap at once 0, after 3 s 1, again 1
D: A destroyed: raised: ok eagl=no root view: SDL_uikitview
D: contexts: events: D:WINDOW_SAFE_AREA_CHANGED(0,0)
D: srgb: create context: ok
D: srgb: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=2
D: srgb: version=OpenGL ES 2.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 1.00
D: srgb: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=2 minor=0 profile=4 share=0 srgb=1 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: srgb: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
D: srgb: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 393x852 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormatS8888
D: srgb: renderbuffer: 393x852 format=0x8c43 pixels=393x852
D: srgb: samples=0 max_samples=4 error=0x0
D: srgb: clear: pixel=[137, 188, 225, 255] error=0x0; swap: ok, error=0x0
D: srgb: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: srgb: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 1 1 0
D: srgb forced off: create context: ok
D: srgb forced off: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=2
D: srgb forced off: version=OpenGL ES 2.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 1.00
D: srgb forced off: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=2 minor=0 profile=4 share=0 srgb=1 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: srgb forced off: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
D: srgb forced off: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 393x852 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormat8888
D: srgb forced off: renderbuffer: 393x852 format=0x8058 pixels=393x852
D: srgb forced off: samples=0 max_samples=4 error=0x0
D: srgb forced off: clear: pixel=[64, 128, 191, 255] error=0x0; swap: ok, error=0x0
D: srgb forced off: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: srgb forced off: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 1 1 0
D: srgb forced on: create context: ok
D: srgb forced on: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=2
D: srgb forced on: version=OpenGL ES 2.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 1.00
D: srgb forced on: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=2 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: srgb forced on: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
D: srgb forced on: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 393x852 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormatS8888
D: srgb forced on: renderbuffer: 393x852 format=0x8c43 pixels=393x852
D: srgb forced on: samples=0 max_samples=4 error=0x0
D: srgb forced on: clear: pixel=[137, 188, 225, 255] error=0x0; swap: ok, error=0x0
D: srgb forced on: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: srgb forced on: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 1 1 0
D: rgb565: create context: ok
D: rgb565: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=2
D: rgb565: version=OpenGL ES 2.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 1.00
D: rgb565: attributes: red=8 green=8 blue=8 alpha=0 buffer=24 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=2 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: rgb565: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
D: rgb565: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 393x852 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormat565
D: rgb565: renderbuffer: 393x852 format=0x8051 pixels=393x852
D: rgb565: samples=0 max_samples=4 error=0x0
D: rgb565: clear: pixel=[64, 128, 191, 255] error=0x0; swap: ok, error=0x0
D: rgb565: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: rgb565: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 1 1 0
D: depth 24 stencil 8: create context: ok
D: depth 24 stencil 8: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=2
D: depth 24 stencil 8: version=OpenGL ES 2.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 1.00
D: depth 24 stencil 8: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=8 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=2 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: depth 24 stencil 8: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
D: depth 24 stencil 8: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 393x852 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormat8888
D: depth 24 stencil 8: renderbuffer: 393x852 format=0x8058 pixels=393x852
D: depth 24 stencil 8: samples=0 max_samples=4 error=0x0
D: depth 24 stencil 8: clear: pixel=[64, 128, 191, 255] error=0x0; swap: ok, error=0x0
D: depth 24 stencil 8: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: depth 24 stencil 8: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 1 1 0
D: no depth, not retained: create context: ok
D: no depth, not retained: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=2
D: no depth, not retained: version=OpenGL ES 2.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 1.00
D: no depth, not retained: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=0 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=0 major=2 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: no depth, not retained: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
D: no depth, not retained: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 393x852 layer: CAEAGLLayer scale=1 opaque=yes retained=no format=EAGLColorFormat8888
D: no depth, not retained: renderbuffer: 393x852 format=0x8058 pixels=393x852
D: no depth, not retained: samples=0 max_samples=4 error=0x0
D: no depth, not retained: clear: pixel=[64, 128, 191, 255] error=0x0; swap: ok, error=0x0
D: no depth, not retained: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: no depth, not retained: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 1 1 0
D: msaa 4 es 2.0: create context: ok
D: msaa 4 es 2.0: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=2
D: msaa 4 es 2.0: version=OpenGL ES 2.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 1.00
D: msaa 4 es 2.0: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=1 mssamples=4 accelerated=1 retained=1 major=2 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: msaa 4 es 2.0: props: framebuffer=2 renderbuffer=1 resolve=1 bound framebuffer=2 renderbuffer=1
D: msaa 4 es 2.0: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 393x852 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormat8888
D: msaa 4 es 2.0: renderbuffer: 393x852 format=0x8058 pixels=393x852
D: msaa 4 es 2.0: samples=4 max_samples=4 error=0x0
D: msaa 4 es 2.0: clear: pixel=- error=0x0; swap: ok, error=0x0
D: msaa 4 es 2.0: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: msaa 4 es 2.0: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 2 1 1
D: msaa 4 es 3.0: create context: ok
D: msaa 4 es 3.0: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=3
D: msaa 4 es 3.0: version=OpenGL ES 3.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 3.00
D: msaa 4 es 3.0: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=1 mssamples=4 accelerated=1 retained=1 major=3 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: msaa 4 es 3.0: props: framebuffer=2 renderbuffer=1 resolve=1 bound framebuffer=2 renderbuffer=1
D: msaa 4 es 3.0: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 393x852 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormat8888
D: msaa 4 es 3.0: renderbuffer: 393x852 format=0x8058 pixels=393x852
D: msaa 4 es 3.0: samples=4 max_samples=9 error=0x0
D: msaa 4 es 3.0: clear: pixel=- error=0x0; swap: ok, error=0x0
D: msaa 4 es 3.0: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: msaa 4 es 3.0: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 2 1 1
D: msaa 64 es 3.0 not retained: create context: ok
D: msaa 64 es 3.0 not retained: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=3
D: msaa 64 es 3.0 not retained: version=OpenGL ES 3.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 3.00
D: msaa 64 es 3.0 not retained: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=1 mssamples=9 accelerated=1 retained=0 major=3 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: msaa 64 es 3.0 not retained: props: framebuffer=2 renderbuffer=1 resolve=1 bound framebuffer=2 renderbuffer=1
D: msaa 64 es 3.0 not retained: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 393x852 layer: CAEAGLLayer scale=1 opaque=yes retained=no format=EAGLColorFormat8888
D: msaa 64 es 3.0 not retained: renderbuffer: 393x852 format=0x8058 pixels=393x852
D: msaa 64 es 3.0 not retained: samples=9 max_samples=9 error=0x0
D: msaa 64 es 3.0 not retained: clear: pixel=- error=0x0; swap: ok, error=0x0
D: msaa 64 es 3.0 not retained: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: msaa 64 es 3.0 not retained: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 2 1 1
D: msaa 64 es 2.0 not retained: create context: ok
D: msaa 64 es 2.0 not retained: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=2
D: msaa 64 es 2.0 not retained: version=OpenGL ES 2.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 1.00
D: msaa 64 es 2.0 not retained: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=1 mssamples=4 accelerated=1 retained=0 major=2 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: msaa 64 es 2.0 not retained: props: framebuffer=2 renderbuffer=1 resolve=1 bound framebuffer=2 renderbuffer=1
D: msaa 64 es 2.0 not retained: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 393x852 layer: CAEAGLLayer scale=1 opaque=yes retained=no format=EAGLColorFormat8888
D: msaa 64 es 2.0 not retained: renderbuffer: 393x852 format=0x8058 pixels=393x852
D: msaa 64 es 2.0 not retained: samples=4 max_samples=4 error=0x0
D: msaa 64 es 2.0 not retained: clear: pixel=- error=0x0; swap: ok, error=0x0
D: msaa 64 es 2.0 not retained: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: msaa 64 es 2.0 not retained: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 2 1 1
D: formats: events: D:WINDOW_SAFE_AREA_CHANGED(0,0)
D: destroyed
D: destroyed: events: D:WINDOW_DESTROYED(0,0)
H: create 320x480 opengl resizable high-density: ok
H: created: events: H:WINDOW_SAFE_AREA_CHANGED(0,0), H:WINDOW_MOUSE_ENTER(0,0), H:WINDOW_FOCUS_GAINED(0,0), H:WINDOW_SHOWN(0,0), H:WINDOW_PIXEL_SIZE_CHANGED(1179,2556), H:WINDOW_DISPLAY_SCALE_CHANGED(0,0)
H: created: size=393x852 pixels=1179x2556 density=3 flags=0x2622
H: created: root view: SDL_uikitview frame=0,0 393x852
H: create es 3.0 context: ok
H: context: events: H:WINDOW_SAFE_AREA_CHANGED(0,0)
H: context: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=3
H: context: version=OpenGL ES 3.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 3.00
H: context: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=3 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
H: context: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
H: context: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 393x852 layer: CAEAGLLayer scale=3 opaque=yes retained=yes format=EAGLColorFormat8888
H: context: renderbuffer: 1179x2556 format=0x8058 pixels=1179x2556
H: context: samples=0 max_samples=9 error=0x0
H: context: clear: pixel=[64, 128, 191, 255] error=0x0; swap: ok, error=0x0
H: landscape right: events: DISPLAY_DESKTOP_MODE_CHANGED(1,852), DISPLAY_CURRENT_MODE_CHANGED(1,852), DISPLAY_ORIENTATION(1,1), H:WINDOW_RESIZED(852,393), H:WINDOW_PIXEL_SIZE_CHANGED(2556,1179), H:WINDOW_SAFE_AREA_CHANGED(0,0)
H: landscape right: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=3
H: landscape right: version=OpenGL ES 3.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 3.00
H: landscape right: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=3 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
H: landscape right: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
H: landscape right: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 852x393 layer: CAEAGLLayer scale=3 opaque=yes retained=yes format=EAGLColorFormat8888
H: landscape right: renderbuffer: 2556x1179 format=0x8058 pixels=2556x1179
H: landscape right: samples=0 max_samples=9 error=0x0
H: landscape right: clear: pixel=[64, 128, 191, 255] error=0x0; swap: ok, error=0x0
H: portrait: events: DISPLAY_DESKTOP_MODE_CHANGED(1,393), DISPLAY_CURRENT_MODE_CHANGED(1,393), DISPLAY_ORIENTATION(1,3), H:WINDOW_RESIZED(393,852), H:WINDOW_PIXEL_SIZE_CHANGED(1179,2556), H:WINDOW_SAFE_AREA_CHANGED(0,0)
H: portrait: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=3
H: portrait: version=OpenGL ES 3.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 3.00
H: portrait: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=3 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
H: portrait: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
H: portrait: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 393x852 layer: CAEAGLLayer scale=3 opaque=yes retained=yes format=EAGLColorFormat8888
H: portrait: renderbuffer: 1179x2556 format=0x8058 pixels=1179x2556
H: portrait: samples=0 max_samples=9 error=0x0
H: context destroyed
H: context destroyed: events: H:WINDOW_SAFE_AREA_CHANGED(0,0)
H: context destroyed: size=393x852 pixels=1179x2556 density=3 flags=0x2622
H: context destroyed: root view: SDL_uikitview frame=0,0 393x852
H: create opengles2 renderer: ok
H: renderer: events: H:WINDOW_SAFE_AREA_CHANGED(0,0)
H: renderer: name=opengles2 output=1179x2556 pixels=1179x2556 max_texture_size=4096 vsync=0
H: renderer: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_XRGB8888, SDL_PIXELFORMAT_XBGR8888, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21
H: renderer: context: SDLEAGLContext eagl=yes window flags=0x2622 root view: SDL_uikitopenglview props: 1 1 0 bound framebuffer=1
scene clear and fill: 64x48 5fe1f24a
scene points and lines: 64x48 75192d12
scene viewport and clip: 64x48 b09f79e5
scene blend modes: 64x48 ebb10095
scene texture SDL_PIXELFORMAT_ARGB8888: 64x48 a2cf2633
scene texture SDL_PIXELFORMAT_ABGR8888: 64x48 a2cf2633
scene texture SDL_PIXELFORMAT_RGB24: 64x48 4fb014c5
scene texture SDL_PIXELFORMAT_XRGB8888: 64x48 4fb014c5
scene streaming texture: 64x48 c2319ec5
scene render target: 64x48 9e5285c5
scene packed SDL_PIXELFORMAT_ABGR2101010: 64x48 8bdc6af5
scene packed SDL_PIXELFORMAT_RGBA64_FLOAT: 64x48 9a74c64d
scene packed SDL_PIXELFORMAT_RGBA128_FLOAT: 64x48 9a74c64d
scene palette texture: 64x48 65a75445
scene yuv SDL_PIXELFORMAT_IYUV: 64x48 785ae64d
scene yuv SDL_PIXELFORMAT_YV12: 64x48 785ae64d
scene yuv SDL_PIXELFORMAT_NV12: 64x48 785ae64d
scene yuv SDL_PIXELFORMAT_NV21: 64x48 785ae64d
scene IYUV planes: 64x48 785ae64d
scene NV12 planes: 64x48 785ae64d
scene locked texture: 64x48 76d14245
scene locked NV12 texture: 64x48 785ae64d
scene custom blend mode: 64x48 af879425
scene wrapping geometry: 64x48 dd468f45
scene colored geometry: 64x48 7a2c48ad
scene linear scaling: 64x48 5599895b
scene debug text: 64x48 73f3e17d
partial read: 8x8 f24b7925
target read: 20x10 da6561f5
H: vsync 1: ok, vsync=1
H: vsync 0: ok, vsync=0
H: vsync 2: error: That operation is not supported, vsync=0
H: vsync -1: error: That operation is not supported, vsync=0
H: scenes: events: (none)
H: renderer landscape right: events: DISPLAY_DESKTOP_MODE_CHANGED(1,852), DISPLAY_CURRENT_MODE_CHANGED(1,852), DISPLAY_ORIENTATION(1,1), H:WINDOW_RESIZED(852,393), H:WINDOW_PIXEL_SIZE_CHANGED(2556,1179), H:WINDOW_SAFE_AREA_CHANGED(0,0)
H: renderer landscape right: name=opengles2 output=2556x1179 pixels=2556x1179 max_texture_size=4096 vsync=0
H: renderer landscape right: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_XRGB8888, SDL_PIXELFORMAT_XBGR8888, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21
H: renderer landscape right: context: SDLEAGLContext eagl=yes window flags=0x2622 root view: SDL_uikitopenglview props: 1 1 0 bound framebuffer=1
H: renderer landscape right corner: 64x48 bf597dc5
H: renderer portrait: events: DISPLAY_DESKTOP_MODE_CHANGED(1,393), DISPLAY_CURRENT_MODE_CHANGED(1,393), DISPLAY_ORIENTATION(1,3), H:WINDOW_RESIZED(393,852), H:WINDOW_PIXEL_SIZE_CHANGED(1179,2556), H:WINDOW_SAFE_AREA_CHANGED(0,0)
H: renderer portrait: name=opengles2 output=1179x2556 pixels=1179x2556 max_texture_size=4096 vsync=0
H: renderer portrait: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_XRGB8888, SDL_PIXELFORMAT_XBGR8888, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21
H: renderer portrait: context: SDLEAGLContext eagl=yes window flags=0x2622 root view: SDL_uikitopenglview props: 1 1 0 bound framebuffer=1
scene again clear and fill: 64x48 5fe1f24a
H: renderer destroyed
H: renderer destroyed: events: H:WINDOW_SAFE_AREA_CHANGED(0,0)
H: renderer destroyed: size=393x852 pixels=1179x2556 density=3 flags=0x2622
H: renderer destroyed: root view: SDL_uikitview frame=0,0 393x852
H: destroyed
H: destroyed: events: H:WINDOW_DESTROYED(0,0)
M: create 320x480 metal: ok
M: created: events: M:WINDOW_SAFE_AREA_CHANGED(0,0), M:WINDOW_MOUSE_ENTER(0,0), M:WINDOW_FOCUS_GAINED(0,0), M:WINDOW_SHOWN(0,0), M:WINDOW_PIXEL_SIZE_CHANGED(393,852)
M: created: size=393x852 pixels=393x852 density=1 flags=0x20000600
M: created: root view: SDL_uikitview frame=0,0 393x852
M: create opengles2 renderer: ok
M: renderer: events: M:WINDOW_HIDDEN(0,0), M:WINDOW_FOCUS_GAINED(0,0), M:WINDOW_SHOWN(0,0), M:WINDOW_SAFE_AREA_CHANGED(0,0)
M: renderer: size=393x852 pixels=393x852 density=1 flags=0x202
M: renderer: root view: SDL_uikitopenglview frame=0,0 393x852
M: renderer: name=opengles2 output=393x852 pixels=393x852 max_texture_size=4096 vsync=0
M: renderer: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_XRGB8888, SDL_PIXELFORMAT_XBGR8888, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21
M: renderer: context: SDLEAGLContext eagl=yes window flags=0x202 root view: SDL_uikitopenglview props: 1 1 0 bound framebuffer=1
scene M clear and fill: 64x48 5fe1f24a
M: renderer destroyed: events: M:WINDOW_SAFE_AREA_CHANGED(0,0)
M: destroyed
M: destroyed: events: M:WINDOW_DESTROYED(0,0)
W: create 320x480: ok
W: created: events: W:WINDOW_SAFE_AREA_CHANGED(0,0), W:WINDOW_MOUSE_ENTER(0,0), W:WINDOW_FOCUS_GAINED(0,0), W:WINDOW_SHOWN(0,0), W:WINDOW_PIXEL_SIZE_CHANGED(393,852)
W: created: size=393x852 pixels=393x852 density=1 flags=0x602
W: created: root view: SDL_uikitview frame=0,0 393x852
W: create renderer: ok
W: renderer: events: W:WINDOW_SAFE_AREA_CHANGED(0,0)
W: renderer: name=metal
W: renderer: size=393x852 pixels=393x852 density=1 flags=0x20000600
W: renderer: root view: SDL_uikitmetalview frame=0,0 393x852
W: renderer destroyed: events: (none)
W: destroyed
W: destroyed: events: W:WINDOW_DESTROYED(0,0)
quit
";

    /// Upstream's trace on tvOS (the C program, on an Apple TV 4K (3rd
    /// generation) simulator with tvOS 18.5).
    #[cfg(target_os = "tvos")]
    const EXPECTED: &str = "\
init video: ok
video driver: uikit
application state: 0
defaults: attributes: double=1 accelerated=1 retained=1 major=2 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0
D: create 320x480: ok
D: created: events: D:WINDOW_SAFE_AREA_CHANGED(0,0), D:WINDOW_MOUSE_ENTER(0,0), D:WINDOW_FOCUS_GAINED(0,0), D:WINDOW_SHOWN(0,0), D:WINDOW_PIXEL_SIZE_CHANGED(1920,1080)
D: created: size=1920x1080 pixels=1920x1080 density=1 flags=0x602
D: created: root view: SDL_uikitview frame=0,0 1920x1080
D: es 2.0: create context: ok
D: es 2.0: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=2
D: es 2.0: version=OpenGL ES 2.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 1.00
D: es 2.0: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=2 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: es 2.0: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
D: es 2.0: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 1920x1080 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormat8888
D: es 2.0: renderbuffer: 1920x1080 format=0x8058 pixels=1920x1080
D: es 2.0: samples=0 max_samples=4 error=0x0
D: es 2.0: clear: pixel=[64, 128, 191, 255] error=0x0; swap: ok, error=0x0
D: es 2.0: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: es 2.0: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 1 1 0
D: es 3.0: create context: ok
D: es 3.0: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=3
D: es 3.0: version=OpenGL ES 3.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 3.00
D: es 3.0: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=3 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: es 3.0: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
D: es 3.0: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 1920x1080 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormat8888
D: es 3.0: renderbuffer: 1920x1080 format=0x8058 pixels=1920x1080
D: es 3.0: samples=0 max_samples=9 error=0x0
D: es 3.0: clear: pixel=[64, 128, 191, 255] error=0x0; swap: ok, error=0x0
D: es 3.0: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: es 3.0: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 1 1 0
D: es 1.1: create context: ok
D: es 1.1: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=1
D: es 1.1: version=OpenGL ES-CM 1.1 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=-
D: es 1.1: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=1 minor=1 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: es 1.1: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
D: es 1.1: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 1920x1080 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormat8888
D: es 1.1: renderbuffer: 1920x1080 format=0x8058 pixels=1920x1080
D: es 1.1: samples=0 max_samples=4 error=0x0
D: es 1.1: clear: pixel=[64, 128, 191, 255] error=0x0; swap: ok, error=0x0
D: es 1.1: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: es 1.1: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 1 1 0
D: es 3.1: create context: error: OpenGL ES 3.1 context could not be created
D: es 4.0: create context: error: OpenGL ES 4.0 context could not be created
D: versions: events: (none)
D: context A: ok
D: context B (shared): same group=yes texture=yes current=yes root view is A's=no
D: context C (not shared): same group=no texture=no
D: A current again: ok texture=yes root view is A's=no eagl=yes
D: B and C destroyed: A current=yes root view is A's=yes
D: pumped after another context: ours=yes
D: animation callbacks after another context: ours=yes, yes
D: raised after another context: ok ours=yes
D: power: state=NoBattery percent=None
D: battery: assertions: swap at once 0, after 3 s 0, again 0
D: A destroyed: raised: ok eagl=no root view: SDL_uikitview
D: contexts: events: (none)
D: srgb: create context: ok
D: srgb: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=2
D: srgb: version=OpenGL ES 2.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 1.00
D: srgb: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=2 minor=0 profile=4 share=0 srgb=1 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: srgb: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
D: srgb: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 1920x1080 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormatS8888
D: srgb: renderbuffer: 1920x1080 format=0x8c43 pixels=1920x1080
D: srgb: samples=0 max_samples=4 error=0x0
D: srgb: clear: pixel=[137, 188, 225, 255] error=0x0; swap: ok, error=0x0
D: srgb: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: srgb: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 1 1 0
D: srgb forced off: create context: ok
D: srgb forced off: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=2
D: srgb forced off: version=OpenGL ES 2.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 1.00
D: srgb forced off: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=2 minor=0 profile=4 share=0 srgb=1 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: srgb forced off: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
D: srgb forced off: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 1920x1080 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormat8888
D: srgb forced off: renderbuffer: 1920x1080 format=0x8058 pixels=1920x1080
D: srgb forced off: samples=0 max_samples=4 error=0x0
D: srgb forced off: clear: pixel=[64, 128, 191, 255] error=0x0; swap: ok, error=0x0
D: srgb forced off: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: srgb forced off: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 1 1 0
D: srgb forced on: create context: ok
D: srgb forced on: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=2
D: srgb forced on: version=OpenGL ES 2.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 1.00
D: srgb forced on: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=2 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: srgb forced on: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
D: srgb forced on: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 1920x1080 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormatS8888
D: srgb forced on: renderbuffer: 1920x1080 format=0x8c43 pixels=1920x1080
D: srgb forced on: samples=0 max_samples=4 error=0x0
D: srgb forced on: clear: pixel=[137, 188, 225, 255] error=0x0; swap: ok, error=0x0
D: srgb forced on: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: srgb forced on: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 1 1 0
D: rgb565: create context: ok
D: rgb565: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=2
D: rgb565: version=OpenGL ES 2.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 1.00
D: rgb565: attributes: red=8 green=8 blue=8 alpha=0 buffer=24 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=2 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: rgb565: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
D: rgb565: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 1920x1080 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormat565
D: rgb565: renderbuffer: 1920x1080 format=0x8051 pixels=1920x1080
D: rgb565: samples=0 max_samples=4 error=0x0
D: rgb565: clear: pixel=[64, 128, 191, 255] error=0x0; swap: ok, error=0x0
D: rgb565: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: rgb565: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 1 1 0
D: depth 24 stencil 8: create context: ok
D: depth 24 stencil 8: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=2
D: depth 24 stencil 8: version=OpenGL ES 2.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 1.00
D: depth 24 stencil 8: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=8 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=2 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: depth 24 stencil 8: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
D: depth 24 stencil 8: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 1920x1080 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormat8888
D: depth 24 stencil 8: renderbuffer: 1920x1080 format=0x8058 pixels=1920x1080
D: depth 24 stencil 8: samples=0 max_samples=4 error=0x0
D: depth 24 stencil 8: clear: pixel=[64, 128, 191, 255] error=0x0; swap: ok, error=0x0
D: depth 24 stencil 8: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: depth 24 stencil 8: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 1 1 0
D: no depth, not retained: create context: ok
D: no depth, not retained: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=2
D: no depth, not retained: version=OpenGL ES 2.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 1.00
D: no depth, not retained: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=0 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=0 major=2 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: no depth, not retained: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
D: no depth, not retained: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 1920x1080 layer: CAEAGLLayer scale=1 opaque=yes retained=no format=EAGLColorFormat8888
D: no depth, not retained: renderbuffer: 1920x1080 format=0x8058 pixels=1920x1080
D: no depth, not retained: samples=0 max_samples=4 error=0x0
D: no depth, not retained: clear: pixel=[64, 128, 191, 255] error=0x0; swap: ok, error=0x0
D: no depth, not retained: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: no depth, not retained: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 1 1 0
D: msaa 4 es 2.0: create context: ok
D: msaa 4 es 2.0: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=2
D: msaa 4 es 2.0: version=OpenGL ES 2.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 1.00
D: msaa 4 es 2.0: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=1 mssamples=4 accelerated=1 retained=1 major=2 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: msaa 4 es 2.0: props: framebuffer=2 renderbuffer=1 resolve=1 bound framebuffer=2 renderbuffer=1
D: msaa 4 es 2.0: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 1920x1080 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormat8888
D: msaa 4 es 2.0: renderbuffer: 1920x1080 format=0x8058 pixels=1920x1080
D: msaa 4 es 2.0: samples=4 max_samples=4 error=0x0
D: msaa 4 es 2.0: clear: pixel=- error=0x0; swap: ok, error=0x0
D: msaa 4 es 2.0: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: msaa 4 es 2.0: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 2 1 1
D: msaa 4 es 3.0: create context: ok
D: msaa 4 es 3.0: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=3
D: msaa 4 es 3.0: version=OpenGL ES 3.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 3.00
D: msaa 4 es 3.0: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=1 mssamples=4 accelerated=1 retained=1 major=3 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: msaa 4 es 3.0: props: framebuffer=2 renderbuffer=1 resolve=1 bound framebuffer=2 renderbuffer=1
D: msaa 4 es 3.0: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 1920x1080 layer: CAEAGLLayer scale=1 opaque=yes retained=yes format=EAGLColorFormat8888
D: msaa 4 es 3.0: renderbuffer: 1920x1080 format=0x8058 pixels=1920x1080
D: msaa 4 es 3.0: samples=4 max_samples=9 error=0x0
D: msaa 4 es 3.0: clear: pixel=- error=0x0; swap: ok, error=0x0
D: msaa 4 es 3.0: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: msaa 4 es 3.0: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 2 1 1
D: msaa 64 es 3.0 not retained: create context: ok
D: msaa 64 es 3.0 not retained: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=3
D: msaa 64 es 3.0 not retained: version=OpenGL ES 3.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 3.00
D: msaa 64 es 3.0 not retained: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=1 mssamples=9 accelerated=1 retained=0 major=3 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: msaa 64 es 3.0 not retained: props: framebuffer=2 renderbuffer=1 resolve=1 bound framebuffer=2 renderbuffer=1
D: msaa 64 es 3.0 not retained: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 1920x1080 layer: CAEAGLLayer scale=1 opaque=yes retained=no format=EAGLColorFormat8888
D: msaa 64 es 3.0 not retained: renderbuffer: 1920x1080 format=0x8058 pixels=1920x1080
D: msaa 64 es 3.0 not retained: samples=9 max_samples=9 error=0x0
D: msaa 64 es 3.0 not retained: clear: pixel=- error=0x0; swap: ok, error=0x0
D: msaa 64 es 3.0 not retained: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: msaa 64 es 3.0 not retained: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 2 1 1
D: msaa 64 es 2.0 not retained: create context: ok
D: msaa 64 es 2.0 not retained: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=2
D: msaa 64 es 2.0 not retained: version=OpenGL ES 2.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 1.00
D: msaa 64 es 2.0 not retained: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=1 mssamples=4 accelerated=1 retained=0 major=2 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
D: msaa 64 es 2.0 not retained: props: framebuffer=2 renderbuffer=1 resolve=1 bound framebuffer=2 renderbuffer=1
D: msaa 64 es 2.0 not retained: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 1920x1080 layer: CAEAGLLayer scale=1 opaque=yes retained=no format=EAGLColorFormat8888
D: msaa 64 es 2.0 not retained: renderbuffer: 1920x1080 format=0x8058 pixels=1920x1080
D: msaa 64 es 2.0 not retained: samples=4 max_samples=4 error=0x0
D: msaa 64 es 2.0 not retained: clear: pixel=- error=0x0; swap: ok, error=0x0
D: msaa 64 es 2.0 not retained: swap interval: set 1: error: Setting the swap interval is not supported; get: error: not implemented
D: msaa 64 es 2.0 not retained: destroy: ok; current: sdl=no eagl=no; root view: SDL_uikitview; props: 2 1 1
D: formats: events: (none)
D: destroyed
D: destroyed: events: D:WINDOW_DESTROYED(0,0)
H: create 320x480 opengl resizable high-density: ok
H: created: events: H:WINDOW_SAFE_AREA_CHANGED(0,0), H:WINDOW_MOUSE_ENTER(0,0), H:WINDOW_FOCUS_GAINED(0,0), H:WINDOW_SHOWN(0,0), H:WINDOW_PIXEL_SIZE_CHANGED(3840,2160), H:WINDOW_DISPLAY_SCALE_CHANGED(0,0)
H: created: size=1920x1080 pixels=3840x2160 density=2 flags=0x2622
H: created: root view: SDL_uikitview frame=0,0 1920x1080
H: create es 3.0 context: ok
H: context: events: (none)
H: context: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=3
H: context: version=OpenGL ES 3.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 3.00
H: context: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=3 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
H: context: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
H: context: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 1920x1080 layer: CAEAGLLayer scale=2 opaque=yes retained=yes format=EAGLColorFormat8888
H: context: renderbuffer: 3840x2160 format=0x8058 pixels=3840x2160
H: context: samples=0 max_samples=9 error=0x0
H: context: clear: pixel=[64, 128, 191, 255] error=0x0; swap: ok, error=0x0
H: landscape right: not turned (no orientations on tvOS)
H: landscape right: events: (none)
H: landscape right: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=3
H: landscape right: version=OpenGL ES 3.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 3.00
H: landscape right: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=3 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
H: landscape right: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
H: landscape right: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 1920x1080 layer: CAEAGLLayer scale=2 opaque=yes retained=yes format=EAGLColorFormat8888
H: landscape right: renderbuffer: 3840x2160 format=0x8058 pixels=3840x2160
H: landscape right: samples=0 max_samples=9 error=0x0
H: landscape right: clear: pixel=[64, 128, 191, 255] error=0x0; swap: ok, error=0x0
H: portrait: not turned (no orientations on tvOS)
H: portrait: events: (none)
H: portrait: current: sdl=yes window=yes eagl=yes class=SDLEAGLContext api=3
H: portrait: version=OpenGL ES 3.0 APPLE-21.1.1 renderer=Apple Software Renderer vendor=Apple Inc. shading=OpenGL ES GLSL ES 3.00
H: portrait: attributes: red=8 green=8 blue=8 alpha=8 buffer=32 depth=24 stencil=0 double=1 msbuffers=0 mssamples=0 accelerated=1 retained=1 major=3 minor=0 profile=4 share=0 srgb=0 accum_red=0 stereo=0 release=(OpenGL error: GL_INVALID_ENUM)
H: portrait: props: framebuffer=1 renderbuffer=1 resolve=0 bound framebuffer=1 renderbuffer=1
H: portrait: view: SDL_uikitopenglview super=SDL_uikitview frame=0,0 1920x1080 layer: CAEAGLLayer scale=2 opaque=yes retained=yes format=EAGLColorFormat8888
H: portrait: renderbuffer: 3840x2160 format=0x8058 pixels=3840x2160
H: portrait: samples=0 max_samples=9 error=0x0
H: context destroyed
H: context destroyed: events: (none)
H: context destroyed: size=1920x1080 pixels=3840x2160 density=2 flags=0x2622
H: context destroyed: root view: SDL_uikitview frame=0,0 1920x1080
H: create opengles2 renderer: ok
H: renderer: events: (none)
H: renderer: name=opengles2 output=3840x2160 pixels=3840x2160 max_texture_size=4096 vsync=0
H: renderer: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_XRGB8888, SDL_PIXELFORMAT_XBGR8888, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21
H: renderer: context: SDLEAGLContext eagl=yes window flags=0x2622 root view: SDL_uikitopenglview props: 1 1 0 bound framebuffer=1
scene clear and fill: 64x48 5fe1f24a
scene points and lines: 64x48 75192d12
scene viewport and clip: 64x48 b09f79e5
scene blend modes: 64x48 ebb10095
scene texture SDL_PIXELFORMAT_ARGB8888: 64x48 a2cf2633
scene texture SDL_PIXELFORMAT_ABGR8888: 64x48 a2cf2633
scene texture SDL_PIXELFORMAT_RGB24: 64x48 4fb014c5
scene texture SDL_PIXELFORMAT_XRGB8888: 64x48 4fb014c5
scene streaming texture: 64x48 c2319ec5
scene render target: 64x48 9e5285c5
scene packed SDL_PIXELFORMAT_ABGR2101010: 64x48 8bdc6af5
scene packed SDL_PIXELFORMAT_RGBA64_FLOAT: 64x48 9a74c64d
scene packed SDL_PIXELFORMAT_RGBA128_FLOAT: 64x48 9a74c64d
scene palette texture: 64x48 65a75445
scene yuv SDL_PIXELFORMAT_IYUV: 64x48 785ae64d
scene yuv SDL_PIXELFORMAT_YV12: 64x48 785ae64d
scene yuv SDL_PIXELFORMAT_NV12: 64x48 785ae64d
scene yuv SDL_PIXELFORMAT_NV21: 64x48 785ae64d
scene IYUV planes: 64x48 785ae64d
scene NV12 planes: 64x48 785ae64d
scene locked texture: 64x48 76d14245
scene locked NV12 texture: 64x48 785ae64d
scene custom blend mode: 64x48 af879425
scene wrapping geometry: 64x48 dd468f45
scene colored geometry: 64x48 7a2c48ad
scene linear scaling: 64x48 5599895b
scene debug text: 64x48 73f3e17d
partial read: 8x8 f24b7925
target read: 20x10 da6561f5
H: vsync 1: ok, vsync=1
H: vsync 0: ok, vsync=0
H: vsync 2: error: That operation is not supported, vsync=0
H: vsync -1: error: That operation is not supported, vsync=0
H: scenes: events: (none)
H: renderer landscape right: not turned (no orientations on tvOS)
H: renderer landscape right: events: (none)
H: renderer landscape right: name=opengles2 output=3840x2160 pixels=3840x2160 max_texture_size=4096 vsync=0
H: renderer landscape right: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_XRGB8888, SDL_PIXELFORMAT_XBGR8888, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21
H: renderer landscape right: context: SDLEAGLContext eagl=yes window flags=0x2622 root view: SDL_uikitopenglview props: 1 1 0 bound framebuffer=1
H: renderer landscape right corner: 64x48 bf597dc5
H: renderer portrait: not turned (no orientations on tvOS)
H: renderer portrait: events: (none)
H: renderer portrait: name=opengles2 output=3840x2160 pixels=3840x2160 max_texture_size=4096 vsync=0
H: renderer portrait: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_XRGB8888, SDL_PIXELFORMAT_XBGR8888, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21
H: renderer portrait: context: SDLEAGLContext eagl=yes window flags=0x2622 root view: SDL_uikitopenglview props: 1 1 0 bound framebuffer=1
scene again clear and fill: 64x48 5fe1f24a
H: renderer destroyed
H: renderer destroyed: events: (none)
H: renderer destroyed: size=1920x1080 pixels=3840x2160 density=2 flags=0x2622
H: renderer destroyed: root view: SDL_uikitview frame=0,0 1920x1080
H: destroyed
H: destroyed: events: H:WINDOW_DESTROYED(0,0)
M: create 320x480 metal: ok
M: created: events: M:WINDOW_SAFE_AREA_CHANGED(0,0), M:WINDOW_MOUSE_ENTER(0,0), M:WINDOW_FOCUS_GAINED(0,0), M:WINDOW_SHOWN(0,0), M:WINDOW_PIXEL_SIZE_CHANGED(1920,1080)
M: created: size=1920x1080 pixels=1920x1080 density=1 flags=0x20000600
M: created: root view: SDL_uikitview frame=0,0 1920x1080
M: create opengles2 renderer: ok
M: renderer: events: M:WINDOW_HIDDEN(0,0), M:WINDOW_FOCUS_GAINED(0,0), M:WINDOW_SHOWN(0,0)
M: renderer: size=1920x1080 pixels=1920x1080 density=1 flags=0x202
M: renderer: root view: SDL_uikitopenglview frame=0,0 1920x1080
M: renderer: name=opengles2 output=1920x1080 pixels=1920x1080 max_texture_size=4096 vsync=0
M: renderer: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_XRGB8888, SDL_PIXELFORMAT_XBGR8888, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21
M: renderer: context: SDLEAGLContext eagl=yes window flags=0x202 root view: SDL_uikitopenglview props: 1 1 0 bound framebuffer=1
scene M clear and fill: 64x48 5fe1f24a
M: renderer destroyed: events: (none)
M: destroyed
M: destroyed: events: M:WINDOW_DESTROYED(0,0)
W: create 320x480: ok
W: created: events: W:WINDOW_SAFE_AREA_CHANGED(0,0), W:WINDOW_MOUSE_ENTER(0,0), W:WINDOW_FOCUS_GAINED(0,0), W:WINDOW_SHOWN(0,0), W:WINDOW_PIXEL_SIZE_CHANGED(1920,1080)
W: created: size=1920x1080 pixels=1920x1080 density=1 flags=0x602
W: created: root view: SDL_uikitview frame=0,0 1920x1080
W: create renderer: ok
W: renderer: events: (none)
W: renderer: name=metal
W: renderer: size=1920x1080 pixels=1920x1080 density=1 flags=0x20000600
W: renderer: root view: SDL_uikitmetalview frame=0,0 1920x1080
W: renderer destroyed: events: (none)
W: destroyed
W: destroyed: events: W:WINDOW_DESTROYED(0,0)
quit
";

    fn yes_no(b: bool) -> &'static str {
        if b {
            "yes"
        } else {
            "no"
        }
    }

    fn list(items: Vec<String>) -> String {
        if items.is_empty() {
            "(none)".into()
        } else {
            items.join(", ")
        }
    }

    fn ok<T>(r: &sdl3::Result<T>) -> String {
        match r {
            Ok(_) => "ok".into(),
            Err(e) => format!("error: {e}"),
        }
    }

    /// The windows the program made, by ID, for the events' labels.
    static LABELS: Mutex<Vec<(WindowID, &'static str)>> = Mutex::new(Vec::new());

    fn name_window(w: &Window, label: &'static str) {
        LABELS.lock().unwrap().push((w.id(), label));
    }

    fn window_label(id: WindowID) -> &'static str {
        LABELS
            .lock()
            .unwrap()
            .iter()
            .find(|(i, _)| *i == id)
            .map_or("?", |(_, l)| l)
    }

    /// An event as the trace names it (the C program's names), `None` for
    /// the mice's device events (see the module's description).
    fn describe(event: &Event) -> Option<String> {
        let name = event_name(event.event_type());
        Some(match event {
            Event::MouseDevice(_) => return None,
            Event::Display(d) => format!("{name}({},{})", d.display_id, d.data1),
            Event::Window(w) => format!(
                "{}:{name}({},{})",
                window_label(w.window_id),
                w.data1,
                w.data2
            ),
            _ => name,
        })
    }

    /// Pump, and the queued events.
    fn take_queued(out: &mut Vec<String>) {
        events::pump();
        while let Some(event) = events::poll() {
            out.extend(describe(&event));
        }
    }

    /// Pump for `ms` milliseconds (10 ms steps).
    fn settle(out: &mut Vec<String>, ms: u64) {
        take_queued(out);
        for _ in 0..ms / 10 {
            std::thread::sleep(Duration::from_millis(10));
            take_queued(out);
        }
    }

    /// Pump until an event containing `until` has come (then for
    /// `after_ms` more), or for at most `limit`.
    fn pump_until(out: &mut Vec<String>, until: &str, limit: Duration, after_ms: u64) {
        let end = Instant::now() + limit;
        while Instant::now() < end {
            take_queued(out);
            if out.iter().any(|e| e.contains(until)) {
                settle(out, after_ms);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn trace_events(label: &str, ms: u64) {
        let mut seen = Vec::new();
        settle(&mut seen, ms);
        t(format!("{label}: events: {}", list(seen)));
    }

    /// `CGRect`.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGRect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }

    fn rect(r: CGRect) -> String {
        format!("{},{} {}x{}", r.x, r.y, r.w, r.h)
    }

    /// The window's `UIWindow` (its property, a number here).
    fn uiwindow_of(w: &Window) -> Id {
        w.properties()
            .ok()
            .and_then(|p| p.get_number(video::window::PROP_WINDOW_UIKIT_WINDOW_POINTER))
            .map_or(std::ptr::null_mut(), |p| p as usize as Id)
    }

    /// Its root view controller.
    fn vc_of(w: &Window) -> Id {
        // SAFETY: a UIWindow property.
        unsafe { objc::send0(uiwindow_of(w), "rootViewController") }
    }

    /// Its root view controller's view.
    fn root_view_of(w: &Window) -> Id {
        // SAFETY: a UIViewController property.
        unsafe { objc::send0(vc_of(w), "view") }
    }

    /// A view's frame.
    fn frame_of(view: Id) -> CGRect {
        if view.is_null() {
            return CGRect::default();
        }
        // SAFETY: a UIView property.
        unsafe { objc::send0(view, "frame") }
    }

    /// The window's sizes and flags, and its view controller's view.
    fn window_state(label: &str, w: &Window) {
        let (ww, wh) = w.size().unwrap_or_default();
        let (pw, ph) = w.size_in_pixels().unwrap_or_default();
        t(format!(
            "{label}: size={ww}x{wh} pixels={pw}x{ph} density={} flags={:#x}",
            w.pixel_density().unwrap_or_default(),
            w.flags().unwrap_or_default().0
        ));
        let v = root_view_of(w);
        t(format!(
            "{label}: root view: {} frame={}",
            objc::class_name(v),
            rect(frame_of(v))
        ));
    }

    /// The window's OpenGL properties (-1: not set).
    fn gl_props(w: &Window) -> (i64, i64, i64) {
        let p = w.properties().unwrap();
        let get = |name| p.get_number(name).unwrap_or(-1);
        (
            get(video::window::PROP_WINDOW_UIKIT_OPENGL_FRAMEBUFFER_NUMBER),
            get(video::window::PROP_WINDOW_UIKIT_OPENGL_RENDERBUFFER_NUMBER),
            get(video::window::PROP_WINDOW_UIKIT_OPENGL_RESOLVE_FRAMEBUFFER_NUMBER),
        )
    }

    /// Ask the window's scene to turn to an orientation (a
    /// `UIInterfaceOrientationMask`), and trace the events that come.
    fn turn(w: &Window, label: &str, mask: usize) {
        if cfg!(target_os = "tvos") {
            let _ = (w, mask);
            t(format!("{label}: not turned (no orientations on tvOS)"));
            trace_events(label, 500);
            return;
        }
        let mut seen = Vec::new();
        println!("uikit_gles: {:.3} s: turning to {label}", elapsed());
        // SAFETY: UIWindowSceneGeometryPreferencesIOS's initializer, the
        // view controller's and the scene's methods (iOS 16).
        unsafe {
            let prefs: Id =
                objc::send0(objc::class("UIWindowSceneGeometryPreferencesIOS"), "alloc");
            let prefs: Id = objc::send1(prefs, "initWithInterfaceOrientations:", mask);
            let () = objc::send0(vc_of(w), "setNeedsUpdateOfSupportedInterfaceOrientations");
            let scene: Id = objc::send0(uiwindow_of(w), "windowScene");
            let () = objc::send2(
                scene,
                "requestGeometryUpdateWithPreferences:errorHandler:",
                prefs,
                std::ptr::null_mut::<std::ffi::c_void>(),
            );
        }
        pump_until(
            &mut seen,
            "DISPLAY_ORIENTATION",
            Duration::from_secs(5),
            1500,
        );
        println!("uikit_gles: {:.3} s: turned", elapsed());
        t(format!("{label}: events: {}", list(seen)));
    }

    // ------------------------------------------------------------------
    // OpenGL ES, through SDL_GL_GetProcAddress()

    mod glf {
        use std::ffi::{c_char, c_void, CStr};

        pub type GLenum = u32;
        pub type GLint = i32;
        pub type GLuint = u32;

        pub const GL_VENDOR: GLenum = 0x1F00;
        pub const GL_RENDERER: GLenum = 0x1F01;
        pub const GL_VERSION: GLenum = 0x1F02;
        pub const GL_SHADING_LANGUAGE_VERSION: GLenum = 0x8B8C;
        pub const GL_COLOR_BUFFER_BIT: u32 = 0x4000;
        pub const GL_RGBA: GLenum = 0x1908;
        pub const GL_UNSIGNED_BYTE: GLenum = 0x1401;
        pub const GL_TEXTURE_2D: GLenum = 0x0DE1;
        pub const GL_SAMPLES: GLenum = 0x80A9;
        pub const GL_FRAMEBUFFER_BINDING: GLenum = 0x8CA6;
        pub const GL_RENDERBUFFER_BINDING: GLenum = 0x8CA7;
        pub const GL_RENDERBUFFER: GLenum = 0x8D41;
        pub const GL_RENDERBUFFER_WIDTH: GLenum = 0x8D42;
        pub const GL_RENDERBUFFER_HEIGHT: GLenum = 0x8D43;
        pub const GL_RENDERBUFFER_INTERNAL_FORMAT: GLenum = 0x8D44;
        pub const GL_MAX_SAMPLES: GLenum = 0x8D57;

        /// A GL function by name (through SDL), cast to its type.
        ///
        /// # Safety
        ///
        /// `F` must be the function's type.
        unsafe fn f<F: Copy>(name: &str) -> F {
            let p = sdl3::video::gl::gl_get_proc_address(name)
                .unwrap_or_else(|| panic!("no GL function {name}"));
            // SAFETY: the caller's contract; a function pointer.
            unsafe { std::mem::transmute_copy::<*const c_void, F>(&p) }
        }

        // SAFETY (all): GL calls with a current context; the out-parameters
        // are valid.
        pub fn get_string(name: GLenum) -> String {
            unsafe {
                let s = f::<unsafe extern "C" fn(GLenum) -> *const u8>("glGetString")(name);
                if s.is_null() {
                    "(null)".into()
                } else {
                    CStr::from_ptr(s as *const c_char)
                        .to_string_lossy()
                        .into_owned()
                }
            }
        }

        pub fn get_integer(pname: GLenum) -> GLint {
            let mut v = 0;
            unsafe {
                f::<unsafe extern "C" fn(GLenum, *mut GLint)>("glGetIntegerv")(pname, &mut v)
            };
            v
        }

        pub fn get_error() -> GLenum {
            unsafe { f::<unsafe extern "C" fn() -> GLenum>("glGetError")() }
        }

        pub fn renderbuffer_parameter(pname: GLenum) -> GLint {
            let mut v = 0;
            unsafe {
                f::<unsafe extern "C" fn(GLenum, GLenum, *mut GLint)>(
                    "glGetRenderbufferParameteriv",
                )(GL_RENDERBUFFER, pname, &mut v)
            };
            v
        }

        pub fn clear(r: f32, g: f32, b: f32, a: f32) {
            unsafe {
                f::<unsafe extern "C" fn(f32, f32, f32, f32)>("glClearColor")(r, g, b, a);
                f::<unsafe extern "C" fn(u32)>("glClear")(GL_COLOR_BUFFER_BIT);
            }
        }

        pub fn read_pixel() -> [u8; 4] {
            let mut p = [0u8; 4];
            unsafe {
                f::<unsafe extern "C" fn(GLint, GLint, i32, i32, GLenum, GLenum, *mut c_void)>(
                    "glReadPixels",
                )(
                    0,
                    0,
                    1,
                    1,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    p.as_mut_ptr() as *mut c_void,
                )
            };
            p
        }

        pub fn gen_texture() -> GLuint {
            let mut t = 0;
            unsafe {
                f::<unsafe extern "C" fn(i32, *mut GLuint)>("glGenTextures")(1, &mut t);
                f::<unsafe extern "C" fn(GLenum, GLuint)>("glBindTexture")(GL_TEXTURE_2D, t);
            }
            t
        }

        pub fn is_texture(t: GLuint) -> bool {
            unsafe { f::<unsafe extern "C" fn(GLuint) -> u8>("glIsTexture")(t) != 0 }
        }

        pub fn delete_texture(t: GLuint) {
            unsafe { f::<unsafe extern "C" fn(i32, *const GLuint)>("glDeleteTextures")(1, &t) };
        }
    }

    /// `[EAGLContext currentContext]`.
    fn eagl_current() -> Id {
        // SAFETY: EAGLContext's class method.
        unsafe { objc::send0(objc::class("EAGLContext"), "currentContext") }
    }

    /// Whether a context is EAGL's current one.
    fn eagl_is(ctx: &GlContext) -> bool {
        std::ptr::eq(eagl_current(), ctx.raw().as_ptr())
    }

    /// `[EAGLContext setCurrentContext:context]`.
    fn eagl_set_current(context: Id) {
        // SAFETY: EAGLContext's class method.
        let _: bool =
            unsafe { objc::send1(objc::class("EAGLContext"), "setCurrentContext:", context) };
    }

    /// Another EAGL context (OpenGL ES 2), as the system's own would be.
    fn other_context() -> Id {
        // SAFETY: EAGLContext's alloc and initializer (owned, kept).
        unsafe {
            let c: Id = objc::send0(objc::class("EAGLContext"), "alloc");
            objc::send1(c, "initWithAPI:", 2usize)
        }
    }

    /// The attributes as SDL reads them back (without a context, only
    /// those it doesn't ask GL for).
    fn attributes(label: &str, with_context: bool) {
        let attrs = [
            ("red", GlAttr::RedSize),
            ("green", GlAttr::GreenSize),
            ("blue", GlAttr::BlueSize),
            ("alpha", GlAttr::AlphaSize),
            ("buffer", GlAttr::BufferSize),
            ("depth", GlAttr::DepthSize),
            ("stencil", GlAttr::StencilSize),
            ("double", GlAttr::DoubleBuffer),
            ("msbuffers", GlAttr::MultisampleBuffers),
            ("mssamples", GlAttr::MultisampleSamples),
            ("accelerated", GlAttr::AcceleratedVisual),
            ("retained", GlAttr::RetainedBacking),
            ("major", GlAttr::ContextMajorVersion),
            ("minor", GlAttr::ContextMinorVersion),
            ("profile", GlAttr::ContextProfileMask),
            ("share", GlAttr::ShareWithCurrentContext),
            ("srgb", GlAttr::FramebufferSrgbCapable),
            ("accum_red", GlAttr::AccumRedSize),
            ("stereo", GlAttr::Stereo),
            ("release", GlAttr::ContextReleaseBehavior),
        ];
        const ASKS_GL: [&str; 10] = [
            "red",
            "green",
            "blue",
            "alpha",
            "buffer",
            "depth",
            "stencil",
            "msbuffers",
            "mssamples",
            "release",
        ];
        let values: Vec<String> = attrs
            .iter()
            .filter(|(name, _)| with_context || !ASKS_GL.contains(name))
            .map(|&(name, attr)| match gl::gl_get_attribute(attr) {
                Ok(v) => format!("{name}={v}"),
                Err(e) => format!("{name}=({e})"),
            })
            .collect();
        t(format!("{label}: attributes: {}", values.join(" ")));
    }

    /// The layer's drawable properties: retained, and the color format
    /// (the keys are the values of `kEAGLDrawablePropertyRetainedBacking`
    /// and `kEAGLDrawablePropertyColorFormat`).
    fn drawable_properties(view: Id) -> String {
        // SAFETY: UIView's layer, a CAEAGLLayer, and its NSDictionary.
        unsafe {
            let layer: Id = objc::send0(view, "layer");
            let props: Id = objc::send0(layer, "drawableProperties");
            let retained: Id = objc::send1(
                props,
                "objectForKey:",
                objc::ns_string("EAGLDrawablePropertyRetained"),
            );
            let retained = if retained.is_null() {
                "nil".to_string()
            } else {
                let b: bool = objc::send0(retained, "boolValue");
                yes_no(b).to_string()
            };
            let format: Id = objc::send1(
                props,
                "objectForKey:",
                objc::ns_string("EAGLDrawablePropertyColorFormat"),
            );
            let scale: f64 = objc::send0(layer, "contentsScale");
            let opaque: bool = objc::send0(layer, "isOpaque");
            format!(
                "{} scale={scale} opaque={} retained={retained} format={}",
                objc::class_name(layer),
                yes_no(opaque),
                objc::string(format).unwrap_or_else(|| "nil".into())
            )
        }
    }

    /// What a new (current) context and its view are.
    fn context_state(label: &str, w: &Window, ctx: &GlContext) {
        let raw = ctx.raw().as_ptr() as Id;
        let current = gl::gl_current_context().ok().flatten();
        // SAFETY: an EAGLContext property.
        let api: usize = unsafe { objc::send0(raw, "API") };
        t(format!(
            "{label}: current: sdl={} window={} eagl={} class={} api={api}",
            yes_no(current == Some(ctx.raw())),
            yes_no(gl::gl_current_window().ok().flatten().map(|cw| cw.id()) == Some(w.id())),
            yes_no(eagl_current() == raw),
            objc::class_name(raw)
        ));
        let shading = if api >= 2 {
            glf::get_string(glf::GL_SHADING_LANGUAGE_VERSION)
        } else {
            "-".into()
        };
        t(format!(
            "{label}: version={} renderer={} vendor={} shading={shading}",
            glf::get_string(glf::GL_VERSION),
            glf::get_string(glf::GL_RENDERER),
            glf::get_string(glf::GL_VENDOR)
        ));
        attributes(label, true);
        let (fb, rb, resolve) = gl_props(w);
        t(format!(
            "{label}: props: framebuffer={fb} renderbuffer={rb} resolve={resolve} bound framebuffer={} renderbuffer={}",
            glf::get_integer(glf::GL_FRAMEBUFFER_BINDING),
            glf::get_integer(glf::GL_RENDERBUFFER_BINDING)
        ));
        let v = root_view_of(w);
        let (pw, ph) = w.size_in_pixels().unwrap_or_default();
        t(format!(
            "{label}: view: {} super={} frame={} layer: {}",
            objc::class_name(v),
            objc::superclass_name(v),
            rect(frame_of(v)),
            drawable_properties(v)
        ));
        t(format!(
            "{label}: renderbuffer: {}x{} format={:#x} pixels={pw}x{ph}",
            glf::renderbuffer_parameter(glf::GL_RENDERBUFFER_WIDTH),
            glf::renderbuffer_parameter(glf::GL_RENDERBUFFER_HEIGHT),
            glf::renderbuffer_parameter(glf::GL_RENDERBUFFER_INTERNAL_FORMAT)
        ));
        t(format!(
            "{label}: samples={} max_samples={} error={:#x}",
            glf::get_integer(glf::GL_SAMPLES),
            glf::get_integer(glf::GL_MAX_SAMPLES),
            glf::get_error()
        ));
    }

    /// Clear, read a pixel back, swap.
    fn draw_and_swap(label: &str, w: &Window, read: bool) {
        glf::clear(0.25, 0.5, 0.75, 1.0);
        let pixel = if read {
            format!("{:?}", glf::read_pixel())
        } else {
            "-".into()
        };
        let error = glf::get_error();
        let swap = gl::gl_swap_window(w);
        t(format!(
            "{label}: clear: pixel={pixel} error={error:#x}; swap: {}, error={:#x}",
            ok(&swap),
            glf::get_error()
        ));
    }

    /// Make a context on `w` with the attributes as they are, trace it (and
    /// draw, swap, set the swap interval) and destroy it.
    fn context_round(label: &str, w: &Window, read: bool) {
        let r = GlContext::new(w);
        t(format!("{label}: create context: {}", ok(&r)));
        let Ok(ctx) = r else { return };
        context_state(label, w, &ctx);
        draw_and_swap(label, w, read);
        let set = gl::gl_set_swap_interval(1);
        let get = gl::gl_get_swap_interval();
        t(format!(
            "{label}: swap interval: set 1: {}; get: {}",
            ok(&set),
            match get {
                Ok(v) => format!("{v}"),
                Err(e) => format!("error: {e}"),
            }
        ));
        let r = ctx.destroy();
        let v = root_view_of(w);
        let (fb, rb, resolve) = gl_props(w);
        t(format!(
            "{label}: destroy: {}; current: sdl={} eagl={}; root view: {}; props: {fb} {rb} {resolve}",
            ok(&r),
            yes_no(gl::gl_current_context().ok().flatten().is_some()),
            yes_no(!eagl_current().is_null()),
            objc::class_name(v)
        ));
    }

    fn set_attr(attr: GlAttr, value: i32) {
        gl::gl_set_attribute(attr, value).unwrap();
    }

    /// The renderer's name, sizes, formats, and its context and view.
    fn renderer_state(label: &str, r: &mut Renderer, w: &Window) {
        let props = r.properties();
        let (ow, oh) = r.output_size().unwrap_or_default();
        let (pw, ph) = w.size_in_pixels().unwrap_or_default();
        t(format!(
            "{label}: name={} output={ow}x{oh} pixels={pw}x{ph} max_texture_size={} vsync={}",
            r.name(),
            props
                .get_number(sdl3::render::PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER)
                .unwrap_or(-1),
            r.vsync()
        ));
        let formats: Vec<String> = r
            .texture_formats()
            .iter()
            .map(|f| f.name().to_string())
            .collect();
        t(format!("{label}: formats: {}", list(formats)));
        let ctx = gl::gl_current_context().ok().flatten();
        let raw = ctx.map_or(std::ptr::null_mut(), |c| c.as_ptr() as Id);
        let v = root_view_of(w);
        let (fb, rb, resolve) = gl_props(w);
        t(format!(
            "{label}: context: {} eagl={} window flags={:#x} root view: {} props: {fb} {rb} {resolve} bound framebuffer={}",
            if raw.is_null() { "nil".to_string() } else { objc::class_name(raw) },
            yes_no(!raw.is_null() && eagl_current() == raw),
            w.flags().unwrap_or_default().0,
            objc::class_name(v),
            glf::get_integer(glf::GL_FRAMEBUFFER_BINDING)
        ));
    }

    fn application_state() -> isize {
        // SAFETY: a UIApplication property.
        unsafe { objc::send0(objc::shared_application(), "applicationState") }
    }

    /// The failed assertions the handler saw.
    static ASSERTIONS: AtomicU32 = AtomicU32::new(0);

    pub(super) fn main() {
        elapsed();
        let status = sdl3::app::run_app(|_args| {
            let status = checked(main_function);
            exit(status);
        });
        println!("uikit_gles: run_app returned {status}, which it never does");
        exit(1);
    }

    /// Whether the simulator has OpenGL ES (else the program skips,
    /// unless `SDL3_TEST_REQUIRE` names `gles`).
    fn gles_available() -> bool {
        let c = objc::class("EAGLContext");
        let why = if c.is_null() {
            "no EAGLContext class".to_string()
        } else {
            let other = other_context();
            if other.is_null() {
                "EAGL couldn't make an OpenGL ES 2 context".to_string()
            } else {
                // SAFETY: the context made above, released.
                unsafe {
                    let () = objc::send0(other, "release");
                }
                return true;
            }
        };
        let required = std::env::var("SDL3_TEST_REQUIRE").unwrap_or_default();
        if required
            .split(',')
            .map(str::trim)
            .any(|c| c == "gles" || c == "all")
        {
            panic!("required capability gles unavailable: {why}");
        }
        eprintln!("note: skipping, gles unavailable: {why}");
        false
    }

    fn main_function() {
        const LANDSCAPE_RIGHT: usize = 1 << 3;
        const PORTRAIT: usize = 1 << 1;

        println!("uikit_gles: {:.3} s: main", elapsed());
        if !gles_available() {
            return;
        }
        let r = init::init(InitFlags::VIDEO);
        t(format!("init video: {}", ok(&r)));
        r.unwrap();
        t(format!(
            "video driver: {}",
            video::current_video_driver().unwrap()
        ));

        // The app becomes active around when the main function starts
        // (the events of its activation are left out: they may come before
        // SDL is up)
        let mut ignored = Vec::new();
        let end = Instant::now() + Duration::from_secs(30);
        while application_state() != 0 && Instant::now() < end {
            take_queued(&mut ignored);
            std::thread::sleep(Duration::from_millis(10));
        }
        for _ in 0..10 {
            take_queued(&mut ignored);
            std::thread::sleep(Duration::from_millis(10));
        }
        t(format!("application state: {}", application_state()));

        attributes("defaults", false);

        // D: a window asking for no graphics API (the OpenGL flag)
        let r = Window::create("D", 320, 480, WindowFlags::NONE);
        t(format!("D: create 320x480: {}", ok(&r)));
        let d = r.unwrap();
        name_window(&d, "D");
        trace_events("D: created", 500);
        window_state("D: created", &d);

        // The versions
        for (major, minor) in [(2, 0), (3, 0), (1, 1), (3, 1), (4, 0)] {
            set_attr(GlAttr::ContextMajorVersion, major);
            set_attr(GlAttr::ContextMinorVersion, minor);
            context_round(&format!("D: es {major}.{minor}"), &d, true);
        }
        trace_events("D: versions", 300);
        set_attr(GlAttr::ContextMajorVersion, 2);
        set_attr(GlAttr::ContextMinorVersion, 0);

        // Share groups
        let a = GlContext::new(&d);
        t(format!("D: context A: {}", ok(&a)));
        let a = a.unwrap();
        let a_view = root_view_of(&d);
        let texture = glf::gen_texture();
        // SAFETY: EAGLContext properties.
        let group_of =
            |c: &GlContext| -> Id { unsafe { objc::send0(c.raw().as_ptr() as Id, "sharegroup") } };
        set_attr(GlAttr::ShareWithCurrentContext, 1);
        let b = GlContext::new(&d).unwrap();
        t(format!(
            "D: context B (shared): same group={} texture={} current={} root view is A's={}",
            yes_no(group_of(&a) == group_of(&b)),
            yes_no(glf::is_texture(texture)),
            yes_no(b.is_current()),
            yes_no(root_view_of(&d) == a_view)
        ));
        set_attr(GlAttr::ShareWithCurrentContext, 0);
        let c = GlContext::new(&d).unwrap();
        t(format!(
            "D: context C (not shared): same group={} texture={}",
            yes_no(group_of(&a) == group_of(&c)),
            yes_no(glf::is_texture(texture))
        ));
        let r = a.make_current(Some(&d));
        t(format!(
            "D: A current again: {} texture={} root view is A's={} eagl={}",
            ok(&r),
            yes_no(glf::is_texture(texture)),
            yes_no(root_view_of(&d) == a_view),
            yes_no(eagl_is(&a))
        ));
        glf::delete_texture(texture);
        drop(b);
        drop(c);
        t(format!(
            "D: B and C destroyed: A current={} root view is A's={}",
            yes_no(a.is_current()),
            yes_no(root_view_of(&d) == a_view)
        ));

        // The current context restored
        let other = other_context();
        eagl_set_current(other);
        let mut seen = Vec::new();
        take_queued(&mut seen);
        t(format!(
            "D: pumped after another context: ours={}",
            yes_no(eagl_is(&a))
        ));
        let frames: Rc<RefCell<Vec<bool>>> = Rc::new(RefCell::new(Vec::new()));
        {
            let frames = frames.clone();
            let ours = a.raw().as_ptr() as usize;
            eagl_set_current(other);
            d.set_ios_animation_callback(
                1,
                Some(Box::new(move || {
                    let mut f = frames.borrow_mut();
                    if f.len() < 2 {
                        f.push(eagl_current() as usize == ours);
                        eagl_set_current(other);
                    }
                    if f.len() == 2 {
                        let _ = d.set_ios_animation_callback(1, None);
                    }
                })),
            )
            .unwrap();
        }
        let end = Instant::now() + Duration::from_secs(5);
        while frames.borrow().len() < 2 && Instant::now() < end {
            // (the display link runs in the pump, which restores the
            // context after it too)
            take_queued(&mut seen);
            std::thread::sleep(Duration::from_millis(5));
        }
        t(format!(
            "D: animation callbacks after another context: ours={}",
            list(
                frames
                    .borrow()
                    .iter()
                    .map(|&b| yes_no(b).to_string())
                    .collect()
            )
        ));
        eagl_set_current(other);
        let r = d.raise();
        t(format!(
            "D: raised after another context: {} ours={}",
            ok(&r),
            yes_no(eagl_is(&a))
        ));

        // The battery monitoring the swaps turn off
        sdl3::assert::set_assertion_handler(|_| {
            ASSERTIONS.fetch_add(1, Ordering::SeqCst);
            sdl3::assert::AssertState::Ignore
        });
        let info = sdl3::power::power_info();
        t(format!(
            "D: power: state={:?} percent={:?}",
            info.state, info.percent
        ));
        let _ = gl::gl_swap_window(&d);
        let before = ASSERTIONS.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(3100));
        let _ = gl::gl_swap_window(&d);
        let after = ASSERTIONS.load(Ordering::SeqCst);
        let _ = gl::gl_swap_window(&d);
        t(format!(
            "D: battery: assertions: swap at once {before}, after 3 s {after}, again {}",
            ASSERTIONS.load(Ordering::SeqCst)
        ));
        sdl3::assert::reset_assertion_handler();

        drop(a);
        // SAFETY: the other context, released.
        unsafe {
            let () = objc::send0(other, "release");
        }
        let r = d.raise();
        t(format!(
            "D: A destroyed: raised: {} eagl={} root view: {}",
            ok(&r),
            yes_no(!eagl_current().is_null()),
            objc::class_name(root_view_of(&d))
        ));
        trace_events("D: contexts", 300);

        // The framebuffer's formats
        set_attr(GlAttr::FramebufferSrgbCapable, 1);
        context_round("D: srgb", &d, true);
        let _ = sdl3::hints::set(sdl3::hints::OPENGL_FORCE_SRGB_FRAMEBUFFER, "0");
        context_round("D: srgb forced off", &d, true);
        set_attr(GlAttr::FramebufferSrgbCapable, 0);
        let _ = sdl3::hints::set(sdl3::hints::OPENGL_FORCE_SRGB_FRAMEBUFFER, "1");
        context_round("D: srgb forced on", &d, true);
        sdl3::hints::reset(sdl3::hints::OPENGL_FORCE_SRGB_FRAMEBUFFER);
        set_attr(GlAttr::RedSize, 5);
        set_attr(GlAttr::GreenSize, 6);
        set_attr(GlAttr::BlueSize, 5);
        set_attr(GlAttr::AlphaSize, 0);
        context_round("D: rgb565", &d, true);
        gl::gl_reset_attributes();
        set_attr(GlAttr::DepthSize, 24);
        set_attr(GlAttr::StencilSize, 8);
        context_round("D: depth 24 stencil 8", &d, true);
        set_attr(GlAttr::DepthSize, 0);
        set_attr(GlAttr::StencilSize, 0);
        set_attr(GlAttr::RetainedBacking, 0);
        context_round("D: no depth, not retained", &d, true);
        gl::gl_reset_attributes();
        set_attr(GlAttr::MultisampleBuffers, 1);
        set_attr(GlAttr::MultisampleSamples, 4);
        context_round("D: msaa 4 es 2.0", &d, false);
        set_attr(GlAttr::ContextMajorVersion, 3);
        context_round("D: msaa 4 es 3.0", &d, false);
        set_attr(GlAttr::RetainedBacking, 0);
        set_attr(GlAttr::MultisampleSamples, 64);
        context_round("D: msaa 64 es 3.0 not retained", &d, false);
        set_attr(GlAttr::ContextMajorVersion, 2);
        context_round("D: msaa 64 es 2.0 not retained", &d, false);
        gl::gl_reset_attributes();
        trace_events("D: formats", 300);

        d.destroy();
        t("D: destroyed");
        trace_events("D: destroyed", 500);

        // H: a resizable high-density OpenGL window, turned with a context
        let r = Window::create(
            "H",
            320,
            480,
            WindowFlags::OPENGL | WindowFlags::RESIZABLE | WindowFlags::HIGH_PIXEL_DENSITY,
        );
        t(format!(
            "H: create 320x480 opengl resizable high-density: {}",
            ok(&r)
        ));
        let h = r.unwrap();
        name_window(&h, "H");
        trace_events("H: created", 500);
        window_state("H: created", &h);
        set_attr(GlAttr::ContextMajorVersion, 3);
        let ctx = GlContext::new(&h);
        t(format!("H: create es 3.0 context: {}", ok(&ctx)));
        let ctx = ctx.unwrap();
        trace_events("H: context", 300);
        context_state("H: context", &h, &ctx);
        draw_and_swap("H: context", &h, true);
        turn(&h, "H: landscape right", LANDSCAPE_RIGHT);
        context_state("H: landscape right", &h, &ctx);
        draw_and_swap("H: landscape right", &h, true);
        turn(&h, "H: portrait", PORTRAIT);
        context_state("H: portrait", &h, &ctx);
        drop(ctx);
        gl::gl_reset_attributes();
        t("H: context destroyed");
        trace_events("H: context destroyed", 300);
        window_state("H: context destroyed", &h);

        // The OpenGL ES 2 renderer on H
        let r = Renderer::for_window(&h, Some("opengles2"));
        t(format!("H: create opengles2 renderer: {}", ok(&r)));
        let mut r = r.unwrap();
        trace_events("H: renderer", 500);
        renderer_state("H: renderer", &mut r, &h);
        let _ = r.present();
        scenes(&mut r, "GLES2");
        for want in [1, 0, 2, -1] {
            let result = r.set_vsync(want);
            t(format!(
                "H: vsync {want}: {}, vsync={}",
                ok(&result),
                r.vsync()
            ));
            r.set_draw_color(0, 0, 0, 255);
            let _ = r.clear();
            let _ = r.present();
        }
        trace_events("H: scenes", 300);

        // The output follows the window as it turns
        turn(&h, "H: renderer landscape right", LANDSCAPE_RIGHT);
        renderer_state("H: renderer landscape right", &mut r, &h);
        r.set_draw_color(0, 128, 0, 255);
        let _ = r.clear();
        let (ow, oh) = r.output_size().unwrap_or_default();
        hash_read(
            "H: renderer landscape right corner",
            r.read_pixels(Some(&Rect::new(ow - RW, oh - RH, RW, RH))),
        );
        let _ = r.present();
        turn(&h, "H: renderer portrait", PORTRAIT);
        renderer_state("H: renderer portrait", &mut r, &h);
        scene(
            &mut r,
            "again clear and fill",
            Some((0, 0)),
            &clear_and_fill,
        );
        check_custom_blend(&mut r);
        drop(r);
        t("H: renderer destroyed");
        trace_events("H: renderer destroyed", 300);
        window_state("H: renderer destroyed", &h);
        h.destroy();
        t("H: destroyed");
        trace_events("H: destroyed", 500);

        // The OpenGL ES 2 renderer on a Metal window: made again
        let r = Window::create("M", 320, 480, WindowFlags::METAL);
        t(format!("M: create 320x480 metal: {}", ok(&r)));
        let m = r.unwrap();
        name_window(&m, "M");
        trace_events("M: created", 500);
        window_state("M: created", &m);
        let r = Renderer::for_window(&m, Some("opengles2"));
        t(format!("M: create opengles2 renderer: {}", ok(&r)));
        let mut r = r.unwrap();
        trace_events("M: renderer", 500);
        window_state("M: renderer", &m);
        renderer_state("M: renderer", &mut r, &m);
        scene(&mut r, "M clear and fill", Some((0, 0)), &clear_and_fill);
        drop(r);
        trace_events("M: renderer destroyed", 300);
        m.destroy();
        t("M: destroyed");
        trace_events("M: destroyed", 500);

        // The default renderer on a window asking for no graphics API
        let r = Window::create("W", 320, 480, WindowFlags::NONE);
        t(format!("W: create 320x480: {}", ok(&r)));
        let w = r.unwrap();
        name_window(&w, "W");
        trace_events("W: created", 500);
        window_state("W: created", &w);
        let r = Renderer::for_window(&w, None);
        t(format!("W: create renderer: {}", ok(&r)));
        let r = r.unwrap();
        trace_events("W: renderer", 500);
        t(format!("W: renderer: name={}", r.name()));
        window_state("W: renderer", &w);
        drop(r);
        trace_events("W: renderer destroyed", 300);
        w.destroy();
        t("W: destroyed");
        trace_events("W: destroyed", 500);

        init::quit();
        t("quit");

        let mismatches = MISMATCHES.lock().unwrap();
        for m in mismatches.iter() {
            println!("uikit_gles: against the software renderer: {m}");
        }
        check_trace(&TRACE.lock().unwrap(), EXPECTED);
        assert!(
            mismatches.is_empty(),
            "the OpenGL ES 2 renderer against the software renderer:\n{}",
            mismatches.join("\n")
        );
        println!("uikit_gles: all checks passed");
    }
}
