// Rust translation of src/SDL_gpu_textengine.c from SDL_ttf.
// Copyright (C) 2001-2025 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The text engine for drawing text with the SDL GPU API.
//!
//! Translation notes: as in the renderer text engine
//! (`renderer_textengine.rs`), the glyphs are indices into an arena of the
//! engine, the atlases are a list in creation order, and the glyph hash
//! tables keep insertion order. The atlas textures are shared (`Rc`) with
//! the draw data handed out by [`gpu_text_draw_data`].

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use sdl3::gpu::{
    ColorTargetInfo, Device, LoadOp, Texture, TextureCreateInfo, TextureFormat, TextureRegion,
    TextureTransferInfo, TextureType, TextureUsageFlags, TransferBufferCreateInfo,
    TransferBufferUsage,
};
use sdl3::video::{FColor, FPoint, Rect, Surface};
use sdl3::{Error, Result};

use crate::qsort::sdl_qsort;
use crate::stb_rect_pack::*;
use crate::text::*;
use crate::ttf::ImageType;

/// The winding order of the vertices returned by [`gpu_text_draw_data`].
/// Translation of `TTF_GPUTextEngineWinding`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GpuTextEngineWinding {
    /// `TTF_GPU_TEXTENGINE_WINDING_INVALID`
    Invalid = -1,
    /// `TTF_GPU_TEXTENGINE_WINDING_CLOCKWISE`
    Clockwise = 0,
    /// `TTF_GPU_TEXTENGINE_WINDING_COUNTER_CLOCKWISE`
    CounterClockwise = 1,
}

/// Draw sequence returned by [`gpu_text_draw_data`]. Translation of
/// `TTF_GPUAtlasDrawSequence` (the sequences are a slice instead of a
/// linked list).
#[derive(Debug, Clone)]
pub struct GpuAtlasDrawSequence {
    /// Texture atlas that stores the glyphs (`None` for filled
    /// rectangles)
    pub atlas_texture: Option<Rc<Texture>>,
    /// An array of vertex positions
    pub xy: Vec<FPoint>,
    /// An array of normalized texture coordinates for each vertex (empty
    /// without an atlas texture)
    pub uv: Vec<FPoint>,
    /// Number of vertices
    pub num_vertices: i32,
    /// An array of indices into the 'vertices' arrays
    pub indices: Vec<i32>,
    /// Number of indices
    pub num_indices: i32,
    /// The image type of this draw sequence
    pub image_type: ImageType,
}

/// `GlyphSurface`
#[derive(Debug)]
struct GlyphSurface {
    surface: Option<Surface<'static>>,
    image_type: ImageType,
}

/// `AtlasGlyph` (`next`, the free list link, is the atlas's list)
#[derive(Debug, Clone)]
struct AtlasGlyph {
    refcount: i32,
    atlas: usize,
    image_type: ImageType,
    rect: Rect,
    texcoords: [f32; 8],
}

/// `AtlasTexture` (`packing_nodes` are the packer's, `next` is the next
/// atlas of the engine's list)
struct AtlasTexture {
    texture: Rc<Texture>,
    packer: StbrpContext,
    free_glyphs: Vec<usize>,
}

impl std::fmt::Debug for AtlasTexture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AtlasTexture")
            .field("free_glyphs", &self.free_glyphs)
            .finish_non_exhaustive()
    }
}

/// `TTF_GPUTextEngineTextData`
#[derive(Debug)]
struct GpuTextEngineTextData {
    glyphs: Vec<usize>,
    draw_sequence: Rc<[GpuAtlasDrawSequence]>,
}

/// A glyph hash table (`SDL_CreateGlyphHashTable(NukeGlyph)`), in
/// insertion order.
#[derive(Debug, Default)]
struct GlyphHashTable {
    index: HashMap<(FontHandle, u32), usize>,
    entries: Vec<((FontHandle, u32), usize)>,
}

impl GlyphHashTable {
    /// `SDL_FindInGlyphHashTable`
    fn find(&self, font: &FontHandle, glyph_index: u32) -> Option<usize> {
        self.index
            .get(&(font.clone(), glyph_index))
            .map(|&i| self.entries[i].1)
    }
}

/// `TTF_GPUTextEngineFontData`
#[derive(Debug)]
struct GpuTextEngineFontData {
    /* font: the key */
    generation: u32,
    glyphs: GlyphHashTable,
}

/// `TTF_GPUTextEngineData` (but the device)
#[derive(Debug)]
struct EngineData {
    fonts: HashMap<FontHandle, GpuTextEngineFontData>,
    atlas: Vec<AtlasTexture>,
    atlas_texture_size: i32,
    winding: GpuTextEngineWinding,
    /// the glyphs (C allocates each)
    glyph_arena: Vec<AtlasGlyph>,
}

/// The text engine for drawing text with the SDL GPU API
/// (`TTF_GPUTextEngineData`, as the `userdata` of a `TTF_TextEngine`).
#[derive(Debug)]
pub struct GpuTextEngine {
    device: Device,
    data: RefCell<EngineData>,
}

/// `SortMissing`
fn sort_missing(reserved: &[Option<usize>], a: &StbrpRect, b: &StbrpRect) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;

    // Sort missing first
    if reserved[a.id as usize].is_none() && reserved[b.id as usize].is_some() {
        return Less;
    }
    if reserved[b.id as usize].is_none() && reserved[a.id as usize].is_some() {
        return Greater;
    }

    // Sort largest first
    if a.w != b.w {
        if a.w > b.w {
            return Less;
        } else {
            return Greater;
        }
    }
    if a.h != b.h {
        if a.h > b.h {
            return Less;
        } else {
            return Greater;
        }
    }

    // It doesn't matter, sort by ID
    if a.id < b.id {
        Less
    } else {
        Greater
    }
}

/// `SortOperations`
fn sort_operations(
    glyphs: &[AtlasGlyph],
    a: &(DrawOperation, Option<usize>),
    b: &(DrawOperation, Option<usize>),
) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;

    let a_copy = matches!(a.0, DrawOperation::Copy(_));
    let b_copy = matches!(b.0, DrawOperation::Copy(_));
    if a_copy && b_copy {
        let glyph_a = &glyphs[a.1.expect("glyph")];
        let glyph_b = &glyphs[b.1.expect("glyph")];
        if glyph_a.atlas != glyph_b.atlas {
            // It's not important how we sort this, just that it's consistent
            // (C compares the atlas pointers; these are the creation order)
            return if glyph_a.atlas < glyph_b.atlas {
                Less
            } else {
                Greater
            };
        }

        // We could sort by texture coordinate or whatever, if we cared.
        return Equal;
    }

    if a_copy {
        return Less;
    }
    if b_copy {
        return Greater;
    }
    Equal
}

/* DestroyGlyph: the glyphs live in the engine's arena */
/* DestroyAtlas: dropping it (SDL_ReleaseGPUTexture() when the last
reference to the texture goes) */

/// `CreateAtlas`
fn create_atlas(device: &Device, atlas_texture_size: i32) -> Result<AtlasTexture> {
    let info = TextureCreateInfo {
        texture_type: TextureType::Texture2D,
        format: TextureFormat::B8G8R8A8_UNORM,
        usage: TextureUsageFlags::SAMPLER | TextureUsageFlags::COLOR_TARGET,
        width: atlas_texture_size as u32,
        height: atlas_texture_size as u32,
        layer_count_or_depth: 1,
        num_levels: 1,
        ..TextureCreateInfo::default()
    };

    let texture = device.create_texture(&info)?;

    let mut target_info = ColorTargetInfo::new(&texture);
    target_info.clear_color = FColor::new(0.0, 0.0, 0.0, 0.0);
    target_info.load_op = LoadOp::Clear;

    // (C doesn't check these)
    if let Ok(mut cbuf) = device.acquire_command_buffer() {
        if let Ok(rpass) = cbuf.begin_render_pass(&[target_info], None) {
            rpass.end();
        }
        let _ = cbuf.submit();
    }

    let num_nodes = atlas_texture_size / 4;
    if num_nodes == 0 {
        // FIXME (upstream): stbrp_init_target() divides by the number of
        // nodes (and writes past the empty node array) for atlas sizes
        // below 4; fail instead.
        return Err(Error::new(
            "Failed to create GPU text engine: Invalid texture atlas size.",
        ));
    }
    let mut packer = StbrpContext::default();
    if stbrp_init_target(
        &mut packer,
        atlas_texture_size,
        atlas_texture_size,
        num_nodes,
    )
    .is_err()
    {
        return Err(Error::out_of_memory());
    }
    stbrp_setup_heuristic(&mut packer, STBRP_HEURISTIC_SKYLINE_DEFAULT);

    Ok(AtlasTexture {
        texture: Rc::new(texture),
        packer,
        free_glyphs: Vec::new(),
    })
}

/// `ReleaseGlyph`
fn release_glyph(enginedata: &mut EngineData, glyph: usize) {
    let g = &mut enginedata.glyph_arena[glyph];
    g.refcount -= 1;
    if g.refcount == 0 {
        // (the glyphs always have an atlas)
        let atlas = &mut enginedata.atlas[g.atlas];
        // Insert into free list sorted smallest first
        let size = g.rect.w * g.rect.h;
        let mut pos = atlas.free_glyphs.len();
        for (i, &entry) in atlas.free_glyphs.iter().enumerate() {
            let entry = &enginedata.glyph_arena[entry].rect;
            if size <= entry.w * entry.h {
                pos = i;
                break;
            }
        }
        atlas.free_glyphs.insert(pos, glyph);
    }
}

/// `CreateGlyph`
fn create_glyph(
    enginedata: &mut EngineData,
    atlas: usize,
    atlas_texture_size: i32,
    area: &StbrpRect,
) -> Result<usize> {
    if enginedata.glyph_arena.try_reserve(1).is_err() {
        return Err(Error::out_of_memory());
    }

    // Remove the one pixel extra padding between glyphs
    let rect = Rect::new(area.x, area.y, area.w - 1, area.h - 1);

    let minu = rect.x as f32 / atlas_texture_size as f32;
    let minv = rect.y as f32 / atlas_texture_size as f32;
    let maxu = (rect.x + rect.w) as f32 / atlas_texture_size as f32;
    let maxv = (rect.y + rect.h) as f32 / atlas_texture_size as f32;
    let glyph = AtlasGlyph {
        refcount: 1,
        atlas,
        image_type: ImageType::Invalid,
        rect,
        texcoords: [minu, minv, maxu, minv, maxu, maxv, minu, maxv],
    };

    enginedata.glyph_arena.push(glyph);
    Ok(enginedata.glyph_arena.len() - 1)
}

/// `FindUnusedGlyph`
fn find_unused_glyph(
    enginedata: &mut EngineData,
    atlas: usize,
    width: i32,
    height: i32,
) -> Option<usize> {
    let size = width * height;
    let free_glyphs = &enginedata.atlas[atlas].free_glyphs;
    let mut found = None;
    for (i, &glyph) in free_glyphs.iter().enumerate() {
        let rect = enginedata.glyph_arena[glyph].rect;
        if width == rect.w && height == rect.h {
            found = Some(i);
            break;
        }

        if size < rect.w * rect.h {
            // We didn't find any entries our size, everything else is larger than we want
            break;
        }
    }
    if let Some(i) = found {
        let glyph = enginedata.atlas[atlas].free_glyphs.remove(i);
        enginedata.glyph_arena[glyph].refcount += 1;
        return Some(glyph);
    }

    if atlas + 1 < enginedata.atlas.len() {
        return find_unused_glyph(enginedata, atlas + 1, width, height);
    }
    None
}

/// `UpdateGPUTexture`
fn update_gpu_texture(
    device: &Device,
    texture: &Texture,
    rect: &Rect,
    pixels: &[u8],
    pitch: i32,
) -> Result<()> {
    const TEXTUREBPP: usize = 4;

    let (Some(row_size), true) = ((rect.w as usize).checked_mul(TEXTUREBPP), rect.w >= 0) else {
        return Err(Error::new("update size overflow"));
    };
    let (Some(data_size), true) = ((rect.h as usize).checked_mul(row_size), rect.h >= 0) else {
        return Err(Error::new("update size overflow"));
    };

    let tbci = TransferBufferCreateInfo {
        size: data_size as u32,
        usage: TransferBufferUsage::Upload,
        ..TransferBufferCreateInfo::default()
    };

    let mut tbuf = device.create_transfer_buffer(&tbci)?;

    {
        let mut output = tbuf.map(false)?;
        let output = &mut output[..];

        // FIXME (upstream): a reused atlas area can be a pixel wider and
        // taller than the glyph (FindUnusedGlyph() compares the padded size
        // of the glyph with the unpadded size of the area), and C then reads
        // past the glyph's pixels; the bytes past them are zeros here.
        let copy = |dst: &mut [u8], src_offset: usize| {
            let src = pixels.get(src_offset..).unwrap_or(&[]);
            let n = dst.len().min(src.len());
            dst[..n].copy_from_slice(&src[..n]);
            dst[n..].fill(0);
        };
        if pitch as usize == row_size {
            let n = data_size.min(output.len());
            copy(&mut output[..n], 0);
        } else {
            // FIXME is negative pitch supposed to work?
            // If not, maybe use SDL_GPUTextureTransferInfo::pixels_per_row instead of this
            for i in 0..rect.h as usize {
                let Some(dst) = output.get_mut(i * row_size..(i + 1) * row_size) else {
                    break;
                };
                copy(dst, i.wrapping_mul(pitch as usize));
            }
        }
        // SDL_UnmapGPUTransferBuffer(): dropping the mapping
    }

    let mut cbuf = device.acquire_command_buffer()?;
    {
        let mut cpass = cbuf.begin_copy_pass()?;

        let tex_src = TextureTransferInfo {
            transfer_buffer: &tbuf,
            offset: 0,
            rows_per_layer: rect.h as u32,
            pixels_per_row: rect.w as u32,
        };

        let tex_dst = TextureRegion {
            texture,
            mip_level: 0,
            layer: 0,
            x: rect.x as u32,
            y: rect.y as u32,
            z: 0,
            w: rect.w as u32,
            h: rect.h as u32,
            d: 1,
        };

        cpass.upload_to_texture(&tex_src, &tex_dst, false);
        cpass.end();
    }
    // (SDL_ReleaseGPUTransferBuffer(): dropping it, after the submission)
    cbuf.submit()?;
    drop(tbuf);

    Ok(())
}

/// `UpdateGlyph`
fn update_glyph(
    enginedata: &mut EngineData,
    device: &Device,
    glyph: usize,
    surface: &Surface<'_>,
    image_type: ImageType,
) -> Result<()> {
    // SDL_assert(glyph->rect.w > 0 && glyph->rect.h > 0);
    let g = &enginedata.glyph_arena[glyph];

    /* FIXME: We should update the whole texture at once or at least cache the transfer buffers */
    let _ = update_gpu_texture(
        device,
        &enginedata.atlas[g.atlas].texture,
        &g.rect,
        surface.pixels().unwrap_or(&[]),
        surface.pitch(),
    );
    enginedata.glyph_arena[glyph].image_type = image_type;
    Ok(())
}

/// `AddGlyphToFont`
fn add_glyph_to_font(
    enginedata: &mut EngineData,
    font: &FontHandle,
    glyph_font: &FontHandle,
    glyph_index: u32,
    glyph: usize,
) -> Result<()> {
    // SDL_InsertIntoGlyphHashTable(), replacing (and releasing) any entry
    let key = (glyph_font.clone(), glyph_index);
    let fontdata = enginedata.fonts.get_mut(font).expect("font data");
    let table = &mut fontdata.glyphs;
    if let Some(&i) = table.index.get(&key) {
        let old = table.entries[i].1;
        table.entries[i].1 = glyph;
        release_glyph(enginedata, old);
        return Ok(());
    }
    if table.entries.try_reserve(1).is_err() || table.index.try_reserve(1).is_err() {
        return Err(Error::out_of_memory());
    }
    table.index.insert(key.clone(), table.entries.len());
    table.entries.push((key, glyph));
    Ok(())
}

/// The operations being turned into a text representation: the copies
/// of the text's draw operations, with their glyphs (`reserved`).
type Ops = Vec<(DrawOperation, Option<usize>)>;

/// The glyph font and index of a copy operation.
fn copy_glyph(op: &DrawOperation) -> Option<(&FontHandle, u32)> {
    match op {
        DrawOperation::Copy(copy) => Some((&copy.glyph_font, copy.glyph_index)),
        _ => None,
    }
}

/// `ResolveMissingGlyphs`
#[allow(clippy::too_many_arguments)]
fn resolve_missing_glyphs(
    enginedata: &mut EngineData,
    device: &Device,
    atlas: usize,
    font: &FontHandle,
    surfaces: &[GlyphSurface],
    ops: &mut Ops,
    missing: &mut Vec<StbrpRect>,
) -> Result<()> {
    // See if we can reuse any existing entries
    if !enginedata.atlas[atlas].free_glyphs.is_empty() {
        // Search from the smallest to the largest to minimize time spent searching the free list and shortening the missing entries
        let mut i = missing.len();
        while i > 0 {
            i -= 1;
            let Some(glyph) = find_unused_glyph(enginedata, atlas, missing[i].w, missing[i].h)
            else {
                continue;
            };

            let id = missing[i].id as usize;
            let surface = &surfaces[id];
            let Some(s) = surface.surface.as_ref() else {
                release_glyph(enginedata, glyph);
                return Err(Error::invalid_param("surface"));
            };
            if let Err(e) = update_glyph(enginedata, device, glyph, s, surface.image_type) {
                release_glyph(enginedata, glyph);
                return Err(e);
            }

            let (glyph_font, glyph_index) = copy_glyph(&ops[id].0).expect("copy operation");
            let glyph_font = glyph_font.clone();
            if let Err(e) = add_glyph_to_font(enginedata, font, &glyph_font, glyph_index, glyph) {
                release_glyph(enginedata, glyph);
                return Err(e);
            }

            ops[id].1 = Some(glyph);

            // Remove this from the missing entries
            // (C moves the rest with SDL_memcpy(), on overlapping memory)
            missing.remove(i);
        }
        if missing.is_empty() {
            return Ok(());
        }
    }

    // Try to pack all the missing glyphs into the current atlas
    let all_packed = stbrp_pack_rects(&mut enginedata.atlas[atlas].packer, missing) == 1;

    let atlas_texture_size = enginedata.atlas_texture_size;
    for m in missing.iter() {
        if m.was_packed == 0 {
            continue;
        }

        let glyph = create_glyph(enginedata, atlas, atlas_texture_size, m)?;

        let id = m.id as usize;
        let surface = &surfaces[id];
        let Some(s) = surface.surface.as_ref() else {
            release_glyph(enginedata, glyph);
            return Err(Error::invalid_param("surface"));
        };
        if let Err(e) = update_glyph(enginedata, device, glyph, s, surface.image_type) {
            release_glyph(enginedata, glyph);
            return Err(e);
        }

        let (glyph_font, glyph_index) = copy_glyph(&ops[id].0).expect("copy operation");
        let glyph_font = glyph_font.clone();
        if let Err(e) = add_glyph_to_font(enginedata, font, &glyph_font, glyph_index, glyph) {
            release_glyph(enginedata, glyph);
            return Err(e);
        }

        ops[id].1 = Some(glyph);
    }

    if all_packed {
        return Ok(());
    }

    // Sort the remaining missing glyphs and try in the next atlas
    let reserved: Vec<Option<usize>> = ops.iter().map(|op| op.1).collect();
    sdl_qsort(missing, |a, b| sort_missing(&reserved, a, b));
    if let Some(i) = missing.iter().position(|m| ops[m.id as usize].1.is_some()) {
        // No longer missing!
        missing.truncate(i);
    }

    if atlas + 1 >= enginedata.atlas.len() {
        let next = create_atlas(device, atlas_texture_size)?;
        enginedata.atlas.push(next);
    }
    resolve_missing_glyphs(enginedata, device, atlas + 1, font, surfaces, ops, missing)
}

/// `CreateMissingGlyphs`
fn create_missing_glyphs(
    enginedata: &mut EngineData,
    device: &Device,
    font: &FontHandle,
    ops: &mut Ops,
    num_missing: usize,
) -> Result<()> {
    let atlas_texture_size = enginedata.atlas_texture_size;

    // Build a list of missing glyphs
    let mut missing: Vec<StbrpRect> = Vec::new();
    if missing.try_reserve_exact(num_missing).is_err() {
        return Err(Error::out_of_memory());
    }

    let mut surfaces: Vec<GlyphSurface> = Vec::new();
    if surfaces.try_reserve_exact(ops.len()).is_err() {
        return Err(Error::out_of_memory());
    }
    surfaces.resize_with(ops.len(), || GlyphSurface {
        surface: None,
        image_type: ImageType::Invalid,
    });

    let mut checked: std::collections::HashSet<(FontHandle, u32)> = Default::default();

    for i in 0..ops.len() {
        let op = &ops[i];
        if let (DrawOperation::Copy(copy), None) = (&op.0, op.1) {
            let glyph_font = &copy.glyph_font;
            let glyph_index = copy.glyph_index;
            let key = (glyph_font.clone(), glyph_index);
            if checked.contains(&key) {
                continue;
            }
            if checked.try_reserve(1).is_err() {
                return Err(Error::out_of_memory());
            }
            checked.insert(key);

            let (surface, image_type) = glyph_font.glyph_image_for_index(glyph_index)?;
            // FIXME (upstream): a glyph whose padded size doesn't fit an
            // empty atlas is never packed, and C then creates atlases
            // without end; fail as for larger glyphs.
            if surface.width() > atlas_texture_size
                || surface.height() > atlas_texture_size
                || !stbrp_fits_empty_target(
                    surface.width() + 1,
                    surface.height() + 1,
                    atlas_texture_size,
                )
            {
                return Err(Error::new(format!(
                    "Glyph surface {}x{} larger than atlas texture {}x{}",
                    surface.width(),
                    surface.height(),
                    atlas_texture_size,
                    atlas_texture_size
                )));
            }

            let (w, h) = (surface.width(), surface.height());
            surfaces[i].surface = Some(surface);
            surfaces[i].image_type = image_type;

            // Add one pixel extra padding between glyphs
            missing.push(StbrpRect {
                id: i as i32,
                w: w + 1,
                h: h + 1,
                ..StbrpRect::default()
            });
        }
    }

    // Sort the glyphs by size
    let reserved: Vec<Option<usize>> = ops.iter().map(|op| op.1).collect();
    sdl_qsort(&mut missing, |a, b| sort_missing(&reserved, a, b));

    // Create the texture atlas if necessary
    if enginedata.atlas.is_empty() {
        let atlas = create_atlas(device, atlas_texture_size)?;
        enginedata.atlas.push(atlas);
    }

    resolve_missing_glyphs(enginedata, device, 0, font, &surfaces, ops, &mut missing)?;

    // Resolve any duplicates
    let fontdata = enginedata.fonts.get(font).expect("font data");
    for op in ops.iter_mut() {
        if let (DrawOperation::Copy(copy), None) = (&op.0, op.1) {
            match fontdata.glyphs.find(&copy.glyph_font, copy.glyph_index) {
                Some(glyph) => op.1 = Some(glyph),
                None => {
                    // Something is very wrong...
                    return Err(Error::new("Missing glyph"));
                }
            }
        }
    }

    // (dropping the surfaces is SDL_DestroySurface())
    Ok(())
}

/* DestroyDrawSequence: dropping it */

/// `GetOperationTexture` (the atlas's index)
fn get_operation_texture(
    enginedata: &EngineData,
    op: &(DrawOperation, Option<usize>),
) -> Option<usize> {
    if let DrawOperation::Copy(_) = op.0 {
        let glyph = &enginedata.glyph_arena[op.1.expect("glyph")];
        return Some(glyph.atlas);
    }
    None
}

/// `GetOperationImageType`
fn get_operation_image_type(
    enginedata: &EngineData,
    op: &(DrawOperation, Option<usize>),
) -> ImageType {
    if let DrawOperation::Copy(_) = op.0 {
        let glyph = &enginedata.glyph_arena[op.1.expect("glyph")];
        return glyph.image_type;
    }
    ImageType::Invalid
}

/// Reserve `n` elements of `v`, failing as `SDL_malloc()` does.
fn try_vec<T>(n: usize) -> Result<Vec<T>> {
    let mut v = Vec::new();
    if v.try_reserve_exact(n).is_err() {
        return Err(Error::out_of_memory());
    }
    Ok(v)
}

/// `CreateDrawSequence` (the sequences of all the operations, in order)
fn create_draw_sequence(
    enginedata: &EngineData,
    ops: &[(DrawOperation, Option<usize>)],
    winding: GpuTextEngineWinding,
    sequences: &mut Vec<GpuAtlasDrawSequence>,
) -> Result<()> {
    let num_ops = ops.len();
    debug_assert!(num_ops > 0);

    let texture = get_operation_texture(enginedata, &ops[0]);
    let image_type = get_operation_image_type(enginedata, &ops[0]);
    let mut end = None;
    for (i, op) in ops.iter().enumerate().skip(1) {
        if get_operation_texture(enginedata, op) != texture
            || get_operation_image_type(enginedata, op) != image_type
        {
            end = Some(i);
            break;
        }
    }

    let count = end.unwrap_or(num_ops);
    let atlas_texture = texture.map(|t| enginedata.atlas[t].texture.clone());
    let num_vertices = (count * 4) as i32;
    let num_indices = (count * 6) as i32;

    let mut uv = Vec::new();
    if texture.is_some() {
        uv = try_vec(count * 4)?;

        for op in &ops[..count] {
            let glyph = &enginedata.glyph_arena[op.1.expect("glyph")];
            for p in glyph.texcoords.chunks_exact(2) {
                uv.push(FPoint::new(p[0], p[1]));
            }
        }
    }

    let mut xy = try_vec(count * 4)?;
    for op in &ops[..count] {
        let dst = match &op.0 {
            DrawOperation::Fill(fill) => fill.rect,
            DrawOperation::Copy(copy) => copy.dst,
            // (C reads through a null pointer; the layout makes no no-ops)
            DrawOperation::Noop => Rect::default(),
        };

        let minx = dst.x as f32;
        let maxx = (dst.x + dst.w) as f32;
        let miny = dst.y as f32;
        let maxy = (dst.y + dst.h) as f32;

        // In the GPU API postive y-axis is upwards so the signs of the y-coords is reversed
        xy.push(FPoint::new(minx, -miny));
        xy.push(FPoint::new(maxx, -miny));
        xy.push(FPoint::new(maxx, -maxy));
        xy.push(FPoint::new(minx, -maxy));
    }

    // (C allocates 12 indices per rectangle and fills 6)
    let mut indices = try_vec(count * 6)?;

    const RECT_INDEX_ORDER_CW: [i32; 6] = [0, 1, 2, 0, 2, 3];
    const RECT_INDEX_ORDER_CCW: [i32; 6] = [0, 2, 1, 0, 3, 2];

    let rect_index_order = if winding == GpuTextEngineWinding::Clockwise {
        RECT_INDEX_ORDER_CW
    } else {
        RECT_INDEX_ORDER_CCW
    };

    let mut vertex_index = 0;
    for _ in 0..count {
        for o in rect_index_order {
            indices.push(vertex_index + o);
        }
        vertex_index += 4;
    }

    if sequences.try_reserve(1).is_err() {
        return Err(Error::out_of_memory());
    }
    sequences.push(GpuAtlasDrawSequence {
        atlas_texture,
        xy,
        uv,
        num_vertices,
        indices,
        num_indices,
        image_type,
    });

    if count < num_ops {
        create_draw_sequence(enginedata, &ops[count..], winding, sequences)?;
    }
    Ok(())
}

/// `DestroyTextData`
fn destroy_text_data(enginedata: &mut EngineData, glyphs: Vec<usize>) {
    // DestroyDrawSequence(): dropping it

    for glyph in glyphs {
        release_glyph(enginedata, glyph);
    }
}

/// `CreateTextData`
fn create_text_data(
    enginedata: &mut EngineData,
    device: &Device,
    font: &FontHandle,
    ops: &mut Ops,
) -> Result<GpuTextEngineTextData> {
    let mut glyphs = Vec::new();

    // First, match draw operations to existing glyphs
    let mut num_glyphs = 0;
    let mut num_missing = 0;
    {
        let fontdata = enginedata.fonts.get(font).expect("font data");
        for op in ops.iter_mut() {
            let DrawOperation::Copy(copy) = &op.0 else {
                continue;
            };

            num_glyphs += 1;

            match fontdata.glyphs.find(&copy.glyph_font, copy.glyph_index) {
                Some(glyph) => op.1 = Some(glyph),
                None => num_missing += 1,
            }
        }
    }

    // Create any missing glyphs
    if num_missing > 0 {
        create_missing_glyphs(enginedata, device, font, ops, num_missing)?;
    }

    // Add references to all the glyphs
    if glyphs.try_reserve_exact(num_glyphs).is_err() {
        return Err(Error::out_of_memory());
    }
    for op in ops.iter() {
        if !matches!(op.0, DrawOperation::Copy(_)) {
            continue;
        }

        let glyph = op.1.expect("glyph");
        enginedata.glyph_arena[glyph].refcount += 1;
        glyphs.push(glyph);
    }

    // Sort the operations to batch by texture
    {
        let arena = &enginedata.glyph_arena;
        let mut order: Vec<usize> = (0..ops.len()).collect();
        sdl_qsort(&mut order, |&a, &b| {
            sort_operations(arena, &ops[a], &ops[b])
        });
        let sorted: Ops = order.iter().map(|&i| ops[i].clone()).collect();
        *ops = sorted;
    }

    // Create batched draw sequences
    let mut draw_sequence = Vec::new();
    if let Err(e) = create_draw_sequence(enginedata, ops, enginedata.winding, &mut draw_sequence) {
        destroy_text_data(enginedata, glyphs);
        return Err(e);
    }

    Ok(GpuTextEngineTextData {
        glyphs,
        draw_sequence: draw_sequence.into(),
    })
}

/* DestroyFontData, NukeGlyph: clear_font_glyphs() */

/// `SDL_ClearHashTable()` of a font's glyphs (`NukeGlyph`)
fn clear_font_glyphs(enginedata: &mut EngineData, font: &FontHandle) {
    let Some(fontdata) = enginedata.fonts.get_mut(font) else {
        return;
    };
    let entries = std::mem::take(&mut fontdata.glyphs.entries);
    fontdata.glyphs.index.clear();
    for (_, glyph) in entries {
        release_glyph(enginedata, glyph);
    }
}

/// `CreateFontData`
fn create_font_data(
    enginedata: &mut EngineData,
    font: &FontHandle,
    font_generation: u32,
) -> Result<()> {
    if enginedata.fonts.try_reserve(1).is_err() {
        return Err(Error::out_of_memory());
    }
    enginedata.fonts.insert(
        font.clone(),
        GpuTextEngineFontData {
            generation: font_generation,
            glyphs: GlyphHashTable::default(),
        },
    );
    Ok(())
}

/* DestroyEngineData, NukeFontData: dropping the engine */

/// `CreateEngineData`
fn create_engine_data(atlas_texture_size: i32) -> EngineData {
    EngineData {
        fonts: HashMap::new(),
        atlas: Vec::new(),
        atlas_texture_size,
        winding: GpuTextEngineWinding::Clockwise,
        glyph_arena: Vec::new(),
    }
}

impl TextEngine for GpuTextEngine {
    /// `CreateText`
    fn create_text(&self, text: &TextData) -> Result<Box<dyn Any>> {
        let font = text.font().ok_or_else(|| Error::invalid_param("font"))?;
        let font_generation = font.generation();
        let mut enginedata = self.data.borrow_mut();
        let enginedata = &mut *enginedata;

        match enginedata.fonts.get_mut(&font) {
            None => create_font_data(enginedata, &font, font_generation)?,
            Some(fontdata) => {
                if font_generation != fontdata.generation {
                    fontdata.generation = font_generation;
                    clear_font_glyphs(enginedata, &font);
                }
            }
        }

        // Make a sortable copy of the draw operations
        let mut ops: Ops = Vec::new();
        if ops.try_reserve_exact(text.ops().len()).is_err() {
            return Err(Error::out_of_memory());
        }
        ops.extend(text.ops().iter().map(|op| (op.clone(), None)));

        let data = create_text_data(enginedata, &self.device, &font, &mut ops)?;
        Ok(Box::new(data))
    }

    /// `DestroyText`
    fn destroy_text(&self, engine_text: Box<dyn Any>) {
        if let Ok(data) = engine_text.downcast::<GpuTextEngineTextData>() {
            destroy_text_data(&mut self.data.borrow_mut(), data.glyphs);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl GpuTextEngine {
    /// Create a text engine for drawing text with the SDL GPU API, with
    /// atlas textures of 1024x1024 pixels. Translation of
    /// `TTF_CreateGPUTextEngine()` (dropping the last reference is
    /// `TTF_DestroyGPUTextEngine()`).
    pub fn new(device: &Device) -> Result<Rc<GpuTextEngine>> {
        GpuTextEngine::with_atlas_texture_size(device, 1024)
    }

    /// Create a text engine for drawing text with the SDL GPU API, with
    /// atlas textures of `atlas_texture_size` x `atlas_texture_size`
    /// pixels. Translation of `TTF_CreateGPUTextEngineWithProperties()`
    /// (with `TTF_PROP_GPU_TEXT_ENGINE_ATLAS_TEXTURE_SIZE`).
    pub fn with_atlas_texture_size(
        device: &Device,
        atlas_texture_size: i32,
    ) -> Result<Rc<GpuTextEngine>> {
        if atlas_texture_size <= 0 {
            return Err(Error::new(
                "Failed to create GPU text engine: Invalid texture atlas size.",
            ));
        }

        Ok(Rc::new(GpuTextEngine {
            device: device.clone(),
            data: RefCell::new(create_engine_data(atlas_texture_size)),
        }))
    }

    /// Sets the winding order of the vertices returned by
    /// [`gpu_text_draw_data`]. Translation of
    /// `TTF_SetGPUTextEngineWinding()`.
    pub fn set_winding(&self, winding: GpuTextEngineWinding) -> Result<()> {
        if winding == GpuTextEngineWinding::Invalid {
            return Err(Error::invalid_param("winding"));
        }

        self.data.borrow_mut().winding = winding;
        Ok(())
    }

    /// Get the winding order of the vertices returned by
    /// [`gpu_text_draw_data`]. Translation of
    /// `TTF_GetGPUTextEngineWinding()`.
    pub fn winding(&self) -> GpuTextEngineWinding {
        self.data.borrow().winding
    }

    /// The GPU device of this engine.
    pub fn device(&self) -> &Device {
        &self.device
    }
}

/// Get the geometry data needed for drawing the text (the text must use
/// a [`GpuTextEngine`]): the draw sequences, `None` for an empty text.
/// The positions are relative to the text's upper left corner, with y
/// going up. Translation of `TTF_GetGPUTextDrawData()`.
pub fn gpu_text_draw_data(text: &Text) -> Result<Option<Rc<[GpuAtlasDrawSequence]>>> {
    let is_gpu_engine = text
        .engine()
        .is_some_and(|e| e.as_any().downcast_ref::<GpuTextEngine>().is_some());
    if !is_gpu_engine {
        return Err(Error::invalid_param("text"));
    }

    // Make sure the text is up to date
    text.update()?;

    let td = text.rc.borrow();
    let Some(data) = td
        .engine_text
        .as_ref()
        .and_then(|d| d.downcast_ref::<GpuTextEngineTextData>())
    else {
        // Empty string, nothing to do
        return Ok(None);
    };

    Ok(Some(data.draw_sequence.clone()))
}
