// Rust translation of src/video/cocoa/SDL_cocoaopengl.m and
// SDL_cocoaopengl.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! NSOpenGL implementation of SDL OpenGL support.
//!
//! The contexts are `SDL3OpenGLContext`s, an `NSOpenGLContext` subclass
//! declared at run time whose instance variables (`dirty`, `window`, the
//! display link and the swap interval state) are a Rust structure the
//! object points to. Vsync is handled by synchronizing with a display link,
//! as upstream does: the context's own swap interval stays 0.
//!
//! We still support OpenGL as long as Apple offers it, deprecated or not.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};

use super::modes::DisplayData;
use super::sys::CGDirectDisplayID;
use super::window::window_data;
use crate::core::apple::block::{run_on_main_async, run_on_main_sync};
use crate::core::apple::foundation::is_main_thread;
use crate::core::apple::objc::{
    autoreleasepool, class, imp, ivar_ptr, msg_send, msg_send_super, sel, Class, ClassBuilder,
    DeclaredClass, Id, NSInteger, Sel, StrongPtr, WeakId, NIL,
};
use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::WindowID;
use crate::hints;
use crate::loadso::SharedObject;
use crate::video::core::{with_display, with_window};
use crate::video::gl::{self, RawGlContext, GL_CONTEXT_PROFILE_CORE, GL_CONTEXT_PROFILE_ES};

/// `DEFAULT_OPENGL`.
const DEFAULT_OPENGL: &str = "/System/Library/Frameworks/OpenGL.framework/Libraries/libGL.dylib";

// <AppKit/NSOpenGL.h>, <OpenGL/CGLRenderers.h>

type NSOpenGLPixelFormatAttribute = u32;
const NSOpenGLPFADoubleBuffer: NSOpenGLPixelFormatAttribute = 5;
const NSOpenGLPFAStereo: NSOpenGLPixelFormatAttribute = 6;
const NSOpenGLPFAColorSize: NSOpenGLPixelFormatAttribute = 8;
const NSOpenGLPFADepthSize: NSOpenGLPixelFormatAttribute = 12;
const NSOpenGLPFAStencilSize: NSOpenGLPixelFormatAttribute = 13;
const NSOpenGLPFAAccumSize: NSOpenGLPixelFormatAttribute = 14;
const NSOpenGLPFASampleBuffers: NSOpenGLPixelFormatAttribute = 55;
const NSOpenGLPFASamples: NSOpenGLPixelFormatAttribute = 56;
const NSOpenGLPFAColorFloat: NSOpenGLPixelFormatAttribute = 58;
const NSOpenGLPFARendererID: NSOpenGLPixelFormatAttribute = 70;
const NSOpenGLPFANoRecovery: NSOpenGLPixelFormatAttribute = 72;
const NSOpenGLPFAAccelerated: NSOpenGLPixelFormatAttribute = 73;
const NSOpenGLPFAScreenMask: NSOpenGLPixelFormatAttribute = 84;
const NSOpenGLPFAAllowOfflineRenderers: NSOpenGLPixelFormatAttribute = 96;
const NSOpenGLPFAOpenGLProfile: NSOpenGLPixelFormatAttribute = 99;
const NSOpenGLProfileVersionLegacy: NSOpenGLPixelFormatAttribute = 0x1000;
const NSOpenGLProfileVersion3_2Core: NSOpenGLPixelFormatAttribute = 0x3200;
const kCGLRendererGenericFloatID: NSOpenGLPixelFormatAttribute = 0x0002_0400;

/// `NSOpenGLContextParameterSwapInterval` (`NSOpenGLCPSwapInterval`).
const NSOpenGLCPSwapInterval: NSInteger = 222;
/// `NSOpenGLContextParameterSurfaceOpacity` (`NSOpenGLCPSurfaceOpacity`).
const NSOpenGLCPSurfaceOpacity: NSInteger = 236;

/// `GL_VERSION`.
const GL_VERSION: u32 = 0x1F02;

// <QuartzCore/CVDisplayLink.h>

type CVDisplayLinkRef = *mut c_void;
type CVReturn = i32;
type CVOptionFlags = u64;
const kCVReturnSuccess: CVReturn = 0;
type CVDisplayLinkOutputCallback = extern "C" fn(
    display_link: CVDisplayLinkRef,
    in_now: *const c_void,
    in_output_time: *const c_void,
    flags_in: CVOptionFlags,
    flags_out: *mut CVOptionFlags,
    display_link_context: *mut c_void,
) -> CVReturn;

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVDisplayLinkCreateWithActiveCGDisplays(display_link_out: *mut CVDisplayLinkRef)
        -> CVReturn;
    fn CVDisplayLinkSetOutputCallback(
        display_link: CVDisplayLinkRef,
        callback: CVDisplayLinkOutputCallback,
        user_info: *mut c_void,
    ) -> CVReturn;
    fn CVDisplayLinkSetCurrentCGDisplayFromOpenGLContext(
        display_link: CVDisplayLinkRef,
        cgl_context: *mut c_void,
        cgl_pixel_format: *mut c_void,
    ) -> CVReturn;
    fn CVDisplayLinkStart(display_link: CVDisplayLinkRef) -> CVReturn;
    fn CVDisplayLinkRelease(display_link: CVDisplayLinkRef);
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGDisplayIDToOpenGLDisplayMask(display: CGDirectDisplayID) -> u32;
}

// NSOpenGLContext and NSOpenGLPixelFormat live in AppKit, OpenGL's CGL
// objects in OpenGL.framework.
#[link(name = "OpenGL", kind = "framework")]
unsafe extern "C" {}

/// `SDL_opengl_async_dispatch`.
static SDL_OPENGL_ASYNC_DISPATCH: AtomicBool = AtomicBool::new(false);

/// The hint callback the contexts add (`SDL_AddHintCallback()` replaces an
/// identical one, so there is one at most).
static ASYNC_DISPATCH_WATCH: Mutex<Option<hints::Callback>> = Mutex::new(None);

/// Translation of `SDL_OpenGLAsyncDispatchChanged()`.
fn opengl_async_dispatch_changed(hint: Option<&str>) {
    SDL_OPENGL_ASYNC_DISPATCH.store(hints::string_to_bool(hint, false), Ordering::Relaxed);
}

/// `SDL_GLDriverData` and `gl_config.dll_handle`: the loaded libGL.
static GL_DLL: Mutex<Option<SharedObject>> = Mutex::new(None);

fn gl_dll() -> MutexGuard<'static, Option<SharedObject>> {
    GL_DLL.lock().unwrap_or_else(|e| e.into_inner())
}

/// Which functions the device's `GL_*` pointers are: NSOpenGL
/// (`Cocoa_GL_*`) or EGL (`Cocoa_GLES_*`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum GlBackend {
    Cgl,
    Egl,
}

/// The device's current `GL_*` functions.
static GL_BACKEND_IS_EGL: AtomicBool = AtomicBool::new(false);

/// The `GL_*` functions in use.
pub(crate) fn gl_backend() -> GlBackend {
    if GL_BACKEND_IS_EGL.load(Ordering::Relaxed) {
        GlBackend::Egl
    } else {
        GlBackend::Cgl
    }
}

/// Switch the device's `GL_*` functions.
pub(crate) fn set_gl_backend(backend: GlBackend) {
    GL_BACKEND_IS_EGL.store(backend == GlBackend::Egl, Ordering::Relaxed);
}

/// The instance variables of `SDL3OpenGLContext`.
struct ContextState {
    dirty: AtomicI32,
    window: Mutex<Option<WindowID>>,
    display_link: AtomicPtr<c_void>,
    swap_interval_mutex: Mutex<()>,
    swap_interval_cond: Condvar,
    swap_interval_setting: AtomicI32,
    swap_intervals_passed: AtomicI32,
    /// `openglPixelFormat` (macOS 10.10 has -[NSOpenGLContext pixelFormat]
    /// but this handles older OS releases).
    opengl_pixel_format: Mutex<StrongPtr>,
}

impl ContextState {
    fn swap_interval_lock(&self) -> MutexGuard<'_, ()> {
        self.swap_interval_mutex
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn window(&self) -> Option<WindowID> {
        *self.window.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Translation of `DisplayLinkCallback()`.
extern "C" fn display_link_callback(
    _display_link: CVDisplayLinkRef,
    _now: *const c_void,
    _output_time: *const c_void,
    _flags_in: CVOptionFlags,
    _flags_out: *mut CVOptionFlags,
    display_link_context: *mut c_void,
) -> CVReturn {
    // SAFETY: the context is the state of a live SDL3OpenGLContext (freed
    // after its display link is released).
    let nscontext = unsafe { &*(display_link_context as *const ContextState) };

    // printf("DISPLAY LINK! %u\n", (unsigned int) SDL_GetTicks());
    let setting = nscontext.swap_interval_setting.load(Ordering::SeqCst);
    if setting != 0 {
        // nothing to do if vsync is disabled, don't even lock
        let _guard = nscontext.swap_interval_lock();
        nscontext
            .swap_intervals_passed
            .fetch_add(1, Ordering::SeqCst);
        nscontext.swap_interval_cond.notify_one();
    }

    kCVReturnSuccess
}

static SDL3_OPENGL_CONTEXT: DeclaredClass = DeclaredClass::new();

/// The `SDL3OpenGLContext` class (`@implementation SDL3OpenGLContext : NSOpenGLContext`).
fn sdl3_opengl_context_class() -> Class {
    SDL3_OPENGL_CONTEXT.get(|| {
        let mut b = ClassBuilder::new(c"SDL3OpenGLContext", class!(NSOpenGLContext))
            .expect("SDL3OpenGLContext");
        b.add_ivar::<*mut c_void>(c"_state", c"^v");
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_method(
                sel!(movedToNewScreen),
                imp!(context_moved_to_new_screen as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(scheduleUpdate),
                imp!(context_schedule_update as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(updateIfNeeded),
                imp!(context_update_if_needed as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(update),
                imp!(context_update as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(setWindow:),
                imp!(context_set_window as extern "C" fn(Id, Sel, *mut c_void)),
                c"v@:^v",
            );
            b.add_method(
                sel!(window),
                imp!(context_window as extern "C" fn(Id, Sel) -> *mut c_void),
                c"^v@:",
            );
            b.add_method(
                sel!(explicitUpdate),
                imp!(context_explicit_update as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(cleanup),
                imp!(context_cleanup as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(dealloc),
                imp!(context_dealloc as extern "C" fn(Id, Sel)),
                c"v@:",
            );
        }
        b.register()
    })
}

/// The instance variables of an `SDL3OpenGLContext`.
///
/// # Safety
///
/// `this` must be an initialized `SDL3OpenGLContext`.
unsafe fn context_state<'a>(this: Id) -> &'a ContextState {
    // SAFETY: the caller's contract; `_state` is set by init_context() and
    // freed by dealloc.
    unsafe {
        &*(*ivar_ptr::<*mut c_void>(this, sdl3_opengl_context_class(), c"_state")
            as *const ContextState)
    }
}

/// `-[SDL3OpenGLContext initWithFormat:shareContext:]` on an allocated
/// instance: the superclass's initializer, then the instance variables.
fn init_context(obj: Id, format: Id, share: Id) -> Id {
    // SAFETY: an allocated SDL3OpenGLContext and a pixel format.
    unsafe {
        let this: Id = msg_send![obj, initWithFormat: format shareContext: share];
        if !this.is_null() {
            let state = Box::new(ContextState {
                dirty: AtomicI32::new(0),
                window: Mutex::new(None),
                display_link: AtomicPtr::new(std::ptr::null_mut()),
                swap_interval_mutex: Mutex::new(()),
                swap_interval_cond: Condvar::new(),
                swap_interval_setting: AtomicI32::new(0),
                swap_intervals_passed: AtomicI32::new(0),
                opengl_pixel_format: Mutex::new(StrongPtr::retain(format)),
            });
            let state = Box::into_raw(state);
            *ivar_ptr::<*mut c_void>(this, sdl3_opengl_context_class(), c"_state") =
                state as *mut c_void;

            // !!! FIXME: check return values.
            let mut display_link: CVDisplayLinkRef = std::ptr::null_mut();
            CVDisplayLinkCreateWithActiveCGDisplays(&mut display_link);
            CVDisplayLinkSetOutputCallback(
                display_link,
                display_link_callback,
                state as *mut c_void,
            );
            let cgl_context: *mut c_void = msg_send![this, CGLContextObj];
            let cgl_pixel_format: *mut c_void = msg_send![format, CGLPixelFormatObj];
            CVDisplayLinkSetCurrentCGDisplayFromOpenGLContext(
                display_link,
                cgl_context,
                cgl_pixel_format,
            );
            CVDisplayLinkStart(display_link);
            (*state).display_link.store(display_link, Ordering::SeqCst);
        }

        // (SDL_AddHintCallback() calls it with the current value)
        let watch = hints::watch(hints::MAC_OPENGL_ASYNC_DISPATCH, |change| {
            opengl_async_dispatch_changed(change.new_value)
        })
        .ok();
        opengl_async_dispatch_changed(hints::get(hints::MAC_OPENGL_ASYNC_DISPATCH).as_deref());
        *ASYNC_DISPATCH_WATCH
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = watch;
        this
    }
}

/// Translation of `-[SDL3OpenGLContext movedToNewScreen]`.
extern "C" fn context_moved_to_new_screen(this: Id, _sel: Sel) {
    // SAFETY: an SDL3OpenGLContext and its CGL objects.
    unsafe {
        let state = context_state(this);
        let display_link = state.display_link.load(Ordering::SeqCst);
        if !display_link.is_null() {
            let format = state
                .opengl_pixel_format
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get();
            let cgl_context: *mut c_void = msg_send![this, CGLContextObj];
            let cgl_pixel_format: *mut c_void = msg_send![format, CGLPixelFormatObj];
            CVDisplayLinkSetCurrentCGDisplayFromOpenGLContext(
                display_link,
                cgl_context,
                cgl_pixel_format,
            );
        }
    }
}

/// Translation of `-[SDL3OpenGLContext scheduleUpdate]`.
extern "C" fn context_schedule_update(this: Id, _sel: Sel) {
    // SAFETY: an SDL3OpenGLContext.
    unsafe { context_state(this) }
        .dirty
        .fetch_add(1, Ordering::SeqCst);
}

/// Translation of `-[SDL3OpenGLContext updateIfNeeded]`.
///
/// This should only be called on the thread on which a user is using the context.
extern "C" fn context_update_if_needed(this: Id, _sel: Sel) {
    // SAFETY: an SDL3OpenGLContext.
    let value = unsafe { context_state(this) }
        .dirty
        .swap(0, Ordering::SeqCst);
    if value > 0 {
        // We call the real underlying update here, since -[SDL3OpenGLContext update] just calls us.
        // SAFETY: the method declared above.
        let _: () = unsafe { msg_send![this, explicitUpdate] };
    }
}

/// Translation of `-[SDL3OpenGLContext update]`.
///
/// This should only be called on the thread on which a user is using the context.
extern "C" fn context_update(this: Id, _sel: Sel) {
    // This ensures that regular 'update' calls clear the atomic dirty flag.
    // SAFETY: the methods declared above.
    unsafe {
        let _: () = msg_send![this, scheduleUpdate];
        let _: () = msg_send![this, updateIfNeeded];
    }
}

/// `[context setView:view]` on the main thread.
fn set_view_on_main_thread(this: Id, view: Id) {
    let (this, view) = (WeakId(this), WeakId(view));
    run_on_main_sync(move || {
        // (capture the Send wrappers whole, not their fields)
        let (this, view) = (this, view);
        // SAFETY: the context and view outlive the synchronous call.
        let _: () = unsafe { msg_send![this.0, setView: view.0] };
    });
}

/// Translation of `-[SDL3OpenGLContext setWindow:]`: updates the drawable
/// for the contexts and manages related state. The argument is the SDL
/// window's ID (NULL: none).
extern "C" fn context_set_window(this: Id, _sel: Sel, new_window: *mut c_void) {
    let new_window = (!new_window.is_null()).then_some(new_window as usize as WindowID);
    // SAFETY: an SDL3OpenGLContext.
    let state = unsafe { context_state(this) };

    if let Some(old) = state.window() {
        if let Some(oldwindowdata) = window_data(old) {
            // Make sure to remove us from the old window's context list, or we'll get scheduled updates from it too.
            // (@synchronized(contexts))
            oldwindowdata.state().nscontexts.retain(|c| c.get() != this);
        }
    }

    *state.window.lock().unwrap_or_else(|e| e.into_inner()) = new_window;

    if let Some(windowdata) = new_window.and_then(window_data) {
        let contentview = windowdata.sdl_content_view.get();

        // Now sign up for scheduled updates for the new window.
        // (@synchronized(contexts))
        // SAFETY: retaining the live context.
        windowdata
            .state()
            .nscontexts
            .push(unsafe { StrongPtr::retain(this) });

        // SAFETY: an NSOpenGLContext's view and the current context.
        unsafe {
            let view: Id = msg_send![this, view];
            if view != contentview {
                set_view_on_main_thread(this, contentview);
                let current: Id = msg_send![class!(NSOpenGLContext), currentContext];
                if this == current {
                    let _: () = msg_send![this, explicitUpdate];
                } else {
                    let _: () = msg_send![this, scheduleUpdate];
                }
            }
        }
    } else {
        set_view_on_main_thread(this, NIL);
    }
}

/// Translation of `-[SDL3OpenGLContext window]` (the window's ID, NULL if
/// none).
extern "C" fn context_window(this: Id, _sel: Sel) -> *mut c_void {
    // SAFETY: an SDL3OpenGLContext.
    unsafe { context_state(this) }
        .window()
        .map_or(std::ptr::null_mut(), |w| w as usize as *mut c_void)
}

/// Translation of `-[SDL3OpenGLContext explicitUpdate]`.
extern "C" fn context_explicit_update(this: Id, _sel: Sel) {
    fn super_update(this: Id) {
        // SAFETY: NSOpenGLContext's own update.
        let _: () = unsafe { msg_send_super![this, sdl3_opengl_context_class(), update] };
    }
    if is_main_thread() {
        super_update(this);
    } else if SDL_OPENGL_ASYNC_DISPATCH.load(Ordering::Relaxed) {
        // (the block retains self)
        // SAFETY: a live context, retained until the block ran.
        let ctx = unsafe { StrongPtr::retain(this) };
        run_on_main_async(move || super_update(ctx.get()));
    } else {
        let this = WeakId(this);
        run_on_main_sync(move || {
            // (capture the Send wrapper whole, not its field)
            let this = this;
            super_update(this.0)
        });
    }
}

/// Translation of `-[SDL3OpenGLContext cleanup]`.
extern "C" fn context_cleanup(this: Id, _sel: Sel) {
    // SAFETY: the method declared above.
    let _: () = unsafe { msg_send![this, setWindow: std::ptr::null_mut::<c_void>()] };

    *ASYNC_DISPATCH_WATCH
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = None;
    // SAFETY: an SDL3OpenGLContext.
    let state = unsafe { context_state(this) };
    let display_link = state
        .display_link
        .swap(std::ptr::null_mut(), Ordering::SeqCst);
    if !display_link.is_null() {
        // SAFETY: the display link the context made.
        unsafe { CVDisplayLinkRelease(display_link) };
    }
    // (the swap interval mutex and condition are freed with the state)
}

/// `-[SDL3OpenGLContext dealloc]`: frees the instance variables (ARC's
/// part upstream).
extern "C" fn context_dealloc(this: Id, _sel: Sel) {
    // SAFETY: the state init_context() leaked, freed once, then the
    // superclass's dealloc.
    unsafe {
        let p = ivar_ptr::<*mut c_void>(this, sdl3_opengl_context_class(), c"_state");
        if !(*p).is_null() {
            let state = Box::from_raw(*p as *mut ContextState);
            let display_link = state.display_link.load(Ordering::SeqCst);
            if !display_link.is_null() {
                CVDisplayLinkRelease(display_link);
            }
            *p = std::ptr::null_mut();
            drop(state);
        }
        let _: () = msg_send_super![this, sdl3_opengl_context_class(), dealloc];
    }
}

/// The NSOpenGLContext of an SDL context handle.
fn nscontext_of(context: RawGlContext) -> Id {
    context.as_ptr() as Id
}

/// Translation of `Cocoa_GL_LoadLibrary()`.
pub(crate) fn cgl_load_library(path: Option<&str>) -> Result<()> {
    // Load the OpenGL library
    let path = path
        .map(str::to_owned)
        .or_else(|| hints::get(hints::OPENGL_LIBRARY))
        .unwrap_or_else(|| DEFAULT_OPENGL.to_owned());
    let dll_handle = SharedObject::load(&path)?;
    *gl_dll() = Some(dll_handle);
    gl::set_driver_path(Some(&path));
    Ok(())
}

/// Translation of `Cocoa_GL_GetProcAddress()`.
pub(crate) fn cgl_get_proc_address(proc_name: &str) -> Option<std::ptr::NonNull<c_void>> {
    gl_dll().as_ref()?.symbol(proc_name).ok()
}

/// Translation of `Cocoa_GL_UnloadLibrary()`.
pub(crate) fn cgl_unload_library() {
    *gl_dll() = None;
}

/// Translation of `Cocoa_GL_CreateContext()`.
pub(crate) fn cgl_create_context(window: WindowID) -> Result<RawGlContext> {
    autoreleasepool(|| {
        let config = gl::gl_config();

        if config.profile_mask == GL_CONTEXT_PROFILE_ES {
            // Switch to EGL based functions
            cgl_unload_library();
            set_gl_backend(GlBackend::Egl);

            super::opengles::gles_load_library(None)?;
            return super::opengles::gles_create_context(window);
        }

        let display_id = crate::video::window::display_for_window(window)?;
        let (bytes_per_pixel, cg_display) = with_display(display_id, |d| {
            let cg_display = d
                .internal
                .as_ref()
                .and_then(|i| i.downcast_ref::<DisplayData>())
                .map_or(0, |data| data.display);
            (d.current_mode().format.bytes_per_pixel(), cg_display)
        })
        .ok_or_else(|| Error::new("Invalid display"))?;

        let mut attr: Vec<NSOpenGLPixelFormatAttribute> = Vec::with_capacity(32);
        attr.push(NSOpenGLPFAAllowOfflineRenderers);

        let mut profile = NSOpenGLProfileVersionLegacy;
        if config.profile_mask == GL_CONTEXT_PROFILE_CORE {
            profile = NSOpenGLProfileVersion3_2Core;
        }
        attr.push(NSOpenGLPFAOpenGLProfile);
        attr.push(profile);

        attr.push(NSOpenGLPFAColorSize);
        attr.push(bytes_per_pixel * 8);

        attr.push(NSOpenGLPFADepthSize);
        attr.push(config.depth_size as u32);

        if config.double_buffer != 0 {
            attr.push(NSOpenGLPFADoubleBuffer);
        }

        if config.stereo != 0 {
            attr.push(NSOpenGLPFAStereo);
        }

        if config.stencil_size != 0 {
            attr.push(NSOpenGLPFAStencilSize);
            attr.push(config.stencil_size as u32);
        }

        if (config.accum_red_size
            + config.accum_green_size
            + config.accum_blue_size
            + config.accum_alpha_size)
            > 0
        {
            attr.push(NSOpenGLPFAAccumSize);
            attr.push(
                (config.accum_red_size
                    + config.accum_green_size
                    + config.accum_blue_size
                    + config.accum_alpha_size) as u32,
            );
        }

        if config.multisamplebuffers != 0 {
            attr.push(NSOpenGLPFASampleBuffers);
            attr.push(config.multisamplebuffers as u32);
        }

        if config.multisamplesamples != 0 {
            attr.push(NSOpenGLPFASamples);
            attr.push(config.multisamplesamples as u32);
            attr.push(NSOpenGLPFANoRecovery);
        }
        if config.floatbuffers != 0 {
            attr.push(NSOpenGLPFAColorFloat);
        }

        if config.accelerated >= 0 {
            if config.accelerated != 0 {
                attr.push(NSOpenGLPFAAccelerated);
            } else {
                attr.push(NSOpenGLPFARendererID);
                attr.push(kCGLRendererGenericFloatID);
            }
        }

        attr.push(NSOpenGLPFAScreenMask);
        // SAFETY: a display ID.
        attr.push(unsafe { CGDisplayIDToOpenGLDisplayMask(cg_display) });
        attr.push(0);

        // SAFETY: NSOpenGLPixelFormat from a zero-terminated attribute
        // list, and the context class declared above.
        unsafe {
            let fmt: Id = msg_send![super::alloc(class!(NSOpenGLPixelFormat)), initWithAttributes: attr.as_ptr()];
            let fmt = StrongPtr::from_owned(fmt);
            if fmt.is_nil() {
                return Err(Error::new("Failed creating OpenGL pixel format"));
            }

            let mut share_context: Id = NIL;
            if config.share_with_current_context != 0 {
                share_context = gl::current_thread_context().map_or(NIL, nscontext_of);
            }

            let context = init_context(
                super::alloc(sdl3_opengl_context_class()),
                fmt.get(),
                share_context,
            );

            if context.is_null() {
                return Err(Error::new("Failed creating OpenGL context"));
            }

            // (CFBridgingRetain: the handle owns the +1 from alloc/init)
            let sdlcontext = RawGlContext::from_ptr(context as *mut c_void)
                .ok_or_else(|| Error::new("Failed creating OpenGL context"))?;

            // vsync is handled separately by synchronizing with a display link.
            let interval: i32 = 0;
            let _: () = msg_send![context, setValues: &interval as *const i32 forParameter: NSOpenGLCPSwapInterval];

            let transparent = with_window(window, |w| w.flags().contains(WindowFlags::TRANSPARENT))
                .unwrap_or(false);
            let opaque: i32 = if transparent { 0 } else { 1 };
            let _: () = msg_send![context, setValues: &opaque as *const i32 forParameter: NSOpenGLCPSurfaceOpacity];

            if cgl_make_current(Some(window), Some(sdlcontext)).is_err() {
                destroy_failed_context(sdlcontext);
                return Err(Error::new("Failed making OpenGL context current"));
            }

            if config.major_version < 3 && config.profile_mask == 0 && config.flags == 0 {
                // This is a legacy profile, so to match other backends, we're done.
            } else {
                type PfnGlGetString = unsafe extern "C" fn(u32) -> *const u8;
                let Some(gl_get_string_func) = cgl_get_proc_address("glGetString") else {
                    destroy_failed_context(sdlcontext);
                    return Err(Error::new("Failed getting OpenGL glGetString entry point"));
                };
                let gl_get_string_func: PfnGlGetString =
                    std::mem::transmute(gl_get_string_func.as_ptr());

                let glversion = gl_get_string_func(GL_VERSION);
                if glversion.is_null() {
                    destroy_failed_context(sdlcontext);
                    return Err(Error::new("Failed getting OpenGL context version"));
                }
                let glversion = std::ffi::CStr::from_ptr(glversion as *const std::ffi::c_char)
                    .to_string_lossy()
                    .into_owned();

                let Some((glversion_major, glversion_minor)) = parse_gl_version(&glversion) else {
                    destroy_failed_context(sdlcontext);
                    return Err(Error::new("Failed parsing OpenGL context version"));
                };

                if (glversion_major < config.major_version)
                    || ((glversion_major == config.major_version)
                        && (glversion_minor < config.minor_version))
                {
                    destroy_failed_context(sdlcontext);
                    return Err(Error::new(
                        "Failed creating OpenGL context at version requested",
                    ));
                }

                /* In the future we'll want to do this, but to match other platforms
                we'll leave the OpenGL version the way it is for now
                */
                // _this->gl_config.major_version = glversion_major;
                // _this->gl_config.minor_version = glversion_minor;
            }
            Ok(sdlcontext)
        }
    })
}

/// `SDL_GL_DestroyContext(sdlcontext)` on a context that failed its
/// checks: it is current in the driver (not yet in SDL's state).
fn destroy_failed_context(context: RawGlContext) {
    let _ = cgl_make_current(None, None);
    let _ = cgl_destroy_context(context);
}

/// Translation of `Cocoa_GL_MakeCurrent()`.
pub(crate) fn cgl_make_current(
    window: Option<WindowID>,
    context: Option<RawGlContext>,
) -> Result<()> {
    autoreleasepool(|| {
        // SAFETY: an SDL3OpenGLContext, and NSOpenGLContext's class method.
        unsafe {
            if let Some(context) = context {
                let nscontext = nscontext_of(context);
                let current_window: *mut c_void = msg_send![nscontext, window];
                let window_ptr = window.map_or(std::ptr::null_mut(), |w| w as usize as *mut c_void);
                if current_window != window_ptr {
                    let _: () = msg_send![nscontext, setWindow: window_ptr];
                    let _: () = msg_send![nscontext, updateIfNeeded];
                }
                let _: () = msg_send![nscontext, makeCurrentContext];
            } else {
                let _: () = msg_send![class!(NSOpenGLContext), clearCurrentContext];
            }
        }

        Ok(())
    })
}

/// Translation of `Cocoa_GL_SetSwapInterval()`.
pub(crate) fn cgl_set_swap_interval(interval: i32) -> Result<()> {
    autoreleasepool(|| {
        let Some(context) = gl::current_thread_context() else {
            return Err(Error::new("No current OpenGL context"));
        };
        // SAFETY: an SDL3OpenGLContext.
        let nscontext = unsafe { context_state(nscontext_of(context)) };
        let _guard = nscontext.swap_interval_lock();
        nscontext.swap_intervals_passed.store(0, Ordering::SeqCst);
        nscontext
            .swap_interval_setting
            .store(interval, Ordering::SeqCst);
        Ok(())
    })
}

/// Translation of `Cocoa_GL_GetSwapInterval()`.
pub(crate) fn cgl_get_swap_interval() -> Result<i32> {
    autoreleasepool(|| match gl::current_thread_context() {
        // SAFETY: an SDL3OpenGLContext.
        Some(context) => Ok(unsafe { context_state(nscontext_of(context)) }
            .swap_interval_setting
            .load(Ordering::SeqCst)),
        None => Err(Error::new("no OpenGL context")),
    })
}

/// Translation of `Cocoa_GL_SwapWindow()`.
pub(crate) fn cgl_swap_window(_window: WindowID) -> Result<()> {
    autoreleasepool(|| {
        let Some(context) = gl::current_thread_context() else {
            // (upstream dereferences the NULL context)
            return Err(Error::new("No current OpenGL context"));
        };
        let nscontext = nscontext_of(context);
        // SAFETY: an SDL3OpenGLContext.
        let state = unsafe { context_state(nscontext) };
        let setting = state.swap_interval_setting.load(Ordering::SeqCst);

        if setting == 0 {
            // nothing to do if vsync is disabled, don't even lock
        } else if setting < 0 {
            // late swap tearing
            let mut guard = state.swap_interval_lock();
            while state.swap_intervals_passed.load(Ordering::SeqCst) == 0 {
                guard = state
                    .swap_interval_cond
                    .wait(guard)
                    .unwrap_or_else(|e| e.into_inner());
            }
            state.swap_intervals_passed.store(0, Ordering::SeqCst);
        } else {
            let mut guard = state.swap_interval_lock();
            loop {
                // always wait here so we know we just hit a swap interval.
                guard = state
                    .swap_interval_cond
                    .wait(guard)
                    .unwrap_or_else(|e| e.into_inner());
                if state.swap_intervals_passed.load(Ordering::SeqCst) % setting == 0 {
                    break;
                }
            }
            state.swap_intervals_passed.store(0, Ordering::SeqCst);
        }

        // { static Uint64 prev = 0; const Uint64 now = SDL_GetTicks(); const unsigned int diff = (unsigned int) (now - prev); prev = now; printf("GLSWAPBUFFERS TICKS %u\n", diff); }

        /* on 10.14 ("Mojave") and later, this deadlocks if two contexts in two
        threads try to swap at the same time, so put a mutex around it. */
        let videodata = super::video_data();
        let _swaplock = videodata
            .as_ref()
            .map(|d| d.swaplock.lock().unwrap_or_else(|e| e.into_inner()));
        // SAFETY: the current context.
        unsafe {
            let _: () = msg_send![nscontext, flushBuffer];
            let _: () = msg_send![nscontext, updateIfNeeded];
        }
        Ok(())
    })
}

/// Translation of `Cocoa_GL_DestroyContext()`.
pub(crate) fn cgl_destroy_context(context: RawGlContext) -> Result<()> {
    /// Translation of `DispatchedDestroyContext()`.
    fn dispatched_destroy_context(context: usize) {
        autoreleasepool(|| {
            let nscontext = context as Id;
            // SAFETY: the context handle's reference, released once.
            unsafe {
                let _: () = msg_send![nscontext, cleanup];
                crate::core::apple::objc::release(nscontext);
            }
        })
    }

    let context = context.as_ptr() as usize;
    if is_main_thread() {
        dispatched_destroy_context(context);
    } else if SDL_OPENGL_ASYNC_DISPATCH.load(Ordering::Relaxed) {
        run_on_main_async(move || dispatched_destroy_context(context));
    } else {
        run_on_main_sync(move || dispatched_destroy_context(context));
    }

    Ok(())
}

/// `SDL_sscanf(glversion, "%d.%d", &major, &minor) == 2`.
fn parse_gl_version(s: &str) -> Option<(i32, i32)> {
    let s = s.trim_start();
    let digits = |s: &str| -> Option<(i32, usize)> {
        let (sign, rest) = match s.as_bytes().first() {
            Some(b'-') => (-1, &s[1..]),
            Some(b'+') => (1, &s[1..]),
            _ => (1, s),
        };
        let n = rest.bytes().take_while(u8::is_ascii_digit).count();
        if n == 0 {
            return None;
        }
        Some((
            sign * rest[..n].parse::<i32>().ok()?,
            s.len() - rest.len() + n,
        ))
    };
    let (major, used) = digits(s)?;
    let rest = s[used..].strip_prefix('.')?;
    let (minor, _) = digits(rest.trim_start())?;
    Some((major, minor))
}

#[cfg(test)]
mod tests {
    use super::parse_gl_version;

    #[test]
    fn gl_version_strings() {
        assert_eq!(parse_gl_version("4.1 Metal - 89.3"), Some((4, 1)));
        assert_eq!(parse_gl_version("2.1 Metal - 89.3"), Some((2, 1)));
        assert_eq!(parse_gl_version("  3.3 (Core Profile)"), Some((3, 3)));
        assert_eq!(parse_gl_version("OpenGL ES 3.0"), None);
        assert_eq!(parse_gl_version("4"), None);
    }
}
