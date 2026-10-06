// Rust translation of src/IMG_gpu.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Loading images into GPU textures: the image is loaded as a surface,
//! converted to RGBA32, and uploaded through a transfer buffer by a copy
//! pass the caller submits.
//!
//! (Upstream's checks for a NULL device or copy pass can't fail: they are
//! references.)

use std::path::Path;

use sdl3::error::{Error, Result};
use sdl3::gpu::{
    CopyPass, Device, Texture, TextureCreateInfo, TextureFormat, TextureRegion,
    TextureTransferInfo, TextureType, TextureUsageFlags, TransferBufferCreateInfo,
    TransferBufferUsage,
};
use sdl3::io::IoStream;
use sdl3::video::{PixelFormat, Surface};

/// Translation of `LoadGPUTexture()`: `surface` (a load's result) uploaded
/// to a new texture, with its width and height.
fn load_gpu_texture_surface(
    device: &Device,
    copy_pass: &mut CopyPass<'_>,
    surface: Result<Surface<'static>>,
) -> Result<(Texture, i32, i32)> {
    let mut surface = surface?;

    if surface.format() != PixelFormat::RGBA32 {
        surface = surface.convert(PixelFormat::RGBA32)?;
    }

    let texture_create_info = TextureCreateInfo {
        format: TextureFormat::R8G8B8A8_UNORM,
        texture_type: TextureType::Texture2D,
        layer_count_or_depth: 1,
        num_levels: 1,
        usage: TextureUsageFlags::SAMPLER | TextureUsageFlags::COMPUTE_STORAGE_READ,
        width: surface.width() as u32,
        height: surface.height() as u32,
        ..Default::default()
    };
    let texture = device.create_texture(&texture_create_info)?;

    let transfer_buffer_create_info = TransferBufferCreateInfo {
        size: surface
            .width()
            .wrapping_mul(surface.height())
            .wrapping_mul(4) as u32,
        usage: TransferBufferUsage::Upload,
        ..Default::default()
    };
    let mut transfer_buffer = device.create_transfer_buffer(&transfer_buffer_create_info)?;

    {
        let mut dst = transfer_buffer.map(false)?;
        let src = surface.pixels().unwrap_or(&[]);
        let pitch = surface.pitch() as usize;
        let row_bytes = surface.width() as usize * 4;
        let rows = surface.height() as usize;
        // (Upstream copies past the end of a transfer buffer whose size
        // overflowed; texture size limits keep that from happening.)
        if rows > 0 && (dst.len() < row_bytes * rows || src.len() < pitch * (rows - 1) + row_bytes)
        {
            return Err(Error::out_of_memory());
        }
        if row_bytes == pitch {
            dst[..row_bytes * rows].copy_from_slice(&src[..row_bytes * rows]);
        } else {
            for y in 0..rows {
                dst[y * row_bytes..(y + 1) * row_bytes]
                    .copy_from_slice(&src[y * pitch..y * pitch + row_bytes]);
            }
        }
        // SDL_UnmapGPUTransferBuffer(): the mapping's drop
    }

    let texture_transfer_info = TextureTransferInfo {
        transfer_buffer: &transfer_buffer,
        offset: 0,
        pixels_per_row: 0,
        rows_per_layer: 0,
    };
    let texture_region = TextureRegion {
        texture: &texture,
        mip_level: 0,
        layer: 0,
        x: 0,
        y: 0,
        z: 0,
        w: surface.width() as u32,
        h: surface.height() as u32,
        d: 1,
    };
    copy_pass.upload_to_texture(&texture_transfer_info, &texture_region, false);

    let (width, height) = (surface.width(), surface.height());

    // The surface and the transfer buffer are released here (the upload
    // keeps what it needs until the command buffer is done).
    drop(surface);
    drop(transfer_buffer);

    Ok((texture, width, height))
}

/// Load an image from a filesystem path into a GPU texture: a texture of
/// format [`TextureFormat::R8G8B8A8_UNORM`] with one mip level, usable as a
/// sampled texture in graphics and compute pipelines and as a read-only
/// storage texture in compute pipelines. Returns the texture with its
/// width and height. Translation of `IMG_LoadGPUTexture()`.
///
/// The upload is recorded into `copy_pass`; the texture holds the image
/// once the pass's command buffer is submitted and done.
pub fn load_gpu_texture(
    device: &Device,
    copy_pass: &mut CopyPass<'_>,
    file: impl AsRef<Path>,
) -> Result<(Texture, i32, i32)> {
    load_gpu_texture_surface(device, copy_pass, crate::load(file))
}

/// Load an image from a data source into a GPU texture (see
/// [`load_gpu_texture`]). Translation of `IMG_LoadGPUTexture_IO()`.
pub fn load_gpu_texture_io(
    device: &Device,
    copy_pass: &mut CopyPass<'_>,
    src: &mut IoStream<'_>,
) -> Result<(Texture, i32, i32)> {
    load_gpu_texture_typed_io(device, copy_pass, src, None)
}

/// Load an image of an optionally specified type from a data source into
/// a GPU texture (see [`load_gpu_texture`]). Translation of
/// `IMG_LoadGPUTextureTyped_IO()`.
pub fn load_gpu_texture_typed_io(
    device: &Device,
    copy_pass: &mut CopyPass<'_>,
    src: &mut IoStream<'_>,
    type_: Option<&str>,
) -> Result<(Texture, i32, i32)> {
    load_gpu_texture_surface(device, copy_pass, crate::load_typed_io(src, type_))
}
