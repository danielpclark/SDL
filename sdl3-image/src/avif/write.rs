// Rust translation of src/write.c from libavif
// (https://github.com/AOMediaCodec/libavif, at the revision SDL_image's
// external/libavif pins: libavif 1.1.1 with SDL's patches), as SDL_image
// builds it (without the experimental gain maps, sample transforms, reduced
// header and AV2).
// Copyright 2019 Joe Drago. All rights reserved.
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The encoder (`avifEncoder*()`): it codes the color and alpha planes of
//! the added images with the AV1 codec (`codec_aom.rs`) and writes the
//! ISOBMFF/HEIF container: `ftyp`, the `meta` box with its items and
//! (deduplicated) item properties, the tracks of an image sequence, and
//! `mdat`.
//!
//! Items refer to each other by index into the item array (upstream's
//! pointers), and a codec and its output are taken out of their item while
//! the codec reads the encoder's settings.

use std::mem;

use super::avif::{
    avif_are_grid_dimensions_valid, avif_codec_create_encoder, avif_codec_name,
    avif_codec_specific_options_clear, avif_codec_specific_options_create,
    avif_codec_specific_options_set, avif_codec_type_from_choice, avif_image_allocate_planes,
    avif_image_copy, avif_image_create_empty, avif_image_is_opaque, avif_image_plane,
    avif_image_plane_height, avif_image_plane_mut, avif_image_plane_row_bytes,
    avif_image_plane_width, avif_image_uses_u16, AvifAddImageFlags, AvifCodecChoice, AvifEncoder,
    AvifFraction, AvifHeaderFormat, AvifImage, AvifIoStats, AvifPixelFormat, AvifResult,
    AvifScalingMode, AVIF_ADD_IMAGE_FLAG_FORCE_KEYFRAME, AVIF_ADD_IMAGE_FLAG_SINGLE, AVIF_CHAN_A,
    AVIF_CHAN_Y, AVIF_CODEC_FLAG_CAN_ENCODE, AVIF_MAX_AV1_LAYER_COUNT, AVIF_PLANES_A,
    AVIF_PLANES_YUV, AVIF_QUALITY_DEFAULT, AVIF_QUANTIZER_BEST_QUALITY,
    AVIF_QUANTIZER_WORST_QUALITY, AVIF_REPETITION_COUNT_INFINITE, AVIF_SPEED_DEFAULT,
    AVIF_TRANSFORM_CLAP, AVIF_TRANSFORM_IMIR, AVIF_TRANSFORM_IROT, AVIF_TRANSFORM_PASP,
};
use super::codec_aom::AvifCodec;
use super::diag::{clear_error, printf, AvifDiagnostics};
use super::exif::avif_get_exif_tiff_header_offset;
use super::internal::{
    avif_assert_or_return, avif_checkerr, avif_checkres, avif_is_alpha, AvifCodecConfigurationBox,
    AvifCodecEncodeOutput, AvifCodecType, AvifEncodeSample, AvifEncoderChanges, AvifItemCategory,
    AvifSequenceHeader, AVIF_ENCODER_CHANGE_CODEC_SPECIFIC, AVIF_ENCODER_CHANGE_MAX_QUANTIZER,
    AVIF_ENCODER_CHANGE_MAX_QUANTIZER_ALPHA, AVIF_ENCODER_CHANGE_MIN_QUANTIZER,
    AVIF_ENCODER_CHANGE_MIN_QUANTIZER_ALPHA, AVIF_ENCODER_CHANGE_QUANTIZER,
    AVIF_ENCODER_CHANGE_QUANTIZER_ALPHA, AVIF_ENCODER_CHANGE_SCALING_MODE,
    AVIF_ENCODER_CHANGE_TILE_COLS_LOG2, AVIF_ENCODER_CHANGE_TILE_ROWS_LOG2,
    AVIF_INDEFINITE_DURATION64,
};
use super::obu::avif_sequence_header_parse;
use super::rawdata::{avif_rw_data_realloc, avif_rw_data_set};
use super::stream::{AvifBoxMarker, AvifRWStream, AVIF_BOX_SIZE_TBD};

const MAX_ASSOCIATIONS: usize = 16;
/// `struct ipmaArray`
#[derive(Clone, Copy, Debug, Default)]
struct IpmaArray {
    associations: [u8; MAX_ASSOCIATIONS],
    essential: [bool; MAX_ASSOCIATIONS],
    count: u8,
}

/// Used to store offsets in meta boxes which need to point at mdat offsets that
/// aren't known yet. When an item's mdat payload is written, all registered fixups
/// will have this now-known offset "fixed up". Translation of
/// `avifOffsetFixup`.
#[derive(Clone, Copy, Debug, Default)]
struct AvifOffsetFixup {
    offset: usize,
}

// (alphaURN and xmpContentType with their NUL terminators, as their sizeof
// is written)
const ALPHA_URN: &[u8] = b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha\0";

const XMP_CONTENT_TYPE: &[u8] = b"application/rdf+xml\0";

// ---------------------------------------------------------------------------
// avifSetTileConfiguration

/// Translation of `floorLog2()`.
fn floor_log2(mut n: u32) -> i32 {
    debug_assert!(n > 0);
    let mut count = 0;
    while n != 0 {
        count += 1;
        n >>= 1;
    }
    count - 1
}

// Splits tilesLog2 into *tileDim1Log2 and *tileDim2Log2, considering the ratio of dim1 to dim2.
//
// Precondition:
//     dim1 >= dim2
// Postcondition:
//     tilesLog2 == *tileDim1Log2 + *tileDim2Log2
//     *tileDim1Log2 >= *tileDim2Log2
/// Translation of `splitTilesLog2()`.
fn split_tiles_log2(
    dim1: u32,
    dim2: u32,
    tiles_log2: i32,
    tile_dim1_log2: &mut i32,
    tile_dim2_log2: &mut i32,
) {
    debug_assert!(dim1 >= dim2);
    let ratio = dim1 / dim2;
    let diff_log2 = floor_log2(ratio);
    let mut subtract = tiles_log2 - diff_log2;
    if subtract < 0 {
        subtract = 0;
    }
    *tile_dim2_log2 = subtract / 2;
    *tile_dim1_log2 = tiles_log2 - *tile_dim2_log2;
    debug_assert!(*tile_dim1_log2 >= *tile_dim2_log2);
}

// Set the tile configuration: the number of tiles and the tile size.
//
// Tiles improve encoding and decoding speeds when multiple threads are available. However, for
// image coding, the total tile boundary length affects the compression efficiency because intra
// prediction can't go across tile boundaries. So the more tiles there are in an image, the worse
// the compression ratio is. For a given number of tiles, making the tile size close to a square
// tends to reduce the total tile boundary length inside the image. Use more tiles along the longer
// dimension of the image to make the tile size closer to a square.
/// Translation of `avifSetTileConfiguration()`.
pub(crate) fn avif_set_tile_configuration(
    threads: i32,
    width: u32,
    height: u32,
    tile_rows_log2: &mut i32,
    tile_cols_log2: &mut i32,
) {
    *tile_rows_log2 = 0;
    *tile_cols_log2 = 0;
    if threads > 1 {
        // Avoid small tiles because they are particularly bad for image coding.
        //
        // Use no more tiles than the number of threads. Aim for one tile per thread. Using more
        // than one thread inside one tile could be less efficient. Using more tiles than the
        // number of threads would result in a compression penalty without much benefit.
        const K_MIN_TILE_AREA: u32 = 512 * 512;
        const K_MAX_TILES: u32 = 32;
        let image_area = width.wrapping_mul(height);
        let mut tiles = (image_area.wrapping_add(K_MIN_TILE_AREA - 1)) / K_MIN_TILE_AREA;
        if tiles > K_MAX_TILES {
            tiles = K_MAX_TILES;
        }
        if tiles > threads as u32 {
            tiles = threads as u32;
        }
        let tiles_log2 = floor_log2(tiles);
        // If the image's width is greater than the height, use more tile columns than tile rows.
        if width >= height {
            split_tiles_log2(width, height, tiles_log2, tile_cols_log2, tile_rows_log2);
        } else {
            split_tiles_log2(height, width, tiles_log2, tile_rows_log2, tile_cols_log2);
        }
    }
}

// ---------------------------------------------------------------------------
// avifCodecEncodeOutput

// (avifCodecEncodeOutputCreate() is `AvifCodecEncodeOutput::default()`)

/// Translation of `avifCodecEncodeOutputAddSample()`.
pub(crate) fn avif_codec_encode_output_add_sample(
    encode_output: &mut AvifCodecEncodeOutput,
    data: &[u8],
    sync: bool,
) -> AvifResult {
    avif_checkerr!(
        encode_output.samples.try_reserve(1).is_ok(),
        AvifResult::OutOfMemory
    );
    let mut sample = AvifEncodeSample::default();
    let result = avif_rw_data_set(&mut sample.data, data);
    if result != AvifResult::Ok {
        return result;
    }
    sample.sync = sync;
    encode_output.samples.push(sample);
    AvifResult::Ok
}

// (avifCodecEncodeOutputDestroy() is dropping it)

// ---------------------------------------------------------------------------
// avifEncoderItem

/// one "item" worth for encoder. Translation of `avifEncoderItem`.
#[derive(Debug, Default)]
pub(crate) struct AvifEncoderItem {
    id: u16,
    /// 4-character 'item_type' field in the 'infe' (item info entry) box
    type_: [u8; 4],
    /// only present on image items
    codec: Option<Box<AvifCodec>>,
    /// AV1 sample data
    encode_output: AvifCodecEncodeOutput,
    /// Exif/XMP data
    metadata_payload: Vec<u8>,
    /// Harvested in avifEncoderFinish(), if encodeOutput has samples
    /// TODO(yguyon): Rename or add av2C
    av1c: AvifCodecConfigurationBox,
    /// Which row-major cell index corresponds to this item. only present on image items
    cell_index: u32,
    /// Category of item being encoded
    item_category: AvifItemCategory,
    /// A hidden image item has (flags & 1) equal to 1 in its ItemInfoEntry.
    hidden_image: bool,

    /// (with its NUL terminator: `infeNameSize` is its length)
    infe_name: &'static [u8],
    /// (with its NUL terminator: `infeContentTypeSize` is its length)
    infe_content_type: &'static [u8],
    mdat_fixups: Vec<AvifOffsetFixup>,

    /// if non-zero, make an iref from this id -> irefToID
    iref_to_id: u16,
    iref_type: &'static [u8],

    /// if non-zero (legal range [1-256]), this is a grid item
    grid_cols: u32,
    /// if non-zero (legal range [1-256]), this is a grid item
    grid_rows: u32,

    // the reconstructed image of a grid item will be trimmed to these dimensions (only present on grid items)
    grid_width: u32,
    grid_height: u32,

    /// if non-zero (legal range [1-(AVIF_MAX_AV1_LAYER_COUNT-1)]), this is a layered AV1 image
    extra_layer_count: u32,

    /// if non-zero, make an iref from dimgFromID -> this id
    dimg_from_id: u16,

    ipma: IpmaArray,
}

// ---------------------------------------------------------------------------
// avifEncoderItemReference

// pointer to one "item" interested in
// (avifEncoderItemReference: an index into the encoder data's items)

// ---------------------------------------------------------------------------
// avifEncoderFrame

/// Translation of `avifEncoderFrame`.
#[derive(Clone, Copy, Debug, Default)]
struct AvifEncoderFrame {
    duration_in_timescales: u64,
}

// ---------------------------------------------------------------------------
// avifEncoderData

/// The settings avifEncoderBackupSettings() copies to the encoder data's
/// `lastEncoder` (an `avifEncoder` upstream; `initialized` is its `data`
/// member, which only marks that lastEncoder is initialized).
#[derive(Clone, Copy, Debug)]
struct AvifEncoderLastSettings {
    initialized: bool,
    codec_choice: AvifCodecChoice,
    max_threads: i32,
    speed: i32,
    keyframe_interval: i32,
    timescale: u64,
    repetition_count: i32,
    extra_layer_count: u32,
    min_quantizer: i32,
    max_quantizer: i32,
    min_quantizer_alpha: i32,
    max_quantizer_alpha: i32,
    scaling_mode: AvifScalingMode,
}

impl Default for AvifEncoderLastSettings {
    fn default() -> Self {
        // (memset() to 0)
        AvifEncoderLastSettings {
            initialized: false,
            codec_choice: AvifCodecChoice::Auto,
            max_threads: 0,
            speed: 0,
            keyframe_interval: 0,
            timescale: 0,
            repetition_count: 0,
            extra_layer_count: 0,
            min_quantizer: 0,
            max_quantizer: 0,
            min_quantizer_alpha: 0,
            max_quantizer_alpha: 0,
            scaling_mode: AvifScalingMode {
                horizontal: AvifFraction::default(),
                vertical: AvifFraction::default(),
            },
        }
    }
}

/// Translation of `avifEncoderData`.
#[derive(Debug)]
pub(crate) struct AvifEncoderData {
    items: Vec<AvifEncoderItem>,
    frames: Vec<AvifEncoderFrame>,
    // Map the encoder settings quality and qualityAlpha to quantizer and quantizerAlpha
    quantizer: i32,
    quantizer_alpha: i32,
    // tileRowsLog2 and tileColsLog2 are the actual tiling values after automatic tiling is handled
    tile_rows_log2: i32,
    tile_cols_log2: i32,
    last_encoder: AvifEncoderLastSettings,
    // lastQuantizer and lastQuantizerAlpha are the quantizer and quantizerAlpha values used last
    // time
    last_quantizer: i32,
    last_quantizer_alpha: i32,
    // lastTileRowsLog2 and lastTileColsLog2 are the actual tiling values used last time
    last_tile_rows_log2: i32,
    last_tile_cols_log2: i32,
    image_metadata: Box<AvifImage>,
    last_item_id: u16,
    primary_item_id: u16,
    /// list of item ids for an 'altr' box (group of alternatives to each other)
    alternative_item_ids: Vec<u16>,
    /// if true, the AVIF_ADD_IMAGE_FLAG_SINGLE flag was set on the first call to avifEncoderAddImage()
    single_image: bool,
    alpha_present: bool,
    gain_map_size_bytes: usize,
    // Fields specific to AV1/AV2
    /// "av01" for AV1 ("av02" for AV2 if AVIF_CODEC_AVM)
    image_item_type: &'static [u8],
    /// "av1C" for AV1 ("av2C" for AV2 if AVIF_CODEC_AVM)
    config_prop_name: &'static [u8],
}

// Returns NULL if a memory allocation failed.
/// Translation of `avifEncoderDataCreate()`.
fn avif_encoder_data_create() -> Option<Box<AvifEncoderData>> {
    let image_metadata = avif_image_create_empty()?;
    let mut items = Vec::new();
    items.try_reserve(8).ok()?;
    Some(Box::new(AvifEncoderData {
        items,
        frames: Vec::new(),
        quantizer: 0,
        quantizer_alpha: 0,
        tile_rows_log2: 0,
        tile_cols_log2: 0,
        last_encoder: AvifEncoderLastSettings::default(),
        last_quantizer: 0,
        last_quantizer_alpha: 0,
        last_tile_rows_log2: 0,
        last_tile_cols_log2: 0,
        image_metadata,
        last_item_id: 0,
        primary_item_id: 0,
        alternative_item_ids: Vec::new(),
        single_image: false,
        alpha_present: false,
        gain_map_size_bytes: 0,
        image_item_type: b"",
        config_prop_name: b"",
    }))
}

/// Translation of `avifEncoderDataCreateItem()`: returns the new item's
/// index (`type_` is 4 bytes; `infe_name` holds its NUL terminator).
fn avif_encoder_data_create_item(
    data: &mut AvifEncoderData,
    type_: &[u8],
    infe_name: &'static [u8],
    cell_index: u32,
) -> Option<usize> {
    data.items.try_reserve(1).ok()?;
    data.last_item_id = data.last_item_id.wrapping_add(1);
    let mut item = AvifEncoderItem {
        id: data.last_item_id,
        infe_name,
        cell_index,
        ..Default::default()
    };
    item.type_.copy_from_slice(&type_[..4]);
    if item.mdat_fixups.try_reserve(4).is_err() {
        data.last_item_id = data.last_item_id.wrapping_sub(1);
        return None;
    }
    data.items.push(item);
    Some(data.items.len() - 1)
}

/// Translation of `avifEncoderDataFindItemByID()`: returns the item's
/// index.
fn avif_encoder_data_find_item_by_id(data: &AvifEncoderData, id: u16) -> Option<usize> {
    for (item_index, item) in data.items.iter().enumerate() {
        if item.id == id {
            return Some(item_index);
        }
    }
    None
}

// (avifEncoderDataDestroy() is dropping it)

/// Translation of `avifEncoderItemAddMdatFixup()`.
fn avif_encoder_item_add_mdat_fixup(item: &mut AvifEncoderItem, s: &AvifRWStream) -> AvifResult {
    avif_checkerr!(
        item.mdat_fixups.try_reserve(1).is_ok(),
        AvifResult::OutOfMemory
    );
    item.mdat_fixups
        .push(AvifOffsetFixup { offset: s.offset() });
    AvifResult::Ok
}

// ---------------------------------------------------------------------------
// avifItemPropertyDedup - Provides ipco deduplication

/// Translation of `avifItemProperty`.
#[derive(Clone, Copy, Debug, Default)]
struct AvifItemProperty {
    index: u8,
    offset: usize,
    size: usize,
}

/// Translation of `avifItemPropertyDedup` (the temporary stream owns its
/// storage, upstream's `buffer`).
#[derive(Debug, Default)]
struct AvifItemPropertyDedup {
    properties: Vec<AvifItemProperty>,
    /// Temporary stream for each new property, checked against already-written boxes for deduplications
    s: AvifRWStream,
    /// 1-indexed, incremented every time another unique property is finished
    next_index: u8,
}

/// Translation of `avifItemPropertyDedupCreate()`.
fn avif_item_property_dedup_create() -> Option<Box<AvifItemPropertyDedup>> {
    let mut dedup = Box::<AvifItemPropertyDedup>::default();
    dedup.properties.try_reserve(8).ok()?;
    if avif_rw_data_realloc(&mut dedup.s.raw, 2048) != AvifResult::Ok {
        return None;
    }
    Some(dedup)
}

// (avifItemPropertyDedupDestroy() is dropping it)

/// Resets the dedup's temporary write stream in preparation for a single item property's worth of writing
/// Translation of `avifItemPropertyDedupStart()`.
fn avif_item_property_dedup_start(dedup: &mut AvifItemPropertyDedup) {
    let buffer = mem::take(&mut dedup.s.raw);
    dedup.s = AvifRWStream::start(buffer);
}

// This compares the newly written item property (in the dedup's temporary storage buffer) to
// already-written properties (whose offsets/sizes in outputStream are recorded in the dedup). If a
// match is found, the previous property's index is used. If this new property is unique, it is
// assigned the next available property index, written to the output stream, and its offset/size in
// the output stream is recorded in the dedup for future comparisons.
//
// On success, this function adds to the given ipma box a property association linking the reused
// or newly created property with the item.
/// Translation of `avifItemPropertyDedupFinish()`.
fn avif_item_property_dedup_finish(
    dedup: &mut AvifItemPropertyDedup,
    output_stream: &mut AvifRWStream,
    ipma: &mut IpmaArray,
    essential: bool,
) -> AvifResult {
    let mut property_index: u8 = 0;
    let new_property_size = dedup.s.offset();

    for property in &dedup.properties {
        if (property.size == new_property_size)
            && output_stream.raw[property.offset..property.offset + new_property_size]
                == dedup.s.raw[..new_property_size]
        {
            // We've already written this exact property, reuse it
            property_index = property.index;
            avif_assert_or_return!(property_index != 0);
            break;
        }
    }

    if property_index == 0 {
        // Write a new property, and remember its location in the output stream for future deduplication
        avif_checkerr!(
            dedup.properties.try_reserve(1).is_ok(),
            AvifResult::OutOfMemory
        );
        dedup.next_index = dedup.next_index.wrapping_add(1); // preincrement so the first new index is 1 (as ipma is 1-indexed)
        let property = AvifItemProperty {
            index: dedup.next_index,
            size: new_property_size,
            offset: output_stream.offset(),
        };
        dedup.properties.push(property);
        avif_checkres!(output_stream.write(&dedup.s.raw[..new_property_size]));
        property_index = property.index;
    }

    avif_checkerr!(
        (ipma.count as usize) < MAX_ASSOCIATIONS,
        AvifResult::UnknownError
    );
    ipma.associations[ipma.count as usize] = property_index;
    ipma.essential[ipma.count as usize] = essential;
    ipma.count += 1;
    AvifResult::Ok
}

/// `if (dedup) { avifItemPropertyDedupStart(dedup); }`
fn dedup_start(dedup: Option<&mut AvifItemPropertyDedup>) {
    if let Some(dedup) = dedup {
        avif_item_property_dedup_start(dedup);
    }
}

/// `if (dedup) { AVIF_CHECKRES(avifItemPropertyDedupFinish(dedup,
/// outputStream, ipma, essential)); }` (the callers check that ipma is set
/// with dedup, so a missing one is an internal error).
fn dedup_finish(
    dedup: Option<&mut AvifItemPropertyDedup>,
    output_stream: &mut AvifRWStream,
    ipma: Option<&mut IpmaArray>,
    essential: bool,
) -> AvifResult {
    if let Some(dedup) = dedup {
        let Some(ipma) = ipma else {
            return AvifResult::InternalError;
        };
        avif_checkres!(avif_item_property_dedup_finish(
            dedup,
            output_stream,
            ipma,
            essential
        ));
    }
    AvifResult::Ok
}

/// The `dedupStream` of the color property writers: the dedup's temporary
/// stream when there is a dedup, the output stream otherwise.
macro_rules! dedup_stream {
    ($dedup:expr, $output_stream:expr) => {
        match $dedup.as_deref_mut() {
            Some(d) => &mut d.s,
            None => &mut *$output_stream,
        }
    };
}

// ---------------------------------------------------------------------------

const NO_SCALING: AvifScalingMode = AvifScalingMode {
    horizontal: AvifFraction { n: 1, d: 1 },
    vertical: AvifFraction { n: 1, d: 1 },
};

/// Translation of `avifEncoderCreate()`.
pub(crate) fn avif_encoder_create() -> Option<Box<AvifEncoder>> {
    let data = avif_encoder_data_create()?;
    Some(Box::new(AvifEncoder {
        codec_choice: AvifCodecChoice::Auto,
        max_threads: 1,
        speed: AVIF_SPEED_DEFAULT,
        keyframe_interval: 0,
        timescale: 1,
        repetition_count: AVIF_REPETITION_COUNT_INFINITE,
        extra_layer_count: 0,
        quality: AVIF_QUALITY_DEFAULT,
        quality_alpha: AVIF_QUALITY_DEFAULT,
        min_quantizer: AVIF_QUANTIZER_BEST_QUALITY,
        max_quantizer: AVIF_QUANTIZER_WORST_QUALITY,
        min_quantizer_alpha: AVIF_QUANTIZER_BEST_QUALITY,
        max_quantizer_alpha: AVIF_QUANTIZER_WORST_QUALITY,
        tile_rows_log2: 0,
        tile_cols_log2: 0,
        auto_tiling: false,
        scaling_mode: NO_SCALING,
        io_stats: AvifIoStats::default(),
        diag: AvifDiagnostics::default(),
        data,
        cs_options: avif_codec_specific_options_create(),
        header_format: AvifHeaderFormat::Full,
    }))
}

// (avifEncoderDestroy() is dropping it)

/// Translation of `avifEncoderSetCodecSpecificOption()`.
pub(crate) fn avif_encoder_set_codec_specific_option(
    encoder: &mut AvifEncoder,
    key: &str,
    value: Option<&str>,
) -> AvifResult {
    avif_codec_specific_options_set(&mut encoder.cs_options, key, value)
}

/// Translation of `avifEncoderBackupSettings()`.
fn avif_encoder_backup_settings(encoder: &mut AvifEncoder) {
    let last_encoder = &mut encoder.data.last_encoder;

    // lastEncoder->data is only used to mark that lastEncoder is initialized. lastEncoder->data
    // must not be dereferenced.
    last_encoder.initialized = true;
    last_encoder.codec_choice = encoder.codec_choice;
    last_encoder.max_threads = encoder.max_threads;
    last_encoder.speed = encoder.speed;
    last_encoder.keyframe_interval = encoder.keyframe_interval;
    last_encoder.timescale = encoder.timescale;
    last_encoder.repetition_count = encoder.repetition_count;
    last_encoder.extra_layer_count = encoder.extra_layer_count;
    last_encoder.min_quantizer = encoder.min_quantizer;
    last_encoder.max_quantizer = encoder.max_quantizer;
    last_encoder.min_quantizer_alpha = encoder.min_quantizer_alpha;
    last_encoder.max_quantizer_alpha = encoder.max_quantizer_alpha;
    encoder.data.last_quantizer = encoder.data.quantizer;
    encoder.data.last_quantizer_alpha = encoder.data.quantizer_alpha;
    encoder.data.last_tile_rows_log2 = encoder.data.tile_rows_log2;
    encoder.data.last_tile_cols_log2 = encoder.data.tile_cols_log2;
    encoder.data.last_encoder.scaling_mode = encoder.scaling_mode;
}

// This function detects changes made on avifEncoder. It returns true on success (i.e., if every
// change is valid), or false on failure (i.e., if any setting that can't change was changed). It
// reports a bitwise-OR of detected changes in encoderChanges.
/// Translation of `avifEncoderDetectChanges()`.
fn avif_encoder_detect_changes(
    encoder: &AvifEncoder,
    encoder_changes: &mut AvifEncoderChanges,
) -> bool {
    let last_encoder = &encoder.data.last_encoder;
    *encoder_changes = 0;

    if !last_encoder.initialized {
        // lastEncoder is not initialized.
        return true;
    }

    if (last_encoder.codec_choice != encoder.codec_choice)
        || (last_encoder.max_threads != encoder.max_threads)
        || (last_encoder.speed != encoder.speed)
        || (last_encoder.keyframe_interval != encoder.keyframe_interval)
        || (last_encoder.timescale != encoder.timescale)
        || (last_encoder.repetition_count != encoder.repetition_count)
        || (last_encoder.extra_layer_count != encoder.extra_layer_count)
    {
        return false;
    }

    if encoder.data.last_quantizer != encoder.data.quantizer {
        *encoder_changes |= AVIF_ENCODER_CHANGE_QUANTIZER;
    }
    if encoder.data.last_quantizer_alpha != encoder.data.quantizer_alpha {
        *encoder_changes |= AVIF_ENCODER_CHANGE_QUANTIZER_ALPHA;
    }
    if last_encoder.min_quantizer != encoder.min_quantizer {
        *encoder_changes |= AVIF_ENCODER_CHANGE_MIN_QUANTIZER;
    }
    if last_encoder.max_quantizer != encoder.max_quantizer {
        *encoder_changes |= AVIF_ENCODER_CHANGE_MAX_QUANTIZER;
    }
    if last_encoder.min_quantizer_alpha != encoder.min_quantizer_alpha {
        *encoder_changes |= AVIF_ENCODER_CHANGE_MIN_QUANTIZER_ALPHA;
    }
    if last_encoder.max_quantizer_alpha != encoder.max_quantizer_alpha {
        *encoder_changes |= AVIF_ENCODER_CHANGE_MAX_QUANTIZER_ALPHA;
    }
    if encoder.data.last_tile_rows_log2 != encoder.data.tile_rows_log2 {
        *encoder_changes |= AVIF_ENCODER_CHANGE_TILE_ROWS_LOG2;
    }
    if encoder.data.last_tile_cols_log2 != encoder.data.tile_cols_log2 {
        *encoder_changes |= AVIF_ENCODER_CHANGE_TILE_COLS_LOG2;
    }
    if last_encoder.scaling_mode != encoder.scaling_mode {
        *encoder_changes |= AVIF_ENCODER_CHANGE_SCALING_MODE;
    }
    if !encoder.cs_options.is_empty() {
        *encoder_changes |= AVIF_ENCODER_CHANGE_CODEC_SPECIFIC;
    }

    true
}

/// Same as 'avifEncoderWriteColorProperties' but for the colr nclx box only.
/// Translation of `avifEncoderWriteNclxProperty()` (`dedupStream` is the
/// dedup's stream when there is a dedup, the output stream otherwise).
fn avif_encoder_write_nclx_property(
    output_stream: &mut AvifRWStream,
    image_metadata: &AvifImage,
    ipma: Option<&mut IpmaArray>,
    mut dedup: Option<&mut AvifItemPropertyDedup>,
) -> AvifResult {
    dedup_start(dedup.as_deref_mut());
    let dedup_stream = dedup_stream!(dedup, output_stream);
    let mut colr: AvifBoxMarker = 0;
    avif_checkres!(dedup_stream.write_box(b"colr", AVIF_BOX_SIZE_TBD, Some(&mut colr)));
    avif_checkres!(dedup_stream.write_chars(b"nclx")); // unsigned int(32) colour_type;
    avif_checkres!(dedup_stream.write_u16(image_metadata.color_primaries)); // unsigned int(16) colour_primaries;
    avif_checkres!(dedup_stream.write_u16(image_metadata.transfer_characteristics)); // unsigned int(16) transfer_characteristics;
    avif_checkres!(dedup_stream.write_u16(image_metadata.matrix_coefficients)); // unsigned int(16) matrix_coefficients;
    avif_checkres!(dedup_stream.write_bits(
        if image_metadata.yuv_range == super::avif::AvifRange::Full {
            1
        } else {
            0
        },
        /*bitCount=*/ 1
    )); // unsigned int(1) full_range_flag;
    avif_checkres!(dedup_stream.write_bits(0, /*bitCount=*/ 7)); // unsigned int(7) reserved = 0;
    dedup_stream.finish_box(colr);
    avif_checkres!(dedup_finish(dedup, output_stream, ipma, false));
    AvifResult::Ok
}

// This function is used in two codepaths:
// * writing color *item* properties
// * writing color *track* properties
//
// Item properties must have property associations with them and can be deduplicated (by reusing
// these associations), so this function leverages the ipma and dedup arguments to do this.
//
// Track properties, however, are implicitly associated by the track in which they are contained, so
// there is no need to build a property association box (ipma), and no way to deduplicate/reuse a
// property. In this case, the ipma and dedup properties should/will be set to NULL, and this
// function will avoid using them.
/// Translation of `avifEncoderWriteColorProperties()`.
fn avif_encoder_write_color_properties(
    output_stream: &mut AvifRWStream,
    image_metadata: &AvifImage,
    mut ipma: Option<&mut IpmaArray>,
    mut dedup: Option<&mut AvifItemPropertyDedup>,
) -> AvifResult {
    // outputStream is the final bitstream that will be output by the libavif encoder API.
    // dedupStream is either equal to outputStream or to &dedup->s which is a temporary stream used
    // to store parts of the final bitstream; these parts may be discarded if they are a duplicate
    // of an already stored property.
    if dedup.is_some() {
        avif_assert_or_return!(ipma.is_some());

        // Use the dedup's temporary stream for box writes.
    }

    if !image_metadata.icc.is_empty() {
        dedup_start(dedup.as_deref_mut());
        let dedup_stream = dedup_stream!(dedup, output_stream);
        let mut colr: AvifBoxMarker = 0;
        avif_checkres!(dedup_stream.write_box(b"colr", AVIF_BOX_SIZE_TBD, Some(&mut colr)));
        avif_checkres!(dedup_stream.write_chars(b"prof")); // unsigned int(32) colour_type;
        avif_checkres!(dedup_stream.write(&image_metadata.icc));
        dedup_stream.finish_box(colr);
        avif_checkres!(dedup_finish(
            dedup.as_deref_mut(),
            output_stream,
            ipma.as_deref_mut(),
            false
        ));
    }

    // HEIF 6.5.5.1, from Amendment 3 allows multiple colr boxes: "at most one for a given value of colour type"
    // Therefore, *always* writing an nclx box, even if an a prof box was already written above.
    avif_checkres!(avif_encoder_write_nclx_property(
        output_stream,
        image_metadata,
        ipma.as_deref_mut(),
        dedup.as_deref_mut()
    ));

    avif_encoder_write_extended_color_properties(output_stream, image_metadata, ipma, dedup)
}

/// Same as 'avifEncoderWriteColorProperties' but for properties related to High Dynamic Range only.
/// Translation of `avifEncoderWriteHDRProperties()`.
///
/// FIXME (upstream): the track path calls it with a NULL `dedupStream`
/// (and no dedup), which it dereferences when the image has content light
/// level information; here the `clli` box then goes to the output stream.
fn avif_encoder_write_hdr_properties(
    output_stream: &mut AvifRWStream,
    image_metadata: &AvifImage,
    ipma: Option<&mut IpmaArray>,
    mut dedup: Option<&mut AvifItemPropertyDedup>,
) -> AvifResult {
    // Write Content Light Level Information, if present
    if image_metadata.clli.max_cll != 0 || image_metadata.clli.max_pall != 0 {
        dedup_start(dedup.as_deref_mut());
        let dedup_stream = dedup_stream!(dedup, output_stream);
        let mut clli: AvifBoxMarker = 0;
        avif_checkres!(dedup_stream.write_box(b"clli", AVIF_BOX_SIZE_TBD, Some(&mut clli)));
        avif_checkres!(dedup_stream.write_u16(image_metadata.clli.max_cll)); // unsigned int(16) max_content_light_level;
        avif_checkres!(dedup_stream.write_u16(image_metadata.clli.max_pall)); // unsigned int(16) max_pic_average_light_level;
        dedup_stream.finish_box(clli);
        avif_checkres!(dedup_finish(dedup, output_stream, ipma, false));
    }

    // TODO(maryla): add other HDR boxes: mdcv, cclv, etc.

    AvifResult::Ok
}

/// Subset of avifEncoderWriteColorProperties() for the properties pasp, clap, irot, imir.
/// Translation of `avifEncoderWriteExtendedColorProperties()`.
fn avif_encoder_write_extended_color_properties(
    output_stream: &mut AvifRWStream,
    image_metadata: &AvifImage,
    mut ipma: Option<&mut IpmaArray>,
    mut dedup: Option<&mut AvifItemPropertyDedup>,
) -> AvifResult {
    // Write (Optional) Transformations
    if image_metadata.transform_flags & AVIF_TRANSFORM_PASP != 0 {
        dedup_start(dedup.as_deref_mut());
        let dedup_stream = dedup_stream!(dedup, output_stream);
        let mut pasp: AvifBoxMarker = 0;
        avif_checkres!(dedup_stream.write_box(b"pasp", AVIF_BOX_SIZE_TBD, Some(&mut pasp)));
        avif_checkres!(dedup_stream.write_u32(image_metadata.pasp.h_spacing)); // unsigned int(32) hSpacing;
        avif_checkres!(dedup_stream.write_u32(image_metadata.pasp.v_spacing)); // unsigned int(32) vSpacing;
        dedup_stream.finish_box(pasp);
        avif_checkres!(dedup_finish(
            dedup.as_deref_mut(),
            output_stream,
            ipma.as_deref_mut(),
            false
        ));
    }
    if image_metadata.transform_flags & AVIF_TRANSFORM_CLAP != 0 {
        dedup_start(dedup.as_deref_mut());
        let dedup_stream = dedup_stream!(dedup, output_stream);
        let mut clap: AvifBoxMarker = 0;
        avif_checkres!(dedup_stream.write_box(b"clap", AVIF_BOX_SIZE_TBD, Some(&mut clap)));
        avif_checkres!(dedup_stream.write_u32(image_metadata.clap.width_n)); // unsigned int(32) cleanApertureWidthN;
        avif_checkres!(dedup_stream.write_u32(image_metadata.clap.width_d)); // unsigned int(32) cleanApertureWidthD;
        avif_checkres!(dedup_stream.write_u32(image_metadata.clap.height_n)); // unsigned int(32) cleanApertureHeightN;
        avif_checkres!(dedup_stream.write_u32(image_metadata.clap.height_d)); // unsigned int(32) cleanApertureHeightD;
        avif_checkres!(dedup_stream.write_u32(image_metadata.clap.horiz_off_n)); // unsigned int(32) horizOffN;
        avif_checkres!(dedup_stream.write_u32(image_metadata.clap.horiz_off_d)); // unsigned int(32) horizOffD;
        avif_checkres!(dedup_stream.write_u32(image_metadata.clap.vert_off_n)); // unsigned int(32) vertOffN;
        avif_checkres!(dedup_stream.write_u32(image_metadata.clap.vert_off_d)); // unsigned int(32) vertOffD;
        dedup_stream.finish_box(clap);
        avif_checkres!(dedup_finish(
            dedup.as_deref_mut(),
            output_stream,
            ipma.as_deref_mut(),
            true
        ));
    }
    if image_metadata.transform_flags & AVIF_TRANSFORM_IROT != 0 {
        dedup_start(dedup.as_deref_mut());
        let dedup_stream = dedup_stream!(dedup, output_stream);
        let mut irot: AvifBoxMarker = 0;
        avif_checkres!(dedup_stream.write_box(b"irot", AVIF_BOX_SIZE_TBD, Some(&mut irot)));
        avif_checkres!(dedup_stream.write_bits(0, /*bitCount=*/ 6)); // unsigned int (6) reserved = 0;
        avif_checkres!(dedup_stream.write_bits(
            (image_metadata.irot.angle & 0x3) as u32,
            /*bitCount=*/ 2
        )); // unsigned int (2) angle;
        dedup_stream.finish_box(irot);
        avif_checkres!(dedup_finish(
            dedup.as_deref_mut(),
            output_stream,
            ipma.as_deref_mut(),
            true
        ));
    }
    if image_metadata.transform_flags & AVIF_TRANSFORM_IMIR != 0 {
        dedup_start(dedup.as_deref_mut());
        let dedup_stream = dedup_stream!(dedup, output_stream);
        let mut imir: AvifBoxMarker = 0;
        avif_checkres!(dedup_stream.write_box(b"imir", AVIF_BOX_SIZE_TBD, Some(&mut imir)));
        avif_checkres!(dedup_stream.write_bits(0, /*bitCount=*/ 7)); // unsigned int(7) reserved = 0;
        avif_checkres!(dedup_stream.write_bits(
            if image_metadata.imir.axis != 0 { 1 } else { 0 },
            /*bitCount=*/ 1
        )); // unsigned int(1) axis;
        dedup_stream.finish_box(imir);
        avif_checkres!(dedup_finish(dedup, output_stream, ipma, true));
    }
    AvifResult::Ok
}

/// Write unassociated metadata items (EXIF, XMP) to a small meta box inside of a trak box.
/// These items are implicitly associated with the track they are contained within.
/// Translation of `avifEncoderWriteTrackMetaBox()`.
fn avif_encoder_write_track_meta_box(
    data: &mut AvifEncoderData,
    s: &mut AvifRWStream,
) -> AvifResult {
    // Count how many non-image items (such as EXIF/XMP) are being written
    let mut metadata_item_count: u32 = 0;
    for item in &data.items {
        if item.type_[..] != data.image_item_type[..4] {
            metadata_item_count += 1;
        }
    }
    if metadata_item_count == 0 {
        // Don't even bother writing the trak meta box
        return AvifResult::Ok;
    }

    let mut meta: AvifBoxMarker = 0;
    avif_checkres!(s.write_full_box(b"meta", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut meta)));

    let mut hdlr: AvifBoxMarker = 0;
    avif_checkres!(s.write_full_box(b"hdlr", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut hdlr)));
    avif_checkres!(s.write_u32(0)); // unsigned int(32) pre_defined = 0;
    avif_checkres!(s.write_chars(b"pict")); // unsigned int(32) handler_type;
    avif_checkres!(s.write_zeros(12)); // const unsigned int(32)[3] reserved = 0;
    avif_checkres!(s.write_chars(b"libavif\0")); // string name; (writing null terminator)
    s.finish_box(hdlr);

    let mut iloc: AvifBoxMarker = 0;
    avif_checkres!(s.write_full_box(b"iloc", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut iloc)));
    avif_checkres!(s.write_bits(4, /*bitCount=*/ 4)); // unsigned int(4) offset_size;
    avif_checkres!(s.write_bits(4, /*bitCount=*/ 4)); // unsigned int(4) length_size;
    avif_checkres!(s.write_bits(0, /*bitCount=*/ 4)); // unsigned int(4) base_offset_size;
    avif_checkres!(s.write_bits(0, /*bitCount=*/ 4)); // unsigned int(4) reserved;
    avif_checkres!(s.write_u16(metadata_item_count as u16)); // unsigned int(16) item_count;
    let image_item_type = data.image_item_type;
    for item in data.items.iter_mut() {
        if item.type_[..] == image_item_type[..4] {
            // Skip over all non-metadata items
            continue;
        }

        avif_checkres!(s.write_u16(item.id)); // unsigned int(16) item_ID;
        avif_checkres!(s.write_u16(0)); // unsigned int(16) data_reference_index;
        avif_checkres!(s.write_u16(1)); // unsigned int(16) extent_count;
        avif_checkres!(avif_encoder_item_add_mdat_fixup(item, s)); //
        avif_checkres!(s.write_u32(0 /* set later */)); // unsigned int(offset_size*8) extent_offset;
        avif_checkres!(s.write_u32(item.metadata_payload.len() as u32)); // unsigned int(length_size*8) extent_length;
    }
    s.finish_box(iloc);

    let mut iinf: AvifBoxMarker = 0;
    avif_checkres!(s.write_full_box(b"iinf", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut iinf)));
    avif_checkres!(s.write_u16(metadata_item_count as u16)); //  unsigned int(16) entry_count;
    for item in &data.items {
        if item.type_[..] == image_item_type[..4] {
            continue;
        }

        avif_assert_or_return!(!item.hidden_image);
        let mut infe: AvifBoxMarker = 0;
        avif_checkres!(s.write_full_box(b"infe", AVIF_BOX_SIZE_TBD, 2, 0, Some(&mut infe)));
        avif_checkres!(s.write_u16(item.id)); // unsigned int(16) item_ID;
        avif_checkres!(s.write_u16(0)); // unsigned int(16) item_protection_index;
        avif_checkres!(s.write(&item.type_)); // unsigned int(32) item_type;
        avif_checkres!(s.write_chars(item.infe_name)); // string item_name; (writing null terminator)
        if !item.infe_content_type.is_empty() {
            // string content_type; (writing null terminator)
            avif_checkres!(s.write_chars(item.infe_content_type));
        }
        s.finish_box(infe);
    }
    s.finish_box(iinf);

    s.finish_box(meta);
    AvifResult::Ok
}

/// Translation of `avifWriteGridPayload()`.
fn avif_write_grid_payload(
    data: &mut Vec<u8>,
    grid_cols: u32,
    grid_rows: u32,
    grid_width: u32,
    grid_height: u32,
) -> AvifResult {
    // ISO/IEC 23008-12 6.6.2.3.2
    // aligned(8) class ImageGrid {
    //     unsigned int(8) version = 0;
    //     unsigned int(8) flags;
    //     FieldLength = ((flags & 1) + 1) * 16;
    //     unsigned int(8) rows_minus_one;
    //     unsigned int(8) columns_minus_one;
    //     unsigned int(FieldLength) output_width;
    //     unsigned int(FieldLength) output_height;
    // }

    let grid_flags: u8 = if (grid_width > 65535) || (grid_height > 65535) {
        1
    } else {
        0
    };

    let mut s = AvifRWStream::start(mem::take(data));
    let result = (|| {
        avif_checkres!(s.write_u8(0)); // unsigned int(8) version = 0;
        avif_checkres!(s.write_u8(grid_flags)); // unsigned int(8) flags;
        avif_checkres!(s.write_u8((grid_rows - 1) as u8)); // unsigned int(8) rows_minus_one;
        avif_checkres!(s.write_u8((grid_cols - 1) as u8)); // unsigned int(8) columns_minus_one;
        if grid_flags & 1 != 0 {
            avif_checkres!(s.write_u32(grid_width)); // unsigned int(FieldLength) output_width;
            avif_checkres!(s.write_u32(grid_height)); // unsigned int(FieldLength) output_height;
        } else {
            let tmp_width = grid_width as u16;
            let tmp_height = grid_height as u16;
            avif_checkres!(s.write_u16(tmp_width)); // unsigned int(FieldLength) output_width;
            avif_checkres!(s.write_u16(tmp_height)); // unsigned int(FieldLength) output_height;
        }
        s.finish_write();
        AvifResult::Ok
    })();
    *data = s.raw;
    result
}

/// Translation of `avifEncoderDataCreateExifItem()`.
fn avif_encoder_data_create_exif_item(data: &mut AvifEncoderData, exif: &[u8]) -> AvifResult {
    let mut exif_tiff_header_offset = 0;
    let result = avif_get_exif_tiff_header_offset(exif, &mut exif_tiff_header_offset);
    if result != AvifResult::Ok {
        // Couldn't find the TIFF header
        return result;
    }

    let Some(exif_item) = avif_encoder_data_create_item(data, b"Exif", b"Exif\0", 0) else {
        return AvifResult::OutOfMemory;
    };
    let primary_item_id = data.primary_item_id;
    let exif_item = &mut data.items[exif_item];
    exif_item.iref_to_id = primary_item_id;
    exif_item.iref_type = b"cdsc";

    let offset32bit = (exif_tiff_header_offset as u32).to_be_bytes();
    avif_checkres!(avif_rw_data_realloc(
        &mut exif_item.metadata_payload,
        offset32bit.len() + exif.len()
    ));
    exif_item.metadata_payload[..4].copy_from_slice(&offset32bit);
    exif_item.metadata_payload[4..].copy_from_slice(exif);
    AvifResult::Ok
}

/// Translation of `avifEncoderDataCreateXMPItem()`.
fn avif_encoder_data_create_xmp_item(data: &mut AvifEncoderData, xmp: &[u8]) -> AvifResult {
    let Some(xmp_item) = avif_encoder_data_create_item(data, b"mime", b"XMP\0", 0) else {
        return AvifResult::OutOfMemory;
    };
    let primary_item_id = data.primary_item_id;
    let xmp_item = &mut data.items[xmp_item];
    xmp_item.iref_to_id = primary_item_id;
    xmp_item.iref_type = b"cdsc";

    xmp_item.infe_content_type = XMP_CONTENT_TYPE;
    avif_checkres!(avif_rw_data_set(&mut xmp_item.metadata_payload, xmp));
    AvifResult::Ok
}

/// Same as avifImageCopy() but pads the dstImage with border pixel values to reach dstWidth and dstHeight.
/// Translation of `avifImageCopyAndPad()`.
fn avif_image_copy_and_pad(
    dst_image: &mut AvifImage,
    src_image: &AvifImage,
    dst_width: u32,
    dst_height: u32,
) -> AvifResult {
    avif_assert_or_return!(dst_image.width == 0 && dst_image.height == 0); // dstImage is not set yet.
    avif_assert_or_return!(dst_width >= src_image.width);
    avif_assert_or_return!(dst_height >= src_image.height);

    // Copy all fields but do not allocate the planes.
    avif_checkres!(avif_image_copy(dst_image, src_image, 0));
    dst_image.width = dst_width;
    dst_image.height = dst_height;

    if src_image.yuv_planes[AVIF_CHAN_Y].is_some() {
        avif_checkres!(avif_image_allocate_planes(dst_image, AVIF_PLANES_YUV));
    }
    if src_image.alpha_plane.is_some() {
        avif_checkres!(avif_image_allocate_planes(dst_image, AVIF_PLANES_A));
    }
    let uses_u16 = avif_image_uses_u16(src_image);
    for plane in AVIF_CHAN_Y..=AVIF_CHAN_A {
        let src_row_bytes = avif_image_plane_row_bytes(src_image, plane) as usize;
        let src_plane_width = avif_image_plane_width(src_image, plane) as usize;
        let src_plane_height = avif_image_plane_height(src_image, plane) as usize; // 0 for A if no alpha and 0 for UV if 4:0:0.

        let dst_row_bytes = avif_image_plane_row_bytes(dst_image, plane) as usize;
        let dst_plane_width = avif_image_plane_width(dst_image, plane) as usize;
        let dst_plane_height = avif_image_plane_height(dst_image, plane) as usize; // 0 for A if no alpha and 0 for UV if 4:0:0.

        let (Some(src_row), Some(dst_row)) = (
            avif_image_plane(src_image, plane).cloned(),
            avif_image_plane_mut(dst_image, plane),
        ) else {
            continue;
        };
        if uses_u16 {
            let src = src_row.u16s();
            let dst = dst_row.u16s_mut();
            let (srs, drs) = (src_row_bytes / 2, dst_row_bytes / 2);
            let mut j = 0;
            while j < src_plane_height {
                let d = j * drs;
                dst[d..d + src_plane_width]
                    .copy_from_slice(&src[j * srs..j * srs + src_plane_width]);

                // Pad columns.
                if dst_plane_width > src_plane_width {
                    for x in src_plane_width..dst_plane_width {
                        dst[d + x] = dst[d + src_plane_width - 1];
                    }
                }
                j += 1;
            }

            // Pad rows.
            for j in src_plane_height..dst_plane_height {
                dst.copy_within((j - 1) * drs..(j - 1) * drs + dst_plane_width, j * drs);
            }
        } else {
            let src = src_row.u8s();
            let dst = dst_row.u8s_mut();
            let (srs, drs) = (src_row_bytes, dst_row_bytes);
            for j in 0..src_plane_height {
                let d = j * drs;
                dst[d..d + src_plane_width]
                    .copy_from_slice(&src[j * srs..j * srs + src_plane_width]);

                // Pad columns.
                if dst_plane_width > src_plane_width {
                    let v = dst[d + src_plane_width - 1];
                    dst[d + src_plane_width..d + dst_plane_width].fill(v);
                }
            }

            // Pad rows.
            for j in src_plane_height..dst_plane_height {
                dst.copy_within((j - 1) * drs..(j - 1) * drs + dst_plane_width, j * drs);
            }
        }
    }
    AvifResult::Ok
}

/// Translation of `avifQualityToQuantizer()`.
fn avif_quality_to_quantizer(mut quality: i32, min_quantizer: i32, max_quantizer: i32) -> i32 {
    let mut quantizer;
    if quality == AVIF_QUALITY_DEFAULT {
        // In older libavif releases, avifEncoder didn't have the quality and qualityAlpha fields.
        // Supply a default value for quantizer.
        quantizer = (min_quantizer + max_quantizer) / 2;
        quantizer = quantizer.clamp(0, 63);
    } else {
        quality = quality.clamp(0, 100);
        quantizer = ((100 - quality) * 63 + 50) / 100;
    }
    quantizer
}

const INFE_NAME_COLOR: &[u8] = b"Color\0";
const INFE_NAME_ALPHA: &[u8] = b"Alpha\0";

/// Translation of `getInfeName()` (the name with its NUL terminator).
fn get_infe_name(item_category: AvifItemCategory) -> &'static [u8] {
    if avif_is_alpha(item_category) {
        return INFE_NAME_ALPHA;
    }
    INFE_NAME_COLOR
}

// Adds the items for a single cell or a grid of cells. Outputs the topLevelItemID which is
// the only item if there is exactly one cell, or the grid item for multiple cells.
// Note: The topLevelItemID output argument has the type uint16_t* instead of avifEncoderItem** because
//       the avifEncoderItem pointer may be invalidated by a call to avifEncoderDataCreateItem().
/// Translation of `avifEncoderAddImageItems()`.
fn avif_encoder_add_image_items(
    encoder: &mut AvifEncoder,
    grid_cols: u32,
    grid_rows: u32,
    grid_width: u32,
    grid_height: u32,
    item_category: AvifItemCategory,
    top_level_item_id: &mut u16,
) -> AvifResult {
    let cell_count = grid_cols * grid_rows;
    let infe_name = get_infe_name(item_category);

    if cell_count > 1 {
        // FIXME (upstream): the grid item isn't checked for NULL (a failed
        // allocation) before it is used.
        let Some(grid_item) =
            avif_encoder_data_create_item(&mut encoder.data, b"grid", infe_name, 0)
        else {
            return AvifResult::OutOfMemory;
        };
        let grid_item = &mut encoder.data.items[grid_item];
        avif_checkres!(avif_write_grid_payload(
            &mut grid_item.metadata_payload,
            grid_cols,
            grid_rows,
            grid_width,
            grid_height
        ));
        grid_item.item_category = item_category;
        grid_item.grid_cols = grid_cols;
        grid_item.grid_rows = grid_rows;
        grid_item.grid_width = grid_width;
        grid_item.grid_height = grid_height;
        *top_level_item_id = grid_item.id;
    }

    for cell_index in 0..cell_count {
        let image_item_type = encoder.data.image_item_type;
        let item = avif_encoder_data_create_item(
            &mut encoder.data,
            image_item_type,
            infe_name,
            cell_index,
        );
        avif_checkerr!(item.is_some(), AvifResult::OutOfMemory);
        let Some(item) = item else {
            return AvifResult::OutOfMemory;
        };
        let extra_layer_count = encoder.extra_layer_count;
        let codec_choice = encoder.codec_choice;
        let item = &mut encoder.data.items[item];
        avif_checkres!(avif_codec_create_encoder(
            codec_choice,
            AVIF_CODEC_FLAG_CAN_ENCODE,
            &mut item.codec
        ));
        // (item->codec->csOptions and item->codec->diag: the encoder's,
        // passed to the codec's functions)
        item.item_category = item_category;
        item.extra_layer_count = extra_layer_count;

        if cell_count > 1 {
            item.dimg_from_id = *top_level_item_id;
            item.hidden_image = true;
        } else {
            *top_level_item_id = item.id;
        }
    }
    AvifResult::Ok
}

/// Translation of `avifEncoderGetCodecType()`.
fn avif_encoder_get_codec_type(encoder: &AvifEncoder) -> AvifCodecType {
    // TODO(yguyon): Rework when AVIF_CODEC_CHOICE_AUTO can be AVM
    debug_assert!(
        (encoder.codec_choice != AvifCodecChoice::Auto)
            || (avif_codec_name(encoder.codec_choice, AVIF_CODEC_FLAG_CAN_ENCODE) != Some("avm"))
    );
    avif_codec_type_from_choice(encoder.codec_choice, AVIF_CODEC_FLAG_CAN_ENCODE)
}

// This function is called after every color frame is encoded. It returns AVIF_TRUE if a keyframe needs to be forced for the next
// alpha frame to be encoded, AVIF_FALSE otherwise.
/// Translation of `avifEncoderDataShouldForceKeyframeForAlpha()`.
fn avif_encoder_data_should_force_keyframe_for_alpha(
    data: &AvifEncoderData,
    color_item: &AvifEncoderItem,
    add_image_flags: AvifAddImageFlags,
) -> bool {
    if !data.alpha_present {
        // There is no alpha plane.
        return false;
    }
    if add_image_flags & AVIF_ADD_IMAGE_FLAG_SINGLE != 0 {
        // Not an animated image.
        return false;
    }
    if data.frames.is_empty() {
        // data->frames.count is the number of frames that have been encoded so far by previous calls to avifEncoderAddImage. If
        // this is the first frame, there is no need to force keyframe.
        return false;
    }
    let color_frames_output_so_far = color_item.encode_output.samples.len();
    let is_lagged_output = (data.frames.len() + 1) != color_frames_output_so_far;
    if is_lagged_output {
        // If the encoder is operating with lag, then there is no way to determine if the last encoded frame was a keyframe until
        // the encoder outputs it (after the lag). So do not force keyframe for alpha channel in this case.
        return false;
    }
    color_item.encode_output.samples[color_frames_output_so_far - 1].sync
}

/// Translation of `avifGetErrorForItemCategory()`.
fn avif_get_error_for_item_category(item_category: AvifItemCategory) -> AvifResult {
    if avif_is_alpha(item_category) {
        AvifResult::EncodeAlphaFailed
    } else {
        AvifResult::EncodeColorFailed
    }
}

/// Translation of `avifGridWidth()`.
fn avif_grid_width(grid_cols: u32, first_cell: &AvifImage, bottom_right_cell: &AvifImage) -> u32 {
    (grid_cols - 1) * first_cell.width + bottom_right_cell.width
}

/// Translation of `avifGridHeight()`.
fn avif_grid_height(grid_rows: u32, first_cell: &AvifImage, bottom_right_cell: &AvifImage) -> u32 {
    (grid_rows - 1) * first_cell.height + bottom_right_cell.height
}

/// Translation of `avifValidateGrid()`.
fn avif_validate_grid(
    grid_cols: u32,
    grid_rows: u32,
    cell_images: &[&AvifImage],
    validate_gain_map: bool,
    diag: &AvifDiagnostics,
) -> AvifResult {
    let cell_count = grid_cols * grid_rows;
    let first_cell = cell_images[0];
    let bottom_right_cell = cell_images[cell_count as usize - 1];
    let tile_width = first_cell.width;
    let tile_height = first_cell.height;
    let grid_width = avif_grid_width(grid_cols, first_cell, bottom_right_cell);
    let grid_height = avif_grid_height(grid_rows, first_cell, bottom_right_cell);
    for cell_index in 0..cell_count {
        let cell_image = cell_images[cell_index as usize];
        let expected_cell_width = if (cell_index + 1) % grid_cols != 0 {
            tile_width
        } else {
            bottom_right_cell.width
        };
        let expected_cell_height = if cell_index < (cell_count - grid_cols) {
            tile_height
        } else {
            bottom_right_cell.height
        };
        if (cell_image.width != expected_cell_width) || (cell_image.height != expected_cell_height)
        {
            printf(
                Some(diag),
                format_args!(
                    "{} cell {} has invalid dimensions: expected {}x{} found {}x{}",
                    if validate_gain_map {
                        "gain map"
                    } else {
                        "image"
                    },
                    cell_index,
                    expected_cell_width,
                    expected_cell_height,
                    cell_image.width,
                    cell_image.height
                ),
            );
            return AvifResult::InvalidImageGrid;
        }

        // MIAF (ISO 23000-22:2019), Section 7.3.11.4.1:
        //   All input images of a grid image item shall use the same coding format, chroma sampling format, and the
        //   same decoder configuration (see 7.3.6.2).
        if (cell_image.depth != first_cell.depth)
            || (cell_image.yuv_format != first_cell.yuv_format)
            || (cell_image.yuv_range != first_cell.yuv_range)
            || (cell_image.color_primaries != first_cell.color_primaries)
            || (cell_image.transfer_characteristics != first_cell.transfer_characteristics)
            || (cell_image.matrix_coefficients != first_cell.matrix_coefficients)
            || (cell_image.alpha_plane.is_some() != first_cell.alpha_plane.is_some())
            || (cell_image.alpha_premultiplied != first_cell.alpha_premultiplied)
        {
            printf(
                Some(diag),
                format_args!(
                    "all grid cells should have the same value for: depth, yuvFormat, yuvRange, colorPrimaries, \
                     transferCharacteristics, matrixCoefficients, alphaPlane presence, alphaPremultiplied"
                ),
            );
            return AvifResult::InvalidImageGrid;
        }

        if cell_image.yuv_planes[AVIF_CHAN_Y].is_none() {
            return AvifResult::NoContent;
        }
    }

    if (bottom_right_cell.width > tile_width) || (bottom_right_cell.height > tile_height) {
        printf(
            Some(diag),
            format_args!(
                "the last {} cell can be smaller but not larger than the other cells which are {}x{}, found {}x{}",
                if validate_gain_map { "gain map" } else { "image" },
                tile_width,
                tile_height,
                bottom_right_cell.width,
                bottom_right_cell.height
            ),
        );
        return AvifResult::InvalidImageGrid;
    }
    if (cell_count > 1)
        && !avif_are_grid_dimensions_valid(
            first_cell.yuv_format,
            grid_width,
            grid_height,
            tile_width,
            tile_height,
            Some(diag),
        )
    {
        return AvifResult::InvalidImageGrid;
    }

    AvifResult::Ok
}

/// Translation of `avifEncoderAddImageInternal()`.
fn avif_encoder_add_image_internal(
    encoder: &mut AvifEncoder,
    grid_cols: u32,
    grid_rows: u32,
    cell_images: &[&AvifImage],
    mut duration_in_timescales: u64,
    mut add_image_flags: AvifAddImageFlags,
) -> AvifResult {
    // -----------------------------------------------------------------------
    // Verify encoding is possible

    if avif_codec_name(encoder.codec_choice, AVIF_CODEC_FLAG_CAN_ENCODE).is_none() {
        return AvifResult::NoCodecAvailable;
    }

    if encoder.extra_layer_count >= AVIF_MAX_AV1_LAYER_COUNT {
        printf(
            Some(&encoder.diag),
            format_args!(
                "extraLayerCount [{}] must be less than {}",
                encoder.extra_layer_count, AVIF_MAX_AV1_LAYER_COUNT
            ),
        );
        return AvifResult::InvalidArgument;
    }

    // -----------------------------------------------------------------------
    // Validate images

    let cell_count = grid_cols * grid_rows;
    if cell_count == 0 {
        return AvifResult::InvalidArgument;
    }

    let first_cell = cell_images[0];
    let bottom_right_cell = cell_images[cell_count as usize - 1];
    avif_checkerr!(
        first_cell.depth == 8 || first_cell.depth == 10 || first_cell.depth == 12,
        AvifResult::UnsupportedDepth
    );
    avif_checkerr!(
        first_cell.yuv_format != AvifPixelFormat::None,
        AvifResult::NoYuvFormatSelected
    );
    if first_cell.width == 0
        || first_cell.height == 0
        || bottom_right_cell.width == 0
        || bottom_right_cell.height == 0
    {
        return AvifResult::NoContent;
    }

    avif_checkres!(avif_validate_grid(
        grid_cols,
        grid_rows,
        cell_images,
        /*validateGainMap=*/ false,
        &encoder.diag
    ));

    // -----------------------------------------------------------------------
    // Validate flags

    if encoder.data.single_image {
        // The previous call to avifEncoderAddImage() set AVIF_ADD_IMAGE_FLAG_SINGLE.
        // avifEncoderAddImage() cannot be called again for this encode.
        return AvifResult::EncodeColorFailed;
    }

    if add_image_flags & AVIF_ADD_IMAGE_FLAG_SINGLE != 0 {
        encoder.data.single_image = true;

        if encoder.extra_layer_count > 0 {
            // AVIF_ADD_IMAGE_FLAG_SINGLE may not be set for layered image.
            return AvifResult::InvalidArgument;
        }

        if !encoder.data.items.is_empty() {
            // AVIF_ADD_IMAGE_FLAG_SINGLE may only be set on the first and only image.
            return AvifResult::InvalidArgument;
        }
    }

    // -----------------------------------------------------------------------
    // Choose AV1 or AV2

    let codec_type = avif_encoder_get_codec_type(encoder);
    match codec_type {
        AvifCodecType::Av1 => {
            encoder.data.image_item_type = b"av01";
            encoder.data.config_prop_name = b"av1C";
        }
        _ => return AvifResult::NoCodecAvailable,
    }

    // -----------------------------------------------------------------------
    // Map quality and qualityAlpha to quantizer and quantizerAlpha
    encoder.data.quantizer = avif_quality_to_quantizer(
        encoder.quality,
        encoder.min_quantizer,
        encoder.max_quantizer,
    );
    encoder.data.quantizer_alpha = avif_quality_to_quantizer(
        encoder.quality_alpha,
        encoder.min_quantizer_alpha,
        encoder.max_quantizer_alpha,
    );

    // -----------------------------------------------------------------------
    // Handle automatic tiling

    encoder.data.tile_rows_log2 = encoder.tile_rows_log2.clamp(0, 6);
    encoder.data.tile_cols_log2 = encoder.tile_cols_log2.clamp(0, 6);
    if encoder.auto_tiling {
        // Use as many tiles as allowed by the minimum tile area requirement and impose a maximum
        // of 8 tiles.
        let threads = 8;
        let data = &mut *encoder.data;
        avif_set_tile_configuration(
            threads,
            first_cell.width,
            first_cell.height,
            &mut data.tile_rows_log2,
            &mut data.tile_cols_log2,
        );
    }

    // -----------------------------------------------------------------------
    // All encoder settings are known now. Detect changes.

    let mut encoder_changes: AvifEncoderChanges = 0;
    if !avif_encoder_detect_changes(encoder, &mut encoder_changes) {
        return AvifResult::CannotChangeSetting;
    }
    avif_encoder_backup_settings(encoder);

    // -----------------------------------------------------------------------

    if duration_in_timescales == 0 {
        duration_in_timescales = 1;
    }

    if encoder.data.items.is_empty() {
        // Make a copy of the first image's metadata (sans pixels) for future writing/validation
        avif_checkres!(avif_image_copy(
            &mut encoder.data.image_metadata,
            first_cell,
            0
        ));

        // Prepare all AV1 items
        let mut color_item_id: u16 = 0;
        let grid_width = avif_grid_width(grid_cols, first_cell, bottom_right_cell);
        let grid_height = avif_grid_height(grid_rows, first_cell, bottom_right_cell);
        avif_checkres!(avif_encoder_add_image_items(
            encoder,
            grid_cols,
            grid_rows,
            grid_width,
            grid_height,
            AvifItemCategory::Color,
            &mut color_item_id
        ));
        encoder.data.primary_item_id = color_item_id;

        encoder.data.alpha_present = first_cell.alpha_plane.is_some();
        if encoder.data.alpha_present && (add_image_flags & AVIF_ADD_IMAGE_FLAG_SINGLE != 0) {
            // If encoding a single image in which the alpha plane exists but is entirely opaque,
            // simply skip writing an alpha AV1 payload entirely, as it'll be interpreted as opaque
            // and is less bytes.
            //
            // However, if encoding an image sequence, the first frame's alpha plane being entirely
            // opaque could be a false positive for removing the alpha AV1 payload, as it might simply
            // be a fade out later in the sequence. This is why avifImageIsOpaque() is only called
            // when encoding a single image.

            encoder.data.alpha_present = false;
            for cell_index in 0..cell_count {
                let cell_image = cell_images[cell_index as usize];
                if !avif_image_is_opaque(cell_image) {
                    encoder.data.alpha_present = true;
                    break;
                }
            }
        }

        if encoder.data.alpha_present {
            let mut alpha_item_id: u16 = 0;
            avif_checkres!(avif_encoder_add_image_items(
                encoder,
                grid_cols,
                grid_rows,
                grid_width,
                grid_height,
                AvifItemCategory::Alpha,
                &mut alpha_item_id
            ));
            let alpha_item = avif_encoder_data_find_item_by_id(&encoder.data, alpha_item_id);
            avif_assert_or_return!(alpha_item.is_some());
            let Some(alpha_item) = alpha_item else {
                return AvifResult::InternalError;
            };
            encoder.data.items[alpha_item].iref_type = b"auxl";
            encoder.data.items[alpha_item].iref_to_id = color_item_id;
            if encoder.data.image_metadata.alpha_premultiplied {
                let color_item = avif_encoder_data_find_item_by_id(&encoder.data, color_item_id);
                avif_assert_or_return!(color_item.is_some());
                let Some(color_item) = color_item else {
                    return AvifResult::InternalError;
                };
                encoder.data.items[color_item].iref_type = b"prem";
                encoder.data.items[color_item].iref_to_id = alpha_item_id;
            }
        }

        // -----------------------------------------------------------------------
        // Create metadata items (Exif, XMP)

        if !first_cell.exif.is_empty() {
            let result = avif_encoder_data_create_exif_item(&mut encoder.data, &first_cell.exif);
            if result != AvifResult::Ok {
                return result;
            }
        }

        if !first_cell.xmp.is_empty() {
            let result = avif_encoder_data_create_xmp_item(&mut encoder.data, &first_cell.xmp);
            if result != AvifResult::Ok {
                return result;
            }
        }
    } else {
        // Another frame in an image sequence, or layer in a layered image

        let image_metadata = &encoder.data.image_metadata;
        // Image metadata that are copied to the configuration property and nclx boxes are not allowed to change.
        // If the first image in the sequence had an alpha plane (even if fully opaque), all
        // subsequent images must have alpha as well.
        if (image_metadata.depth != first_cell.depth)
            || (image_metadata.yuv_format != first_cell.yuv_format)
            || (image_metadata.yuv_range != first_cell.yuv_range)
            || (image_metadata.yuv_chroma_sample_position != first_cell.yuv_chroma_sample_position)
            || (image_metadata.color_primaries != first_cell.color_primaries)
            || (image_metadata.transfer_characteristics != first_cell.transfer_characteristics)
            || (image_metadata.matrix_coefficients != first_cell.matrix_coefficients)
            || (image_metadata.alpha_premultiplied != first_cell.alpha_premultiplied)
            || (encoder.data.alpha_present && first_cell.alpha_plane.is_none())
        {
            return AvifResult::IncompatibleImage;
        }
    }

    if encoder.data.frames.len() == 1 {
        // We will be writing an image sequence. When writing the AV1SampleEntry (derived from
        // VisualSampleEntry) in the stsd box, we need to cast imageMetadata->width and
        // imageMetadata->height to uint16_t:
        //     class VisualSampleEntry(codingname) extends SampleEntry (codingname){
        //        ...
        //        unsigned int(16) width;
        //        unsigned int(16) height;
        //        ...
        //     }
        // Check whether it is safe to cast width and height to uint16_t. The maximum width and
        // height of an AV1 frame are 65536, which just exceeds uint16_t.
        avif_assert_or_return!(!encoder.data.items.is_empty());
        let image_metadata = &encoder.data.image_metadata;
        avif_checkerr!(
            image_metadata.width <= 65535 && image_metadata.height <= 65535,
            AvifResult::InvalidArgument
        );
    }

    // -----------------------------------------------------------------------
    // Encode AV1 OBUs

    for item_index in 0..encoder.data.items.len() {
        if encoder.data.items[item_index].codec.is_some() {
            let item = &encoder.data.items[item_index];
            let mut cell_image: &AvifImage = cell_images[item.cell_index as usize];
            // May be used as a temporary, modified cellImage. Left as NULL otherwise.
            let mut cell_image_placeholder: Option<Box<AvifImage>> = None;
            let first_cell_image = first_cell;

            if (cell_image.width != first_cell_image.width)
                || (cell_image.height != first_cell_image.height)
            {
                // Pad the right-most and/or bottom-most tiles so that all tiles share the same dimensions.
                let placeholder = avif_image_create_empty();
                avif_checkerr!(placeholder.is_some(), AvifResult::OutOfMemory);
                let Some(mut placeholder) = placeholder else {
                    return AvifResult::OutOfMemory;
                };
                let result = avif_image_copy_and_pad(
                    &mut placeholder,
                    cell_image,
                    first_cell_image.width,
                    first_cell_image.height,
                );
                if result != AvifResult::Ok {
                    return result;
                }
                cell_image_placeholder = Some(placeholder);
            }
            if let Some(placeholder) = &cell_image_placeholder {
                cell_image = placeholder;
            }

            let is_alpha = avif_is_alpha(item.item_category);
            let quantizer = if is_alpha {
                encoder.data.quantizer_alpha
            } else {
                encoder.data.quantizer
            };

            // If alpha channel is present, set disableLaggedOutput to AVIF_TRUE. If the encoder supports it, this enables
            // avifEncoderDataShouldForceKeyframeForAlpha to force a keyframe in the alpha channel whenever a keyframe has been
            // encoded in the color channel for animated images.
            let item = &mut encoder.data.items[item_index];
            let item_category = item.item_category;
            let (Some(mut codec), mut encode_output) =
                (item.codec.take(), mem::take(&mut item.encode_output))
            else {
                return AvifResult::InternalError;
            };
            let mut encode_result = codec.encode_image(
                encoder,
                cell_image,
                is_alpha,
                encoder.data.tile_rows_log2,
                encoder.data.tile_cols_log2,
                quantizer,
                encoder_changes,
                /*disableLaggedOutput=*/ encoder.data.alpha_present,
                add_image_flags,
                &mut encode_output,
            );
            let item = &mut encoder.data.items[item_index];
            item.codec = Some(codec);
            item.encode_output = encode_output;
            drop(cell_image_placeholder);
            if encode_result == AvifResult::UnknownError {
                encode_result = avif_get_error_for_item_category(item_category);
            }
            avif_checkres!(encode_result);
            if item_index == 0
                && avif_encoder_data_should_force_keyframe_for_alpha(
                    &encoder.data,
                    &encoder.data.items[item_index],
                    add_image_flags,
                )
            {
                add_image_flags |= AVIF_ADD_IMAGE_FLAG_FORCE_KEYFRAME;
            }
        }
    }

    avif_codec_specific_options_clear(&mut encoder.cs_options);
    avif_checkerr!(
        encoder.data.frames.try_reserve(1).is_ok(),
        AvifResult::OutOfMemory
    );
    encoder.data.frames.push(AvifEncoderFrame {
        duration_in_timescales,
    });
    AvifResult::Ok
}

/// Translation of `avifEncoderAddImage()`.
pub(crate) fn avif_encoder_add_image(
    encoder: &mut AvifEncoder,
    image: &AvifImage,
    duration_in_timescales: u64,
    add_image_flags: AvifAddImageFlags,
) -> AvifResult {
    clear_error(Some(&encoder.diag));
    avif_encoder_add_image_internal(
        encoder,
        1,
        1,
        &[image],
        duration_in_timescales,
        add_image_flags,
    )
}

/// Translation of `avifEncoderAddImageGrid()`.
pub(crate) fn avif_encoder_add_image_grid(
    encoder: &mut AvifEncoder,
    grid_cols: u32,
    grid_rows: u32,
    cell_images: &[&AvifImage],
    mut add_image_flags: AvifAddImageFlags,
) -> AvifResult {
    clear_error(Some(&encoder.diag));
    if (grid_cols == 0) || (grid_cols > 256) || (grid_rows == 0) || (grid_rows > 256) {
        return AvifResult::InvalidImageGrid;
    }
    if encoder.extra_layer_count == 0 {
        add_image_flags |= AVIF_ADD_IMAGE_FLAG_SINGLE; // image grids cannot be image sequences
    }
    avif_encoder_add_image_internal(
        encoder,
        grid_cols,
        grid_rows,
        cell_images,
        1,
        add_image_flags,
    )
}

/// Translation of `avifEncoderFindExistingChunk()`.
fn avif_encoder_find_existing_chunk(
    s: &AvifRWStream,
    mdat_start_offset: usize,
    data: &[u8],
) -> usize {
    let size = data.len();
    let mdat_current_offset = s.offset();
    let mdat_search_size = mdat_current_offset - mdat_start_offset;
    if mdat_search_size < size {
        return 0;
    }
    let mdat_end_search_offset = mdat_current_offset - size;
    for search_offset in mdat_start_offset..=mdat_end_search_offset {
        if data == &s.raw[search_offset..search_offset + size] {
            return search_offset;
        }
    }
    0
}

/// Translation of `avifEncoderWriteMediaDataBox()` (the item references
/// are item indices).
fn avif_encoder_write_media_data_box(
    encoder: &mut AvifEncoder,
    s: &mut AvifRWStream,
    layered_color_items: &mut Vec<usize>,
    layered_alpha_items: &mut Vec<usize>,
) -> AvifResult {
    encoder.io_stats.color_obu_size = 0;
    encoder.io_stats.alpha_obu_size = 0;
    encoder.data.gain_map_size_bytes = 0;

    let mut mdat: AvifBoxMarker = 0;
    avif_checkres!(s.write_box(b"mdat", AVIF_BOX_SIZE_TBD, Some(&mut mdat)));
    let mdat_start_offset = s.offset();
    for item_passes in 0..3u32 {
        // Use multiple passes to pack in the following order:
        //   * Pass 0: metadata (Exif/XMP)
        //   * Pass 1: alpha, gain map (AV1)
        //   * Pass 2: all other item data (AV1 color)
        //
        // See here for the discussion on alpha coming before color:
        // https://github.com/AOMediaCodec/libavif/issues/287
        //
        // Exif and XMP are packed first as they're required to be fully available
        // by avifDecoderParse() before it returns AVIF_RESULT_OK, unless ignoreXMP
        // and ignoreExif are enabled.
        //
        let metadata_pass = item_passes == 0;
        let alpha_and_gain_map_pass = item_passes == 1;

        for item_index in 0..encoder.data.items.len() {
            let item = &encoder.data.items[item_index];
            if item.metadata_payload.is_empty() && item.encode_output.samples.is_empty() {
                // this item has nothing for the mdat box
                continue;
            }
            let is_metadata = &item.type_ == b"mime" || &item.type_ == b"Exif";
            if metadata_pass != is_metadata {
                // only process metadata (XMP/Exif) payloads when metadataPass is true
                continue;
            }
            let is_alpha = avif_is_alpha(item.item_category);
            let is_alpha_or_gain_map = is_alpha;
            if alpha_and_gain_map_pass != is_alpha_or_gain_map {
                // only process alpha payloads when alphaPass is true
                continue;
            }

            if (encoder.extra_layer_count > 0) && !item.encode_output.samples.is_empty() {
                // Interleave - Pick out AV1 items and interleave them later.
                // We always interleave all AV1 items for layered images.
                avif_assert_or_return!(item.encode_output.samples.len() == item.mdat_fixups.len());

                let refs = if is_alpha {
                    &mut *layered_alpha_items
                } else {
                    &mut *layered_color_items
                };
                avif_checkerr!(refs.try_reserve(1).is_ok(), AvifResult::OutOfMemory);
                refs.push(item_index);
                continue;
            }

            let mut chunk_offset = 0;

            // Deduplication - See if an identical chunk to this has already been written.
            // Doing it when item->encodeOutput->samples.count > 1 would require contiguous memory.
            if item.encode_output.samples.len() == 1 {
                let sample = &item.encode_output.samples[0];
                chunk_offset = avif_encoder_find_existing_chunk(s, mdat_start_offset, &sample.data);
            } else if item.encode_output.samples.is_empty() {
                chunk_offset =
                    avif_encoder_find_existing_chunk(s, mdat_start_offset, &item.metadata_payload);
            }

            if chunk_offset == 0 {
                // We've never seen this chunk before; write it out
                chunk_offset = s.offset();
                if !item.encode_output.samples.is_empty() {
                    for sample in &item.encode_output.samples {
                        avif_checkres!(s.write(&sample.data));

                        if is_alpha {
                            encoder.io_stats.alpha_obu_size += sample.data.len();
                        } else if item.item_category == AvifItemCategory::Color {
                            encoder.io_stats.color_obu_size += sample.data.len();
                        }
                    }
                } else {
                    avif_checkres!(s.write(&item.metadata_payload));
                }
            }

            for fixup in &item.mdat_fixups {
                let prev_offset = s.offset();
                s.set_offset(fixup.offset);
                avif_checkres!(s.write_u32(chunk_offset as u32));
                s.set_offset(prev_offset);
            }
        }
    }

    let layered_item_count = layered_color_items.len().max(layered_alpha_items.len());
    if layered_item_count > 0 {
        // Interleave samples of all AV1 items.
        // We first write the first layer of all items,
        // in which we write first layer of each cell,
        // in which we write alpha first and then color.
        let mut has_more_sample;
        let mut layer_index: usize = 0;
        loop {
            has_more_sample = false;
            for item_index in 0..layered_item_count {
                for sample_pass in 0..2 {
                    // Alpha coming before color
                    let current_items = if sample_pass == 0 {
                        &*layered_alpha_items
                    } else {
                        &*layered_color_items
                    };
                    if item_index >= current_items.len() {
                        continue;
                    }

                    // TODO: Offer the ability for a user to specify which grid cell should be written first.
                    let item = &encoder.data.items[current_items[item_index]];
                    if item.encode_output.samples.len() <= layer_index {
                        // We've already written all samples of this item
                        continue;
                    } else if item.encode_output.samples.len() > layer_index + 1 {
                        has_more_sample = true;
                    }
                    let data = &item.encode_output.samples[layer_index].data;
                    let mut chunk_offset =
                        avif_encoder_find_existing_chunk(s, mdat_start_offset, data);
                    if chunk_offset == 0 {
                        // We've never seen this chunk before; write it out
                        chunk_offset = s.offset();
                        avif_checkres!(s.write(data));
                        if sample_pass == 0 {
                            encoder.io_stats.alpha_obu_size += data.len();
                        } else {
                            encoder.io_stats.color_obu_size += data.len();
                        }
                    }

                    let prev_offset = s.offset();
                    s.set_offset(item.mdat_fixups[layer_index].offset);
                    avif_checkres!(s.write_u32(chunk_offset as u32));
                    s.set_offset(prev_offset);
                }
            }
            layer_index += 1;
            if !has_more_sample {
                break;
            }
        }

        avif_assert_or_return!(layer_index <= AVIF_MAX_AV1_LAYER_COUNT as usize);
    }
    s.finish_box(mdat);
    AvifResult::Ok
}

/// Translation of `avifWriteAltrGroup()`.
fn avif_write_altr_group(s: &mut AvifRWStream, item_ids: &[u16]) -> AvifResult {
    let mut grpl: AvifBoxMarker = 0;
    avif_checkres!(s.write_box(b"grpl", AVIF_BOX_SIZE_TBD, Some(&mut grpl)));

    let mut altr: AvifBoxMarker = 0;
    avif_checkres!(s.write_full_box(b"altr", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut altr)));

    avif_checkres!(s.write_u32(1)); // unsigned int(32) group_id;
    avif_checkres!(s.write_u32(item_ids.len() as u32)); // unsigned int(32) num_entities_in_group;
    for &item_id in item_ids {
        avif_checkres!(s.write_u32(item_id as u32)); // unsigned int(32) entity_id;
    }

    s.finish_box(altr);

    s.finish_box(grpl);

    AvifResult::Ok
}

/// Translation of `avifRWStreamWriteProperties()` (the image metadata is
/// the encoder data's; there is no alternative image metadata without gain
/// maps).
fn avif_rw_stream_write_properties(
    dedup: &mut AvifItemPropertyDedup,
    s: &mut AvifRWStream,
    data: &mut AvifEncoderData,
) -> AvifResult {
    let image_metadata = &*data.image_metadata;
    for item_index in 0..data.items.len() {
        let item = &mut data.items[item_index];
        let is_grid = item.grid_cols > 0;
        // Whether there is ipma to write for this item.
        let has_ipma_to_write = item.codec.is_some() || is_grid;
        item.ipma = IpmaArray::default();
        if !has_ipma_to_write {
            continue;
        }

        if item.dimg_from_id != 0 && (item.extra_layer_count == 0) {
            let dimg_from_id = item.dimg_from_id;
            let parent_item = avif_encoder_data_find_item_by_id(data, dimg_from_id);
            if let Some(parent_item) = parent_item {
                if &data.items[parent_item].type_ == b"grid" {
                    // All image cells from a grid should share the exact same properties unless they are
                    // layered image which have different al1x, so see if we've already written properties
                    // out for another cell in this grid, and if so, just steal their ipma and move on.
                    // This is a sneaky way to provide iprp deduplication.

                    let mut found_previous_cell = false;
                    for dedup_index in 0..item_index {
                        let dedup_item = &data.items[dedup_index];
                        if (dimg_from_id == dedup_item.dimg_from_id)
                            && (dedup_item.extra_layer_count == 0)
                        {
                            // We've already written dedup's items out. Steal their ipma indices and move on!
                            data.items[item_index].ipma = data.items[dedup_index].ipma;
                            found_previous_cell = true;
                            break;
                        }
                    }
                    if found_previous_cell {
                        continue;
                    }
                }
            }
        }

        let item = &mut data.items[item_index];
        let item_metadata = image_metadata;
        let mut image_width = item_metadata.width;
        let mut image_height = item_metadata.height;
        if is_grid {
            image_width = item.grid_width;
            image_height = item.grid_height;
        }

        // Properties all image items need (coded and derived)
        // ispe = image spatial extent (width, height)
        avif_item_property_dedup_start(dedup);
        let mut ispe: AvifBoxMarker = 0;
        avif_checkres!(dedup
            .s
            .write_full_box(b"ispe", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut ispe)));
        avif_checkres!(dedup.s.write_u32(image_width)); // unsigned int(32) image_width;
        avif_checkres!(dedup.s.write_u32(image_height)); // unsigned int(32) image_height;
        dedup.s.finish_box(ispe);
        avif_checkres!(avif_item_property_dedup_finish(
            dedup,
            s,
            &mut item.ipma,
            false
        ));

        // pixi = pixel information (depth, channel count)
        let has_pixi = true;
        let is_alpha = avif_is_alpha(item.item_category);
        let depth = item_metadata.depth as u8;
        if has_pixi {
            avif_item_property_dedup_start(dedup);
            let channel_count: u8 =
                if is_alpha || (item_metadata.yuv_format == AvifPixelFormat::Yuv400) {
                    1
                } else {
                    3
                };
            let mut pixi: AvifBoxMarker = 0;
            avif_checkres!(dedup.s.write_full_box(
                b"pixi",
                AVIF_BOX_SIZE_TBD,
                0,
                0,
                Some(&mut pixi)
            ));
            avif_checkres!(dedup.s.write_u8(channel_count)); // unsigned int (8) num_channels;
            for _chan in 0..channel_count {
                avif_checkres!(dedup.s.write_u8(depth)); // unsigned int (8) bits_per_channel;
            }
            dedup.s.finish_box(pixi);
            avif_checkres!(avif_item_property_dedup_finish(
                dedup,
                s,
                &mut item.ipma,
                false
            ));
        }

        // Codec configuration box ('av1C' or 'av2C')
        if item.codec.is_some() {
            avif_item_property_dedup_start(dedup);
            avif_checkres!(write_config_box(
                &mut dedup.s,
                &item.av1c,
                data.config_prop_name
            ));
            avif_checkres!(avif_item_property_dedup_finish(
                dedup,
                s,
                &mut item.ipma,
                true
            ));
        }

        if is_alpha {
            // Alpha specific properties

            avif_item_property_dedup_start(dedup);
            let mut aux_c: AvifBoxMarker = 0;
            avif_checkres!(dedup.s.write_full_box(
                b"auxC",
                AVIF_BOX_SIZE_TBD,
                0,
                0,
                Some(&mut aux_c)
            ));
            avif_checkres!(dedup.s.write_chars(ALPHA_URN)); //  string aux_type;
            dedup.s.finish_box(aux_c);
            avif_checkres!(avif_item_property_dedup_finish(
                dedup,
                s,
                &mut item.ipma,
                false
            ));
        } else if item.item_category == AvifItemCategory::Color {
            // Color specific properties
            // Note the 'tmap' (tone mapped image) item when a gain map is present also has itemCategory AVIF_ITEM_COLOR.

            avif_checkres!(avif_encoder_write_color_properties(
                s,
                item_metadata,
                Some(&mut item.ipma),
                Some(dedup)
            ));
            avif_checkres!(avif_encoder_write_hdr_properties(
                s,
                item_metadata,
                Some(&mut item.ipma),
                Some(dedup)
            ));
        }

        if item.extra_layer_count > 0 {
            // Layered Image Indexing Property

            avif_item_property_dedup_start(dedup);
            let mut a1lx: AvifBoxMarker = 0;
            avif_checkres!(dedup
                .s
                .write_box(b"a1lx", AVIF_BOX_SIZE_TBD, Some(&mut a1lx)));
            let mut layer_size = [0u32; AVIF_MAX_AV1_LAYER_COUNT as usize - 1];
            let mut large_size = false;

            for valid_layer in 0..item.extra_layer_count as usize {
                let size = item.encode_output.samples[valid_layer].data.len() as u32;
                layer_size[valid_layer] = size;
                if size > 0xffff {
                    large_size = true;
                }
            }

            avif_checkres!(dedup.s.write_bits(0, /*bitCount=*/ 7)); // unsigned int(7) reserved = 0;
            avif_checkres!(dedup
                .s
                .write_bits(if large_size { 1 } else { 0 }, /*bitCount=*/ 1)); // unsigned int(1) large_size;

            // FieldLength = (large_size + 1) * 16;
            // unsigned int(FieldLength) layer_size[3];
            for layer in 0..AVIF_MAX_AV1_LAYER_COUNT as usize - 1 {
                if large_size {
                    avif_checkres!(dedup.s.write_u32(layer_size[layer]));
                } else {
                    avif_checkres!(dedup.s.write_u16(layer_size[layer] as u16));
                }
            }
            dedup.s.finish_box(a1lx);
            avif_checkres!(avif_item_property_dedup_finish(
                dedup,
                s,
                &mut item.ipma,
                false
            ));
        }
    }
    AvifResult::Ok
}

/// The output stream of avifEncoderFinish(): the stream started on the
/// output data, which gets the stream's data back when it is dropped
/// (whatever was written, as upstream's stream writes into the output).
struct OutputStream<'o> {
    s: AvifRWStream,
    output: &'o mut Vec<u8>,
}

impl Drop for OutputStream<'_> {
    fn drop(&mut self) {
        *self.output = mem::take(&mut self.s.raw);
    }
}

/// Translation of `avifEncoderFinish()`.
pub(crate) fn avif_encoder_finish(encoder: &mut AvifEncoder, output: &mut Vec<u8>) -> AvifResult {
    clear_error(Some(&encoder.diag));
    if encoder.data.items.is_empty() {
        return AvifResult::NoContent;
    }

    let codec_type = avif_encoder_get_codec_type(encoder);
    if codec_type == AvifCodecType::Unknown {
        return AvifResult::NoCodecAvailable;
    }

    // -----------------------------------------------------------------------
    // Finish up encoding

    for item_index in 0..encoder.data.items.len() {
        let item = &mut encoder.data.items[item_index];
        if let Some(codec) = item.codec.as_mut() {
            if !codec.encode_finish(&encoder.diag, &mut item.encode_output) {
                return avif_get_error_for_item_category(item.item_category);
            }

            if item.encode_output.samples.len() != encoder.data.frames.len() {
                return avif_get_error_for_item_category(item.item_category);
            }

            if (item.extra_layer_count > 0)
                && (item.encode_output.samples.len() != item.extra_layer_count as usize + 1)
            {
                // Check whether user has sent enough frames to encoder.
                printf(
                    Some(&encoder.diag),
                    format_args!(
                        "Expected {} frames given to avifEncoderAddImage() to encode this layered image according to extraLayerCount, but got {} frames.",
                        item.extra_layer_count + 1,
                        item.encode_output.samples.len()
                    ),
                );
                return AvifResult::InvalidArgument;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Harvest configuration properties from sequence headers

    for item in encoder.data.items.iter_mut() {
        if !item.encode_output.samples.is_empty() {
            let first_sample = &item.encode_output.samples[0];
            let mut sequence_header = AvifSequenceHeader::default();
            avif_checkerr!(
                avif_sequence_header_parse(&mut sequence_header, &first_sample.data, codec_type),
                avif_get_error_for_item_category(item.item_category)
            );
            item.av1c = sequence_header.av1c;
        }
    }

    // -----------------------------------------------------------------------
    // Begin write stream

    // The epoch for creation_time and modification_time is midnight, Jan. 1,
    // 1904, in UTC time. Add the number of seconds between that epoch and the
    // Unix epoch.
    let unix_now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let now: u64 = unix_now.wrapping_add(2082844800);

    let mut stream = OutputStream {
        s: AvifRWStream::start(mem::take(output)),
        output,
    };
    let s = &mut stream.s;

    // -----------------------------------------------------------------------
    // Write ftyp

    // Layered sequence is not supported for now.
    let is_sequence = (encoder.extra_layer_count == 0) && (encoder.data.frames.len() > 1);

    let mut major_brand: &[u8] = b"avif";
    if is_sequence {
        major_brand = b"avis";
    }

    let minor_version: u32 = 0;

    // According to section 5.2 of AV1 Image File Format specification v1.1.0:
    //   If the primary item or all the items referenced by the primary item are AV1 image items made only
    //   of Intra Frames, the brand "avio" should be used in the compatible_brands field of the FileTypeBox.
    // See https://aomediacodec.github.io/av1-avif/v1.1.0.html#image-and-image-collection-brand.
    // This rule corresponds to using the "avio" brand in all cases except for layered images, because:
    //  - Non-layered still images are always Intra Frames, even with grids;
    //  - Sequences cannot be combined with layers or grids, and the first frame of the sequence
    //    (referred to by the primary image item) is always an Intra Frame.
    let mut use_avio_brand;
    if is_sequence {
        // According to section 5.3 of AV1 Image File Format specification v1.1.0:
        //   Additionally, if a file contains AV1 image sequences and the brand avio is used in the
        //   compatible_brands field of the FileTypeBox, the item constraints for this brand shall be met
        //   and at least one of the AV1 image sequences shall be made only of AV1 Samples marked as sync.
        // See https://aomediacodec.github.io/av1-avif/v1.1.0.html#image-sequence-brand.
        use_avio_brand = false;
        for item in &encoder.data.items {
            if item.encode_output.samples.is_empty() {
                continue; // Not a track.
            }
            let mut only_sync_samples = true;
            for sample in &item.encode_output.samples {
                if !sample.sync {
                    only_sync_samples = false;
                    break;
                }
            }
            if only_sync_samples {
                use_avio_brand = true; // at least one of the AV1 image sequences is made only of sync samples
                break;
            }
        }
    } else {
        // The gpac/ComplianceWarden tool only warns about the lack of the "avio" brand for sequences,
        // and the specification says the brand "should" be used, not "shall". Leverage that opportunity
        // to save four bytes for still images.
        use_avio_brand = false; // Should be (encoder->extraLayerCount == 0) to be fully compliant.
    }

    let image_metadata = &*encoder.data.image_metadata;
    let mut ftyp: AvifBoxMarker = 0;
    avif_checkres!(s.write_box(b"ftyp", AVIF_BOX_SIZE_TBD, Some(&mut ftyp)));
    avif_checkres!(s.write_chars(major_brand)); // unsigned int(32) major_brand;
    avif_checkres!(s.write_u32(minor_version)); // unsigned int(32) minor_version;
    avif_checkres!(s.write_chars(b"avif")); // unsigned int(32) compatible_brands[];
    if use_avio_brand {
        //
        avif_checkres!(s.write_chars(b"avio")); // ... compatible_brands[]
    } //
    if is_sequence {
        //
        avif_checkres!(s.write_chars(b"avis")); // ... compatible_brands[]
        avif_checkres!(s.write_chars(b"msf1")); // ... compatible_brands[]
        avif_checkres!(s.write_chars(b"iso8")); // ... compatible_brands[]
    } //
    avif_checkres!(s.write_chars(b"mif1")); // ... compatible_brands[]
    avif_checkres!(s.write_chars(b"miaf")); // ... compatible_brands[]
    if (image_metadata.depth == 8) || (image_metadata.depth == 10) {
        //
        if image_metadata.yuv_format == AvifPixelFormat::Yuv420 {
            //
            avif_checkres!(s.write_chars(b"MA1B")); // ... compatible_brands[]
        } else if image_metadata.yuv_format == AvifPixelFormat::Yuv444 {
            //
            avif_checkres!(s.write_chars(b"MA1A")); // ... compatible_brands[]
        }
    }
    s.finish_box(ftyp);

    // -----------------------------------------------------------------------
    // Start meta

    let mut meta: AvifBoxMarker = 0;
    avif_checkres!(s.write_full_box(b"meta", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut meta)));

    // -----------------------------------------------------------------------
    // Write hdlr

    let mut hdlr: AvifBoxMarker = 0;
    avif_checkres!(s.write_full_box(b"hdlr", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut hdlr)));
    avif_checkres!(s.write_u32(0)); // unsigned int(32) pre_defined = 0;
    avif_checkres!(s.write_chars(b"pict")); // unsigned int(32) handler_type;
    avif_checkres!(s.write_zeros(12)); // const unsigned int(32)[3] reserved = 0;
    avif_checkres!(s.write_chars(b"libavif\0")); // string name; (writing null terminator)
    s.finish_box(hdlr);

    // -----------------------------------------------------------------------
    // Write pitm

    if encoder.data.primary_item_id != 0 {
        avif_checkres!(s.write_full_box(
            b"pitm",
            std::mem::size_of::<u16>(),
            0,
            0,
            /*marker=*/ None
        ));
        avif_checkres!(s.write_u16(encoder.data.primary_item_id)); //  unsigned int(16) item_ID;
    }

    // -----------------------------------------------------------------------
    // Write iloc

    let mut iloc: AvifBoxMarker = 0;
    avif_checkres!(s.write_full_box(b"iloc", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut iloc)));
    avif_checkres!(s.write_bits(4, /*bitCount=*/ 4)); // unsigned int(4) offset_size;
    avif_checkres!(s.write_bits(4, /*bitCount=*/ 4)); // unsigned int(4) length_size;
    avif_checkres!(s.write_bits(0, /*bitCount=*/ 4)); // unsigned int(4) base_offset_size;
    avif_checkres!(s.write_bits(0, /*bitCount=*/ 4)); // unsigned int(4) reserved;
    avif_checkres!(s.write_u16(encoder.data.items.len() as u16)); // unsigned int(16) item_count;

    for item in encoder.data.items.iter_mut() {
        avif_checkres!(s.write_u16(item.id)); // unsigned int(16) item_ID;
        avif_checkres!(s.write_u16(0)); // unsigned int(16) data_reference_index;

        // Layered Image, write location for all samples
        if item.extra_layer_count > 0 {
            let layer_count = item.extra_layer_count + 1;
            avif_checkres!(s.write_u16(layer_count as u16)); // unsigned int(16) extent_count;
            for i in 0..layer_count as usize {
                avif_checkres!(avif_encoder_item_add_mdat_fixup(item, s));
                avif_checkres!(s.write_u32(0 /* set later */)); // unsigned int(offset_size*8) extent_offset;
                avif_checkres!(s.write_u32(item.encode_output.samples[i].data.len() as u32));
                // unsigned int(length_size*8) extent_length;
            }
            continue;
        }

        let mut content_size = item.metadata_payload.len() as u32;
        if !item.encode_output.samples.is_empty() {
            // This is choosing sample 0's size as there are two cases here:
            // * This is a single image, in which case this is correct
            // * This is an image sequence, but this file should still be a valid single-image avif,
            //   so there must still be a primary item pointing at a sync sample. Since the first
            //   frame of the image sequence is guaranteed to be a sync sample, it is chosen here.
            //
            // TODO: Offer the ability for a user to specify which frame in the sequence should
            //       become the primary item's image, and force that frame to be a keyframe.
            content_size = item.encode_output.samples[0].data.len() as u32;
        }

        avif_checkres!(s.write_u16(1)); // unsigned int(16) extent_count;
        avif_checkres!(avif_encoder_item_add_mdat_fixup(item, s)); //
        avif_checkres!(s.write_u32(0 /* set later */)); // unsigned int(offset_size*8) extent_offset;
        avif_checkres!(s.write_u32(content_size)); // unsigned int(length_size*8) extent_length;
    }

    s.finish_box(iloc);

    // -----------------------------------------------------------------------
    // Write iinf

    // Section 8.11.6.2 of ISO/IEC 14496-12.
    let mut iinf: AvifBoxMarker = 0;
    avif_checkres!(s.write_full_box(b"iinf", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut iinf)));
    avif_checkres!(s.write_u16(encoder.data.items.len() as u16)); //  unsigned int(16) entry_count;

    for item in &encoder.data.items {
        let flags: u32 = if item.hidden_image { 1 } else { 0 };
        let mut infe: AvifBoxMarker = 0;
        avif_checkres!(s.write_full_box(b"infe", AVIF_BOX_SIZE_TBD, 2, flags, Some(&mut infe)));
        avif_checkres!(s.write_u16(item.id)); // unsigned int(16) item_ID;
        avif_checkres!(s.write_u16(0)); // unsigned int(16) item_protection_index;
        avif_checkres!(s.write(&item.type_)); // unsigned int(32) item_type;
        avif_checkres!(s.write_chars(item.infe_name)); // utf8string item_name; (writing null terminator)
        if &item.type_ == b"mime" {
            avif_checkres!(s.write_chars(item.infe_content_type)); // utf8string content_type; (writing null terminator)
                                                                   // utf8string content_encoding; //optional
        } else if &item.type_ == b"uri " {
            // utf8string item_uri_type;
            return AvifResult::NotImplemented;
        }
        s.finish_box(infe);
    }

    s.finish_box(iinf);

    // -----------------------------------------------------------------------
    // Write iref boxes

    let mut iref: AvifBoxMarker = 0;
    for item in &encoder.data.items {
        // Count how many other items refer to this item with dimgFromID
        let mut dimg_count: u16 = 0;
        for dimg_item in &encoder.data.items {
            if dimg_item.dimg_from_id == item.id {
                dimg_count += 1;
            }
        }

        if dimg_count > 0 {
            if iref == 0 {
                avif_checkres!(s.write_full_box(b"iref", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut iref)));
            }
            let mut ref_type: AvifBoxMarker = 0;
            avif_checkres!(s.write_box(b"dimg", AVIF_BOX_SIZE_TBD, Some(&mut ref_type)));
            avif_checkres!(s.write_u16(item.id)); // unsigned int(16) from_item_ID;
            avif_checkres!(s.write_u16(dimg_count)); // unsigned int(16) reference_count;
            for dimg_item in &encoder.data.items {
                if dimg_item.dimg_from_id == item.id {
                    avif_checkres!(s.write_u16(dimg_item.id)); // unsigned int(16) to_item_ID;
                }
            }
            s.finish_box(ref_type);
        }

        if item.iref_to_id != 0 {
            if iref == 0 {
                avif_checkres!(s.write_full_box(b"iref", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut iref)));
            }
            let mut ref_type: AvifBoxMarker = 0;
            avif_checkres!(s.write_box(item.iref_type, AVIF_BOX_SIZE_TBD, Some(&mut ref_type)));
            avif_checkres!(s.write_u16(item.id)); // unsigned int(16) from_item_ID;
            avif_checkres!(s.write_u16(1)); // unsigned int(16) reference_count;
            avif_checkres!(s.write_u16(item.iref_to_id)); // unsigned int(16) to_item_ID;
            s.finish_box(ref_type);
        }
    }
    if iref != 0 {
        s.finish_box(iref);
    }

    // -----------------------------------------------------------------------
    // Write iprp -> ipco/ipma

    let mut iprp: AvifBoxMarker = 0;
    avif_checkres!(s.write_box(b"iprp", AVIF_BOX_SIZE_TBD, Some(&mut iprp)));

    let dedup = avif_item_property_dedup_create();
    avif_checkerr!(dedup.is_some(), AvifResult::OutOfMemory);
    let Some(mut dedup) = dedup else {
        return AvifResult::OutOfMemory;
    };
    let mut ipco: AvifBoxMarker = 0;
    avif_checkres!(s.write_box(b"ipco", AVIF_BOX_SIZE_TBD, Some(&mut ipco)));
    // (altImageMetadata: NULL without gain maps)
    let result = avif_rw_stream_write_properties(&mut dedup, s, &mut encoder.data);
    drop(dedup);
    avif_checkres!(result);
    s.finish_box(ipco);

    let mut ipma: AvifBoxMarker = 0;
    avif_checkres!(s.write_full_box(b"ipma", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut ipma)));
    {
        let mut ipma_count: i32 = 0;
        for item in &encoder.data.items {
            if item.ipma.count > 0 {
                ipma_count += 1;
            }
        }
        avif_checkres!(s.write_u32(ipma_count as u32)); // unsigned int(32) entry_count;

        for item in &encoder.data.items {
            if item.ipma.count == 0 {
                continue;
            }

            avif_checkres!(s.write_u16(item.id)); // unsigned int(16) item_ID;
            avif_checkres!(s.write_u8(item.ipma.count)); // unsigned int(8) association_count;
            for i in 0..item.ipma.count as usize {
                avif_checkres!(s.write_bits(
                    if item.ipma.essential[i] { 1 } else { 0 },
                    /*bitCount=*/ 1
                )); // bit(1) essential;
                avif_checkres!(s.write_bits(item.ipma.associations[i] as u32, /*bitCount=*/ 7));
                // unsigned int(7) property_index;
            }
        }
    }
    s.finish_box(ipma);

    s.finish_box(iprp);

    // -----------------------------------------------------------------------
    // Write grpl/altr box

    if !encoder.data.alternative_item_ids.is_empty() {
        avif_checkres!(avif_write_altr_group(s, &encoder.data.alternative_item_ids));
    }

    // -----------------------------------------------------------------------
    // Finish meta box

    s.finish_box(meta);

    // -----------------------------------------------------------------------
    // Write tracks (if an image sequence)

    if is_sequence {
        #[rustfmt::skip]
        static UNITY_MATRIX: [[u8; 4]; 9] = [
            [0x00, 0x01, 0x00, 0x00],
            [0; 4],
            [0; 4],
            [0; 4],
            [0x00, 0x01, 0x00, 0x00],
            [0; 4],
            [0; 4],
            [0; 4],
            [0x40, 0x00, 0x00, 0x00],
        ];
        let unity_matrix = UNITY_MATRIX.as_flattened();

        if encoder.repetition_count < 0
            && encoder.repetition_count != AVIF_REPETITION_COUNT_INFINITE
        {
            return AvifResult::InvalidArgument;
        }

        let mut frames_duration_in_timescales: u64 = 0;
        for frame in &encoder.data.frames {
            frames_duration_in_timescales =
                frames_duration_in_timescales.wrapping_add(frame.duration_in_timescales);
        }
        let duration_in_timescales: u64;
        if encoder.repetition_count == AVIF_REPETITION_COUNT_INFINITE {
            duration_in_timescales = AVIF_INDEFINITE_DURATION64;
        } else {
            let loop_count = encoder.repetition_count as u64 + 1;
            avif_assert_or_return!(frames_duration_in_timescales != 0);
            if loop_count > u64::MAX / frames_duration_in_timescales {
                // The multiplication will overflow uint64_t.
                return AvifResult::InvalidArgument;
            }
            duration_in_timescales = frames_duration_in_timescales * loop_count;
        }

        // -------------------------------------------------------------------
        // Start moov

        let mut moov: AvifBoxMarker = 0;
        avif_checkres!(s.write_box(b"moov", AVIF_BOX_SIZE_TBD, Some(&mut moov)));

        let mut mvhd: AvifBoxMarker = 0;
        avif_checkres!(s.write_full_box(b"mvhd", AVIF_BOX_SIZE_TBD, 1, 0, Some(&mut mvhd)));
        avif_checkres!(s.write_u64(now)); // unsigned int(64) creation_time;
        avif_checkres!(s.write_u64(now)); // unsigned int(64) modification_time;
        avif_checkres!(s.write_u32(encoder.timescale as u32)); // unsigned int(32) timescale;
        avif_checkres!(s.write_u64(duration_in_timescales)); // unsigned int(64) duration;
        avif_checkres!(s.write_u32(0x00010000)); // template int(32) rate = 0x00010000; // typically 1.0
        avif_checkres!(s.write_u16(0x0100)); // template int(16) volume = 0x0100; // typically, full volume
        avif_checkres!(s.write_u16(0)); // const bit(16) reserved = 0;
        avif_checkres!(s.write_zeros(8)); // const unsigned int(32)[2] reserved = 0;
        avif_checkres!(s.write(unity_matrix));
        avif_checkres!(s.write_zeros(24)); // bit(32)[6] pre_defined = 0;
        avif_checkres!(s.write_u32(encoder.data.items.len() as u32)); // unsigned int(32) next_track_ID;
        s.finish_box(mvhd);

        // -------------------------------------------------------------------
        // Write tracks

        for item_index in 0..encoder.data.items.len() {
            if encoder.data.items[item_index]
                .encode_output
                .samples
                .is_empty()
            {
                continue;
            }

            let mut sync_samples_count: u32 = 0;
            for sample in &encoder.data.items[item_index].encode_output.samples {
                if sample.sync {
                    sync_samples_count += 1;
                }
            }

            let mut trak: AvifBoxMarker = 0;
            avif_checkres!(s.write_box(b"trak", AVIF_BOX_SIZE_TBD, Some(&mut trak)));

            let image_metadata = &*encoder.data.image_metadata;
            let mut tkhd: AvifBoxMarker = 0;
            avif_checkres!(s.write_full_box(b"tkhd", AVIF_BOX_SIZE_TBD, 1, 1, Some(&mut tkhd)));
            avif_checkres!(s.write_u64(now)); // unsigned int(64) creation_time;
            avif_checkres!(s.write_u64(now)); // unsigned int(64) modification_time;
            avif_checkres!(s.write_u32(item_index as u32 + 1)); // unsigned int(32) track_ID;
            avif_checkres!(s.write_u32(0)); // const unsigned int(32) reserved = 0;
            avif_checkres!(s.write_u64(duration_in_timescales)); // unsigned int(64) duration;
            avif_checkres!(s.write_zeros(std::mem::size_of::<u32>() * 2)); // const unsigned int(32)[2] reserved = 0;
            avif_checkres!(s.write_u16(0)); // template int(16) layer = 0;
            avif_checkres!(s.write_u16(0)); // template int(16) alternate_group = 0;
            avif_checkres!(s.write_u16(0)); // template int(16) volume = {if track_is_audio 0x0100 else 0};
            avif_checkres!(s.write_u16(0)); // const unsigned int(16) reserved = 0;
            avif_checkres!(s.write(unity_matrix)); // template int(32)[9] matrix= // { 0x00010000,0,0,0,0x00010000,0,0,0,0x40000000 };
            avif_checkres!(s.write_u32(image_metadata.width << 16)); // unsigned int(32) width;
            avif_checkres!(s.write_u32(image_metadata.height << 16)); // unsigned int(32) height;
            s.finish_box(tkhd);

            let item = &encoder.data.items[item_index];
            if item.iref_to_id != 0 {
                let mut tref: AvifBoxMarker = 0;
                avif_checkres!(s.write_box(b"tref", AVIF_BOX_SIZE_TBD, Some(&mut tref)));
                let mut ref_type: AvifBoxMarker = 0;
                avif_checkres!(s.write_box(item.iref_type, AVIF_BOX_SIZE_TBD, Some(&mut ref_type)));
                avif_checkres!(s.write_u32(item.iref_to_id as u32));
                s.finish_box(ref_type);
                s.finish_box(tref);
            }

            let mut edts: AvifBoxMarker = 0;
            avif_checkres!(s.write_box(b"edts", AVIF_BOX_SIZE_TBD, Some(&mut edts)));
            let elst_flags: u32 = (encoder.repetition_count != 0) as u32;
            let mut elst: AvifBoxMarker = 0;
            avif_checkres!(s.write_full_box(
                b"elst",
                AVIF_BOX_SIZE_TBD,
                1,
                elst_flags,
                Some(&mut elst)
            ));
            avif_checkres!(s.write_u32(1)); // unsigned int(32) entry_count;
            avif_checkres!(s.write_u64(frames_duration_in_timescales)); // unsigned int(64) segment_duration;
            avif_checkres!(s.write_u64(0)); // int(64) media_time;
            avif_checkres!(s.write_u16(1)); // int(16) media_rate_integer;
            avif_checkres!(s.write_u16(0)); // int(16) media_rate_fraction = 0;
            s.finish_box(elst);
            s.finish_box(edts);

            let item_category = item.item_category;
            if item_category != AvifItemCategory::Alpha {
                avif_checkres!(avif_encoder_write_track_meta_box(&mut encoder.data, s));
            }
            let image_metadata = &*encoder.data.image_metadata;
            let item = &encoder.data.items[item_index];

            let mut mdia: AvifBoxMarker = 0;
            avif_checkres!(s.write_box(b"mdia", AVIF_BOX_SIZE_TBD, Some(&mut mdia)));

            let mut mdhd: AvifBoxMarker = 0;
            avif_checkres!(s.write_full_box(b"mdhd", AVIF_BOX_SIZE_TBD, 1, 0, Some(&mut mdhd)));
            avif_checkres!(s.write_u64(now)); // unsigned int(64) creation_time;
            avif_checkres!(s.write_u64(now)); // unsigned int(64) modification_time;
            avif_checkres!(s.write_u32(encoder.timescale as u32)); // unsigned int(32) timescale;
            avif_checkres!(s.write_u64(frames_duration_in_timescales)); // unsigned int(64) duration;
            avif_checkres!(s.write_u16(21956)); // bit(1) pad = 0; unsigned int(5)[3] language; ("und")
            avif_checkres!(s.write_u16(0)); // unsigned int(16) pre_defined = 0;
            s.finish_box(mdhd);

            let mut hdlr_trak: AvifBoxMarker = 0;
            avif_checkres!(s.write_full_box(
                b"hdlr",
                AVIF_BOX_SIZE_TBD,
                0,
                0,
                Some(&mut hdlr_trak)
            ));
            avif_checkres!(s.write_u32(0)); // unsigned int(32) pre_defined = 0;
            avif_checkres!(
                s.write_chars(if item.item_category == AvifItemCategory::Alpha {
                    b"auxv"
                } else {
                    b"pict"
                })
            ); // unsigned int(32) handler_type;
            avif_checkres!(s.write_zeros(12)); // const unsigned int(32)[3] reserved = 0;
            avif_checkres!(s.write_chars(b"libavif\0")); // string name; (writing null terminator)
            s.finish_box(hdlr_trak);

            let mut minf: AvifBoxMarker = 0;
            avif_checkres!(s.write_box(b"minf", AVIF_BOX_SIZE_TBD, Some(&mut minf)));

            let mut vmhd: AvifBoxMarker = 0;
            avif_checkres!(s.write_full_box(b"vmhd", AVIF_BOX_SIZE_TBD, 0, 1, Some(&mut vmhd)));
            avif_checkres!(s.write_u16(0)); // template unsigned int(16) graphicsmode = 0; (copy over the existing image)
            avif_checkres!(s.write_zeros(6)); // template unsigned int(16)[3] opcolor = {0, 0, 0};
            s.finish_box(vmhd);

            let mut dinf: AvifBoxMarker = 0;
            avif_checkres!(s.write_box(b"dinf", AVIF_BOX_SIZE_TBD, Some(&mut dinf)));
            let mut dref: AvifBoxMarker = 0;
            avif_checkres!(s.write_full_box(b"dref", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut dref)));
            avif_checkres!(s.write_u32(1)); // unsigned int(32) entry_count;
            avif_checkres!(s.write_full_box(b"url ", /*contentSize=*/ 0, 0, 1, /*marker=*/ None)); // flags:1 means data is in this file
            s.finish_box(dref);
            s.finish_box(dinf);

            // The boxes within the "stbl" box are ordered using the following recommendation in ISO/IEC 14496-12, Section 6.2.3:
            // 4) It is recommended that the boxes within the Sample Table Box be in the following order: Sample Description
            // (stsd), Time to Sample (stts), Sample to Chunk (stsc), Sample Size (stsz), Chunk Offset (stco).
            //
            // Any boxes not listed in the above line are placed in the end (after the "stco" box).
            let mut stbl: AvifBoxMarker = 0;
            avif_checkres!(s.write_box(b"stbl", AVIF_BOX_SIZE_TBD, Some(&mut stbl)));

            let mut stsd: AvifBoxMarker = 0;
            avif_checkres!(s.write_full_box(b"stsd", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut stsd)));
            avif_checkres!(s.write_u32(1)); // unsigned int(32) entry_count;
            let mut image_item: AvifBoxMarker = 0;
            avif_checkres!(s.write_box(
                encoder.data.image_item_type,
                AVIF_BOX_SIZE_TBD,
                Some(&mut image_item)
            ));
            avif_checkres!(s.write_zeros(6)); // const unsigned int(8)[6] reserved = 0;
            avif_checkres!(s.write_u16(1)); // unsigned int(16) data_reference_index;
            avif_checkres!(s.write_u16(0)); // unsigned int(16) pre_defined = 0;
            avif_checkres!(s.write_u16(0)); // const unsigned int(16) reserved = 0;
            avif_checkres!(s.write_zeros(std::mem::size_of::<u32>() * 3)); // unsigned int(32)[3] pre_defined = 0;
            avif_checkres!(s.write_u16(image_metadata.width as u16)); // unsigned int(16) width;
            avif_checkres!(s.write_u16(image_metadata.height as u16)); // unsigned int(16) height;
            avif_checkres!(s.write_u32(0x00480000)); // template unsigned int(32) horizresolution
            avif_checkres!(s.write_u32(0x00480000)); // template unsigned int(32) vertresolution
            avif_checkres!(s.write_u32(0)); // const unsigned int(32) reserved = 0;
            avif_checkres!(s.write_u16(1)); // template unsigned int(16) frame_count = 1;
            avif_checkres!(s.write_chars(b"\x0aAOM Coding")); // string[32] compressorname;
            avif_checkres!(s.write_zeros(32 - 11)); //
            avif_checkres!(s.write_u16(0x0018)); // template unsigned int(16) depth = 0x0018;
            avif_checkres!(s.write_u16(0xffff)); // int(16) pre_defined = -1;
            avif_checkres!(write_config_box(
                s,
                &item.av1c,
                encoder.data.config_prop_name
            ));
            if item.item_category == AvifItemCategory::Color {
                avif_checkres!(avif_encoder_write_color_properties(
                    s,
                    image_metadata,
                    None,
                    None
                ));
                avif_checkres!(avif_encoder_write_hdr_properties(
                    s,
                    image_metadata,
                    None,
                    None
                ));
            }

            let mut ccst: AvifBoxMarker = 0;
            avif_checkres!(s.write_full_box(b"ccst", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut ccst)));
            avif_checkres!(s.write_bits(0, /*bitCount=*/ 1)); // unsigned int(1) all_ref_pics_intra;
            avif_checkres!(s.write_bits(1, /*bitCount=*/ 1)); // unsigned int(1) intra_pred_used;
            avif_checkres!(s.write_bits(15, /*bitCount=*/ 4)); // unsigned int(4) max_ref_per_pic;
            avif_checkres!(s.write_bits(0, /*bitCount=*/ 26)); // unsigned int(26) reserved;
            s.finish_box(ccst);

            if item.item_category == AvifItemCategory::Alpha {
                let mut auxi: AvifBoxMarker = 0;
                avif_checkres!(s.write_full_box(b"auxi", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut auxi)));
                avif_checkres!(s.write_chars(ALPHA_URN)); //  string aux_track_type;
                s.finish_box(auxi);
            }

            s.finish_box(image_item);
            s.finish_box(stsd);

            let mut stts: AvifBoxMarker = 0;
            avif_checkres!(s.write_full_box(b"stts", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut stts)));
            let stts_entry_count_offset = s.offset();
            let mut stts_entry_count: u32 = 0;
            avif_checkres!(s.write_u32(0)); // unsigned int(32) entry_count;
            let mut sample_count: u32 = 0;
            for frame_index in 0..encoder.data.frames.len() {
                let frame = &encoder.data.frames[frame_index];
                sample_count += 1;
                if frame_index < (encoder.data.frames.len() - 1) {
                    let next_frame = &encoder.data.frames[frame_index + 1];
                    if frame.duration_in_timescales == next_frame.duration_in_timescales {
                        continue;
                    }
                }
                avif_checkres!(s.write_u32(sample_count)); // unsigned int(32) sample_count;
                avif_checkres!(s.write_u32(frame.duration_in_timescales as u32)); // unsigned int(32) sample_delta;
                sample_count = 0;
                stts_entry_count += 1;
            }
            let prev_offset = s.offset();
            s.set_offset(stts_entry_count_offset);
            avif_checkres!(s.write_u32(stts_entry_count));
            s.set_offset(prev_offset);
            s.finish_box(stts);

            let mut stsc: AvifBoxMarker = 0;
            avif_checkres!(s.write_full_box(b"stsc", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut stsc)));
            avif_checkres!(s.write_u32(1)); // unsigned int(32) entry_count;
            avif_checkres!(s.write_u32(1)); // unsigned int(32) first_chunk;
            avif_checkres!(s.write_u32(item.encode_output.samples.len() as u32)); // unsigned int(32) samples_per_chunk;
            avif_checkres!(s.write_u32(1)); // unsigned int(32) sample_description_index;
            s.finish_box(stsc);

            let mut stsz: AvifBoxMarker = 0;
            avif_checkres!(s.write_full_box(b"stsz", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut stsz)));
            avif_checkres!(s.write_u32(0)); // unsigned int(32) sample_size;
            avif_checkres!(s.write_u32(item.encode_output.samples.len() as u32)); // unsigned int(32) sample_count;
            for sample in &item.encode_output.samples {
                avif_checkres!(s.write_u32(sample.data.len() as u32)); // unsigned int(32) entry_size;
            }
            s.finish_box(stsz);

            let mut stco: AvifBoxMarker = 0;
            avif_checkres!(s.write_full_box(b"stco", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut stco)));
            avif_checkres!(s.write_u32(1)); // unsigned int(32) entry_count;
            avif_checkres!(avif_encoder_item_add_mdat_fixup(
                &mut encoder.data.items[item_index],
                s
            )); //
            avif_checkres!(s.write_u32(1)); // unsigned int(32) chunk_offset; (set later)
            s.finish_box(stco);

            let item = &encoder.data.items[item_index];
            let mut has_non_sync_sample = false;
            for sample in &item.encode_output.samples {
                if !sample.sync {
                    has_non_sync_sample = true;
                    break;
                }
            }
            // ISO/IEC 14496-12, Section 8.6.2.1:
            //   If the SyncSampleBox is not present, every sample is a sync sample.
            if has_non_sync_sample {
                let mut stss: AvifBoxMarker = 0;
                avif_checkres!(s.write_full_box(b"stss", AVIF_BOX_SIZE_TBD, 0, 0, Some(&mut stss)));
                avif_checkres!(s.write_u32(sync_samples_count)); // unsigned int(32) entry_count;
                for (sample_index, sample) in item.encode_output.samples.iter().enumerate() {
                    if sample.sync {
                        avif_checkres!(s.write_u32(sample_index as u32 + 1)); // unsigned int(32) sample_number;
                    }
                }
                s.finish_box(stss);
            }

            s.finish_box(stbl);

            s.finish_box(minf);
            s.finish_box(mdia);
            s.finish_box(trak);
        }

        // -------------------------------------------------------------------
        // Finish moov box

        s.finish_box(moov);
    }

    // -----------------------------------------------------------------------
    // Write mdat

    let mut layered_color_items: Vec<usize> = Vec::new();
    let mut layered_alpha_items: Vec<usize> = Vec::new();
    let result = avif_encoder_write_media_data_box(
        encoder,
        s,
        &mut layered_color_items,
        &mut layered_alpha_items,
    );
    avif_checkres!(result);

    // -----------------------------------------------------------------------
    // Finish up stream

    s.finish_write();

    AvifResult::Ok
}

/// Translation of `avifEncoderWrite()`.
pub(crate) fn avif_encoder_write(
    encoder: &mut AvifEncoder,
    image: &AvifImage,
    output: &mut Vec<u8>,
) -> AvifResult {
    let add_image_result = avif_encoder_add_image(encoder, image, 1, AVIF_ADD_IMAGE_FLAG_SINGLE);
    if add_image_result != AvifResult::Ok {
        return add_image_result;
    }
    avif_encoder_finish(encoder, output)
}

// Implementation of section 2.3.3 of AV1 Codec ISO Media File Format Binding specification v1.2.0.
// See https://aomediacodec.github.io/av1-isobmff/v1.2.0.html#av1codecconfigurationbox-syntax.
/// Translation of `writeCodecConfig()`.
fn write_codec_config(s: &mut AvifRWStream, cfg: &AvifCodecConfigurationBox) -> AvifResult {
    avif_checkres!(s.write_bits(1, /*bitCount=*/ 1)); // unsigned int (1) marker = 1;
    avif_checkres!(s.write_bits(1, /*bitCount=*/ 7)); // unsigned int (7) version = 1;

    avif_checkres!(s.write_bits(cfg.seq_profile as u32, /*bitCount=*/ 3)); // unsigned int (3) seq_profile;
    avif_checkres!(s.write_bits(cfg.seq_level_idx0 as u32, /*bitCount=*/ 5)); // unsigned int (5) seq_level_idx_0;

    avif_checkres!(s.write_bits(cfg.seq_tier0 as u32, /*bitCount=*/ 1)); // unsigned int (1) seq_tier_0;
    avif_checkres!(s.write_bits(cfg.high_bitdepth as u32, /*bitCount=*/ 1)); // unsigned int (1) high_bitdepth;
    avif_checkres!(s.write_bits(cfg.twelve_bit as u32, /*bitCount=*/ 1)); // unsigned int (1) twelve_bit;
    avif_checkres!(s.write_bits(cfg.monochrome as u32, /*bitCount=*/ 1)); // unsigned int (1) monochrome;
    avif_checkres!(s.write_bits(cfg.chroma_subsampling_x as u32, /*bitCount=*/ 1)); // unsigned int (1) chroma_subsampling_x;
    avif_checkres!(s.write_bits(cfg.chroma_subsampling_y as u32, /*bitCount=*/ 1)); // unsigned int (1) chroma_subsampling_y;
    avif_checkres!(s.write_bits(cfg.chroma_sample_position as u32, /*bitCount=*/ 2)); // unsigned int (2) chroma_sample_position;

    avif_checkres!(s.write_bits(0, /*bitCount=*/ 3)); // unsigned int (3) reserved = 0;
    avif_checkres!(s.write_bits(0, /*bitCount=*/ 1)); // unsigned int (1) initial_presentation_delay_present;
    avif_checkres!(s.write_bits(0, /*bitCount=*/ 4)); // unsigned int (4) reserved = 0;

    // According to section 2.2.1 of AV1 Image File Format specification v1.1.0,
    // there is no need to write any OBU here.
    // See https://aomediacodec.github.io/av1-avif/v1.1.0.html#av1-configuration-item-property.
    // unsigned int (8) configOBUs[];
    AvifResult::Ok
}

/// Translation of `writeConfigBox()`.
fn write_config_box(
    s: &mut AvifRWStream,
    cfg: &AvifCodecConfigurationBox,
    config_prop_name: &[u8],
) -> AvifResult {
    let mut config_box: AvifBoxMarker = 0;
    avif_checkres!(s.write_box(config_prop_name, AVIF_BOX_SIZE_TBD, Some(&mut config_box)));
    avif_checkres!(write_codec_config(s, cfg));
    s.finish_box(config_box);
    AvifResult::Ok
}
