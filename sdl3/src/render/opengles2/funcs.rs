// Rust translation of src/render/opengles2/SDL_gles2funcs.h from Simple
// DirectMedia Layer, with the OpenGL ES 2.0 types and constants the
// renderer uses (from SDL_opengles2_gl2.h and SDL_opengles2_gl2ext.h).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The OpenGL ES 2.0 entry points of the renderer, looked up at run time
//! with [`gl_get_proc_address`](crate::video::gl::gl_get_proc_address).
//!
//! The functions are wrapped as methods. The wrappers rely on what upstream
//! relies on for every GL call: the renderer's context is current
//! (`GLES2_ActivateRenderer()`). Calls that read memory the caller
//! described with raw pointers (client-side vertex arrays) stay `unsafe`;
//! the ones taking buffers take slices and check their sizes.

use std::ffi::{c_char, c_void, CStr};

use crate::error::{Error, Result};
use crate::video::gl;

pub(super) type GLenum = u32;
pub(super) type GLboolean = u8;
pub(super) type GLbitfield = u32;
pub(super) type GLint = i32;
pub(super) type GLsizei = i32;
pub(super) type GLuint = u32;
pub(super) type GLfloat = f32;
pub(super) type GLintptr = isize;
pub(super) type GLsizeiptr = isize;

pub(super) const GL_FALSE: GLboolean = 0;
pub(super) const GL_TRUE: GLboolean = 1;

pub(super) const GL_POINTS: GLenum = 0x0000;
pub(super) const GL_LINES: GLenum = 0x0001;
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
pub(super) const GL_FUNC_ADD: GLenum = 0x8006;
pub(super) const GL_MIN_EXT: GLenum = 0x8007;
pub(super) const GL_MAX_EXT: GLenum = 0x8008;
pub(super) const GL_FUNC_SUBTRACT: GLenum = 0x800A;
pub(super) const GL_FUNC_REVERSE_SUBTRACT: GLenum = 0x800B;

pub(super) const GL_NO_ERROR: GLenum = 0;
pub(super) const GL_INVALID_ENUM: GLenum = 0x0500;
pub(super) const GL_INVALID_VALUE: GLenum = 0x0501;
pub(super) const GL_INVALID_OPERATION: GLenum = 0x0502;
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
pub(super) const GL_RGBA: GLenum = 0x1908;
pub(super) const GL_LUMINANCE: GLenum = 0x1909;
pub(super) const GL_LUMINANCE_ALPHA: GLenum = 0x190A;
pub(super) const GL_NONE: GLenum = 0;

pub(super) const GL_NEAREST: GLint = 0x2600;
pub(super) const GL_LINEAR: GLint = 0x2601;
pub(super) const GL_TEXTURE_MAG_FILTER: GLenum = 0x2800;
pub(super) const GL_TEXTURE_MIN_FILTER: GLenum = 0x2801;
pub(super) const GL_TEXTURE_WRAP_S: GLenum = 0x2802;
pub(super) const GL_TEXTURE_WRAP_T: GLenum = 0x2803;
pub(super) const GL_REPEAT: GLint = 0x2901;
pub(super) const GL_CLAMP_TO_EDGE: GLint = 0x812F;

pub(super) const GL_COLOR_BUFFER_BIT: GLbitfield = 0x4000;

pub(super) const GL_TEXTURE0: GLenum = 0x84C0;
pub(super) const GL_TEXTURE1: GLenum = 0x84C1;
pub(super) const GL_TEXTURE2: GLenum = 0x84C2;

pub(super) const GL_FRAGMENT_SHADER: GLenum = 0x8B30;
pub(super) const GL_VERTEX_SHADER: GLenum = 0x8B31;
pub(super) const GL_COMPILE_STATUS: GLenum = 0x8B81;
pub(super) const GL_LINK_STATUS: GLenum = 0x8B82;
pub(super) const GL_INFO_LOG_LENGTH: GLenum = 0x8B84;

pub(super) const GL_FRAMEBUFFER_BINDING: GLenum = 0x8CA6;
pub(super) const GL_FRAMEBUFFER_COMPLETE: GLenum = 0x8CD5;
pub(super) const GL_COLOR_ATTACHMENT0: GLenum = 0x8CE0;
pub(super) const GL_FRAMEBUFFER: GLenum = 0x8D40;
pub(super) const GL_TEXTURE_EXTERNAL_OES: GLenum = 0x8D65;
/// This is always the same number between the various EXT/ARB/GLES extensions.
pub(super) const GL_FRAMEBUFFER_SRGB: GLenum = 0x8DB9;

/// Declares the function table (`SDL_PROC(ret, func, params)` for each
/// line of `SDL_gles2funcs.h`) and its loader.
macro_rules! gles2_funcs {
    ($($field:ident = $func:ident($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        /// The GLES2 functions of a renderer (the `SDL_PROC` members of
        /// `GLES2_RenderData`).
        #[allow(dead_code)] // (loaded as upstream does; the VBO functions are Emscripten's)
        pub(super) struct Gles2Funcs {
            $($field: unsafe extern "system" fn($($arg),*) $(-> $ret)?,)*
        }

        impl Gles2Funcs {
            /// Look the functions up in the current GL library.
            /// Translation of `GLES2_LoadFunctions()`.
            pub(super) fn load() -> Result<Gles2Funcs> {
                Ok(Gles2Funcs {
                    $($field: {
                        let name = stringify!($func);
                        let Some(p) = gl::gl_get_proc_address(name) else {
                            return Err(Error::new(format!(
                                "Couldn't load GLES2 function {name}: not found"
                            )));
                        };
                        // SAFETY: the GL entry point has this signature
                        // (SDL_gles2funcs.h); `p` is not null.
                        unsafe {
                            std::mem::transmute::<
                                *const c_void,
                                unsafe extern "system" fn($($arg),*) $(-> $ret)?,
                            >(p)
                        }
                    },)*
                })
            }
        }
    };
}

gles2_funcs! {
    active_texture_fn = glActiveTexture(GLenum);
    attach_shader_fn = glAttachShader(GLuint, GLuint);
    bind_attrib_location_fn = glBindAttribLocation(GLuint, GLuint, *const c_char);
    bind_texture_fn = glBindTexture(GLenum, GLuint);
    blend_equation_separate_fn = glBlendEquationSeparate(GLenum, GLenum);
    blend_func_separate_fn = glBlendFuncSeparate(GLenum, GLenum, GLenum, GLenum);
    clear_fn = glClear(GLbitfield);
    clear_color_fn = glClearColor(GLfloat, GLfloat, GLfloat, GLfloat);
    compile_shader_fn = glCompileShader(GLuint);
    create_program_fn = glCreateProgram() -> GLuint;
    create_shader_fn = glCreateShader(GLenum) -> GLuint;
    delete_program_fn = glDeleteProgram(GLuint);
    delete_shader_fn = glDeleteShader(GLuint);
    delete_textures_fn = glDeleteTextures(GLsizei, *const GLuint);
    disable_fn = glDisable(GLenum);
    disable_vertex_attrib_array_fn = glDisableVertexAttribArray(GLuint);
    draw_arrays_fn = glDrawArrays(GLenum, GLint, GLsizei);
    enable_fn = glEnable(GLenum);
    enable_vertex_attrib_array_fn = glEnableVertexAttribArray(GLuint);
    finish_fn = glFinish();
    gen_framebuffers_fn = glGenFramebuffers(GLsizei, *mut GLuint);
    gen_textures_fn = glGenTextures(GLsizei, *mut GLuint);
    get_string_fn = glGetString(GLenum) -> *const u8;
    get_error_fn = glGetError() -> GLenum;
    get_integerv_fn = glGetIntegerv(GLenum, *mut GLint);
    get_programiv_fn = glGetProgramiv(GLuint, GLenum, *mut GLint);
    get_shader_info_log_fn = glGetShaderInfoLog(GLuint, GLsizei, *mut GLsizei, *mut c_char);
    get_shaderiv_fn = glGetShaderiv(GLuint, GLenum, *mut GLint);
    get_uniform_location_fn = glGetUniformLocation(GLuint, *const c_char) -> GLint;
    link_program_fn = glLinkProgram(GLuint);
    pixel_storei_fn = glPixelStorei(GLenum, GLint);
    read_pixels_fn = glReadPixels(GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, *mut c_void);
    scissor_fn = glScissor(GLint, GLint, GLsizei, GLsizei);
    shader_binary_fn = glShaderBinary(GLsizei, *const GLuint, GLenum, *const c_void, GLsizei);
    shader_source_fn = glShaderSource(GLuint, GLsizei, *const *const c_char, *const GLint);
    tex_image_2d_fn = glTexImage2D(
        GLenum, GLint, GLint, GLsizei, GLsizei, GLint, GLenum, GLenum, *const c_void
    );
    tex_parameteri_fn = glTexParameteri(GLenum, GLenum, GLint);
    tex_sub_image_2d_fn = glTexSubImage2D(
        GLenum, GLint, GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, *const c_void
    );
    uniform1i_fn = glUniform1i(GLint, GLint);
    uniform3f_fn = glUniform3f(GLint, GLfloat, GLfloat, GLfloat);
    uniform4f_fn = glUniform4f(GLint, GLfloat, GLfloat, GLfloat, GLfloat);
    uniform_matrix3fv_fn = glUniformMatrix3fv(GLint, GLsizei, GLboolean, *const GLfloat);
    uniform_matrix4fv_fn = glUniformMatrix4fv(GLint, GLsizei, GLboolean, *const GLfloat);
    use_program_fn = glUseProgram(GLuint);
    vertex_attrib_pointer_fn = glVertexAttribPointer(
        GLuint, GLint, GLenum, GLboolean, GLsizei, *const c_void
    );
    viewport_fn = glViewport(GLint, GLint, GLsizei, GLsizei);
    bind_framebuffer_fn = glBindFramebuffer(GLenum, GLuint);
    framebuffer_texture_2d_fn = glFramebufferTexture2D(GLenum, GLenum, GLenum, GLuint, GLint);
    check_framebuffer_status_fn = glCheckFramebufferStatus(GLenum) -> GLenum;
    delete_framebuffers_fn = glDeleteFramebuffers(GLsizei, *const GLuint);
    get_attrib_location_fn = glGetAttribLocation(GLuint, *const c_char) -> GLint;
    get_program_info_log_fn = glGetProgramInfoLog(GLuint, GLsizei, *mut GLsizei, *mut c_char);
    gen_buffers_fn = glGenBuffers(GLsizei, *mut GLuint);
    delete_buffers_fn = glDeleteBuffers(GLsizei, *const GLuint);
    bind_buffer_fn = glBindBuffer(GLenum, GLuint);
    buffer_data_fn = glBufferData(GLenum, GLsizeiptr, *const c_void, GLenum);
    buffer_sub_data_fn = glBufferSubData(GLenum, GLintptr, GLsizeiptr, *const c_void);
}

/// The bytes of one pixel of a `format`/`type` pair the renderer uploads
/// or reads.
fn pixel_bytes(format: GLenum, ty: GLenum) -> usize {
    match (format, ty) {
        (GL_RGBA, GL_UNSIGNED_BYTE) => 4,
        (GL_LUMINANCE_ALPHA, GL_UNSIGNED_BYTE) => 2,
        _ => 1,
    }
}

// SAFETY (for the calls below): the renderer's context is current, and
// every pointer passed points to memory of the size GL reads or writes
// (checked against the slices where GL takes a buffer; the unpack and pack
// alignments are 1).
impl Gles2Funcs {
    pub(super) fn active_texture(&self, texture: GLenum) {
        unsafe { (self.active_texture_fn)(texture) }
    }
    pub(super) fn attach_shader(&self, program: GLuint, shader: GLuint) {
        unsafe { (self.attach_shader_fn)(program, shader) }
    }
    pub(super) fn bind_attrib_location(&self, program: GLuint, index: GLuint, name: &CStr) {
        unsafe { (self.bind_attrib_location_fn)(program, index, name.as_ptr()) }
    }
    pub(super) fn bind_texture(&self, target: GLenum, texture: GLuint) {
        unsafe { (self.bind_texture_fn)(target, texture) }
    }
    pub(super) fn blend_equation_separate(&self, mode_rgb: GLenum, mode_alpha: GLenum) {
        unsafe { (self.blend_equation_separate_fn)(mode_rgb, mode_alpha) }
    }
    pub(super) fn blend_func_separate(
        &self,
        src: GLenum,
        dst: GLenum,
        src_a: GLenum,
        dst_a: GLenum,
    ) {
        unsafe { (self.blend_func_separate_fn)(src, dst, src_a, dst_a) }
    }
    pub(super) fn clear(&self, mask: GLbitfield) {
        unsafe { (self.clear_fn)(mask) }
    }
    pub(super) fn clear_color(&self, r: GLfloat, g: GLfloat, b: GLfloat, a: GLfloat) {
        unsafe { (self.clear_color_fn)(r, g, b, a) }
    }
    pub(super) fn compile_shader(&self, shader: GLuint) {
        unsafe { (self.compile_shader_fn)(shader) }
    }
    pub(super) fn create_program(&self) -> GLuint {
        unsafe { (self.create_program_fn)() }
    }
    pub(super) fn create_shader(&self, ty: GLenum) -> GLuint {
        unsafe { (self.create_shader_fn)(ty) }
    }
    pub(super) fn delete_program(&self, program: GLuint) {
        unsafe { (self.delete_program_fn)(program) }
    }
    pub(super) fn delete_shader(&self, shader: GLuint) {
        unsafe { (self.delete_shader_fn)(shader) }
    }
    pub(super) fn delete_texture(&self, texture: GLuint) {
        unsafe { (self.delete_textures_fn)(1, &texture) }
    }
    pub(super) fn disable(&self, cap: GLenum) {
        unsafe { (self.disable_fn)(cap) }
    }
    pub(super) fn disable_vertex_attrib_array(&self, index: GLuint) {
        unsafe { (self.disable_vertex_attrib_array_fn)(index) }
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
    pub(super) fn enable_vertex_attrib_array(&self, index: GLuint) {
        unsafe { (self.enable_vertex_attrib_array_fn)(index) }
    }
    pub(super) fn finish(&self) {
        unsafe { (self.finish_fn)() }
    }
    pub(super) fn gen_framebuffer(&self) -> GLuint {
        let mut fbo = 0;
        unsafe { (self.gen_framebuffers_fn)(1, &mut fbo) };
        fbo
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
    pub(super) fn get_program(&self, program: GLuint, pname: GLenum) -> GLint {
        let mut value = 0;
        unsafe { (self.get_programiv_fn)(program, pname, &mut value) };
        value
    }
    pub(super) fn get_shader(&self, shader: GLuint, pname: GLenum) -> GLint {
        let mut value = 0;
        unsafe { (self.get_shaderiv_fn)(shader, pname, &mut value) };
        value
    }
    /// The info log of a shader, `length` bytes at most.
    pub(super) fn get_shader_info_log(&self, shader: GLuint, length: GLint) -> String {
        let mut info = vec![0u8; length.max(1) as usize];
        let mut written = 0;
        unsafe {
            (self.get_shader_info_log_fn)(
                shader,
                info.len() as GLsizei,
                &mut written,
                info.as_mut_ptr().cast(),
            )
        };
        info.truncate((written.max(0) as usize).min(info.len()));
        String::from_utf8_lossy(&info).into_owned()
    }
    pub(super) fn get_uniform_location(&self, program: GLuint, name: &CStr) -> GLint {
        unsafe { (self.get_uniform_location_fn)(program, name.as_ptr()) }
    }
    pub(super) fn link_program(&self, program: GLuint) {
        unsafe { (self.link_program_fn)(program) }
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
    /// `glShaderSource()` with the strings and their lengths.
    pub(super) fn shader_source(&self, shader: GLuint, sources: &[&str]) {
        let ptrs: Vec<*const c_char> = sources.iter().map(|s| s.as_ptr().cast()).collect();
        let lens: Vec<GLint> = sources.iter().map(|s| s.len() as GLint).collect();
        unsafe {
            (self.shader_source_fn)(shader, ptrs.len() as GLsizei, ptrs.as_ptr(), lens.as_ptr())
        }
    }
    /// `glTexImage2D()` without data (the texture's storage).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn tex_image_2d_empty(
        &self,
        target: GLenum,
        internal_format: GLenum,
        w: GLsizei,
        h: GLsizei,
        format: GLenum,
        ty: GLenum,
    ) {
        unsafe {
            (self.tex_image_2d_fn)(
                target,
                0,
                internal_format as GLint,
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
    pub(super) fn uniform1i(&self, location: GLint, v0: GLint) {
        unsafe { (self.uniform1i_fn)(location, v0) }
    }
    pub(super) fn uniform3f(&self, location: GLint, v0: GLfloat, v1: GLfloat, v2: GLfloat) {
        unsafe { (self.uniform3f_fn)(location, v0, v1, v2) }
    }
    pub(super) fn uniform4f(&self, location: GLint, v: [GLfloat; 4]) {
        unsafe { (self.uniform4f_fn)(location, v[0], v[1], v[2], v[3]) }
    }
    pub(super) fn uniform_matrix3(&self, location: GLint, matrix: &[GLfloat; 9]) {
        unsafe { (self.uniform_matrix3fv_fn)(location, 1, GL_FALSE, matrix.as_ptr()) }
    }
    pub(super) fn uniform_matrix4(&self, location: GLint, matrix: &[[GLfloat; 4]; 4]) {
        unsafe { (self.uniform_matrix4fv_fn)(location, 1, GL_FALSE, matrix.as_ptr().cast()) }
    }
    pub(super) fn use_program(&self, program: GLuint) {
        unsafe { (self.use_program_fn)(program) }
    }
    /// # Safety
    ///
    /// `pointer` must stay valid (for the vertices drawn) until the draw
    /// calls using it.
    pub(super) unsafe fn vertex_attrib_pointer(
        &self,
        index: GLuint,
        size: GLint,
        normalized: GLboolean,
        stride: GLsizei,
        pointer: *const f32,
    ) {
        unsafe {
            (self.vertex_attrib_pointer_fn)(
                index,
                size,
                GL_FLOAT,
                normalized,
                stride,
                pointer.cast(),
            )
        }
    }
    pub(super) fn viewport(&self, x: GLint, y: GLint, w: GLsizei, h: GLsizei) {
        unsafe { (self.viewport_fn)(x, y, w, h) }
    }
    pub(super) fn bind_framebuffer(&self, target: GLenum, framebuffer: GLuint) {
        unsafe { (self.bind_framebuffer_fn)(target, framebuffer) }
    }
    pub(super) fn framebuffer_texture_2d(
        &self,
        target: GLenum,
        attachment: GLenum,
        textarget: GLenum,
        texture: GLuint,
    ) {
        unsafe { (self.framebuffer_texture_2d_fn)(target, attachment, textarget, texture, 0) }
    }
    pub(super) fn check_framebuffer_status(&self, target: GLenum) -> GLenum {
        unsafe { (self.check_framebuffer_status_fn)(target) }
    }
    pub(super) fn delete_framebuffer(&self, framebuffer: GLuint) {
        unsafe { (self.delete_framebuffers_fn)(1, &framebuffer) }
    }
}
