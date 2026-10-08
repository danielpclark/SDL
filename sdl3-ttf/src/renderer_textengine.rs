// Rust translation of src/SDL_renderer_textengine.c from SDL_ttf.
// Copyright (C) 2001-2025 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The text engine for drawing text with SDL renderers.
//!
//! Translation notes:
//!
//! * The engine shares its renderer (`Rc<RefCell<Renderer>>`): it creates
//!   and fills atlas textures while texts are updated, and draws with it.
//! * The glyphs (`AtlasGlyph *`) are indices into an arena of the engine,
//!   with C's reference counts; the atlases (`AtlasTexture *`, a linked
//!   list) are indices into a list in creation order, and their free lists
//!   are lists of glyph indices, sorted as C's are.
//! * The hash tables of glyphs (`SDL_HashTable`, keyed by font pointer and
//!   glyph index) keep their entries in insertion order, so clearing one
//!   releases its glyphs in a fixed order (C's order is that of its
//!   buckets). The order only decides which free atlas areas are reused.
//! * The draw operations' `reserved` pointers are the glyph indices kept
//!   next to the copies of the operations.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use sdl3::render::{Indices, Renderer, Texture, TextureAccess};
use sdl3::video::surface::ScaleMode;
use sdl3::video::{FColor, PixelFormat, Rect, Surface};
use sdl3::{Error, Result};

use crate::qsort::sdl_qsort;
use crate::stb_rect_pack::*;
use crate::text::*;
use crate::ttf::ImageType;

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
#[derive(Debug)]
struct AtlasTexture {
    texture: Texture,
    packer: StbrpContext,
    free_glyphs: Vec<usize>,
}

/// `AtlasDrawSequence` (`next` is the next sequence of the text's list)
#[derive(Debug)]
struct AtlasDrawSequence {
    texture: Option<Texture>,
    image_type: ImageType,
    rects: Vec<Rect>,
    texcoords: Vec<f32>,
    positions: Vec<f32>,
    indices: Vec<i32>,
}

/// `TTF_RendererTextEngineTextData`
#[derive(Debug)]
struct RendererTextEngineTextData {
    glyphs: Vec<usize>,
    draw_sequence: Vec<AtlasDrawSequence>,
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

/// `TTF_RendererTextEngineFontData`
#[derive(Debug)]
struct RendererTextEngineFontData {
    /* font: the key */
    generation: u32,
    glyphs: GlyphHashTable,
}

/// `TTF_RendererTextEngineData` (but the renderer)
#[derive(Debug)]
struct EngineData {
    fonts: HashMap<FontHandle, RendererTextEngineFontData>,
    atlas: Vec<AtlasTexture>,
    atlas_texture_size: i32,
    /// the glyphs (C allocates each)
    glyph_arena: Vec<AtlasGlyph>,
}

/// The text engine for drawing text with an SDL renderer
/// (`TTF_RendererTextEngineData`, as the `userdata` of a
/// `TTF_TextEngine`).
pub struct RendererTextEngine {
    renderer: Rc<RefCell<Renderer>>,
    data: RefCell<EngineData>,
}

impl std::fmt::Debug for RendererTextEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererTextEngine")
            .field("data", &self.data)
            .finish_non_exhaustive()
    }
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

/// `DestroyAtlas`
fn destroy_atlas(renderer: &mut Renderer, atlas: &AtlasTexture) {
    renderer.destroy_texture(atlas.texture);
}

/// `CreateAtlas`
fn create_atlas(renderer: &mut Renderer, atlas_texture_size: i32) -> Result<AtlasTexture> {
    let texture = renderer.create_texture(
        PixelFormat::ARGB8888,
        TextureAccess::Streaming,
        atlas_texture_size,
        atlas_texture_size,
    )?;
    let _ = renderer.set_texture_scale_mode(texture, ScaleMode::Nearest);

    let num_nodes = atlas_texture_size / 4;
    if num_nodes == 0 {
        // FIXME (upstream): stbrp_init_target() divides by the number of
        // nodes (and writes past the empty node array) for atlas sizes
        // below 4; fail instead.
        renderer.destroy_texture(texture);
        return Err(Error::new(
            "Failed to create renderer text engine: Invalid texture atlas size.",
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
        renderer.destroy_texture(texture);
        return Err(Error::out_of_memory());
    }
    stbrp_setup_heuristic(&mut packer, STBRP_HEURISTIC_SKYLINE_DEFAULT);

    Ok(AtlasTexture {
        texture,
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

    let minu = area.x as f32 / atlas_texture_size as f32;
    let minv = area.y as f32 / atlas_texture_size as f32;
    let maxu = (area.x + area.w) as f32 / atlas_texture_size as f32;
    let maxv = (area.y + area.h) as f32 / atlas_texture_size as f32;
    let glyph = AtlasGlyph {
        refcount: 1,
        atlas,
        image_type: ImageType::Invalid,
        rect: Rect::new(area.x, area.y, area.w, area.h),
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

/// `UpdateGlyph`
fn update_glyph(
    enginedata: &mut EngineData,
    renderer: &mut Renderer,
    glyph: usize,
    surface: &Surface<'_>,
    image_type: ImageType,
) -> Result<()> {
    let g = &enginedata.glyph_arena[glyph];
    let texture = enginedata.atlas[g.atlas].texture;
    let rect = g.rect;
    {
        let mut lock = renderer.lock_texture(texture, Some(&rect))?;

        let src = surface.pixels().unwrap_or(&[]);
        let src_pitch = surface.pitch() as usize;
        let dst_pitch = lock.pitch() as usize;
        let dst = lock.pixels();
        let row = rect.w as usize * 4;
        for i in 0..rect.h as usize {
            let s = src.get(i * src_pitch..i * src_pitch + row);
            let d = dst.get_mut(i * dst_pitch..i * dst_pitch + row);
            if let (Some(s), Some(d)) = (s, d) {
                d.copy_from_slice(s);
            }
        }
        // SDL_UnlockTexture(): dropping the lock
    }

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
    renderer: &mut Renderer,
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
            if let Err(e) = update_glyph(enginedata, renderer, glyph, s, surface.image_type) {
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
        if let Err(e) = update_glyph(enginedata, renderer, glyph, s, surface.image_type) {
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
        let next = create_atlas(renderer, atlas_texture_size)?;
        enginedata.atlas.push(next);
    }
    resolve_missing_glyphs(
        enginedata,
        renderer,
        atlas + 1,
        font,
        surfaces,
        ops,
        missing,
    )
}

/// `CreateMissingGlyphs`
fn create_missing_glyphs(
    enginedata: &mut EngineData,
    renderer: &mut Renderer,
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
            if surface.width() > atlas_texture_size || surface.height() > atlas_texture_size {
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

            missing.push(StbrpRect {
                id: i as i32,
                w,
                h,
                ..StbrpRect::default()
            });
        }
    }

    // Sort the glyphs by size
    let reserved: Vec<Option<usize>> = ops.iter().map(|op| op.1).collect();
    sdl_qsort(&mut missing, |a, b| sort_missing(&reserved, a, b));

    // Create the texture atlas if necessary
    if enginedata.atlas.is_empty() {
        let atlas = create_atlas(renderer, atlas_texture_size)?;
        enginedata.atlas.push(atlas);
    }

    resolve_missing_glyphs(enginedata, renderer, 0, font, &surfaces, ops, &mut missing)?;

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

/// `GetOperationTexture`
fn get_operation_texture(
    enginedata: &EngineData,
    op: &(DrawOperation, Option<usize>),
) -> Option<Texture> {
    if let DrawOperation::Copy(_) = op.0 {
        let glyph = &enginedata.glyph_arena[op.1.expect("glyph")];
        return Some(enginedata.atlas[glyph.atlas].texture);
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
    sequences: &mut Vec<AtlasDrawSequence>,
) -> Result<()> {
    let num_ops = ops.len();
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
    let mut rects = try_vec(count)?;

    for op in &ops[..count] {
        let dst = match &op.0 {
            DrawOperation::Fill(fill) => fill.rect,
            DrawOperation::Copy(copy) => copy.dst,
            // (C copies from a null pointer; the layout makes no no-ops)
            DrawOperation::Noop => Rect::default(),
        };
        rects.push(dst);
    }

    let mut texcoords = Vec::new();
    if texture.is_some() {
        texcoords = try_vec(count * 8)?;

        for op in &ops[..count] {
            let glyph = &enginedata.glyph_arena[op.1.expect("glyph")];
            texcoords.extend_from_slice(&glyph.texcoords);
        }
    }

    let mut positions = try_vec(count * 8)?;
    positions.resize(count * 8, 0.0);

    let mut indices = try_vec(count * 12)?;

    const RECT_INDEX_ORDER: [i32; 6] = [0, 1, 2, 0, 2, 3];
    let mut vertex_index = 0;
    for _ in 0..count {
        for o in RECT_INDEX_ORDER {
            indices.push(vertex_index + o);
        }
        vertex_index += 4;
    }

    if sequences.try_reserve(1).is_err() {
        return Err(Error::out_of_memory());
    }
    sequences.push(AtlasDrawSequence {
        texture,
        image_type,
        rects,
        texcoords,
        positions,
        indices,
    });

    if count < num_ops {
        create_draw_sequence(enginedata, &ops[count..], sequences)?;
    }
    Ok(())
}

/// `DestroyTextData`
fn destroy_text_data(enginedata: &mut EngineData, data: RendererTextEngineTextData) {
    // DestroyDrawSequence(): dropping it

    for glyph in data.glyphs {
        release_glyph(enginedata, glyph);
    }
}

/// `CreateTextData`
fn create_text_data(
    enginedata: &mut EngineData,
    renderer: &mut Renderer,
    font: &FontHandle,
    ops: &mut Ops,
) -> Result<RendererTextEngineTextData> {
    let mut data = RendererTextEngineTextData {
        glyphs: Vec::new(),
        draw_sequence: Vec::new(),
    };

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
        create_missing_glyphs(enginedata, renderer, font, ops, num_missing)?;
    }

    // Add references to all the glyphs
    if data.glyphs.try_reserve_exact(num_glyphs).is_err() {
        return Err(Error::out_of_memory());
    }
    for op in ops.iter() {
        if !matches!(op.0, DrawOperation::Copy(_)) {
            continue;
        }

        let glyph = op.1.expect("glyph");
        enginedata.glyph_arena[glyph].refcount += 1;
        data.glyphs.push(glyph);
    }

    // Sort the operations to batch by texture
    {
        let glyphs = &enginedata.glyph_arena;
        let mut order: Vec<usize> = (0..ops.len()).collect();
        sdl_qsort(&mut order, |&a, &b| {
            sort_operations(glyphs, &ops[a], &ops[b])
        });
        let sorted: Ops = order.iter().map(|&i| ops[i].clone()).collect();
        *ops = sorted;
    }

    // Create batched draw sequences
    if let Err(e) = create_draw_sequence(enginedata, ops, &mut data.draw_sequence) {
        destroy_text_data(enginedata, data);
        return Err(e);
    }

    Ok(data)
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
        RendererTextEngineFontData {
            generation: font_generation,
            glyphs: GlyphHashTable::default(),
        },
    );
    Ok(())
}

/* DestroyEngineData, NukeFontData: Drop for RendererTextEngine */

/// `CreateEngineData`
fn create_engine_data(atlas_texture_size: i32) -> EngineData {
    EngineData {
        fonts: HashMap::new(),
        atlas: Vec::new(),
        atlas_texture_size,
        glyph_arena: Vec::new(),
    }
}

impl TextEngine for RendererTextEngine {
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

        let mut renderer = self
            .renderer
            .try_borrow_mut()
            .map_err(|_| Error::invalid_param("renderer"))?;
        let data = create_text_data(enginedata, &mut renderer, &font, &mut ops)?;
        Ok(Box::new(data))
    }

    /// `DestroyText`
    fn destroy_text(&self, engine_text: Box<dyn Any>) {
        if let Ok(data) = engine_text.downcast::<RendererTextEngineTextData>() {
            destroy_text_data(&mut self.data.borrow_mut(), *data);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl RendererTextEngine {
    /// Create a text engine for drawing text with an SDL renderer, with
    /// atlas textures of 1024x1024 pixels. Translation of
    /// `TTF_CreateRendererTextEngine()` (dropping the last reference is
    /// `TTF_DestroyRendererTextEngine()`).
    pub fn new(renderer: Rc<RefCell<Renderer>>) -> Result<Rc<RendererTextEngine>> {
        RendererTextEngine::with_atlas_texture_size(renderer, 1024)
    }

    /// Create a text engine for drawing text with an SDL renderer, with
    /// atlas textures of `atlas_texture_size` x `atlas_texture_size`
    /// pixels. Translation of `TTF_CreateRendererTextEngineWithProperties()`
    /// (with `TTF_PROP_RENDERER_TEXT_ENGINE_ATLAS_TEXTURE_SIZE`).
    pub fn with_atlas_texture_size(
        renderer: Rc<RefCell<Renderer>>,
        atlas_texture_size: i32,
    ) -> Result<Rc<RendererTextEngine>> {
        if atlas_texture_size <= 0 {
            return Err(Error::new(
                "Failed to create renderer text engine: Invalid texture atlas size.",
            ));
        }

        Ok(Rc::new(RendererTextEngine {
            renderer,
            data: RefCell::new(create_engine_data(atlas_texture_size)),
        }))
    }

    /// The renderer of this engine.
    pub fn renderer(&self) -> &Rc<RefCell<Renderer>> {
        &self.renderer
    }
}

impl Drop for RendererTextEngine {
    /// `TTF_DestroyRendererTextEngine()` (`DestroyEngineData`)
    fn drop(&mut self) {
        let data = self.data.get_mut();
        data.fonts.clear();

        if let Ok(mut renderer) = self.renderer.try_borrow_mut() {
            for atlas in &data.atlas {
                destroy_atlas(&mut renderer, atlas);
            }
        }
    }
}

/// Draw text with the renderer of its [`RendererTextEngine`], at
/// (`x`, `y`) in renderer coordinates. Translation of
/// `TTF_DrawRendererText()`.
pub fn draw_renderer_text(text: &Text, x: f32, y: f32) -> Result<()> {
    let engine = text.engine();
    let Some(engine) = engine
        .as_ref()
        .and_then(|e| e.as_any().downcast_ref::<RendererTextEngine>())
    else {
        return Err(Error::invalid_param("text"));
    };

    // Make sure the text is up to date
    text.update()?;

    let mut td = text.rc.borrow_mut();
    let text_color = td.color;
    let Some(data) = td
        .engine_text
        .as_mut()
        .and_then(|d| d.downcast_mut::<RendererTextEngineTextData>())
    else {
        // Empty string, nothing to do
        return Ok(());
    };

    let mut renderer = engine
        .renderer
        .try_borrow_mut()
        .map_err(|_| Error::invalid_param("renderer"))?;
    for sequence in &mut data.draw_sequence {
        let positions = &mut sequence.positions;
        for (i, dst) in sequence.rects.iter().enumerate() {
            let minx = x + dst.x as f32;
            let maxx = x + dst.x as f32 + dst.w as f32;
            let miny = y + dst.y as f32;
            let maxy = y + dst.y as f32 + dst.h as f32;

            positions[i * 8..i * 8 + 8]
                .copy_from_slice(&[minx, miny, maxx, miny, maxx, maxy, minx, maxy]);
        }

        let color = if sequence.image_type == ImageType::Alpha {
            text_color
        } else {
            // Don't alter the color data in the image
            FColor::new(1.0, 1.0, 1.0, text_color.a)
        };

        let _ = renderer.render_geometry_raw(
            sequence.texture,
            &sequence.positions,
            2,
            &[color],
            0,
            &sequence.texcoords,
            2,
            sequence.rects.len() * 4,
            Some(Indices::I32(&sequence.indices)),
        );
    }
    Ok(())
}
