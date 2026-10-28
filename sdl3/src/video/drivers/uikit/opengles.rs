// Rust translation of src/video/uikit/SDL_uikitopengles.m and
// SDL_uikitopengles.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! OpenGL ES contexts on UIKit (iOS and tvOS; EAGL, which Apple has
//! deprecated): an `SDLEAGLContext`, an `EAGLContext` subclass declared at
//! run time, owns the [`SDL_uikitopenglview`](super::openglview) it draws
//! to, which becomes its window's view while the context is current for
//! it; contexts can share a share group (`SDL_GL_SHARE_WITH_CURRENT_CONTEXT`).
//! OpenGL ES 1.1, 2.0 and 3.0 are offered (the rendering API is the major
//! version). The view's framebuffer, renderbuffer and resolve framebuffer
//! are the window's `SDL_PROP_WINDOW_UIKIT_OPENGL_*_NUMBER` properties;
//! there's no swap interval to set (the display link paces an app).
//!
//! The GL functions are OpenGLES.framework's, which is linked (on iOS and
//! tvOS, as upstream links it): [`gl_get_proc_address`] looks them up in
//! every loaded image, as upstream does, and there's no library to load.

use std::ffi::{c_void, CString};

use super::openglview::{
    current_context, drawable_framebuffer, drawable_renderbuffer, msaa_resolve_framebuffer,
    new_gl_view, set_current_context, GlViewConfig,
};
use super::sys::EAGLRenderingAPI;
use super::view::set_sdl_window;
use super::window::window_data;
use crate::core::apple::foundation::CGFloat;
use crate::core::apple::objc::{
    autoreleasepool, class, imp, ivar_ptr, msg_send, msg_send_super, release, retain, sel, Class,
    ClassBuilder, DeclaredClass, Id, Sel, StrongPtr, NIL,
};
use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::WindowID;
use crate::hints;
use crate::video::core::with_window;
use crate::video::gl::{self, RawGlContext};
use crate::video::window::{
    PROP_WINDOW_UIKIT_OPENGL_FRAMEBUFFER_NUMBER, PROP_WINDOW_UIKIT_OPENGL_RENDERBUFFER_NUMBER,
    PROP_WINDOW_UIKIT_OPENGL_RESOLVE_FRAMEBUFFER_NUMBER,
};
use crate::video::Window;

static SDL_EAGLCONTEXT: DeclaredClass = DeclaredClass::new();

/// The `SDLEAGLContext` class (`@interface SDLEAGLContext : EAGLContext`),
/// with its `sdlView` (strong): the OpenGL ES context owns a view /
/// drawable.
fn eaglcontext_class() -> Class {
    SDL_EAGLCONTEXT.get(|| {
        let mut b =
            ClassBuilder::new(c"SDLEAGLContext", class!(EAGLContext)).expect("SDLEAGLContext");
        b.add_ivar::<Id>(c"_sdlView", c"@");
        // SAFETY: the implementation has the signature of the encoding.
        unsafe {
            b.add_method(
                sel!(dealloc),
                imp!(context_dealloc as extern "C" fn(Id, Sel)),
                c"v@:",
            );
        }
        b.register()
    })
}

/// The `_sdlView` variable of a context.
fn sdl_view_ivar(context: Id) -> *mut Id {
    // SAFETY: an SDLEAGLContext, which has the variable.
    unsafe { ivar_ptr::<Id>(context, eaglcontext_class(), c"_sdlView") }
}

/// `context.sdlView` (nil for nil).
fn sdl_view(context: Id) -> Id {
    if context.is_null() {
        return NIL;
    }
    // SAFETY: the context's variable.
    unsafe { *sdl_view_ivar(context) }
}

/// `context.sdlView = view` (retained).
fn set_sdl_view(context: Id, view: Id) {
    // SAFETY: the context's variable, which holds a reference.
    unsafe {
        let p = sdl_view_ivar(context);
        let old = *p;
        *p = retain(view);
        release(old);
    }
}

/// Translation of `-[SDLEAGLContext dealloc]`, with what ARC adds: the
/// view released, then the superclass's.
extern "C" fn context_dealloc(this: Id, _sel: Sel) {
    /* When the context is deallocated, its view should be removed from any
     * SDL window that it's attached to. */
    let view = sdl_view(this);
    if !view.is_null() {
        set_sdl_window(view, None);
    }
    // SAFETY: the variable's reference, released once; then EAGLContext's
    // dealloc.
    unsafe {
        *sdl_view_ivar(this) = NIL;
        release(view);
        let () = msg_send_super![this, eaglcontext_class(), dealloc];
    }
}

/// The `SDLEAGLContext` of an SDL context handle.
fn eaglcontext_of(context: RawGlContext) -> Id {
    context.as_ptr() as Id
}

/// Translation of `UIKit_GL_GetProcAddress()`: the function, from any
/// loaded image.
pub(crate) fn gl_get_proc_address(proc_name: &str) -> Option<std::ptr::NonNull<c_void>> {
    /* Look through all SO's for the proc symbol.  Here's why:
     * -Looking for the path to the OpenGL Library seems not to work in the iOS Simulator.
     * -We don't know that the path won't change in the future. */
    // (a name with a NUL in it names nothing)
    let name = CString::new(proc_name).ok()?;
    // SAFETY: RTLD_DEFAULT searches every loaded image; the name is
    // NUL-terminated.
    std::ptr::NonNull::new(unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) })
}

/// Translation of `UIKit_GL_MakeCurrent()`: the context's view moves to
/// the window.
///
/// note that SDL_GL_DestroyContext makes it current without passing the window
pub(crate) fn gl_make_current(
    window: Option<WindowID>,
    context: Option<RawGlContext>,
) -> Result<()> {
    autoreleasepool(|| {
        let eaglcontext = context.map_or(NIL, eaglcontext_of);

        if !set_current_context(eaglcontext) {
            return Err(Error::new("Could not make EAGL context current"));
        }

        if !eaglcontext.is_null() {
            let view = sdl_view(eaglcontext);
            // (a message to nil without a view)
            if !view.is_null() {
                set_sdl_window(view, window);
            }
        }

        Ok(())
    })
}

/// Translation of `UIKit_GL_LoadLibrary()`.
pub(crate) fn gl_load_library(path: Option<&str>) -> Result<()> {
    /* We shouldn't pass a path to this function, since we've already loaded the
     * library. */
    if path.is_some() {
        return Err(Error::new(
            "iOS GL Load Library just here for compatibility",
        ));
    }
    Ok(())
}

/// Translation of `UIKit_GL_SwapWindow()`: the current context's view
/// shows its drawable.
pub(crate) fn gl_swap_window(_window: WindowID) -> Result<()> {
    autoreleasepool(|| {
        let context = gl::current_thread_context().map_or(NIL, eaglcontext_of);

        // SDL_POWER_UIKIT
        // Check once a frame to see if we should turn off the battery monitor.
        crate::power::uikit::update_battery_monitoring();

        let view = sdl_view(context);
        if !view.is_null() {
            // SAFETY: an SDL_uikitopenglview's method.
            unsafe {
                let () = msg_send![view, swapBuffers];
            }
        }

        /* You need to pump events in order for the OS to make changes visible.
         * We don't pump events here because we don't want iOS application events
         * (low memory, terminate, etc.) to happen inside low level rendering. */
    });
    Ok(())
}

/// Translation of `UIKit_GL_CreateContext()`: a context (owning a
/// reference, which [`gl_destroy_context`] releases) for the window's
/// attributes, current for the window, whose view becomes the window's.
pub(crate) fn gl_create_context(window: WindowID) -> Result<RawGlContext> {
    autoreleasepool(|| {
        let data = window_data(window).ok_or_else(|| Error::invalid_param("window"))?;
        // SAFETY: the UIWindow's screen.
        let screen: Id = unsafe { msg_send![data.uiwindow(), screen] };
        let frame = super::compute_view_frame(window, screen);
        let config = gl::gl_config();
        let mut sharegroup: Id = NIL;
        let mut scale: CGFloat = 1.0;
        let mut samples = 0;
        let major = config.major_version;
        let minor = config.minor_version;

        /* The EAGLRenderingAPI enum values currently map 1:1 to major GLES
         * versions. */
        let api = major as EAGLRenderingAPI;

        // iOS currently doesn't support GLES >3.0.
        if major > 3 || (major == 3 && minor > 0) {
            return Err(Error::new(format!(
                "OpenGL ES {major}.{minor} context could not be created"
            )));
        }

        if config.multisamplebuffers > 0 {
            samples = config.multisamplesamples;
        }

        if config.share_with_current_context != 0 {
            let curr_context = gl::current_thread_context().map_or(NIL, eaglcontext_of);
            // SAFETY: an EAGLContext property (a message to nil without one).
            sharegroup = unsafe { msg_send![curr_context, sharegroup] };
        }

        if with_window(window, |w| {
            w.flags().contains(WindowFlags::HIGH_PIXEL_DENSITY)
        })? {
            /* Set the scale to the natural scale factor of the screen - the
             * backing dimensions of the OpenGL view will match the pixel
             * dimensions of the screen rather than the dimensions in points. */
            // SAFETY: a UIScreen property.
            scale = unsafe { msg_send![screen, nativeScale] };
        }

        // SAFETY: the class's alloc and EAGLContext's initializer (owned).
        let context = unsafe {
            let obj: Id = msg_send![eaglcontext_class(), alloc];
            StrongPtr::from_owned(msg_send![obj, initWithAPI: api sharegroup: sharegroup])
        };
        if context.is_nil() {
            return Err(Error::new(format!(
                "OpenGL ES {major} context could not be created"
            )));
        }

        let mut srgb = config.framebuffer_srgb_capable;
        if let Some(srgbhint) = hints::get(hints::OPENGL_FORCE_SRGB_FRAMEBUFFER) {
            if !srgbhint.is_empty() {
                // there is no "skip" here, since initWithFrame expects it, so we'll treat it as false.
                srgb = hints::string_to_bool(Some(&srgbhint), false) as i32;
            }
        }

        // construct our view, passing in SDL's OpenGL configuration data
        let view = new_gl_view(
            &GlViewConfig {
                frame,
                scale,
                retain_backing: config.retained_backing != 0,
                r_bits: config.red_size,
                g_bits: config.green_size,
                b_bits: config.blue_size,
                a_bits: config.alpha_size,
                depth_bits: config.depth_size,
                stencil_bits: config.stencil_size,
                srgb,
                multisamples: samples,
            },
            context.get(),
        )?;

        let props = Window::from_raw(window).properties()?;
        let _ = props.set(
            PROP_WINDOW_UIKIT_OPENGL_FRAMEBUFFER_NUMBER,
            drawable_framebuffer(view.get()) as i64,
        );
        let _ = props.set(
            PROP_WINDOW_UIKIT_OPENGL_RENDERBUFFER_NUMBER,
            drawable_renderbuffer(view.get()) as i64,
        );
        let _ = props.set(
            PROP_WINDOW_UIKIT_OPENGL_RESOLVE_FRAMEBUFFER_NUMBER,
            msaa_resolve_framebuffer(view.get()) as i64,
        );

        // The context owns the view / drawable.
        set_sdl_view(context.get(), view.get());
        drop(view);

        let raw = RawGlContext::from_ptr(context.get() as *mut c_void)
            .ok_or_else(|| Error::new("OpenGL ES context could not be created"))?;
        // (on failure, the context is released: UIKit_GL_DestroyContext())
        gl_make_current(Some(window), Some(raw))?;

        /* We return a +1'd context. The window's internal owns the view (via
         * MakeCurrent.) */
        let _ = context.into_raw();
        Ok(raw)
    })
}

/// Translation of `UIKit_GL_DestroyContext()`.
pub(crate) fn gl_destroy_context(context: RawGlContext) -> Result<()> {
    autoreleasepool(|| {
        /* The context was retained in SDL_GL_CreateContext, so we release it
         * here. The context's view will be detached from its window when the
         * context is deallocated. */
        // SAFETY: the handle's reference, released once.
        unsafe { release(eaglcontext_of(context)) };
    });
    Ok(())
}

/// Make this thread's current SDL context EAGL's current context again
/// if something else made another one current. Translation of
/// `UIKit_GL_RestoreCurrentContext()`, which the event pump and the
/// animation callbacks call.
pub(crate) fn restore_current_context() {
    autoreleasepool(|| {
        /* Some iOS system functionality (such as Dictation on the on-screen
        keyboard) uses its own OpenGL ES context but doesn't restore the
        previous one when it's done. This is a workaround to make sure the
        expected SDL-created OpenGL ES context is active after the OS is
        finished running its own code for the frame. If this isn't done, the
        app may crash or have other nasty symptoms when Dictation is used.
        */
        let context = gl::current_thread_context().map_or(NIL, eaglcontext_of);
        if !context.is_null() && current_context() != context {
            set_current_context(context);
        }
    })
}
