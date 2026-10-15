// Rust translation of src/render/metal/SDL_render_metal.m from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Metal renderer ("metal", the first one tried on macOS): draws
//! through a Metal device into the `CAMetalLayer` of a Metal view on the
//! window (the window's own Metal view when it has one, otherwise one made
//! with [`metal_create_view`](crate::video::vulkan::metal_create_view)).
//! The shaders are upstream's precompiled library ([`shaders`]).
//!
//! Textures can wrap the application's `MTLTexture`s
//! ([`TextureCreateInfo::metal_texture`](crate::render::TextureCreateInfo::metal_texture)
//! and the others, `SDL_PROP_TEXTURE_CREATE_METAL_TEXTURE_*`) or the
//! planes of a `CVPixelBuffer`'s `IOSurface`
//! (`SDL_PROP_TEXTURE_CREATE_METAL_PIXELBUFFER_POINTER`).
//!
//! Upstream's `@interface` data objects (`SDL3METAL_RenderData`, the
//! texture and palette data) are Rust structures here, their retained
//! properties [`StrongPtr`]s; its sampler dictionary is a `HashMap`, and its
//! pipeline caches `Vec`s. What isn't translated: the iOS, tvOS and visionOS
//! paths (only macOS is targeted).

mod shaders;
#[cfg(test)]
mod tests;

use std::any::Any;
use std::collections::HashMap;
use std::ffi::c_void;

use crate::core::apple::block::{dispatch_get_global_queue, dispatch_release, RcBlock};
use crate::core::apple::foundation::{macos_at_least, ns_array_items, ns_string, NSSize};
use crate::core::apple::objc::{
    autoreleasepool, class, msg_send, release, Bool, Id, NSInteger, NSUInteger, StrongPtr, NIL, NO,
    YES,
};
use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::EventType;
use crate::gpu::metal::sys::*;
use crate::hints;
use crate::properties::Properties;
use crate::render::sysrender::{
    invalid_texture, CopyEx, DrawCmd, DrawKind, Geometry, GpuRenderStates, RenderBackend,
    RenderCommand, TextureCreateProps, TextureData, TextureStore,
};
use crate::render::{RendererCreateInfo, Texture, TextureAccess, TextureAddressMode};
use crate::video::blendmode::{BlendFactor, BlendOperation};
use crate::video::pixels::{
    convert_color_709_to_2020, srgb_to_linear, Color, Colorspace, FColor, PixelFormat,
    TransferCharacteristics,
};
use crate::video::rect::{FPoint, FRect, Rect};
use crate::video::surface::{ScaleMode, Surface};
use crate::video::window::{
    PROP_WINDOW_COCOA_METAL_VIEW_TAG_NUMBER, PROP_WINDOW_COCOA_WINDOW_POINTER,
    PROP_WINDOW_HDR_HEADROOM_FLOAT,
};
use crate::video::{BlendMode, Window};

/// The name of the Metal renderer (`METAL_RenderDriver.name`).
pub(crate) const METAL_RENDERER: &str = "metal";

/// The `MTLDevice` of the renderer. Translation of
/// `SDL_PROP_RENDERER_METAL_DEVICE_POINTER`.
pub const PROP_RENDERER_METAL_DEVICE_POINTER: &str = "SDL.renderer.metal.device";
/// The `MTLCommandQueue` of the renderer. Translation of
/// `SDL_PROP_RENDERER_METAL_COMMAND_QUEUE_POINTER`.
pub const PROP_RENDERER_METAL_COMMAND_QUEUE_POINTER: &str = "SDL.renderer.metal.command_queue";
/// The `MTLTexture` of a texture (its Y plane for YUV textures).
/// Translation of `SDL_PROP_TEXTURE_METAL_TEXTURE_POINTER`.
pub const PROP_TEXTURE_METAL_TEXTURE_POINTER: &str = "SDL.texture.metal.texture";
/// The `MTLTexture` of the UV plane of an NV12 texture. Translation of
/// `SDL_PROP_TEXTURE_METAL_TEXTURE_UV_POINTER`.
pub const PROP_TEXTURE_METAL_TEXTURE_UV_POINTER: &str = "SDL.texture.metal.texture_uv";
/// The `MTLTexture` of the U plane of a YUV texture. Translation of
/// `SDL_PROP_TEXTURE_METAL_TEXTURE_U_POINTER`.
pub const PROP_TEXTURE_METAL_TEXTURE_U_POINTER: &str = "SDL.texture.metal.texture_u";
/// The `MTLTexture` of the V plane of a YUV texture. Translation of
/// `SDL_PROP_TEXTURE_METAL_TEXTURE_V_POINTER`.
pub const PROP_TEXTURE_METAL_TEXTURE_V_POINTER: &str = "SDL.texture.metal.texture_v";

/// `IOSurfaceRef`.
type IoSurfaceRef = *mut c_void;

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVPixelBufferGetIOSurface(pixel_buffer: *mut c_void) -> IoSurfaceRef;
}

// Apple Metal renderer implementation

/// macOS requires constants in a buffer to have a 256 byte alignment.
/// Translation of `CONSTANT_ALIGN()` (macOS).
const fn constant_align(_x: usize) -> usize {
    256
}

/// Translation of `DEVICE_ALIGN()`.
const fn device_align(x: usize) -> usize {
    if x < 4 {
        4
    } else {
        x
    }
}

/// Translation of `ALIGN_CONSTANTS()`.
const fn align_constants(align: usize, size: usize) -> usize {
    (size + constant_align(align) - 1) & !(constant_align(align) - 1)
}

const FLOAT: usize = size_of::<f32>();

const CONSTANTS_OFFSET_INVALID: usize = 0xFFFFFFFF;
const CONSTANTS_OFFSET_IDENTITY: usize = 0;
const CONSTANTS_OFFSET_HALF_PIXEL_TRANSFORM: usize =
    align_constants(16, CONSTANTS_OFFSET_IDENTITY + FLOAT * 16);
const CONSTANTS_OFFSET_DECODE_BT601_LIMITED: usize =
    align_constants(16, CONSTANTS_OFFSET_HALF_PIXEL_TRANSFORM + FLOAT * 16);
const CONSTANTS_OFFSET_DECODE_BT601_FULL: usize =
    align_constants(16, CONSTANTS_OFFSET_DECODE_BT601_LIMITED + FLOAT * 4 * 4);
const CONSTANTS_OFFSET_DECODE_BT709_LIMITED: usize =
    align_constants(16, CONSTANTS_OFFSET_DECODE_BT601_FULL + FLOAT * 4 * 4);
const CONSTANTS_OFFSET_DECODE_BT709_FULL: usize =
    align_constants(16, CONSTANTS_OFFSET_DECODE_BT709_LIMITED + FLOAT * 4 * 4);
const CONSTANTS_OFFSET_DECODE_BT2020_LIMITED: usize =
    align_constants(16, CONSTANTS_OFFSET_DECODE_BT709_FULL + FLOAT * 4 * 4);
const CONSTANTS_OFFSET_DECODE_BT2020_FULL: usize =
    align_constants(16, CONSTANTS_OFFSET_DECODE_BT2020_LIMITED + FLOAT * 4 * 4);
const CONSTANTS_LENGTH: usize = CONSTANTS_OFFSET_DECODE_BT2020_FULL + FLOAT * 4 * 4;

/// Translation of `SDL_MetalVertexFunction`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VertexFunction {
    Solid,
    Copy,
}

/// Translation of `SDL_MetalFragmentFunction`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FragmentFunction {
    Solid = 0,
    Palette,
    Copy,
    Yuv,
    Nv12,
}

/// `SDL_METAL_FRAGMENT_COUNT`.
const FRAGMENT_COUNT: usize = 5;

/// Translation of `METAL_PipelineState`.
struct PipelineState {
    blend_mode: BlendMode,
    pipe: StrongPtr,
}

/// Translation of `METAL_PipelineCache`.
struct PipelineCache {
    states: Vec<PipelineState>,
    vertex_function: VertexFunction,
    fragment_function: FragmentFunction,
    render_target_format: NSUInteger,
    label: &'static str,
}

/// Each shader combination used by drawing functions has a separate pipeline
/// cache, and we have a separate list of caches for each render target pixel
/// format. This is more efficient than iterating over a global cache to find
/// the pipeline based on the specified shader combination and RT pixel format,
/// since we know what the RT pixel format is when we set the render target, and
/// we know what the shader combination is inside each drawing function's code.
/// Translation of `METAL_ShaderPipelines`.
struct ShaderPipelines {
    render_target_format: NSUInteger,
    caches: [PipelineCache; FRAGMENT_COUNT],
}

/// Per-palette data. Translation of `SDL3METAL_PaletteData`.
struct MetalPalette {
    mtltexture: StrongPtr,
    hasdata: bool,
}

/// Per-texture data. Translation of `SDL3METAL_TextureData`.
struct MetalTexture {
    mtltexture: StrongPtr,
    mtltexture_uv: StrongPtr,
    mtltexture_u: StrongPtr,
    mtltexture_v: StrongPtr,
    fragment_function: FragmentFunction,
    yuv: bool,
    nv12: bool,
    conversion_buffer_offset: usize,
    hasdata: bool,
    lockedbuffer: StrongPtr,
    lockedrect: Rect,
    /// The `mtltexture` of the texture's palette (`texture->palette->internal`),
    /// as the front end last changed it.
    palette: StrongPtr,
}

fn metal_texture(texture: &TextureData) -> Option<&MetalTexture> {
    texture.internal.as_ref()?.downcast_ref::<MetalTexture>()
}

fn metal_texture_mut(texture: &mut TextureData) -> Option<&mut MetalTexture> {
    texture.internal.as_mut()?.downcast_mut::<MetalTexture>()
}

/// The renderer's data. Translation of `SDL3METAL_RenderData`, with the
/// parts of `SDL_Renderer` the backend reads (`window`, the colorspaces,
/// `hidden`, `target`) and its vertex storage (`vertex_data`, which
/// `SDL_AllocateRenderVertices()` fills).
pub(crate) struct MetalRenderer {
    window: Window,
    output_colorspace: Colorspace,
    current_colorspace: Colorspace,
    /// `renderer->hidden`, followed through the window events as the
    /// front end follows them.
    hidden: bool,
    /// `renderer->target`, and the `MTLTexture`, size and HDR headroom of
    /// its texture data.
    target: Option<Texture>,
    target_mtltexture: StrongPtr,
    target_size: (i32, i32),

    mtldevice: StrongPtr,
    mtlcmdqueue: StrongPtr,
    mtlcmdbuffer: StrongPtr,
    mtlcmdencoder: StrongPtr,
    mtllibrary: StrongPtr,
    mtlbackbuffer: StrongPtr,
    mtlsamplers: HashMap<NSInteger, StrongPtr>,
    mtlbufconstants: StrongPtr,
    mtlbufquadindices: StrongPtr,
    /// The `SDL_MetalView`, a reference the renderer owns.
    mtlview: usize,
    mtllayer: StrongPtr,
    mtlpassdesc: StrongPtr,
    /// The index of the active pipelines in `allpipelines`.
    activepipelines: Option<usize>,
    allpipelines: Vec<ShaderPipelines>,

    /// The vertex data of the queued commands (`renderer->vertex_data`).
    vertex_data: Vec<u8>,
    texture_formats: Vec<PixelFormat>,
    /// The renderer's properties, once the front end has made them.
    props: Option<Properties>,
}

/// An invalid blend operation (`invalidBlendOperation`).
const INVALID_BLEND_OPERATION: NSUInteger = 0xFFFFFFFF;
/// An invalid blend factor (`invalidBlendFactor`).
const INVALID_BLEND_FACTOR: NSUInteger = 0xFFFFFFFF;

/// Translation of `GetBlendOperation()`.
fn get_blend_operation(operation: Option<BlendOperation>) -> NSUInteger {
    match operation {
        Some(BlendOperation::Add) => MTL_BLEND_OPERATION_ADD,
        Some(BlendOperation::Subtract) => MTL_BLEND_OPERATION_SUBTRACT,
        Some(BlendOperation::RevSubtract) => MTL_BLEND_OPERATION_REVERSE_SUBTRACT,
        Some(BlendOperation::Minimum) => MTL_BLEND_OPERATION_MIN,
        Some(BlendOperation::Maximum) => MTL_BLEND_OPERATION_MAX,
        None => INVALID_BLEND_OPERATION,
    }
}

/// Translation of `GetBlendFactor()`.
fn get_blend_factor(factor: Option<BlendFactor>) -> NSUInteger {
    match factor {
        Some(BlendFactor::Zero) => MTL_BLEND_FACTOR_ZERO,
        Some(BlendFactor::One) => MTL_BLEND_FACTOR_ONE,
        Some(BlendFactor::SrcColor) => MTL_BLEND_FACTOR_SOURCE_COLOR,
        Some(BlendFactor::OneMinusSrcColor) => MTL_BLEND_FACTOR_ONE_MINUS_SOURCE_COLOR,
        Some(BlendFactor::SrcAlpha) => MTL_BLEND_FACTOR_SOURCE_ALPHA,
        Some(BlendFactor::OneMinusSrcAlpha) => MTL_BLEND_FACTOR_ONE_MINUS_SOURCE_ALPHA,
        Some(BlendFactor::DstColor) => MTL_BLEND_FACTOR_DESTINATION_COLOR,
        Some(BlendFactor::OneMinusDstColor) => MTL_BLEND_FACTOR_ONE_MINUS_DESTINATION_COLOR,
        Some(BlendFactor::DstAlpha) => MTL_BLEND_FACTOR_DESTINATION_ALPHA,
        Some(BlendFactor::OneMinusDstAlpha) => MTL_BLEND_FACTOR_ONE_MINUS_DESTINATION_ALPHA,
        None => INVALID_BLEND_FACTOR,
    }
}

/// Translation of `GetVertexFunctionName()`.
fn get_vertex_function_name(function: VertexFunction) -> &'static str {
    match function {
        VertexFunction::Solid => "SDL_Solid_vertex",
        VertexFunction::Copy => "SDL_Copy_vertex",
    }
}

/// Translation of `GetFragmentFunctionName()`.
fn get_fragment_function_name(function: FragmentFunction) -> &'static str {
    match function {
        FragmentFunction::Solid => "SDL_Solid_fragment",
        FragmentFunction::Palette => "SDL_Palette_fragment",
        FragmentFunction::Copy => "SDL_Copy_fragment",
        FragmentFunction::Yuv => "SDL_YUV_fragment",
        FragmentFunction::Nv12 => "SDL_NV12_fragment",
    }
}

/// `[array objectAtIndexedSubscript:i]` (`array[i]` of the descriptor
/// arrays).
///
/// # Safety
///
/// `array` must be a valid descriptor array.
unsafe fn subscript(array: Id, i: NSUInteger) -> Id {
    // SAFETY: the caller's contract.
    unsafe { msg_send![array, objectAtIndexedSubscript: i] }
}

/// Set an object's `label` (`obj.label = @"..."`).
///
/// # Safety
///
/// `obj` must be nil or a valid object with a `label` property.
unsafe fn set_label(obj: Id, label: &str) {
    let label = ns_string(label);
    // SAFETY: the caller's contract.
    let _: () = unsafe { msg_send![obj, setLabel: label.get()] };
}

/// Translation of `MakePipelineState()`.
fn make_pipeline_state(
    mtldevice: Id,
    mtllibrary: Id,
    cache: &mut PipelineCache,
    blendlabel: &str,
    blendmode: BlendMode,
) -> Id {
    // SAFETY: the renderer's device and library; descriptors made and
    // configured through their documented properties.
    unsafe {
        let vname = ns_string(get_vertex_function_name(cache.vertex_function));
        let fname = ns_string(get_fragment_function_name(cache.fragment_function));
        let mtlvertfn =
            StrongPtr::from_owned(msg_send![mtllibrary, newFunctionWithName: vname.get()]);
        let mtlfragfn =
            StrongPtr::from_owned(msg_send![mtllibrary, newFunctionWithName: fname.get()]);
        crate::sdl_assert!(!mtlvertfn.is_nil());
        crate::sdl_assert!(!mtlfragfn.is_nil());

        let alloc: Id = msg_send![class!(MTLRenderPipelineDescriptor), alloc];
        let mtlpipedesc = StrongPtr::from_owned(msg_send![alloc, init]);
        let _: () = msg_send![mtlpipedesc, setVertexFunction: mtlvertfn.get()];
        let _: () = msg_send![mtlpipedesc, setFragmentFunction: mtlfragfn.get()];

        let vertdesc: Id = msg_send![class!(MTLVertexDescriptor), vertexDescriptor];
        let layouts: Id = msg_send![vertdesc, layouts];
        let attributes: Id = msg_send![vertdesc, attributes];
        let layout0 = subscript(layouts, 0);
        let attribute = |i: NSUInteger, format: NSUInteger, offset: usize| {
            let a = subscript(attributes, i);
            let _: () = msg_send![a, setFormat: format];
            let _: () = msg_send![a, setOffset: offset as NSUInteger];
            let _: () = msg_send![a, setBufferIndex: 0 as NSUInteger];
        };

        match cache.vertex_function {
            VertexFunction::Solid => {
                // position (float2), color (float4)
                let _: () = msg_send![layout0, setStride: (FLOAT * 2 + FLOAT * 4) as NSUInteger];
                let _: () =
                    msg_send![layout0, setStepFunction: MTL_VERTEX_STEP_FUNCTION_PER_VERTEX];

                attribute(0, MTL_VERTEX_FORMAT_FLOAT2, 0);
                attribute(1, MTL_VERTEX_FORMAT_FLOAT4, FLOAT * 2);
            }
            VertexFunction::Copy => {
                // position (float2), color (float4), texcoord (float2)
                let _: () = msg_send![layout0, setStride: (FLOAT * 2 + FLOAT * 4 + FLOAT * 2) as NSUInteger];
                let _: () =
                    msg_send![layout0, setStepFunction: MTL_VERTEX_STEP_FUNCTION_PER_VERTEX];

                attribute(0, MTL_VERTEX_FORMAT_FLOAT2, 0);
                attribute(1, MTL_VERTEX_FORMAT_FLOAT4, FLOAT * 2);
                attribute(2, MTL_VERTEX_FORMAT_FLOAT2, FLOAT * 2 + FLOAT * 4);
            }
        }

        let _: () = msg_send![mtlpipedesc, setVertexDescriptor: vertdesc];

        let attachments: Id = msg_send![mtlpipedesc, colorAttachments];
        let rtdesc = subscript(attachments, 0);
        let _: () = msg_send![rtdesc, setPixelFormat: cache.render_target_format];

        if blendmode != BlendMode::NONE {
            let _: () = msg_send![rtdesc, setBlendingEnabled: YES];
            let _: () = msg_send![rtdesc, setSourceRGBBlendFactor: get_blend_factor(blendmode.src_color_factor())];
            let _: () = msg_send![rtdesc, setDestinationRGBBlendFactor: get_blend_factor(blendmode.dst_color_factor())];
            let _: () = msg_send![rtdesc, setRgbBlendOperation: get_blend_operation(blendmode.color_operation())];
            let _: () = msg_send![rtdesc, setSourceAlphaBlendFactor: get_blend_factor(blendmode.src_alpha_factor())];
            let _: () = msg_send![rtdesc, setDestinationAlphaBlendFactor: get_blend_factor(blendmode.dst_alpha_factor())];
            let _: () = msg_send![rtdesc, setAlphaBlendOperation: get_blend_operation(blendmode.alpha_operation())];
        } else {
            let _: () = msg_send![rtdesc, setBlendingEnabled: NO];
        }

        set_label(mtlpipedesc.get(), &format!("{}{}", cache.label, blendlabel));

        let mut err: Id = NIL;
        let state = StrongPtr::from_owned(
            msg_send![mtldevice, newRenderPipelineStateWithDescriptor: mtlpipedesc.get()
                                                                 error: &mut err as *mut Id],
        );
        crate::sdl_assert!(err.is_null());

        let pipe = state.get();
        cache.states.push(PipelineState {
            blend_mode: blendmode,
            pipe: state,
        });
        pipe
    }
}

/// Translation of `MakePipelineCache()`.
fn make_pipeline_cache(
    mtldevice: Id,
    mtllibrary: Id,
    label: &'static str,
    rtformat: NSUInteger,
    vertfn: VertexFunction,
    fragfn: FragmentFunction,
) -> PipelineCache {
    let mut cache = PipelineCache {
        states: Vec::new(),
        vertex_function: vertfn,
        fragment_function: fragfn,
        render_target_format: rtformat,
        label,
    };

    /* Create pipeline states for the default blend modes. Custom blend modes
     * will be added to the cache on-demand. */
    for (blendlabel, blendmode) in [
        (" (blend=none)", BlendMode::NONE),
        (" (blend=blend)", BlendMode::BLEND),
        (" (blend=add)", BlendMode::ADD),
        (" (blend=mod)", BlendMode::MOD),
        (" (blend=mul)", BlendMode::MUL),
    ] {
        make_pipeline_state(mtldevice, mtllibrary, &mut cache, blendlabel, blendmode);
    }
    cache
}

/// Translation of `MakeShaderPipelines()`.
fn make_shader_pipelines(mtldevice: Id, mtllibrary: Id, rtformat: NSUInteger) -> ShaderPipelines {
    use FragmentFunction as F;
    use VertexFunction as V;
    let make = |label, vertfn, fragfn| {
        make_pipeline_cache(mtldevice, mtllibrary, label, rtformat, vertfn, fragfn)
    };
    ShaderPipelines {
        render_target_format: rtformat,
        caches: [
            make("SDL primitives pipeline", V::Solid, F::Solid),
            make("SDL palette pipeline", V::Copy, F::Palette),
            make("SDL copy pipeline", V::Copy, F::Copy),
            make("SDL YUV pipeline", V::Copy, F::Yuv),
            make("SDL NV12 pipeline", V::Copy, F::Nv12),
        ],
    }
}

/// Translation of `GetBT601ConversionMatrix()`.
fn get_bt601_conversion_matrix(colorspace: Colorspace) -> usize {
    use crate::video::pixels::ColorRange;
    match colorspace.range() {
        ColorRange::Limited | ColorRange::Unknown => CONSTANTS_OFFSET_DECODE_BT601_LIMITED,
        ColorRange::Full => CONSTANTS_OFFSET_DECODE_BT601_FULL,
    }
}

/// Translation of `GetBT709ConversionMatrix()`.
fn get_bt709_conversion_matrix(colorspace: Colorspace) -> usize {
    use crate::video::pixels::ColorRange;
    match colorspace.range() {
        ColorRange::Limited | ColorRange::Unknown => CONSTANTS_OFFSET_DECODE_BT709_LIMITED,
        ColorRange::Full => CONSTANTS_OFFSET_DECODE_BT709_FULL,
    }
}

/// Translation of `GetBT2020ConversionMatrix()`.
fn get_bt2020_conversion_matrix(colorspace: Colorspace) -> usize {
    use crate::video::pixels::ColorRange;
    match colorspace.range() {
        ColorRange::Limited | ColorRange::Unknown => CONSTANTS_OFFSET_DECODE_BT2020_LIMITED,
        ColorRange::Full => CONSTANTS_OFFSET_DECODE_BT2020_FULL,
    }
}

/// The offset of a colorspace's YCbCr to RGB matrix in the constants buffer
/// (0 when there is none). Translation of `GetYCbCRtoRGBConversionMatrix()`.
fn get_ycbcr_to_rgb_conversion_matrix(
    colorspace: Colorspace,
    _w: i32,
    h: i32,
    bits_per_pixel: i32,
) -> usize {
    use crate::video::pixels::MatrixCoefficients;
    const YUV_SD_THRESHOLD: i32 = 576;

    match colorspace.matrix() {
        MatrixCoefficients::Bt470bg | MatrixCoefficients::Bt601 => {
            get_bt601_conversion_matrix(colorspace)
        }

        MatrixCoefficients::Bt709 => get_bt709_conversion_matrix(colorspace),

        MatrixCoefficients::Bt2020Ncl => get_bt2020_conversion_matrix(colorspace),

        MatrixCoefficients::Unspecified => match bits_per_pixel {
            8 => {
                if h <= YUV_SD_THRESHOLD {
                    get_bt601_conversion_matrix(colorspace)
                } else {
                    get_bt709_conversion_matrix(colorspace)
                }
            }
            10 | 16 => get_bt2020_conversion_matrix(colorspace),
            _ => 0,
        },
        _ => 0,
    }
}

/// The 16 floats `SDL_GetYCbCRtoRGBConversionMatrix()` returns for a
/// colorspace: the offsets, then the R, G and B coefficients, each in a
/// row of four.
fn ycbcr_shader_matrix(colorspace: Colorspace, bits_per_pixel: u32) -> [f32; 16] {
    let mut out = [0.0; 16];
    if let Some(matrix) = colorspace.ycbcr_to_rgb_matrix(0, bits_per_pixel) {
        out[..3].copy_from_slice(&matrix.offset);
        for (row, coeff) in matrix.coeff.iter().enumerate() {
            out[4 + 4 * row..7 + 4 * row].copy_from_slice(coeff);
        }
    }
    out
}

// These should mirror the definitions in SDL_shaders_metal.metal
//static const float TONEMAP_NONE = 0;
//static const float TONEMAP_LINEAR = 1;
const TONEMAP_CHROME: f32 = 2.0;

//static const float TEXTURETYPE_NONE = 0;
const TEXTURETYPE_RGB: f32 = 1.0;
const TEXTURETYPE_RGB_PIXELART: f32 = 2.0;
const TEXTURETYPE_PALETTE_NEAREST: f32 = 3.0;
const TEXTURETYPE_PALETTE_LINEAR: f32 = 4.0;
const TEXTURETYPE_PALETTE_PIXELART: f32 = 5.0;
const TEXTURETYPE_NV12: f32 = 6.0;
const TEXTURETYPE_NV21: f32 = 7.0;
const TEXTURETYPE_YUV: f32 = 8.0;

//static const float INPUTTYPE_UNSPECIFIED = 0;
const INPUTTYPE_SRGB: f32 = 1.0;
const INPUTTYPE_SCRGB: f32 = 2.0;
const INPUTTYPE_HDR10: f32 = 3.0;

/// Translation of `PixelShaderConstants`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct PixelShaderConstants {
    sc_rgb_output: f32,
    texture_type: f32,
    input_type: f32,
    color_scale: f32,

    texel_width: f32,
    texel_height: f32,
    texture_width: f32,
    texture_height: f32,

    tonemap_method: f32,
    tonemap_factor1: f32,
    tonemap_factor2: f32,
    sdr_white_point: f32,
}

impl PixelShaderConstants {
    /// The bytes upstream's `SDL_memcmp()` compares.
    fn bits(&self) -> [u32; 12] {
        [
            self.sc_rgb_output,
            self.texture_type,
            self.input_type,
            self.color_scale,
            self.texel_width,
            self.texel_height,
            self.texture_width,
            self.texture_height,
            self.tonemap_method,
            self.tonemap_factor1,
            self.tonemap_factor2,
            self.sdr_white_point,
        ]
        .map(f32::to_bits)
    }
}

/// Translation of `METAL_DrawStateCache`.
struct DrawStateCache {
    pipeline: Id,
    vertex_buffer: Id,
    constants_offset: usize,
    texture: Option<Texture>,
    texture_palette: bool,
    texture_format: PixelFormat,
    texture_scale_mode: Option<ScaleMode>,
    texture_address_mode_u: Option<TextureAddressMode>,
    texture_address_mode_v: Option<TextureAddressMode>,
    cliprect_dirty: bool,
    cliprect_enabled: bool,
    cliprect: Rect,
    viewport_dirty: bool,
    viewport: Rect,
    projection_offset: usize,
    shader_constants_dirty: bool,
    shader_constants: PixelShaderConstants,
}

/// The front end's view of the texture a draw copies from (what upstream
/// reads of `SDL_Texture`).
#[derive(Clone, Copy)]
struct SourceTexture {
    format: PixelFormat,
    w: i32,
    h: i32,
    colorspace: Colorspace,
    sdr_white_point: f32,
    hdr_headroom: f32,
    has_palette: bool,
}

impl SourceTexture {
    fn of(texture: &TextureData) -> SourceTexture {
        SourceTexture {
            format: texture.format,
            w: texture.w,
            h: texture.h,
            colorspace: texture.colorspace,
            sdr_white_point: texture.sdr_white_point,
            hdr_headroom: texture.hdr_headroom,
            has_palette: texture.palette.is_some(),
        }
    }
}

/// Translation of `RENDER_SAMPLER_HASHKEY()` (from `SDL_sysrender.h`).
fn render_sampler_hashkey(
    scale_mode: ScaleMode,
    address_u: TextureAddressMode,
    address_v: TextureAddressMode,
) -> NSInteger {
    ((scale_mode == ScaleMode::Nearest) as NSInteger)
        | (((address_u == TextureAddressMode::Wrap) as NSInteger) << 1)
        | (((address_v == TextureAddressMode::Wrap) as NSInteger) << 2)
}

/// Translation of `METAL_UploadTextureData()`.
///
/// # Safety
///
/// `texture` must be a valid `MTLTexture` and `pixels` hold `rect` rows of
/// `pitch` bytes.
unsafe fn upload_texture_data(
    texture: Id,
    rect: Rect,
    slice: NSUInteger,
    pixels: *const u8,
    pitch: i32,
) {
    // SAFETY: the caller's contract.
    unsafe {
        let _: () = msg_send![texture, replaceRegion: mtl_region_make_2d(rect.x, rect.y, rect.w, rect.h)
                                           mipmapLevel: 0 as NSUInteger
                                                 slice: slice
                                             withBytes: pixels as *const c_void
                                           bytesPerRow: pitch as NSUInteger
                                         bytesPerImage: 0 as NSUInteger];
    }
}

/// Translation of `METAL_GetStorageMode()`.
///
/// # Safety
///
/// `resource` must be a valid `MTLResource`.
unsafe fn get_storage_mode(resource: Id) -> NSUInteger {
    // SAFETY: the caller's contract.
    unsafe { msg_send![resource, storageMode] }
}

/// The bytes of `pixels` from `offset` on, checked to hold `rows` rows of
/// `row_len` bytes `pitch` apart (upstream reads whatever is there).
fn plane(pixels: &[u8], offset: usize, pitch: usize, row_len: usize, rows: usize) -> Result<&[u8]> {
    let needed = if rows == 0 {
        0
    } else {
        (rows - 1) * pitch + row_len
    };
    pixels
        .get(offset..)
        .filter(|p| p.len() >= needed)
        .ok_or_else(|| Error::invalid_param("pixels"))
}

impl MetalRenderer {
    /// Translation of `SDL_RenderingLinearSpace()`.
    fn rendering_linear_space(&self) -> bool {
        self.current_colorspace == Colorspace::SRGB_LINEAR
            || self.current_colorspace == Colorspace::HDR10
    }

    /// Translation of `SDL_ConvertToLinear()`.
    fn convert_to_linear(&self, color: &mut FColor) {
        color.r = srgb_to_linear(color.r);
        color.g = srgb_to_linear(color.g);
        color.b = srgb_to_linear(color.b);
        if self.current_colorspace == Colorspace::HDR10 {
            let [r, g, b] = convert_color_709_to_2020([color.r, color.g, color.b]);
            (color.r, color.g, color.b) = (r, g, b);
        }
    }

    /// `renderer->HDR_headroom`, as `UpdateHDRProperties()` sets it: the
    /// window's for linear output, 1 otherwise.
    fn hdr_headroom(&self) -> f32 {
        if self.output_colorspace == Colorspace::SRGB_LINEAR
            || self.output_colorspace == Colorspace::HDR10
        {
            self.window
                .properties()
                .ok()
                .and_then(|p| p.get_float(PROP_WINDOW_HDR_HEADROOM_FLOAT))
                .unwrap_or(1.0)
        } else {
            1.0
        }
    }

    /// Translation of `SDL_AllocateRenderVertices()`: room for `numbytes`
    /// at an offset aligned to `alignment`, returned with the offset.
    fn allocate_render_vertices(
        &mut self,
        numbytes: usize,
        alignment: usize,
    ) -> (usize, &mut [u8]) {
        let current_offset = self.vertex_data.len();
        let aligner = if alignment != 0 && (current_offset & (alignment - 1)) != 0 {
            alignment - (current_offset & (alignment - 1))
        } else {
            0
        };
        let aligned = current_offset + aligner;
        self.vertex_data.resize(aligned + numbytes, 0);
        (aligned, &mut self.vertex_data[aligned..])
    }

    /// Write floats into vertex data from `SDL_AllocateRenderVertices()`.
    fn write_floats(dst: &mut [u8], floats: &[f32]) {
        for (d, f) in dst.chunks_exact_mut(FLOAT).zip(floats) {
            d.copy_from_slice(&f.to_ne_bytes());
        }
    }

    /// Translation of `ChooseShaderPipelines()`.
    fn choose_shader_pipelines(&mut self, rtformat: NSUInteger) -> Option<usize> {
        if let Some(i) = self
            .allpipelines
            .iter()
            .position(|p| p.render_target_format == rtformat)
        {
            return Some(i);
        }

        let pipelines =
            make_shader_pipelines(self.mtldevice.get(), self.mtllibrary.get(), rtformat);
        self.allpipelines.push(pipelines);
        Some(self.allpipelines.len() - 1)
    }

    /// Translation of `ChoosePipelineState()`.
    fn choose_pipeline_state(
        &mut self,
        pipelines: usize,
        fragfn: FragmentFunction,
        blendmode: BlendMode,
    ) -> Id {
        let (mtldevice, mtllibrary) = (self.mtldevice.get(), self.mtllibrary.get());
        let cache = &mut self.allpipelines[pipelines].caches[fragfn as usize];

        if let Some(state) = cache.states.iter().find(|s| s.blend_mode == blendmode) {
            return state.pipe.get();
        }

        make_pipeline_state(
            mtldevice,
            mtllibrary,
            cache,
            &format!(" (blend=custom 0x{:x})", blendmode.0),
            blendmode,
        )
    }

    /// Translation of `METAL_ActivateRenderCommandEncoder()`.
    fn activate_render_command_encoder(
        &mut self,
        mut load: NSUInteger,
        clear_color: Option<&MTLClearColor>,
        vertex_buffer: Id,
    ) -> bool {
        /* Our SetRenderTarget just signals that the next render operation should
         * set up a new render pass. This is where that work happens. */
        if !self.mtlcmdencoder.is_nil() {
            return true;
        }

        // SAFETY: the renderer's objects, messaged with their methods'
        // signatures.
        unsafe {
            let mut mtltexture: Id = NIL;

            if self.target.is_some() {
                mtltexture = self.target_mtltexture.get();
            } else {
                // Standard rendering path: use CAMetalLayer drawable
                if self.mtlbackbuffer.is_nil() {
                    // The backbuffer's contents aren't guaranteed to persist after
                    // presenting, so we can leave it undefined when loading it.
                    self.mtlbackbuffer = StrongPtr::retain(msg_send![self.mtllayer, nextDrawable]);
                    if load == MTL_LOAD_ACTION_LOAD {
                        load = MTL_LOAD_ACTION_DONT_CARE;
                    }
                }
                if !self.mtlbackbuffer.is_nil() {
                    mtltexture = msg_send![self.mtlbackbuffer, texture];
                }
            }

            let attachments: Id = msg_send![self.mtlpassdesc, colorAttachments];
            let attachment0 = subscript(attachments, 0);
            let _: () = msg_send![attachment0, setTexture: mtltexture];

            /* mtltexture can be nil here if macOS refused to give us a drawable,
            which apparently can happen for minimized windows, etc. */
            if mtltexture.is_null() {
                return false;
            }

            if load == MTL_LOAD_ACTION_CLEAR {
                crate::sdl_assert!(clear_color.is_some());
                if let Some(color) = clear_color {
                    let _: () = msg_send![attachment0, setClearColor: *color];
                }
            }

            let _: () = msg_send![attachment0, setLoadAction: load];

            self.mtlcmdbuffer = StrongPtr::retain(msg_send![self.mtlcmdqueue, commandBuffer]);
            self.mtlcmdencoder = StrongPtr::retain(
                msg_send![self.mtlcmdbuffer, renderCommandEncoderWithDescriptor: self.mtlpassdesc.get()],
            );

            let backbuffer_texture: Id = if self.mtlbackbuffer.is_nil() {
                NIL
            } else {
                msg_send![self.mtlbackbuffer, texture]
            };
            if !self.mtlbackbuffer.is_nil() && mtltexture == backbuffer_texture {
                set_label(self.mtlcmdencoder.get(), "SDL metal renderer backbuffer");
            } else {
                set_label(self.mtlcmdencoder.get(), "SDL metal renderer render target");
            }

            /* Set up buffer bindings for positions, texcoords, and color once here,
             * the offsets are adjusted in the code that uses them. */
            if !vertex_buffer.is_null() {
                let _: () = msg_send![self.mtlcmdencoder, setVertexBuffer: vertex_buffer
                                                                  offset: 0 as NSUInteger
                                                                 atIndex: 0 as NSUInteger];
                let _: () = msg_send![self.mtlcmdencoder, setFragmentBuffer: vertex_buffer
                                                                    offset: 0 as NSUInteger
                                                                   atIndex: 0 as NSUInteger];
            }

            let pixel_format: NSUInteger = msg_send![mtltexture, pixelFormat];
            self.activepipelines = self.choose_shader_pipelines(pixel_format);

            // make sure this has a definite place in the queue. This way it will
            //  execute reliably whether the app tries to make its own command buffers
            //  or whatever. This means we can _always_ batch rendering commands!
            let _: () = msg_send![self.mtlcmdbuffer, enqueue];
        }

        true
    }

    /// Translation of `METAL_GetOutputSize()`.
    fn get_output_size(&self) -> (i32, i32) {
        autoreleasepool(|| {
            // SAFETY: the renderer's layer.
            let size: NSSize = unsafe { msg_send![self.mtllayer, drawableSize] };
            (size.width as i32, size.height as i32)
        })
    }

    /// Translation of `METAL_UpdateTextureInternal()`.
    fn update_texture_internal(
        &mut self,
        hasdata: bool,
        texture: Id,
        rect: Rect,
        slice: NSUInteger,
        pixels: &[u8],
        pitch: i32,
    ) -> Result<()> {
        let stagingrect = Rect::new(0, 0, rect.w, rect.h);

        // SAFETY: the renderer's objects and a texture of it; `pixels` holds
        // the rows (the callers check it).
        unsafe {
            /* If the texture is managed or shared and this is the first upload, we can
             * use replaceRegion to upload to it directly. Otherwise we upload the data
             * to a staging texture and copy that over. */
            if !hasdata && get_storage_mode(texture) != MTL_STORAGE_MODE_PRIVATE {
                upload_texture_data(texture, rect, slice, pixels.as_ptr(), pitch);
                return Ok(());
            }

            let pixel_format: NSUInteger = msg_send![texture, pixelFormat];
            let desc: Id = msg_send![class!(MTLTextureDescriptor),
                texture2DDescriptorWithPixelFormat: pixel_format
                                             width: rect.w as NSUInteger
                                            height: rect.h as NSUInteger
                                         mipmapped: NO];

            if desc.is_null() {
                return Err(Error::out_of_memory());
            }

            /* TODO: We could have a pool of textures or a MTLHeap we allocate from,
             * and release a staging texture back to the pool in the command buffer's
             * completion handler. */
            let stagingtex =
                StrongPtr::from_owned(msg_send![self.mtldevice, newTextureWithDescriptor: desc]);
            if stagingtex.is_nil() {
                return Err(Error::out_of_memory());
            }

            upload_texture_data(stagingtex.get(), stagingrect, 0, pixels.as_ptr(), pitch);

            if !self.mtlcmdencoder.is_nil() {
                let _: () = msg_send![self.mtlcmdencoder, endEncoding];
                self.mtlcmdencoder = StrongPtr::nil();
            }

            if self.mtlcmdbuffer.is_nil() {
                self.mtlcmdbuffer = StrongPtr::retain(msg_send![self.mtlcmdqueue, commandBuffer]);
            }

            let blitcmd: Id = msg_send![self.mtlcmdbuffer, blitCommandEncoder];

            let _: () = msg_send![blitcmd, copyFromTexture: stagingtex.get()
                                               sourceSlice: 0 as NSUInteger
                                               sourceLevel: 0 as NSUInteger
                                              sourceOrigin: mtl_origin_make(0, 0, 0)
                                                sourceSize: mtl_size_make(rect.w as u32, rect.h as u32, 1)
                                                 toTexture: texture
                                          destinationSlice: slice
                                          destinationLevel: 0 as NSUInteger
                                         destinationOrigin: mtl_origin_make(rect.x as u32, rect.y as u32, 0)];

            let _: () = msg_send![blitcmd, endEncoding];

            /* TODO: This isn't very efficient for the YUV formats, which call
             * UpdateTextureInternal multiple times in a row. */
            let _: () = msg_send![self.mtlcmdbuffer, commit];
            self.mtlcmdbuffer = StrongPtr::nil();
        }

        Ok(())
    }

    /// Translation of `SetupShaderConstants()`.
    fn setup_shader_constants(
        &self,
        cmd: &DrawCmd,
        texture: Option<&SourceTexture>,
        output_headroom: f32,
    ) -> PixelShaderConstants {
        use PixelFormat as F;
        let mut constants = PixelShaderConstants {
            sc_rgb_output: self.rendering_linear_space() as i32 as f32,
            color_scale: cmd.color_scale,
            ..Default::default()
        };

        if let Some(texture) = texture {
            match texture.format {
                F::INDEX8 => match cmd.texture_scale_mode {
                    ScaleMode::Nearest => constants.texture_type = TEXTURETYPE_PALETTE_NEAREST,
                    ScaleMode::Linear => constants.texture_type = TEXTURETYPE_PALETTE_LINEAR,
                    ScaleMode::PixelArt => constants.texture_type = TEXTURETYPE_PALETTE_PIXELART,
                },
                F::YV12 | F::IYUV | F::I444 | F::I0FL | F::I4FL => {
                    constants.texture_type = TEXTURETYPE_YUV;
                }
                F::NV12 => constants.texture_type = TEXTURETYPE_NV12,
                F::NV21 => constants.texture_type = TEXTURETYPE_NV21,
                F::P010 => constants.texture_type = TEXTURETYPE_NV12,
                _ => {
                    if cmd.texture_scale_mode == ScaleMode::PixelArt {
                        constants.texture_type = TEXTURETYPE_RGB_PIXELART;
                    } else {
                        constants.texture_type = TEXTURETYPE_RGB;
                    }
                }
            }

            match texture.colorspace.transfer() {
                TransferCharacteristics::Linear => constants.input_type = INPUTTYPE_SCRGB,
                TransferCharacteristics::Pq => constants.input_type = INPUTTYPE_HDR10,
                _ => constants.input_type = INPUTTYPE_SRGB,
            }

            if constants.texture_type == TEXTURETYPE_PALETTE_LINEAR
                || constants.texture_type == TEXTURETYPE_PALETTE_PIXELART
                || constants.texture_type == TEXTURETYPE_RGB_PIXELART
            {
                constants.texture_width = texture.w as f32;
                constants.texture_height = texture.h as f32;
                constants.texel_width = 1.0 / constants.texture_width;
                constants.texel_height = 1.0 / constants.texture_height;
            }

            constants.sdr_white_point = texture.sdr_white_point;

            if texture.hdr_headroom > output_headroom && output_headroom > 0.0 {
                constants.tonemap_method = TONEMAP_CHROME;
                constants.tonemap_factor1 =
                    output_headroom / (texture.hdr_headroom * texture.hdr_headroom);
                constants.tonemap_factor2 = 1.0 / output_headroom;
            }
        }
        constants
    }

    /// The HDR headroom of the output (`renderer->target->HDR_headroom` or
    /// `renderer->HDR_headroom`).
    fn output_headroom(&self, textures: &TextureStore) -> f32 {
        match self.target.and_then(|t| textures.get(t)) {
            Some(target) => target.hdr_headroom,
            None => self.hdr_headroom(),
        }
    }

    /// Translation of `SetDrawState()`.
    #[allow(clippy::too_many_arguments)]
    fn set_draw_state(
        &mut self,
        cmd: &DrawCmd,
        shader: FragmentFunction,
        shader_constants: Option<&PixelShaderConstants>,
        constants_offset: usize,
        mtlbufvertex: Id,
        statecache: &mut DrawStateCache,
    ) -> Result<bool> {
        let blend = cmd.blend;
        let first = cmd.first;

        if !self.activate_render_command_encoder(
            MTL_LOAD_ACTION_LOAD,
            None,
            statecache.vertex_buffer,
        ) {
            return Ok(false);
        }

        // SAFETY: the active encoder, messaged with its methods' signatures.
        unsafe {
            let encoder = self.mtlcmdencoder.get();

            if statecache.viewport_dirty {
                let viewport = MTLViewport {
                    origin_x: statecache.viewport.x as f64,
                    origin_y: statecache.viewport.y as f64,
                    width: statecache.viewport.w as f64,
                    height: statecache.viewport.h as f64,
                    znear: 0.0,
                    zfar: 1.0,
                };
                let _: () = msg_send![encoder, setViewport: viewport];
                let _: () = msg_send![encoder, setVertexBuffer: mtlbufvertex
                                                        offset: statecache.projection_offset as NSUInteger
                                                       atIndex: 2 as NSUInteger]; // projection
                statecache.viewport_dirty = false;
            }

            if statecache.cliprect_dirty {
                let mut clip = if statecache.cliprect_enabled {
                    let mut clip = statecache.cliprect;
                    clip.x += statecache.viewport.x;
                    clip.y += statecache.viewport.y;
                    clip
                } else {
                    statecache.viewport
                };

                // Set Scissor Rect Validation: w/h must be <= render pass
                let mut output = Rect::default();

                if self.target.is_some() {
                    (output.w, output.h) = self.target_size;
                } else {
                    (output.w, output.h) = self.get_output_size();
                }

                // Note (upstream): SDL_GetRectIntersection() leaves a
                // negative width or height in its result when the clip lies
                // outside the output, which becomes a huge scissor; here the
                // scissor is empty then.
                clip = output.intersection(&clip).unwrap_or_default();

                let mtlrect = MTLScissorRect {
                    x: clip.x as NSUInteger,
                    y: clip.y as NSUInteger,
                    width: clip.w as NSUInteger,
                    height: clip.h as NSUInteger,
                };
                let _: () = msg_send![encoder, setScissorRect: mtlrect];

                statecache.cliprect_dirty = false;
            }

            let pipelines = self.activepipelines.unwrap_or(0);
            let newpipeline = self.choose_pipeline_state(pipelines, shader, blend);
            if newpipeline != statecache.pipeline {
                let _: () = msg_send![encoder, setRenderPipelineState: newpipeline];
                statecache.pipeline = newpipeline;
            }

            let solid_constants;
            let shader_constants = match shader_constants {
                Some(c) => c,
                None => {
                    solid_constants = self.setup_shader_constants(cmd, None, 0.0);
                    &solid_constants
                }
            };

            if statecache.shader_constants_dirty
                || shader_constants.bits() != statecache.shader_constants.bits()
            {
                let size = size_of::<PixelShaderConstants>();
                let mtlbufconstants = StrongPtr::from_owned(
                    msg_send![self.mtldevice, newBufferWithLength: size as NSUInteger
                                                         options: MTL_RESOURCE_STORAGE_MODE_SHARED],
                );
                if mtlbufconstants.is_nil() {
                    return Err(Error::out_of_memory());
                }
                set_label(mtlbufconstants.get(), "SDL shader constants data");
                let contents: *mut c_void = msg_send![mtlbufconstants, contents];
                std::ptr::copy_nonoverlapping(
                    shader_constants as *const PixelShaderConstants as *const u8,
                    contents as *mut u8,
                    size,
                );
                let _: () = msg_send![encoder, setFragmentBuffer: mtlbufconstants.get()
                                                          offset: 0 as NSUInteger
                                                         atIndex: 0 as NSUInteger];

                statecache.shader_constants = *shader_constants;
                statecache.shader_constants_dirty = false;
            }

            if constants_offset != statecache.constants_offset {
                if constants_offset != CONSTANTS_OFFSET_INVALID {
                    let _: () = msg_send![encoder, setVertexBuffer: self.mtlbufconstants.get()
                                                            offset: constants_offset as NSUInteger
                                                           atIndex: 3 as NSUInteger];
                }
                statecache.constants_offset = constants_offset;
            }

            let _: () = msg_send![encoder, setVertexBufferOffset: first as NSUInteger
                                                         atIndex: 0 as NSUInteger];
            // position/texcoords
        }
        Ok(true)
    }

    /// Translation of `GetSampler()`.
    fn get_sampler(
        &mut self,
        format: PixelFormat,
        mut scale_mode: ScaleMode,
        address_u: TextureAddressMode,
        address_v: TextureAddressMode,
    ) -> Result<Id> {
        if format == PixelFormat::INDEX8 {
            // We'll do linear sampling in the shader if needed
            scale_mode = ScaleMode::Nearest;
        }

        let key = render_sampler_hashkey(scale_mode, address_u, address_v);
        if let Some(sampler) = self.mtlsamplers.get(&key) {
            return Ok(sampler.get());
        }

        // SAFETY: a sampler descriptor configured through its properties,
        // and the renderer's device.
        unsafe {
            let alloc: Id = msg_send![class!(MTLSamplerDescriptor), alloc];
            let samplerdesc = StrongPtr::from_owned(msg_send![alloc, init]);
            let filter = match scale_mode {
                ScaleMode::Nearest => MTL_SAMPLER_MIN_MAG_FILTER_NEAREST,
                // (SDL_SCALEMODE_PIXELART uses linear sampling)
                ScaleMode::PixelArt | ScaleMode::Linear => MTL_SAMPLER_MIN_MAG_FILTER_LINEAR,
            };
            let _: () = msg_send![samplerdesc, setMinFilter: filter];
            let _: () = msg_send![samplerdesc, setMagFilter: filter];
            let address = |mode| match mode {
                TextureAddressMode::Clamp => Ok(MTL_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE),
                TextureAddressMode::Wrap => Ok(MTL_SAMPLER_ADDRESS_MODE_REPEAT),
                _ => Err(Error::new(format!(
                    "Unknown texture address mode: {}",
                    mode as i32
                ))),
            };
            let _: () = msg_send![samplerdesc, setSAddressMode: address(address_u)?];
            let _: () = msg_send![samplerdesc, setTAddressMode: address(address_v)?];
            let mtlsampler = StrongPtr::from_owned(
                msg_send![self.mtldevice, newSamplerStateWithDescriptor: samplerdesc.get()],
            );
            if mtlsampler.is_nil() {
                return Err(Error::new("Couldn't create sampler"));
            }
            let sampler = mtlsampler.get();
            self.mtlsamplers.insert(key, mtlsampler);
            Ok(sampler)
        }
    }

    /// Translation of `SetCopyState()`.
    fn set_copy_state(
        &mut self,
        cmd: &DrawCmd,
        textures: &TextureStore,
        constants_offset: usize,
        mtlbufvertex: Id,
        statecache: &mut DrawStateCache,
    ) -> Result<bool> {
        let handle = cmd.texture.ok_or_else(invalid_texture)?;
        let texture = textures.get(handle).ok_or_else(invalid_texture)?;
        let source = SourceTexture::of(texture);
        let texturedata = metal_texture(texture).ok_or_else(invalid_texture)?;
        let (fragment_function, yuv, nv12, conversion_buffer_offset) = (
            texturedata.fragment_function,
            texturedata.yuv,
            texturedata.nv12,
            texturedata.conversion_buffer_offset,
        );
        let planes = [
            texturedata.mtltexture.get(),
            texturedata.palette.get(),
            texturedata.mtltexture_u.get(),
            texturedata.mtltexture_v.get(),
            texturedata.mtltexture_uv.get(),
        ];

        let output_headroom = self.output_headroom(textures);
        let constants = self.setup_shader_constants(cmd, Some(&source), output_headroom);

        if !self.set_draw_state(
            cmd,
            fragment_function,
            Some(&constants),
            constants_offset,
            mtlbufvertex,
            statecache,
        )? {
            return Ok(false);
        }

        // SAFETY: the active encoder and the texture's objects.
        unsafe {
            let encoder = self.mtlcmdencoder.get();
            let [mtltexture, palette, mtltexture_u, mtltexture_v, mtltexture_uv] = planes;
            if Some(handle) != statecache.texture {
                let _: () =
                    msg_send![encoder, setFragmentTexture: mtltexture atIndex: 0 as NSUInteger];
                if source.has_palette {
                    let _: () =
                        msg_send![encoder, setFragmentTexture: palette atIndex: 1 as NSUInteger];
                }
                if yuv || nv12 {
                    if yuv {
                        let _: () = msg_send![encoder, setFragmentTexture: mtltexture_u atIndex: 1 as NSUInteger];
                        let _: () = msg_send![encoder, setFragmentTexture: mtltexture_v atIndex: 2 as NSUInteger];
                    } else {
                        let _: () = msg_send![encoder, setFragmentTexture: mtltexture_uv atIndex: 1 as NSUInteger];
                    }
                    let _: () = msg_send![encoder, setFragmentBuffer: self.mtlbufconstants.get()
                                                              offset: conversion_buffer_offset as NSUInteger
                                                             atIndex: 1 as NSUInteger];
                }
                statecache.texture = Some(handle);
            }

            if source.format != statecache.texture_format
                || Some(cmd.texture_scale_mode) != statecache.texture_scale_mode
                || Some(cmd.texture_address_mode_u) != statecache.texture_address_mode_u
                || Some(cmd.texture_address_mode_v) != statecache.texture_address_mode_v
            {
                let mtlsampler = self.get_sampler(
                    source.format,
                    cmd.texture_scale_mode,
                    cmd.texture_address_mode_u,
                    cmd.texture_address_mode_v,
                )?;
                let _: () = msg_send![encoder, setFragmentSamplerState: mtlsampler atIndex: 0 as NSUInteger];

                statecache.texture_format = source.format;
                statecache.texture_scale_mode = Some(cmd.texture_scale_mode);
                statecache.texture_address_mode_u = Some(cmd.texture_address_mode_u);
                statecache.texture_address_mode_v = Some(cmd.texture_address_mode_v);
            }
            if source.has_palette {
                if !statecache.texture_palette {
                    let mtlsampler = self.get_sampler(
                        PixelFormat::UNKNOWN,
                        ScaleMode::Nearest,
                        TextureAddressMode::Clamp,
                        TextureAddressMode::Clamp,
                    )?;
                    let _: () = msg_send![encoder, setFragmentSamplerState: mtlsampler atIndex: 1 as NSUInteger];
                    statecache.texture_palette = true;
                }
            } else {
                statecache.texture_palette = false;
            }
        }
        Ok(true)
    }

    /// `[encoder drawPrimitives:type vertexStart:0 vertexCount:count]`.
    fn draw_primitives(&self, primitive_type: NSUInteger, count: usize) {
        // SAFETY: the active encoder.
        unsafe {
            let _: () = msg_send![self.mtlcmdencoder, drawPrimitives: primitive_type
                                                         vertexStart: 0 as NSUInteger
                                                         vertexCount: count as NSUInteger];
        }
    }

    /// Translation of `METAL_RunCommandQueue()`.
    fn run_commands(&mut self, cmds: &[RenderCommand], textures: &TextureStore) -> Result<()> {
        let mut mtlbufvertex = StrongPtr::nil();
        let mut statecache = DrawStateCache {
            pipeline: NIL,
            vertex_buffer: NIL,
            constants_offset: CONSTANTS_OFFSET_INVALID,
            texture: None,
            texture_palette: false,
            texture_format: PixelFormat::UNKNOWN,
            texture_scale_mode: None,
            texture_address_mode_u: None,
            texture_address_mode_v: None,
            cliprect_dirty: true,
            cliprect_enabled: false,
            cliprect: Rect::default(),
            viewport_dirty: true,
            viewport: Rect::default(),
            projection_offset: 0,
            shader_constants_dirty: true,
            shader_constants: PixelShaderConstants::default(),
        };

        // SAFETY: the renderer's objects, messaged with their methods'
        // signatures; the vertex buffer holds `vertsize` bytes.
        unsafe {
            // !!! FIXME: have a ring of pre-made MTLBuffers we cycle through? How expensive is creation?
            let vertsize = self.vertex_data.len();
            if vertsize > 0 {
                /* We can memcpy to a shared buffer from the CPU and read it from the GPU
                 * without any extra copying. It's a bit slower on macOS to read shared
                 * data from the GPU than to read managed/private data, but we avoid the
                 * cost of copying the data and the code's simpler. Apple's best
                 * practices guide recommends this approach for streamed vertex data.
                 */
                mtlbufvertex = StrongPtr::from_owned(
                    msg_send![self.mtldevice, newBufferWithLength: vertsize as NSUInteger
                                                         options: MTL_RESOURCE_STORAGE_MODE_SHARED],
                );
                if mtlbufvertex.is_nil() {
                    return Err(Error::out_of_memory());
                }
                set_label(mtlbufvertex.get(), "SDL vertex data");
                let contents: *mut c_void = msg_send![mtlbufvertex, contents];
                std::ptr::copy_nonoverlapping(
                    self.vertex_data.as_ptr(),
                    contents as *mut u8,
                    vertsize,
                );

                statecache.vertex_buffer = mtlbufvertex.get();
            }

            // If there's a command buffer here unexpectedly (app requested one?). Commit it so we can start fresh.
            let _: () = msg_send![self.mtlcmdencoder, endEncoding];
            let _: () = msg_send![self.mtlcmdbuffer, commit];
            self.mtlcmdencoder = StrongPtr::nil();
            self.mtlcmdbuffer = StrongPtr::nil();
        }
        let mtlbufvertex = mtlbufvertex.get();

        let mut i = 0;
        while i < cmds.len() {
            match cmds[i] {
                RenderCommand::SetViewport { first, rect } => {
                    statecache.viewport = rect;
                    statecache.projection_offset = first;
                    statecache.viewport_dirty = true;
                    statecache.cliprect_dirty = true;
                }

                RenderCommand::SetClipRect { enabled, rect } => {
                    statecache.cliprect = rect;
                    statecache.cliprect_enabled = enabled;
                    statecache.cliprect_dirty = true;
                }

                RenderCommand::SetDrawColor { .. } => {}

                RenderCommand::Clear {
                    mut color,
                    color_scale,
                    ..
                } => {
                    /* If we're already encoding a command buffer, dump it without committing it. We'd just
                    clear all its work anyhow, and starting a new encoder will let us use a hardware clear
                    operation via MTLLoadActionClear. */
                    if !self.mtlcmdencoder.is_nil() {
                        // SAFETY: the active encoder and its command buffer.
                        unsafe {
                            let _: () = msg_send![self.mtlcmdencoder, endEncoding];

                            // !!! FIXME: have to commit, or an uncommitted but enqueued buffer will prevent the frame from finishing.
                            let _: () = msg_send![self.mtlcmdbuffer, commit];
                        }
                        self.mtlcmdencoder = StrongPtr::nil();
                        self.mtlcmdbuffer = StrongPtr::nil();
                    }

                    // force all this state to be reconfigured on next command buffer.
                    statecache.pipeline = NIL;
                    statecache.constants_offset = CONSTANTS_OFFSET_INVALID;
                    statecache.texture = None;
                    statecache.shader_constants_dirty = true;
                    statecache.cliprect_dirty = true;
                    statecache.viewport_dirty = true;

                    if self.rendering_linear_space() {
                        self.convert_to_linear(&mut color);
                    }
                    color.r *= color_scale;
                    color.g *= color_scale;
                    color.b *= color_scale;
                    let mtlcolor = MTLClearColor {
                        red: color.r as f64,
                        green: color.g as f64,
                        blue: color.b as f64,
                        alpha: color.a as f64,
                    };

                    // get new command encoder, set up with an initial clear operation.
                    // (this might fail, and future draw operations will notice.)
                    self.activate_render_command_encoder(
                        MTL_LOAD_ACTION_CLEAR,
                        Some(&mtlcolor),
                        mtlbufvertex,
                    );
                }

                RenderCommand::Draw(DrawKind::Lines, d) => {
                    if self.set_draw_state(
                        &d,
                        FragmentFunction::Solid,
                        None,
                        CONSTANTS_OFFSET_HALF_PIXEL_TRANSFORM,
                        mtlbufvertex,
                        &mut statecache,
                    )? {
                        let mut count = d.count;
                        if count > 2 {
                            // joined lines cannot be grouped
                            self.draw_primitives(MTL_PRIMITIVE_TYPE_LINE_STRIP, count);
                        } else {
                            // let's group non joined lines
                            let mut finalcmd = i;
                            let thiscolorscale = d.color_scale;
                            let thisblend = d.blend;

                            for (j, next) in cmds.iter().enumerate().skip(i + 1) {
                                match next {
                                    RenderCommand::Draw(DrawKind::Lines, n) => {
                                        if n.count != 2 {
                                            break; // can't go any further on this draw call, those are joined lines
                                        } else if n.blend != thisblend
                                            || n.color_scale != thiscolorscale
                                        {
                                            break; // can't go any further on this draw call, different blendmode copy up next.
                                        } else {
                                            finalcmd = j; // we can combine copy operations here. Mark this one as the furthest okay command.
                                            count += n.count;
                                        }
                                    }
                                    RenderCommand::SetDrawColor { .. } => {
                                        // The vertex data has the draw color built in, ignore this
                                        continue;
                                    }
                                    _ => break, // can't go any further on this draw call, different render command up next.
                                }
                            }

                            self.draw_primitives(MTL_PRIMITIVE_TYPE_LINE, count);
                            i = finalcmd; // skip any copy commands we just combined in here.
                        }
                    }
                }

                RenderCommand::Draw(DrawKind::FillRects | DrawKind::Copy | DrawKind::CopyEx, _) => {
                    // unused
                }

                RenderCommand::Draw(thiscmdtype @ (DrawKind::Points | DrawKind::Geometry), d) => {
                    let mut finalcmd = i;
                    let mut count = d.count;
                    for (j, next) in cmds.iter().enumerate().skip(i + 1) {
                        match next {
                            RenderCommand::Draw(nextcmdtype, n) if *nextcmdtype == thiscmdtype => {
                                if n.texture != d.texture
                                    || n.texture_scale_mode != d.texture_scale_mode
                                    || n.texture_address_mode_u != d.texture_address_mode_u
                                    || n.texture_address_mode_v != d.texture_address_mode_v
                                    || n.blend != d.blend
                                    || n.color_scale != d.color_scale
                                {
                                    break; // can't go any further on this draw call, different texture/blendmode copy up next.
                                } else {
                                    finalcmd = j; // we can combine copy operations here. Mark this one as the furthest okay command.
                                    count += n.count;
                                }
                            }
                            RenderCommand::SetDrawColor { .. } => {
                                // The vertex data has the draw color built in, ignore this
                                continue;
                            }
                            _ => break, // can't go any further on this draw call, different render command up next.
                        }
                    }

                    if thiscmdtype == DrawKind::Geometry {
                        if d.texture.is_some() {
                            if self.set_copy_state(
                                &d,
                                textures,
                                CONSTANTS_OFFSET_IDENTITY,
                                mtlbufvertex,
                                &mut statecache,
                            )? {
                                self.draw_primitives(MTL_PRIMITIVE_TYPE_TRIANGLE, count);
                            }
                        } else if self.set_draw_state(
                            &d,
                            FragmentFunction::Solid,
                            None,
                            CONSTANTS_OFFSET_IDENTITY,
                            mtlbufvertex,
                            &mut statecache,
                        )? {
                            self.draw_primitives(MTL_PRIMITIVE_TYPE_TRIANGLE, count);
                        }
                    } else if self.set_draw_state(
                        &d,
                        FragmentFunction::Solid,
                        None,
                        CONSTANTS_OFFSET_HALF_PIXEL_TRANSFORM,
                        mtlbufvertex,
                        &mut statecache,
                    )? {
                        self.draw_primitives(MTL_PRIMITIVE_TYPE_POINT, count);
                    }
                    i = finalcmd; // skip any copy commands we just combined in here.
                }

                RenderCommand::NoOp => {}
            }
            i += 1;
        }

        Ok(())
    }

    /// Translation of `METAL_RenderReadPixels()`.
    fn render_read_pixels(&mut self, rect: &Rect) -> Result<Surface<'static>> {
        let read_rect = *rect;

        if !self.activate_render_command_encoder(MTL_LOAD_ACTION_LOAD, None, NIL) {
            return Err(Error::new(
                "Failed to activate render command encoder (is your window in the background?",
            ));
        }

        // SAFETY: the active encoder and command buffer, and the texture
        // they render to; the surface holds `read_rect`'s pixels.
        unsafe {
            let _: () = msg_send![self.mtlcmdencoder, endEncoding];
            let attachments: Id = msg_send![self.mtlpassdesc, colorAttachments];
            let mtltexture = StrongPtr::retain(msg_send![subscript(attachments, 0), texture]);

            /* on macOS with managed-storage textures, we need to tell the driver to
             * update the CPU-side copy of the texture data.
             * NOTE: Currently all of our textures are managed on macOS. We'll need some
             * extra copying for any private textures. */
            if get_storage_mode(mtltexture.get()) == MTL_STORAGE_MODE_MANAGED {
                let blit: Id = msg_send![self.mtlcmdbuffer, blitCommandEncoder];
                let _: () = msg_send![blit, synchronizeResource: mtltexture.get()];
                let _: () = msg_send![blit, endEncoding];
            }

            /* Commit the current command buffer and wait until it's completed, to make
             * sure the GPU has finished rendering to it by the time we read it. */
            let _: () = msg_send![self.mtlcmdbuffer, commit];
            let _: () = msg_send![self.mtlcmdbuffer, waitUntilCompleted];
            self.mtlcmdencoder = StrongPtr::nil();
            self.mtlcmdbuffer = StrongPtr::nil();

            let mtlregion = mtl_region_make_2d(read_rect.x, read_rect.y, read_rect.w, read_rect.h);

            let pixel_format: NSUInteger = msg_send![mtltexture, pixelFormat];
            let format = match pixel_format {
                MTL_PIXEL_FORMAT_BGRA8_UNORM | MTL_PIXEL_FORMAT_BGRA8_UNORM_SRGB => {
                    PixelFormat::ARGB8888
                }
                MTL_PIXEL_FORMAT_RGBA8_UNORM | MTL_PIXEL_FORMAT_RGBA8_UNORM_SRGB => {
                    PixelFormat::ABGR8888
                }
                MTL_PIXEL_FORMAT_RGB10A2_UNORM => PixelFormat::ABGR2101010,
                MTL_PIXEL_FORMAT_RGBA16_FLOAT => PixelFormat::RGBA64_FLOAT,
                MTL_PIXEL_FORMAT_RGBA32_FLOAT => PixelFormat::RGBA128_FLOAT,
                MTL_PIXEL_FORMAT_B5G6R5_UNORM => PixelFormat::RGB565,
                MTL_PIXEL_FORMAT_A1BGR5_UNORM => PixelFormat::RGBA5551,
                MTL_PIXEL_FORMAT_BGR5A1_UNORM => PixelFormat::ARGB1555,
                MTL_PIXEL_FORMAT_ABGR4_UNORM => PixelFormat::RGBA4444,
                _ => return Err(Error::new("Unknown framebuffer pixel format")),
            };
            let mut surface = Surface::new_uninitialized(read_rect.w, read_rect.h, format)?;
            let pitch = surface.pitch();
            if let Some(pixels) = surface.pixels_mut() {
                let _: () = msg_send![mtltexture, getBytes: pixels.as_mut_ptr() as *mut c_void
                                              bytesPerRow: pitch as NSUInteger
                                               fromRegion: mtlregion
                                              mipmapLevel: 0 as NSUInteger];
            }
            if self.rendering_linear_space() && !format.is_10bit() && !format.is_float() {
                let (w, h) = (surface.width(), surface.height());
                if let Some(pixels) = surface.pixels_mut() {
                    // (upstream converts in place)
                    let src = pixels.to_vec();
                    crate::video::surface::convert_pixels_and_colorspace(
                        w,
                        h,
                        format,
                        Colorspace::SRGB_LINEAR,
                        None,
                        &src,
                        pitch,
                        format,
                        Colorspace::SRGB,
                        None,
                        pixels,
                        pitch,
                    )?;
                }
            }
            Ok(surface)
        }
    }

    /// Translation of `METAL_RenderPresent()`.
    fn render_present(&mut self) -> bool {
        autoreleasepool(|| {
            let mut ready = true;

            // If we don't have a command buffer, we can't present, so activate to get one.
            if self.mtlcmdencoder.is_nil() {
                // We haven't even gotten a backbuffer yet? Load and clear it. Otherwise, load the existing data.
                if self.mtlbackbuffer.is_nil() {
                    let transparent = self
                        .window
                        .flags()
                        .unwrap_or_default()
                        .contains(WindowFlags::TRANSPARENT);
                    let alpha = if transparent { 0.0 } else { 1.0 };
                    let color = MTLClearColor {
                        red: 0.0,
                        green: 0.0,
                        blue: 0.0,
                        alpha,
                    };
                    ready = self.activate_render_command_encoder(
                        MTL_LOAD_ACTION_CLEAR,
                        Some(&color),
                        NIL,
                    );
                } else {
                    ready = self.activate_render_command_encoder(MTL_LOAD_ACTION_LOAD, None, NIL);
                }
            }

            // SAFETY: the renderer's encoder, command buffer and drawable
            // (nil-safe messages when there are none).
            unsafe {
                let _: () = msg_send![self.mtlcmdencoder, endEncoding];

                // If we don't have a drawable to present, don't try to present it.
                //  But we'll still try to commit the command buffer in case it was already enqueued.
                if ready {
                    crate::sdl_assert!(!self.mtlbackbuffer.is_nil());
                    let _: () =
                        msg_send![self.mtlcmdbuffer, presentDrawable: self.mtlbackbuffer.get()];
                }

                let _: () = msg_send![self.mtlcmdbuffer, commit];
            }

            self.mtlcmdencoder = StrongPtr::nil();
            self.mtlcmdbuffer = StrongPtr::nil();
            self.mtlbackbuffer = StrongPtr::nil();

            !(self.hidden || !ready)
        })
    }

    /// Translation of `METAL_InvalidateCachedState()`.
    fn invalidate(&mut self) {
        autoreleasepool(|| {
            // METAL_DrawStateCache only exists during a run of METAL_RunCommandQueue, so there's nothing to invalidate!

            // Make sure any queued drawing is submitted before returning to the application
            // SAFETY: the renderer's encoder and command buffer.
            unsafe {
                if !self.mtlcmdencoder.is_nil() {
                    let _: () = msg_send![self.mtlcmdencoder, endEncoding];
                    self.mtlcmdencoder = StrongPtr::nil();
                }

                if !self.mtlcmdbuffer.is_nil() {
                    let _: () = msg_send![self.mtlcmdbuffer, commit];
                    self.mtlcmdbuffer = StrongPtr::nil();
                }
            }
        })
    }

    /// The renderer's `CAMetalLayer`. Translation of `METAL_GetMetalLayer()`.
    fn get_metal_layer(&self) -> Id {
        self.mtllayer.get()
    }

    /// The current render command encoder (nil if none could be made).
    /// Translation of `METAL_GetMetalCommandEncoder()`.
    fn get_metal_command_encoder(&mut self) -> Id {
        autoreleasepool(|| {
            // note that data.mtlcmdencoder can be nil if METAL_ActivateRenderCommandEncoder fails.
            //  Before SDL 2.0.18, it might have returned a non-nil encoding that might not have been
            //  usable for presentation. Check your return values!
            self.activate_render_command_encoder(MTL_LOAD_ACTION_LOAD, None, NIL);
            self.mtlcmdencoder.get()
        })
    }

    /// Translation of `GetWindowView()`: the window's own Metal view, with
    /// a reference for the renderer, if it has one.
    fn get_window_view(window: &Window) -> usize {
        let Ok(props) = window.properties() else {
            return 0;
        };
        let nswindow = props
            .get_number(PROP_WINDOW_COCOA_WINDOW_POINTER)
            .unwrap_or(0) as usize as Id;
        let tag = props
            .get_number(PROP_WINDOW_COCOA_METAL_VIEW_TAG_NUMBER)
            .unwrap_or(0) as NSInteger;
        if !nswindow.is_null() && tag != 0 {
            // SAFETY: the window's NSWindow and its views.
            unsafe {
                let mut view: Id = msg_send![nswindow, contentView];
                let subviews: Id = msg_send![view, subviews];
                let subviews = ns_array_items(subviews);
                if let Some(&first) = subviews.first() {
                    view = first;
                    let view_tag: NSInteger = msg_send![view, tag];
                    if view_tag == tag {
                        // (CFBridgingRetain)
                        return crate::core::apple::objc::retain(view) as usize;
                    }
                }
            }
        }
        0
    }

    /// Translation of `METAL_CreateRenderer()`.
    pub(crate) fn for_window(
        window: Window,
        output_colorspace: Colorspace,
        info: &RendererCreateInfo,
    ) -> Result<MetalRenderer> {
        autoreleasepool(|| Self::create(window, output_colorspace, info))
    }

    fn create(
        window: Window,
        output_colorspace: Colorspace,
        info: &RendererCreateInfo,
    ) -> Result<MetalRenderer> {
        let quadcount = u16::MAX as usize / 4;
        let indicessize = size_of::<u16>() * quadcount * 6;

        // Note: matrices are column major.
        #[rustfmt::skip]
        let identitytransform: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ];

        #[rustfmt::skip]
        let halfpixeltransform: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.5, 0.5, 0.0, 1.0,
        ];

        // (SDL_SetupRendererColorspace() is the front end's)

        // (@available(macos 10.11, *): every macOS the translation runs on)
        let sc_rgb_supported = true;
        if output_colorspace != Colorspace::SRGB
            && !(output_colorspace == Colorspace::SRGB_LINEAR && sc_rgb_supported)
        {
            return Err(Error::new("Unsupported output colorspace"));
        }

        // SAFETY: Metal's functions and objects, messaged with their
        // methods' signatures; the application's objects are its contract
        // (NativeObject).
        unsafe {
            let mut mtldevice = StrongPtr::nil();
            let mut mtlcmdqueue = StrongPtr::nil();

            if let Some(queue) = info.metal_command_queue {
                mtlcmdqueue = StrongPtr::retain(queue.as_ptr() as Id);
                mtldevice = StrongPtr::retain(msg_send![mtlcmdqueue, device]);
            }

            if mtldevice.is_nil() {
                if let Some(device) = info.metal_device {
                    mtldevice = StrongPtr::retain(device.as_ptr() as Id);
                }
            }

            if mtldevice.is_nil()
                && hints::get_bool(hints::RENDER_METAL_PREFER_LOW_POWER_DEVICE, true)
            {
                let devices = StrongPtr::from_owned(MTLCopyAllDevices());

                for device in ns_array_items(devices.get()) {
                    let low_power: Bool = msg_send![device, isLowPower];
                    if low_power.is_true() {
                        mtldevice = StrongPtr::retain(device);
                        break;
                    }
                }
            }

            if mtldevice.is_nil() {
                mtldevice = StrongPtr::from_owned(MTLCreateSystemDefaultDevice());
            }

            if mtldevice.is_nil() {
                return Err(Error::new("Failed to obtain Metal device"));
            }

            let mut view = Self::get_window_view(&window);
            if view == 0 {
                view = crate::video::vulkan::metal_create_view(&window)?;
            }

            let layer: Id = msg_send![view as Id, layer];

            let mut data = MetalRenderer {
                window,
                output_colorspace,
                current_colorspace: output_colorspace,
                hidden: false,
                target: None,
                target_mtltexture: StrongPtr::nil(),
                target_size: (0, 0),
                mtldevice: StrongPtr::nil(),
                mtlcmdqueue: StrongPtr::nil(),
                mtlcmdbuffer: StrongPtr::nil(),
                mtlcmdencoder: StrongPtr::nil(),
                mtllibrary: StrongPtr::nil(),
                mtlbackbuffer: StrongPtr::nil(),
                mtlsamplers: HashMap::new(),
                mtlbufconstants: StrongPtr::nil(),
                mtlbufquadindices: StrongPtr::nil(),
                mtlview: view,
                mtllayer: StrongPtr::retain(layer),
                mtlpassdesc: StrongPtr::nil(),
                activepipelines: None,
                allpipelines: Vec::new(),
                vertex_data: Vec::new(),
                texture_formats: Vec::new(),
                props: None,
            };
            let flags = data.window.flags().unwrap_or_default();
            data.hidden = flags.intersects(WindowFlags::HIDDEN | WindowFlags::MINIMIZED);
            data.invalidate();

            if output_colorspace == Colorspace::SRGB_LINEAR {
                let _: () = msg_send![layer, setWantsExtendedDynamicRangeContent: YES];
                let _: () = msg_send![layer, setPixelFormat: MTL_PIXEL_FORMAT_RGBA16_FLOAT];

                let colorspace = CGColorSpaceCreateWithName(kCGColorSpaceExtendedLinearSRGB);
                let _: () = msg_send![layer, setColorspace: colorspace];
                CGColorSpaceRelease(colorspace);
            }

            let _: () = msg_send![layer, setDevice: mtldevice.get()];

            // Necessary for RenderReadPixels.
            let _: () = msg_send![layer, setFramebufferOnly: NO];

            data.mtldevice = StrongPtr::retain(msg_send![layer, device]);
            if mtlcmdqueue.is_nil() {
                mtlcmdqueue = StrongPtr::from_owned(msg_send![data.mtldevice, newCommandQueue]);
            }
            data.mtlcmdqueue = mtlcmdqueue;
            set_label(data.mtlcmdqueue.get(), "SDL Metal Renderer");
            data.mtlpassdesc = StrongPtr::retain(msg_send![
                class!(MTLRenderPassDescriptor),
                renderPassDescriptor
            ]);

            // The compiled .metallib is embedded in a static array in a header file
            // but the original shader source code is in SDL_shaders_metal.metal.
            let destructor = RcBlock::new0(|| {});
            let mtllibdata = dispatch_data_create(
                shaders::SDL_METALLIB.as_ptr() as *const c_void,
                shaders::SDL_METALLIB.len(),
                dispatch_get_global_queue(0, 0),
                destructor.as_ptr(),
            );
            let mut err: Id = NIL;
            let mtllibrary = StrongPtr::from_owned(
                msg_send![data.mtldevice, newLibraryWithData: mtllibdata error: &mut err as *mut Id],
            );
            dispatch_release(mtllibdata);
            data.mtllibrary = mtllibrary;
            crate::sdl_assert!(err.is_null());
            set_label(data.mtllibrary.get(), "SDL Metal renderer shader library");

            // Do some shader pipeline state loading up-front rather than on demand.
            data.choose_shader_pipelines(MTL_PIXEL_FORMAT_BGRA8_UNORM);

            let mtlbufconstantstaging = StrongPtr::from_owned(
                msg_send![data.mtldevice, newBufferWithLength: CONSTANTS_LENGTH as NSUInteger
                                                      options: MTL_RESOURCE_STORAGE_MODE_SHARED],
            );

            let constantdata: *mut c_void = msg_send![mtlbufconstantstaging, contents];
            let constantdata =
                std::slice::from_raw_parts_mut(constantdata as *mut u8, CONSTANTS_LENGTH);
            let put = |dst: &mut [u8], offset: usize, floats: &[f32]| {
                MetalRenderer::write_floats(
                    &mut dst[offset..offset + floats.len() * FLOAT],
                    floats,
                );
            };
            put(constantdata, CONSTANTS_OFFSET_IDENTITY, &identitytransform);
            put(
                constantdata,
                CONSTANTS_OFFSET_HALF_PIXEL_TRANSFORM,
                &halfpixeltransform,
            );
            put(
                constantdata,
                CONSTANTS_OFFSET_DECODE_BT601_LIMITED,
                &ycbcr_shader_matrix(Colorspace::BT601_LIMITED, 8),
            );
            put(
                constantdata,
                CONSTANTS_OFFSET_DECODE_BT601_FULL,
                &ycbcr_shader_matrix(Colorspace::BT601_FULL, 8),
            );
            put(
                constantdata,
                CONSTANTS_OFFSET_DECODE_BT709_LIMITED,
                &ycbcr_shader_matrix(Colorspace::BT709_LIMITED, 8),
            );
            put(
                constantdata,
                CONSTANTS_OFFSET_DECODE_BT709_FULL,
                &ycbcr_shader_matrix(Colorspace::BT709_FULL, 8),
            );
            put(
                constantdata,
                CONSTANTS_OFFSET_DECODE_BT2020_LIMITED,
                &ycbcr_shader_matrix(Colorspace::BT2020_LIMITED, 10),
            );
            put(
                constantdata,
                CONSTANTS_OFFSET_DECODE_BT2020_FULL,
                &ycbcr_shader_matrix(Colorspace::BT2020_FULL, 10),
            );

            let mtlbufquadindicesstaging = StrongPtr::from_owned(
                msg_send![data.mtldevice, newBufferWithLength: indicessize as NSUInteger
                                                      options: MTL_RESOURCE_STORAGE_MODE_SHARED],
            );

            /* Quads in the following vertex order (matches the FillRects vertices):
             * 1---3
             * | \ |
             * 0---2
             */
            let indexdata: *mut c_void = msg_send![mtlbufquadindicesstaging, contents];
            let indexdata = std::slice::from_raw_parts_mut(indexdata as *mut u16, quadcount * 6);
            for i in 0..quadcount {
                indexdata[i * 6] = (i * 4) as u16;
                indexdata[i * 6 + 1] = (i * 4 + 1) as u16;
                indexdata[i * 6 + 2] = (i * 4 + 2) as u16;

                indexdata[i * 6 + 3] = (i * 4 + 2) as u16;
                indexdata[i * 6 + 4] = (i * 4 + 1) as u16;
                indexdata[i * 6 + 5] = (i * 4 + 3) as u16;
            }

            data.mtlbufconstants = StrongPtr::from_owned(
                msg_send![data.mtldevice, newBufferWithLength: CONSTANTS_LENGTH as NSUInteger
                                                      options: MTL_RESOURCE_STORAGE_MODE_PRIVATE],
            );
            set_label(data.mtlbufconstants.get(), "SDL constant data");

            data.mtlbufquadindices = StrongPtr::from_owned(
                msg_send![data.mtldevice, newBufferWithLength: indicessize as NSUInteger
                                                      options: MTL_RESOURCE_STORAGE_MODE_PRIVATE],
            );
            set_label(data.mtlbufquadindices.get(), "SDL quad index buffer");

            let cmdbuffer: Id = msg_send![data.mtlcmdqueue, commandBuffer];
            let blitcmd: Id = msg_send![cmdbuffer, blitCommandEncoder];

            let _: () = msg_send![blitcmd, copyFromBuffer: mtlbufconstantstaging.get()
                                             sourceOffset: 0 as NSUInteger
                                                 toBuffer: data.mtlbufconstants.get()
                                        destinationOffset: 0 as NSUInteger
                                                     size: CONSTANTS_LENGTH as NSUInteger];
            let _: () = msg_send![blitcmd, copyFromBuffer: mtlbufquadindicesstaging.get()
                                             sourceOffset: 0 as NSUInteger
                                                 toBuffer: data.mtlbufquadindices.get()
                                        destinationOffset: 0 as NSUInteger
                                                     size: indicessize as NSUInteger];

            let _: () = msg_send![blitcmd, endEncoding];
            let _: () = msg_send![cmdbuffer, commit];

            // !!! FIXME: force more clears here so all the drawables are sane to start, and our static buffers are definitely flushed.

            data.texture_formats = vec![
                PixelFormat::ARGB8888,
                PixelFormat::ABGR8888,
                PixelFormat::ABGR2101010,
                PixelFormat::RGBA64_FLOAT,
                PixelFormat::RGBA128_FLOAT,
            ];
            if macos_at_least(11, 0, 0) {
                let apple1: Bool = msg_send![data.mtldevice, supportsFamily: MTL_GPU_FAMILY_APPLE1];
                if apple1.is_true() {
                    data.texture_formats.extend([
                        PixelFormat::RGB565,
                        PixelFormat::RGBA5551,
                        PixelFormat::ARGB1555,
                        PixelFormat::RGBA4444,
                    ]);
                }
            }
            data.texture_formats.extend([
                PixelFormat::INDEX8,
                PixelFormat::YV12,
                PixelFormat::IYUV,
                PixelFormat::I444,
                PixelFormat::NV12,
                PixelFormat::NV21,
                PixelFormat::P010,
                PixelFormat::I0FL,
                PixelFormat::I4FL,
            ]);

            let _: () = msg_send![data.mtllayer, setDisplaySyncEnabled: NO];

            Ok(data)
        }
    }
}

impl RenderBackend for MetalRenderer {
    fn name(&self) -> &'static str {
        METAL_RENDERER
    }

    /// Translation of `METAL_GetOutputSize()`.
    fn output_size(&self, _textures: &TextureStore) -> Option<Result<(i32, i32)>> {
        Some(Ok(self.get_output_size()))
    }

    fn texture_formats(&self) -> Option<Vec<PixelFormat>> {
        Some(self.texture_formats.clone())
    }

    /// <https://developer.apple.com/metal/Metal-Feature-Set-Tables.pdf>
    fn max_texture_size(&self) -> Option<i32> {
        Some(16384)
    }

    fn set_properties(&mut self, props: &Properties) {
        self.props = Some(props.clone());
        let _ = props.set(
            PROP_RENDERER_METAL_DEVICE_POINTER,
            self.mtldevice.get() as usize as i64,
        );
        let _ = props.set(
            PROP_RENDERER_METAL_COMMAND_QUEUE_POINTER,
            self.mtlcmdqueue.get() as usize as i64,
        );
    }

    /// Translation of `METAL_SupportsBlendMode()`.
    fn supports_blend_mode(&self, blend_mode: BlendMode) -> bool {
        !(get_blend_factor(blend_mode.src_color_factor()) == INVALID_BLEND_FACTOR
            || get_blend_factor(blend_mode.src_alpha_factor()) == INVALID_BLEND_FACTOR
            || get_blend_operation(blend_mode.color_operation()) == INVALID_BLEND_OPERATION
            || get_blend_factor(blend_mode.dst_color_factor()) == INVALID_BLEND_FACTOR
            || get_blend_factor(blend_mode.dst_alpha_factor()) == INVALID_BLEND_FACTOR
            || get_blend_operation(blend_mode.alpha_operation()) == INVALID_BLEND_OPERATION)
    }

    /// Translation of `METAL_CreateTexture()`.
    fn create_texture(
        &mut self,
        texture: &mut TextureData,
        create_props: &TextureCreateProps,
    ) -> Result<()> {
        use PixelFormat as F;
        autoreleasepool(|| {
            let mut pixfmt = MTL_PIXEL_FORMAT_INVALID;
            let mut surface: IoSurfaceRef = std::ptr::null_mut();

            // SAFETY: the renderer's device, the descriptors' properties, and
            // the application's objects (NativeObject's contract).
            unsafe {
                if create_props.metal_pixelbuffer != 0 {
                    surface = CVPixelBufferGetIOSurface(
                        create_props.metal_pixelbuffer as usize as *mut c_void,
                    );
                    if surface.is_null() {
                        return Err(Error::new("CVPixelBufferGetIOSurface() failed"));
                    }
                }

                match texture.format {
                    F::ABGR8888 => {
                        pixfmt = if self.output_colorspace == Colorspace::SRGB_LINEAR {
                            MTL_PIXEL_FORMAT_RGBA8_UNORM_SRGB
                        } else {
                            MTL_PIXEL_FORMAT_RGBA8_UNORM
                        };
                    }
                    F::ARGB8888 => {
                        pixfmt = if self.output_colorspace == Colorspace::SRGB_LINEAR {
                            MTL_PIXEL_FORMAT_BGRA8_UNORM_SRGB
                        } else {
                            MTL_PIXEL_FORMAT_BGRA8_UNORM
                        };
                    }
                    F::ABGR2101010 => pixfmt = MTL_PIXEL_FORMAT_RGB10A2_UNORM,
                    F::RGB565 => {
                        if macos_at_least(11, 0, 0) {
                            pixfmt = MTL_PIXEL_FORMAT_B5G6R5_UNORM;
                        }
                    }
                    F::RGBA5551 => {
                        if macos_at_least(11, 0, 0) {
                            pixfmt = MTL_PIXEL_FORMAT_A1BGR5_UNORM;
                        }
                    }
                    F::ARGB1555 => {
                        if macos_at_least(11, 0, 0) {
                            pixfmt = MTL_PIXEL_FORMAT_BGR5A1_UNORM;
                        }
                    }
                    F::RGBA4444 => {
                        if macos_at_least(11, 0, 0) {
                            pixfmt = MTL_PIXEL_FORMAT_ABGR4_UNORM;
                        }
                    }
                    F::INDEX8 | F::YV12 | F::IYUV | F::I444 | F::NV12 | F::NV21 => {
                        pixfmt = MTL_PIXEL_FORMAT_R8_UNORM;
                    }
                    F::P010 | F::I0FL | F::I4FL => pixfmt = MTL_PIXEL_FORMAT_R16_UNORM,
                    F::RGBA64_FLOAT => pixfmt = MTL_PIXEL_FORMAT_RGBA16_FLOAT,
                    F::RGBA128_FLOAT => pixfmt = MTL_PIXEL_FORMAT_RGBA32_FLOAT,
                    _ => {}
                }

                if pixfmt == MTL_PIXEL_FORMAT_INVALID {
                    return Err(Error::new(format!(
                        "Texture format {} not supported by Metal",
                        texture.format.name()
                    )));
                }

                let mtltexdesc: Id = msg_send![class!(MTLTextureDescriptor),
                    texture2DDescriptorWithPixelFormat: pixfmt
                                                 width: texture.w as NSUInteger
                                                height: texture.h as NSUInteger
                                             mipmapped: NO];

                let mut usage = if texture.access == TextureAccess::Target {
                    MTL_TEXTURE_USAGE_SHADER_READ | MTL_TEXTURE_USAGE_RENDER_TARGET
                } else {
                    MTL_TEXTURE_USAGE_SHADER_READ
                };
                usage |= create_props.metal_texture_usage as NSUInteger;
                let _: () = msg_send![mtltexdesc, setUsage: usage];

                // A plane: the application's texture (retained), or one made
                // here, on the IOSurface's plane if there is one.
                let device = self.mtldevice.get();
                let make = |given: u64, desc: Id, plane: NSUInteger| -> StrongPtr {
                    if given != 0 {
                        StrongPtr::retain(given as usize as Id)
                    } else if !surface.is_null() {
                        StrongPtr::from_owned(msg_send![device, newTextureWithDescriptor: desc
                                                                               iosurface: surface
                                                                                   plane: plane])
                    } else {
                        StrongPtr::from_owned(msg_send![device, newTextureWithDescriptor: desc])
                    }
                };

                let mtltexture = make(create_props.metal_texture[0], mtltexdesc, 0);
                if mtltexture.is_nil() {
                    return Err(Error::new("Texture allocation failed"));
                }
                let props = texture.props.get_or_insert_with(Properties::new).clone();
                let _ = props.set(
                    PROP_TEXTURE_METAL_TEXTURE_POINTER,
                    mtltexture.get() as usize as i64,
                );

                let format = texture.format;
                let yuv = matches!(format, F::YV12 | F::IYUV | F::I444 | F::I0FL | F::I4FL);
                let nv12 = matches!(format, F::NV12 | F::NV21 | F::P010);

                if yuv {
                    let _: () = msg_send![mtltexdesc, setPixelFormat: pixfmt];
                    if format == F::I444 || format == F::I4FL {
                        let _: () = msg_send![mtltexdesc, setWidth: texture.w as NSUInteger];
                        let _: () = msg_send![mtltexdesc, setHeight: texture.h as NSUInteger];
                    } else {
                        let _: () =
                            msg_send![mtltexdesc, setWidth: ((texture.w + 1) / 2) as NSUInteger];
                        let _: () =
                            msg_send![mtltexdesc, setHeight: ((texture.h + 1) / 2) as NSUInteger];
                    }
                } else if format == F::P010 {
                    let _: () = msg_send![mtltexdesc, setPixelFormat: MTL_PIXEL_FORMAT_RG16_UNORM];
                    let _: () =
                        msg_send![mtltexdesc, setWidth: ((texture.w + 1) / 2) as NSUInteger];
                    let _: () =
                        msg_send![mtltexdesc, setHeight: ((texture.h + 1) / 2) as NSUInteger];
                } else if nv12 {
                    let _: () = msg_send![mtltexdesc, setPixelFormat: MTL_PIXEL_FORMAT_RG8_UNORM];
                    let _: () =
                        msg_send![mtltexdesc, setWidth: ((texture.w + 1) / 2) as NSUInteger];
                    let _: () =
                        msg_send![mtltexdesc, setHeight: ((texture.h + 1) / 2) as NSUInteger];
                }

                let mut texturedata = MetalTexture {
                    mtltexture,
                    mtltexture_uv: StrongPtr::nil(),
                    mtltexture_u: StrongPtr::nil(),
                    mtltexture_v: StrongPtr::nil(),
                    fragment_function: FragmentFunction::Copy,
                    yuv: false,
                    nv12: false,
                    conversion_buffer_offset: 0,
                    hasdata: false,
                    lockedbuffer: StrongPtr::nil(),
                    lockedrect: Rect::default(),
                    palette: StrongPtr::nil(),
                };

                if yuv {
                    let uplane = if format == F::YV12 { 2 } else { 1 };
                    let mtltexture = make(create_props.metal_texture[1], mtltexdesc, uplane);
                    if mtltexture.is_nil() {
                        return Err(Error::new("Texture allocation failed"));
                    }
                    let _ = props.set(
                        PROP_TEXTURE_METAL_TEXTURE_U_POINTER,
                        mtltexture.get() as usize as i64,
                    );
                    texturedata.mtltexture_u = mtltexture;

                    let vplane = if format == F::YV12 { 1 } else { 2 };
                    let mtltexture = make(create_props.metal_texture[2], mtltexdesc, vplane);
                    if mtltexture.is_nil() {
                        return Err(Error::new("Texture allocation failed"));
                    }
                    let _ = props.set(
                        PROP_TEXTURE_METAL_TEXTURE_V_POINTER,
                        mtltexture.get() as usize as i64,
                    );
                    texturedata.mtltexture_v = mtltexture;
                } else if nv12 {
                    let mtltexture = make(create_props.metal_texture_uv, mtltexdesc, 1);
                    if mtltexture.is_nil() {
                        return Err(Error::new("Texture allocation failed"));
                    }
                    let _ = props.set(
                        PROP_TEXTURE_METAL_TEXTURE_UV_POINTER,
                        mtltexture.get() as usize as i64,
                    );
                    texturedata.mtltexture_uv = mtltexture;
                }

                texturedata.fragment_function = if format == F::INDEX8 {
                    FragmentFunction::Palette
                } else if yuv {
                    FragmentFunction::Yuv
                } else if nv12 {
                    FragmentFunction::Nv12
                } else {
                    FragmentFunction::Copy
                };
                texturedata.yuv = yuv;
                texturedata.nv12 = nv12;
                if yuv || nv12 {
                    let offset = get_ycbcr_to_rgb_conversion_matrix(
                        texture.colorspace,
                        texture.w,
                        texture.h,
                        8,
                    );
                    if offset == 0 {
                        return Err(Error::new("Unsupported YUV colorspace"));
                    }
                    texturedata.conversion_buffer_offset = offset;
                }
                texture.internal = Some(Box::new(texturedata));
            }
            Ok(())
        })
    }

    /// Translation of `METAL_QueueSetViewport()`.
    fn queue_set_viewport(&mut self, cmd: &mut RenderCommand) -> Result<()> {
        let RenderCommand::SetViewport { first, rect } = cmd else {
            return Ok(());
        };
        let (w, h) = (rect.w, rect.h);
        let mut projection = [[0f32; 4]; 4]; // Prepare an orthographic projection
        let matrixlen = size_of_val(&projection);
        let (offset, matrix) = self.allocate_render_vertices(matrixlen, constant_align(16));
        *first = offset;

        if w != 0 && h != 0 {
            projection[0][0] = 2.0 / w as f32;
            projection[1][1] = -2.0 / h as f32;
            projection[3][0] = -1.0;
            projection[3][1] = 1.0;
            projection[3][3] = 1.0;
        }
        MetalRenderer::write_floats(matrix, projection.as_flattened());

        Ok(())
    }

    /// Translation of `METAL_QueueNoOp()`.
    fn queue_set_draw_color(&mut self, _cmd: &mut RenderCommand) -> Result<()> {
        Ok(()) // nothing to do in this backend.
    }

    /// Translation of `METAL_QueueDrawPoints()`.
    fn queue_draw_points(&mut self, cmd: &mut DrawCmd, points: &[FPoint]) -> Result<()> {
        let mut color = cmd.color;
        let convert_color = self.rendering_linear_space();
        if convert_color {
            self.convert_to_linear(&mut color);
        }

        let vertlen = (2 * FLOAT + 4 * FLOAT) * points.len();
        let (first, verts) = self.allocate_render_vertices(vertlen, device_align(8));
        cmd.first = first;
        cmd.count = points.len();

        let floats: Vec<f32> = points
            .iter()
            .flat_map(|p| [p.x, p.y, color.r, color.g, color.b, color.a])
            .collect();
        MetalRenderer::write_floats(verts, &floats);
        Ok(())
    }

    /// Translation of `METAL_QueueDrawLines()`.
    fn queue_draw_lines(&mut self, cmd: &mut DrawCmd, points: &[FPoint]) -> Option<Result<()>> {
        let mut color = cmd.color;
        let convert_color = self.rendering_linear_space();

        crate::sdl_assert!(points.len() >= 2); // should have been checked at the higher level.

        let vertlen = (2 * FLOAT + 4 * FLOAT) * points.len();

        if convert_color {
            self.convert_to_linear(&mut color);
        }

        let mut floats: Vec<f32> = points
            .iter()
            .flat_map(|p| [p.x, p.y, color.r, color.g, color.b, color.a])
            .collect();

        /* If the line segment is completely horizontal or vertical,
        make it one pixel longer, to satisfy the diamond-exit rule.
        We should probably do this for diagonal lines too, but we'd have to
        do some trigonometry to figure out the correct pixel and generally
        when we have problems with pixel perfection, it's for straight lines
        that are missing a pixel that frames something and not arbitrary
        angles. Maybe !!! FIXME for later, though. */

        if points.len() >= 2 {
            let n = points.len();
            // update the last line.
            let (start, end) = (points[n - 2], points[n - 1]);
            let last = (n - 1) * 6;

            let (xstart, ystart) = (start.x, start.y);
            let (xend, yend) = (end.x, end.y);

            if ystart == yend {
                // horizontal line
                floats[last] += if xend > xstart { 1.0 } else { -1.0 };
            } else if xstart == xend {
                // vertical line
                floats[last + 1] += if yend > ystart { 1.0 } else { -1.0 };
            }
        }

        let (first, verts) = self.allocate_render_vertices(vertlen, device_align(8));
        cmd.first = first;
        cmd.count = points.len();
        MetalRenderer::write_floats(verts, &floats);
        Some(Ok(()))
    }

    fn has_queue_fill_rects(&self) -> bool {
        false
    }

    fn queue_fill_rects(&mut self, _cmd: &mut DrawCmd, _rects: &[FRect]) -> Result<()> {
        Err(Error::unsupported())
    }

    fn has_queue_copy(&self) -> bool {
        false
    }

    fn queue_copy(
        &mut self,
        _cmd: &mut DrawCmd,
        _texture: &TextureData,
        _srcrect: &FRect,
        _dstrect: &FRect,
    ) -> Result<()> {
        Err(Error::unsupported())
    }

    fn has_queue_copy_ex(&self) -> bool {
        false
    }

    fn queue_copy_ex(
        &mut self,
        _cmd: &mut DrawCmd,
        _texture: &TextureData,
        _copy: &CopyEx,
    ) -> Result<()> {
        Err(Error::unsupported())
    }

    fn has_queue_geometry(&self) -> bool {
        true
    }

    /// Translation of `METAL_QueueGeometry()`.
    fn queue_geometry(
        &mut self,
        cmd: &mut DrawCmd,
        texture: Option<&TextureData>,
        geometry: &Geometry<'_>,
        scale_x: f32,
        scale_y: f32,
    ) -> Result<()> {
        let convert_color = self.rendering_linear_space();
        let count = geometry.count();
        let floats_per_vertex = 2 + 4 + if texture.is_some() { 2 } else { 0 };
        let vertlen = floats_per_vertex * FLOAT * count;

        let mut floats = Vec::with_capacity(floats_per_vertex * count);
        for i in 0..count {
            let j = geometry.vertex(i);
            let (x, y) = geometry.xy(j);
            floats.push(x * scale_x);
            floats.push(y * scale_y);

            let mut col = geometry.color(j);
            if convert_color {
                self.convert_to_linear(&mut col);
            }
            floats.extend([col.r, col.g, col.b, col.a]);

            if texture.is_some() {
                let (u, v) = geometry.uv(j);
                floats.extend([u, v]);
            }
        }

        let (first, verts) = self.allocate_render_vertices(vertlen, device_align(8));
        cmd.first = first;
        cmd.count = count;
        MetalRenderer::write_floats(verts, &floats);
        Ok(())
    }

    fn invalidate_cached_state(&mut self) {
        self.invalidate();
    }

    /// Translation of `METAL_RunCommandQueue()`.
    fn run_command_queue(
        &mut self,
        cmds: &[RenderCommand],
        textures: &mut TextureStore,
        _gpu_render_states: &GpuRenderStates,
    ) -> Result<()> {
        autoreleasepool(|| self.run_commands(cmds, textures))
    }

    fn reset_vertices(&mut self) {
        self.vertex_data.clear();
    }

    /// Translation of `METAL_CreatePalette()`.
    fn create_palette(&mut self) -> Result<Box<dyn Any>> {
        autoreleasepool(|| {
            let pixfmt = if self.output_colorspace == Colorspace::SRGB_LINEAR {
                MTL_PIXEL_FORMAT_RGBA8_UNORM_SRGB
            } else {
                MTL_PIXEL_FORMAT_RGBA8_UNORM
            };
            // SAFETY: a descriptor configured through its properties, and the
            // renderer's device.
            unsafe {
                let mtltexdesc: Id = msg_send![class!(MTLTextureDescriptor),
                    texture2DDescriptorWithPixelFormat: pixfmt
                                                 width: 256 as NSUInteger
                                                height: 1 as NSUInteger
                                             mipmapped: NO];
                let _: () = msg_send![mtltexdesc, setUsage: MTL_TEXTURE_USAGE_SHADER_READ];

                let mtltexture = StrongPtr::from_owned(
                    msg_send![self.mtldevice, newTextureWithDescriptor: mtltexdesc],
                );
                if mtltexture.is_nil() {
                    return Err(Error::new("Palette allocation failed"));
                }

                Ok(Box::new(MetalPalette {
                    mtltexture,
                    hasdata: false,
                }) as Box<dyn Any>)
            }
        })
    }

    /// Translation of `METAL_UpdatePalette()`.
    fn update_palette(&mut self, palette: &mut dyn Any, colors: &[Color]) -> Result<()> {
        autoreleasepool(|| {
            let palettedata = palette
                .downcast_mut::<MetalPalette>()
                .ok_or_else(|| Error::invalid_param("palette"))?;
            let rect = Rect::new(0, 0, colors.len() as i32, 1);
            let bytes: Vec<u8> = colors.iter().flat_map(|c| [c.r, c.g, c.b, c.a]).collect();
            let texture = palettedata.mtltexture.clone();

            self.update_texture_internal(
                palettedata.hasdata,
                texture.get(),
                rect,
                0,
                &bytes,
                bytes.len() as i32,
            )?;
            palettedata.hasdata = true;
            Ok(())
        })
    }

    /// Translation of `METAL_DestroyPalette()`.
    fn destroy_palette(&mut self, palette: Box<dyn Any>) {
        autoreleasepool(|| drop(palette));
    }

    /// Record the texture's palette, which the draws bind
    /// (`texture->palette->internal`).
    fn change_texture_palette(
        &mut self,
        texture: &mut TextureData,
        palette: Option<&dyn Any>,
    ) -> Option<Result<()>> {
        let palette = palette
            .and_then(|p| p.downcast_ref::<MetalPalette>())
            .map(|p| p.mtltexture.clone())
            .unwrap_or_default();
        if let Some(t) = metal_texture_mut(texture) {
            t.palette = palette;
        }
        Some(Ok(()))
    }

    /// Translation of `METAL_UpdateTexture()`.
    fn update_texture(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        pixels: &[u8],
        pitch: usize,
    ) -> Result<()> {
        autoreleasepool(|| {
            let format = texture.format;
            let texturedata = metal_texture(texture).ok_or_else(invalid_texture)?;
            let (hasdata, yuv, nv12) = (texturedata.hasdata, texturedata.yuv, texturedata.nv12);
            let main = texturedata.mtltexture.clone();
            // YV12 stores V before U, so the plane order is swapped for it.
            let (firstplane, secondplane) = if format == PixelFormat::YV12 {
                (
                    texturedata.mtltexture_v.clone(),
                    texturedata.mtltexture_u.clone(),
                )
            } else {
                (
                    texturedata.mtltexture_u.clone(),
                    texturedata.mtltexture_v.clone(),
                )
            };
            let uvplane = texturedata.mtltexture_uv.clone();
            let bpp = format.bytes_per_pixel() as usize;
            let pitch_i = pitch as i32;

            self.update_texture_internal(
                hasdata,
                main.get(),
                *rect,
                0,
                plane(
                    pixels,
                    0,
                    pitch,
                    rect.w.max(0) as usize * bpp,
                    rect.h.max(0) as usize,
                )?,
                pitch_i,
            )?;
            if yuv {
                let (uvpitch, uvrect) =
                    if format == PixelFormat::I444 || format == PixelFormat::I4FL {
                        (pitch, *rect)
                    } else {
                        (
                            (pitch / bpp).div_ceil(2) * bpp,
                            Rect::new(rect.x / 2, rect.y / 2, (rect.w + 1) / 2, (rect.h + 1) / 2),
                        )
                    };
                let uvrow = uvrect.w.max(0) as usize * bpp;
                let uvrows = uvrect.h.max(0) as usize;

                // Skip to the correct offset into the next texture
                let mut offset = rect.h.max(0) as usize * pitch;
                self.update_texture_internal(
                    hasdata,
                    firstplane.get(),
                    uvrect,
                    0,
                    plane(pixels, offset, uvpitch, uvrow, uvrows)?,
                    uvpitch as i32,
                )?;

                // Skip to the correct offset into the next texture
                offset += uvrows * uvpitch;
                self.update_texture_internal(
                    hasdata,
                    secondplane.get(),
                    uvrect,
                    0,
                    plane(pixels, offset, uvpitch, uvrow, uvrows)?,
                    uvpitch as i32,
                )?;
            }

            if nv12 {
                let uvpitch = (pitch / bpp).div_ceil(2) * 2 * bpp;
                let uvrect = Rect::new(rect.x / 2, rect.y / 2, (rect.w + 1) / 2, (rect.h + 1) / 2);

                // Skip to the correct offset into the next texture
                let offset = rect.h.max(0) as usize * pitch;
                self.update_texture_internal(
                    hasdata,
                    uvplane.get(),
                    uvrect,
                    0,
                    plane(
                        pixels,
                        offset,
                        uvpitch,
                        uvrect.w.max(0) as usize * 2 * bpp,
                        uvrect.h.max(0) as usize,
                    )?,
                    uvpitch as i32,
                )?;
            }
            if let Some(t) = metal_texture_mut(texture) {
                t.hasdata = true;
            }

            Ok(())
        })
    }

    /// Translation of `METAL_UpdateTextureYUV()`.
    fn update_texture_yuv(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        y: (&[u8], usize),
        u: (&[u8], usize),
        v: (&[u8], usize),
    ) -> Option<Result<()>> {
        Some(autoreleasepool(|| {
            let format = texture.format;
            let texturedata = metal_texture(texture).ok_or_else(invalid_texture)?;
            let hasdata = texturedata.hasdata;
            let (main, tu, tv) = (
                texturedata.mtltexture.clone(),
                texturedata.mtltexture_u.clone(),
                texturedata.mtltexture_v.clone(),
            );
            let uvrect = if format == PixelFormat::I444 || format == PixelFormat::I4FL {
                *rect
            } else {
                Rect::new(rect.x / 2, rect.y / 2, (rect.w + 1) / 2, (rect.h + 1) / 2)
            };

            // Bail out if we're supposed to update an empty rectangle
            if rect.w <= 0 || rect.h <= 0 {
                return Ok(());
            }

            let bpp = format.bytes_per_pixel() as usize;
            self.update_texture_internal(
                hasdata,
                main.get(),
                *rect,
                0,
                plane(y.0, 0, y.1, rect.w as usize * bpp, rect.h as usize)?,
                y.1 as i32,
            )?;
            let (uvrow, uvrows) = (uvrect.w.max(0) as usize * bpp, uvrect.h.max(0) as usize);
            self.update_texture_internal(
                hasdata,
                tu.get(),
                uvrect,
                0,
                plane(u.0, 0, u.1, uvrow, uvrows)?,
                u.1 as i32,
            )?;
            self.update_texture_internal(
                hasdata,
                tv.get(),
                uvrect,
                0,
                plane(v.0, 0, v.1, uvrow, uvrows)?,
                v.1 as i32,
            )?;

            if let Some(t) = metal_texture_mut(texture) {
                t.hasdata = true;
            }

            Ok(())
        }))
    }

    /// Translation of `METAL_UpdateTextureNV()`.
    fn update_texture_nv(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        y: (&[u8], usize),
        uv: (&[u8], usize),
    ) -> Option<Result<()>> {
        Some(autoreleasepool(|| {
            let format = texture.format;
            let texturedata = metal_texture(texture).ok_or_else(invalid_texture)?;
            let hasdata = texturedata.hasdata;
            let (main, tuv) = (
                texturedata.mtltexture.clone(),
                texturedata.mtltexture_uv.clone(),
            );
            let uvrect = Rect::new(rect.x / 2, rect.y / 2, (rect.w + 1) / 2, (rect.h + 1) / 2);

            // Bail out if we're supposed to update an empty rectangle
            if rect.w <= 0 || rect.h <= 0 {
                return Ok(());
            }

            let bpp = format.bytes_per_pixel() as usize;
            self.update_texture_internal(
                hasdata,
                main.get(),
                *rect,
                0,
                plane(y.0, 0, y.1, rect.w as usize * bpp, rect.h as usize)?,
                y.1 as i32,
            )?;

            self.update_texture_internal(
                hasdata,
                tuv.get(),
                uvrect,
                0,
                plane(
                    uv.0,
                    0,
                    uv.1,
                    uvrect.w as usize * 2 * bpp,
                    uvrect.h as usize,
                )?,
                uv.1 as i32,
            )?;

            if let Some(t) = metal_texture_mut(texture) {
                t.hasdata = true;
            }

            Ok(())
        }))
    }

    /// Translation of `METAL_LockTexture()`: the pixels are the locked
    /// buffer's, from its start.
    fn lock_texture(&mut self, texture: &mut TextureData, rect: &Rect) -> Result<(usize, i32)> {
        autoreleasepool(|| {
            let format = texture.format;
            let texturedata = metal_texture(texture).ok_or_else(invalid_texture)?;

            if rect.w <= 0 || rect.h <= 0 {
                return Err(Error::new("Invalid rectangle dimensions for LockTexture."));
            }

            let (size, calculated_pitch) = if texturedata.yuv || texturedata.nv12 {
                crate::video::surface::calculate_yuv_size(format, rect.w, rect.h)?
            } else {
                let calculated_pitch = format.bytes_per_pixel() as usize * rect.w as usize;
                (rect.h as usize * calculated_pitch, calculated_pitch)
            };

            // SAFETY: the renderer's device.
            let lockedbuffer = unsafe {
                StrongPtr::from_owned(
                    msg_send![self.mtldevice, newBufferWithLength: size as NSUInteger
                                                                            options: MTL_RESOURCE_STORAGE_MODE_SHARED],
                )
            };
            if lockedbuffer.is_nil() {
                return Err(Error::out_of_memory());
            }

            let texturedata = metal_texture_mut(texture).ok_or_else(invalid_texture)?;
            texturedata.lockedrect = *rect;
            texturedata.lockedbuffer = lockedbuffer;
            Ok((0, calculated_pitch as i32))
        })
    }

    fn texture_pixels_mut<'t>(&mut self, texture: &'t mut TextureData) -> Option<&'t mut [u8]> {
        let texturedata = metal_texture_mut(texture)?;
        if texturedata.lockedbuffer.is_nil() {
            return None;
        }
        // SAFETY: the locked buffer's shared memory, `length` bytes, which
        // lives as long as the lock (the texture data owns the buffer).
        unsafe {
            let contents: *mut c_void = msg_send![texturedata.lockedbuffer, contents];
            let length: NSUInteger = msg_send![texturedata.lockedbuffer, length];
            Some(std::slice::from_raw_parts_mut(contents as *mut u8, length))
        }
    }

    /// Translation of `METAL_UnlockTexture()`.
    fn unlock_texture(&mut self, texture: &mut TextureData) {
        autoreleasepool(|| {
            let format = texture.format;
            let Some(texturedata) = metal_texture(texture) else {
                return;
            };
            let rect = texturedata.lockedrect;
            let bpp = format.bytes_per_pixel() as i32;
            let pitch = bpp * rect.w;
            let uvrect = Rect::new(rect.x / 2, rect.y / 2, (rect.w + 1) / 2, (rect.h + 1) / 2);

            if texturedata.lockedbuffer.is_nil() {
                return;
            }
            let lockedbuffer = texturedata.lockedbuffer.get();
            let (yuv, nv12) = (texturedata.yuv, texturedata.nv12);
            let main = texturedata.mtltexture.get();
            // YV12 stores V before U, so the plane order is swapped for it.
            let (firstplane, secondplane) = if format == PixelFormat::YV12 {
                (
                    texturedata.mtltexture_v.get(),
                    texturedata.mtltexture_u.get(),
                )
            } else {
                (
                    texturedata.mtltexture_u.get(),
                    texturedata.mtltexture_v.get(),
                )
            };
            let uvplane = texturedata.mtltexture_uv.get();

            // SAFETY: the renderer's command objects, the texture's planes and
            // its locked buffer.
            unsafe {
                if !self.mtlcmdencoder.is_nil() {
                    let _: () = msg_send![self.mtlcmdencoder, endEncoding];
                    self.mtlcmdencoder = StrongPtr::nil();
                }

                if self.mtlcmdbuffer.is_nil() {
                    self.mtlcmdbuffer =
                        StrongPtr::retain(msg_send![self.mtlcmdqueue, commandBuffer]);
                }

                let blitcmd: Id = msg_send![self.mtlcmdbuffer, blitCommandEncoder];

                let _: () = msg_send![blitcmd, copyFromBuffer: lockedbuffer
                                                 sourceOffset: 0 as NSUInteger
                                            sourceBytesPerRow: pitch as NSUInteger
                                          sourceBytesPerImage: 0 as NSUInteger
                                                   sourceSize: mtl_size_make(rect.w as u32, rect.h as u32, 1)
                                                    toTexture: main
                                             destinationSlice: 0 as NSUInteger
                                             destinationLevel: 0 as NSUInteger
                                            destinationOrigin: mtl_origin_make(rect.x as u32, rect.y as u32, 0)];

                if yuv {
                    let uvpitch = ((pitch / bpp + 1) / 2) * bpp;

                    let _: () = msg_send![blitcmd, copyFromBuffer: lockedbuffer
                                                     sourceOffset: (rect.h * pitch) as NSUInteger
                                                sourceBytesPerRow: uvpitch as NSUInteger
                                              sourceBytesPerImage: (uvpitch * uvrect.h) as NSUInteger
                                                       sourceSize: mtl_size_make(uvrect.w as u32, uvrect.h as u32, 1)
                                                        toTexture: firstplane
                                                 destinationSlice: 0 as NSUInteger
                                                 destinationLevel: 0 as NSUInteger
                                                destinationOrigin: mtl_origin_make(uvrect.x as u32, uvrect.y as u32, 0)];

                    let _: () = msg_send![blitcmd, copyFromBuffer: lockedbuffer
                                                     sourceOffset: ((rect.h * pitch) + uvrect.h * uvpitch) as NSUInteger
                                                sourceBytesPerRow: uvpitch as NSUInteger
                                              sourceBytesPerImage: (uvpitch * uvrect.h) as NSUInteger
                                                       sourceSize: mtl_size_make(uvrect.w as u32, uvrect.h as u32, 1)
                                                        toTexture: secondplane
                                                 destinationSlice: 0 as NSUInteger
                                                 destinationLevel: 0 as NSUInteger
                                                destinationOrigin: mtl_origin_make(uvrect.x as u32, uvrect.y as u32, 0)];
                }

                if nv12 {
                    let uvpitch = ((pitch / bpp + 1) / 2) * 2 * bpp;

                    let _: () = msg_send![blitcmd, copyFromBuffer: lockedbuffer
                                                     sourceOffset: (rect.h * pitch) as NSUInteger
                                                sourceBytesPerRow: uvpitch as NSUInteger
                                              sourceBytesPerImage: 0 as NSUInteger
                                                       sourceSize: mtl_size_make(uvrect.w as u32, uvrect.h as u32, 1)
                                                        toTexture: uvplane
                                                 destinationSlice: 0 as NSUInteger
                                                 destinationLevel: 0 as NSUInteger
                                                destinationOrigin: mtl_origin_make(uvrect.x as u32, uvrect.y as u32, 0)];
                }
                let _: () = msg_send![blitcmd, endEncoding];

                let _: () = msg_send![self.mtlcmdbuffer, commit];
                self.mtlcmdbuffer = StrongPtr::nil();
            }

            if let Some(t) = metal_texture_mut(texture) {
                t.lockedbuffer = StrongPtr::nil(); // Retained property, so it calls release.
                t.hasdata = true;
            }
        })
    }

    /// Translation of `METAL_SetRenderTarget()`.
    fn set_render_target(
        &mut self,
        target: Option<Texture>,
        textures: &TextureStore,
    ) -> Result<()> {
        autoreleasepool(|| {
            if !self.mtlcmdencoder.is_nil() {
                /* End encoding for the previous render target so we can set up a new
                 * render pass for this one. */
                // SAFETY: the renderer's encoder and command buffer.
                unsafe {
                    let _: () = msg_send![self.mtlcmdencoder, endEncoding];
                    let _: () = msg_send![self.mtlcmdbuffer, commit];
                }

                self.mtlcmdencoder = StrongPtr::nil();
                self.mtlcmdbuffer = StrongPtr::nil();
            }

            // (the front end's renderer->target, and SDL_SetRenderTarget()'s
            // current colorspace)
            match target {
                Some(t) => {
                    let texture = textures.get(t).ok_or_else(invalid_texture)?;
                    let texturedata = metal_texture(texture).ok_or_else(invalid_texture)?;
                    self.target_mtltexture = texturedata.mtltexture.clone();
                    self.target_size = (texture.w, texture.h);
                    self.current_colorspace = texture.colorspace;
                }
                None => {
                    self.target_mtltexture = StrongPtr::nil();
                    self.target_size = (0, 0);
                    self.current_colorspace = self.output_colorspace;
                }
            }
            self.target = target;

            /* We don't begin a new render pass right away - we delay it until an actual
             * draw or clear happens. That way we can use hardware clears when possible,
             * which are only available when beginning a new render pass. */
            Ok(())
        })
    }

    /// Translation of `METAL_RenderReadPixels()`.
    fn read_pixels(
        &mut self,
        rect: &Rect,
        _textures: &mut TextureStore,
    ) -> Option<Result<Surface<'static>>> {
        Some(autoreleasepool(|| self.render_read_pixels(rect)))
    }

    /// Translation of `METAL_RenderPresent()`.
    fn present(&mut self) -> bool {
        self.render_present()
    }

    /// Translation of `METAL_DestroyTexture()`.
    fn destroy_texture(&mut self, texture: &mut TextureData) {
        autoreleasepool(|| {
            texture.internal = None;
        })
    }

    /// Translation of `METAL_SetVSync()`.
    fn set_vsync(&mut self, vsync: i32) -> Option<Result<()>> {
        // SAFETY: the renderer's layer.
        Some(match vsync {
            0 => {
                let _: () = unsafe { msg_send![self.mtllayer, setDisplaySyncEnabled: NO] };
                Ok(())
            }
            1 => {
                let _: () = unsafe { msg_send![self.mtllayer, setDisplaySyncEnabled: YES] };
                Ok(())
            }
            _ => Err(Error::unsupported()),
        })
    }

    /// Translation of `METAL_WindowEvent()` (which does nothing), keeping
    /// `renderer->hidden` as `SDL_RendererEventWatch()` does.
    fn window_event(&mut self, event_type: EventType) {
        let flags = self.window.flags().unwrap_or_default();
        if event_type == EventType::WINDOW_HIDDEN || event_type == EventType::WINDOW_MINIMIZED {
            self.hidden = true;
        } else if event_type == EventType::WINDOW_SHOWN {
            if !flags.contains(WindowFlags::MINIMIZED) {
                self.hidden = false;
            }
        } else if (event_type == EventType::WINDOW_RESTORED
            || event_type == EventType::WINDOW_MAXIMIZED)
            && !flags.contains(WindowFlags::HIDDEN)
        {
            self.hidden = false;
        }
    }

    /// Translation of `METAL_DestroyRenderer()`.
    fn destroy(&mut self) {
        autoreleasepool(|| {
            // SAFETY: the renderer's encoder and command buffer.
            unsafe {
                if !self.mtlcmdencoder.is_nil() {
                    let _: () = msg_send![self.mtlcmdencoder, endEncoding];
                }

                if !self.mtlcmdbuffer.is_nil() {
                    let _: () = msg_send![self.mtlcmdbuffer, commit];
                    let _: () = msg_send![self.mtlcmdbuffer, waitUntilCompleted];
                }
            }
            self.mtlcmdencoder = StrongPtr::nil();
            self.mtlcmdbuffer = StrongPtr::nil();

            self.allpipelines.clear();
            self.activepipelines = None;

            /* Release the metal view instead of destroying it,
              in case we want to use it later (recreating the renderer)
            */
            // SDL_Metal_DestroyView(data.mtlview);
            if self.mtlview != 0 {
                // SAFETY: the reference the renderer owns.
                unsafe { release(self.mtlview as Id) };
                self.mtlview = 0;
            }
        })
    }

    fn metal_layer(&mut self) -> Option<usize> {
        Some(autoreleasepool(|| self.get_metal_layer()) as usize)
    }

    fn metal_command_encoder(&mut self) -> Option<usize> {
        Some(self.get_metal_command_encoder() as usize)
    }
}

impl Drop for MetalRenderer {
    fn drop(&mut self) {
        // (the front end calls destroy(); a renderer dropped without it
        // still gives its view back)
        if self.mtlview != 0 {
            self.destroy();
        }
    }
}
