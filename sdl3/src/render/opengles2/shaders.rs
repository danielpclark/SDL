// Rust translation of src/render/opengles2/SDL_shaders_gles2.c and
// SDL_shaders_gles2.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The GLSL ES sources of the OpenGL ES 2.0 renderer's shaders.
//!
//! This is the OpenGL ES 2.0 build (`OPENGLES_300` undefined): the pixel
//! art shaders sample like the nearest ones, since `fwidth()` and
//! `textureGrad()` need OpenGL ES 3.0.

use crate::hints;

/// The precision prelude of a fragment shader. Translation of
/// `GLES2_ShaderIncludeType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ShaderInclude {
    None = 0,
    BestTexcoordPrecision,
    MediumTexcoordPrecision,
    HighTexcoordPrecision,
    UndefPrecision,
}

/// A shader of the renderer. Translation of `GLES2_ShaderType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ShaderType {
    VertexDefault = 0,
    FragmentSolid,
    FragmentTexturePaletteNearest,
    FragmentTexturePaletteLinear,
    FragmentTexturePalettePixelart,
    FragmentTexturePaletteNearestColorswap,
    FragmentTexturePaletteLinearColorswap,
    FragmentTexturePalettePixelartColorswap,
    FragmentTextureRgb,
    FragmentTextureRgbPixelart,
    FragmentTextureBgr,
    FragmentTextureBgrPixelart,
    FragmentTextureArgb,
    FragmentTextureArgbPixelart,
    FragmentTextureAbgr,
    FragmentTextureAbgrPixelart,
    FragmentTextureYuv,
    FragmentTextureNv12Ra,
    FragmentTextureNv12Rg,
    FragmentTextureNv21Ra,
    FragmentTextureNv21Rg,
    // Shaders beyond this point are optional and not cached at render creation
    FragmentTextureExternalOes,
}

impl ShaderType {
    /// Translation of `GLES2_SHADER_COUNT`.
    pub(super) const COUNT: usize = ShaderType::FragmentTextureExternalOes as usize + 1;

    /// The shaders in `GLES2_ShaderType` order.
    pub(super) const ALL: [ShaderType; ShaderType::COUNT] = {
        use ShaderType::*;
        [
            VertexDefault,
            FragmentSolid,
            FragmentTexturePaletteNearest,
            FragmentTexturePaletteLinear,
            FragmentTexturePalettePixelart,
            FragmentTexturePaletteNearestColorswap,
            FragmentTexturePaletteLinearColorswap,
            FragmentTexturePalettePixelartColorswap,
            FragmentTextureRgb,
            FragmentTextureRgbPixelart,
            FragmentTextureBgr,
            FragmentTextureBgrPixelart,
            FragmentTextureArgb,
            FragmentTextureArgbPixelart,
            FragmentTextureAbgr,
            FragmentTextureAbgrPixelart,
            FragmentTextureYuv,
            FragmentTextureNv12Ra,
            FragmentTextureNv12Rg,
            FragmentTextureNv21Ra,
            FragmentTextureNv21Rg,
            FragmentTextureExternalOes,
        ]
    };

    /// The source that goes before the precision prelude.
    /// Translation of `GLES2_GetShaderPrologue()`.
    pub(super) fn prologue(self) -> &'static str {
        match self {
            ShaderType::FragmentTextureExternalOes => FRAGMENT_TEXTURE_EXTERNAL_OES_PROLOGUE,
            _ => "",
        }
    }

    /// The shader's body. Translation of `GLES2_GetShader()`.
    pub(super) fn source(self) -> &'static str {
        use ShaderType::*;
        match self {
            VertexDefault => VERTEX_DEFAULT,
            FragmentSolid => FRAGMENT_SOLID,
            FragmentTexturePaletteNearest => FRAGMENT_TEXTURE_PALETTE_NEAREST,
            FragmentTexturePaletteLinear => FRAGMENT_TEXTURE_PALETTE_LINEAR,
            FragmentTexturePalettePixelart => FRAGMENT_TEXTURE_PALETTE_PIXELART,
            FragmentTexturePaletteNearestColorswap => FRAGMENT_TEXTURE_PALETTE_NEAREST_COLORSWAP,
            FragmentTexturePaletteLinearColorswap => FRAGMENT_TEXTURE_PALETTE_LINEAR_COLORSWAP,
            FragmentTexturePalettePixelartColorswap => FRAGMENT_TEXTURE_PALETTE_PIXELART_COLORSWAP,
            FragmentTextureRgb => FRAGMENT_TEXTURE_RGB,
            FragmentTextureRgbPixelart => FRAGMENT_TEXTURE_RGB_PIXELART,
            FragmentTextureBgr => FRAGMENT_TEXTURE_BGR,
            FragmentTextureBgrPixelart => FRAGMENT_TEXTURE_BGR_PIXELART,
            FragmentTextureArgb => FRAGMENT_TEXTURE_ARGB,
            FragmentTextureArgbPixelart => FRAGMENT_TEXTURE_ARGB_PIXELART,
            FragmentTextureAbgr => FRAGMENT_TEXTURE_ABGR,
            FragmentTextureAbgrPixelart => FRAGMENT_TEXTURE_ABGR_PIXELART,
            FragmentTextureYuv => FRAGMENT_TEXTURE_YUV,
            FragmentTextureNv12Ra => FRAGMENT_TEXTURE_NV12_RA,
            FragmentTextureNv12Rg => FRAGMENT_TEXTURE_NV12_RG,
            FragmentTextureNv21Ra => FRAGMENT_TEXTURE_NV21_RA,
            FragmentTextureNv21Rg => FRAGMENT_TEXTURE_NV21_RG,
            FragmentTextureExternalOes => FRAGMENT_TEXTURE_EXTERNAL_OES,
        }
    }
}

impl ShaderInclude {
    /// The precision prelude's source. Translation of `GLES2_GetShaderInclude()`.
    pub(super) fn source(self) -> &'static str {
        match self {
            ShaderInclude::UndefPrecision => FRAGMENT_INCLUDE_UNDEF_PRECISION,
            ShaderInclude::BestTexcoordPrecision => FRAGMENT_INCLUDE_BEST_TEXTURE_PRECISION,
            ShaderInclude::MediumTexcoordPrecision => FRAGMENT_INCLUDE_MEDIUM_TEXTURE_PRECISION,
            ShaderInclude::HighTexcoordPrecision => FRAGMENT_INCLUDE_HIGH_TEXTURE_PRECISION,
            ShaderInclude::None => "",
        }
    }

    /// The prelude the `SDL_RENDER_OPENGLES2_TEXCOORD_PRECISION` hint asks
    /// for. Translation of `GLES2_GetTexCoordPrecisionEnumFromHint()`.
    pub(super) fn texcoord_precision_from_hint() -> ShaderInclude {
        let texcoord_hint = hints::get("SDL_RENDER_OPENGLES2_TEXCOORD_PRECISION");
        let value = ShaderInclude::BestTexcoordPrecision;
        if let Some(texcoord_hint) = texcoord_hint {
            if texcoord_hint == "undefined" {
                return ShaderInclude::UndefPrecision;
            }
            if texcoord_hint == "high" {
                return ShaderInclude::HighTexcoordPrecision;
            }
            if texcoord_hint == "medium" {
                return ShaderInclude::MediumTexcoordPrecision;
            }
        }
        value
    }
}

/*************************************************************************************************
 * Vertex/fragment shader source                                                                 *
 *************************************************************************************************/

const FRAGMENT_INCLUDE_BEST_TEXTURE_PRECISION: &str = "\
#ifdef GL_FRAGMENT_PRECISION_HIGH
#define SDL_TEXCOORD_PRECISION highp
#else
#define SDL_TEXCOORD_PRECISION mediump
#endif

precision mediump float;

";

const FRAGMENT_INCLUDE_MEDIUM_TEXTURE_PRECISION: &str = "\
#define SDL_TEXCOORD_PRECISION mediump
precision mediump float;

";

const FRAGMENT_INCLUDE_HIGH_TEXTURE_PRECISION: &str = "\
#define SDL_TEXCOORD_PRECISION highp
precision mediump float;

";

const FRAGMENT_INCLUDE_UNDEF_PRECISION: &str = "\
#define mediump
#define highp
#define lowp
#define SDL_TEXCOORD_PRECISION

";

const VERTEX_DEFAULT: &str = "\
uniform mat4 u_projection;
attribute vec2 a_position;
attribute vec4 a_color;
attribute vec2 a_texCoord;
varying vec2 v_texCoord;
varying vec4 v_color;

void main()
{
    v_texCoord = a_texCoord;
    gl_Position = u_projection * vec4(a_position, 0.0, 1.0);
    gl_PointSize = 1.0;
    v_color = a_color;
}
";

const FRAGMENT_SOLID: &str = "\
varying mediump vec4 v_color;

void main()
{
    gl_FragColor = v_color;
}
";

macro_rules! rgb_shader_prologue {
    () => {
        "\
uniform sampler2D u_texture;
varying mediump vec4 v_color;
varying SDL_TEXCOORD_PRECISION vec2 v_texCoord;
"
    };
}

macro_rules! rgb_pixelart_shader_prologue {
    () => {
        "\
uniform sampler2D u_texture;
uniform mediump vec4 u_texel_size;
varying mediump vec4 v_color;
varying SDL_TEXCOORD_PRECISION vec2 v_texCoord;
"
    };
}

macro_rules! palette_shader_prologue {
    () => {
        "\
uniform sampler2D u_texture;
uniform sampler2D u_palette;
uniform mediump vec4 u_texel_size;
varying mediump vec4 v_color;
varying SDL_TEXCOORD_PRECISION vec2 v_texCoord;
"
    };
}

// Implementation with thanks from bgolus:
// https://discussions.unity.com/t/how-to-make-data-shader-support-bilinear-trilinear/598639/8
macro_rules! palette_shader_functions {
    () => {
        "\
mediump vec4 SamplePaletteNearest(SDL_TEXCOORD_PRECISION vec2 uv)
{
    mediump float index = texture2D(u_texture, uv).r * 255.0;
    return texture2D(u_palette, vec2((index + 0.5) / 256.0, 0.5));
}

mediump vec4 SamplePaletteLinear(SDL_TEXCOORD_PRECISION vec2 uv)
{
    // scale & offset uvs to integer values at texel centers
    SDL_TEXCOORD_PRECISION vec2 uv_texels = uv * u_texel_size.zw + 0.5;

    // get uvs for the center of the 4 surrounding texels by flooring
    SDL_TEXCOORD_PRECISION vec4 uv_min_max = vec4((floor(uv_texels) - 0.5) * u_texel_size.xy, (floor(uv_texels) + 0.5) * u_texel_size.xy);

    // blend factor
    SDL_TEXCOORD_PRECISION vec2 uv_frac = fract(uv_texels);

    // sample all 4 texels
    mediump vec4 texelA = SamplePaletteNearest(uv_min_max.xy);
    mediump vec4 texelB = SamplePaletteNearest(uv_min_max.xw);
    mediump vec4 texelC = SamplePaletteNearest(uv_min_max.zy);
    mediump vec4 texelD = SamplePaletteNearest(uv_min_max.zw);

    // bilinear interpolation
    return mix(mix(texelA, texelB, uv_frac.y), mix(texelC, texelD, uv_frac.y), uv_frac.x);
}

"
    };
}

// (the OPENGLES_300 version uses fwidth() and textureGrad())
macro_rules! pixelart_shader_functions {
    () => {
        "\
mediump vec4 GetPixelArtSample(vec2 uv)
{
    return texture2D(u_texture, uv);
}

"
    };
}

const FRAGMENT_TEXTURE_PALETTE_NEAREST: &str = concat!(
    palette_shader_prologue!(),
    palette_shader_functions!(),
    "
void main()
{
    mediump vec4 color = SamplePaletteNearest(v_texCoord);
    gl_FragColor = color;
    gl_FragColor *= v_color;
}
"
);

const FRAGMENT_TEXTURE_PALETTE_NEAREST_COLORSWAP: &str = concat!(
    palette_shader_prologue!(),
    palette_shader_functions!(),
    "
void main()
{
    mediump vec4 color = SamplePaletteNearest(v_texCoord);
    gl_FragColor = vec4(color.b, color.g, color.r, color.a);
    gl_FragColor *= v_color;
}
"
);

const FRAGMENT_TEXTURE_PALETTE_LINEAR: &str = concat!(
    palette_shader_prologue!(),
    palette_shader_functions!(),
    "
void main()
{
    mediump vec4 color = SamplePaletteLinear(v_texCoord);
    gl_FragColor = color;
    gl_FragColor *= v_color;
}
"
);

const FRAGMENT_TEXTURE_PALETTE_LINEAR_COLORSWAP: &str = concat!(
    palette_shader_prologue!(),
    palette_shader_functions!(),
    "
void main()
{
    mediump vec4 color = SamplePaletteLinear(v_texCoord);
    gl_FragColor = vec4(color.b, color.g, color.r, color.a);
    gl_FragColor *= v_color;
}
"
);

const FRAGMENT_TEXTURE_PALETTE_PIXELART: &str = concat!(
    palette_shader_prologue!(),
    palette_shader_functions!(),
    pixelart_shader_functions!(),
    "
void main()
{
    mediump vec4 color = SamplePaletteNearest(v_texCoord);
    gl_FragColor = color;
    gl_FragColor *= v_color;
}
"
);

const FRAGMENT_TEXTURE_PALETTE_PIXELART_COLORSWAP: &str = concat!(
    palette_shader_prologue!(),
    palette_shader_functions!(),
    pixelart_shader_functions!(),
    "
void main()
{
    mediump vec4 color = SamplePaletteNearest(v_texCoord);
    gl_FragColor = vec4(color.b, color.g, color.r, color.a);
    gl_FragColor *= v_color;
}
"
);

// RGB to ABGR conversion
const FRAGMENT_TEXTURE_RGB: &str = concat!(
    rgb_shader_prologue!(),
    "
void main()
{
    mediump vec4 color = texture2D(u_texture, v_texCoord);
    gl_FragColor = vec4(color.b, color.g, color.r, 1.0);
    gl_FragColor *= v_color;
}
"
);

// RGB to ABGR conversion
const FRAGMENT_TEXTURE_RGB_PIXELART: &str = concat!(
    rgb_pixelart_shader_prologue!(),
    pixelart_shader_functions!(),
    "
void main()
{
    mediump vec4 color = GetPixelArtSample(v_texCoord);
    gl_FragColor = vec4(color.b, color.g, color.r, 1.0);
    gl_FragColor *= v_color;
}
"
);

// BGR to ABGR conversion
const FRAGMENT_TEXTURE_BGR: &str = concat!(
    rgb_shader_prologue!(),
    "
void main()
{
    mediump vec4 color = texture2D(u_texture, v_texCoord);
    gl_FragColor = vec4(color.r, color.g, color.b, 1.0);
    gl_FragColor *= v_color;
}
"
);

// BGR to ABGR conversion
const FRAGMENT_TEXTURE_BGR_PIXELART: &str = concat!(
    rgb_pixelart_shader_prologue!(),
    pixelart_shader_functions!(),
    "
void main()
{
    mediump vec4 color = GetPixelArtSample(v_texCoord);
    gl_FragColor = vec4(color.r, color.g, color.b, 1.0);
    gl_FragColor *= v_color;
}
"
);

// ARGB to ABGR conversion
const FRAGMENT_TEXTURE_ARGB: &str = concat!(
    rgb_shader_prologue!(),
    "
void main()
{
    mediump vec4 color = texture2D(u_texture, v_texCoord);
    gl_FragColor = vec4(color.b, color.g, color.r, color.a);
    gl_FragColor *= v_color;
}
"
);

// ARGB to ABGR conversion
const FRAGMENT_TEXTURE_ARGB_PIXELART: &str = concat!(
    rgb_pixelart_shader_prologue!(),
    pixelart_shader_functions!(),
    "
void main()
{
    mediump vec4 color = GetPixelArtSample(v_texCoord);
    gl_FragColor = vec4(color.b, color.g, color.r, color.a);
    gl_FragColor *= v_color;
}
"
);

const FRAGMENT_TEXTURE_ABGR: &str = concat!(
    rgb_shader_prologue!(),
    "
void main()
{
    mediump vec4 color = texture2D(u_texture, v_texCoord);
    gl_FragColor = color;
    gl_FragColor *= v_color;
}
"
);

const FRAGMENT_TEXTURE_ABGR_PIXELART: &str = concat!(
    rgb_pixelart_shader_prologue!(),
    pixelart_shader_functions!(),
    "
void main()
{
    mediump vec4 color = GetPixelArtSample(v_texCoord);
    gl_FragColor = color;
    gl_FragColor *= v_color;
}
"
);

macro_rules! yuv_shader_prologue {
    () => {
        "\
uniform sampler2D u_texture;
uniform sampler2D u_texture_u;
uniform sampler2D u_texture_v;
uniform vec3 u_offset;
uniform mat3 u_matrix;
varying mediump vec4 v_color;
varying SDL_TEXCOORD_PRECISION vec2 v_texCoord;

"
    };
}

/// The body of the planar and semi-planar YUV shaders, with the line that
/// fetches the chroma.
macro_rules! yuv_shader_body {
    ($chroma:literal) => {
        concat!(
            "void main()\n",
            "{\n",
            "    mediump vec3 yuv;\n",
            "    lowp vec3 rgb;\n",
            "\n",
            "    // Get the YUV values \n",
            "    yuv.x = texture2D(u_texture,   v_texCoord).r;\n",
            $chroma,
            "\n",
            "    // Do the color transform \n",
            "    yuv += u_offset;\n",
            "    rgb = yuv * u_matrix;\n",
            "\n",
            "    // That was easy. :) \n",
            "    gl_FragColor = vec4(rgb, 1);\n",
            "    gl_FragColor *= v_color;\n",
            "}"
        )
    };
}

// YUV to ABGR conversion
const FRAGMENT_TEXTURE_YUV: &str = concat!(
    yuv_shader_prologue!(),
    yuv_shader_body!(
        "    yuv.y = texture2D(u_texture_u, v_texCoord).r;\n    yuv.z = texture2D(u_texture_v, v_texCoord).r;\n"
    )
);

// NV12 to ABGR conversion
const FRAGMENT_TEXTURE_NV12_RA: &str = concat!(
    yuv_shader_prologue!(),
    yuv_shader_body!("    yuv.yz = texture2D(u_texture_u, v_texCoord).ra;\n")
);
const FRAGMENT_TEXTURE_NV12_RG: &str = concat!(
    yuv_shader_prologue!(),
    yuv_shader_body!("    yuv.yz = texture2D(u_texture_u, v_texCoord).rg;\n")
);

// NV21 to ABGR conversion
const FRAGMENT_TEXTURE_NV21_RA: &str = concat!(
    yuv_shader_prologue!(),
    yuv_shader_body!("    yuv.yz = texture2D(u_texture_u, v_texCoord).ar;\n")
);
const FRAGMENT_TEXTURE_NV21_RG: &str = concat!(
    yuv_shader_prologue!(),
    yuv_shader_body!("    yuv.yz = texture2D(u_texture_u, v_texCoord).gr;\n")
);

// Custom Android video format texture
const FRAGMENT_TEXTURE_EXTERNAL_OES_PROLOGUE: &str = "\
#extension GL_OES_EGL_image_external : require

";
const FRAGMENT_TEXTURE_EXTERNAL_OES: &str = "\
uniform samplerExternalOES u_texture;
varying mediump vec4 v_color;
varying SDL_TEXCOORD_PRECISION vec2 v_texCoord;

void main()
{
    gl_FragColor = texture2D(u_texture, v_texCoord);
    gl_FragColor *= v_color;
}
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_types_keep_upstreams_values() {
        assert_eq!(ShaderType::COUNT, 22);
        for (i, t) in ShaderType::ALL.iter().enumerate() {
            assert_eq!(*t as usize, i);
        }
        assert_eq!(ShaderType::FragmentTextureYuv as u32, 16);
        assert_eq!(ShaderInclude::UndefPrecision as u32, 4);
    }

    #[test]
    fn shader_sources_match_upstreams() {
        // The YUV body as the C string literals spell it (with their
        // trailing spaces and no final newline).
        assert_eq!(
            FRAGMENT_TEXTURE_NV21_RA,
            "uniform sampler2D u_texture;\n\
             uniform sampler2D u_texture_u;\n\
             uniform sampler2D u_texture_v;\n\
             uniform vec3 u_offset;\n\
             uniform mat3 u_matrix;\n\
             varying mediump vec4 v_color;\n\
             varying SDL_TEXCOORD_PRECISION vec2 v_texCoord;\n\
             \n\
             void main()\n\
             {\n\
             \x20   mediump vec3 yuv;\n\
             \x20   lowp vec3 rgb;\n\
             \n\
             \x20   // Get the YUV values \n\
             \x20   yuv.x = texture2D(u_texture,   v_texCoord).r;\n\
             \x20   yuv.yz = texture2D(u_texture_u, v_texCoord).ar;\n\
             \n\
             \x20   // Do the color transform \n\
             \x20   yuv += u_offset;\n\
             \x20   rgb = yuv * u_matrix;\n\
             \n\
             \x20   // That was easy. :) \n\
             \x20   gl_FragColor = vec4(rgb, 1);\n\
             \x20   gl_FragColor *= v_color;\n\
             }"
        );
        assert_eq!(
            FRAGMENT_TEXTURE_ARGB,
            "uniform sampler2D u_texture;\n\
             varying mediump vec4 v_color;\n\
             varying SDL_TEXCOORD_PRECISION vec2 v_texCoord;\n\
             \n\
             void main()\n\
             {\n\
             \x20   mediump vec4 color = texture2D(u_texture, v_texCoord);\n\
             \x20   gl_FragColor = vec4(color.b, color.g, color.r, color.a);\n\
             \x20   gl_FragColor *= v_color;\n\
             }\n"
        );
        assert!(FRAGMENT_TEXTURE_PALETTE_PIXELART.starts_with(concat!(
            palette_shader_prologue!(),
            palette_shader_functions!(),
            "mediump vec4 GetPixelArtSample(vec2 uv)\n{\n    return texture2D(u_texture, uv);\n}\n\n\nvoid main()\n"
        )));
        assert_eq!(ShaderType::FragmentSolid.prologue(), "");
        assert!(ShaderType::FragmentTextureExternalOes
            .prologue()
            .starts_with("#extension GL_OES_EGL_image_external : require\n"));
        assert_eq!(ShaderInclude::None.source(), "");
    }

    #[test]
    fn texcoord_precision_hint() {
        let _l = crate::test_support::test_lock();
        const HINT: &str = "SDL_RENDER_OPENGLES2_TEXCOORD_PRECISION";
        assert_eq!(
            ShaderInclude::texcoord_precision_from_hint(),
            ShaderInclude::BestTexcoordPrecision
        );
        for (value, expected) in [
            ("undefined", ShaderInclude::UndefPrecision),
            ("high", ShaderInclude::HighTexcoordPrecision),
            ("medium", ShaderInclude::MediumTexcoordPrecision),
            ("low", ShaderInclude::BestTexcoordPrecision),
        ] {
            hints::set(HINT, value).unwrap();
            assert_eq!(ShaderInclude::texcoord_precision_from_hint(), expected);
        }
        hints::reset(HINT);
    }
}
