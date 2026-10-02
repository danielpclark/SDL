// Rust translation of include/SDL3/SDL_blendmode.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Blend modes used by surfaces and (later) the renderer.

/// A set of blend modes used in drawing operations. Translation of `SDL_BlendMode`.
///
/// The predefined modes are associated constants. Custom modes (built by the
/// renderer's `SDL_ComposeCustomBlendMode()`) are other bit patterns, which
/// is why this is a newtype over `u32` and not an enum.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BlendMode(pub u32);

impl BlendMode {
    /// no blending: dstRGBA = srcRGBA
    pub const NONE: BlendMode = BlendMode(0x00000000);
    /// alpha blending: dstRGB = (srcRGB * srcA) + (dstRGB * (1-srcA)), dstA = srcA + (dstA * (1-srcA))
    pub const BLEND: BlendMode = BlendMode(0x00000001);
    /// pre-multiplied alpha blending: dstRGBA = srcRGBA + (dstRGBA * (1-srcA))
    pub const BLEND_PREMULTIPLIED: BlendMode = BlendMode(0x00000010);
    /// additive blending: dstRGB = (srcRGB * srcA) + dstRGB, dstA = dstA
    pub const ADD: BlendMode = BlendMode(0x00000002);
    /// pre-multiplied additive blending: dstRGB = srcRGB + dstRGB, dstA = dstA
    pub const ADD_PREMULTIPLIED: BlendMode = BlendMode(0x00000020);
    /// color modulate: dstRGB = srcRGB * dstRGB, dstA = dstA
    pub const MOD: BlendMode = BlendMode(0x00000004);
    /// color multiply: dstRGB = (srcRGB * dstRGB) + (dstRGB * (1-srcA)), dstA = dstA
    pub const MUL: BlendMode = BlendMode(0x00000008);
    pub const INVALID: BlendMode = BlendMode(0x7FFFFFFF);

    /// The constant's name, or `None` for a custom mode.
    pub fn name(self) -> Option<&'static str> {
        Some(match self {
            BlendMode::NONE => "NONE",
            BlendMode::BLEND => "BLEND",
            BlendMode::BLEND_PREMULTIPLIED => "BLEND_PREMULTIPLIED",
            BlendMode::ADD => "ADD",
            BlendMode::ADD_PREMULTIPLIED => "ADD_PREMULTIPLIED",
            BlendMode::MOD => "MOD",
            BlendMode::MUL => "MUL",
            BlendMode::INVALID => "INVALID",
            _ => return None,
        })
    }
}

impl std::fmt::Debug for BlendMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.name() {
            Some(n) => write!(f, "BlendMode::{n}"),
            None => write!(f, "BlendMode({:#010x})", self.0),
        }
    }
}

/// The blend operation used when combining source and destination pixel
/// components. Translation of `SDL_BlendOperation`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u32)]
pub enum BlendOperation {
    /// dst + src: supported by all renderers
    Add = 0x1,
    /// src - dst : supported by D3D, OpenGL, OpenGLES, and Vulkan
    Subtract = 0x2,
    /// dst - src : supported by D3D, OpenGL, OpenGLES, and Vulkan
    RevSubtract = 0x3,
    /// min(dst, src) : supported by D3D, OpenGL, OpenGLES, and Vulkan
    Minimum = 0x4,
    /// max(dst, src) : supported by D3D, OpenGL, OpenGLES, and Vulkan
    Maximum = 0x5,
}

/// The normalized factor used to multiply pixel components. Translation of `SDL_BlendFactor`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u32)]
pub enum BlendFactor {
    /// 0, 0, 0, 0
    Zero = 0x1,
    /// 1, 1, 1, 1
    One = 0x2,
    /// srcR, srcG, srcB, srcA
    SrcColor = 0x3,
    /// 1-srcR, 1-srcG, 1-srcB, 1-srcA
    OneMinusSrcColor = 0x4,
    /// srcA, srcA, srcA, srcA
    SrcAlpha = 0x5,
    /// 1-srcA, 1-srcA, 1-srcA, 1-srcA
    OneMinusSrcAlpha = 0x6,
    /// dstR, dstG, dstB, dstA
    DstColor = 0x7,
    /// 1-dstR, 1-dstG, 1-dstB, 1-dstA
    OneMinusDstColor = 0x8,
    /// dstA, dstA, dstA, dstA
    DstAlpha = 0x9,
    /// 1-dstA, 1-dstA, 1-dstA, 1-dstA
    OneMinusDstAlpha = 0xA,
}
