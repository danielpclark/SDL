// Rust translation of src/render/opengles/SDL_glesfuncs.h from Simple
// DirectMedia Layer, with the OpenGL ES 1.1 types and constants the
// renderer uses (from SDL_opengles.h's OpenGLES/ES1/gl.h and glext.h).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The OpenGL ES 1.1 entry points of the renderer, looked up at run time
//! with [`gl_get_proc_address`](crate::video::gl::gl_get_proc_address)
//! (on UIKit and Android too, where upstream links them: see
//! `GlesFuncs::load`).
//!
//! The functions are wrapped as methods. The wrappers rely on what upstream
//! relies on for every GL call: the renderer's context is current
//! (`GLES_ActivateRenderer()`). Calls that read memory the caller
//! described with raw pointers (client-side vertex arrays) stay `unsafe`;
//! the ones taking buffers take slices and check their sizes. The
//! `SDL_PROC_OES` functions (the extensions') may be missing, and are
//! `Option`s.

use std::ffi::c_void;

use crate::error::{Error, Result};
use crate::video::gl;

pub(super) type GLenum = u32;
pub(super) type GLbitfield = u32;
pub(super) type GLint = i32;
pub(super) type GLsizei = i32;
pub(super) type GLuint = u32;
pub(super) type GLfloat = f32;
pub(super) type GLclampf = f32;

pub(super) const GL_POINTS: GLenum = 0x0000;
pub(super) const GL_LINE_STRIP: GLenum = 0x0003;
pub(super) const GL_TRIANGLES: GLenum = 0x0004;

pub(super) const GL_ZERO: GLenum = 0;
pub(super) const GL_ONE: GLenum = 1;
pub(super) const GL_SRC_COLOR: GLenum = 0x0300;
pub(super) const GL_ONE_MINUS_SRC_COLOR: GLenum = 0x0301;
pub(super) const GL_SRC_ALPHA: GLenum = 0x0302;
pub(super) const GL_ONE_MINUS_SRC_ALPHA: GLenum = 0x0303;
pub(super) const GL_DST_ALPHA: GLenum = 0x0304;
pub(super) const GL_ONE_MINUS_DST_ALPHA: GLenum = 0x0305;
pub(super) const GL_DST_COLOR: GLenum = 0x0306;
pub(super) const GL_ONE_MINUS_DST_COLOR: GLenum = 0x0307;
// (GL_OES_blend_subtract, GL_EXT_blend_minmax)
pub(super) const GL_FUNC_ADD_OES: GLenum = 0x8006;
pub(super) const GL_MIN_EXT: GLenum = 0x8007;
pub(super) const GL_MAX_EXT: GLenum = 0x8008;
pub(super) const GL_FUNC_SUBTRACT_OES: GLenum = 0x800A;
pub(super) const GL_FUNC_REVERSE_SUBTRACT_OES: GLenum = 0x800B;

pub(super) const GL_NO_ERROR: GLenum = 0;
pub(super) const GL_INVALID_ENUM: GLenum = 0x0500;
pub(super) const GL_INVALID_VALUE: GLenum = 0x0501;
pub(super) const GL_INVALID_OPERATION: GLenum = 0x0502;
pub(super) const GL_STACK_OVERFLOW: GLenum = 0x0503;
pub(super) const GL_STACK_UNDERFLOW: GLenum = 0x0504;
pub(super) const GL_OUT_OF_MEMORY: GLenum = 0x0505;

pub(super) const GL_CULL_FACE: GLenum = 0x0B44;
pub(super) const GL_DEPTH_TEST: GLenum = 0x0B71;
pub(super) const GL_BLEND: GLenum = 0x0BE2;
pub(super) const GL_SCISSOR_TEST: GLenum = 0x0C11;
pub(super) const GL_UNPACK_ALIGNMENT: GLenum = 0x0CF5;
pub(super) const GL_PACK_ALIGNMENT: GLenum = 0x0D05;
pub(super) const GL_MAX_TEXTURE_SIZE: GLenum = 0x0D33;
pub(super) const GL_TEXTURE_2D: GLenum = 0x0DE1;

pub(super) const GL_UNSIGNED_BYTE: GLenum = 0x1401;
pub(super) const GL_FLOAT: GLenum = 0x1406;
pub(super) const GL_MODELVIEW: GLenum = 0x1700;
pub(super) const GL_PROJECTION: GLenum = 0x1701;
pub(super) const GL_RGBA: GLenum = 0x1908;

pub(super) const GL_NEAREST: GLint = 0x2600;
pub(super) const GL_LINEAR: GLint = 0x2601;
pub(super) const GL_TEXTURE_MAG_FILTER: GLenum = 0x2800;
pub(super) const GL_TEXTURE_MIN_FILTER: GLenum = 0x2801;
pub(super) const GL_TEXTURE_WRAP_S: GLenum = 0x2802;
pub(super) const GL_TEXTURE_WRAP_T: GLenum = 0x2803;
pub(super) const GL_REPEAT: GLint = 0x2901;
pub(super) const GL_CLAMP_TO_EDGE: GLint = 0x812F;

pub(super) const GL_COLOR_BUFFER_BIT: GLbitfield = 0x4000;

pub(super) const GL_VERTEX_ARRAY: GLenum = 0x8074;
pub(super) const GL_COLOR_ARRAY: GLenum = 0x8076;
pub(super) const GL_TEXTURE_COORD_ARRAY: GLenum = 0x8078;

// (GL_OES_framebuffer_object)
pub(super) const GL_FRAMEBUFFER_BINDING_OES: GLenum = 0x8CA6;
pub(super) const GL_FRAMEBUFFER_COMPLETE_OES: GLenum = 0x8CD5;
pub(super) const GL_COLOR_ATTACHMENT0_OES: GLenum = 0x8CE0;
pub(super) const GL_FRAMEBUFFER_OES: GLenum = 0x8D40;

/// Declares the function table (`SDL_PROC(ret, func, params)` and
/// `SDL_PROC_OES(ret, func, params)` for each line of `SDL_glesfuncs.h`)
/// and its loader.
macro_rules! gles_funcs {
    (
        required {
            $($field:ident = $func:ident($($arg:ty),*) $(-> $ret:ty)?;)*
        }
        oes {
            $($oes_field:ident = $oes_func:ident($($oes_arg:ty),*) $(-> $oes_ret:ty)?;)*
        }
    ) => {
        /// The GLES functions of a renderer (the `SDL_PROC` and
        /// `SDL_PROC_OES` members of `GLES_RenderData`).
        #[allow(dead_code)] // (loaded as upstream does, the ones the renderer never calls too)
        pub(super) struct GlesFuncs {
            $($field: unsafe extern "system" fn($($arg),*) $(-> $ret)?,)*
            $(pub(super) $oes_field: Option<unsafe extern "system" fn($($oes_arg),*) $(-> $oes_ret)?>,)*
        }

        impl GlesFuncs {
            /// Look the functions up in the current GL library (a missing
            /// `SDL_PROC_OES` one is `None`). Translation of
            /// `GLES_LoadFunctions()`.
            ///
            /// Note: on UIKit and Android upstream defines
            /// `__SDL_NOGETPROCADDR__`, and the renderer takes the
            /// functions' addresses from the OpenGL ES library it's linked
            /// with (so there the `SDL_PROC_OES` ones are never missing).
            /// Here they're looked up through `SDL_GL_GetProcAddress()`
            /// there too: on UIKit `dlsym(RTLD_DEFAULT, ...)` over the
            /// loaded images, among them the OpenGLES framework the UIKit
            /// driver links, and on Android the EGL driver's lookup
            /// (`eglGetProcAddress()`, or `libGLESv1_CM.so`'s exports,
            /// which have the extensions' functions too): the same
            /// functions (on the emulator, the same addresses), though a
            /// missing one is an error (or `None`) here rather than a link
            /// failure.
            pub(super) fn load() -> Result<GlesFuncs> {
                Ok(GlesFuncs {
                    $($field: {
                        let name = stringify!($func);
                        let Some(p) = gl::gl_get_proc_address(name) else {
                            return Err(Error::new(format!(
                                "Couldn't load GLES function {name}: not found"
                            )));
                        };
                        // SAFETY: the GL entry point has this signature
                        // (SDL_glesfuncs.h); `p` is not null.
                        unsafe {
                            std::mem::transmute::<
                                *const c_void,
                                unsafe extern "system" fn($($arg),*) $(-> $ret)?,
                            >(p)
                        }
                    },)*
                    $($oes_field: gl::gl_get_proc_address(stringify!($oes_func)).map(|p| {
                        // SAFETY: as above.
                        unsafe {
                            std::mem::transmute::<
                                *const c_void,
                                unsafe extern "system" fn($($oes_arg),*) $(-> $oes_ret)?,
                            >(p)
                        }
                    }),)*
                })
            }
        }
    };
}

gles_funcs! {
    required {
    bind_texture_fn = glBindTexture(GLenum, GLuint);
    blend_func_fn = glBlendFunc(GLenum, GLenum);
    clear_fn = glClear(GLbitfield);
    clear_color_fn = glClearColor(GLclampf, GLclampf, GLclampf, GLclampf);
    color4f_fn = glColor4f(GLfloat, GLfloat, GLfloat, GLfloat);
    color_pointer_fn = glColorPointer(GLint, GLenum, GLsizei, *const c_void);
    delete_textures_fn = glDeleteTextures(GLsizei, *const GLuint);
    disable_fn = glDisable(GLenum);
    disable_client_state_fn = glDisableClientState(GLenum);
    draw_arrays_fn = glDrawArrays(GLenum, GLint, GLsizei);
    enable_fn = glEnable(GLenum);
    enable_client_state_fn = glEnableClientState(GLenum);
    finish_fn = glFinish();
    gen_textures_fn = glGenTextures(GLsizei, *mut GLuint);
    get_error_fn = glGetError() -> GLenum;
    get_integerv_fn = glGetIntegerv(GLenum, *mut GLint);
    load_identity_fn = glLoadIdentity();
    matrix_mode_fn = glMatrixMode(GLenum);
    orthof_fn = glOrthof(GLfloat, GLfloat, GLfloat, GLfloat, GLfloat, GLfloat);
    pixel_storei_fn = glPixelStorei(GLenum, GLint);
    read_pixels_fn = glReadPixels(GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, *mut c_void);
    scissor_fn = glScissor(GLint, GLint, GLsizei, GLsizei);
    tex_coord_pointer_fn = glTexCoordPointer(GLint, GLenum, GLsizei, *const c_void);
    tex_envf_fn = glTexEnvf(GLenum, GLenum, GLfloat);
    tex_image_2d_fn = glTexImage2D(
        GLenum, GLint, GLint, GLsizei, GLsizei, GLint, GLenum, GLenum, *const c_void
    );
    tex_parameteri_fn = glTexParameteri(GLenum, GLenum, GLint);
    tex_parameteriv_fn = glTexParameteriv(GLenum, GLenum, *const GLint);
    tex_sub_image_2d_fn = glTexSubImage2D(
        GLenum, GLint, GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, *const c_void
    );
    vertex_pointer_fn = glVertexPointer(GLint, GLenum, GLsizei, *const c_void);
    viewport_fn = glViewport(GLint, GLint, GLsizei, GLsizei);
    }
    oes {
        blend_equation_oes = glBlendEquationOES(GLenum);
        blend_equation_separate_oes = glBlendEquationSeparateOES(GLenum, GLenum);
        blend_func_separate_oes = glBlendFuncSeparateOES(GLenum, GLenum, GLenum, GLenum);
        draw_texf_oes = glDrawTexfOES(GLfloat, GLfloat, GLfloat, GLfloat, GLfloat);
        gen_framebuffers_oes = glGenFramebuffersOES(GLsizei, *mut GLuint);
        bind_framebuffer_oes = glBindFramebufferOES(GLenum, GLuint);
        framebuffer_texture_2d_oes = glFramebufferTexture2DOES(GLenum, GLenum, GLenum, GLuint, GLint);
        check_framebuffer_status_oes = glCheckFramebufferStatusOES(GLenum) -> GLenum;
        delete_framebuffers_oes = glDeleteFramebuffersOES(GLsizei, *const GLuint);
    }
}

/// The bytes of one pixel of the `format`/`type` pairs the renderer uploads
/// or reads (`GL_RGBA`, `GL_UNSIGNED_BYTE`).
fn pixel_bytes(format: GLenum, ty: GLenum) -> usize {
    match (format, ty) {
        (GL_RGBA, GL_UNSIGNED_BYTE) => 4,
        _ => 1,
    }
}

// SAFETY (for the calls below): the renderer's context is current, and
// every pointer passed points to memory of the size GL reads or writes
// (checked against the slices where GL takes a buffer; the unpack and pack
// alignments are 1). The `SDL_PROC_OES` wrappers do nothing (and return
// 0) when the function is missing, where upstream would call through a
// null pointer (only once the extension is known to be there, or its
// generating function found).
impl GlesFuncs {
    pub(super) fn bind_texture(&self, target: GLenum, texture: GLuint) {
        unsafe { (self.bind_texture_fn)(target, texture) }
    }
    pub(super) fn blend_func(&self, sfactor: GLenum, dfactor: GLenum) {
        unsafe { (self.blend_func_fn)(sfactor, dfactor) }
    }
    pub(super) fn blend_equation_oes(&self, mode: GLenum) {
        if let Some(f) = self.blend_equation_oes {
            unsafe { f(mode) }
        }
    }
    pub(super) fn blend_equation_separate_oes(&self, mode_rgb: GLenum, mode_alpha: GLenum) {
        if let Some(f) = self.blend_equation_separate_oes {
            unsafe { f(mode_rgb, mode_alpha) }
        }
    }
    pub(super) fn blend_func_separate_oes(
        &self,
        src: GLenum,
        dst: GLenum,
        src_a: GLenum,
        dst_a: GLenum,
    ) {
        if let Some(f) = self.blend_func_separate_oes {
            unsafe { f(src, dst, src_a, dst_a) }
        }
    }
    pub(super) fn clear(&self, mask: GLbitfield) {
        unsafe { (self.clear_fn)(mask) }
    }
    pub(super) fn clear_color(&self, r: GLclampf, g: GLclampf, b: GLclampf, a: GLclampf) {
        unsafe { (self.clear_color_fn)(r, g, b, a) }
    }
    pub(super) fn color4f(&self, r: GLfloat, g: GLfloat, b: GLfloat, a: GLfloat) {
        unsafe { (self.color4f_fn)(r, g, b, a) }
    }
    /// # Safety
    ///
    /// `pointer` must stay valid (for the vertices drawn) until the draw
    /// calls using it.
    pub(super) unsafe fn color_pointer(&self, size: GLint, stride: GLsizei, pointer: *const f32) {
        unsafe { (self.color_pointer_fn)(size, GL_FLOAT, stride, pointer.cast()) }
    }
    pub(super) fn delete_texture(&self, texture: GLuint) {
        unsafe { (self.delete_textures_fn)(1, &texture) }
    }
    pub(super) fn disable(&self, cap: GLenum) {
        unsafe { (self.disable_fn)(cap) }
    }
    pub(super) fn disable_client_state(&self, array: GLenum) {
        unsafe { (self.disable_client_state_fn)(array) }
    }
    /// # Safety
    ///
    /// The enabled vertex arrays must point to at least `first + count`
    /// vertices of live memory.
    pub(super) unsafe fn draw_arrays(&self, mode: GLenum, first: GLint, count: GLsizei) {
        unsafe { (self.draw_arrays_fn)(mode, first, count) }
    }
    pub(super) fn enable(&self, cap: GLenum) {
        unsafe { (self.enable_fn)(cap) }
    }
    pub(super) fn enable_client_state(&self, array: GLenum) {
        unsafe { (self.enable_client_state_fn)(array) }
    }
    pub(super) fn finish(&self) {
        unsafe { (self.finish_fn)() }
    }
    pub(super) fn gen_texture(&self) -> GLuint {
        let mut texture = 0;
        unsafe { (self.gen_textures_fn)(1, &mut texture) };
        texture
    }
    pub(super) fn get_error(&self) -> GLenum {
        unsafe { (self.get_error_fn)() }
    }
    pub(super) fn get_integer(&self, pname: GLenum) -> GLint {
        let mut value = 0;
        unsafe { (self.get_integerv_fn)(pname, &mut value) };
        value
    }
    pub(super) fn load_identity(&self) {
        unsafe { (self.load_identity_fn)() }
    }
    pub(super) fn matrix_mode(&self, mode: GLenum) {
        unsafe { (self.matrix_mode_fn)(mode) }
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn orthof(
        &self,
        left: GLfloat,
        right: GLfloat,
        bottom: GLfloat,
        top: GLfloat,
        near: GLfloat,
        far: GLfloat,
    ) {
        unsafe { (self.orthof_fn)(left, right, bottom, top, near, far) }
    }
    pub(super) fn pixel_storei(&self, pname: GLenum, param: GLint) {
        unsafe { (self.pixel_storei_fn)(pname, param) }
    }
    /// Read a `w`x`h` block into `pixels` (tightly packed).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn read_pixels(
        &self,
        x: GLint,
        y: GLint,
        w: GLsizei,
        h: GLsizei,
        format: GLenum,
        ty: GLenum,
        pixels: &mut [u8],
    ) {
        let needed = w.max(0) as usize * h.max(0) as usize * pixel_bytes(format, ty);
        assert!(pixels.len() >= needed, "glReadPixels buffer too small");
        unsafe { (self.read_pixels_fn)(x, y, w, h, format, ty, pixels.as_mut_ptr().cast()) }
    }
    pub(super) fn scissor(&self, x: GLint, y: GLint, w: GLsizei, h: GLsizei) {
        unsafe { (self.scissor_fn)(x, y, w, h) }
    }
    /// # Safety
    ///
    /// As [`GlesFuncs::color_pointer`].
    pub(super) unsafe fn tex_coord_pointer(
        &self,
        size: GLint,
        stride: GLsizei,
        pointer: *const f32,
    ) {
        unsafe { (self.tex_coord_pointer_fn)(size, GL_FLOAT, stride, pointer.cast()) }
    }
    /// `glTexImage2D()` of level 0 without data (the texture's storage).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn tex_image_2d_empty(
        &self,
        target: GLenum,
        internal_format: GLint,
        w: GLsizei,
        h: GLsizei,
        format: GLenum,
        ty: GLenum,
    ) {
        unsafe {
            (self.tex_image_2d_fn)(
                target,
                0,
                internal_format,
                w,
                h,
                0,
                format,
                ty,
                std::ptr::null(),
            )
        }
    }
    pub(super) fn tex_parameteri(&self, target: GLenum, pname: GLenum, param: GLint) {
        unsafe { (self.tex_parameteri_fn)(target, pname, param) }
    }
    /// `glTexSubImage2D()` of level 0 from tightly packed `pixels`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn tex_sub_image_2d(
        &self,
        target: GLenum,
        x: GLint,
        y: GLint,
        w: GLsizei,
        h: GLsizei,
        format: GLenum,
        ty: GLenum,
        pixels: &[u8],
    ) {
        let needed = w.max(0) as usize * h.max(0) as usize * pixel_bytes(format, ty);
        assert!(pixels.len() >= needed, "glTexSubImage2D buffer too small");
        unsafe {
            (self.tex_sub_image_2d_fn)(target, 0, x, y, w, h, format, ty, pixels.as_ptr().cast())
        }
    }
    /// # Safety
    ///
    /// As [`GlesFuncs::color_pointer`].
    pub(super) unsafe fn vertex_pointer(&self, size: GLint, stride: GLsizei, pointer: *const f32) {
        unsafe { (self.vertex_pointer_fn)(size, GL_FLOAT, stride, pointer.cast()) }
    }
    pub(super) fn viewport(&self, x: GLint, y: GLint, w: GLsizei, h: GLsizei) {
        unsafe { (self.viewport_fn)(x, y, w, h) }
    }
    pub(super) fn gen_framebuffer_oes(&self) -> GLuint {
        let mut fbo = 0;
        if let Some(f) = self.gen_framebuffers_oes {
            unsafe { f(1, &mut fbo) };
        }
        fbo
    }
    pub(super) fn bind_framebuffer_oes(&self, target: GLenum, framebuffer: GLuint) {
        if let Some(f) = self.bind_framebuffer_oes {
            unsafe { f(target, framebuffer) }
        }
    }
    pub(super) fn framebuffer_texture_2d_oes(
        &self,
        target: GLenum,
        attachment: GLenum,
        textarget: GLenum,
        texture: GLuint,
    ) {
        if let Some(f) = self.framebuffer_texture_2d_oes {
            unsafe { f(target, attachment, textarget, texture, 0) }
        }
    }
    pub(super) fn check_framebuffer_status_oes(&self, target: GLenum) -> GLenum {
        match self.check_framebuffer_status_oes {
            Some(f) => unsafe { f(target) },
            None => 0,
        }
    }
    pub(super) fn delete_framebuffer_oes(&self, framebuffer: GLuint) {
        if let Some(f) = self.delete_framebuffers_oes {
            unsafe { f(1, &framebuffer) }
        }
    }
}
