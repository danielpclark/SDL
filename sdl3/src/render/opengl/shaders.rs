// Rust translation of src/render/opengl/SDL_shaders_gl.c and
// SDL_shaders_gl.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! OpenGL shader implementation: the GLSL programs of the OpenGL renderer,
//! compiled through the `GL_ARB_shader_objects` entry points.

use std::ffi::c_char;

use crate::log::{self, Category};
use crate::video::gl;

use super::{GLenum, GLint};

/// `GLhandleARB`: an `unsigned int`, a pointer on Apple platforms.
#[cfg(not(target_vendor = "apple"))]
type GLhandleARB = u32;
#[cfg(target_vendor = "apple")]
type GLhandleARB = usize;

const GL_NO_ERROR: GLenum = 0;
const GL_FRAGMENT_SHADER_ARB: GLenum = 0x8B30;
const GL_VERTEX_SHADER_ARB: GLenum = 0x8B31;
const GL_OBJECT_COMPILE_STATUS_ARB: GLenum = 0x8B81;
const GL_OBJECT_INFO_LOG_LENGTH_ARB: GLenum = 0x8B84;

/// The shader programs. Translation of `GL_Shader` (`SHADER_INVALID` is
/// `None` where a shader may be unknown).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Shader {
    None,
    Solid,
    PaletteNearest,
    PaletteLinear,
    PalettePixelart,
    Rgb,
    RgbPixelart,
    Rgba,
    RgbaPixelart,
    Yuv,
    Nv12Ra,
    Nv12Rg,
    Nv21Ra,
    Nv21Rg,
}

/// `NUM_SHADERS`.
const NUM_SHADERS: usize = 14;

impl Shader {
    /// All shaders, in `GL_Shader` order.
    const ALL: [Shader; NUM_SHADERS] = [
        Shader::None,
        Shader::Solid,
        Shader::PaletteNearest,
        Shader::PaletteLinear,
        Shader::PalettePixelart,
        Shader::Rgb,
        Shader::RgbPixelart,
        Shader::Rgba,
        Shader::RgbaPixelart,
        Shader::Yuv,
        Shader::Nv12Ra,
        Shader::Nv12Rg,
        Shader::Nv21Ra,
        Shader::Nv21Rg,
    ];

    fn index(self) -> usize {
        self as usize
    }

    /// Whether the shader takes the texel size (the shaders upstream gives
    /// 4 floats of parameters).
    fn takes_texel_size(self) -> bool {
        matches!(
            self,
            Shader::PaletteLinear
                | Shader::PalettePixelart
                | Shader::RgbPixelart
                | Shader::RgbaPixelart
        )
    }

    /// `shader >= SHADER_YUV`: the shaders that take a YCbCr matrix.
    fn takes_ycbcr_matrix(self) -> bool {
        self.index() >= Shader::Yuv.index()
    }
}

/// The parameters of a shader (upstream's `const float *shader_params`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum ShaderParams {
    /// xy: texel size, zw: texture dimensions
    TexelSize([f32; 4]),
    /// Yoffset, 0, Rcoeff, 0, Gcoeff, 0, Bcoeff, 0 (the 16 floats of
    /// `SDL_GetYCbCRtoRGBConversionMatrix()`)
    YcbcrMatrix([f32; 16]),
}

type PfnAttachObject = unsafe extern "system" fn(GLhandleARB, GLhandleARB);
type PfnCompileShader = unsafe extern "system" fn(GLhandleARB);
type PfnCreateProgramObject = unsafe extern "system" fn() -> GLhandleARB;
type PfnCreateShaderObject = unsafe extern "system" fn(GLenum) -> GLhandleARB;
type PfnDeleteObject = unsafe extern "system" fn(GLhandleARB);
type PfnGetInfoLog = unsafe extern "system" fn(GLhandleARB, i32, *mut i32, *mut c_char);
type PfnGetObjectParameteriv = unsafe extern "system" fn(GLhandleARB, GLenum, *mut GLint);
type PfnGetUniformLocation = unsafe extern "system" fn(GLhandleARB, *const c_char) -> GLint;
type PfnLinkProgram = unsafe extern "system" fn(GLhandleARB);
type PfnShaderSource =
    unsafe extern "system" fn(GLhandleARB, i32, *const *const c_char, *const GLint);
type PfnUniform1i = unsafe extern "system" fn(GLint, GLint);
type PfnUniform1f = unsafe extern "system" fn(GLint, f32);
type PfnUniform3f = unsafe extern "system" fn(GLint, f32, f32, f32);
type PfnUniform4f = unsafe extern "system" fn(GLint, f32, f32, f32, f32);
type PfnUseProgramObject = unsafe extern "system" fn(GLhandleARB);
type PfnGetError = unsafe extern "system" fn() -> GLenum;

/// The shader entry points.
#[allow(non_snake_case)]
#[derive(Clone, Copy)]
struct ShaderFuncs {
    glGetError: PfnGetError,
    glAttachObjectARB: PfnAttachObject,
    glCompileShaderARB: PfnCompileShader,
    glCreateProgramObjectARB: PfnCreateProgramObject,
    glCreateShaderObjectARB: PfnCreateShaderObject,
    glDeleteObjectARB: PfnDeleteObject,
    glGetInfoLogARB: PfnGetInfoLog,
    glGetObjectParameterivARB: PfnGetObjectParameteriv,
    glGetUniformLocationARB: PfnGetUniformLocation,
    glLinkProgramARB: PfnLinkProgram,
    glShaderSourceARB: PfnShaderSource,
    glUniform1iARB: PfnUniform1i,
    #[allow(dead_code)] // (loaded and checked as upstream does; no shader uses it)
    glUniform1fARB: PfnUniform1f,
    glUniform3fARB: PfnUniform3f,
    glUniform4fARB: PfnUniform4f,
    glUseProgramObjectARB: PfnUseProgramObject,
}

/// Translation of `GL_ShaderData`.
#[derive(Clone, Copy, Default)]
struct ShaderData {
    program: GLhandleARB,
    vert_shader: GLhandleARB,
    frag_shader: GLhandleARB,
}

/// The compiled shaders. Translation of `GL_ShaderContext`.
pub(crate) struct ShaderContext {
    f: ShaderFuncs,
    gl_arb_texture_rectangle_supported: bool,
    shaders: [ShaderData; NUM_SHADERS],
    shader_params: [Option<ShaderParams>; NUM_SHADERS],
}

/* *INDENT-OFF* */
// clang-format off

macro_rules! color_vertex_shader {
    () => {
        concat!(
            "varying vec4 v_color;\n",
            "\n",
            "void main()\n",
            "{\n",
            "    gl_Position = gl_ModelViewProjectionMatrix * gl_Vertex;\n",
            "    v_color = gl_Color;\n",
            "}",
        )
    };
}

macro_rules! texture_vertex_shader {
    () => {
        concat!(
            "varying vec4 v_color;\n",
            "varying vec2 v_texCoord;\n",
            "\n",
            "void main()\n",
            "{\n",
            "    gl_Position = gl_ModelViewProjectionMatrix * gl_Vertex;\n",
            "    v_color = gl_Color;\n",
            "    v_texCoord = vec2(gl_MultiTexCoord0);\n",
            "}",
        )
    };
}

macro_rules! rgb_shader_prologue {
    () => {
        concat!(
            "varying vec4 v_color;\n",
            "varying vec2 v_texCoord;\n",
            "uniform sampler2D tex0;\n",
            "\n",
        )
    };
}

macro_rules! rgb_pixelart_shader_prologue {
    () => {
        concat!(
            "varying vec4 v_color;\n",
            "varying vec2 v_texCoord;\n",
            "uniform sampler2D tex0;\n",
            "uniform vec4 texel_size; // texel size (xy: texel size, zw: texture dimensions)\n",
            "\n",
        )
    };
}

macro_rules! palette_shader_prologue {
    () => {
        concat!(
            "varying vec4 v_color;\n",
            "varying vec2 v_texCoord;\n",
            "uniform sampler2D tex0;\n",
            "uniform sampler2D tex1;\n",
            "uniform vec4 texel_size; // texel size (xy: texel size, zw: texture dimensions)\n",
            "\n",
        )
    };
}

// Implementation with thanks from bgolus:
// https://discussions.unity.com/t/how-to-make-data-shader-support-bilinear-trilinear/598639/8
macro_rules! palette_shader_functions {
    () => {
        concat!(
            "vec4 SamplePaletteNearest(vec2 uv)\n",
            "{\n",
            "    float index = texture2D(tex0, uv).r * 255.0;\n",
            "    return texture2D(tex1, vec2((index + 0.5) / 256.0, 0.5));\n",
            "}\n",
            "\n",
            "vec4 SamplePaletteLinear(vec2 uv)\n",
            "{\n",
            "    // scale & offset uvs to integer values at texel centers\n",
            "    vec2 uv_texels = uv * texel_size.zw + 0.5;\n",
            "\n",
            "    // get uvs for the center of the 4 surrounding texels by flooring\n",
            "    vec4 uv_min_max = vec4((floor(uv_texels) - 0.5) * texel_size.xy, (floor(uv_texels) + 0.5) * texel_size.xy);\n",
            "\n",
            "    // blend factor\n",
            "    vec2 uv_frac = fract(uv_texels);\n",
            "\n",
            "    // sample all 4 texels\n",
            "    vec4 texelA = SamplePaletteNearest(uv_min_max.xy);\n",
            "    vec4 texelB = SamplePaletteNearest(uv_min_max.xw);\n",
            "    vec4 texelC = SamplePaletteNearest(uv_min_max.zy);\n",
            "    vec4 texelD = SamplePaletteNearest(uv_min_max.zw);\n",
            "\n",
            "    // bilinear interpolation\n",
            "    return mix(mix(texelA, texelB, uv_frac.y), mix(texelC, texelD, uv_frac.y), uv_frac.x);\n",
            "}\n",
            "\n",
        )
    };
}

macro_rules! pixelart_shader_functions {
    () => {
        concat!(
            "vec2 GetPixelArtUV(vec2 uv)\n",
            "{\n",
            "    vec2 boxSize = clamp(fwidth(uv) * texel_size.zw, 1e-5, 1.0);\n",
            "    vec2 tx = uv * texel_size.zw - 0.5 * boxSize;\n",
            "    vec2 txOffset = smoothstep(vec2(1.0) - boxSize, vec2(1.0), fract(tx));\n",
            "    return (floor(tx) + 0.5 + txOffset) * texel_size.xy;\n",
            "}\n",
            "\n",
            "vec4 GetPixelArtSample(vec2 uv)\n",
            "{\n",
            "    return textureGrad(tex0, GetPixelArtUV(uv), dFdx(v_texCoord), dFdy(v_texCoord));\n",
            "}\n",
            "\n",
        )
    };
}

macro_rules! yuv_shader_prologue {
    () => {
        concat!(
            "varying vec4 v_color;\n",
            "varying vec2 v_texCoord;\n",
            "uniform sampler2D tex0; // Y \n",
            "uniform sampler2D tex1; // U \n",
            "uniform sampler2D tex2; // V \n",
            "uniform vec3 Yoffset;\n",
            "uniform vec3 Rcoeff;\n",
            "uniform vec3 Gcoeff;\n",
            "uniform vec3 Bcoeff;\n",
            "\n",
        )
    };
}

macro_rules! yuv_shader_body {
    () => {
        concat!(
            "\n",
            "void main()\n",
            "{\n",
            "    vec2 tcoord;\n",
            "    vec3 yuv, rgb;\n",
            "\n",
            "    // Get the Y value \n",
            "    tcoord = v_texCoord;\n",
            "    yuv.x = texture2D(tex0, tcoord).r;\n",
            "\n",
            "    // Get the U and V values \n",
            "    tcoord *= UVCoordScale;\n",
            "    yuv.y = texture2D(tex1, tcoord).r;\n",
            "    yuv.z = texture2D(tex2, tcoord).r;\n",
            "\n",
            "    // Do the color transform \n",
            "    yuv += Yoffset;\n",
            "    rgb.r = dot(yuv, Rcoeff);\n",
            "    rgb.g = dot(yuv, Gcoeff);\n",
            "    rgb.b = dot(yuv, Bcoeff);\n",
            "\n",
            "    // That was easy. :) \n",
            "    gl_FragColor = vec4(rgb, 1.0) * v_color;\n",
            "}",
        )
    };
}

macro_rules! nv12_shader_prologue {
    () => {
        concat!(
            "varying vec4 v_color;\n",
            "varying vec2 v_texCoord;\n",
            "uniform sampler2D tex0; // Y \n",
            "uniform sampler2D tex1; // U/V \n",
            "uniform vec3 Yoffset;\n",
            "uniform vec3 Rcoeff;\n",
            "uniform vec3 Gcoeff;\n",
            "uniform vec3 Bcoeff;\n",
            "\n",
        )
    };
}

/// The NV12/NV21 shader bodies differ in the swizzle of the U/V texture.
macro_rules! nv_shader_body {
    ($swizzle:literal) => {
        concat!(
            "\n",
            "void main()\n",
            "{\n",
            "    vec2 tcoord;\n",
            "    vec3 yuv, rgb;\n",
            "\n",
            "    // Get the Y value \n",
            "    tcoord = v_texCoord;\n",
            "    yuv.x = texture2D(tex0, tcoord).r;\n",
            "\n",
            "    // Get the U and V values \n",
            "    tcoord *= UVCoordScale;\n",
            "    yuv.yz = texture2D(tex1, tcoord).",
            $swizzle,
            ";\n",
            "\n",
            "    // Do the color transform \n",
            "    yuv += Yoffset;\n",
            "    rgb.r = dot(yuv, Rcoeff);\n",
            "    rgb.g = dot(yuv, Gcoeff);\n",
            "    rgb.b = dot(yuv, Bcoeff);\n",
            "\n",
            "    // That was easy. :) \n",
            "    gl_FragColor = vec4(rgb, 1.0) * v_color;\n",
            "}",
        )
    };
}

/// The sources of a shader program (an entry of upstream's `shader_source`).
struct ShaderSource {
    vertex_shader: &'static str,
    fragment_shader: &'static str,
    fragment_version: Option<&'static str>,
}

/*
 * NOTE: Always use sampler2D, etc here. We'll #define them to the
 *  texture_rectangle versions if we choose to use that extension.
 */
/// The sources of each shader but `Shader::None`, in `GL_Shader` order.
static SHADER_SOURCE: [ShaderSource; NUM_SHADERS - 1] = [
    // SHADER_SOLID
    ShaderSource {
        vertex_shader: color_vertex_shader!(),
        fragment_shader: concat!(
            "varying vec4 v_color;\n",
            "\n",
            "void main()\n",
            "{\n",
            "    gl_FragColor = v_color;\n",
            "}",
        ),
        fragment_version: None,
    },
    // SHADER_PALETTE_NEAREST
    ShaderSource {
        vertex_shader: texture_vertex_shader!(),
        fragment_shader: concat!(
            palette_shader_prologue!(),
            palette_shader_functions!(),
            "\n",
            "void main()\n",
            "{\n",
            "    gl_FragColor = SamplePaletteNearest(v_texCoord) * v_color;\n",
            "}",
        ),
        fragment_version: None,
    },
    // SHADER_PALETTE_LINEAR
    ShaderSource {
        vertex_shader: texture_vertex_shader!(),
        fragment_shader: concat!(
            palette_shader_prologue!(),
            palette_shader_functions!(),
            "\n",
            "void main()\n",
            "{\n",
            "    gl_FragColor = SamplePaletteLinear(v_texCoord) * v_color;\n",
            "}",
        ),
        fragment_version: None,
    },
    // SHADER_PALETTE_PIXELART
    ShaderSource {
        vertex_shader: texture_vertex_shader!(),
        fragment_shader: concat!(
            palette_shader_prologue!(),
            palette_shader_functions!(),
            pixelart_shader_functions!(),
            "\n",
            "void main()\n",
            "{\n",
            "    gl_FragColor = SamplePaletteLinear(GetPixelArtUV(v_texCoord)) * v_color;\n",
            "}",
        ),
        fragment_version: Some("#version 130\n"),
    },
    // SHADER_RGB
    ShaderSource {
        vertex_shader: texture_vertex_shader!(),
        fragment_shader: concat!(
            rgb_shader_prologue!(),
            "\n",
            "void main()\n",
            "{\n",
            "    gl_FragColor = texture2D(tex0, v_texCoord);\n",
            "    gl_FragColor.a = 1.0;\n",
            "    gl_FragColor *= v_color;\n",
            "}",
        ),
        fragment_version: None,
    },
    // SHADER_RGB_PIXELART
    ShaderSource {
        vertex_shader: texture_vertex_shader!(),
        fragment_shader: concat!(
            rgb_pixelart_shader_prologue!(),
            pixelart_shader_functions!(),
            "\n",
            "void main()\n",
            "{\n",
            "    gl_FragColor = GetPixelArtSample(v_texCoord);\n",
            "    gl_FragColor.a = 1.0;\n",
            "    gl_FragColor *= v_color;\n",
            "}",
        ),
        fragment_version: Some("#version 130\n"),
    },
    // SHADER_RGBA
    ShaderSource {
        vertex_shader: texture_vertex_shader!(),
        fragment_shader: concat!(
            rgb_shader_prologue!(),
            "\n",
            "void main()\n",
            "{\n",
            "    gl_FragColor = texture2D(tex0, v_texCoord) * v_color;\n",
            "}",
        ),
        fragment_version: None,
    },
    // SHADER_RGBA_PIXELART
    ShaderSource {
        vertex_shader: texture_vertex_shader!(),
        fragment_shader: concat!(
            rgb_pixelart_shader_prologue!(),
            pixelart_shader_functions!(),
            "\n",
            "void main()\n",
            "{\n",
            "    gl_FragColor = GetPixelArtSample(v_texCoord);\n",
            "    gl_FragColor *= v_color;\n",
            "}",
        ),
        fragment_version: Some("#version 130\n"),
    },
    // SHADER_YUV
    ShaderSource {
        vertex_shader: texture_vertex_shader!(),
        fragment_shader: concat!(yuv_shader_prologue!(), yuv_shader_body!()),
        fragment_version: None,
    },
    // SHADER_NV12_RA
    ShaderSource {
        vertex_shader: texture_vertex_shader!(),
        fragment_shader: concat!(nv12_shader_prologue!(), nv_shader_body!("ra")),
        fragment_version: None,
    },
    // SHADER_NV12_RG
    ShaderSource {
        vertex_shader: texture_vertex_shader!(),
        fragment_shader: concat!(nv12_shader_prologue!(), nv_shader_body!("rg")),
        fragment_version: None,
    },
    // SHADER_NV21_RA
    ShaderSource {
        vertex_shader: texture_vertex_shader!(),
        fragment_shader: concat!(nv12_shader_prologue!(), nv_shader_body!("ar")),
        fragment_version: None,
    },
    // SHADER_NV21_RG
    ShaderSource {
        vertex_shader: texture_vertex_shader!(),
        fragment_shader: concat!(nv12_shader_prologue!(), nv_shader_body!("gr")),
        fragment_version: None,
    },
];

/* *INDENT-ON* */
// clang-format on

impl ShaderFuncs {
    /// The shader entry points, if the driver has all of them.
    fn load() -> Option<ShaderFuncs> {
        // SAFETY: each type is the type of the entry point of that name.
        unsafe {
            Some(ShaderFuncs {
                glGetError: gl::gl_function("glGetError")?,
                glAttachObjectARB: gl::gl_function("glAttachObjectARB")?,
                glCompileShaderARB: gl::gl_function("glCompileShaderARB")?,
                glCreateProgramObjectARB: gl::gl_function("glCreateProgramObjectARB")?,
                glCreateShaderObjectARB: gl::gl_function("glCreateShaderObjectARB")?,
                glDeleteObjectARB: gl::gl_function("glDeleteObjectARB")?,
                glGetInfoLogARB: gl::gl_function("glGetInfoLogARB")?,
                glGetObjectParameterivARB: gl::gl_function("glGetObjectParameterivARB")?,
                glGetUniformLocationARB: gl::gl_function("glGetUniformLocationARB")?,
                glLinkProgramARB: gl::gl_function("glLinkProgramARB")?,
                glShaderSourceARB: gl::gl_function("glShaderSourceARB")?,
                glUniform1iARB: gl::gl_function("glUniform1iARB")?,
                glUniform1fARB: gl::gl_function("glUniform1fARB")?,
                glUniform3fARB: gl::gl_function("glUniform3fARB")?,
                // Note (upstream): glUniform4fARB is the one function
                // upstream doesn't check for (and would call through NULL);
                // here a driver without it has no shaders.
                glUniform4fARB: gl::gl_function("glUniform4fARB")?,
                glUseProgramObjectARB: gl::gl_function("glUseProgramObjectARB")?,
            })
        }
    }
}

impl ShaderContext {
    /// Translation of `CompileShader()`.
    fn compile_shader(
        &self,
        shader: GLhandleARB,
        version: &str,
        defines: &str,
        source: &str,
    ) -> bool {
        let f = &self.f;
        let sources = [version, defines, source];
        let pointers = sources.map(|s| s.as_ptr() as *const c_char);
        // (the lengths are passed, so the strings need no terminator)
        let lengths = sources.map(|s| s.len() as GLint);
        let mut status: GLint = 0;
        // SAFETY: the context of the shaders is current; the pointers and
        // lengths describe the three strings.
        unsafe {
            (f.glShaderSourceARB)(
                shader,
                sources.len() as i32,
                pointers.as_ptr(),
                lengths.as_ptr(),
            );
            (f.glCompileShaderARB)(shader);
            (f.glGetObjectParameterivARB)(shader, GL_OBJECT_COMPILE_STATUS_ARB, &mut status);
        }
        if status == 0 {
            let mut length: GLint = 0;
            // SAFETY: as above; `length` is an out-parameter.
            unsafe {
                (f.glGetObjectParameterivARB)(shader, GL_OBJECT_INFO_LOG_LENGTH_ARB, &mut length)
            };
            let mut info = vec![0u8; length.max(0) as usize + 1];
            // SAFETY: `info` holds `length` + 1 bytes.
            unsafe {
                (f.glGetInfoLogARB)(
                    shader,
                    length,
                    std::ptr::null_mut(),
                    info.as_mut_ptr() as *mut c_char,
                )
            };
            let end = info.iter().position(|&c| c == 0).unwrap_or(info.len());
            let info = String::from_utf8_lossy(&info[..end]);
            log::debug!(Category::Render, "Failed to compile shader:");
            log::debug!(Category::Render, "{}", version);
            log::debug!(Category::Render, "{}", defines);
            log::debug!(Category::Render, "{}", source);
            log::debug!(Category::Render, "{}", info);
            false
        } else {
            true
        }
    }

    /// Translation of `CompileShaderProgram()`.
    fn compile_shader_program(&mut self, shader: Shader) -> bool {
        const NUM_TMUS_BOUND: usize = 4;
        let vert_defines = "";

        if shader == Shader::None {
            return true;
        }
        let source = &SHADER_SOURCE[shader.index() - 1];
        let f = self.f;

        // SAFETY: the context of the shaders is current.
        unsafe { (f.glGetError)() };

        // Make sure we use the correct sampler type for our texture type
        let frag_defines = if self.gl_arb_texture_rectangle_supported {
            concat!(
                "#define sampler2D sampler2DRect\n",
                "#define texture2D texture2DRect\n",
                "#define UVCoordScale 0.5\n",
            )
        } else {
            "#define UVCoordScale 1.0\n"
        };
        let frag_version = source.fragment_version.unwrap_or("");

        let mut data = ShaderData {
            // Create one program object to rule them all
            // SAFETY: as above.
            program: unsafe { (f.glCreateProgramObjectARB)() },
            ..ShaderData::default()
        };

        // Create the vertex shader
        // SAFETY: as above.
        data.vert_shader = unsafe { (f.glCreateShaderObjectARB)(GL_VERTEX_SHADER_ARB) };
        self.shaders[shader.index()] = data;
        if !self.compile_shader(data.vert_shader, "", vert_defines, source.vertex_shader) {
            return false;
        }

        // Create the fragment shader
        // SAFETY: as above.
        data.frag_shader = unsafe { (f.glCreateShaderObjectARB)(GL_FRAGMENT_SHADER_ARB) };
        self.shaders[shader.index()] = data;
        if !self.compile_shader(
            data.frag_shader,
            frag_version,
            frag_defines,
            source.fragment_shader,
        ) {
            return false;
        }

        // ... and in the darkness bind them
        // SAFETY: as above; the uniform names are NUL-terminated.
        unsafe {
            (f.glAttachObjectARB)(data.program, data.vert_shader);
            (f.glAttachObjectARB)(data.program, data.frag_shader);
            (f.glLinkProgramARB)(data.program);

            // Set up some uniform variables
            (f.glUseProgramObjectARB)(data.program);
            for i in 0..NUM_TMUS_BOUND {
                let tex_name = format!("tex{i}\0");
                let location =
                    (f.glGetUniformLocationARB)(data.program, tex_name.as_ptr() as *const c_char);
                if location >= 0 {
                    (f.glUniform1iARB)(location, i as GLint);
                }
            }
            (f.glUseProgramObjectARB)(0);

            (f.glGetError)() == GL_NO_ERROR
        }
    }

    /// Translation of `DestroyShaderProgram()`.
    fn destroy_shader_program(&mut self, shader: Shader) {
        let f = self.f;
        let data = &mut self.shaders[shader.index()];
        // SAFETY: the context of the shaders is current; the objects are
        // ours.
        unsafe {
            if data.vert_shader != 0 {
                (f.glDeleteObjectARB)(data.vert_shader);
                data.vert_shader = 0;
            }
            if data.frag_shader != 0 {
                (f.glDeleteObjectARB)(data.frag_shader);
                data.frag_shader = 0;
            }
            if data.program != 0 {
                (f.glDeleteObjectARB)(data.program);
                data.program = 0;
            }
        }
    }

    /// The shaders of the current context, `None` without shader support.
    /// Translation of `GL_CreateShaderContext()`.
    pub(crate) fn new() -> Option<ShaderContext> {
        let gl_arb_texture_rectangle_supported =
            !gl::gl_extension_supported("GL_ARB_texture_non_power_of_two")
                && (gl::gl_extension_supported("GL_ARB_texture_rectangle")
                    || gl::gl_extension_supported("GL_EXT_texture_rectangle"));

        // Check for shader support
        let mut funcs = None;
        if gl::gl_extension_supported("GL_ARB_shader_objects")
            && gl::gl_extension_supported("GL_ARB_shading_language_100")
            && gl::gl_extension_supported("GL_ARB_vertex_shader")
            && gl::gl_extension_supported("GL_ARB_fragment_shader")
        {
            funcs = ShaderFuncs::load();
        }

        let f = funcs?;
        let mut ctx = ShaderContext {
            f,
            gl_arb_texture_rectangle_supported,
            shaders: [ShaderData::default(); NUM_SHADERS],
            shader_params: [None; NUM_SHADERS],
        };

        // Compile all the shaders
        for shader in Shader::ALL {
            if !ctx.compile_shader_program(shader) {
                ctx.destroy_shader_program(shader);
            }
        }

        // We're done!
        Some(ctx)
    }

    /// Whether `shader` compiled. Translation of `GL_SupportsShader()` (a
    /// missing context is `Option::None` at the caller).
    pub(crate) fn supports(&self, shader: Shader) -> bool {
        self.shaders[shader.index()].program != 0
    }

    /// Use `shader` with its parameters. Translation of `GL_SelectShader()`.
    pub(crate) fn select(&mut self, shader: Shader, shader_params: Option<&ShaderParams>) {
        let f = self.f;
        let program = self.shaders[shader.index()].program;

        // SAFETY: the context of the shaders is current.
        unsafe { (f.glUseProgramObjectARB)(program) };

        crate::sdl_assert!(
            shader_params.is_none() || shader.takes_texel_size() || shader.takes_ycbcr_matrix()
        );

        let Some(params) = shader_params else {
            return;
        };
        if self.shader_params[shader.index()].as_ref() == Some(params) {
            return;
        }
        let location = |name: &std::ffi::CStr| {
            // SAFETY: as above; the name is NUL-terminated.
            unsafe { (f.glGetUniformLocationARB)(program, name.as_ptr()) }
        };
        match params {
            ShaderParams::TexelSize(p) if shader.takes_texel_size() => {
                let location = location(c"texel_size");
                if location >= 0 {
                    // SAFETY: as above.
                    unsafe { (f.glUniform4fARB)(location, p[0], p[1], p[2], p[3]) };
                }
            }
            ShaderParams::YcbcrMatrix(p) if shader.takes_ycbcr_matrix() => {
                // YUV shader params are Yoffset, 0, Rcoeff, 0, Gcoeff, 0, Bcoeff, 0
                for (name, i) in [
                    (c"Yoffset", 0),
                    (c"Rcoeff", 4),
                    (c"Gcoeff", 8),
                    (c"Bcoeff", 12),
                ] {
                    let location = location(name);
                    if location >= 0 {
                        // SAFETY: as above.
                        unsafe { (f.glUniform3fARB)(location, p[i], p[i + 1], p[i + 2]) };
                    }
                }
            }
            _ => {}
        }
        self.shader_params[shader.index()] = Some(*params);
    }

    /// Translation of `GL_DestroyShaderContext()`: delete the programs (the
    /// renderer's context must be current).
    pub(crate) fn destroy(mut self) {
        for shader in Shader::ALL {
            self.destroy_shader_program(shader);
        }
    }
}

/// The vertex shader, fragment shader and fragment version of a shader
/// (for the tests).
#[cfg(test)]
pub(crate) fn source_of(
    shader: Shader,
) -> Option<(&'static str, &'static str, Option<&'static str>)> {
    if shader == Shader::None {
        return None;
    }
    let s = &SHADER_SOURCE[shader.index() - 1];
    Some((s.vertex_shader, s.fragment_shader, s.fragment_version))
}
