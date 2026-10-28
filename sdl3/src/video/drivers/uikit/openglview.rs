// Rust translation of src/video/uikit/SDL_uikitopenglview.m and
// SDL_uikitopenglview.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! OpenGL ES views: `SDL_uikitopenglview`, an [`SDL_uikitview`](super::view)
//! backed by a `CAEAGLLayer`, with the framebuffer object its OpenGL ES
//! context draws to (an `SDLEAGLContext` owns its view, see
//! [`super::opengles`]): a color renderbuffer whose storage is the layer's
//! drawable (RGBA8, RGB565 or sRGB RGBA8, retained or not), a depth (or
//! packed depth and stencil) renderbuffer if asked for, and with
//! multisampling a framebuffer of its own to draw to, resolved into the
//! layer's at each swap. The renderbuffers' storage follows the view's
//! bounds at its scale (`layoutSubviews`).
//!
//! The view's instance variables (the GL objects, the backing size, the
//! samples) are a Rust structure the object points to; its `context` is a
//! weak reference, as upstream's (`objc_storeWeak()`, which ARC uses).

use std::cell::Cell;
use std::ffi::c_void;

use super::sys::*;
use super::view::uikitview_class;
use crate::core::apple::foundation::{ns_number_bool, CGFloat, CGRect};
use crate::core::apple::objc::{
    class, imp, ivar_ptr, msg_send, msg_send_super, release, sel, Bool, Class, ClassBuilder,
    DeclaredClass, Id, NSUInteger, Sel, StrongPtr, NIL, YES,
};
use crate::error::{Error, Result};

// The weak references of the Objective-C runtime (what ARC's `weak`
// compiles to).
#[link(name = "objc", kind = "dylib")]
unsafe extern "C" {
    fn objc_storeWeak(location: *mut Id, obj: Id) -> Id;
    fn objc_loadWeakRetained(location: *mut Id) -> Id;
    fn objc_destroyWeak(location: *mut Id);
}

/// The instance variables of an `SDL_uikitopenglview`.
struct ViewState {
    /// `context` (weak).
    context: std::cell::UnsafeCell<Id>,
    /// The renderbuffer and framebuffer used to render to this layer.
    view_renderbuffer: Cell<GLuint>,
    view_framebuffer: Cell<GLuint>,
    /// The depth buffer that is attached to viewFramebuffer, if it exists.
    depth_renderbuffer: Cell<GLuint>,
    color_buffer_format: Cell<GLenum>,
    /// format of depthRenderbuffer
    depth_buffer_format: Cell<GLenum>,
    /// The framebuffer and renderbuffer used for rendering with MSAA.
    msaa_framebuffer: Cell<GLuint>,
    msaa_renderbuffer: Cell<GLuint>,
    /// The number of MSAA samples.
    samples: Cell<i32>,
    retained_backing: Cell<bool>,
    /// The width and height of the drawable in pixels (as opposed to points.)
    backing_width: Cell<GLint>,
    backing_height: Cell<GLint>,
}

impl ViewState {
    /// `context`, retained (nil once it's gone or going).
    fn context(&self) -> StrongContext {
        // SAFETY: the weak location, initialized by objc_storeWeak().
        StrongContext(unsafe { objc_loadWeakRetained(self.context.get()) })
    }
}

/// A strong reference to the view's context, for as long as it's used.
struct StrongContext(Id);

impl StrongContext {
    fn get(&self) -> Id {
        self.0
    }
}

impl Drop for StrongContext {
    fn drop(&mut self) {
        // SAFETY: the reference objc_loadWeakRetained() gave.
        unsafe { release(self.0) };
    }
}

static SDL_UIKITOPENGLVIEW: DeclaredClass = DeclaredClass::new();

/// The `SDL_uikitopenglview` class (`@interface SDL_uikitopenglview :
/// SDL_uikitview`).
fn uikitopenglview_class() -> Class {
    SDL_UIKITOPENGLVIEW.get(|| {
        let mut b = ClassBuilder::new(c"SDL_uikitopenglview", uikitview_class())
            .expect("SDL_uikitopenglview");
        b.add_ivar::<*mut c_void>(c"_state", c"^v");
        // SAFETY: the implementations have the signatures of the encodings.
        unsafe {
            b.add_class_method(
                sel!(layerClass),
                imp!(glview_layer_class as extern "C" fn(Id, Sel) -> Class),
                c"#@:",
            );
            b.add_method(
                sel!(updateFrame),
                imp!(glview_update_frame as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(swapBuffers),
                imp!(glview_swap_buffers as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(layoutSubviews),
                imp!(glview_layout_subviews as extern "C" fn(Id, Sel)),
                c"v@:",
            );
            b.add_method(
                sel!(dealloc),
                imp!(glview_dealloc as extern "C" fn(Id, Sel)),
                c"v@:",
            );
        }
        b.register()
    })
}

/// The `_state` variable of a view.
fn state_ivar(view: Id) -> *mut *mut c_void {
    // SAFETY: an SDL_uikitopenglview, which has the variable.
    unsafe { ivar_ptr::<*mut c_void>(view, uikitopenglview_class(), c"_state") }
}

/// The instance variables of a view (`None` before they're set).
fn state<'a>(view: Id) -> Option<&'a ViewState> {
    // SAFETY: `_state` is null or the state init_gl_view() leaked, freed
    // by dealloc (main thread only).
    unsafe { (*state_ivar(view) as *const ViewState).as_ref() }
}

/// Translation of `+[SDL_uikitopenglview layerClass]`.
extern "C" fn glview_layer_class(_this: Id, _sel: Sel) -> Class {
    class!(CAEAGLLayer)
}

/// `[EAGLContext setCurrentContext:context]`.
pub(crate) fn set_current_context(context: Id) -> bool {
    // SAFETY: EAGLContext's class method (nil: none).
    let ok: Bool = unsafe { msg_send![class!(EAGLContext), setCurrentContext: context] };
    ok.is_true()
}

/// `[EAGLContext currentContext]`.
pub(crate) fn current_context() -> Id {
    // SAFETY: EAGLContext's class method.
    unsafe { msg_send![class!(EAGLContext), currentContext] }
}

/// The parameters of `-[SDL_uikitopenglview
/// initWithFrame:scale:retainBacking:rBits:gBits:bBits:aBits:depthBits:stencilBits:sRGB:multisamples:context:]`.
pub(crate) struct GlViewConfig {
    pub(crate) frame: CGRect,
    pub(crate) scale: CGFloat,
    pub(crate) retain_backing: bool,
    pub(crate) r_bits: i32,
    pub(crate) g_bits: i32,
    pub(crate) b_bits: i32,
    pub(crate) a_bits: i32,
    pub(crate) depth_bits: i32,
    pub(crate) stencil_bits: i32,
    pub(crate) srgb: i32,
    pub(crate) multisamples: i32,
}

/// A new view (owned), or the error that kept its framebuffer from being
/// made. Translation of `[[SDL_uikitopenglview alloc]
/// initWithFrame:...context:]`.
pub(crate) fn new_gl_view(config: &GlViewConfig, glcontext: Id) -> Result<StrongPtr> {
    // SAFETY: the class's alloc and SDL_uikitview's initializer (owned).
    unsafe {
        let obj: Id = msg_send![uikitopenglview_class(), alloc];
        let this: Id = msg_send_super![obj, uikitopenglview_class(), initWithFrame: config.frame];
        if this.is_null() {
            return Err(Error::out_of_memory());
        }
        let this = StrongPtr::from_owned(this);
        // (`return nil;` from the initializer: ARC releases the object)
        init_gl_view(this.get(), config, glcontext)?;
        Ok(this)
    }
}

/// The body of the initializer, on an initialized `SDL_uikitview`.
///
/// # Safety
///
/// `this` must be a new `SDL_uikitopenglview`.
unsafe fn init_gl_view(this: Id, config: &GlViewConfig, glcontext: Id) -> Result<()> {
    let use_stencil_buffer = config.stencil_bits != 0;
    let use_depth_buffer = config.depth_bits != 0;

    let new_state = Box::new(ViewState {
        context: std::cell::UnsafeCell::new(NIL),
        view_renderbuffer: Cell::new(0),
        view_framebuffer: Cell::new(0),
        depth_renderbuffer: Cell::new(0),
        color_buffer_format: Cell::new(0),
        depth_buffer_format: Cell::new(0),
        msaa_framebuffer: Cell::new(0),
        msaa_renderbuffer: Cell::new(0),
        samples: Cell::new(config.multisamples),
        retained_backing: Cell::new(config.retain_backing),
        backing_width: Cell::new(0),
        backing_height: Cell::new(0),
    });
    // SAFETY: the weak location in the leaked (stable) state; the view's
    // variable, which dealloc frees.
    unsafe {
        objc_storeWeak(new_state.context.get(), glcontext);
        *state_ivar(this) = Box::into_raw(new_state) as *mut c_void;
    }
    let s = state(this).expect("state");
    let context = glcontext;

    if context.is_null() || !set_current_context(context) {
        return Err(Error::new(
            "Could not create OpenGL ES drawable (could not make context current)",
        ));
    }

    // SAFETY: OpenGL ES calls with the context current, and the
    // properties and methods of the view, its CAEAGLLayer and the
    // EAGLContext.
    unsafe {
        if s.samples.get() > 0 {
            let mut maxsamples: GLint = 0;
            glGetIntegerv(GL_MAX_SAMPLES, &mut maxsamples);

            // Clamp the samples to the max supported count.
            s.samples.set(s.samples.get().min(maxsamples));
        }

        let color_format = if config.srgb > 0 {
            s.color_buffer_format.set(GL_SRGB8_ALPHA8);
            kEAGLColorFormatSRGBA8
        } else if config.r_bits >= 8
            || config.g_bits >= 8
            || config.b_bits >= 8
            || config.a_bits > 0
        {
            // if user specifically requests rbg888 or some color format higher than 16bpp
            s.color_buffer_format.set(GL_RGBA8);
            kEAGLColorFormatRGBA8
        } else {
            // default case (potentially faster)
            s.color_buffer_format.set(GL_RGB565);
            kEAGLColorFormatRGB565
        };

        let eagl_layer: Id = msg_send![this, layer];

        let () = msg_send![eagl_layer, setOpaque: YES];
        let keys = [
            kEAGLDrawablePropertyRetainedBacking,
            kEAGLDrawablePropertyColorFormat,
        ];
        let objects = [ns_number_bool(config.retain_backing), color_format];
        let properties: Id = msg_send![class!(NSDictionary),
            dictionaryWithObjects: objects.as_ptr()
            forKeys: keys.as_ptr()
            count: 2 as NSUInteger];
        let () = msg_send![eagl_layer, setDrawableProperties: properties];

        // Set the appropriate scale (for retina display support)
        let () = msg_send![this, setContentScaleFactor: config.scale];

        // Create the color Renderbuffer Object
        let mut view_renderbuffer: GLuint = 0;
        glGenRenderbuffers(1, &mut view_renderbuffer);
        s.view_renderbuffer.set(view_renderbuffer);
        glBindRenderbuffer(GL_RENDERBUFFER, view_renderbuffer);

        let stored: Bool = msg_send![context,
            renderbufferStorage: GL_RENDERBUFFER as NSUInteger
            fromDrawable: eagl_layer];
        if !stored.is_true() {
            return Err(Error::new("Failed to create OpenGL ES drawable"));
        }

        // Create the Framebuffer Object
        let mut view_framebuffer: GLuint = 0;
        glGenFramebuffers(1, &mut view_framebuffer);
        s.view_framebuffer.set(view_framebuffer);
        glBindFramebuffer(GL_FRAMEBUFFER, view_framebuffer);

        // attach the color renderbuffer to the FBO
        glFramebufferRenderbuffer(
            GL_FRAMEBUFFER,
            GL_COLOR_ATTACHMENT0,
            GL_RENDERBUFFER,
            view_renderbuffer,
        );

        let (mut width, mut height): (GLint, GLint) = (0, 0);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_WIDTH, &mut width);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_HEIGHT, &mut height);
        s.backing_width.set(width);
        s.backing_height.set(height);

        if glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE {
            return Err(Error::new("Failed creating OpenGL ES framebuffer"));
        }

        /* When MSAA is used we'll use a separate framebuffer for rendering to,
         * since we'll need to do an explicit MSAA resolve before presenting. */
        if s.samples.get() > 0 {
            let mut msaa_framebuffer: GLuint = 0;
            glGenFramebuffers(1, &mut msaa_framebuffer);
            s.msaa_framebuffer.set(msaa_framebuffer);
            glBindFramebuffer(GL_FRAMEBUFFER, msaa_framebuffer);

            let mut msaa_renderbuffer: GLuint = 0;
            glGenRenderbuffers(1, &mut msaa_renderbuffer);
            s.msaa_renderbuffer.set(msaa_renderbuffer);
            glBindRenderbuffer(GL_RENDERBUFFER, msaa_renderbuffer);

            glRenderbufferStorageMultisample(
                GL_RENDERBUFFER,
                s.samples.get(),
                s.color_buffer_format.get(),
                width,
                height,
            );

            glFramebufferRenderbuffer(
                GL_FRAMEBUFFER,
                GL_COLOR_ATTACHMENT0,
                GL_RENDERBUFFER,
                msaa_renderbuffer,
            );
        }

        if use_depth_buffer || use_stencil_buffer {
            if use_stencil_buffer {
                // Apparently you need to pack stencil and depth into one buffer.
                s.depth_buffer_format.set(GL_DEPTH24_STENCIL8_OES);
            } else if use_depth_buffer {
                /* iOS only uses 32-bit float (exposed as fixed point 24-bit)
                 * depth buffers. */
                s.depth_buffer_format.set(GL_DEPTH_COMPONENT24_OES);
            }

            let mut depth_renderbuffer: GLuint = 0;
            glGenRenderbuffers(1, &mut depth_renderbuffer);
            s.depth_renderbuffer.set(depth_renderbuffer);
            glBindRenderbuffer(GL_RENDERBUFFER, depth_renderbuffer);

            if s.samples.get() > 0 {
                glRenderbufferStorageMultisample(
                    GL_RENDERBUFFER,
                    s.samples.get(),
                    s.depth_buffer_format.get(),
                    width,
                    height,
                );
            } else {
                glRenderbufferStorage(GL_RENDERBUFFER, s.depth_buffer_format.get(), width, height);
            }

            if use_depth_buffer {
                glFramebufferRenderbuffer(
                    GL_FRAMEBUFFER,
                    GL_DEPTH_ATTACHMENT,
                    GL_RENDERBUFFER,
                    depth_renderbuffer,
                );
            }
            if use_stencil_buffer {
                glFramebufferRenderbuffer(
                    GL_FRAMEBUFFER,
                    GL_STENCIL_ATTACHMENT,
                    GL_RENDERBUFFER,
                    depth_renderbuffer,
                );
            }
        }

        if glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE {
            return Err(Error::new("Failed creating OpenGL ES framebuffer"));
        }

        glBindRenderbuffer(GL_RENDERBUFFER, view_renderbuffer);

        set_debug_labels(s);
    }

    Ok(())
}

/// The renderbuffer of the view's drawable. Translation of
/// `-[SDL_uikitopenglview drawableRenderbuffer]`.
pub(crate) fn drawable_renderbuffer(view: Id) -> GLuint {
    state(view).map_or(0, |s| s.view_renderbuffer.get())
}

/// The framebuffer to draw to. Translation of `-[SDL_uikitopenglview
/// drawableFramebuffer]`.
pub(crate) fn drawable_framebuffer(view: Id) -> GLuint {
    let Some(s) = state(view) else { return 0 };
    // When MSAA is used, the MSAA draw framebuffer is used for drawing.
    if s.msaa_framebuffer.get() != 0 {
        s.msaa_framebuffer.get()
    } else {
        s.view_framebuffer.get()
    }
}

/// The framebuffer multisampling resolves into (0 without it).
/// Translation of `-[SDL_uikitopenglview msaaResolveFramebuffer]`.
pub(crate) fn msaa_resolve_framebuffer(view: Id) -> GLuint {
    let Some(s) = state(view) else { return 0 };
    /* When MSAA is used, the MSAA draw framebuffer is used for drawing and the
     * view framebuffer is used as a MSAA resolve framebuffer. */
    if s.msaa_framebuffer.get() != 0 {
        s.view_framebuffer.get()
    } else {
        0
    }
}

/// Translation of `-[SDL_uikitopenglview updateFrame]`: the renderbuffers'
/// storage for the layer's size.
extern "C" fn glview_update_frame(this: Id, _sel: Sel) {
    let Some(s) = state(this) else { return };
    let context = s.context();
    // SAFETY: OpenGL ES calls with the view's context current (its
    // callers' contract); the context's and the view's methods.
    unsafe {
        let mut prev_renderbuffer: GLint = 0;
        glGetIntegerv(GL_RENDERBUFFER_BINDING, &mut prev_renderbuffer);

        glBindRenderbuffer(GL_RENDERBUFFER, s.view_renderbuffer.get());
        let layer: Id = msg_send![this, layer];
        // (a message to nil, if the context is gone)
        let _: Bool = msg_send![context.get(),
            renderbufferStorage: GL_RENDERBUFFER as NSUInteger
            fromDrawable: layer];

        let (mut width, mut height): (GLint, GLint) = (0, 0);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_WIDTH, &mut width);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_HEIGHT, &mut height);
        s.backing_width.set(width);
        s.backing_height.set(height);

        if s.msaa_renderbuffer.get() != 0 {
            glBindRenderbuffer(GL_RENDERBUFFER, s.msaa_renderbuffer.get());
            glRenderbufferStorageMultisample(
                GL_RENDERBUFFER,
                s.samples.get(),
                s.color_buffer_format.get(),
                width,
                height,
            );
        }

        if s.depth_renderbuffer.get() != 0 {
            glBindRenderbuffer(GL_RENDERBUFFER, s.depth_renderbuffer.get());

            if s.samples.get() > 0 {
                glRenderbufferStorageMultisample(
                    GL_RENDERBUFFER,
                    s.samples.get(),
                    s.depth_buffer_format.get(),
                    width,
                    height,
                );
            } else {
                glRenderbufferStorage(GL_RENDERBUFFER, s.depth_buffer_format.get(), width, height);
            }
        }

        glBindRenderbuffer(GL_RENDERBUFFER, prev_renderbuffer as GLuint);
    }
}

/// Translation of `-[SDL_uikitopenglview setDebugLabels]`.
///
/// # Safety
///
/// The view's context must be current.
unsafe fn set_debug_labels(s: &ViewState) {
    // SAFETY: the caller's contract; NUL-terminated labels (length 0).
    unsafe {
        if s.view_framebuffer.get() != 0 {
            glLabelObjectEXT(
                GL_FRAMEBUFFER,
                s.view_framebuffer.get(),
                0,
                c"context FBO".as_ptr(),
            );
        }

        if s.view_renderbuffer.get() != 0 {
            glLabelObjectEXT(
                GL_RENDERBUFFER,
                s.view_renderbuffer.get(),
                0,
                c"context color buffer".as_ptr(),
            );
        }

        if s.depth_renderbuffer.get() != 0 {
            if s.depth_buffer_format.get() == GL_DEPTH24_STENCIL8_OES {
                glLabelObjectEXT(
                    GL_RENDERBUFFER,
                    s.depth_renderbuffer.get(),
                    0,
                    c"context depth-stencil buffer".as_ptr(),
                );
            } else {
                glLabelObjectEXT(
                    GL_RENDERBUFFER,
                    s.depth_renderbuffer.get(),
                    0,
                    c"context depth buffer".as_ptr(),
                );
            }
        }

        if s.msaa_framebuffer.get() != 0 {
            glLabelObjectEXT(
                GL_FRAMEBUFFER,
                s.msaa_framebuffer.get(),
                0,
                c"context MSAA FBO".as_ptr(),
            );
        }

        if s.msaa_renderbuffer.get() != 0 {
            glLabelObjectEXT(
                GL_RENDERBUFFER,
                s.msaa_renderbuffer.get(),
                0,
                c"context MSAA renderbuffer".as_ptr(),
            );
        }
    }
}

/// Translation of `-[SDL_uikitopenglview swapBuffers]`: resolve the
/// multisampled framebuffer (if any) and show the drawable.
extern "C" fn glview_swap_buffers(this: Id, _sel: Sel) {
    let Some(s) = state(this) else { return };
    let context = s.context();
    // SAFETY: OpenGL ES calls with the view's context current (its
    // caller's contract); the context's methods.
    unsafe {
        if s.msaa_framebuffer.get() != 0 {
            let attachments: [GLenum; 1] = [GL_COLOR_ATTACHMENT0];

            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, s.view_framebuffer.get());

            /* OpenGL ES 3+ provides explicit MSAA resolves via glBlitFramebuffer.
             * In OpenGL ES 1 and 2, MSAA resolves must be done via an extension. */
            // (a message to nil, if the context is gone: 0)
            let api: EAGLRenderingAPI = msg_send![context.get(), API];
            if api >= kEAGLRenderingAPIOpenGLES3 {
                let w = s.backing_width.get();
                let h = s.backing_height.get();
                glBlitFramebuffer(0, 0, w, h, 0, 0, w, h, GL_COLOR_BUFFER_BIT, GL_NEAREST);

                if !s.retained_backing.get() {
                    // Discard the contents of the MSAA drawable color buffer.
                    glInvalidateFramebuffer(GL_READ_FRAMEBUFFER, 1, attachments.as_ptr());
                }
            } else {
                glResolveMultisampleFramebufferAPPLE();

                if !s.retained_backing.get() {
                    glDiscardFramebufferEXT(GL_READ_FRAMEBUFFER, 1, attachments.as_ptr());
                }
            }

            /* We assume the "drawable framebuffer" (MSAA draw framebuffer) was
             * previously bound... */
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, s.msaa_framebuffer.get());
        }

        /* viewRenderbuffer should always be bound here. Code that binds something
         * else is responsible for rebinding viewRenderbuffer, to reduce duplicate
         * state changes. */
        let _: Bool = msg_send![context.get(), presentRenderbuffer: GL_RENDERBUFFER as NSUInteger];
    }
}

/// Translation of `-[SDL_uikitopenglview layoutSubviews]`: the
/// renderbuffers follow the view's size.
extern "C" fn glview_layout_subviews(this: Id, _sel: Sel) {
    // SAFETY: UIView's method and properties.
    let (bounds, scale): (CGRect, CGFloat) = unsafe {
        let () = msg_send_super![this, uikitopenglview_class(), layoutSubviews];
        (msg_send![this, bounds], msg_send![this, contentScaleFactor])
    };
    let Some(s) = state(this) else { return };

    let width = (bounds.size.width * scale) as i32;
    let height = (bounds.size.height * scale) as i32;

    // Update the color and depth buffer storage if the layer size has changed.
    if width != s.backing_width.get() || height != s.backing_height.get() {
        let context = s.context();
        let prev_context = current_context();
        if prev_context != context.get() {
            set_current_context(context.get());
        }

        // SAFETY: the view's own method.
        unsafe {
            let () = msg_send![this, updateFrame];
        }

        if prev_context != context.get() {
            set_current_context(prev_context);
        }
    }
}

/// Translation of `-[SDL_uikitopenglview destroyFramebuffer]`.
///
/// # Safety
///
/// The view's context must be current.
unsafe fn destroy_framebuffer(s: &ViewState) {
    let delete_framebuffer = |name: &Cell<GLuint>| {
        if name.get() != 0 {
            // SAFETY: the caller's contract; the view's own object.
            unsafe { glDeleteFramebuffers(1, &name.get()) };
            name.set(0);
        }
    };
    let delete_renderbuffer = |name: &Cell<GLuint>| {
        if name.get() != 0 {
            // SAFETY: as above.
            unsafe { glDeleteRenderbuffers(1, &name.get()) };
            name.set(0);
        }
    };

    delete_framebuffer(&s.view_framebuffer);
    delete_renderbuffer(&s.view_renderbuffer);
    delete_renderbuffer(&s.depth_renderbuffer);
    delete_framebuffer(&s.msaa_framebuffer);
    delete_renderbuffer(&s.msaa_renderbuffer);
}

/// Translation of `-[SDL_uikitopenglview dealloc]`, with what ARC adds:
/// the instance variables freed (the weak reference destroyed), then
/// the superclass's.
extern "C" fn glview_dealloc(this: Id, _sel: Sel) {
    if let Some(s) = state(this) {
        let context = s.context();
        if !context.get().is_null() && context.get() == current_context() {
            // SAFETY: the context is current.
            unsafe { destroy_framebuffer(s) };
            set_current_context(NIL);
        }
    }
    // SAFETY: the state init_gl_view() leaked, freed once; then the
    // superclass's dealloc.
    unsafe {
        let p = state_ivar(this);
        if !(*p).is_null() {
            let state = Box::from_raw(*p as *mut ViewState);
            *p = std::ptr::null_mut();
            objc_destroyWeak(state.context.get());
            drop(state);
        }
        let () = msg_send_super![this, uikitopenglview_class(), dealloc];
    }
}
