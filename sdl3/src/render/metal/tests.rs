// Tests of the Metal renderer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What can be checked without a window: the tables and constants against
//! upstream's values. The renderer itself needs a Cocoa window, which only
//! the main thread can make, or a UIKit one, which only an app can:
//! `tests/cocoa_metal.rs` and `tests/uikit_metal.rs` draw with it.

use super::*;

#[test]
fn driver_list_follows_upstreams_order() {
    use crate::render::{num_render_drivers, render_driver, SOFTWARE_RENDERER};
    // (render_drivers[] in SDL_render.c: ..., METAL, ..., GL, GLES2, ..., VULKAN, GPU, SW)
    let names: Vec<_> = (0..num_render_drivers())
        .map(|i| render_driver(i).unwrap())
        .collect();
    #[cfg(target_os = "macos")]
    assert_eq!(
        names,
        [
            "metal",
            "opengl",
            "opengles2",
            "vulkan",
            "gpu",
            SOFTWARE_RENDERER
        ]
    );
    // (no desktop OpenGL on UIKit; upstream's "opengles", the OpenGL ES 1
    // renderer after "opengles2", isn't translated)
    #[cfg(not(any(target_os = "macos", target_os = "visionos")))]
    assert_eq!(
        names,
        ["metal", "opengles2", "vulkan", "gpu", SOFTWARE_RENDERER]
    );
    // (no OpenGL ES on visionOS)
    #[cfg(target_os = "visionos")]
    assert_eq!(names, ["metal", "vulkan", "gpu", SOFTWARE_RENDERER]);
}

#[test]
#[cfg(any(target_os = "macos", target_abi = "sim", target_abi = "macabi"))]
fn constant_buffer_layout() {
    // CONSTANT_ALIGN() is 256 on macOS (and the simulators and Mac
    // Catalyst): each matrix starts on its own 256-byte boundary.
    assert_eq!(CONSTANTS_OFFSET_IDENTITY, 0);
    assert_eq!(CONSTANTS_OFFSET_HALF_PIXEL_TRANSFORM, 256);
    assert_eq!(CONSTANTS_OFFSET_DECODE_BT601_LIMITED, 512);
    assert_eq!(CONSTANTS_OFFSET_DECODE_BT601_FULL, 768);
    assert_eq!(CONSTANTS_OFFSET_DECODE_BT709_LIMITED, 1024);
    assert_eq!(CONSTANTS_OFFSET_DECODE_BT709_FULL, 1280);
    assert_eq!(CONSTANTS_OFFSET_DECODE_BT2020_LIMITED, 1536);
    assert_eq!(CONSTANTS_OFFSET_DECODE_BT2020_FULL, 1792);
    assert_eq!(CONSTANTS_LENGTH, 1856);
    assert_eq!(device_align(8), 8);
    assert_eq!(device_align(2), 4);
    assert_eq!(size_of::<PixelShaderConstants>(), 48);
}

#[test]
#[cfg(not(any(target_os = "macos", target_abi = "sim", target_abi = "macabi")))]
fn constant_buffer_layout() {
    // On the UIKit platforms' devices CONSTANT_ALIGN() is the type's
    // alignment (16 for the matrices): they follow one another.
    assert_eq!(constant_align(16), 16);
    assert_eq!(constant_align(2), 4);
    assert_eq!(CONSTANTS_OFFSET_IDENTITY, 0);
    assert_eq!(CONSTANTS_OFFSET_HALF_PIXEL_TRANSFORM, 64);
    assert_eq!(CONSTANTS_OFFSET_DECODE_BT601_LIMITED, 128);
    assert_eq!(CONSTANTS_OFFSET_DECODE_BT601_FULL, 192);
    assert_eq!(CONSTANTS_OFFSET_DECODE_BT709_LIMITED, 256);
    assert_eq!(CONSTANTS_OFFSET_DECODE_BT709_FULL, 320);
    assert_eq!(CONSTANTS_OFFSET_DECODE_BT2020_LIMITED, 384);
    assert_eq!(CONSTANTS_OFFSET_DECODE_BT2020_FULL, 448);
    assert_eq!(CONSTANTS_LENGTH, 512);
    assert_eq!(size_of::<PixelShaderConstants>(), 48);
}

#[test]
fn ycbcr_matrix_offsets() {
    use crate::video::pixels::Colorspace as C;
    let m = get_ycbcr_to_rgb_conversion_matrix;
    assert_eq!(
        m(C::BT601_LIMITED, 16, 16, 8),
        CONSTANTS_OFFSET_DECODE_BT601_LIMITED
    );
    assert_eq!(
        m(C::BT601_FULL, 16, 16, 8),
        CONSTANTS_OFFSET_DECODE_BT601_FULL
    );
    assert_eq!(
        m(C::BT709_LIMITED, 16, 16, 8),
        CONSTANTS_OFFSET_DECODE_BT709_LIMITED
    );
    assert_eq!(
        m(C::BT709_FULL, 16, 16, 8),
        CONSTANTS_OFFSET_DECODE_BT709_FULL
    );
    assert_eq!(
        m(C::BT2020_LIMITED, 16, 16, 10),
        CONSTANTS_OFFSET_DECODE_BT2020_LIMITED
    );
    assert_eq!(
        m(C::BT2020_FULL, 16, 16, 10),
        CONSTANTS_OFFSET_DECODE_BT2020_FULL
    );
    // JPEG is full-range BT.601
    assert_eq!(m(C::JPEG, 16, 16, 8), CONSTANTS_OFFSET_DECODE_BT601_FULL);
    // RGB colorspaces have no matrix
    assert_eq!(m(C::SRGB, 16, 16, 8), 0);
    // The BT.601 limited matrix of SDL_GetYCbCRtoRGBConversionMatrix()
    let bt601 = ycbcr_shader_matrix(C::BT601_LIMITED, 8);
    assert_eq!(bt601[3], 0.0);
    assert!((bt601[0] + 0.062_745_1).abs() < 1e-6, "{bt601:?}");
    assert!((bt601[4] - 1.1644).abs() < 1e-4, "{bt601:?}");
}

#[test]
fn blend_tables() {
    let custom = BlendMode::compose_custom(
        BlendFactor::One,
        BlendFactor::Zero,
        BlendOperation::Maximum,
        BlendFactor::DstAlpha,
        BlendFactor::OneMinusSrcColor,
        BlendOperation::RevSubtract,
    );
    assert_eq!(
        get_blend_factor(custom.src_color_factor()),
        MTL_BLEND_FACTOR_ONE
    );
    assert_eq!(
        get_blend_operation(custom.color_operation()),
        MTL_BLEND_OPERATION_MAX
    );
    assert_eq!(
        get_blend_factor(custom.dst_alpha_factor()),
        MTL_BLEND_FACTOR_ONE_MINUS_SOURCE_COLOR
    );
    assert_eq!(get_blend_factor(None), INVALID_BLEND_FACTOR);
    assert_eq!(get_blend_operation(None), INVALID_BLEND_OPERATION);
    assert_eq!(
        render_sampler_hashkey(
            ScaleMode::Nearest,
            TextureAddressMode::Wrap,
            TextureAddressMode::Clamp
        ),
        3
    );
    assert_eq!(
        render_sampler_hashkey(
            ScaleMode::Linear,
            TextureAddressMode::Clamp,
            TextureAddressMode::Wrap
        ),
        4
    );
}

#[test]
fn shader_library() {
    // The size of upstream's sdl_metallib for the platform, with the Metal
    // library magic.
    let size = if cfg!(target_os = "macos") {
        38817 // SDL_shaders_metal_macos.h
    } else if cfg!(all(target_os = "tvos", target_abi = "sim")) {
        51031 // SDL_shaders_metal_tvsimulator.h
    } else if cfg!(target_abi = "sim") {
        51015 // SDL_shaders_metal_iphonesimulator.h
    } else {
        41905 // SDL_shaders_metal_ios.h, SDL_shaders_metal_tvos.h
    };
    assert_eq!(shaders::SDL_METALLIB.len(), size);
    assert_eq!(&shaders::SDL_METALLIB[..4], b"MTLB");
    let has = |name: &str| {
        let name = name.as_bytes();
        shaders::SDL_METALLIB.windows(name.len()).any(|w| w == name)
    };
    for f in [VertexFunction::Solid, VertexFunction::Copy] {
        assert!(has(get_vertex_function_name(f)));
    }
    for f in [
        FragmentFunction::Solid,
        FragmentFunction::Palette,
        FragmentFunction::Copy,
        FragmentFunction::Yuv,
        FragmentFunction::Nv12,
    ] {
        assert!(has(get_fragment_function_name(f)));
    }
}
