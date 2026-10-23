// The OpenGL (ES) calls the OpenGL and OpenGL ES 2 renderer tests make
// themselves, to make textures for the renderers to wrap and to read them
// back.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use crate::video::gl;

const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_TEXTURE_BINDING_2D: u32 = 0x8069;
const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_FRAMEBUFFER_BINDING: u32 = 0x8CA6;
const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
const GL_RGBA: u32 = 0x1908;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_PACK_ALIGNMENT: u32 = 0x0D05;
const GL_PACK_ROW_LENGTH: u32 = 0x0D02;

/// The entry points, looked up in the current context's library.
pub(crate) struct TestGl {
    gen_textures: unsafe extern "system" fn(i32, *mut u32),
    delete_textures: unsafe extern "system" fn(i32, *const u32),
    is_texture: unsafe extern "system" fn(u32) -> u8,
    is_shader: unsafe extern "system" fn(u32) -> u8,
    bind_texture: unsafe extern "system" fn(u32, u32),
    get_integerv: unsafe extern "system" fn(u32, *mut i32),
    gen_framebuffers: unsafe extern "system" fn(i32, *mut u32),
    delete_framebuffers: unsafe extern "system" fn(i32, *const u32),
    bind_framebuffer: unsafe extern "system" fn(u32, u32),
    framebuffer_texture_2d: unsafe extern "system" fn(u32, u32, u32, u32, i32),
    check_framebuffer_status: unsafe extern "system" fn(u32) -> u32,
    read_pixels: unsafe extern "system" fn(i32, i32, i32, i32, u32, u32, *mut u8),
    pixel_storei: unsafe extern "system" fn(u32, i32),
    finish: unsafe extern "system" fn(),
    /// The context has `GL_PACK_ROW_LENGTH` (desktop OpenGL; OpenGL ES 2
    /// hasn't).
    pack_row_length: bool,
}

impl TestGl {
    /// The entry points (`EXT` names are tried for the framebuffer
    /// functions of a desktop library without them in core), for a desktop
    /// OpenGL context or an OpenGL ES 2 one.
    pub(crate) fn load(desktop: bool) -> TestGl {
        // SAFETY: each type is the entry point's.
        unsafe {
            macro_rules! f {
                ($($name:literal),+) => {
                    None$(.or_else(|| gl::gl_function($name)))+.expect(concat!($($name),+))
                };
            }
            TestGl {
                gen_textures: f!("glGenTextures"),
                delete_textures: f!("glDeleteTextures"),
                is_texture: f!("glIsTexture"),
                is_shader: f!("glIsShader"),
                bind_texture: f!("glBindTexture"),
                get_integerv: f!("glGetIntegerv"),
                gen_framebuffers: f!("glGenFramebuffers", "glGenFramebuffersEXT"),
                delete_framebuffers: f!("glDeleteFramebuffers", "glDeleteFramebuffersEXT"),
                bind_framebuffer: f!("glBindFramebuffer", "glBindFramebufferEXT"),
                framebuffer_texture_2d: f!("glFramebufferTexture2D", "glFramebufferTexture2DEXT"),
                check_framebuffer_status: f!(
                    "glCheckFramebufferStatus",
                    "glCheckFramebufferStatusEXT"
                ),
                read_pixels: f!("glReadPixels"),
                pixel_storei: f!("glPixelStorei"),
                finish: f!("glFinish"),
                pack_row_length: desktop,
            }
        }
    }

    /// A new texture name, of no texture yet (the renderer makes its
    /// storage). The context the names belong to must be current.
    pub(crate) fn gen_texture(&self) -> u32 {
        let mut name = 0;
        // SAFETY: one name is written.
        unsafe { (self.gen_textures)(1, &mut name) };
        assert_ne!(name, 0);
        name
    }

    /// `glGetIntegerv()` of one value.
    pub(crate) fn get_integer(&self, pname: u32) -> i32 {
        let mut value = 0;
        // SAFETY: a plain GL call on the current context, one value written.
        unsafe { (self.get_integerv)(pname, &mut value) };
        value
    }

    /// Bind `name` as a 2D texture (making it one), then put the previous
    /// binding back.
    pub(crate) fn make_2d_texture(&self, name: u32) {
        // SAFETY: plain GL calls on the current context.
        unsafe {
            let mut previous = 0;
            (self.get_integerv)(GL_TEXTURE_BINDING_2D, &mut previous);
            (self.bind_texture)(GL_TEXTURE_2D, name);
            (self.bind_texture)(GL_TEXTURE_2D, previous as u32);
        }
    }

    /// Whether `name` is (still) a texture.
    pub(crate) fn is_texture(&self, name: u32) -> bool {
        // SAFETY: a plain GL call on the current context.
        unsafe { (self.is_texture)(name) != 0 }
    }

    /// Whether `name` is a shader object.
    pub(crate) fn is_shader(&self, name: u32) -> bool {
        // SAFETY: a plain GL call on the current context.
        unsafe { (self.is_shader)(name) != 0 }
    }

    /// Delete a texture.
    pub(crate) fn delete_texture(&self, name: u32) {
        // SAFETY: one name is read.
        unsafe { (self.delete_textures)(1, &name) };
    }

    /// The RGBA bytes of a `w` x `h` 2D texture, through a framebuffer of
    /// the test's (the previous binding is put back), bottom row first as
    /// GL reads them.
    pub(crate) fn read_texture(&self, name: u32, w: i32, h: i32) -> Vec<u8> {
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        // SAFETY: plain GL calls on the current context; `pixels` holds
        // the w x h RGBA pixels read, tightly packed.
        unsafe {
            let mut previous = 0;
            (self.get_integerv)(GL_FRAMEBUFFER_BINDING, &mut previous);
            let mut fbo = 0;
            (self.gen_framebuffers)(1, &mut fbo);
            (self.bind_framebuffer)(GL_FRAMEBUFFER, fbo);
            (self.framebuffer_texture_2d)(
                GL_FRAMEBUFFER,
                GL_COLOR_ATTACHMENT0,
                GL_TEXTURE_2D,
                name,
                0,
            );
            assert_eq!(
                (self.check_framebuffer_status)(GL_FRAMEBUFFER),
                GL_FRAMEBUFFER_COMPLETE
            );
            // (the renderers' reads leave their packing set)
            let (mut alignment, mut row_length) = (0, 0);
            (self.get_integerv)(GL_PACK_ALIGNMENT, &mut alignment);
            (self.pixel_storei)(GL_PACK_ALIGNMENT, 1);
            if self.pack_row_length {
                (self.get_integerv)(GL_PACK_ROW_LENGTH, &mut row_length);
                (self.pixel_storei)(GL_PACK_ROW_LENGTH, 0);
            }
            (self.read_pixels)(0, 0, w, h, GL_RGBA, GL_UNSIGNED_BYTE, pixels.as_mut_ptr());
            (self.pixel_storei)(GL_PACK_ALIGNMENT, alignment);
            if self.pack_row_length {
                (self.pixel_storei)(GL_PACK_ROW_LENGTH, row_length);
            }
            (self.finish)();
            (self.bind_framebuffer)(GL_FRAMEBUFFER, previous as u32);
            (self.delete_framebuffers)(1, &fbo);
        }
        pixels
    }
}
