// Rust translation of src/read.c from libavif
// (https://github.com/AOMediaCodec/libavif, at the revision SDL_image's
// external/libavif pins: libavif 1.1.1 with SDL's patches).
// Copyright 2019 Joe Drago. All rights reserved.
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The decoder: the ISOBMFF/HEIF container parser (the file type, meta
//! and movie boxes; items, their locations, properties and references;
//! tracks and their sample tables), the tiles (grid cells) and samples to
//! decode, and the `avifDecoder*()` API (parse, next/nth image, timing,
//! keyframes, extents).
//!
//! As SDL_image builds libavif, the experimental features (gain maps,
//! sample transforms, the reduced 'mini' header) are not compiled in. The
//! items of a meta box are a vector (an item is its index; the
//! `avifDecoderItem.meta` back pointers are the meta box the functions are
//! passed), the property union of an `avifProperty` is a struct with one
//! field per property type, and the reader (`decoder->io`) is passed to
//! the calls that read (see [`AvifIo`]).

use super::avif::*;
use super::codec_dav1d::AvifCodec;
use super::diag::{self, fourcc, AvifDiagnostics};
use super::exif::avif_get_exif_tiff_header_offset;
use super::internal::{
    avif_assert_or_return, avif_check, avif_checkerr, avif_checkres, avif_is_alpha, AvifBoxHeader,
    AvifCodecConfigurationBox, AvifCodecDecodeInput, AvifCodecType, AvifDecodeSample,
    AvifItemCategory, AvifSequenceHeader, SampleData, AVIF_CONTENT_TYPE_XMP,
    AVIF_INDEFINITE_DURATION32, AVIF_INDEFINITE_DURATION64, AVIF_ITEM_CATEGORY_COUNT,
    AVIF_SPATIAL_ID_UNSET, AVIF_URN_ALPHA0, AVIF_URN_ALPHA1,
};
use super::io::AvifIo;
use super::obu::avif_sequence_header_parse;
use super::rawdata::avif_rw_data_set;
use super::reformat::avif_limited_to_full_y;
use super::scale::avif_image_scale_with_limit;
use super::stream::AvifROStream;
use super::utils::try_push;

/// `avifDiagnosticsPrintf()`
macro_rules! diag_printf {
    ($diag:expr, $($arg:tt)*) => {
        diag::printf($diag, format_args!($($arg)*))
    };
}

const AUXTYPE_SIZE: usize = 64;
const CONTENTTYPE_SIZE: usize = 64;

// class VisualSampleEntry(codingname) extends SampleEntry(codingname) {
//     unsigned int(16) pre_defined = 0;
//     const unsigned int(16) reserved = 0;
//     unsigned int(32)[3] pre_defined = 0;
//     unsigned int(16) width;
//     unsigned int(16) height;
//     template unsigned int(32) horizresolution = 0x00480000; // 72 dpi
//     template unsigned int(32) vertresolution = 0x00480000;  // 72 dpi
//     const unsigned int(32) reserved = 0;
//     template unsigned int(16) frame_count = 1;
//     string[32] compressorname;
//     template unsigned int(16) depth = 0x0018;
//     int(16) pre_defined = -1;
//     // other boxes from derived specifications
//     CleanApertureBox clap;    // optional
//     PixelAspectRatioBox pasp; // optional
// }
const VISUALSAMPLEENTRY_SIZE: usize = 78;

// The only supported ipma box values for both version and flags are [0,1], so there technically
// can't be more than 4 unique tuples right now.
const MAX_IPMA_VERSION_AND_FLAGS_SEEN: usize = 4;

// ---------------------------------------------------------------------------
// AVIF codec type (AV1 or AV2)

/// Translation of `avifGetCodecType()`.
fn avif_get_codec_type(fourcc: &[u8; 4]) -> AvifCodecType {
    if fourcc == b"av01" {
        return AvifCodecType::Av1;
    }
    AvifCodecType::Unknown
}

/// Translation of `avifGetConfigurationPropertyName()` (upstream asserts
/// for an unknown codec type and returns `NULL`, the empty name here).
fn avif_get_configuration_property_name(codec_type: AvifCodecType) -> &'static [u8; 4] {
    match codec_type {
        AvifCodecType::Av1 => b"av1C",
        _ => b"\0\0\0\0",
    }
}

// ---------------------------------------------------------------------------
// Box data structures

/// ftyp. Translation of `avifFileType`.
#[derive(Clone, Copy, Default)]
struct AvifFileType<'a> {
    major_brand: [u8; 4],
    minor_version: u32,
    /// If not null, points to a memory block of 4 * compatibleBrandsCount bytes.
    compatible_brands: &'a [u8],
    compatible_brands_count: i32,
}

/// ispe. Translation of `avifImageSpatialExtents`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct AvifImageSpatialExtents {
    width: u32,
    height: u32,
}

/// auxC. Translation of `avifAuxiliaryType` (the string without its NUL).
#[derive(Clone, Default, Debug)]
pub(crate) struct AvifAuxiliaryType {
    aux_type: Vec<u8>,
}

/// infe mime content_type. Translation of `avifContentType` (the string
/// without its NUL).
#[derive(Clone, Default, Debug)]
pub(crate) struct AvifContentType {
    content_type: Vec<u8>,
}

/// colr. Translation of `avifColourInformationBox`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct AvifColourInformationBox {
    has_icc: bool,
    icc_offset: u64,
    icc_size: usize,

    has_nclx: bool,
    color_primaries: AvifColorPrimaries,
    transfer_characteristics: AvifTransferCharacteristics,
    matrix_coefficients: AvifMatrixCoefficients,
    range: AvifRange,
}

const MAX_PIXI_PLANE_DEPTHS: usize = 4;
/// Translation of `avifPixelInformationProperty`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct AvifPixelInformationProperty {
    plane_depths: [u8; MAX_PIXI_PLANE_DEPTHS],
    plane_count: u8,
}

/// Translation of `avifOperatingPointSelectorProperty`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct AvifOperatingPointSelectorProperty {
    op_index: u8,
}

/// Translation of `avifLayerSelectorProperty`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct AvifLayerSelectorProperty {
    layer_id: u16,
}

/// Translation of `avifAV1LayeredImageIndexingProperty`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct AvifAV1LayeredImageIndexingProperty {
    layer_size: [u32; 3],
}

// ---------------------------------------------------------------------------
// Top-level structures

/// The property data of an `avifProperty`: upstream's union, with one
/// field per type (only the one of the property's type is meaningful).
#[derive(Clone, Default, Debug)]
pub(crate) struct AvifPropertyU {
    ispe: AvifImageSpatialExtents,
    aux_c: AvifAuxiliaryType,
    colr: AvifColourInformationBox,
    /// TODO(yguyon): Rename or add av2C
    av1c: AvifCodecConfigurationBox,
    pasp: AvifPixelAspectRatioBox,
    clap: AvifCleanApertureBox,
    irot: AvifImageRotation,
    imir: AvifImageMirror,
    pixi: AvifPixelInformationProperty,
    a1op: AvifOperatingPointSelectorProperty,
    lsel: AvifLayerSelectorProperty,
    a1lx: AvifAV1LayeredImageIndexingProperty,
    clli: AvifContentLightLevelInformationBox,
}

/// Temporary storage for ipco/stsd contents until they can be associated
/// and memcpy'd to an avifDecoderItem. Translation of `avifProperty`.
#[derive(Clone, Default, Debug)]
pub(crate) struct AvifProperty {
    type_: [u8; 4],
    u: AvifPropertyU,
}

/// Finds the first property of a given type. Translation of
/// `avifPropertyArrayFind()`.
fn avif_property_array_find<'p>(
    properties: &'p [AvifProperty],
    type_: &[u8; 4],
) -> Option<&'p AvifProperty> {
    properties.iter().find(|prop| &prop.type_ == type_)
}

/// The merged extents of an item (`avifDecoderItem.mergedExtents`):
/// none yet, a buffer the item owns (`ownsMergedExtents`; its length is
/// how much of it was read), or the single extent of an item stored in its
/// meta box's idat (a range of the idat, which upstream points into).
#[derive(Clone, Debug, Default)]
enum MergedExtents {
    #[default]
    None,
    Owned(Vec<u8>),
    Idat {
        offset: usize,
        size: usize,
    },
}

/// one "item" worth for decoding (all iref, iloc, iprp, etc refer to one
/// of these). Translation of `avifDecoderItem` (its `meta` back pointer
/// is the meta box it is in).
#[derive(Clone, Default, Debug)]
pub(crate) struct AvifDecoderItem {
    id: u32,
    type_: [u8; 4],
    size: usize,
    /// If true, offset is relative to the associated meta box's idat box (iloc construction_method==1)
    idat_stored: bool,
    /// Set from this item's ispe property, if present
    width: u32,
    /// Set from this item's ispe property, if present
    height: u32,
    content_type: AvifContentType,
    properties: Vec<AvifProperty>,
    /// All extent offsets/sizes
    extents: Vec<AvifExtent>,
    /// if set, is a single contiguous block of this item's extents (unused when extents.count == 1)
    merged_extents: MergedExtents,
    /// if true, mergedExtents must be freed when this item is destroyed
    owns_merged_extents: bool,
    /// If true, mergedExtents doesn't have all of the item data yet
    partial_merged_extents: bool,
    /// if non-zero, this item is a thumbnail for Item #{thumbnailForID}
    thumbnail_for_id: u32,
    /// if non-zero, this item is an auxC plane for Item #{auxForID}
    aux_for_id: u32,
    /// if non-zero, this item is a content description for Item #{descForID}
    desc_for_id: u32,
    /// if non-zero, this item is an input of derived Item #{dimgForID}
    dimg_for_id: u32,
    /// If dimgForId is non-zero, this is the zero-based index of this item in the list of Item #{dimgForID}'s dimg.
    dimg_idx: u32,
    /// whether there is a 'dimg' box with this item's id as 'fromID'
    has_dimg_from: bool,
    /// if non-zero, this item is premultiplied by Item #{premByID}
    prem_by_id: u32,
    /// If true, this item cites a property flagged as 'essential' that libavif doesn't support (yet). Ignore the item, if so.
    has_unsupported_essential_property: bool,
    /// if true, this item already received a property association
    ipma_seen: bool,
    /// if true, this item has progressive layers (a1lx), but does not select a specific layer (the layer_id value in lsel is set to 0xFFFF)
    progressive: bool,
}

impl AvifDecoderItem {
    /// The bytes of `mergedExtents` (`None` for `NULL`).
    fn merged_extents_data<'m>(&'m self, idat: &'m [u8]) -> Option<&'m [u8]> {
        match &self.merged_extents {
            MergedExtents::None => None,
            MergedExtents::Owned(v) => Some(v),
            MergedExtents::Idat { offset, size } => idat.get(*offset..*offset + *size),
        }
    }
}

/// grid storage. Translation of `avifImageGrid`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct AvifImageGrid {
    /// Legal range: [1-256]
    rows: u32,
    /// Legal range: [1-256]
    columns: u32,
    output_width: u32,
    output_height: u32,
}

// ---------------------------------------------------------------------------
// avifTrack

/// Translation of `avifSampleTableSampleToChunk`.
#[derive(Clone, Copy, Default, Debug)]
struct AvifSampleTableSampleToChunk {
    first_chunk: u32,
    samples_per_chunk: u32,
    sample_description_index: u32,
}

/// Translation of `avifSampleTableTimeToSample`.
#[derive(Clone, Copy, Default, Debug)]
struct AvifSampleTableTimeToSample {
    sample_count: u32,
    sample_delta: u32,
}

/// Translation of `avifSampleDescription`.
#[derive(Clone, Default, Debug)]
struct AvifSampleDescription {
    format: [u8; 4],
    properties: Vec<AvifProperty>,
}

/// Translation of `avifSampleTable` (`avifSampleTableCreate()` is
/// `Default`, `avifSampleTableDestroy()` dropping it). The chunks
/// (`avifSampleTableChunk`), sample sizes (`avifSampleTableSampleSize`)
/// and sync samples (`avifSyncSample`) are vectors of their one field.
#[derive(Clone, Default, Debug)]
struct AvifSampleTable {
    chunks: Vec<u64>,
    sample_descriptions: Vec<AvifSampleDescription>,
    sample_to_chunks: Vec<AvifSampleTableSampleToChunk>,
    sample_sizes: Vec<u32>,
    time_to_samples: Vec<AvifSampleTableTimeToSample>,
    sync_samples: Vec<u32>,
    /// If this is non-zero, sampleSizes will be empty and all samples will be this size
    all_samples_size: u32,
}

/// Translation of `avifSampleTableGetImageDelta()`.
fn avif_sample_table_get_image_delta(sample_table: &AvifSampleTable, image_index: u32) -> u32 {
    let mut max_sample_index: u32 = 0;
    for (i, time_to_sample) in sample_table.time_to_samples.iter().enumerate() {
        max_sample_index = max_sample_index.wrapping_add(time_to_sample.sample_count);
        if (image_index < max_sample_index) || (i == (sample_table.time_to_samples.len() - 1)) {
            return time_to_sample.sample_delta;
        }
    }

    // TODO: fail here?
    1
}

/// Translation of `avifSampleTableGetCodecType()`.
fn avif_sample_table_get_codec_type(sample_table: &AvifSampleTable) -> AvifCodecType {
    for description in &sample_table.sample_descriptions {
        let codec_type = avif_get_codec_type(&description.format);
        if codec_type != AvifCodecType::Unknown {
            return codec_type;
        }
    }
    AvifCodecType::Unknown
}

/// Translation of `avifCodecConfigurationBoxGetDepth()`.
fn avif_codec_configuration_box_get_depth(av1c: &AvifCodecConfigurationBox) -> u32 {
    if av1c.twelve_bit != 0 {
        return 12;
    } else if av1c.high_bitdepth != 0 {
        return 10;
    }
    8
}

/// This is used as a hint to validating the clap box in
/// avifDecoderItemValidateProperties. Translation of
/// `avifCodecConfigurationBoxGetFormat()`.
fn avif_codec_configuration_box_get_format(av1c: &AvifCodecConfigurationBox) -> AvifPixelFormat {
    if av1c.monochrome != 0 {
        return AvifPixelFormat::Yuv400;
    } else if av1c.chroma_subsampling_y == 1 {
        return AvifPixelFormat::Yuv420;
    } else if av1c.chroma_subsampling_x == 1 {
        return AvifPixelFormat::Yuv422;
    }
    AvifPixelFormat::Yuv444
}

/// Translation of `avifSampleTableGetProperties()`.
fn avif_sample_table_get_properties(
    sample_table: &AvifSampleTable,
    codec_type: AvifCodecType,
) -> Option<&[AvifProperty]> {
    for description in &sample_table.sample_descriptions {
        if avif_get_codec_type(&description.format) == codec_type {
            return Some(&description.properties);
        }
    }
    None
}

/// one video track ("trak" contents). Translation of `avifTrack`.
#[derive(Clone, Default, Debug)]
pub(crate) struct AvifTrack {
    id: u32,
    /// if non-zero, this track is an auxC plane for Track #{auxForID}
    aux_for_id: u32,
    /// if non-zero, this track is premultiplied by Track #{premByID}
    prem_by_id: u32,
    media_timescale: u32,
    media_duration: u64,
    track_duration: u64,
    segment_duration: u64,
    is_repeating: bool,
    repetition_count: i32,
    width: u32,
    height: u32,
    sample_table: Option<Box<AvifSampleTable>>,
    meta: Box<AvifMeta>,
}

// ---------------------------------------------------------------------------
// avifCodecDecodeInput

/// Returns how many samples are in the chunk. Translation of
/// `avifGetSampleCountOfChunk()`.
fn avif_get_sample_count_of_chunk(
    sample_to_chunks: &[AvifSampleTableSampleToChunk],
    chunk_index: u32,
) -> u32 {
    let mut sample_count = 0;
    for sample_to_chunk in sample_to_chunks.iter().rev() {
        if sample_to_chunk.first_chunk <= chunk_index.wrapping_add(1) {
            sample_count = sample_to_chunk.samples_per_chunk;
            break;
        }
    }
    sample_count
}

/// Translation of `avifCodecDecodeInputFillFromSampleTable()`.
fn avif_codec_decode_input_fill_from_sample_table(
    decode_input: &mut AvifCodecDecodeInput,
    sample_table: &AvifSampleTable,
    image_count_limit: u32,
    size_hint: u64,
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    if image_count_limit != 0 {
        // Verify that the we're not about to exceed the frame count limit.

        let mut image_count_left = image_count_limit;
        for chunk_index in 0..sample_table.chunks.len() as u32 {
            // First, figure out how many samples are in this chunk
            let sample_count =
                avif_get_sample_count_of_chunk(&sample_table.sample_to_chunks, chunk_index);
            if sample_count == 0 {
                // chunks with 0 samples are invalid
                diag_printf!(diag, "Sample table contains a chunk with 0 samples");
                return AvifResult::BmffParseFailed;
            }

            if sample_count > image_count_left {
                // This file exceeds the imageCountLimit, bail out
                diag_printf!(diag, "Exceeded avifDecoder's imageCountLimit");
                return AvifResult::BmffParseFailed;
            }
            image_count_left -= sample_count;
        }
    }

    let mut sample_size_index: u32 = 0;
    for chunk_index in 0..sample_table.chunks.len() as u32 {
        let chunk_offset = sample_table.chunks[chunk_index as usize];

        // First, figure out how many samples are in this chunk
        let sample_count =
            avif_get_sample_count_of_chunk(&sample_table.sample_to_chunks, chunk_index);
        if sample_count == 0 {
            // chunks with 0 samples are invalid
            diag_printf!(diag, "Sample table contains a chunk with 0 samples");
            return AvifResult::BmffParseFailed;
        }

        let mut sample_offset = chunk_offset;
        for _sample_index in 0..sample_count {
            let mut sample_size = sample_table.all_samples_size;
            if sample_size == 0 {
                if sample_size_index as usize >= sample_table.sample_sizes.len() {
                    // We've run out of samples to sum
                    diag_printf!(diag, "Truncated sample table");
                    return AvifResult::BmffParseFailed;
                }
                sample_size = sample_table.sample_sizes[sample_size_index as usize];
            }

            let sample = AvifDecodeSample {
                offset: sample_offset,
                size: sample_size as usize,
                spatial_id: AVIF_SPATIAL_ID_UNSET, // Not filtering by spatial_id
                sync: false, // to potentially be set to true following the outer loop
                ..Default::default()
            };
            avif_checkerr!(
                try_push(&mut decode_input.samples, sample),
                AvifResult::OutOfMemory
            );

            if sample_size as u64 > u64::MAX - sample_offset {
                diag_printf!(
                    diag,
                    "Sample table contains an offset/size pair which overflows: [{} / {}]",
                    sample_offset,
                    sample_size
                );
                return AvifResult::BmffParseFailed;
            }
            if size_hint != 0 && ((sample_offset + sample_size as u64) > size_hint) {
                diag_printf!(diag, "Exceeded avifIO's sizeHint, possibly truncated data");
                return AvifResult::BmffParseFailed;
            }

            sample_offset += sample_size as u64;
            sample_size_index = sample_size_index.wrapping_add(1);
        }
    }

    // Mark appropriate samples as sync
    for &sample_number in &sample_table.sync_samples {
        let frame_index = sample_number.wrapping_sub(1); // sampleNumber is 1-based
        if (frame_index as usize) < decode_input.samples.len() {
            decode_input.samples[frame_index as usize].sync = true;
        }
    }

    // Assume frame 0 is sync, just in case the stss box is absent in the BMFF. (Unnecessary?)
    if let Some(first) = decode_input.samples.first_mut() {
        first.sync = true;
    }
    AvifResult::Ok
}

/// Translation of `avifCodecDecodeInputFillFromDecoderItem()`.
fn avif_codec_decode_input_fill_from_decoder_item(
    decode_input: &mut AvifCodecDecodeInput,
    item: &mut AvifDecoderItem,
    allow_progressive: bool,
    image_count_limit: u32,
    size_hint: u64,
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    if size_hint != 0 && (item.size as u64 > size_hint) {
        diag_printf!(diag, "Exceeded avifIO's sizeHint, possibly truncated data");
        return AvifResult::BmffParseFailed;
    }

    let mut layer_count: u8 = 0;
    let mut layer_sizes: [usize; 4] = [0; 4];
    let a1lx_prop = avif_property_array_find(&item.properties, b"a1lx").cloned();
    if let Some(a1lx_prop) = &a1lx_prop {
        // Calculate layer count and all layer sizes from the a1lx box, and then validate

        let mut remaining_size = item.size;
        for i in 0..3 {
            layer_count += 1;

            let layer_size = a1lx_prop.u.a1lx.layer_size[i] as usize;
            if layer_size != 0 {
                if layer_size >= remaining_size {
                    // >= instead of > because there must be room for the last layer
                    diag_printf!(diag, "a1lx layer index [{}] does not fit in item size", i);
                    return AvifResult::BmffParseFailed;
                }
                layer_sizes[i] = layer_size;
                remaining_size -= layer_size;
            } else {
                layer_sizes[i] = remaining_size;
                remaining_size = 0;
                break;
            }
        }
        if remaining_size > 0 {
            avif_assert_or_return!(layer_count == 3);
            layer_count += 1;
            layer_sizes[3] = remaining_size;
        }
    }

    let lsel_prop = avif_property_array_find(&item.properties, b"lsel").cloned();
    // Progressive images offer layers via the a1lxProp, but don't specify a layer selection with lsel.
    //
    // For backward compatibility with earlier drafts of AVIF spec v1.1.0, treat an absent lsel as
    // equivalent to layer_id == 0xFFFF during the transitional period. Remove !lselProp when the test
    // images have been updated to the v1.1.0 spec.
    item.progressive = a1lx_prop.is_some()
        && (lsel_prop.is_none()
            || lsel_prop
                .as_ref()
                .is_some_and(|p| p.u.lsel.layer_id == 0xFFFF));
    if let Some(lsel_prop) = lsel_prop.as_ref().filter(|p| p.u.lsel.layer_id != 0xFFFF) {
        // Layer selection. This requires that the underlying AV1 codec decodes all layers,
        // and then only returns the requested layer as a single frame. To the user of libavif,
        // this appears to be a single frame.

        decode_input.all_layers = true;

        let mut sample_size: usize = 0;
        if layer_count > 0 {
            // Optimization: If we're selecting a layer that doesn't require the entire image's payload (hinted via the a1lx box)

            if lsel_prop.u.lsel.layer_id >= layer_count as u16 {
                diag_printf!(
                    diag,
                    "lsel property requests layer index [{}] which isn't present in a1lx property ([{}] layers)",
                    lsel_prop.u.lsel.layer_id,
                    layer_count
                );
                return AvifResult::BmffParseFailed;
            }

            for i in 0..=lsel_prop.u.lsel.layer_id as usize {
                sample_size = sample_size.wrapping_add(layer_sizes[i]);
            }
        } else {
            // This layer's payload subsection is unknown, just use the whole payload
            sample_size = item.size;
        }

        avif_assert_or_return!((lsel_prop.u.lsel.layer_id as u32) < AVIF_MAX_AV1_LAYER_COUNT);
        let sample = AvifDecodeSample {
            item_id: item.id,
            offset: 0,
            size: sample_size,
            spatial_id: lsel_prop.u.lsel.layer_id as u8,
            sync: true,
            ..Default::default()
        };
        avif_checkerr!(
            try_push(&mut decode_input.samples, sample),
            AvifResult::OutOfMemory
        );
    } else if allow_progressive && item.progressive {
        // Progressive image. Decode all layers and expose them all to the user.

        if image_count_limit != 0 && (layer_count as u32 > image_count_limit) {
            diag_printf!(diag, "Exceeded avifDecoder's imageCountLimit (progressive)");
            return AvifResult::BmffParseFailed;
        }

        decode_input.all_layers = true;

        let mut offset: usize = 0;
        for i in 0..layer_count as usize {
            let sample = AvifDecodeSample {
                item_id: item.id,
                offset: offset as u64,
                size: layer_sizes[i],
                spatial_id: AVIF_SPATIAL_ID_UNSET,
                sync: i == 0, // Assume all layers depend on the first layer
                ..Default::default()
            };
            avif_checkerr!(
                try_push(&mut decode_input.samples, sample),
                AvifResult::OutOfMemory
            );

            offset = offset.wrapping_add(layer_sizes[i]);
        }
    } else {
        // Typical case: Use the entire item's payload for a single frame output

        let sample = AvifDecodeSample {
            item_id: item.id,
            offset: 0,
            size: item.size,
            spatial_id: AVIF_SPATIAL_ID_UNSET,
            sync: true,
            ..Default::default()
        };
        avif_checkerr!(
            try_push(&mut decode_input.samples, sample),
            AvifResult::OutOfMemory
        );
    }
    AvifResult::Ok
}

// ---------------------------------------------------------------------------
// Helper macros / functions

// (BEGIN_STREAM() is AvifROStream::start())

/// Use this to keep track of whether or not a child box that must be unique (0 or 1 present) has
/// been seen yet, when parsing a parent box. If the "seen" bit is already set for a given box when
/// it is encountered during parse, an error is thrown. Which bit corresponds to which box is
/// dictated entirely by the calling function.
///
/// Translation of `uniqueBoxSeen()`.
fn unique_box_seen(
    unique_box_flags: &mut u32,
    which_flag: u32,
    parent_box_type: &str,
    box_type: &str,
    diagnostics: Option<&AvifDiagnostics>,
) -> bool {
    let flag = 1u32 << which_flag;
    if *unique_box_flags & flag != 0 {
        // This box has already been seen. Error!
        diag_printf!(
            diagnostics,
            "Box[{}] contains a duplicate unique box of type '{}'",
            parent_box_type,
            box_type
        );
        return false;
    }

    // Mark this box as seen.
    *unique_box_flags |= flag;
    true
}

// ---------------------------------------------------------------------------
// avifDecoderData

/// The codec a tile decodes with: none yet, the decoder data's shared
/// color or alpha codec (`avifDecoderData.codec`/`codecAlpha`), or one the
/// tile owns.
#[derive(Debug, Default)]
enum TileCodec {
    #[default]
    None,
    Shared,
    SharedAlpha,
    Own(Box<AvifCodec>),
}

/// Translation of `avifTile`.
#[derive(Debug)]
struct AvifTile {
    input: AvifCodecDecodeInput,
    codec_type: AvifCodecType,
    /// This may point to a codec that it owns or point to a shared codec that it does not own. In the shared case, this will
    /// point to one of the avifCodec instances in avifDecoderData.
    codec: TileCodec,
    image: Box<AvifImage>,
    /// Either avifTrack.width or avifDecoderItem.width
    width: u32,
    /// Either avifTrack.height or avifDecoderItem.height
    height: u32,
    operating_point: u8,
}

/// This holds one "meta" box (from the BMFF and HEIF standards) worth of relevant-to-AVIF information.
/// * If a meta box is parsed from the root level of the BMFF, it can contain the information about
///   "items" which might be color planes, alpha planes, or EXIF or XMP metadata.
/// * If a meta box is parsed from inside of a track ("trak") box, any metadata (EXIF/XMP) items inside
///   of that box are implicitly associated with that track.
///
/// Translation of `avifMeta` (`avifMetaCreate()` is `Default`,
/// `avifMetaDestroy()` dropping it).
#[derive(Clone, Default, Debug)]
pub(crate) struct AvifMeta {
    /// Items (from HEIF) are the generic storage for any data that does not require timed processing
    /// (single image color planes, alpha planes, EXIF, XMP, etc). Each item has a unique integer ID >1,
    /// and is defined by a series of child boxes in a meta box:
    ///  * iloc - location:     byte offset to item data, item size in bytes
    ///  * iinf - information:  type of item (color planes, alpha plane, EXIF, XMP)
    ///  * ipco - properties:   dimensions, aspect ratio, image transformations, references to other items
    ///  * ipma - associations: Attaches an item in the properties list to a given item
    ///
    /// Items are lazily created in this array when any of the above boxes refer to one by a new (unseen) ID,
    /// and are then further modified/updated as new information for an item's ID is parsed.
    items: Vec<AvifDecoderItem>,

    /// Any ipco boxes explained above are populated into this array as a staging area, which are
    /// then duplicated into the appropriate items upon encountering an item property association
    /// (ipma) box.
    properties: Vec<AvifProperty>,

    /// Filled with the contents of this meta box's "idat" box, which is raw data that an item can
    /// directly refer to in its item location box (iloc) instead of just giving an offset into the
    /// overall file. If all items' iloc boxes simply point at an offset/length in the file itself,
    /// this buffer will likely be empty.
    idat: Vec<u8>,

    /// Ever-incrementing ID for uniquely identifying which 'meta' box contains an idat (when
    /// multiple meta boxes exist as BMFF siblings). Each time avifParseMetaBox() is called on an
    /// avifMeta struct, this value is incremented. Any time an additional meta box is detected at
    /// the same "level" (root level, trak level, etc), this ID helps distinguish which meta box's
    /// "idat" is which, as items implicitly reference idat boxes that exist in the same meta
    /// box.
    idat_id: u32,

    /// Contents of a pitm box, which signal which of the items in this file is the main image. For
    /// AVIF, this should point at an image item containing color planes, and all other items
    /// are ignored unless they refer to this item in some way (alpha plane, EXIF/XMP metadata).
    primary_item_id: u32,
}

/// Translation of `avifCheckItemID()`.
fn avif_check_item_id(
    box_fourcc: &str,
    item_id: u32,
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    if item_id == 0 {
        diag_printf!(
            diag,
            "Box[{}] has an invalid item ID [{}]",
            box_fourcc,
            item_id
        );
        return AvifResult::BmffParseFailed;
    }
    AvifResult::Ok
}

/// Translation of `avifMetaFindOrCreateItem()`: the item's index.
fn avif_meta_find_or_create_item(
    meta: &mut AvifMeta,
    item_id: u32,
    item: &mut usize,
) -> AvifResult {
    *item = usize::MAX;
    avif_assert_or_return!(item_id != 0);

    for (i, it) in meta.items.iter().enumerate() {
        if it.id == item_id {
            *item = i;
            return AvifResult::Ok;
        }
    }

    let new_item = AvifDecoderItem {
        id: item_id,
        ..Default::default()
    };
    avif_checkerr!(try_push(&mut meta.items, new_item), AvifResult::OutOfMemory);
    *item = meta.items.len() - 1;
    AvifResult::Ok
}

/// A group of AVIF tiles in an image item, such as a single tile or a grid
/// of multiple tiles. Translation of `avifTileInfo`.
#[derive(Clone, Copy, Default, Debug)]
struct AvifTileInfo {
    tile_count: u32,
    decoded_tile_count: u32,
    /// Within avifDecoderData.tiles.
    first_tile_index: u32,
    grid: AvifImageGrid,
}

/// Translation of `avifDecoderData` (`diag` is the decoder's, passed
/// along; `sourceSampleTable` is the index of the track whose sample table
/// it is).
#[derive(Debug)]
pub(crate) struct AvifDecoderData {
    /// The root-level meta box
    meta: Box<AvifMeta>,
    tracks: Vec<AvifTrack>,
    tiles: Vec<AvifTile>,
    tile_infos: [AvifTileInfo; AVIF_ITEM_CATEGORY_COUNT],
    source: AvifDecoderSource,
    /// When decoding AVIF images with grid, use a single decoder instance for all the tiles instead of creating a decoder instance
    /// for each tile. If that is the case, |codec| will be used by all the tiles.
    ///
    /// There are some edge cases where we will still need multiple decoder instances:
    /// * For animated AVIF with alpha, we will need two instances (one for the color planes and one for the alpha plane since they are both
    ///   encoded as separate video sequences). In this case, |codec| will be used for the color planes and |codecAlpha| will be
    ///   used for the alpha plane.
    /// * For grid images with multiple layers. In this case, each tile will need its own decoder instance since there would be
    ///   multiple layers in each tile. In this case, |codec| and |codecAlpha| are not used and each tile will have its own
    ///   decoder instance.
    /// * For grid images where the operating points of all the tiles are not the same. In this case, each tile needs its own
    ///   decoder instance (same as above).
    codec: Option<Box<AvifCodec>>,
    codec_alpha: Option<Box<AvifCodec>>,
    /// From the file's ftyp, used by AVIF_DECODER_SOURCE_AUTO
    major_brand: [u8; 4],
    /// NULL unless (source == AVIF_DECODER_SOURCE_TRACKS), owned by an avifTrack
    source_sample_table: Option<usize>,
    /// True if avifDecoder's image has had its CICP set correctly yet.
    /// This allows nclx colr boxes to override AV1 CICP, as specified in the MIAF
    /// standard (ISO/IEC 23000-22:2019), section 7.3.6.4:
    ///   The colour information property takes precedence over any colour information
    ///   in the image bitstream, i.e. if the property is present, colour information in
    ///   the bitstream shall be ignored.
    cicp_set: bool,
}

/// Translation of `avifDecoderDataCreate()`.
fn avif_decoder_data_create() -> Box<AvifDecoderData> {
    Box::new(AvifDecoderData {
        meta: Box::default(),
        tracks: Vec::new(),
        tiles: Vec::new(),
        tile_infos: [AvifTileInfo::default(); AVIF_ITEM_CATEGORY_COUNT],
        source: AvifDecoderSource::Auto,
        codec: None,
        codec_alpha: None,
        major_brand: [0; 4],
        source_sample_table: None,
        cicp_set: false,
    })
}

/// Translation of `avifDecoderDataResetCodec()`.
fn avif_decoder_data_reset_codec(data: &mut AvifDecoderData) {
    for tile in &mut data.tiles {
        avif_image_free_planes(&mut tile.image, AVIF_PLANES_ALL); // forget any pointers into codec image buffers
                                                                  // Check if tile->codec was created separately and destroy it in that case.
        tile.codec = TileCodec::None;
    }
    for c in 0..AVIF_ITEM_CATEGORY_COUNT {
        data.tile_infos[c].decoded_tile_count = 0;
    }
    data.codec = None;
    data.codec_alpha = None;
}

/// Translation of `avifDecoderDataCreateTile()`: the new tile's index.
fn avif_decoder_data_create_tile(
    data: &mut AvifDecoderData,
    codec_type: AvifCodecType,
    width: u32,
    height: u32,
    operating_point: u8,
) -> Option<usize> {
    let image = avif_image_create_empty()?;
    let tile = AvifTile {
        input: AvifCodecDecodeInput::default(),
        codec_type,
        codec: TileCodec::None,
        image,
        width,
        height,
        operating_point,
    };
    if !try_push(&mut data.tiles, tile) {
        return None;
    }
    Some(data.tiles.len() - 1)
}

/// Translation of `avifDecoderDataCreateTrack()`: the new track's index.
fn avif_decoder_data_create_track(data: &mut AvifDecoderData) -> Option<usize> {
    let track = AvifTrack::default();
    if !try_push(&mut data.tracks, track) {
        return None;
    }
    Some(data.tracks.len() - 1)
}

/// Translation of `avifDecoderDataClearTiles()`.
fn avif_decoder_data_clear_tiles(data: &mut AvifDecoderData) {
    data.tiles.clear();
    for c in 0..AVIF_ITEM_CATEGORY_COUNT {
        data.tile_infos[c].tile_count = 0;
        data.tile_infos[c].decoded_tile_count = 0;
    }
    data.codec = None;
    data.codec_alpha = None;
}

// (avifDecoderDataDestroy() is dropping the data)

/// This returns the max extent that has to be read in order to decode this item. If
/// the item is stored in an idat, the data has already been read during Parse() and
/// this function will return AVIF_RESULT_OK with a 0-byte extent.
///
/// Translation of `avifDecoderItemMaxExtent()`.
fn avif_decoder_item_max_extent(
    item: &AvifDecoderItem,
    idat: &[u8],
    sample: &AvifDecodeSample,
    out_extent: &mut AvifExtent,
) -> AvifResult {
    if item.extents.is_empty() {
        return AvifResult::TruncatedData;
    }

    if item.idat_stored {
        // construction_method: idat(1)

        if !idat.is_empty() {
            // Already read from a meta box during Parse()
            *out_extent = AvifExtent::default();
            return AvifResult::Ok;
        }

        // no associated idat box was found in the meta box, bail out
        return AvifResult::NoContent;
    }

    // construction_method: file(0)

    if sample.size == 0 {
        return AvifResult::TruncatedData;
    }
    let mut remaining_offset = sample.offset;
    let mut remaining_bytes = sample.size; // This may be smaller than item->size if the item is progressive

    // Assert that the for loop below will execute at least one iteration.
    avif_assert_or_return!(!item.extents.is_empty());
    let mut min_offset = u64::MAX;
    let mut max_offset: u64 = 0;
    for extent in &item.extents {
        // Make local copies of extent->offset and extent->size as they might need to be adjusted
        // due to the sample's offset.
        let mut start_offset = extent.offset;
        let mut extent_size = extent.size;
        if remaining_offset != 0 {
            if remaining_offset >= extent_size as u64 {
                remaining_offset -= extent_size as u64;
                continue;
            } else {
                if remaining_offset > u64::MAX - start_offset {
                    return AvifResult::BmffParseFailed;
                }
                start_offset += remaining_offset;
                extent_size -= remaining_offset as usize;
                remaining_offset = 0;
            }
        }

        let used_extent_size = if extent_size < remaining_bytes {
            extent_size
        } else {
            remaining_bytes
        };

        if used_extent_size as u64 > u64::MAX - start_offset {
            return AvifResult::BmffParseFailed;
        }
        let end_offset = start_offset + used_extent_size as u64;

        if min_offset > start_offset {
            min_offset = start_offset;
        }
        if max_offset < end_offset {
            max_offset = end_offset;
        }

        remaining_bytes -= used_extent_size;
        if remaining_bytes == 0 {
            // We've got enough bytes for this sample.
            break;
        }
    }

    if remaining_bytes != 0 {
        return AvifResult::TruncatedData;
    }

    out_extent.offset = min_offset;
    let extent_length = max_offset.wrapping_sub(min_offset);
    if extent_length > usize::MAX as u64 {
        return AvifResult::BmffParseFailed;
    }
    out_extent.size = extent_length as usize;
    AvifResult::Ok
}

/// Translation of `avifDecoderItemOperatingPoint()`.
fn avif_decoder_item_operating_point(item: &AvifDecoderItem) -> u8 {
    if let Some(a1op_prop) = avif_property_array_find(&item.properties, b"a1op") {
        return a1op_prop.u.a1op.op_index;
    }
    0 // default
}

/// Translation of `avifDecoderItemValidateProperties()`.
fn avif_decoder_item_validate_properties(
    meta: &AvifMeta,
    item_idx: usize,
    config_prop_name: &[u8; 4],
    diag: Option<&AvifDiagnostics>,
    strict_flags: AvifStrictFlags,
) -> AvifResult {
    let item = &meta.items[item_idx];
    let Some(config_prop) = avif_property_array_find(&item.properties, config_prop_name) else {
        // An item configuration property box is mandatory in all valid AVIF configurations. Bail out.
        diag_printf!(
            diag,
            "Item ID {} of type '{}' is missing mandatory {} property",
            item.id,
            fourcc(&item.type_),
            fourcc(config_prop_name)
        );
        return AvifResult::BmffParseFailed;
    };

    if &item.type_ == b"grid" {
        for tile in &meta.items {
            if tile.dimg_for_id != item.id {
                continue;
            }
            // Tile item types were checked in avifDecoderGenerateImageTiles(), no need to do it here.

            // MIAF (ISO 23000-22:2019), Section 7.3.11.4.1:
            //   All input images of a grid image item shall use the same [...] chroma sampling format,
            //   and the same decoder configuration (see 7.3.6.2).

            // The chroma sampling format is part of the decoder configuration.
            let Some(tile_config_prop) =
                avif_property_array_find(&tile.properties, config_prop_name)
            else {
                diag_printf!(
                    diag,
                    "Tile item ID {} of type '{}' is missing mandatory {} property",
                    tile.id,
                    fourcc(&tile.type_),
                    fourcc(config_prop_name)
                );
                return AvifResult::BmffParseFailed;
            };
            // configProp was copied from a tile item to the grid item. Comparing tileConfigProp with it
            // is equivalent to comparing tileConfigProp with the configPropName from the first tile.
            let (a, b) = (&tile_config_prop.u.av1c, &config_prop.u.av1c);
            if (a.seq_profile != b.seq_profile)
                || (a.seq_level_idx0 != b.seq_level_idx0)
                || (a.seq_tier0 != b.seq_tier0)
                || (a.high_bitdepth != b.high_bitdepth)
                || (a.twelve_bit != b.twelve_bit)
                || (a.monochrome != b.monochrome)
                || (a.chroma_subsampling_x != b.chroma_subsampling_x)
                || (a.chroma_subsampling_y != b.chroma_subsampling_y)
                || (a.chroma_sample_position != b.chroma_sample_position)
            {
                diag_printf!(
                    diag,
                    "The fields of the {} property of tile item ID {} of type '{}' differs from other tiles",
                    fourcc(config_prop_name),
                    tile.id,
                    fourcc(&tile.type_)
                );
                return AvifResult::BmffParseFailed;
            }
        }
    }

    let pixi_prop = avif_property_array_find(&item.properties, b"pixi");
    if pixi_prop.is_none() && (strict_flags & AVIF_STRICT_PIXI_REQUIRED) != 0 {
        // A pixi box is mandatory in all valid AVIF configurations. Bail out.
        diag_printf!(
            diag,
            "[Strict] Item ID {} of type '{}' is missing mandatory pixi property",
            item.id,
            fourcc(&item.type_)
        );
        return AvifResult::BmffParseFailed;
    }

    if let Some(pixi_prop) = pixi_prop {
        let config_depth = avif_codec_configuration_box_get_depth(&config_prop.u.av1c);
        for i in 0..pixi_prop.u.pixi.plane_count as usize {
            if pixi_prop.u.pixi.plane_depths[i] as u32 != config_depth {
                // pixi depth must match configuration property depth
                diag_printf!(
                    diag,
                    "Item ID {} depth specified by pixi property [{}] does not match {} property depth [{}]",
                    item.id,
                    pixi_prop.u.pixi.plane_depths[i],
                    fourcc(config_prop_name),
                    config_depth
                );
                return AvifResult::BmffParseFailed;
            }
        }
    }

    if strict_flags & AVIF_STRICT_CLAP_VALID != 0 {
        if let Some(clap_prop) = avif_property_array_find(&item.properties, b"clap") {
            let Some(ispe_prop) = avif_property_array_find(&item.properties, b"ispe") else {
                diag_printf!(
                    diag,
                    "[Strict] Item ID {} is missing an ispe property, so its clap property cannot be validated",
                    item.id
                );
                return AvifResult::BmffParseFailed;
            };

            let mut crop_rect = AvifCropRect::default();
            let image_w = ispe_prop.u.ispe.width;
            let image_h = ispe_prop.u.ispe.height;
            let config_format = avif_codec_configuration_box_get_format(&config_prop.u.av1c);
            let valid_clap = avif_crop_rect_convert_clean_aperture_box(
                &mut crop_rect,
                &clap_prop.u.clap,
                image_w,
                image_h,
                config_format,
                diag,
            );
            if !valid_clap {
                return AvifResult::BmffParseFailed;
            }
        }
    }
    AvifResult::Ok
}

/// Translation of `avifDecoderItemRead()`: on success, the range of the
/// item's merged extents (`item->mergedExtents` from the offset) that
/// `outData` points to (see [`AvifDecoderItem::merged_extents_data`]).
fn avif_decoder_item_read(
    meta: &mut AvifMeta,
    item_idx: usize,
    io: &mut dyn AvifIo,
    out_data: &mut (usize, usize),
    offset: usize,
    partial_byte_count: usize,
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    let AvifMeta { items, idat, .. } = meta;
    let item = &mut items[item_idx];
    if let Some(merged) = item.merged_extents_data(idat) {
        if !item.partial_merged_extents {
            // Multiple extents have already been concatenated for this item, just return it
            let merged_size = merged.len();
            if offset >= merged_size {
                diag_printf!(diag, "Item ID {} read has overflowing offset", item.id);
                return AvifResult::TruncatedData;
            }
            *out_data = (offset, merged_size - offset);
            return AvifResult::Ok;
        }
    }

    if item.extents.is_empty() {
        diag_printf!(diag, "Item ID {} has zero extents", item.id);
        return AvifResult::TruncatedData;
    }

    // Find this item's source of all extents' data, based on the construction method
    let mut idat_buffer: Option<&[u8]> = None;
    if item.idat_stored {
        // construction_method: idat(1)

        if !idat.is_empty() {
            idat_buffer = Some(idat);
        } else {
            // no associated idat box was found in the meta box, bail out
            diag_printf!(
                diag,
                "Item ID {} is stored in an idat, but no associated idat box was found",
                item.id
            );
            return AvifResult::NoContent;
        }
    }

    // Merge extents into a single contiguous buffer
    if (io.size_hint() > 0) && (item.size as u64 > io.size_hint()) {
        // Sanity check: somehow the sum of extents exceeds the entire file or idat size!
        diag_printf!(
            diag,
            "Item ID {} reported size failed size hint sanity check. Truncated data?",
            item.id
        );
        return AvifResult::TruncatedData;
    }

    if offset >= item.size {
        diag_printf!(diag, "Item ID {} read has overflowing offset", item.id);
        return AvifResult::TruncatedData;
    }
    let max_output_size = item.size - offset;
    let read_output_size = if partial_byte_count != 0 && (partial_byte_count < max_output_size) {
        partial_byte_count
    } else {
        max_output_size
    };
    let total_bytes_to_read = offset + read_output_size;

    // If there is a single extent for this item and the source of the read buffer is going to be
    // persistent for the lifetime of the avifDecoder (whether it comes from its own internal
    // idatBuffer or from a known-persistent IO), we can avoid buffer duplication and just use the
    // preexisting buffer.
    // (the bytes of a persistent IO are copied here, which gives the same
    // results: only the idat is kept as a range)
    let single_persistent_buffer = (item.extents.len() == 1) && idat_buffer.is_some();
    let mut merged: Vec<u8> = Vec::new();
    if !single_persistent_buffer {
        // Always allocate the item's full size here, as progressive image decodes will do partial
        // reads into this buffer and begin feeding the buffer to the underlying AV1 decoder, but
        // will then write more into this buffer without flushing the AV1 decoder (which is still
        // holding the address of the previous allocation of this buffer). This strategy avoids
        // use-after-free issues in the AV1 decoder and unnecessary reallocs as a typical
        // progressive decode use case will eventually decode the final layer anyway.
        // (the buffer is reserved, and filled as the extents are read)
        if let MergedExtents::Owned(v) = &mut item.merged_extents {
            merged = std::mem::take(v);
        }
        merged.clear();
        if merged.capacity() < item.size && merged.try_reserve_exact(item.size).is_err() {
            return AvifResult::OutOfMemory;
        }
        item.owns_merged_extents = true;
    }

    // Set this until we manage to fill the entire mergedExtents buffer
    item.partial_merged_extents = true;

    let mut remaining_bytes = total_bytes_to_read;
    let mut result = AvifResult::Ok;
    for extent in &item.extents {
        let mut bytes_to_read = extent.size;
        if bytes_to_read > remaining_bytes {
            bytes_to_read = remaining_bytes;
        }

        if let Some(idat_buffer) = idat_buffer {
            if extent.offset > idat_buffer.len() as u64 {
                diag_printf!(
                    diag,
                    "Item ID {} has impossible extent offset in idat buffer",
                    item.id
                );
                result = AvifResult::BmffParseFailed;
                break;
            }
            // Since extent->offset (a uint64_t) is not bigger than idatBuffer->size (a size_t),
            // it is safe to cast extent->offset to size_t.
            let extent_offset = extent.offset as usize;
            if extent.size > idat_buffer.len() - extent_offset {
                diag_printf!(
                    diag,
                    "Item ID {} has impossible extent size in idat buffer",
                    item.id
                );
                result = AvifResult::BmffParseFailed;
                break;
            }
            if single_persistent_buffer {
                item.merged_extents = MergedExtents::Idat {
                    offset: extent_offset,
                    size: bytes_to_read,
                };
            } else {
                merged
                    .extend_from_slice(&idat_buffer[extent_offset..extent_offset + bytes_to_read]);
            }
        } else {
            // construction_method: file(0)

            if (io.size_hint() > 0) && (extent.offset > io.size_hint()) {
                diag_printf!(
                    diag,
                    "Item ID {} extent offset failed size hint sanity check. Truncated data?",
                    item.id
                );
                result = AvifResult::BmffParseFailed;
                break;
            }
            let offset_buffer = match io.read(0, extent.offset, bytes_to_read) {
                Ok(b) => b,
                Err(read_result) => {
                    result = read_result;
                    break;
                }
            };
            if bytes_to_read != offset_buffer.len() {
                diag_printf!(
                    diag,
                    "Item ID {} tried to read {} bytes, but only received {} bytes",
                    item.id,
                    bytes_to_read,
                    offset_buffer.len()
                );
                result = AvifResult::TruncatedData;
                break;
            }
            merged.extend_from_slice(offset_buffer);
        }

        remaining_bytes -= bytes_to_read;
        if remaining_bytes == 0 {
            // This happens when partialByteCount is set
            break;
        }
    }
    if !single_persistent_buffer {
        item.merged_extents = MergedExtents::Owned(merged);
    }
    if result != AvifResult::Ok {
        return result;
    }
    if remaining_bytes != 0 {
        // This should be impossible?
        diag_printf!(
            diag,
            "Item ID {} has {} unexpected trailing bytes",
            item.id,
            remaining_bytes
        );
        return AvifResult::TruncatedData;
    }

    *out_data = (offset, read_output_size);
    item.partial_merged_extents = item.size != total_bytes_to_read;
    AvifResult::Ok
}

/// The bytes [`avif_decoder_item_read`] read (`outData`).
fn item_read_bytes(meta: &AvifMeta, item_idx: usize, out: (usize, usize)) -> &[u8] {
    let item = &meta.items[item_idx];
    item.merged_extents_data(&meta.idat)
        .and_then(|d| d.get(out.0..out.0 + out.1))
        .unwrap_or(&[])
}

/// Returns the avifCodecType of the first tile of the gridItem.
/// Translation of `avifDecoderItemGetGridCodecType()`.
fn avif_decoder_item_get_grid_codec_type(meta: &AvifMeta, grid_item_idx: usize) -> AvifCodecType {
    let grid_id = meta.items[grid_item_idx].id;
    for item in &meta.items {
        let tile_codec_type = avif_get_codec_type(&item.type_);
        if (item.dimg_for_id == grid_id) && (tile_codec_type != AvifCodecType::Unknown) {
            return tile_codec_type;
        }
    }
    AvifCodecType::Unknown
}

/// Fills the dimgIdxToItemIdx array with a mapping from each 0-based tile index in the 'dimg' reference
/// to its corresponding 0-based index in the avifMeta::items array.
///
/// Translation of `avifFillDimgIdxToItemIdxArray()`.
fn avif_fill_dimg_idx_to_item_idx_array(
    dimg_idx_to_item_idx: &mut [u32],
    num_expected_tiles: u32,
    meta: &AvifMeta,
    grid_item_idx: usize,
) -> AvifResult {
    let item_index_not_set = u32::MAX;
    for dimg_idx in 0..num_expected_tiles as usize {
        dimg_idx_to_item_idx[dimg_idx] = item_index_not_set;
    }
    let grid_id = meta.items[grid_item_idx].id;
    let mut num_tiles: u32 = 0;
    for (i, item) in meta.items.iter().enumerate() {
        if item.dimg_for_id == grid_id {
            let tile_item_dimg_idx = item.dimg_idx;
            avif_checkerr!(
                tile_item_dimg_idx < num_expected_tiles,
                AvifResult::InvalidImageGrid
            );
            avif_checkerr!(
                dimg_idx_to_item_idx[tile_item_dimg_idx as usize] == item_index_not_set,
                AvifResult::InvalidImageGrid
            );
            dimg_idx_to_item_idx[tile_item_dimg_idx as usize] = i as u32;
            num_tiles += 1;
        }
    }
    // The number of tiles has been verified in avifDecoderItemReadAndParse().
    avif_assert_or_return!(num_tiles == num_expected_tiles);
    AvifResult::Ok
}

/// Creates the tiles and associate them to the items in the order of the
/// 'dimg' association. Translation of `avifDecoderGenerateImageGridTiles()`.
#[allow(clippy::too_many_arguments)]
fn avif_decoder_generate_image_grid_tiles(
    data: &mut AvifDecoderData,
    allow_progressive: bool,
    image_count_limit: u32,
    size_hint: u64,
    diag: Option<&AvifDiagnostics>,
    grid_item_idx: usize,
    item_category: AvifItemCategory,
    dimg_idx_to_item_idx: &[u32],
    num_tiles: u32,
) -> AvifResult {
    let mut first_tile_item: Option<usize> = None;
    let mut progressive = true;
    for dimg_idx in 0..num_tiles as usize {
        let item_idx = dimg_idx_to_item_idx[dimg_idx] as usize;
        avif_assert_or_return!(item_idx < data.meta.items.len());
        let item = &data.meta.items[item_idx];

        // According to HEIF (ISO 14496-12), Section 6.6.2.3.1, the SingleItemTypeReferenceBox of type 'dimg'
        // identifies the input images of the derived image item of type 'grid'. Since the reference_count
        // shall be equal to rows*columns, unknown tile item types cannot be skipped but must be considered
        // as errors.
        let tile_codec_type = avif_get_codec_type(&item.type_);
        if tile_codec_type == AvifCodecType::Unknown {
            let mut type_ = [0u8; 4];
            for j in 0..4 {
                if (0x20..0x7f).contains(&item.type_[j]) {
                    type_[j] = item.type_[j];
                } else {
                    type_[j] = b'.';
                }
            }
            diag_printf!(
                diag,
                "Tile item ID {} has an unknown item type '{}' ({:02x}{:02x}{:02x}{:02x})",
                item.id,
                fourcc(&type_),
                item.type_[0],
                item.type_[1],
                item.type_[2],
                item.type_[3]
            );
            return AvifResult::InvalidImageGrid;
        }

        if item.has_unsupported_essential_property {
            // An essential property isn't supported by libavif; can't
            // decode a grid image if any tile in the grid isn't supported.
            diag_printf!(
                diag,
                "Grid image contains tile with an unsupported property marked as essential"
            );
            return AvifResult::InvalidImageGrid;
        }

        let (width, height, operating_point) = (
            item.width,
            item.height,
            avif_decoder_item_operating_point(item),
        );
        let tile =
            avif_decoder_data_create_tile(data, tile_codec_type, width, height, operating_point);
        avif_checkerr!(tile.is_some(), AvifResult::OutOfMemory);
        let Some(tile) = tile else {
            return AvifResult::OutOfMemory;
        };
        let AvifDecoderData { tiles, meta, .. } = data;
        avif_checkres!(avif_codec_decode_input_fill_from_decoder_item(
            &mut tiles[tile].input,
            &mut meta.items[item_idx],
            allow_progressive,
            image_count_limit,
            size_hint,
            diag
        ));
        tiles[tile].input.item_category = item_category;

        let item = &meta.items[item_idx];
        if let Some(first) = first_tile_item {
            if item.type_ != meta.items[first].type_ {
                // MIAF (ISO 23000-22:2019), Section 7.3.11.4.1:
                //   All input images of a grid image item shall use the same coding format [...]
                // The coding format is defined by the item type.
                diag_printf!(
                    diag,
                    "Tile item ID {} of type '{}' differs from other tile type '{}'",
                    item.id,
                    fourcc(&item.type_),
                    fourcc(&meta.items[first].type_)
                );
                return AvifResult::InvalidImageGrid;
            }
        } else {
            first_tile_item = Some(item_idx);

            // Adopt the configuration property of the first image item tile, so that it can be queried from
            // the top-level color/alpha item during avifDecoderReset().
            let codec_type = avif_get_codec_type(&item.type_);
            let config_prop_name = avif_get_configuration_property_name(codec_type);
            let Some(src_prop) =
                avif_property_array_find(&item.properties, config_prop_name).cloned()
            else {
                diag_printf!(
                    diag,
                    "Grid image's first tile is missing an {} property",
                    fourcc(config_prop_name)
                );
                return AvifResult::InvalidImageGrid;
            };
            avif_checkerr!(
                try_push(&mut meta.items[grid_item_idx].properties, src_prop),
                AvifResult::OutOfMemory
            );
        }
        if !meta.items[item_idx].progressive {
            progressive = false;
        }
    }
    if item_category == AvifItemCategory::Color && progressive {
        // If all the items that make up the grid are progressive, then propagate that status to the top-level grid item.
        data.meta.items[grid_item_idx].progressive = true;
    }
    AvifResult::Ok
}

/// Allocates the dstImage. Also verifies some spec compliance rules for
/// grids, if relevant. Translation of
/// `avifDecoderDataAllocateImagePlanes()`.
fn avif_decoder_data_allocate_image_planes(
    data: &mut AvifDecoderData,
    info: &AvifTileInfo,
    dst_image: &mut AvifImage,
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    let tile = &data.tiles[info.first_tile_index as usize];
    let dst_width;
    let dst_height;

    if info.grid.rows > 0 && info.grid.columns > 0 {
        let grid = &info.grid;
        // Validate grid image size and tile size.
        //
        // HEIF (ISO/IEC 23008-12:2017), Section 6.6.2.3.1:
        //   The tiled input images shall completely "cover" the reconstructed image grid canvas, ...
        if (tile.image.width.wrapping_mul(grid.columns) < grid.output_width)
            || (tile.image.height.wrapping_mul(grid.rows) < grid.output_height)
        {
            diag_printf!(
                diag,
                "Grid image tiles do not completely cover the image (HEIF (ISO/IEC 23008-12:2017), Section 6.6.2.3.1)"
            );
            return AvifResult::InvalidImageGrid;
        }
        // Tiles in the rightmost column and bottommost row must overlap the reconstructed image grid canvas. See MIAF (ISO/IEC 23000-22:2019), Section 7.3.11.4.2, Figure 2.
        if (tile.image.width.wrapping_mul(grid.columns - 1) >= grid.output_width)
            || (tile.image.height.wrapping_mul(grid.rows - 1) >= grid.output_height)
        {
            diag_printf!(
                diag,
                "Grid image tiles in the rightmost column and bottommost row do not overlap the reconstructed image grid canvas. See MIAF (ISO/IEC 23000-22:2019), Section 7.3.11.4.2, Figure 2"
            );
            return AvifResult::InvalidImageGrid;
        }
        if !avif_are_grid_dimensions_valid(
            tile.image.yuv_format,
            grid.output_width,
            grid.output_height,
            tile.image.width,
            tile.image.height,
            diag,
        ) {
            return AvifResult::InvalidImageGrid;
        }
        dst_width = grid.output_width;
        dst_height = grid.output_height;
    } else {
        // Only one tile. Width and height are inherited from the 'ispe' property of the corresponding avifDecoderItem.
        dst_width = tile.width;
        dst_height = tile.height;
    }

    let alpha = avif_is_alpha(tile.input.item_category);
    if alpha {
        // An alpha tile does not contain any YUV pixels.
        avif_assert_or_return!(tile.image.yuv_format == AvifPixelFormat::None);
    }

    let dst_depth = tile.image.depth;

    // Lazily populate dstImage with the new frame's properties.
    let dims_or_depth_is_different = (dst_image.width != dst_width)
        || (dst_image.height != dst_height)
        || (dst_image.depth != dst_depth);
    let yuv_format_is_different = !alpha && (dst_image.yuv_format != tile.image.yuv_format);
    if dims_or_depth_is_different || yuv_format_is_different {
        if alpha {
            // Alpha doesn't match size, just bail out
            diag_printf!(
                diag,
                "Alpha plane dimensions do not match color plane dimensions"
            );
            return AvifResult::InvalidImageGrid;
        }

        if dims_or_depth_is_different {
            avif_image_free_planes(dst_image, AVIF_PLANES_ALL);
            dst_image.width = dst_width;
            dst_image.height = dst_height;
            dst_image.depth = dst_depth;
        }
        if yuv_format_is_different {
            avif_image_free_planes(dst_image, AVIF_PLANES_YUV);
            dst_image.yuv_format = tile.image.yuv_format;
        }
        // Keep dstImage->yuvRange which is already set to its correct value
        // (extracted from the 'colr' box if parsed or from a Sequence Header OBU otherwise).

        if !data.cicp_set {
            data.cicp_set = true;
            dst_image.color_primaries = tile.image.color_primaries;
            dst_image.transfer_characteristics = tile.image.transfer_characteristics;
            dst_image.matrix_coefficients = tile.image.matrix_coefficients;
        }
    }

    if avif_image_allocate_planes(
        dst_image,
        if alpha {
            AVIF_PLANES_A
        } else {
            AVIF_PLANES_YUV
        },
    ) != AvifResult::Ok
    {
        diag_printf!(diag, "Image allocation failure");
        return AvifResult::OutOfMemory;
    }
    AvifResult::Ok
}

/// Copies over the pixels from the tile into dstImage.
/// Verifies that the relevant properties of the tile match those of the first tile in case of a grid.
///
/// Translation of `avifDecoderDataCopyTileToImage()`: the destination
/// view (`dstView`) is a view of the planes that the copy writes through,
/// so the copy is made into `dstImage` itself at the view's offsets.
fn avif_decoder_data_copy_tile_to_image(
    data: &AvifDecoderData,
    info: &AvifTileInfo,
    dst_image: &mut AvifImage,
    tile_index_in_tiles: usize,
    tile_index: u32,
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    let first_tile = &data.tiles[info.first_tile_index as usize];
    let tile = &data.tiles[tile_index_in_tiles];
    if tile_index_in_tiles != info.first_tile_index as usize {
        // Check for tile consistency. All tiles in a grid image should match the first tile in the properties checked below.
        if (tile.image.width != first_tile.image.width)
            || (tile.image.height != first_tile.image.height)
            || (tile.image.depth != first_tile.image.depth)
            || (tile.image.yuv_format != first_tile.image.yuv_format)
            || (tile.image.yuv_range != first_tile.image.yuv_range)
            || (tile.image.color_primaries != first_tile.image.color_primaries)
            || (tile.image.transfer_characteristics != first_tile.image.transfer_characteristics)
            || (tile.image.matrix_coefficients != first_tile.image.matrix_coefficients)
        {
            diag_printf!(diag, "Grid image contains mismatched tiles");
            return AvifResult::InvalidImageGrid;
        }
    }

    let mut src_view = AvifImage::default();
    avif_image_set_defaults(&mut src_view);
    let mut dst_view = AvifImage::default();
    avif_image_set_defaults(&mut dst_view);
    let mut dst_view_rect = AvifCropRect {
        x: 0,
        y: 0,
        width: first_tile.image.width,
        height: first_tile.image.height,
    };
    if info.grid.rows > 0 && info.grid.columns > 0 {
        let row_index = tile_index / info.grid.columns;
        let col_index = tile_index % info.grid.columns;
        dst_view_rect.x = first_tile.image.width.wrapping_mul(col_index);
        dst_view_rect.y = first_tile.image.height.wrapping_mul(row_index);
        if dst_view_rect.x.wrapping_add(dst_view_rect.width) > info.grid.output_width {
            dst_view_rect.width = info.grid.output_width.wrapping_sub(dst_view_rect.x);
        }
        if dst_view_rect.y.wrapping_add(dst_view_rect.height) > info.grid.output_height {
            dst_view_rect.height = info.grid.output_height.wrapping_sub(dst_view_rect.y);
        }
    }
    let src_view_rect = AvifCropRect {
        x: 0,
        y: 0,
        width: dst_view_rect.width,
        height: dst_view_rect.height,
    };
    avif_assert_or_return!(
        avif_image_set_view_rect(&mut dst_view, dst_image, &dst_view_rect) == AvifResult::Ok
            && avif_image_set_view_rect(&mut src_view, &tile.image, &src_view_rect)
                == AvifResult::Ok
    );
    let planes = if avif_is_alpha(tile.input.item_category) {
        AVIF_PLANES_A
    } else {
        AVIF_PLANES_YUV
    };
    // (avifImageCopySamples(&dstView, &srcView, planes): the destination
    // view's planes are dstImage's, at the view's offsets)
    let offsets: [Option<usize>; 4] = [
        dst_view.yuv_planes[AVIF_CHAN_Y].as_ref().map(|p| p.offset),
        dst_view.yuv_planes[AVIF_CHAN_U].as_ref().map(|p| p.offset),
        dst_view.yuv_planes[AVIF_CHAN_V].as_ref().map(|p| p.offset),
        dst_view.alpha_plane.as_ref().map(|p| p.offset),
    ];
    drop(dst_view);
    let mut dst = AvifImage {
        width: src_view.width,
        height: src_view.height,
        depth: dst_image.depth,
        yuv_format: dst_image.yuv_format,
        yuv_row_bytes: dst_image.yuv_row_bytes,
        alpha_row_bytes: dst_image.alpha_row_bytes,
        ..Default::default()
    };
    // Move the destination planes into the view, copy, and move them back.
    for c in AVIF_CHAN_Y..=AVIF_CHAN_V {
        dst.yuv_planes[c] = dst_image.yuv_planes[c].take().map(|mut p| {
            p.offset = offsets[c].unwrap_or(p.offset);
            p
        });
    }
    dst.alpha_plane = dst_image.alpha_plane.take().map(|mut p| {
        p.offset = offsets[3].unwrap_or(p.offset);
        p
    });
    avif_image_copy_samples(&mut dst, &src_view, planes);
    for c in AVIF_CHAN_Y..=AVIF_CHAN_V {
        dst_image.yuv_planes[c] = dst.yuv_planes[c].take().map(|mut p| {
            p.offset = 0;
            p
        });
    }
    dst_image.alpha_plane = dst.alpha_plane.take().map(|mut p| {
        p.offset = 0;
        p
    });
    AvifResult::Ok
}

/// If colorId == 0 (a sentinel value as item IDs must be nonzero), accept any found EXIF/XMP metadata. Passing in 0
/// is used when finding metadata in a meta box embedded in a trak box, as any items inside of a meta box that is
/// inside of a trak box are implicitly associated to the track.
///
/// Translation of `avifDecoderFindMetadata()`.
fn avif_decoder_find_metadata(
    ignore_exif: bool,
    ignore_xmp: bool,
    diag: &AvifDiagnostics,
    io: &mut dyn AvifIo,
    meta: &mut AvifMeta,
    image: &mut AvifImage,
    color_id: u32,
) -> AvifResult {
    if ignore_exif && ignore_xmp {
        // Nothing to do!
        return AvifResult::Ok;
    }

    for item_index in 0..meta.items.len() {
        let item = &meta.items[item_index];
        if item.size == 0 {
            continue;
        }
        if item.has_unsupported_essential_property {
            // An essential property isn't supported by libavif; ignore the item.
            continue;
        }

        if (color_id > 0) && (item.desc_for_id != color_id) {
            // Not a content description (metadata) for the colorOBU, skip it
            continue;
        }

        if !ignore_exif && &item.type_ == b"Exif" {
            let mut exif_contents = (0, 0);
            let read_result =
                avif_decoder_item_read(meta, item_index, io, &mut exif_contents, 0, 0, Some(diag));
            if read_result != AvifResult::Ok {
                return read_result;
            }
            let exif_contents = item_read_bytes(meta, item_index, exif_contents);

            // Advance past Annex A.2.1's header
            let mut exif_box_stream = AvifROStream::start(exif_contents, Some(diag), "Exif header");
            {
                let mut exif_tiff_header_offset = 0u32;
                avif_checkerr!(
                    exif_box_stream.read_u32(&mut exif_tiff_header_offset),
                    AvifResult::InvalidExifPayload
                ); // unsigned int(32) exif_tiff_header_offset;
                let mut expected_exif_tiff_header_offset = 0usize;
                avif_checkres!(avif_get_exif_tiff_header_offset(
                    exif_box_stream.current(),
                    &mut expected_exif_tiff_header_offset
                ));
                avif_checkerr!(
                    exif_tiff_header_offset as usize == expected_exif_tiff_header_offset,
                    AvifResult::InvalidExifPayload
                );
            }

            avif_checkres!(avif_rw_data_set(&mut image.exif, exif_box_stream.current()));
        } else if !ignore_xmp
            && &item.type_ == b"mime"
            && item.content_type.content_type == AVIF_CONTENT_TYPE_XMP
        {
            let mut xmp_contents = (0, 0);
            let read_result =
                avif_decoder_item_read(meta, item_index, io, &mut xmp_contents, 0, 0, Some(diag));
            if read_result != AvifResult::Ok {
                return read_result;
            }
            let xmp_contents = item_read_bytes(meta, item_index, xmp_contents);

            avif_checkres!(avif_image_set_metadata_xmp(image, xmp_contents));
        }
    }
    AvifResult::Ok
}

// ---------------------------------------------------------------------------
// URN

/// Translation of `isAlphaURN()`.
fn is_alpha_urn(urn: &[u8]) -> bool {
    urn == AVIF_URN_ALPHA0 || urn == AVIF_URN_ALPHA1
}

// ---------------------------------------------------------------------------
// BMFF Parsing

/// Translation of `avifParseHandlerBox()`.
fn avif_parse_handler_box(raw: &[u8], diag: Option<&AvifDiagnostics>) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[hdlr]");

    avif_check!(s.read_and_enforce_version(0));

    let mut predefined = 0u32;
    avif_check!(s.read_u32(&mut predefined)); // unsigned int(32) pre_defined = 0;
    if predefined != 0 {
        diag_printf!(
            diag,
            "Box[hdlr] contains a pre_defined value that is nonzero"
        );
        return false;
    }

    let mut handler_type = [0u8; 4];
    avif_check!(s.read(&mut handler_type)); // unsigned int(32) handler_type;
    if &handler_type != b"pict" {
        diag_printf!(diag, "Box[hdlr] handler_type is not 'pict'");
        return false;
    }

    for _ in 0..3 {
        let mut reserved = 0u32;
        avif_check!(s.read_u32(&mut reserved)); // const unsigned int(32)[3] reserved = 0;
    }

    // Verify that a valid string is here, but don't bother to store it
    avif_check!(s.read_string(None, 0)); // string name;
    true
}

/// Translation of `avifParseItemLocationBox()`.
fn avif_parse_item_location_box(
    meta: &mut AvifMeta,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    let mut s = AvifROStream::start(raw, diag, "Box[iloc]");

    // Section 8.11.3.2 of ISO/IEC 14496-12.
    let mut version = 0u8;
    avif_checkerr!(
        s.read_version_and_flags(Some(&mut version), None),
        AvifResult::BmffParseFailed
    );
    if version > 2 {
        diag_printf!(diag, "Box[iloc] has an unsupported version [{}]", version);
        return AvifResult::BmffParseFailed;
    }

    let mut offset_size = 0u8;
    let mut length_size = 0u8;
    let mut base_offset_size = 0u8;
    let mut index_size = 0u8;
    let mut reserved = 0u32;
    avif_checkerr!(
        s.read_bits8(&mut offset_size, /*bitCount=*/ 4),
        AvifResult::BmffParseFailed
    ); // unsigned int(4) offset_size;
    avif_checkerr!(
        s.read_bits8(&mut length_size, /*bitCount=*/ 4),
        AvifResult::BmffParseFailed
    ); // unsigned int(4) length_size;
    avif_checkerr!(
        s.read_bits8(&mut base_offset_size, /*bitCount=*/ 4),
        AvifResult::BmffParseFailed
    ); // unsigned int(4) base_offset_size;
    if version == 1 || version == 2 {
        avif_checkerr!(
            s.read_bits8(&mut index_size, /*bitCount=*/ 4),
            AvifResult::BmffParseFailed
        ); // unsigned int(4) index_size;
    } else {
        avif_checkerr!(
            s.read_bits(&mut reserved, /*bitCount=*/ 4),
            AvifResult::BmffParseFailed
        ); // unsigned int(4) reserved;
    }

    // Section 8.11.3.3 of ISO/IEC 14496-12.
    let valid = |v: u8| v == 0 || v == 4 || v == 8;
    if !valid(offset_size) || !valid(length_size) || !valid(base_offset_size) || !valid(index_size)
    {
        diag_printf!(diag, "Box[iloc] has an invalid size");
        return AvifResult::BmffParseFailed;
    }

    let mut tmp16 = 0u16;
    let mut item_count = 0u32;
    if version < 2 {
        avif_checkerr!(s.read_u16(&mut tmp16), AvifResult::BmffParseFailed); // unsigned int(16) item_count;
        item_count = tmp16 as u32;
    } else {
        avif_checkerr!(s.read_u32(&mut item_count), AvifResult::BmffParseFailed);
        // unsigned int(32) item_count;
    }
    for _ in 0..item_count {
        let mut item_id = 0u32;
        if version < 2 {
            avif_checkerr!(s.read_u16(&mut tmp16), AvifResult::BmffParseFailed); // unsigned int(16) item_ID;
            item_id = tmp16 as u32;
        } else {
            avif_checkerr!(s.read_u32(&mut item_id), AvifResult::BmffParseFailed);
            // unsigned int(32) item_ID;
        }
        avif_checkres!(avif_check_item_id("iloc", item_id, diag));

        let mut item = 0usize;
        avif_checkres!(avif_meta_find_or_create_item(meta, item_id, &mut item));
        let item = &mut meta.items[item];
        if !item.extents.is_empty() {
            // This item has already been given extents via this iloc box. This is invalid.
            diag_printf!(
                diag,
                "Item ID [{}] contains duplicate sets of extents",
                item_id
            );
            return AvifResult::BmffParseFailed;
        }

        if version == 1 || version == 2 {
            avif_checkerr!(
                s.read_bits(&mut reserved, /*bitCount=*/ 12),
                AvifResult::BmffParseFailed
            ); // unsigned int(12) reserved = 0;
            if reserved != 0 {
                diag_printf!(
                    diag,
                    "Box[iloc] has a non null reserved field [{}]",
                    reserved
                );
                return AvifResult::BmffParseFailed;
            }
            let mut construction_method = 0u8;
            avif_checkerr!(
                s.read_bits8(&mut construction_method, /*bitCount=*/ 4),
                AvifResult::BmffParseFailed
            ); // unsigned int(4) construction_method;
            if construction_method != 0 /* file offset */ && construction_method != 1
            /* idat offset */
            {
                // construction method 2 (item offset) unsupported
                diag_printf!(
                    diag,
                    "Box[iloc] has an unsupported construction method [{}]",
                    construction_method
                );
                return AvifResult::BmffParseFailed;
            }
            if construction_method == 1 {
                item.idat_stored = true;
            }
        }

        let mut data_reference_index = 0u16;
        avif_checkerr!(
            s.read_u16(&mut data_reference_index),
            AvifResult::BmffParseFailed
        ); // unsigned int(16) data_reference_index;
        let mut base_offset = 0u64;
        avif_checkerr!(
            s.read_ux8(&mut base_offset, base_offset_size as u64),
            AvifResult::BmffParseFailed
        ); // unsigned int(base_offset_size*8) base_offset;
        let mut extent_count = 0u16;
        avif_checkerr!(s.read_u16(&mut extent_count), AvifResult::BmffParseFailed); // unsigned int(16) extent_count;
        for _ in 0..extent_count {
            if (version == 1 || version == 2) && index_size > 0 {
                // Section 8.11.3.1 of ISO/IEC 14496-12:
                //   The item_reference_index is only used for the method item_offset; it indicates the 1-based index
                //   of the item reference with referenceType 'iloc' linked from this item. If index_size is 0, then
                //   the value 1 is implied; the value 0 is reserved.
                let mut item_reference_index = 0u64; // Ignored unless construction_method=2 which is unsupported, but still read it.
                avif_checkerr!(
                    s.read_ux8(&mut item_reference_index, index_size as u64),
                    AvifResult::BmffParseFailed
                ); // unsigned int(index_size*8) item_reference_index;
            }

            let mut extent_offset = 0u64;
            avif_checkerr!(
                s.read_ux8(&mut extent_offset, offset_size as u64),
                AvifResult::BmffParseFailed
            ); // unsigned int(offset_size*8) extent_offset;
            let mut extent_length = 0u64;
            avif_checkerr!(
                s.read_ux8(&mut extent_length, length_size as u64),
                AvifResult::BmffParseFailed
            ); // unsigned int(length_size*8) extent_length;

            avif_checkerr!(
                try_push(&mut item.extents, AvifExtent::default()),
                AvifResult::OutOfMemory
            );
            if extent_offset > u64::MAX - base_offset {
                diag_printf!(
                    diag,
                    "Item ID [{}] contains an extent offset which overflows: [base: {} offset:{}]",
                    item_id,
                    base_offset,
                    extent_offset
                );
                return AvifResult::BmffParseFailed;
            }
            let offset = base_offset + extent_offset;
            let extent_index = item.extents.len() - 1;
            item.extents[extent_index].offset = offset;
            if extent_length > usize::MAX as u64 {
                diag_printf!(
                    diag,
                    "Item ID [{}] contains an extent length which overflows: [{}]",
                    item_id,
                    extent_length
                );
                return AvifResult::BmffParseFailed;
            }
            item.extents[extent_index].size = extent_length as usize;
            if extent_length as usize > usize::MAX - item.size {
                diag_printf!(
                    diag,
                    "Item ID [{}] contains an extent length which overflows the item size: [{}, {}]",
                    item_id,
                    extent_length,
                    item.size
                );
                return AvifResult::BmffParseFailed;
            }
            item.size += extent_length as usize;
        }
    }
    AvifResult::Ok
}

/// Translation of `avifParseImageGridBox()`.
fn avif_parse_image_grid_box(
    grid: &mut AvifImageGrid,
    raw: &[u8],
    image_size_limit: u32,
    image_dimension_limit: u32,
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[grid]");

    let mut version = 0u8;
    let mut flags = 0u8;
    avif_check!(s.read_u8(&mut version)); // unsigned int(8) version = 0;
    if version != 0 {
        diag_printf!(diag, "Box[grid] has unsupported version [{}]", version);
        return false;
    }
    let mut rows_minus_one = 0u8;
    let mut columns_minus_one = 0u8;
    avif_check!(s.read_u8(&mut flags)); // unsigned int(8) flags;
    avif_check!(s.read_u8(&mut rows_minus_one)); // unsigned int(8) rows_minus_one;
    avif_check!(s.read_u8(&mut columns_minus_one)); // unsigned int(8) columns_minus_one;
    grid.rows = rows_minus_one as u32 + 1;
    grid.columns = columns_minus_one as u32 + 1;

    let field_length = ((flags as u32 & 1) + 1) * 16;
    if field_length == 16 {
        let mut output_width16 = 0u16;
        let mut output_height16 = 0u16;
        avif_check!(s.read_u16(&mut output_width16)); // unsigned int(FieldLength) output_width;
        avif_check!(s.read_u16(&mut output_height16)); // unsigned int(FieldLength) output_height;
        grid.output_width = output_width16 as u32;
        grid.output_height = output_height16 as u32;
    } else {
        if field_length != 32 {
            // This should be impossible
            diag_printf!(
                diag,
                "Grid box contains illegal field length: [{}]",
                field_length
            );
            return false;
        }
        avif_check!(s.read_u32(&mut grid.output_width)); // unsigned int(FieldLength) output_width;
        avif_check!(s.read_u32(&mut grid.output_height)); // unsigned int(FieldLength) output_height;
    }
    if (grid.output_width == 0) || (grid.output_height == 0) {
        diag_printf!(
            diag,
            "Grid box contains illegal dimensions: [{} x {}]",
            grid.output_width,
            grid.output_height
        );
        return false;
    }
    if avif_dimensions_too_large(
        grid.output_width,
        grid.output_height,
        image_size_limit,
        image_dimension_limit,
    ) {
        diag_printf!(
            diag,
            "Grid box dimensions are too large: [{} x {}]",
            grid.output_width,
            grid.output_height
        );
        return false;
    }
    s.remaining_bytes() == 0
}

/// Extracts the codecType from the item type or from its children.
/// Also parses and outputs grid information if the item is a grid.
/// isItemInInput must be false if the item is a made-up structure
/// (and thus not part of the parseable input bitstream).
///
/// Translation of `avifDecoderItemReadAndParse()`.
#[allow(clippy::too_many_arguments)]
fn avif_decoder_item_read_and_parse(
    image_size_limit: u32,
    image_dimension_limit: u32,
    diag: Option<&AvifDiagnostics>,
    io: &mut dyn AvifIo,
    meta: &mut AvifMeta,
    item_idx: usize,
    is_item_in_input: bool,
    grid: &mut AvifImageGrid,
    codec_type: &mut AvifCodecType,
) -> AvifResult {
    if &meta.items[item_idx].type_ == b"grid" {
        if is_item_in_input {
            let mut read_data = (0, 0);
            avif_checkres!(avif_decoder_item_read(
                meta,
                item_idx,
                io,
                &mut read_data,
                0,
                0,
                diag
            ));
            avif_checkerr!(
                avif_parse_image_grid_box(
                    grid,
                    item_read_bytes(meta, item_idx, read_data),
                    image_size_limit,
                    image_dimension_limit,
                    diag
                ),
                AvifResult::InvalidImageGrid
            );
            // Validate that there are exactly the same number of dimg items to form the grid.
            let id = meta.items[item_idx].id;
            let mut dimg_item_count: u32 = 0;
            for item in &meta.items {
                if item.dimg_for_id == id {
                    dimg_item_count += 1;
                }
            }
            avif_checkerr!(
                dimg_item_count == grid.rows * grid.columns,
                AvifResult::InvalidImageGrid
            );
        } else {
            // item was generated for convenience and is not part of the bitstream.
            // grid information should already be set.
            avif_assert_or_return!(grid.rows > 0 && grid.columns > 0);
        }
        *codec_type = avif_decoder_item_get_grid_codec_type(meta, item_idx);
        avif_checkerr!(
            *codec_type != AvifCodecType::Unknown,
            AvifResult::InvalidImageGrid
        );
    } else {
        *codec_type = avif_get_codec_type(&meta.items[item_idx].type_);
        avif_assert_or_return!(*codec_type != AvifCodecType::Unknown);
    }
    // TODO(yguyon): If AVIF_ENABLE_EXPERIMENTAL_SAMPLE_TRANSFORM is defined, backward-incompatible
    //               files with a primary 'sato' Sample Transform derived image item could be
    //               handled here (compared to backward-compatible files with a 'sato' item in the
    //               same 'altr' group as the primary regular color item which are handled in
    //               avifDecoderDataFindSampleTransformImageItem() below).
    AvifResult::Ok
}

/// Translation of `avifParseImageSpatialExtentsProperty()`.
fn avif_parse_image_spatial_extents_property(
    prop: &mut AvifProperty,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[ispe]");
    avif_check!(s.read_and_enforce_version(0));

    let ispe = &mut prop.u.ispe;
    avif_check!(s.read_u32(&mut ispe.width));
    avif_check!(s.read_u32(&mut ispe.height));
    true
}

/// Translation of `avifParseAuxiliaryTypeProperty()`.
fn avif_parse_auxiliary_type_property(
    prop: &mut AvifProperty,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[auxC]");
    avif_check!(s.read_and_enforce_version(0));

    avif_check!(s.read_string(Some(&mut prop.u.aux_c.aux_type), AUXTYPE_SIZE));
    true
}

/// Translation of `avifParseColourInformationBox()`.
fn avif_parse_colour_information_box(
    prop: &mut AvifProperty,
    raw_offset: u64,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[colr]");

    let colr = &mut prop.u.colr;
    colr.has_icc = false;
    colr.has_nclx = false;

    let mut color_type = [0u8; 4]; // unsigned int(32) colour_type;
    avif_check!(s.read(&mut color_type));
    if &color_type == b"rICC" || &color_type == b"prof" {
        colr.has_icc = true;
        // Remember the offset of the ICC payload relative to the beginning of the stream. A direct pointer cannot be stored
        // because decoder->io->persistent could have been AVIF_FALSE when obtaining raw through decoder->io->read().
        // The bytes could be copied now instead of remembering the offset, but it is as invasive as passing rawOffset everywhere.
        colr.icc_offset = raw_offset.wrapping_add(s.offset() as u64);
        colr.icc_size = s.remaining_bytes();
    } else if &color_type == b"nclx" {
        avif_check!(s.read_u16(&mut colr.color_primaries)); // unsigned int(16) colour_primaries;
        avif_check!(s.read_u16(&mut colr.transfer_characteristics)); // unsigned int(16) transfer_characteristics;
        avif_check!(s.read_u16(&mut colr.matrix_coefficients)); // unsigned int(16) matrix_coefficients;
        let mut full_range_flag = 0u8;
        avif_check!(s.read_bits8(&mut full_range_flag, /*bitCount=*/ 1)); // unsigned int(1) full_range_flag;
        colr.range = if full_range_flag != 0 {
            AvifRange::Full
        } else {
            AvifRange::Limited
        };
        let mut reserved = 0u8;
        avif_check!(s.read_bits8(&mut reserved, /*bitCount=*/ 7)); // unsigned int(7) reserved = 0;
        if reserved != 0 {
            diag_printf!(
                diag,
                "Box[colr] contains nonzero reserved bits [{}]",
                reserved
            );
            return false;
        }
        colr.has_nclx = true;
    }
    true
}

/// Translation of `avifParseContentLightLevelInformationBox()`.
fn avif_parse_content_light_level_information_box(
    prop: &mut AvifProperty,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[clli]");

    let clli = &mut prop.u.clli;

    avif_check!(s.read_u16(&mut clli.max_cll)); // unsigned int(16) max_content_light_level
    avif_check!(s.read_u16(&mut clli.max_pall)); // unsigned int(16) max_pic_average_light_level
    true
}

/// Implementation of section 2.3.3 of AV1 Codec ISO Media File Format Binding specification v1.2.0.
/// See https://aomediacodec.github.io/av1-isobmff/v1.2.0.html#av1codecconfigurationbox-syntax.
///
/// Translation of `avifParseCodecConfiguration()`.
fn avif_parse_codec_configuration(
    s: &mut AvifROStream<'_, '_>,
    config: &mut AvifCodecConfigurationBox,
    config_prop_name: &str,
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let mut marker = 0u32;
    let mut version = 0u32;
    avif_check!(s.read_bits(&mut marker, /*bitCount=*/ 1)); // unsigned int (1) marker = 1;
    if marker == 0 {
        diag_printf!(
            diag,
            "{} contains illegal marker: [{}]",
            config_prop_name,
            marker
        );
        return false;
    }
    avif_check!(s.read_bits(&mut version, /*bitCount=*/ 7)); // unsigned int (7) version = 1;
    if version != 1 {
        diag_printf!(
            diag,
            "{} contains illegal version: [{}]",
            config_prop_name,
            version
        );
        return false;
    }

    avif_check!(s.read_bits8(&mut config.seq_profile, /*bitCount=*/ 3)); // unsigned int (3) seq_profile;
    avif_check!(s.read_bits8(&mut config.seq_level_idx0, /*bitCount=*/ 5)); // unsigned int (5) seq_level_idx_0;
    avif_check!(s.read_bits8(&mut config.seq_tier0, /*bitCount=*/ 1)); // unsigned int (1) seq_tier_0;
    avif_check!(s.read_bits8(&mut config.high_bitdepth, /*bitCount=*/ 1)); // unsigned int (1) high_bitdepth;
    avif_check!(s.read_bits8(&mut config.twelve_bit, /*bitCount=*/ 1)); // unsigned int (1) twelve_bit;
    avif_check!(s.read_bits8(&mut config.monochrome, /*bitCount=*/ 1)); // unsigned int (1) monochrome;
    avif_check!(s.read_bits8(&mut config.chroma_subsampling_x, /*bitCount=*/ 1)); // unsigned int (1) chroma_subsampling_x;
    avif_check!(s.read_bits8(&mut config.chroma_subsampling_y, /*bitCount=*/ 1)); // unsigned int (1) chroma_subsampling_y;
    avif_check!(s.read_bits8(&mut config.chroma_sample_position, /*bitCount=*/ 2)); // unsigned int (2) chroma_sample_position;

    // unsigned int (3) reserved = 0;
    // unsigned int (1) initial_presentation_delay_present;
    // if (initial_presentation_delay_present) {
    //   unsigned int (4) initial_presentation_delay_minus_one;
    // } else {
    //   unsigned int (4) reserved = 0;
    // }
    avif_check!(s.skip(/*byteCount=*/ 1));

    // According to section 2.2.1 of AV1 Image File Format specification v1.1.0:
    //   - Sequence Header OBUs should not be present in the AV1CodecConfigurationBox.
    //   - If a Sequence Header OBU is present in the AV1CodecConfigurationBox,
    //     it shall match the Sequence Header OBU in the AV1 Image Item Data.
    //   - Metadata OBUs, if present, shall match the values given in other item properties,
    //     such as the PixelInformationProperty or ColourInformationBox.
    // See https://aomediacodec.github.io/av1-avif/v1.1.0.html#av1-configuration-item-property.
    // For simplicity, the constraints above are not enforced.
    // The following is skipped by avifParseItemPropertyContainerBox().
    // unsigned int (8) configOBUs[];
    true
}

/// Translation of `avifParseCodecConfigurationBoxProperty()`.
fn avif_parse_codec_configuration_box_property(
    prop: &mut AvifProperty,
    raw: &[u8],
    config_prop_name: &str,
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let diag_context = format!("Box[{config_prop_name}]"); // "Box[av1C]" or "Box[av2C]"
    let mut s = AvifROStream::start(raw, diag, &diag_context);
    avif_parse_codec_configuration(&mut s, &mut prop.u.av1c, config_prop_name, diag)
}

/// Translation of `avifParsePixelAspectRatioBoxProperty()`.
fn avif_parse_pixel_aspect_ratio_box_property(
    prop: &mut AvifProperty,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[pasp]");

    let pasp = &mut prop.u.pasp;
    avif_check!(s.read_u32(&mut pasp.h_spacing)); // unsigned int(32) hSpacing;
    avif_check!(s.read_u32(&mut pasp.v_spacing)); // unsigned int(32) vSpacing;
    true
}

/// Translation of `avifParseCleanApertureBoxProperty()`.
fn avif_parse_clean_aperture_box_property(
    prop: &mut AvifProperty,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[clap]");

    let clap = &mut prop.u.clap;
    avif_check!(s.read_u32(&mut clap.width_n)); // unsigned int(32) cleanApertureWidthN;
    avif_check!(s.read_u32(&mut clap.width_d)); // unsigned int(32) cleanApertureWidthD;
    avif_check!(s.read_u32(&mut clap.height_n)); // unsigned int(32) cleanApertureHeightN;
    avif_check!(s.read_u32(&mut clap.height_d)); // unsigned int(32) cleanApertureHeightD;
    avif_check!(s.read_u32(&mut clap.horiz_off_n)); // unsigned int(32) horizOffN;
    avif_check!(s.read_u32(&mut clap.horiz_off_d)); // unsigned int(32) horizOffD;
    avif_check!(s.read_u32(&mut clap.vert_off_n)); // unsigned int(32) vertOffN;
    avif_check!(s.read_u32(&mut clap.vert_off_d)); // unsigned int(32) vertOffD;
    true
}

/// Translation of `avifParseImageRotationProperty()`.
fn avif_parse_image_rotation_property(
    prop: &mut AvifProperty,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[irot]");

    let irot = &mut prop.u.irot;
    let mut reserved = 0u8;
    avif_check!(s.read_bits8(&mut reserved, /*bitCount=*/ 6)); // unsigned int (6) reserved = 0;
    if reserved != 0 {
        diag_printf!(
            diag,
            "Box[irot] contains nonzero reserved bits [{}]",
            reserved
        );
        return false;
    }
    avif_check!(s.read_bits8(&mut irot.angle, /*bitCount=*/ 2)); // unsigned int (2) angle;
    true
}

/// Translation of `avifParseImageMirrorProperty()`.
fn avif_parse_image_mirror_property(
    prop: &mut AvifProperty,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[imir]");

    let imir = &mut prop.u.imir;
    let mut reserved = 0u8;
    avif_check!(s.read_bits8(&mut reserved, /*bitCount=*/ 7)); // unsigned int(7) reserved = 0;
    if reserved != 0 {
        diag_printf!(
            diag,
            "Box[imir] contains nonzero reserved bits [{}]",
            reserved
        );
        return false;
    }
    avif_check!(s.read_bits8(&mut imir.axis, /*bitCount=*/ 1)); // unsigned int(1) axis;
    true
}

/// Translation of `avifParsePixelInformationProperty()`.
fn avif_parse_pixel_information_property(
    prop: &mut AvifProperty,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[pixi]");
    avif_check!(s.read_and_enforce_version(0));

    let pixi = &mut prop.u.pixi;
    avif_check!(s.read_u8(&mut pixi.plane_count)); // unsigned int (8) num_channels;
    if pixi.plane_count < 1 || pixi.plane_count as usize > MAX_PIXI_PLANE_DEPTHS {
        diag_printf!(
            diag,
            "Box[pixi] contains unsupported plane count [{}]",
            pixi.plane_count
        );
        return false;
    }
    for i in 0..pixi.plane_count as usize {
        avif_check!(s.read_u8(&mut pixi.plane_depths[i])); // unsigned int (8) bits_per_channel;
        if pixi.plane_depths[i] != pixi.plane_depths[0] {
            diag_printf!(
                diag,
                "Box[pixi] contains unsupported mismatched plane depths [{} != {}]",
                pixi.plane_depths[i],
                pixi.plane_depths[0]
            );
            return false;
        }
    }
    true
}

/// Translation of `avifParseOperatingPointSelectorProperty()`.
fn avif_parse_operating_point_selector_property(
    prop: &mut AvifProperty,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[a1op]");

    let a1op = &mut prop.u.a1op;
    avif_check!(s.read_u8(&mut a1op.op_index));
    if a1op.op_index > 31 {
        // 31 is AV1's max operating point value
        diag_printf!(
            diag,
            "Box[a1op] contains an unsupported operating point [{}]",
            a1op.op_index
        );
        return false;
    }
    true
}

/// Translation of `avifParseLayerSelectorProperty()`.
fn avif_parse_layer_selector_property(
    prop: &mut AvifProperty,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[lsel]");

    let lsel = &mut prop.u.lsel;
    avif_check!(s.read_u16(&mut lsel.layer_id));
    if (lsel.layer_id != 0xFFFF) && (lsel.layer_id as u32 >= AVIF_MAX_AV1_LAYER_COUNT) {
        diag_printf!(
            diag,
            "Box[lsel] contains an unsupported layer [{}]",
            lsel.layer_id
        );
        return false;
    }
    true
}

/// Translation of `avifParseAV1LayeredImageIndexingProperty()`.
fn avif_parse_av1_layered_image_indexing_property(
    prop: &mut AvifProperty,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[a1lx]");

    let a1lx = &mut prop.u.a1lx;

    let mut large_size = 0u8;
    avif_check!(s.read_u8(&mut large_size));
    if large_size & 0xFE != 0 {
        diag_printf!(
            diag,
            "Box[a1lx] has bits set in the reserved section [{}]",
            large_size
        );
        return false;
    }

    for i in 0..3 {
        if large_size != 0 {
            avif_check!(s.read_u32(&mut a1lx.layer_size[i]));
        } else {
            let mut layer_size16 = 0u16;
            avif_check!(s.read_u16(&mut layer_size16));
            a1lx.layer_size[i] = layer_size16 as u32;
        }
    }

    // Layer sizes will be validated later (when the item's size is known)
    true
}

/// Translation of `avifParseItemPropertyContainerBox()`.
fn avif_parse_item_property_container_box(
    properties: &mut Vec<AvifProperty>,
    raw_offset: u64,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    let mut s = AvifROStream::start(raw, diag, "Box[ipco]");

    while s.has_bytes_left(1) {
        let mut header = AvifBoxHeader::default();
        avif_checkerr!(s.read_box_header(&mut header), AvifResult::BmffParseFailed);

        avif_checkerr!(
            try_push(properties, AvifProperty::default()),
            AvifResult::OutOfMemory
        );
        let prop_index = properties.len() - 1;
        let prop = &mut properties[prop_index];
        prop.type_ = header.type_;
        let body = &s.current()[..header.size];
        let ok = match &header.type_ {
            b"ispe" => avif_parse_image_spatial_extents_property(prop, body, diag),
            b"auxC" => avif_parse_auxiliary_type_property(prop, body, diag),
            b"colr" => avif_parse_colour_information_box(
                prop,
                raw_offset.wrapping_add(s.offset() as u64),
                body,
                diag,
            ),
            b"av1C" => avif_parse_codec_configuration_box_property(prop, body, "av1C", diag),
            b"pasp" => avif_parse_pixel_aspect_ratio_box_property(prop, body, diag),
            b"clap" => avif_parse_clean_aperture_box_property(prop, body, diag),
            b"irot" => avif_parse_image_rotation_property(prop, body, diag),
            b"imir" => avif_parse_image_mirror_property(prop, body, diag),
            b"pixi" => avif_parse_pixel_information_property(prop, body, diag),
            b"a1op" => avif_parse_operating_point_selector_property(prop, body, diag),
            b"lsel" => avif_parse_layer_selector_property(prop, body, diag),
            b"a1lx" => avif_parse_av1_layered_image_indexing_property(prop, body, diag),
            b"clli" => avif_parse_content_light_level_information_box(prop, body, diag),
            _ => true,
        };
        avif_checkerr!(ok, AvifResult::BmffParseFailed);

        avif_checkerr!(s.skip(header.size), AvifResult::BmffParseFailed);
    }
    AvifResult::Ok
}

/// Translation of `avifParseItemPropertyAssociation()`.
fn avif_parse_item_property_association(
    meta: &mut AvifMeta,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
    out_version_and_flags: &mut u32,
) -> AvifResult {
    // NOTE: If this function ever adds support for versions other than [0,1] or flags other than
    //       [0,1], please increase the value of MAX_IPMA_VERSION_AND_FLAGS_SEEN accordingly.

    let mut s = AvifROStream::start(raw, diag, "Box[ipma]");

    let mut version = 0u8;
    let mut flags = 0u32;
    avif_checkerr!(
        s.read_version_and_flags(Some(&mut version), Some(&mut flags)),
        AvifResult::BmffParseFailed
    );
    let property_index_is_u15 = (flags & 0x1) != 0;
    *out_version_and_flags = ((version as u32) << 24) | flags;

    let mut entry_count = 0u32;
    avif_checkerr!(s.read_u32(&mut entry_count), AvifResult::BmffParseFailed);
    let mut prev_item_id: u32 = 0;
    for _ in 0..entry_count {
        // ISO/IEC 14496-12, Seventh edition, 2022-01, Section 8.11.14.1:
        //   Each ItemPropertyAssociationBox shall be ordered by increasing item_ID, and there shall
        //   be at most one occurrence of a given item_ID, in the set of ItemPropertyAssociationBox
        //   boxes.
        let mut item_id = 0u32;
        if version < 1 {
            let mut tmp = 0u16;
            avif_checkerr!(s.read_u16(&mut tmp), AvifResult::BmffParseFailed);
            item_id = tmp as u32;
        } else {
            avif_checkerr!(s.read_u32(&mut item_id), AvifResult::BmffParseFailed);
        }
        avif_checkres!(avif_check_item_id("ipma", item_id, diag));
        if item_id <= prev_item_id {
            diag_printf!(diag, "Box[ipma] item IDs are not ordered by increasing ID");
            return AvifResult::BmffParseFailed;
        }
        prev_item_id = item_id;

        let mut item = 0usize;
        avif_checkres!(avif_meta_find_or_create_item(meta, item_id, &mut item));
        if meta.items[item].ipma_seen {
            diag_printf!(diag, "Duplicate Box[ipma] for item ID [{}]", item_id);
            return AvifResult::BmffParseFailed;
        }
        meta.items[item].ipma_seen = true;

        let mut association_count = 0u8;
        avif_checkerr!(
            s.read_u8(&mut association_count),
            AvifResult::BmffParseFailed
        );
        for _ in 0..association_count {
            let mut essential = 0u8;
            avif_checkerr!(
                s.read_bits8(&mut essential, /*bitCount=*/ 1),
                AvifResult::BmffParseFailed
            ); // bit(1) essential;
            let mut property_index = 0u32;
            avif_checkerr!(
                s.read_bits(
                    &mut property_index,
                    /*bitCount=*/ if property_index_is_u15 { 15 } else { 7 }
                ),
                AvifResult::BmffParseFailed
            ); // unsigned int(7/15) property_index;

            if property_index == 0 {
                // Not associated with any item
                continue;
            }
            property_index -= 1; // 1-indexed

            if property_index as usize >= meta.properties.len() {
                diag_printf!(
                    diag,
                    "Box[ipma] for item ID [{}] contains an illegal property index [{}] (out of [{}] properties)",
                    item_id,
                    property_index,
                    meta.properties.len()
                );
                return AvifResult::BmffParseFailed;
            }

            // Copy property to item
            let src_prop = &meta.properties[property_index as usize];

            const SUPPORTED_TYPES: [&[u8; 4]; 13] = [
                b"ispe", b"auxC", b"colr", b"av1C", b"pasp", b"clap", b"irot", b"imir", b"pixi",
                b"a1op", b"lsel", b"a1lx", b"clli",
            ];
            let supported_type = SUPPORTED_TYPES.iter().any(|t| **t == src_prop.type_);
            if supported_type {
                if essential != 0 {
                    // Verify that it is legal for this property to be flagged as essential. Any
                    // types in this list are *required* in the spec to not be flagged as essential
                    // when associated with an item.
                    const NONESSENTIAL_TYPES: [&str; 1] = [
                        // AVIF: Section 2.3.2.3.2: "If associated, it shall not be marked as essential."
                        "a1lx",
                    ];
                    for t in NONESSENTIAL_TYPES {
                        if src_prop.type_ == t.as_bytes() {
                            diag_printf!(
                                diag,
                                "Item ID [{}] has a {} property association which must not be marked essential, but is",
                                item_id,
                                t
                            );
                            return AvifResult::BmffParseFailed;
                        }
                    }
                } else {
                    // Verify that it is legal for this property to not be flagged as essential. Any
                    // types in this list are *required* in the spec to be flagged as essential when
                    // associated with an item.
                    const ESSENTIAL_TYPES: [&str; 2] = [
                        // AVIF: Section 2.3.2.1.1: "If associated, it shall be marked as essential."
                        "a1op",
                        // HEIF: Section 6.5.11.1: "essential shall be equal to 1 for an 'lsel' item property."
                        "lsel",
                    ];
                    for t in ESSENTIAL_TYPES {
                        if src_prop.type_ == t.as_bytes() {
                            diag_printf!(
                                diag,
                                "Item ID [{}] has a {} property association which must be marked essential, but is not",
                                item_id,
                                t
                            );
                            return AvifResult::BmffParseFailed;
                        }
                    }
                }

                // Supported and valid; associate it with this item.
                let dst_prop = src_prop.clone();
                avif_checkerr!(
                    try_push(&mut meta.items[item].properties, dst_prop),
                    AvifResult::OutOfMemory
                );
            } else if essential != 0 {
                // Discovered an essential item property that libavif doesn't support!
                // Make a note to ignore this item later.
                meta.items[item].has_unsupported_essential_property = true;
            }
        }
    }
    AvifResult::Ok
}

/// Translation of `avifParsePrimaryItemBox()`.
fn avif_parse_primary_item_box(
    meta: &mut AvifMeta,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> bool {
    if meta.primary_item_id > 0 {
        // Illegal to have multiple pitm boxes, bail out
        diag_printf!(diag, "Multiple boxes of unique Box[pitm] found");
        return false;
    }

    let mut s = AvifROStream::start(raw, diag, "Box[pitm]");

    let mut version = 0u8;
    avif_check!(s.read_version_and_flags(Some(&mut version), None));

    if version == 0 {
        let mut tmp16 = 0u16;
        avif_check!(s.read_u16(&mut tmp16)); // unsigned int(16) item_ID;
        meta.primary_item_id = tmp16 as u32;
    } else {
        avif_check!(s.read_u32(&mut meta.primary_item_id)); // unsigned int(32) item_ID;
    }
    true
}

/// Translation of `avifParseItemDataBox()`.
fn avif_parse_item_data_box(
    meta: &mut AvifMeta,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> bool {
    // Check to see if we've already seen an idat box for this meta box. If so, bail out
    if !meta.idat.is_empty() {
        diag_printf!(diag, "Meta box contains multiple idat boxes");
        return false;
    }
    if raw.is_empty() {
        diag_printf!(diag, "idat box has a length of 0");
        return false;
    }

    if avif_rw_data_set(&mut meta.idat, raw) != AvifResult::Ok {
        return false;
    }
    true
}

/// Translation of `avifParseItemPropertiesBox()`.
fn avif_parse_item_properties_box(
    meta: &mut AvifMeta,
    raw_offset: u64,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    let mut s = AvifROStream::start(raw, diag, "Box[iprp]");

    let mut ipco_header = AvifBoxHeader::default();
    avif_checkerr!(
        s.read_box_header(&mut ipco_header),
        AvifResult::BmffParseFailed
    );
    if &ipco_header.type_ != b"ipco" {
        diag_printf!(
            diag,
            "Failed to find Box[ipco] as the first box in Box[iprp]"
        );
        return AvifResult::BmffParseFailed;
    }

    // Read all item properties inside of ItemPropertyContainerBox
    avif_checkres!(avif_parse_item_property_container_box(
        &mut meta.properties,
        raw_offset.wrapping_add(s.offset() as u64),
        &s.current()[..ipco_header.size],
        diag
    ));
    avif_checkerr!(s.skip(ipco_header.size), AvifResult::BmffParseFailed);

    let mut version_and_flags_seen = [0u32; MAX_IPMA_VERSION_AND_FLAGS_SEEN];
    let mut version_and_flags_seen_count = 0usize;

    // Now read all ItemPropertyAssociation until the end of the box, and make associations
    while s.has_bytes_left(1) {
        let mut ipma_header = AvifBoxHeader::default();
        avif_checkerr!(
            s.read_box_header(&mut ipma_header),
            AvifResult::BmffParseFailed
        );

        if &ipma_header.type_ == b"ipma" {
            let mut version_and_flags = 0u32;
            avif_checkres!(avif_parse_item_property_association(
                meta,
                &s.current()[..ipma_header.size],
                diag,
                &mut version_and_flags
            ));
            for &seen in &version_and_flags_seen[..version_and_flags_seen_count] {
                if seen == version_and_flags {
                    // BMFF (ISO/IEC 14496-12:2022) 8.11.14.1 - There shall be at most one
                    // ItemPropertyAssociationBox with a given pair of values of version and
                    // flags.
                    diag_printf!(
                        diag,
                        "Multiple Box[ipma] with a given pair of values of version and flags. See BMFF (ISO/IEC 14496-12:2022) 8.11.14.1"
                    );
                    return AvifResult::BmffParseFailed;
                }
            }
            if version_and_flags_seen_count == MAX_IPMA_VERSION_AND_FLAGS_SEEN {
                diag_printf!(
                    diag,
                    "Exceeded possible count of unique ipma version and flags tuples"
                );
                return AvifResult::BmffParseFailed;
            }
            version_and_flags_seen[version_and_flags_seen_count] = version_and_flags;
            version_and_flags_seen_count += 1;
        } else {
            // These must all be type ipma
            diag_printf!(diag, "Box[iprp] contains a box that isn't type 'ipma'");
            return AvifResult::BmffParseFailed;
        }

        avif_checkerr!(s.skip(ipma_header.size), AvifResult::BmffParseFailed);
    }
    AvifResult::Ok
}

/// Translation of `avifParseItemInfoEntry()`.
fn avif_parse_item_info_entry(
    meta: &mut AvifMeta,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    // Section 8.11.6.2 of ISO/IEC 14496-12.
    let mut s = AvifROStream::start(raw, diag, "Box[infe]");

    let mut version = 0u8;
    let mut flags = 0u32;
    avif_checkerr!(
        s.read_version_and_flags(Some(&mut version), Some(&mut flags)),
        AvifResult::BmffParseFailed
    );
    // Version 2+ is required for item_type
    if version != 2 && version != 3 {
        diag_printf!(
            s.diag,
            "{}: Expecting box version 2 or 3, got version {}",
            s.diag_context,
            version
        );
        return AvifResult::BmffParseFailed;
    }
    // TODO: check flags. ISO/IEC 23008-12:2017, Section 9.2 says:
    //   The flags field of ItemInfoEntry with version greater than or equal to 2 is specified as
    //   follows:
    //
    //   (flags & 1) equal to 1 indicates that the item is not intended to be a part of the
    //   presentation. For example, when (flags & 1) is equal to 1 for an image item, the image
    //   item should not be displayed.
    //   (flags & 1) equal to 0 indicates that the item is intended to be a part of the
    //   presentation.
    //
    // See also Section 6.4.2.

    let mut item_id = 0u32;
    if version == 2 {
        let mut tmp = 0u16;
        avif_checkerr!(s.read_u16(&mut tmp), AvifResult::BmffParseFailed); // unsigned int(16) item_ID;
        item_id = tmp as u32;
    } else {
        avif_assert_or_return!(version == 3);
        avif_checkerr!(s.read_u32(&mut item_id), AvifResult::BmffParseFailed); // unsigned int(32) item_ID;
    }
    avif_checkres!(avif_check_item_id("infe", item_id, diag));
    let mut item_protection_index = 0u16;
    avif_checkerr!(
        s.read_u16(&mut item_protection_index),
        AvifResult::BmffParseFailed
    ); // unsigned int(16) item_protection_index;
    let mut item_type = [0u8; 4];
    avif_checkerr!(s.read(&mut item_type), AvifResult::BmffParseFailed); // unsigned int(32) item_type;
    avif_checkerr!(s.read_string(None, 0), AvifResult::BmffParseFailed); // utf8string item_name; (skipped)
    let mut content_type = AvifContentType::default();
    if &item_type == b"mime" {
        avif_checkerr!(
            s.read_string(Some(&mut content_type.content_type), CONTENTTYPE_SIZE),
            AvifResult::BmffParseFailed
        ); // utf8string content_type;
           // utf8string content_encoding; //optional
    } else {
        // if (item_type == 'uri ') {
        //  utf8string item_uri_type;
        // }
        content_type = AvifContentType::default();
    }

    let mut item = 0usize;
    avif_checkres!(avif_meta_find_or_create_item(meta, item_id, &mut item));

    meta.items[item].type_ = item_type;
    meta.items[item].content_type = content_type;
    AvifResult::Ok
}

/// Translation of `avifParseItemInfoBox()`.
fn avif_parse_item_info_box(
    meta: &mut AvifMeta,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    let mut s = AvifROStream::start(raw, diag, "Box[iinf]");

    let mut version = 0u8;
    avif_checkerr!(
        s.read_version_and_flags(Some(&mut version), None),
        AvifResult::BmffParseFailed
    );
    let mut entry_count = 0u32;
    if version == 0 {
        let mut tmp = 0u16;
        avif_checkerr!(s.read_u16(&mut tmp), AvifResult::BmffParseFailed); // unsigned int(16) entry_count;
        entry_count = tmp as u32;
    } else if version == 1 {
        avif_checkerr!(s.read_u32(&mut entry_count), AvifResult::BmffParseFailed);
    // unsigned int(32) entry_count;
    } else {
        diag_printf!(diag, "Box[iinf] has an unsupported version {}", version);
        return AvifResult::BmffParseFailed;
    }

    for _ in 0..entry_count {
        let mut infe_header = AvifBoxHeader::default();
        avif_checkerr!(
            s.read_box_header(&mut infe_header),
            AvifResult::BmffParseFailed
        );

        if &infe_header.type_ == b"infe" {
            avif_checkres!(avif_parse_item_info_entry(
                meta,
                &s.current()[..infe_header.size],
                diag
            ));
        } else {
            // These must all be type infe
            diag_printf!(diag, "Box[iinf] contains a box that isn't type 'infe'");
            return AvifResult::BmffParseFailed;
        }

        avif_checkerr!(s.skip(infe_header.size), AvifResult::BmffParseFailed);
    }

    AvifResult::Ok
}

/// Translation of `avifParseItemReferenceBox()`.
fn avif_parse_item_reference_box(
    meta: &mut AvifMeta,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    let mut s = AvifROStream::start(raw, diag, "Box[iref]");

    let mut version = 0u8;
    avif_checkerr!(
        s.read_version_and_flags(Some(&mut version), None),
        AvifResult::BmffParseFailed
    );
    if version > 1 {
        // iref versions > 1 are not supported. Skip it.
        return AvifResult::Ok;
    }

    while s.has_bytes_left(1) {
        let mut iref_header = AvifBoxHeader::default();
        avif_checkerr!(
            s.read_box_header(&mut iref_header),
            AvifResult::BmffParseFailed
        );

        let mut from_id = 0u32;
        if version == 0 {
            let mut tmp = 0u16;
            avif_checkerr!(s.read_u16(&mut tmp), AvifResult::BmffParseFailed); // unsigned int(16) from_item_ID;
            from_id = tmp as u32;
        } else {
            // version == 1
            avif_checkerr!(s.read_u32(&mut from_id), AvifResult::BmffParseFailed);
            // unsigned int(32) from_item_ID;
        }
        // ISO 14496-12 section 8.11.12.1: "index values start at 1"
        avif_checkres!(avif_check_item_id("iref", from_id, diag));

        let mut item = 0usize;
        avif_checkres!(avif_meta_find_or_create_item(meta, from_id, &mut item));
        if &iref_header.type_ == b"dimg" {
            if meta.items[item].has_dimg_from {
                // ISO/IEC 23008-12 (HEIF) 6.6.1: The number of SingleItemTypeReferenceBoxes with the box type 'dimg'
                // and with the same value of from_item_ID shall not be greater than 1.
                diag_printf!(
                    diag,
                    "Box[iinf] contains duplicate boxes of type 'dimg' with the same from_item_ID value {}",
                    from_id
                );
                return AvifResult::BmffParseFailed;
            }
            meta.items[item].has_dimg_from = true;
        }

        let mut reference_count = 0u16;
        avif_checkerr!(
            s.read_u16(&mut reference_count),
            AvifResult::BmffParseFailed
        ); // unsigned int(16) reference_count;

        for ref_index in 0..reference_count {
            let mut to_id = 0u32;
            if version == 0 {
                let mut tmp = 0u16;
                avif_checkerr!(s.read_u16(&mut tmp), AvifResult::BmffParseFailed); // unsigned int(16) to_item_ID;
                to_id = tmp as u32;
            } else {
                // version == 1
                avif_checkerr!(s.read_u32(&mut to_id), AvifResult::BmffParseFailed);
                // unsigned int(32) to_item_ID;
            }
            avif_checkres!(avif_check_item_id("iref", to_id, diag));

            // Read this reference as "{fromID} is a {irefType} for {toID}"
            match &iref_header.type_ {
                b"thmb" => meta.items[item].thumbnail_for_id = to_id,
                b"auxl" => meta.items[item].aux_for_id = to_id,
                b"cdsc" => meta.items[item].desc_for_id = to_id,
                b"dimg" => {
                    // derived images refer in the opposite direction
                    let mut dimg = 0usize;
                    avif_checkres!(avif_meta_find_or_create_item(meta, to_id, &mut dimg));

                    // Section 8.11.12.1 of ISO/IEC 14496-12:
                    //   The items linked to are then represented by an array of to_item_IDs;
                    //   within a given array, a given value shall occur at most once.
                    avif_checkerr!(
                        meta.items[dimg].dimg_for_id != from_id,
                        AvifResult::InvalidImageGrid
                    );
                    // A given value may occur within multiple arrays but this is not supported by libavif.
                    avif_checkerr!(
                        meta.items[dimg].dimg_for_id == 0,
                        AvifResult::NotImplemented
                    );
                    meta.items[dimg].dimg_for_id = from_id;
                    meta.items[dimg].dimg_idx = ref_index as u32;
                }
                b"prem" => meta.items[item].prem_by_id = to_id,
                _ => {}
            }
        }
    }

    AvifResult::Ok
}

/// Translation of `avifParseMetaBox()`.
fn avif_parse_meta_box(
    meta: &mut AvifMeta,
    raw_offset: u64,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    let mut s = AvifROStream::start(raw, diag, "Box[meta]");

    let mut version = 0u8;
    let mut flags = 0u32;
    avif_checkerr!(
        s.read_version_and_flags(Some(&mut version), Some(&mut flags)),
        AvifResult::BmffParseFailed
    );

    if version != 0 {
        diag_printf!(
            diag,
            "Box[meta]: Expecting box version 0, got version {}",
            version
        );
        return AvifResult::BmffParseFailed;
    }

    meta.idat_id = meta.idat_id.wrapping_add(1); // for tracking idat

    let mut first_box = true;
    let mut unique_box_flags = 0u32;
    while s.has_bytes_left(1) {
        let mut header = AvifBoxHeader::default();
        avif_checkerr!(s.read_box_header(&mut header), AvifResult::BmffParseFailed);
        let body = &s.current()[..header.size];

        if first_box {
            if &header.type_ == b"hdlr" {
                avif_checkerr!(
                    avif_parse_handler_box(body, diag),
                    AvifResult::BmffParseFailed
                );
                first_box = false;
            } else {
                // hdlr must be the first box!
                diag_printf!(
                    diag,
                    "Box[meta] does not have a Box[hdlr] as its first child box"
                );
                return AvifResult::BmffParseFailed;
            }
        } else if &header.type_ == b"hdlr" {
            diag_printf!(
                diag,
                "Box[meta] contains a duplicate unique box of type 'hdlr'"
            );
            return AvifResult::BmffParseFailed;
        } else if &header.type_ == b"iloc" {
            avif_checkerr!(
                unique_box_seen(&mut unique_box_flags, 0, "meta", "iloc", diag),
                AvifResult::BmffParseFailed
            );
            avif_checkres!(avif_parse_item_location_box(meta, body, diag));
        } else if &header.type_ == b"pitm" {
            avif_checkerr!(
                unique_box_seen(&mut unique_box_flags, 1, "meta", "pitm", diag),
                AvifResult::BmffParseFailed
            );
            avif_checkerr!(
                avif_parse_primary_item_box(meta, body, diag),
                AvifResult::BmffParseFailed
            );
        } else if &header.type_ == b"idat" {
            avif_checkerr!(
                unique_box_seen(&mut unique_box_flags, 2, "meta", "idat", diag),
                AvifResult::BmffParseFailed
            );
            avif_checkerr!(
                avif_parse_item_data_box(meta, body, diag),
                AvifResult::BmffParseFailed
            );
        } else if &header.type_ == b"iprp" {
            avif_checkerr!(
                unique_box_seen(&mut unique_box_flags, 3, "meta", "iprp", diag),
                AvifResult::BmffParseFailed
            );
            avif_checkres!(avif_parse_item_properties_box(
                meta,
                raw_offset.wrapping_add(s.offset() as u64),
                body,
                diag
            ));
        } else if &header.type_ == b"iinf" {
            avif_checkerr!(
                unique_box_seen(&mut unique_box_flags, 4, "meta", "iinf", diag),
                AvifResult::BmffParseFailed
            );
            avif_checkres!(avif_parse_item_info_box(meta, body, diag));
        } else if &header.type_ == b"iref" {
            avif_checkerr!(
                unique_box_seen(&mut unique_box_flags, 5, "meta", "iref", diag),
                AvifResult::BmffParseFailed
            );
            avif_checkres!(avif_parse_item_reference_box(meta, body, diag));
        }

        avif_checkerr!(s.skip(header.size), AvifResult::BmffParseFailed);
    }
    if first_box {
        // The meta box must not be empty (it must contain at least a hdlr box)
        diag_printf!(diag, "Box[meta] has no child boxes");
        return AvifResult::BmffParseFailed;
    }
    AvifResult::Ok
}

/// Translation of `avifParseTrackHeaderBox()`.
fn avif_parse_track_header_box(
    track: &mut AvifTrack,
    raw: &[u8],
    image_size_limit: u32,
    image_dimension_limit: u32,
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[tkhd]");

    let mut version = 0u8;
    avif_check!(s.read_version_and_flags(Some(&mut version), None));

    let mut ignored32 = 0u32;
    let mut track_id = 0u32;
    let mut ignored64 = 0u64;
    if version == 1 {
        avif_check!(s.read_u64(&mut ignored64)); // unsigned int(64) creation_time;
        avif_check!(s.read_u64(&mut ignored64)); // unsigned int(64) modification_time;
        avif_check!(s.read_u32(&mut track_id)); // unsigned int(32) track_ID;
        avif_check!(s.read_u32(&mut ignored32)); // const unsigned int(32) reserved = 0;
        avif_check!(s.read_u64(&mut track.track_duration)); // unsigned int(64) duration;
    } else if version == 0 {
        let mut track_duration = 0u32;
        avif_check!(s.read_u32(&mut ignored32)); // unsigned int(32) creation_time;
        avif_check!(s.read_u32(&mut ignored32)); // unsigned int(32) modification_time;
        avif_check!(s.read_u32(&mut track_id)); // unsigned int(32) track_ID;
        avif_check!(s.read_u32(&mut ignored32)); // const unsigned int(32) reserved = 0;
        avif_check!(s.read_u32(&mut track_duration)); // unsigned int(32) duration;
        track.track_duration = if track_duration == AVIF_INDEFINITE_DURATION32 {
            AVIF_INDEFINITE_DURATION64
        } else {
            track_duration as u64
        };
    } else {
        // Unsupported version
        diag_printf!(diag, "Box[tkhd] has an unsupported version [{}]", version);
        return false;
    }

    // Skipping the following 52 bytes here:
    // ------------------------------------
    // const unsigned int(32)[2] reserved = 0;
    // template int(16) layer = 0;
    // template int(16) alternate_group = 0;
    // template int(16) volume = {if track_is_audio 0x0100 else 0};
    // const unsigned int(16) reserved = 0;
    // template int(32)[9] matrix= { 0x00010000,0,0,0,0x00010000,0,0,0,0x40000000 }; // unity matrix
    avif_check!(s.skip(52));

    let mut width = 0u32;
    let mut height = 0u32;
    avif_check!(s.read_u32(&mut width)); // unsigned int(32) width;
    avif_check!(s.read_u32(&mut height)); // unsigned int(32) height;
    track.width = width >> 16;
    track.height = height >> 16;

    if (track.width == 0) || (track.height == 0) {
        diag_printf!(
            diag,
            "Track ID [{}] has an invalid size [{}x{}]",
            track.id,
            track.width,
            track.height
        );
        return false;
    }
    if avif_dimensions_too_large(
        track.width,
        track.height,
        image_size_limit,
        image_dimension_limit,
    ) {
        diag_printf!(
            diag,
            "Track ID [{}] dimensions are too large [{}x{}]",
            track.id,
            track.width,
            track.height
        );
        return false;
    }

    // TODO: support scaling based on width/height track header info?

    track.id = track_id;
    true
}

/// Translation of `avifParseMediaHeaderBox()`.
fn avif_parse_media_header_box(
    track: &mut AvifTrack,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[mdhd]");

    let mut version = 0u8;
    avif_check!(s.read_version_and_flags(Some(&mut version), None));

    let mut ignored32 = 0u32;
    let mut media_timescale = 0u32;
    let mut media_duration32 = 0u32;
    let mut ignored64 = 0u64;
    let mut media_duration64 = 0u64;
    if version == 1 {
        avif_check!(s.read_u64(&mut ignored64)); // unsigned int(64) creation_time;
        avif_check!(s.read_u64(&mut ignored64)); // unsigned int(64) modification_time;
        avif_check!(s.read_u32(&mut media_timescale)); // unsigned int(32) timescale;
        avif_check!(s.read_u64(&mut media_duration64)); // unsigned int(64) duration;
        track.media_duration = media_duration64;
    } else if version == 0 {
        avif_check!(s.read_u32(&mut ignored32)); // unsigned int(32) creation_time;
        avif_check!(s.read_u32(&mut ignored32)); // unsigned int(32) modification_time;
        avif_check!(s.read_u32(&mut media_timescale)); // unsigned int(32) timescale;
        avif_check!(s.read_u32(&mut media_duration32)); // unsigned int(32) duration;
        track.media_duration = media_duration32 as u64;
    } else {
        // Unsupported version
        diag_printf!(diag, "Box[mdhd] has an unsupported version [{}]", version);
        return false;
    }

    track.media_timescale = media_timescale;
    true
}

/// Translation of `avifParseChunkOffsetBox()`.
fn avif_parse_chunk_offset_box(
    sample_table: &mut AvifSampleTable,
    large_offsets: bool,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    let mut s = AvifROStream::start(
        raw,
        diag,
        if large_offsets {
            "Box[co64]"
        } else {
            "Box[stco]"
        },
    );

    avif_checkerr!(s.read_and_enforce_version(0), AvifResult::BmffParseFailed);

    let mut entry_count = 0u32;
    avif_checkerr!(s.read_u32(&mut entry_count), AvifResult::BmffParseFailed); // unsigned int(32) entry_count;
    for _ in 0..entry_count {
        let mut offset = 0u64;
        if large_offsets {
            avif_checkerr!(s.read_u64(&mut offset), AvifResult::BmffParseFailed);
        // unsigned int(64) chunk_offset;
        } else {
            let mut offset32 = 0u32;
            avif_checkerr!(s.read_u32(&mut offset32), AvifResult::BmffParseFailed); // unsigned int(32) chunk_offset;
            offset = offset32 as u64;
        }

        avif_checkerr!(
            try_push(&mut sample_table.chunks, offset),
            AvifResult::OutOfMemory
        );
    }
    AvifResult::Ok
}

/// Translation of `avifParseSampleToChunkBox()`.
fn avif_parse_sample_to_chunk_box(
    sample_table: &mut AvifSampleTable,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    let mut s = AvifROStream::start(raw, diag, "Box[stsc]");

    avif_checkerr!(s.read_and_enforce_version(0), AvifResult::BmffParseFailed);

    let mut entry_count = 0u32;
    avif_checkerr!(s.read_u32(&mut entry_count), AvifResult::BmffParseFailed); // unsigned int(32) entry_count;
    let mut prev_first_chunk = 0u32;
    for i in 0..entry_count {
        avif_checkerr!(
            try_push(
                &mut sample_table.sample_to_chunks,
                AvifSampleTableSampleToChunk::default()
            ),
            AvifResult::OutOfMemory
        );
        let index = sample_table.sample_to_chunks.len() - 1;
        let sample_to_chunk = &mut sample_table.sample_to_chunks[index];
        avif_checkerr!(
            s.read_u32(&mut sample_to_chunk.first_chunk),
            AvifResult::BmffParseFailed
        ); // unsigned int(32) first_chunk;
        avif_checkerr!(
            s.read_u32(&mut sample_to_chunk.samples_per_chunk),
            AvifResult::BmffParseFailed
        ); // unsigned int(32) samples_per_chunk;
        avif_checkerr!(
            s.read_u32(&mut sample_to_chunk.sample_description_index),
            AvifResult::BmffParseFailed
        ); // unsigned int(32) sample_description_index;
           // The first_chunk fields should start with 1 and be strictly increasing.
        if i == 0 {
            if sample_to_chunk.first_chunk != 1 {
                diag_printf!(
                    diag,
                    "Box[stsc] does not begin with chunk 1 [{}]",
                    sample_to_chunk.first_chunk
                );
                return AvifResult::BmffParseFailed;
            }
        } else if sample_to_chunk.first_chunk <= prev_first_chunk {
            diag_printf!(diag, "Box[stsc] chunks are not strictly increasing");
            return AvifResult::BmffParseFailed;
        }
        prev_first_chunk = sample_to_chunk.first_chunk;
    }
    AvifResult::Ok
}

/// Translation of `avifParseSampleSizeBox()`.
fn avif_parse_sample_size_box(
    sample_table: &mut AvifSampleTable,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    let mut s = AvifROStream::start(raw, diag, "Box[stsz]");

    avif_checkerr!(s.read_and_enforce_version(0), AvifResult::BmffParseFailed);

    let mut all_samples_size = 0u32;
    let mut sample_count = 0u32;
    avif_checkerr!(
        s.read_u32(&mut all_samples_size),
        AvifResult::BmffParseFailed
    ); // unsigned int(32) sample_size;
    avif_checkerr!(s.read_u32(&mut sample_count), AvifResult::BmffParseFailed); // unsigned int(32) sample_count;

    if all_samples_size > 0 {
        sample_table.all_samples_size = all_samples_size;
    } else {
        for _ in 0..sample_count {
            avif_checkerr!(
                try_push(&mut sample_table.sample_sizes, 0),
                AvifResult::OutOfMemory
            );
            let index = sample_table.sample_sizes.len() - 1;
            avif_checkerr!(
                s.read_u32(&mut sample_table.sample_sizes[index]),
                AvifResult::BmffParseFailed
            ); // unsigned int(32) entry_size;
        }
    }
    AvifResult::Ok
}

/// Translation of `avifParseSyncSampleBox()`.
fn avif_parse_sync_sample_box(
    sample_table: &mut AvifSampleTable,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    let mut s = AvifROStream::start(raw, diag, "Box[stss]");

    avif_checkerr!(s.read_and_enforce_version(0), AvifResult::BmffParseFailed);

    let mut entry_count = 0u32;
    avif_checkerr!(s.read_u32(&mut entry_count), AvifResult::BmffParseFailed); // unsigned int(32) entry_count;

    for _ in 0..entry_count {
        let mut sample_number = 0u32;
        avif_checkerr!(s.read_u32(&mut sample_number), AvifResult::BmffParseFailed); // unsigned int(32) sample_number;
        avif_checkerr!(
            try_push(&mut sample_table.sync_samples, sample_number),
            AvifResult::OutOfMemory
        );
    }
    AvifResult::Ok
}

/// Translation of `avifParseTimeToSampleBox()`.
fn avif_parse_time_to_sample_box(
    sample_table: &mut AvifSampleTable,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    let mut s = AvifROStream::start(raw, diag, "Box[stts]");

    avif_checkerr!(s.read_and_enforce_version(0), AvifResult::BmffParseFailed);

    let mut entry_count = 0u32;
    avif_checkerr!(s.read_u32(&mut entry_count), AvifResult::BmffParseFailed); // unsigned int(32) entry_count;

    for _ in 0..entry_count {
        avif_checkerr!(
            try_push(
                &mut sample_table.time_to_samples,
                AvifSampleTableTimeToSample::default()
            ),
            AvifResult::OutOfMemory
        );
        let index = sample_table.time_to_samples.len() - 1;
        let time_to_sample = &mut sample_table.time_to_samples[index];
        avif_checkerr!(
            s.read_u32(&mut time_to_sample.sample_count),
            AvifResult::BmffParseFailed
        ); // unsigned int(32) sample_count;
        avif_checkerr!(
            s.read_u32(&mut time_to_sample.sample_delta),
            AvifResult::BmffParseFailed
        ); // unsigned int(32) sample_delta;
    }
    AvifResult::Ok
}

/// Translation of `avifParseSampleDescriptionBox()`.
fn avif_parse_sample_description_box(
    sample_table: &mut AvifSampleTable,
    raw_offset: u64,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    let mut s = AvifROStream::start(raw, diag, "Box[stsd]");

    let mut version = 0u8;
    avif_checkerr!(
        s.read_version_and_flags(Some(&mut version), None),
        AvifResult::BmffParseFailed
    );

    // Section 8.5.2.3 of ISO/IEC 14496-12:
    //   version is set to zero. A version number of 1 shall be treated as a version of 0.
    if version != 0 && version != 1 {
        diag_printf!(
            diag,
            "Box[stsd]: Expecting box version 0 or 1, got version {}",
            version
        );
        return AvifResult::BmffParseFailed;
    }

    let mut entry_count = 0u32;
    avif_checkerr!(s.read_u32(&mut entry_count), AvifResult::BmffParseFailed); // unsigned int(32) entry_count;

    for _ in 0..entry_count {
        let mut sample_entry_header = AvifBoxHeader::default();
        avif_checkerr!(
            s.read_box_header(&mut sample_entry_header),
            AvifResult::BmffParseFailed
        );

        let description = AvifSampleDescription {
            format: sample_entry_header.type_,
            properties: Vec::new(),
        };
        avif_checkerr!(
            try_push(&mut sample_table.sample_descriptions, description),
            AvifResult::OutOfMemory
        );
        let index = sample_table.sample_descriptions.len() - 1;
        let description = &mut sample_table.sample_descriptions[index];
        let sample_entry_bytes = sample_entry_header.size;
        if avif_get_codec_type(&description.format) != AvifCodecType::Unknown {
            if sample_entry_bytes < VISUALSAMPLEENTRY_SIZE {
                diag_printf!(diag, "Not enough bytes to parse VisualSampleEntry");
                return AvifResult::BmffParseFailed;
            }
            avif_checkres!(avif_parse_item_property_container_box(
                &mut description.properties,
                raw_offset
                    .wrapping_add(s.offset() as u64)
                    .wrapping_add(VISUALSAMPLEENTRY_SIZE as u64),
                &s.current()[VISUALSAMPLEENTRY_SIZE..sample_entry_bytes],
                diag
            ));
        }

        avif_checkerr!(s.skip(sample_entry_bytes), AvifResult::BmffParseFailed);
    }
    AvifResult::Ok
}

/// Translation of `avifParseSampleTableBox()`.
fn avif_parse_sample_table_box(
    track: &mut AvifTrack,
    raw_offset: u64,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    if track.sample_table.is_some() {
        // A TrackBox may only have one SampleTable
        diag_printf!(diag, "Duplicate Box[stbl] for a single track detected");
        return AvifResult::BmffParseFailed;
    }
    let sample_table = track.sample_table.insert(Box::default());

    let mut s = AvifROStream::start(raw, diag, "Box[stbl]");

    while s.has_bytes_left(1) {
        let mut header = AvifBoxHeader::default();
        avif_checkerr!(s.read_box_header(&mut header), AvifResult::BmffParseFailed);
        let body = &s.current()[..header.size];

        match &header.type_ {
            b"stco" => {
                avif_checkres!(avif_parse_chunk_offset_box(sample_table, false, body, diag));
            }
            b"co64" => {
                avif_checkres!(avif_parse_chunk_offset_box(sample_table, true, body, diag));
            }
            b"stsc" => {
                avif_checkres!(avif_parse_sample_to_chunk_box(sample_table, body, diag));
            }
            b"stsz" => {
                avif_checkres!(avif_parse_sample_size_box(sample_table, body, diag));
            }
            b"stss" => {
                avif_checkres!(avif_parse_sync_sample_box(sample_table, body, diag));
            }
            b"stts" => {
                avif_checkres!(avif_parse_time_to_sample_box(sample_table, body, diag));
            }
            b"stsd" => {
                avif_checkres!(avif_parse_sample_description_box(
                    sample_table,
                    raw_offset.wrapping_add(s.offset() as u64),
                    body,
                    diag
                ));
            }
            _ => {}
        }

        avif_checkerr!(s.skip(header.size), AvifResult::BmffParseFailed);
    }
    AvifResult::Ok
}

/// Translation of `avifParseMediaInformationBox()`.
fn avif_parse_media_information_box(
    track: &mut AvifTrack,
    raw_offset: u64,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    let mut s = AvifROStream::start(raw, diag, "Box[minf]");

    while s.has_bytes_left(1) {
        let mut header = AvifBoxHeader::default();
        avif_checkerr!(s.read_box_header(&mut header), AvifResult::BmffParseFailed);

        if &header.type_ == b"stbl" {
            avif_checkres!(avif_parse_sample_table_box(
                track,
                raw_offset.wrapping_add(s.offset() as u64),
                &s.current()[..header.size],
                diag
            ));
        }

        avif_checkerr!(s.skip(header.size), AvifResult::BmffParseFailed);
    }
    AvifResult::Ok
}

/// Translation of `avifParseMediaBox()`.
fn avif_parse_media_box(
    track: &mut AvifTrack,
    raw_offset: u64,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    let mut s = AvifROStream::start(raw, diag, "Box[mdia]");

    while s.has_bytes_left(1) {
        let mut header = AvifBoxHeader::default();
        avif_checkerr!(s.read_box_header(&mut header), AvifResult::BmffParseFailed);
        let body = &s.current()[..header.size];

        if &header.type_ == b"mdhd" {
            avif_checkerr!(
                avif_parse_media_header_box(track, body, diag),
                AvifResult::BmffParseFailed
            );
        } else if &header.type_ == b"minf" {
            avif_checkres!(avif_parse_media_information_box(
                track,
                raw_offset.wrapping_add(s.offset() as u64),
                body,
                diag
            ));
        }

        avif_checkerr!(s.skip(header.size), AvifResult::BmffParseFailed);
    }
    AvifResult::Ok
}

/// Translation of `avifTrackReferenceBox()`.
fn avif_track_reference_box(
    track: &mut AvifTrack,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[tref]");

    while s.has_bytes_left(1) {
        let mut header = AvifBoxHeader::default();
        avif_check!(s.read_box_header(&mut header));

        if &header.type_ == b"auxl" {
            let mut to_id = 0u32;
            avif_check!(s.read_u32(&mut to_id)); // unsigned int(32) track_IDs[];
            avif_check!(s.skip(header.size.wrapping_sub(4))); // just take the first one
            track.aux_for_id = to_id;
        } else if &header.type_ == b"prem" {
            let mut by_id = 0u32;
            avif_check!(s.read_u32(&mut by_id)); // unsigned int(32) track_IDs[];
            avif_check!(s.skip(header.size.wrapping_sub(4))); // just take the first one
            track.prem_by_id = by_id;
        } else {
            avif_check!(s.skip(header.size));
        }
    }
    true
}

/// Translation of `avifParseEditListBox()`.
fn avif_parse_edit_list_box(
    track: &mut AvifTrack,
    raw: &[u8],
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[elst]");

    let mut version = 0u8;
    let mut flags = 0u32;
    avif_check!(s.read_version_and_flags(Some(&mut version), Some(&mut flags)));

    if (flags & 1) == 0 {
        track.is_repeating = false;
        return true;
    }

    track.is_repeating = true;
    let mut entry_count = 0u32;
    avif_check!(s.read_u32(&mut entry_count)); // unsigned int(32) entry_count;
    if entry_count != 1 {
        diag_printf!(
            diag,
            "Box[elst] contains an entry_count != 1 [{}]",
            entry_count
        );
        return false;
    }

    if version == 1 {
        avif_check!(s.read_u64(&mut track.segment_duration)); // unsigned int(64) segment_duration;
    } else if version == 0 {
        let mut segment_duration = 0u32;
        avif_check!(s.read_u32(&mut segment_duration)); // unsigned int(32) segment_duration;
        track.segment_duration = segment_duration as u64;
    } else {
        // Unsupported version
        diag_printf!(diag, "Box[elst] has an unsupported version [{}]", version);
        return false;
    }
    if track.segment_duration == 0 {
        diag_printf!(diag, "Box[elst] Invalid value for segment_duration (0).");
        return false;
    }
    true
}

/// Translation of `avifParseEditBox()`.
fn avif_parse_edit_box(track: &mut AvifTrack, raw: &[u8], diag: Option<&AvifDiagnostics>) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[edts]");

    let mut elst_box_seen = false;
    while s.has_bytes_left(1) {
        let mut header = AvifBoxHeader::default();
        avif_check!(s.read_box_header(&mut header));

        if &header.type_ == b"elst" {
            if elst_box_seen {
                diag_printf!(diag, "More than one [elst] Box was found.");
                return false;
            }
            avif_check!(avif_parse_edit_list_box(
                track,
                &s.current()[..header.size],
                diag
            ));
            elst_box_seen = true;
        }
        avif_check!(s.skip(header.size));
    }
    if !elst_box_seen {
        diag_printf!(diag, "Box[edts] contains no [elst] Box.");
        return false;
    }
    true
}

/// Translation of `avifParseTrackBox()`.
fn avif_parse_track_box(
    data: &mut AvifDecoderData,
    raw_offset: u64,
    raw: &[u8],
    image_size_limit: u32,
    image_dimension_limit: u32,
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    let mut s = AvifROStream::start(raw, diag, "Box[trak]");

    let track = avif_decoder_data_create_track(data);
    avif_checkerr!(track.is_some(), AvifResult::OutOfMemory);
    let Some(track) = track else {
        return AvifResult::OutOfMemory;
    };
    let track = &mut data.tracks[track];

    let mut edts_box_seen = false;
    let mut tkhd_seen = false;
    while s.has_bytes_left(1) {
        let mut header = AvifBoxHeader::default();
        avif_checkerr!(s.read_box_header(&mut header), AvifResult::BmffParseFailed);
        let body = &s.current()[..header.size];

        if &header.type_ == b"tkhd" {
            if tkhd_seen {
                diag_printf!(
                    diag,
                    "Box[trak] contains a duplicate unique box of type 'tkhd'"
                );
                return AvifResult::BmffParseFailed;
            }
            avif_checkerr!(
                avif_parse_track_header_box(
                    track,
                    body,
                    image_size_limit,
                    image_dimension_limit,
                    diag
                ),
                AvifResult::BmffParseFailed
            );
            tkhd_seen = true;
        } else if &header.type_ == b"meta" {
            avif_checkres!(avif_parse_meta_box(
                &mut track.meta,
                raw_offset.wrapping_add(s.offset() as u64),
                body,
                diag
            ));
        } else if &header.type_ == b"mdia" {
            avif_checkres!(avif_parse_media_box(
                track,
                raw_offset.wrapping_add(s.offset() as u64),
                body,
                diag
            ));
        } else if &header.type_ == b"tref" {
            avif_checkerr!(
                avif_track_reference_box(track, body, diag),
                AvifResult::BmffParseFailed
            );
        } else if &header.type_ == b"edts" {
            if edts_box_seen {
                diag_printf!(
                    diag,
                    "Box[trak] contains a duplicate unique box of type 'edts'"
                );
                return AvifResult::BmffParseFailed;
            }
            avif_checkerr!(
                avif_parse_edit_box(track, body, diag),
                AvifResult::BmffParseFailed
            );
            edts_box_seen = true;
        }

        avif_checkerr!(s.skip(header.size), AvifResult::BmffParseFailed);
    }
    if !tkhd_seen {
        diag_printf!(diag, "Box[trak] does not contain a mandatory [tkhd] box");
        return AvifResult::BmffParseFailed;
    }
    if !edts_box_seen {
        track.repetition_count = AVIF_REPETITION_COUNT_UNKNOWN;
    } else if track.is_repeating {
        if track.track_duration == AVIF_INDEFINITE_DURATION64 {
            // If isRepeating is true and the track duration is unknown/indefinite, then set the repetition count to infinite
            // (Section 9.6.1 of ISO/IEC 23008-12 Part 12).
            track.repetition_count = AVIF_REPETITION_COUNT_INFINITE;
        } else {
            // Section 9.6.1. of ISO/IEC 23008-12 Part 12: 1, the entire edit list is repeated a sufficient number of times to
            // equal the track duration.
            //
            // Since libavif uses repetitionCount (which is 0-based), we subtract the value by 1 to derive the number of
            // repetitions.
            avif_assert_or_return!(track.segment_duration != 0);
            // We specifically check for trackDuration == 0 here and not when it is actually read in order to accept files which
            // inadvertently has a trackDuration of 0 without any edit lists.
            if track.track_duration == 0 {
                diag_printf!(diag, "Invalid track duration 0.");
                return AvifResult::BmffParseFailed;
            }
            let repetition_count = (track.track_duration / track.segment_duration)
                .wrapping_add((track.track_duration % track.segment_duration != 0) as u64)
                .wrapping_sub(1);
            if repetition_count > i32::MAX as u64 {
                // repetitionCount does not fit in an integer and hence it is
                // likely to be a very large value. So, we just set it to
                // infinite.
                track.repetition_count = AVIF_REPETITION_COUNT_INFINITE;
            } else {
                track.repetition_count = repetition_count as i32;
            }
        }
    } else {
        track.repetition_count = 0;
    }

    AvifResult::Ok
}

/// Translation of `avifParseMovieBox()`.
fn avif_parse_movie_box(
    data: &mut AvifDecoderData,
    raw_offset: u64,
    raw: &[u8],
    image_size_limit: u32,
    image_dimension_limit: u32,
    diag: Option<&AvifDiagnostics>,
) -> AvifResult {
    let mut s = AvifROStream::start(raw, diag, "Box[moov]");

    let mut has_trak = false;
    while s.has_bytes_left(1) {
        let mut header = AvifBoxHeader::default();
        avif_checkerr!(s.read_box_header(&mut header), AvifResult::BmffParseFailed);

        if &header.type_ == b"trak" {
            avif_checkres!(avif_parse_track_box(
                data,
                raw_offset.wrapping_add(s.offset() as u64),
                &s.current()[..header.size],
                image_size_limit,
                image_dimension_limit,
                diag
            ));
            has_trak = true;
        }

        avif_checkerr!(s.skip(header.size), AvifResult::BmffParseFailed);
    }
    if !has_trak {
        diag_printf!(diag, "moov box does not contain any tracks");
        return AvifResult::BmffParseFailed;
    }
    AvifResult::Ok
}

/// Translation of `avifParseFileTypeBox()`.
fn avif_parse_file_type_box<'a>(
    ftyp: &mut AvifFileType<'a>,
    raw: &'a [u8],
    diag: Option<&AvifDiagnostics>,
) -> bool {
    let mut s = AvifROStream::start(raw, diag, "Box[ftyp]");

    avif_check!(s.read(&mut ftyp.major_brand));
    avif_check!(s.read_u32(&mut ftyp.minor_version));

    let compatible_brands_bytes = s.remaining_bytes();
    if (compatible_brands_bytes % 4) != 0 {
        diag_printf!(
            diag,
            "Box[ftyp] contains a compatible brands section that isn't divisible by 4 [{}]",
            compatible_brands_bytes
        );
        return false;
    }
    ftyp.compatible_brands = s.current();
    avif_check!(s.skip(compatible_brands_bytes));
    ftyp.compatible_brands_count = (compatible_brands_bytes as i32) / 4;

    true
}

/// Translation of `avifParse()`.
fn avif_parse(decoder: &mut AvifDecoder, io: &mut dyn AvifIo) -> AvifResult {
    // Note: this top-level function is the only avifParse*() function that returns avifResult instead of avifBool.
    // Be sure to use AVIF_CHECKERR() in this function with an explicit error result instead of simply using AVIF_CHECK().

    let mut parse_offset: u64 = 0;
    let Some(data) = decoder.data.as_deref_mut() else {
        return AvifResult::NoContent;
    };
    let diag = Some(&decoder.diag);
    let mut ftyp_seen = false;
    let mut meta_v0_seen = false;
    let mut moov_seen = false;
    let mut needs_meta_v0 = false;
    let mut needs_moov = false;

    loop {
        // Read just enough to get the next box header (a max of 32 bytes)
        if (io.size_hint() > 0) && (parse_offset > io.size_hint()) {
            return AvifResult::BmffParseFailed;
        }
        let header_contents = match io.read(0, parse_offset, 32) {
            Ok(b) => b.to_vec(),
            Err(read_result) => return read_result,
        };
        if header_contents.is_empty() {
            // If we got AVIF_RESULT_OK from the reader but received 0 bytes,
            // we've reached the end of the file with no errors. Hooray!
            break;
        }

        // Parse the header, and find out how many bytes it actually was
        let mut header_stream =
            AvifROStream::start(&header_contents, diag, "File-level box header");
        let mut header = AvifBoxHeader::default();
        avif_checkerr!(
            header_stream.read_box_header_partial(&mut header, /*topLevel=*/ true),
            AvifResult::BmffParseFailed
        );
        parse_offset += header_stream.offset as u64;
        avif_assert_or_return!(io.size_hint() == 0 || parse_offset <= io.size_hint());

        // Try to get the remainder of the box, if necessary
        let mut box_offset: u64 = 0;
        let mut box_contents: Vec<u8> = Vec::new();

        // TODO: reorg this code to only do these memcmps once each
        if &header.type_ == b"ftyp" || &header.type_ == b"meta" || &header.type_ == b"moov" {
            box_offset = parse_offset;
            let size_to_read = if header.is_size_zero_box {
                // The box body goes till the end of the file.
                if io.size_hint() != 0 && io.size_hint() - parse_offset < usize::MAX as u64 {
                    (io.size_hint() - parse_offset) as usize
                } else {
                    usize::MAX // This will get truncated. See the documentation of avifIOReadFunc.
                }
            } else {
                header.size
            };
            match io.read(0, parse_offset, size_to_read) {
                Ok(b) => {
                    if box_contents.try_reserve_exact(b.len()).is_err() {
                        return AvifResult::OutOfMemory;
                    }
                    box_contents.extend_from_slice(b);
                }
                Err(read_result) => return read_result,
            }
            if header.is_size_zero_box {
                header.size = box_contents.len();
            } else if box_contents.len() != header.size {
                // A truncated box, bail out
                return AvifResult::TruncatedData;
            }
        } else if header.is_size_zero_box {
            // An unknown top level box with size 0 was found. If we reach here it means we haven't completed parsing successfully
            // since there are no further boxes left.
            return AvifResult::BmffParseFailed;
        } else if header.size as u64 > (u64::MAX - parse_offset) {
            return AvifResult::BmffParseFailed;
        }
        parse_offset += header.size as u64;

        if &header.type_ == b"ftyp" {
            avif_checkerr!(!ftyp_seen, AvifResult::BmffParseFailed);
            let mut ftyp = AvifFileType::default();
            avif_checkerr!(
                avif_parse_file_type_box(&mut ftyp, &box_contents, diag),
                AvifResult::BmffParseFailed
            );
            if !avif_file_type_is_compatible(&ftyp) {
                return AvifResult::InvalidFtyp;
            }
            ftyp_seen = true;
            data.major_brand = ftyp.major_brand; // Remember the major brand for future AVIF_DECODER_SOURCE_AUTO decisions
            needs_meta_v0 = avif_file_type_has_brand(&ftyp, b"avif");
            needs_moov = avif_file_type_has_brand(&ftyp, b"avis");
        } else if &header.type_ == b"meta" {
            avif_checkerr!(!meta_v0_seen, AvifResult::BmffParseFailed);
            avif_checkres!(avif_parse_meta_box(
                &mut data.meta,
                box_offset,
                &box_contents,
                diag
            ));
            meta_v0_seen = true;
        } else if &header.type_ == b"moov" {
            avif_checkerr!(!moov_seen, AvifResult::BmffParseFailed);
            avif_checkres!(avif_parse_movie_box(
                data,
                box_offset,
                &box_contents,
                decoder.image_size_limit,
                decoder.image_dimension_limit,
                diag
            ));
            moov_seen = true;
            decoder.image_sequence_track_present = true;
        }

        // See if there is enough information to consider Parse() a success and early-out:
        // * If the brand 'avif' is present, require a meta box with version 0
        // * If the brand 'avis' is present, require a moov box
        // * If AVIF_ENABLE_EXPERIMENTAL_METAV1 is defined and the brand 'mif3' is present, require a meta box with version 1
        if ftyp_seen && (!needs_meta_v0 || meta_v0_seen) && (!needs_moov || moov_seen) {
            return AvifResult::Ok;
        }
    }
    if !ftyp_seen {
        return AvifResult::InvalidFtyp;
    }
    if (needs_meta_v0 && !meta_v0_seen) || (needs_moov && !moov_seen) {
        return AvifResult::TruncatedData;
    }
    AvifResult::Ok
}

// ---------------------------------------------------------------------------

/// Translation of `avifFileTypeHasBrand()`.
fn avif_file_type_has_brand(ftyp: &AvifFileType<'_>, brand: &[u8; 4]) -> bool {
    if &ftyp.major_brand == brand {
        return true;
    }

    for compatible_brand_index in 0..ftyp.compatible_brands_count.max(0) as usize {
        let compatible_brand =
            &ftyp.compatible_brands[4 * compatible_brand_index..4 * compatible_brand_index + 4];
        if compatible_brand == brand {
            return true;
        }
    }
    false
}

/// Translation of `avifFileTypeIsCompatible()`.
fn avif_file_type_is_compatible(ftyp: &AvifFileType<'_>) -> bool {
    avif_file_type_has_brand(ftyp, b"avif") || avif_file_type_has_brand(ftyp, b"avis")
}

/// Returns AVIF_TRUE if input begins with a valid FileTypeBox (ftyp) that supports
/// either the brand 'avif' or 'avis' (or both), without performing any allocations.
///
/// Translation of `avifPeekCompatibleFileType()`.
pub(crate) fn avif_peek_compatible_file_type(input: &[u8]) -> bool {
    let mut s = AvifROStream::start(input, None, "");

    let mut header = AvifBoxHeader::default();
    if !s.read_box_header_partial(&mut header, /*topLevel=*/ true) || &header.type_ != b"ftyp" {
        return false;
    }
    if header.is_size_zero_box {
        // The ftyp box goes on till the end of the file. Either there is no brand requiring anything in the file but a
        // FileTypebox (so not AVIF), or it is invalid.
        return false;
    }
    avif_check!(s.has_bytes_left(header.size));

    let mut ftyp = AvifFileType::default();
    let parsed = avif_parse_file_type_box(&mut ftyp, &s.current()[..header.size], None);
    if !parsed {
        return false;
    }
    avif_file_type_is_compatible(&ftyp)
}

// ---------------------------------------------------------------------------

/// Translation of `avifDecoder` (`avifDecoderCreate()` is
/// [`AvifDecoder::new`], `avifDecoderDestroy()` dropping it). The reader
/// (`io`) is an argument of the calls that read.
#[derive(Debug)]
pub(crate) struct AvifDecoder {
    // --------------------------------------------------------------------------------------------
    // Inputs
    /// Defaults to AVIF_CODEC_CHOICE_AUTO: Preference determined by order in availableCodecs table (avif.c)
    pub(crate) codec_choice: AvifCodecChoice,

    /// Defaults to 1. -- NOTE: Please see the "Understanding maxThreads" comment block above
    pub(crate) max_threads: i32,

    /// avifs can have multiple sets of images in them. This specifies which to decode.
    /// Set this via avifDecoderSetSource().
    pub(crate) requested_source: AvifDecoderSource,

    /// If this is true and a progressive AVIF is decoded, avifDecoder will behave as if the AVIF is
    /// an image sequence, in that it will set imageCount to the number of progressive frames
    /// available, and avifDecoderNextImage()/avifDecoderNthImage() will allow for specific layers
    /// of a progressive image to be decoded. To distinguish between a progressive AVIF and an AVIF
    /// image sequence, inspect avifDecoder.progressiveState.
    pub(crate) allow_progressive: bool,

    /// If this is false, avifDecoderNextImage() will start decoding a frame only after there are
    /// enough input bytes to decode all of that frame. If this is true, avifDecoder will decode each
    /// subimage or grid cell as soon as possible. The benefits are: grid images may be partially
    /// displayed before being entirely available, and the overall decoding may finish earlier.
    /// Must be set before calling avifDecoderNextImage() or avifDecoderNthImage().
    /// WARNING: Experimental feature.
    pub(crate) allow_incremental: bool,

    // Enable any of these to avoid reading and surfacing specific data to the decoded avifImage.
    // These can be useful if your avifIO implementation heavily uses AVIF_RESULT_WAITING_ON_IO for
    // streaming data, as some of these payloads are (unfortunately) packed at the end of the file,
    // which will cause avifDecoderParse() to return AVIF_RESULT_WAITING_ON_IO until it finds them.
    // If you don't actually leverage this data, it is best to ignore it here.
    pub(crate) ignore_exif: bool,
    pub(crate) ignore_xmp: bool,

    /// This represents the maximum size of an image (in pixel count) that libavif and the underlying
    /// AV1 decoder should attempt to decode. It defaults to AVIF_DEFAULT_IMAGE_SIZE_LIMIT, and can
    /// be set to a smaller value. The value 0 is reserved.
    /// Note: Only some underlying AV1 codecs support a configurable size limit (such as dav1d).
    pub(crate) image_size_limit: u32,

    /// This represents the maximum dimension of an image (width or height) that libavif should
    /// attempt to decode. It defaults to AVIF_DEFAULT_IMAGE_DIMENSION_LIMIT. Set it to 0 to ignore
    /// the limit.
    pub(crate) image_dimension_limit: u32,

    /// This provides an upper bound on how many images the decoder is willing to attempt to decode,
    /// to provide a bit of protection from malicious or malformed AVIFs citing millions upon
    /// millions of frames, only to be invalid later. The default is AVIF_DEFAULT_IMAGE_COUNT_LIMIT
    /// (see comment above), and setting this to 0 disables the limit.
    pub(crate) image_count_limit: u32,

    /// Strict flags. Defaults to AVIF_STRICT_ENABLED. See avifStrictFlag definitions above.
    pub(crate) strict_flags: AvifStrictFlags,

    // --------------------------------------------------------------------------------------------
    // Outputs
    /// All decoded image data; owned by the decoder. All information in this image is incrementally
    /// added and updated as avifDecoder*() functions are called. After a successful call to
    /// avifDecoderParse(), all values in decoder->image (other than the planes/rowBytes themselves)
    /// will be pre-populated with all information found in the outer AVIF container, prior to any
    /// AV1 decoding. If the contents of the inner AV1 payload disagree with the outer container,
    /// these values may change after calls to avifDecoderRead*(),avifDecoderNextImage(), or
    /// avifDecoderNthImage().
    ///
    /// The YUV and A contents of this image are likely owned by the decoder, so be sure to copy any
    /// data inside of this image before advancing to the next image or reusing the decoder. It is
    /// legal to call avifImageYUVToRGB() on this in between calls to avifDecoderNextImage(), but use
    /// avifImageCopy() if you want to make a complete, permanent copy of this image's YUV content or
    /// metadata.
    pub(crate) image: Option<Box<AvifImage>>,

    // Counts and timing for the current image in an image sequence. Uninteresting for single image files.
    /// 0-based
    pub(crate) image_index: i32,
    /// Always 1 for non-progressive, non-sequence AVIFs.
    pub(crate) image_count: i32,
    /// See avifProgressiveState declaration
    pub(crate) progressive_state: AvifProgressiveState,
    pub(crate) image_timing: AvifImageTiming,
    /// timescale of the media (Hz)
    pub(crate) timescale: u64,
    /// duration of a single playback of the image sequence in seconds
    /// (durationInTimescales / timescale)
    pub(crate) duration: f64,
    /// duration of a single playback of the image sequence in "timescales"
    pub(crate) duration_in_timescales: u64,
    /// number of times the sequence has to be repeated. This can also be one of
    /// AVIF_REPETITION_COUNT_INFINITE or AVIF_REPETITION_COUNT_UNKNOWN. Essentially, if
    /// repetitionCount is a non-negative integer `n`, then the image sequence should be
    /// played back `n + 1` times.
    pub(crate) repetition_count: i32,

    /// This is true when avifDecoderParse() detects an alpha plane. Use this to find out if alpha is
    /// present after a successful call to avifDecoderParse(), but prior to any call to
    /// avifDecoderNextImage() or avifDecoderNthImage(), as decoder->image->alphaPlane won't exist yet.
    pub(crate) alpha_present: bool,

    /// stats from the most recent read, possibly 0s if reading an image sequence
    pub(crate) io_stats: AvifIoStats,

    /// Additional diagnostics (such as detailed error state)
    pub(crate) diag: AvifDiagnostics,

    // --------------------------------------------------------------------------------------------
    // Internals
    /// Internals used by the decoder
    data: Option<Box<AvifDecoderData>>,

    // Version 1.0.0 ends here.
    /// This is true when avifDecoderParse() detects an image sequence track in the image. If this is true, the image can be
    /// decoded either as an animated image sequence or as a still image (the primary image item) by setting avifDecoderSetSource
    /// to the appropriate source.
    pub(crate) image_sequence_track_present: bool,
    // Version 1.1.0 ends here. Add any new members after this line.
}

impl AvifDecoder {
    /// Translation of `avifDecoderCreate()`.
    pub(crate) fn new() -> AvifDecoder {
        AvifDecoder {
            codec_choice: AvifCodecChoice::Auto,
            max_threads: 1,
            requested_source: AvifDecoderSource::Auto,
            allow_progressive: false,
            allow_incremental: false,
            ignore_exif: false,
            ignore_xmp: false,
            image_size_limit: AVIF_DEFAULT_IMAGE_SIZE_LIMIT,
            image_dimension_limit: AVIF_DEFAULT_IMAGE_DIMENSION_LIMIT,
            image_count_limit: AVIF_DEFAULT_IMAGE_COUNT_LIMIT,
            strict_flags: AVIF_STRICT_ENABLED,
            image: None,
            image_index: 0,
            image_count: 0,
            progressive_state: AvifProgressiveState::Unavailable,
            image_timing: AvifImageTiming::default(),
            timescale: 0,
            duration: 0.0,
            duration_in_timescales: 0,
            repetition_count: 0,
            alpha_present: false,
            io_stats: AvifIoStats::default(),
            diag: AvifDiagnostics::default(),
            data: None,
            image_sequence_track_present: false,
        }
    }
}

/// Translation of `avifDecoderCleanup()`.
fn avif_decoder_cleanup(decoder: &mut AvifDecoder) {
    decoder.data = None;

    decoder.image = None;
    diag::clear_error(Some(&decoder.diag));
}

// (avifDecoderDestroy() is dropping the decoder, which drops what
// avifDecoderCleanup() frees; the reader is the caller's.)

/// Translation of `avifDecoderSetSource()`.
pub(crate) fn avif_decoder_set_source(
    decoder: &mut AvifDecoder,
    io: &mut dyn AvifIo,
    source: AvifDecoderSource,
) -> AvifResult {
    decoder.requested_source = source;
    avif_decoder_reset(decoder, io)
}

// (avifDecoderSetIO(), avifDecoderSetIOMemory() and avifDecoderSetIOFile():
// the reader is passed to the calls that read)

/// 0-byte extents are ignored/overwritten during the merge, as they are the signal from helper
/// functions that no extent was necessary for this given sample. If both provided extents are
/// >0 bytes, this will set dst to be an extent that bounds both supplied extents.
///
/// Translation of `avifExtentMerge()`.
fn avif_extent_merge(dst: &mut AvifExtent, src: &AvifExtent) -> AvifResult {
    if dst.size == 0 {
        *dst = *src;
        return AvifResult::Ok;
    }
    if src.size == 0 {
        return AvifResult::Ok;
    }

    let min_extent1 = dst.offset;
    let max_extent1 = dst.offset.wrapping_add(dst.size as u64);
    let min_extent2 = src.offset;
    let max_extent2 = src.offset.wrapping_add(src.size as u64);
    dst.offset = min_extent1.min(min_extent2);
    let extent_length = max_extent1.max(max_extent2).wrapping_sub(dst.offset);
    if extent_length > usize::MAX as u64 {
        return AvifResult::BmffParseFailed;
    }
    dst.size = extent_length as usize;
    AvifResult::Ok
}

/// Streaming data helper - Use this to calculate the maximal AVIF data extent encompassing all AV1
/// sample data needed to decode the Nth image. Translation of
/// `avifDecoderNthImageMaxExtent()`.
pub(crate) fn avif_decoder_nth_image_max_extent(
    decoder: &mut AvifDecoder,
    frame_index: u32,
    out_extent: &mut AvifExtent,
) -> AvifResult {
    if decoder.data.is_none() {
        // Nothing has been parsed yet
        return AvifResult::NoContent;
    }

    *out_extent = AvifExtent::default();

    let start_frame_index = avif_decoder_nearest_keyframe(decoder, frame_index);
    let end_frame_index = frame_index;
    let Some(data) = decoder.data.as_deref_mut() else {
        return AvifResult::NoContent;
    };
    for current_frame_index in start_frame_index..=end_frame_index {
        for tile_index in 0..data.tiles.len() {
            if current_frame_index as usize >= data.tiles[tile_index].input.samples.len() {
                return AvifResult::NoImagesRemaining;
            }

            let sample = data.tiles[tile_index].input.samples[current_frame_index as usize].clone();
            let mut sample_extent = AvifExtent::default();
            if sample.item_id != 0 {
                // The data comes from an item. Let avifDecoderItemMaxExtent() do the heavy lifting.

                let mut item = 0usize;
                avif_checkres!(avif_meta_find_or_create_item(
                    &mut data.meta,
                    sample.item_id,
                    &mut item
                ));
                let max_extent_result = avif_decoder_item_max_extent(
                    &data.meta.items[item],
                    &data.meta.idat,
                    &sample,
                    &mut sample_extent,
                );
                if max_extent_result != AvifResult::Ok {
                    return max_extent_result;
                }
            } else {
                // The data likely comes from a sample table. Use the sample position directly.

                sample_extent.offset = sample.offset;
                sample_extent.size = sample.size;
            }

            if sample_extent.size as u64 > u64::MAX - sample_extent.offset {
                return AvifResult::BmffParseFailed;
            }

            let extent_merge_result = avif_extent_merge(out_extent, &sample_extent);
            if extent_merge_result != AvifResult::Ok {
                return extent_merge_result;
            }
        }
    }
    AvifResult::Ok
}

/// Translation of `avifDecoderPrepareSample()`.
fn avif_decoder_prepare_sample(
    data: &mut AvifDecoderData,
    diag: &AvifDiagnostics,
    io: &mut dyn AvifIo,
    tile_index: usize,
    sample_index: usize,
    partial_byte_count: usize,
) -> AvifResult {
    let sample = &data.tiles[tile_index].input.samples[sample_index];
    if sample.data.size() == 0 || sample.partial_data {
        // This sample hasn't been read from IO or had its extents fully merged yet.

        let mut bytes_to_read = sample.size;
        if partial_byte_count != 0 && (bytes_to_read > partial_byte_count) {
            bytes_to_read = partial_byte_count;
        }

        if sample.item_id != 0 {
            // The data comes from an item. Let avifDecoderItemRead() do the heavy lifting.

            let (item_id, sample_offset) = (sample.item_id, sample.offset);
            let mut item = 0usize;
            avif_checkres!(avif_meta_find_or_create_item(
                &mut data.meta,
                item_id,
                &mut item
            ));
            let mut item_contents = (0, 0);
            if sample_offset > usize::MAX as u64 {
                return AvifResult::BmffParseFailed;
            }
            let offset = sample_offset as usize;
            let read_result = avif_decoder_item_read(
                &mut data.meta,
                item,
                io,
                &mut item_contents,
                offset,
                bytes_to_read,
                Some(diag),
            );
            if read_result != AvifResult::Ok {
                return read_result;
            }

            // avifDecoderItemRead is guaranteed to already be persisted by either the underlying IO
            // or by mergedExtents; just reuse the buffer here.
            let partial = data.meta.items[item].partial_merged_extents;
            let sample = &mut data.tiles[tile_index].input.samples[sample_index];
            sample.data = SampleData::Item {
                offset: item_contents.0,
                size: item_contents.1,
            };
            sample.partial_data = partial;
        } else {
            // The data likely comes from a sample table. Pull the sample and make a copy if necessary.

            if (io.size_hint() > 0) && (sample.offset > io.size_hint()) {
                return AvifResult::BmffParseFailed;
            }
            let sample_size = sample.size;
            let sample_contents = match io.read(0, sample.offset, bytes_to_read) {
                Ok(b) => b,
                Err(read_result) => return read_result,
            };
            if sample_contents.len() != bytes_to_read {
                return AvifResult::TruncatedData;
            }

            // (the bytes are copied whether the reader is persistent or not)
            let mut owned = Vec::new();
            avif_checkres!(avif_rw_data_set(&mut owned, sample_contents));
            let sample = &mut data.tiles[tile_index].input.samples[sample_index];
            sample.partial_data = bytes_to_read != sample_size;
            sample.data = SampleData::Owned(owned);
        }
    }
    AvifResult::Ok
}

/// The bytes of a prepared sample (`sample->data`).
fn sample_bytes<'m>(sample: &'m AvifDecodeSample, meta: &'m AvifMeta) -> &'m [u8] {
    match &sample.data {
        SampleData::None => &[],
        SampleData::Owned(v) => v,
        SampleData::Item { offset, size } => {
            match meta.items.iter().find(|item| item.id == sample.item_id) {
                Some(item) => item
                    .merged_extents_data(&meta.idat)
                    .and_then(|d| d.get(*offset..*offset + *size))
                    .unwrap_or(&[]),
                None => &[],
            }
        }
    }
}

/// Returns AVIF_TRUE if the item should be skipped. Items should be skipped for one of the following reasons:
///  * Size is 0.
///  * Has an essential property that isn't supported by libavif.
///  * Item is not a single image or a grid.
///  * Item is a thumbnail.
///
/// Translation of `avifDecoderItemShouldBeSkipped()`.
fn avif_decoder_item_should_be_skipped(item: &AvifDecoderItem) -> bool {
    item.size == 0
        || item.has_unsupported_essential_property
        || (avif_get_codec_type(&item.type_) == AvifCodecType::Unknown && &item.type_ != b"grid")
        || item.thumbnail_for_id != 0
}

/// Translation of `avifDecoderParse()`.
pub(crate) fn avif_decoder_parse(decoder: &mut AvifDecoder, io: &mut dyn AvifIo) -> AvifResult {
    diag::clear_error(Some(&decoder.diag));

    // An imageSizeLimit greater than AVIF_DEFAULT_IMAGE_SIZE_LIMIT and the special value of 0 to
    // disable the limit are not yet implemented.
    if (decoder.image_size_limit > AVIF_DEFAULT_IMAGE_SIZE_LIMIT) || (decoder.image_size_limit == 0)
    {
        return AvifResult::NotImplemented;
    }
    // (the reader is always set: it is an argument)

    // Cleanup anything lingering in the decoder
    avif_decoder_cleanup(decoder);

    // -----------------------------------------------------------------------
    // Parse BMFF boxes

    decoder.data = Some(avif_decoder_data_create());

    avif_checkres!(avif_parse(decoder, io));

    // Walk the decoded items (if any) and harvest ispe
    let Some(data) = decoder.data.as_deref_mut() else {
        return AvifResult::NoContent;
    };
    let diag = Some(&decoder.diag);
    for item in &mut data.meta.items {
        if avif_decoder_item_should_be_skipped(item) {
            continue;
        }

        if let Some(ispe_prop) = avif_property_array_find(&item.properties, b"ispe") {
            item.width = ispe_prop.u.ispe.width;
            item.height = ispe_prop.u.ispe.height;

            if (item.width == 0) || (item.height == 0) {
                diag_printf!(
                    diag,
                    "Item ID [{}] has an invalid size [{}x{}]",
                    item.id,
                    item.width,
                    item.height
                );
                return AvifResult::BmffParseFailed;
            }
            if avif_dimensions_too_large(
                item.width,
                item.height,
                decoder.image_size_limit,
                decoder.image_dimension_limit,
            ) {
                diag_printf!(
                    diag,
                    "Item ID [{}] dimensions are too large [{}x{}]",
                    item.id,
                    item.width,
                    item.height
                );
                return AvifResult::BmffParseFailed;
            }
        } else {
            let aux_c_prop = avif_property_array_find(&item.properties, b"auxC");
            if aux_c_prop.is_some_and(|p| is_alpha_urn(&p.u.aux_c.aux_type)) {
                if decoder.strict_flags & AVIF_STRICT_ALPHA_ISPE_REQUIRED != 0 {
                    diag_printf!(
                        diag,
                        "[Strict] Alpha auxiliary image item ID [{}] is missing a mandatory ispe property",
                        item.id
                    );
                    return AvifResult::BmffParseFailed;
                }
            } else {
                diag_printf!(
                    diag,
                    "Item ID [{}] is missing a mandatory ispe property",
                    item.id
                );
                return AvifResult::BmffParseFailed;
            }
        }
    }
    avif_decoder_reset(decoder, io)
}

/// Translation of `avifCodecCreateInternal()`.
fn avif_codec_create_internal(
    choice: AvifCodecChoice,
    tile: &AvifTile,
    diag: Option<&AvifDiagnostics>,
    codec: &mut Option<Box<AvifCodec>>,
) -> AvifResult {
    let codec_type_from_choice = avif_codec_type_from_choice(choice, AVIF_CODEC_FLAG_CAN_DECODE);
    if codec_type_from_choice == AvifCodecType::Unknown {
        diag_printf!(
            diag,
            "Tile type is {} but there is no compatible codec available to decode it",
            fourcc(avif_get_configuration_property_name(tile.codec_type))
        );
        return AvifResult::NoCodecAvailable;
    } else if choice != AvifCodecChoice::Auto && codec_type_from_choice != tile.codec_type {
        diag_printf!(
            diag,
            "Tile type is {} but incompatible {} codec was explicitly set as decoding implementation",
            fourcc(avif_get_configuration_property_name(tile.codec_type)),
            avif_codec_name(choice, AVIF_CODEC_FLAG_CAN_DECODE).unwrap_or("(null)")
        );
        return AvifResult::DecodeColorFailed;
    }

    avif_checkres!(avif_codec_create(choice, AVIF_CODEC_FLAG_CAN_DECODE, codec));
    let Some(c) = codec.as_mut() else {
        return AvifResult::OutOfMemory;
    };
    c.operating_point = tile.operating_point;
    c.all_layers = tile.input.all_layers;
    AvifResult::Ok
}

/// Translation of `avifTilesCanBeDecodedWithSameCodecInstance()`.
fn avif_tiles_can_be_decoded_with_same_codec_instance(data: &AvifDecoderData) -> bool {
    let mut num_image_buffers: i32 = 0;
    let mut num_stolen_image_buffers: i32 = 0;
    for c in 0..AVIF_ITEM_CATEGORY_COUNT {
        if data.tile_infos[c].tile_count > 0 {
            num_image_buffers += 1;
        }
        if data.tile_infos[c].tile_count == 1 {
            num_stolen_image_buffers += 1;
        }
    }
    if num_stolen_image_buffers > 0 && num_image_buffers > 1 {
        // Single tile image with single tile alpha plane or gain map. In this case each tile needs its own decoder since the planes will be
        // "stolen". Stealing either the color or the alpha plane (or gain map) will invalidate the other ones when decode is called the second
        // (or third) time.
        return false;
    }
    let first_tile_operating_point = data.tiles[0].operating_point;
    let first_tile_all_layers = data.tiles[0].input.all_layers;
    for tile in &data.tiles[1..] {
        if tile.operating_point != first_tile_operating_point
            || tile.input.all_layers != first_tile_all_layers
        {
            return false;
        }
        // avifDecoderItemValidateProperties() verified during avifDecoderParse() that all tiles
        // share the same coding format so no need to check for codecType equality here.
    }
    true
}

/// Translation of `avifDecoderCreateCodecs()`.
fn avif_decoder_create_codecs(decoder: &mut AvifDecoder) -> AvifResult {
    let image_count = decoder.image_count;
    let codec_choice = decoder.codec_choice;
    let diag = Some(&decoder.diag);
    let Some(data) = decoder.data.as_deref_mut() else {
        return AvifResult::NoContent;
    };
    avif_decoder_data_reset_codec(data);

    if data.source == AvifDecoderSource::Tracks {
        // In this case, we will use at most two codec instances (one for the color planes and one for the alpha plane).
        // Gain maps are not supported.
        avif_checkres!(avif_codec_create_internal(
            codec_choice,
            &data.tiles[0],
            diag,
            &mut data.codec
        ));
        data.tiles[0].codec = TileCodec::Shared;
        if data.tiles.len() > 1 {
            avif_checkres!(avif_codec_create_internal(
                codec_choice,
                &data.tiles[1],
                diag,
                &mut data.codec_alpha
            ));
            data.tiles[1].codec = TileCodec::SharedAlpha;
        }
    } else {
        // In this case, we will use one codec instance when there is only one tile or when all of the following conditions are
        // met:
        //   - The image must have exactly one layer (i.e.) decoder->imageCount == 1.
        //   - All the tiles must have the same operating point (because the codecs take operating point once at initialization
        //     and do not allow it to be changed later).
        //   - All the tiles must have the same value for allLayers (because the codecs take allLayers once at initialization
        //     and do not allow it to be changed later).
        //   - If the image has a single tile, it must not have a single tile alpha plane (in this case we will steal the planes
        //     from the decoder, so we cannot use the same decoder for both the color and the alpha planes).
        //   - All tiles have the same type (AV1 or AV2).
        // Otherwise, we will use |tiles.count| decoder instances (one instance for each tile).
        let can_use_single_codec_instance = (data.tiles.len() == 1)
            || (image_count == 1 && avif_tiles_can_be_decoded_with_same_codec_instance(data));
        if can_use_single_codec_instance {
            avif_checkres!(avif_codec_create_internal(
                codec_choice,
                &data.tiles[0],
                diag,
                &mut data.codec
            ));
            for tile in &mut data.tiles {
                tile.codec = TileCodec::Shared;
            }
        } else {
            for tile in &mut data.tiles {
                let mut codec = None;
                avif_checkres!(avif_codec_create_internal(
                    codec_choice,
                    tile,
                    diag,
                    &mut codec
                ));
                if let Some(codec) = codec {
                    tile.codec = TileCodec::Own(codec);
                }
            }
        }
    }
    AvifResult::Ok
}

/// Returns the primary color item if found, or NULL. Translation of
/// `avifMetaFindColorItem()`.
fn avif_meta_find_color_item(meta: &AvifMeta) -> Option<usize> {
    for (item_index, item) in meta.items.iter().enumerate() {
        if avif_decoder_item_should_be_skipped(item) {
            continue;
        }
        if item.id == meta.primary_item_id {
            return Some(item_index);
        }
    }
    None
}

/// Returns AVIF_TRUE if item is an alpha auxiliary item of the parent
/// color item. Translation of `avifDecoderItemIsAlphaAux()`.
fn avif_decoder_item_is_alpha_aux(item: &AvifDecoderItem, color_item_id: u32) -> bool {
    if item.aux_for_id != color_item_id {
        return false;
    }
    let aux_c_prop = avif_property_array_find(&item.properties, b"auxC");
    aux_c_prop.is_some_and(|p| is_alpha_urn(&p.u.aux_c.aux_type))
}

/// Finds the alpha item whose parent item is colorItem and sets it in the alphaItem output parameter. Returns AVIF_RESULT_OK on
/// success. Note that *alphaItem can be NULL even if the return value is AVIF_RESULT_OK. If the colorItem is a grid and the alpha
/// item is represented as a set of auxl items to each color tile, then a fake item will be created and *isAlphaItemInInput will be
/// set to AVIF_FALSE. In this case, the alpha item merely exists to hold the locations of the alpha tile items. The data of this
/// item need not be read and the pixi property cannot be validated. Otherwise, *isAlphaItemInInput will be set to AVIF_TRUE when
/// *alphaItem is not NULL.
///
/// Translation of `avifMetaFindAlphaItem()`.
fn avif_meta_find_alpha_item(
    meta: &mut AvifMeta,
    color_item_idx: usize,
    color_info: &AvifTileInfo,
    alpha_item: &mut Option<usize>,
    alpha_info: &mut AvifTileInfo,
    is_alpha_item_in_input: &mut bool,
) -> AvifResult {
    let color_item_id = meta.items[color_item_idx].id;
    for (item_index, item) in meta.items.iter().enumerate() {
        if avif_decoder_item_should_be_skipped(item) {
            continue;
        }
        if avif_decoder_item_is_alpha_aux(item, color_item_id) {
            *alpha_item = Some(item_index);
            *is_alpha_item_in_input = true;
            return AvifResult::Ok;
        }
    }
    if &meta.items[color_item_idx].type_ != b"grid" {
        *alpha_item = None;
        *is_alpha_item_in_input = false;
        return AvifResult::Ok;
    }
    // If color item is a grid, check if there is an alpha channel which is represented as an auxl item to each color tile item.
    let tile_count = color_info.grid.rows.wrapping_mul(color_info.grid.columns);
    if tile_count == 0 {
        *alpha_item = None;
        *is_alpha_item_in_input = false;
        return AvifResult::Ok;
    }
    // Keep the same 'dimg' order as it defines where each tile is located in the reconstructed image.
    let item_index_not_set = u32::MAX;
    let mut dimg_idx_to_alpha_item_idx: Vec<u32> = Vec::new();
    avif_checkerr!(
        dimg_idx_to_alpha_item_idx
            .try_reserve_exact(tile_count as usize)
            .is_ok(),
        AvifResult::OutOfMemory
    );
    dimg_idx_to_alpha_item_idx.resize(tile_count as usize, item_index_not_set);
    let mut alpha_item_count: u32 = 0;
    for i in 0..meta.items.len() {
        let item = &meta.items[i];
        if item.dimg_for_id == color_item_id {
            let mut seen_alpha_for_current_item = false;
            for j in 0..meta.items.len() {
                let auxl_item = &meta.items[j];
                if avif_decoder_item_is_alpha_aux(auxl_item, item.id) {
                    if seen_alpha_for_current_item
                        || auxl_item.dimg_for_id != 0
                        || item.dimg_idx >= tile_count
                        || dimg_idx_to_alpha_item_idx[item.dimg_idx as usize] != item_index_not_set
                    {
                        // One of the following invalid cases:
                        // * Multiple items are claiming to be the alpha auxiliary of the current item.
                        // * Alpha auxiliary is dimg for another item.
                        // * There are too many items in the dimg array (also checked later in avifFillDimgIdxToItemIdxArray()).
                        // * There is a repetition in the dimg array (also checked later in avifFillDimgIdxToItemIdxArray()).
                        return AvifResult::InvalidImageGrid;
                    }
                    dimg_idx_to_alpha_item_idx[item.dimg_idx as usize] = j as u32;
                    alpha_item_count += 1;
                    seen_alpha_for_current_item = true;
                }
            }
            if !seen_alpha_for_current_item {
                // No alpha auxiliary item was found for the current item. Treat this as an image without alpha.
                *alpha_item = None;
                *is_alpha_item_in_input = false;
                return AvifResult::Ok;
            }
        }
    }
    if alpha_item_count != tile_count {
        return AvifResult::InvalidImageGrid;
    }
    // Find an unused ID.
    let mut result;
    let mut new_item = 0usize;
    if meta.items.len() as u64 >= (u32::MAX - 1) as u64 {
        // In the improbable case where all IDs are used.
        result = AvifResult::DecodeAlphaFailed;
    } else {
        let mut new_item_id: u32 = 0;
        loop {
            new_item_id = new_item_id.wrapping_add(1);
            let is_used = meta.items.iter().any(|item| item.id == new_item_id);
            if !(is_used && new_item_id != 0) {
                break;
            }
        }
        result = avif_meta_find_or_create_item(meta, new_item_id, &mut new_item);
        // Create new empty item.
    }
    if result != AvifResult::Ok {
        return result;
    }
    *alpha_item = Some(new_item);
    let (color_width, color_height) = (
        meta.items[color_item_idx].width,
        meta.items[color_item_idx].height,
    );
    let alpha_id = {
        let alpha = &mut meta.items[new_item];
        alpha.type_ = *b"grid"; // Make it a grid and register alpha items as its tiles.
        alpha.width = color_width;
        alpha.height = color_height;
        alpha.id
    };
    for dimg_idx in 0..tile_count as usize {
        if dimg_idx_to_alpha_item_idx[dimg_idx] as usize >= meta.items.len() {
            avif_assert_or_return!(false);
        }
        let alpha_tile_item = &mut meta.items[dimg_idx_to_alpha_item_idx[dimg_idx] as usize];
        alpha_tile_item.dimg_for_id = alpha_id;
        alpha_tile_item.dimg_idx = dimg_idx as u32;
    }
    *is_alpha_item_in_input = false;
    alpha_info.grid = color_info.grid;
    result = AvifResult::Ok;
    result
}

/// On success, this function returns AVIF_RESULT_OK and does the following:
/// * If a nclx property was found in |properties|:
///   - Set |*colorPrimaries|, |*transferCharacteristics|, |*matrixCoefficients|
///     and |*yuvRange|.
///   - If cicpSet is not NULL, set |*cicpSet| to AVIF_TRUE.
///
/// This function fails if more than one nclx property is found in |properties|.
/// The output parameters may be populated even in case of failure and must be
/// ignored.
///
/// Translation of `avifReadColorNclxProperty()` (the outputs are the
/// image's).
fn avif_read_color_nclx_property(
    properties: &[AvifProperty],
    image: &mut AvifImage,
    cicp_set: Option<&mut bool>,
) -> AvifResult {
    let mut colr_nclx_seen = false;
    let mut cicp_set = cicp_set;
    for prop in properties {
        if &prop.type_ == b"colr" && prop.u.colr.has_nclx {
            if colr_nclx_seen {
                return AvifResult::BmffParseFailed;
            }
            colr_nclx_seen = true;
            if let Some(cicp_set) = cicp_set.as_deref_mut() {
                *cicp_set = true;
            }
            image.color_primaries = prop.u.colr.color_primaries;
            image.transfer_characteristics = prop.u.colr.transfer_characteristics;
            image.matrix_coefficients = prop.u.colr.matrix_coefficients;
            image.yuv_range = prop.u.colr.range;
        }
    }
    AvifResult::Ok
}

/// On success, this function returns AVIF_RESULT_OK and does the following:
/// * If a colr property was found in |properties|:
///   - Read the icc data into |icc| from |io|.
///   - Sets the CICP values as documented in avifReadColorNclxProperty().
///
/// This function fails if more than one icc or nclx property is found in
/// |properties|. The output parameters may be populated even in case of failure
/// and must be ignored (and the |icc| object may need to be freed).
///
/// Translation of `avifReadColorProperties()` (the outputs are the
/// image's).
fn avif_read_color_properties(
    io: &mut dyn AvifIo,
    properties: &[AvifProperty],
    image: &mut AvifImage,
    cicp_set: Option<&mut bool>,
) -> AvifResult {
    // Find and adopt all colr boxes "at most one for a given value of colour type" (HEIF 6.5.5.1, from Amendment 3)
    // Accept one of each type, and bail out if more than one of a given type is provided.
    let mut colr_icc_seen = false;
    for prop in properties {
        if &prop.type_ == b"colr" && prop.u.colr.has_icc {
            if colr_icc_seen {
                return AvifResult::BmffParseFailed;
            }
            let icc_read = match io.read(0, prop.u.colr.icc_offset, prop.u.colr.icc_size) {
                Ok(b) => b,
                Err(e) => return e,
            };
            colr_icc_seen = true;
            avif_checkres!(avif_rw_data_set(&mut image.icc, icc_read));
        }
    }
    avif_read_color_nclx_property(properties, image, cicp_set)
}

/// Translation of `avifDecoderGenerateImageTiles()`.
#[allow(clippy::too_many_arguments)]
fn avif_decoder_generate_image_tiles(
    data: &mut AvifDecoderData,
    allow_progressive: bool,
    image_count_limit: u32,
    size_hint: u64,
    diag: Option<&AvifDiagnostics>,
    info_index: usize,
    item_idx: usize,
    item_category: AvifItemCategory,
) -> AvifResult {
    let previous_tile_count = data.tiles.len() as u32;
    let info = data.tile_infos[info_index];
    if (info.grid.rows > 0) && (info.grid.columns > 0) {
        // The number of tiles was verified in avifDecoderItemReadAndParse().
        let num_tiles = info.grid.rows * info.grid.columns;
        let mut dimg_idx_to_item_idx: Vec<u32> = Vec::new();
        avif_checkerr!(
            dimg_idx_to_item_idx
                .try_reserve_exact(num_tiles as usize)
                .is_ok(),
            AvifResult::OutOfMemory
        );
        dimg_idx_to_item_idx.resize(num_tiles as usize, 0);
        let mut result = avif_fill_dimg_idx_to_item_idx_array(
            &mut dimg_idx_to_item_idx,
            num_tiles,
            &data.meta,
            item_idx,
        );
        if result == AvifResult::Ok {
            result = avif_decoder_generate_image_grid_tiles(
                data,
                allow_progressive,
                image_count_limit,
                size_hint,
                diag,
                item_idx,
                item_category,
                &dimg_idx_to_item_idx,
                num_tiles,
            );
        }
        avif_checkres!(result);
    } else {
        avif_checkerr!(
            data.meta.items[item_idx].size != 0,
            AvifResult::MissingImageItem
        );

        let item = &data.meta.items[item_idx];
        let codec_type = avif_get_codec_type(&item.type_);
        avif_assert_or_return!(codec_type != AvifCodecType::Unknown);
        let (width, height, operating_point) = (
            item.width,
            item.height,
            avif_decoder_item_operating_point(item),
        );
        let tile = avif_decoder_data_create_tile(data, codec_type, width, height, operating_point);
        avif_checkerr!(tile.is_some(), AvifResult::OutOfMemory);
        let Some(tile) = tile else {
            return AvifResult::OutOfMemory;
        };
        let AvifDecoderData { tiles, meta, .. } = data;
        avif_checkres!(avif_codec_decode_input_fill_from_decoder_item(
            &mut tiles[tile].input,
            &mut meta.items[item_idx],
            allow_progressive,
            image_count_limit,
            size_hint,
            diag
        ));
        tiles[tile].input.item_category = item_category;
    }
    data.tile_infos[info_index].tile_count = data.tiles.len() as u32 - previous_tile_count;
    AvifResult::Ok
}

/// Populates depth, yuvFormat and yuvChromaSamplePosition fields on 'image' based on data from the codec config property (e.g. "av1C").
/// Translation of `avifReadCodecConfigProperty()`.
fn avif_read_codec_config_property(
    image: &mut AvifImage,
    properties: &[AvifProperty],
    codec_type: AvifCodecType,
) -> AvifResult {
    if let Some(config_prop) =
        avif_property_array_find(properties, avif_get_configuration_property_name(codec_type))
    {
        image.depth = avif_codec_configuration_box_get_depth(&config_prop.u.av1c);
        if config_prop.u.av1c.monochrome != 0 {
            image.yuv_format = AvifPixelFormat::Yuv400;
        } else if config_prop.u.av1c.chroma_subsampling_x != 0
            && config_prop.u.av1c.chroma_subsampling_y != 0
        {
            image.yuv_format = AvifPixelFormat::Yuv420;
        } else if config_prop.u.av1c.chroma_subsampling_x != 0 {
            image.yuv_format = AvifPixelFormat::Yuv422;
        } else {
            image.yuv_format = AvifPixelFormat::Yuv444;
        }
        image.yuv_chroma_sample_position =
            config_prop.u.av1c.chroma_sample_position as AvifChromaSamplePosition;
    } else {
        // A configuration property box is mandatory in all valid AVIF configurations. Bail out.
        return AvifResult::BmffParseFailed;
    }
    AvifResult::Ok
}

/// Where the color properties come from: the color track's sample
/// description (track index, codec type) or the main color item (item
/// index).
#[derive(Clone, Copy)]
enum ColorProperties {
    Track(usize, AvifCodecType),
    Item(usize),
}

impl ColorProperties {
    fn get(self, data: &AvifDecoderData) -> &[AvifProperty] {
        self.get_from(&data.tracks, &data.meta)
    }

    /// [`get`](Self::get), from the decoder data's tracks and meta box.
    fn get_from<'d>(self, tracks: &'d [AvifTrack], meta: &'d AvifMeta) -> &'d [AvifProperty] {
        match self {
            ColorProperties::Track(t, codec_type) => tracks[t]
                .sample_table
                .as_deref()
                .and_then(|st| avif_sample_table_get_properties(st, codec_type))
                .unwrap_or(&[]),
            ColorProperties::Item(i) => &meta.items[i].properties,
        }
    }
}

/// Translation of `avifDecoderReset()`.
pub(crate) fn avif_decoder_reset(decoder: &mut AvifDecoder, io: &mut dyn AvifIo) -> AvifResult {
    diag::clear_error(Some(&decoder.diag));

    let Some(data) = decoder.data.as_deref_mut() else {
        // Nothing to reset.
        return AvifResult::Ok;
    };

    for c in 0..AVIF_ITEM_CATEGORY_COUNT {
        data.tile_infos[c].grid = AvifImageGrid::default();
    }
    avif_decoder_data_clear_tiles(data);

    // Prepare / cleanup decoded image state
    decoder.image = avif_image_create_empty();
    let Some(image) = decoder.image.as_deref_mut() else {
        return AvifResult::OutOfMemory;
    };
    decoder.progressive_state = AvifProgressiveState::Unavailable;
    data.cicp_set = false;

    decoder.io_stats = AvifIoStats::default();
    let diag = &decoder.diag;

    // -----------------------------------------------------------------------
    // Build decode input

    data.source_sample_table = None; // Reset
    if decoder.requested_source == AvifDecoderSource::Auto {
        // Honor the major brand (avif or avis) if present, otherwise prefer avis (tracks) if possible.
        if &data.major_brand == b"avis" {
            data.source = AvifDecoderSource::Tracks;
        } else if &data.major_brand == b"avif" {
            data.source = AvifDecoderSource::PrimaryItem;
        } else if !data.tracks.is_empty() {
            data.source = AvifDecoderSource::Tracks;
        } else {
            data.source = AvifDecoderSource::PrimaryItem;
        }
    } else {
        data.source = decoder.requested_source;
    }

    let color_codec_type;
    let color_properties;
    if data.source == AvifDecoderSource::Tracks {
        // Find primary track - this probably needs some better detection
        let mut color_codec_type_found = AvifCodecType::Unknown;
        let mut color_track_index = 0usize;
        while color_track_index < data.tracks.len() {
            let track = &data.tracks[color_track_index];
            let Some(sample_table) = track.sample_table.as_deref() else {
                color_track_index += 1;
                continue;
            };
            if track.id == 0 {
                // trak box might be missing a tkhd box inside, skip it
                color_track_index += 1;
                continue;
            }
            if sample_table.chunks.is_empty() {
                color_track_index += 1;
                continue;
            }
            color_codec_type_found = avif_sample_table_get_codec_type(sample_table);
            if color_codec_type_found == AvifCodecType::Unknown {
                color_track_index += 1;
                continue;
            }
            if track.aux_for_id != 0 {
                color_track_index += 1;
                continue;
            }

            // Found one!
            break;
        }
        color_codec_type = color_codec_type_found;
        if color_track_index == data.tracks.len() {
            diag_printf!(Some(diag), "Failed to find AV1 color track");
            return AvifResult::NoContent;
        }
        let color_track = color_track_index;

        let has_color_properties = data.tracks[color_track]
            .sample_table
            .as_deref()
            .and_then(|st| avif_sample_table_get_properties(st, color_codec_type))
            .is_some();
        if !has_color_properties {
            diag_printf!(
                Some(diag),
                "Failed to find AV1 color track's color properties"
            );
            return AvifResult::BmffParseFailed;
        }
        color_properties = ColorProperties::Track(color_track, color_codec_type);

        // Find Exif and/or XMP metadata, if any
        {
            // See the comment above avifDecoderFindMetadata() for the explanation of using 0 here
            let find_result = avif_decoder_find_metadata(
                decoder.ignore_exif,
                decoder.ignore_xmp,
                diag,
                io,
                &mut data.tracks[color_track].meta,
                image,
                0,
            );
            if find_result != AvifResult::Ok {
                return find_result;
            }
        }

        let color_track_id = data.tracks[color_track].id;
        let mut alpha_track_index = 0usize;
        let mut alpha_codec_type = AvifCodecType::Unknown;
        while alpha_track_index < data.tracks.len() {
            let track = &data.tracks[alpha_track_index];
            let Some(sample_table) = track.sample_table.as_deref() else {
                alpha_track_index += 1;
                continue;
            };
            if track.id == 0 {
                alpha_track_index += 1;
                continue;
            }
            if sample_table.chunks.is_empty() {
                alpha_track_index += 1;
                continue;
            }
            alpha_codec_type = avif_sample_table_get_codec_type(sample_table);
            if alpha_codec_type == AvifCodecType::Unknown {
                alpha_track_index += 1;
                continue;
            }
            if track.aux_for_id == color_track_id {
                // Found it!
                break;
            }
            alpha_track_index += 1;
        }
        let alpha_track = if alpha_track_index != data.tracks.len() {
            Some(alpha_track_index)
        } else {
            None
        };

        let operating_point: u8 = 0; // No way to set operating point via tracks
        let (color_width, color_height) = (
            data.tracks[color_track].width,
            data.tracks[color_track].height,
        );
        let color_tile = avif_decoder_data_create_tile(
            data,
            color_codec_type,
            color_width,
            color_height,
            operating_point,
        );
        avif_checkerr!(color_tile.is_some(), AvifResult::OutOfMemory);
        let Some(color_tile) = color_tile else {
            return AvifResult::OutOfMemory;
        };
        {
            let AvifDecoderData { tiles, tracks, .. } = data;
            let Some(sample_table) = tracks[color_track].sample_table.as_deref() else {
                return AvifResult::NoContent;
            };
            avif_checkres!(avif_codec_decode_input_fill_from_sample_table(
                &mut tiles[color_tile].input,
                sample_table,
                decoder.image_count_limit,
                io.size_hint(),
                Some(diag)
            ));
        }
        data.tile_infos[AvifItemCategory::Color as usize].tile_count = 1;

        if let Some(alpha_track) = alpha_track {
            let (alpha_width, alpha_height) = (
                data.tracks[alpha_track].width,
                data.tracks[alpha_track].height,
            );
            let alpha_tile = avif_decoder_data_create_tile(
                data,
                alpha_codec_type,
                alpha_width,
                alpha_height,
                operating_point,
            );
            avif_checkerr!(alpha_tile.is_some(), AvifResult::OutOfMemory);
            let Some(alpha_tile) = alpha_tile else {
                return AvifResult::OutOfMemory;
            };
            let AvifDecoderData { tiles, tracks, .. } = data;
            let Some(sample_table) = tracks[alpha_track].sample_table.as_deref() else {
                return AvifResult::NoContent;
            };
            avif_checkres!(avif_codec_decode_input_fill_from_sample_table(
                &mut tiles[alpha_tile].input,
                sample_table,
                decoder.image_count_limit,
                io.size_hint(),
                Some(diag)
            ));
            tiles[alpha_tile].input.item_category = AvifItemCategory::Alpha;
            data.tile_infos[AvifItemCategory::Alpha as usize].tile_count = 1;
        }

        // Stash off sample table for future timing information
        data.source_sample_table = Some(color_track);

        // Image sequence timing
        let color_track_ref = &data.tracks[color_track];
        decoder.image_index = -1;
        decoder.image_count = data.tiles[color_tile].input.samples.len() as i32;
        decoder.timescale = color_track_ref.media_timescale as u64;
        decoder.duration_in_timescales = color_track_ref.media_duration;
        if color_track_ref.media_timescale != 0 {
            decoder.duration =
                decoder.duration_in_timescales as f64 / color_track_ref.media_timescale as f64;
        } else {
            decoder.duration = 0.0;
        }
        // If the alphaTrack->repetitionCount and colorTrack->repetitionCount are different, we will simply use the
        // colorTrack's repetitionCount.
        decoder.repetition_count = color_track_ref.repetition_count;

        decoder.image_timing = AvifImageTiming::default(); // to be set in avifDecoderNextImage()

        image.width = color_track_ref.width;
        image.height = color_track_ref.height;
        decoder.alpha_present = alpha_track.is_some();
        image.alpha_premultiplied = decoder.alpha_present
            && alpha_track.is_some_and(|a| color_track_ref.prem_by_id == data.tracks[a].id);
    } else {
        // Create from items

        if data.meta.primary_item_id == 0 {
            // A primary item is required
            diag_printf!(Some(diag), "Primary item not specified");
            return AvifResult::MissingImageItem;
        }

        // Main item of each group category (top-level item such as grid or single tile), if any.
        let mut main_items: [Option<usize>; AVIF_ITEM_CATEGORY_COUNT] =
            [None; AVIF_ITEM_CATEGORY_COUNT];
        let mut codec_type: [AvifCodecType; AVIF_ITEM_CATEGORY_COUNT] =
            [AvifCodecType::Unknown; AVIF_ITEM_CATEGORY_COUNT];

        // Mandatory primary color item
        main_items[AvifItemCategory::Color as usize] = avif_meta_find_color_item(&data.meta);
        let Some(color_item) = main_items[AvifItemCategory::Color as usize] else {
            diag_printf!(Some(diag), "Primary item not found");
            return AvifResult::MissingImageItem;
        };
        avif_checkres!(avif_decoder_item_read_and_parse(
            decoder.image_size_limit,
            decoder.image_dimension_limit,
            Some(diag),
            io,
            &mut data.meta,
            color_item,
            /*isItemInInput=*/ true,
            &mut data.tile_infos[AvifItemCategory::Color as usize].grid,
            &mut codec_type[AvifItemCategory::Color as usize]
        ));
        color_properties = ColorProperties::Item(color_item);
        color_codec_type = codec_type[AvifItemCategory::Color as usize];

        // Optional alpha auxiliary item
        let mut is_alpha_item_in_input = false;
        {
            let color_info = data.tile_infos[AvifItemCategory::Color as usize];
            let mut alpha_info = data.tile_infos[AvifItemCategory::Alpha as usize];
            avif_checkres!(avif_meta_find_alpha_item(
                &mut data.meta,
                color_item,
                &color_info,
                &mut main_items[AvifItemCategory::Alpha as usize],
                &mut alpha_info,
                &mut is_alpha_item_in_input
            ));
            data.tile_infos[AvifItemCategory::Alpha as usize] = alpha_info;
        }
        if let Some(alpha_item) = main_items[AvifItemCategory::Alpha as usize] {
            avif_checkres!(avif_decoder_item_read_and_parse(
                decoder.image_size_limit,
                decoder.image_dimension_limit,
                Some(diag),
                io,
                &mut data.meta,
                alpha_item,
                is_alpha_item_in_input,
                &mut data.tile_infos[AvifItemCategory::Alpha as usize].grid,
                &mut codec_type[AvifItemCategory::Alpha as usize]
            ));
        }

        // Find Exif and/or XMP metadata, if any
        let color_item_id = data.meta.items[color_item].id;
        avif_checkres!(avif_decoder_find_metadata(
            decoder.ignore_exif,
            decoder.ignore_xmp,
            diag,
            io,
            &mut data.meta,
            image,
            color_item_id
        ));

        // Set all counts and timing to safe-but-uninteresting values
        decoder.image_index = -1;
        decoder.image_count = 1;
        decoder.image_timing.timescale = 1;
        decoder.image_timing.pts = 0.0;
        decoder.image_timing.pts_in_timescales = 0;
        decoder.image_timing.duration = 1.0;
        decoder.image_timing.duration_in_timescales = 1;
        decoder.timescale = 1;
        decoder.duration = 1.0;
        decoder.duration_in_timescales = 1;

        for c in (AvifItemCategory::Color as usize)..AVIF_ITEM_CATEGORY_COUNT {
            let Some(main_item) = main_items[c] else {
                continue;
            };

            if avif_is_alpha(AvifItemCategory::from_index(c))
                && data.meta.items[main_item].width == 0
                && data.meta.items[main_item].height == 0
            {
                // NON-STANDARD: Alpha subimage does not have an ispe property; adopt width/height from color item
                avif_assert_or_return!(
                    (decoder.strict_flags & AVIF_STRICT_ALPHA_ISPE_REQUIRED) == 0
                );
                data.meta.items[main_item].width = data.meta.items[color_item].width;
                data.meta.items[main_item].height = data.meta.items[color_item].height;
            }

            avif_checkres!(avif_decoder_generate_image_tiles(
                data,
                decoder.allow_progressive,
                decoder.image_count_limit,
                io.size_hint(),
                Some(diag),
                c,
                main_item,
                AvifItemCategory::from_index(c)
            ));

            let mut strict_flags = decoder.strict_flags;
            if avif_is_alpha(AvifItemCategory::from_index(c)) && !is_alpha_item_in_input {
                // In this case, the made up grid item will not have an associated pixi property. So validate everything else
                // but the pixi property.
                strict_flags &= !AVIF_STRICT_PIXI_REQUIRED;
            }
            avif_checkres!(avif_decoder_item_validate_properties(
                &data.meta,
                main_item,
                avif_get_configuration_property_name(codec_type[c]),
                Some(diag),
                strict_flags
            ));
        }

        if data.meta.items[color_item].progressive {
            decoder.progressive_state = AvifProgressiveState::Available;
            // data->tileInfos[AVIF_ITEM_COLOR].firstTileIndex is not yet defined but will be set to 0 a few lines below.
            let color_tile = &data.tiles[0];
            if color_tile.input.samples.len() > 1 {
                decoder.progressive_state = AvifProgressiveState::Active;
                decoder.image_count = color_tile.input.samples.len() as i32;
            }
        }

        image.width = data.meta.items[color_item].width;
        image.height = data.meta.items[color_item].height;
        decoder.alpha_present = main_items[AvifItemCategory::Alpha as usize].is_some();
        image.alpha_premultiplied = decoder.alpha_present
            && main_items[AvifItemCategory::Alpha as usize]
                .is_some_and(|a| data.meta.items[color_item].prem_by_id == data.meta.items[a].id);
    }

    let mut first_tile_index: u32 = 0;
    for c in 0..AVIF_ITEM_CATEGORY_COUNT {
        data.tile_infos[c].first_tile_index = first_tile_index;
        first_tile_index += data.tile_infos[c].tile_count;
    }

    // Sanity check tiles
    for tile in &data.tiles {
        for sample in &tile.input.samples {
            if sample.size == 0 {
                // Every sample must have some data
                return AvifResult::BmffParseFailed;
            }

            if tile.input.item_category == AvifItemCategory::Color {
                decoder.io_stats.color_obu_size =
                    decoder.io_stats.color_obu_size.wrapping_add(sample.size);
            } else if tile.input.item_category == AvifItemCategory::Alpha {
                decoder.io_stats.alpha_obu_size =
                    decoder.io_stats.alpha_obu_size.wrapping_add(sample.size);
            }
        }
    }

    {
        let properties = color_properties.get_from(&data.tracks, &data.meta);
        avif_checkres!(avif_read_color_properties(
            io,
            properties,
            image,
            Some(&mut data.cicp_set)
        ));
    }

    let properties = color_properties.get(data);
    if let Some(clli_prop) = avif_property_array_find(properties, b"clli") {
        image.clli = clli_prop.u.clli;
    }

    // Transformations
    if let Some(pasp_prop) = avif_property_array_find(properties, b"pasp") {
        image.transform_flags |= AVIF_TRANSFORM_PASP;
        image.pasp = pasp_prop.u.pasp;
    }
    if let Some(clap_prop) = avif_property_array_find(properties, b"clap") {
        image.transform_flags |= AVIF_TRANSFORM_CLAP;
        image.clap = clap_prop.u.clap;
    }
    if let Some(irot_prop) = avif_property_array_find(properties, b"irot") {
        image.transform_flags |= AVIF_TRANSFORM_IROT;
        image.irot = irot_prop.u.irot;
    }
    if let Some(imir_prop) = avif_property_array_find(properties, b"imir") {
        image.transform_flags |= AVIF_TRANSFORM_IMIR;
        image.imir = imir_prop.u.imir;
    }

    if !data.cicp_set && !data.tiles.is_empty() && !data.tiles[0].input.samples.is_empty() {
        let first_tile_codec_type = data.tiles[0].codec_type;
        let sample_size = data.tiles[0].input.samples[0].size;

        // Harvest CICP from the AV1's sequence header, which should be very close to the front
        // of the first sample. Read in successively larger chunks until we successfully parse the sequence.
        const SEARCH_SAMPLE_CHUNK_INCREMENT: usize = 64;
        const SEARCH_SAMPLE_SIZE_MAX: usize = 4096;
        let mut search_sample_size: usize = 0;
        loop {
            search_sample_size += SEARCH_SAMPLE_CHUNK_INCREMENT;
            if search_sample_size > sample_size {
                search_sample_size = sample_size;
            }

            let prepare_result =
                avif_decoder_prepare_sample(data, diag, io, 0, 0, search_sample_size);
            if prepare_result != AvifResult::Ok {
                return prepare_result;
            }

            let mut sequence_header = AvifSequenceHeader::default();
            let sample = &data.tiles[0].input.samples[0];
            if avif_sequence_header_parse(
                &mut sequence_header,
                sample_bytes(sample, &data.meta),
                first_tile_codec_type,
            ) {
                data.cicp_set = true;
                image.color_primaries = sequence_header.color_primaries;
                image.transfer_characteristics = sequence_header.transfer_characteristics;
                image.matrix_coefficients = sequence_header.matrix_coefficients;
                image.yuv_range = sequence_header.range;
                break;
            }
            if !(search_sample_size != sample_size && search_sample_size < SEARCH_SAMPLE_SIZE_MAX) {
                break;
            }
        }
    }

    let properties = color_properties.get(data);
    avif_checkres!(avif_read_codec_config_property(
        image,
        properties,
        color_codec_type
    ));

    AvifResult::Ok
}

/// Translation of `avifDecoderPrepareTiles()`.
fn avif_decoder_prepare_tiles(
    data: &mut AvifDecoderData,
    diag: &AvifDiagnostics,
    io: &mut dyn AvifIo,
    next_image_index: u32,
    info: &AvifTileInfo,
) -> AvifResult {
    for tile_index in info.decoded_tile_count..info.tile_count {
        let tile = (info.first_tile_index + tile_index) as usize;

        if next_image_index as usize >= data.tiles[tile].input.samples.len() {
            return AvifResult::NoImagesRemaining;
        }

        let prepare_result =
            avif_decoder_prepare_sample(data, diag, io, tile, next_image_index as usize, 0);
        if prepare_result != AvifResult::Ok {
            return prepare_result;
        }
    }
    AvifResult::Ok
}

/// Translation of `avifImageLimitedToFullAlpha()`.
fn avif_image_limited_to_full_alpha(image: &mut AvifImage) -> AvifResult {
    if image.image_owns_alpha_plane {
        return AvifResult::NotImplemented;
    }

    let alpha_plane = image.alpha_plane.take();
    let alpha_row_bytes = image.alpha_row_bytes as usize;

    // We cannot do the range conversion in place since it will modify the
    // codec's internal frame buffers. Allocate memory for the conversion.
    image.alpha_row_bytes = 0;
    let allocation_result = avif_image_allocate_planes(image, AVIF_PLANES_A);
    if allocation_result != AvifResult::Ok {
        return allocation_result;
    }
    let Some(alpha_plane) = alpha_plane else {
        return AvifResult::Ok;
    };
    let dst_row_bytes = image.alpha_row_bytes as usize;
    let (width, height, depth) = (image.width as usize, image.height as usize, image.depth);
    let Some(dst) = image.alpha_plane.as_mut() else {
        return AvifResult::OutOfMemory;
    };

    if depth > 8 {
        let src = alpha_plane.u16s();
        let dst = dst.u16s_mut();
        for j in 0..height {
            let src_row = &src[j * alpha_row_bytes / 2..];
            let dst_row = &mut dst[j * dst_row_bytes / 2..];
            for i in 0..width {
                let src_alpha = src_row[i] as i32;
                let dst_alpha = avif_limited_to_full_y(depth, src_alpha);
                dst_row[i] = dst_alpha as u16;
            }
        }
    } else {
        let src = alpha_plane.u8s();
        let dst = dst.u8s_mut();
        for j in 0..height {
            let src_row = &src[j * alpha_row_bytes..];
            let dst_row = &mut dst[j * dst_row_bytes..];
            for i in 0..width {
                let src_alpha = src_row[i] as i32;
                let dst_alpha = avif_limited_to_full_y(depth, src_alpha);
                dst_row[i] = dst_alpha as u8;
            }
        }
    }
    AvifResult::Ok
}

/// Translation of `avifGetErrorForItemCategory()`.
fn avif_get_error_for_item_category(item_category: AvifItemCategory) -> AvifResult {
    if avif_is_alpha(item_category) {
        AvifResult::DecodeAlphaFailed
    } else {
        AvifResult::DecodeColorFailed
    }
}

/// Translation of `avifDecoderDecodeTiles()`.
fn avif_decoder_decode_tiles(
    decoder: &mut AvifDecoder,
    next_image_index: u32,
    info_index: usize,
) -> AvifResult {
    let (max_threads, image_size_limit, image_dimension_limit, allow_incremental) = (
        decoder.max_threads,
        decoder.image_size_limit,
        decoder.image_dimension_limit,
        decoder.allow_incremental,
    );
    let diag = &decoder.diag;
    let Some(data) = decoder.data.as_deref_mut() else {
        return AvifResult::NoContent;
    };
    let Some(dst_image) = decoder.image.as_deref_mut() else {
        return AvifResult::NoContent;
    };
    let old_decoded_tile_count = data.tile_infos[info_index].decoded_tile_count;
    for tile_index in old_decoded_tile_count..data.tile_infos[info_index].tile_count {
        let tile_at = (data.tile_infos[info_index].first_tile_index + tile_index) as usize;

        {
            let AvifDecoderData {
                tiles,
                meta,
                codec,
                codec_alpha,
                ..
            } = &mut *data;
            let AvifTile {
                input,
                codec: tile_codec,
                image: tile_image,
                ..
            } = &mut tiles[tile_at];

            let sample = &input.samples[next_image_index as usize];
            if sample.data.size() < sample.size {
                avif_assert_or_return!(allow_incremental);
                // Data is missing but there is no error yet. Output available pixel rows.
                return AvifResult::Ok;
            }

            let mut is_limited_range_alpha = false;
            let tile_codec = match tile_codec {
                TileCodec::None => None,
                TileCodec::Shared => codec.as_deref_mut(),
                TileCodec::SharedAlpha => codec_alpha.as_deref_mut(),
                TileCodec::Own(c) => Some(&mut **c),
            };
            let decoded = match tile_codec {
                Some(tile_codec) => tile_codec.get_next_image(
                    max_threads,
                    image_size_limit,
                    sample,
                    sample_bytes(sample, meta),
                    avif_is_alpha(input.item_category),
                    &mut is_limited_range_alpha,
                    tile_image,
                ),
                None => false,
            };
            if !decoded {
                diag_printf!(Some(diag), "tile->codec->getNextImage() failed");
                return avif_get_error_for_item_category(input.item_category);
            }

            // Section 2.3.4 of AV1 Codec ISO Media File Format Binding v1.2.0 says:
            //   the full_range_flag in the colr box shall match the color_range
            //   flag in the Sequence Header OBU.
            // See https://aomediacodec.github.io/av1-isobmff/v1.2.0.html#av1codecconfigurationbox-semantics.
            // If a 'colr' box of colour_type 'nclx' was parsed, a mismatch between
            // the 'colr' decoder->image->yuvRange and the AV1 OBU
            // tile->image->yuvRange should be treated as an error.
            // However codec_svt.c was not encoding the color_range field for
            // multiple years, so there probably are files in the wild that will
            // fail decoding if this is enforced. Thus this pattern is allowed.
            // Section 12.1.5.1 of ISO 14496-12 (ISOBMFF) says:
            //   If colour information is supplied in both this [colr] box, and also
            //   in the video bitstream, this box takes precedence, and over-rides
            //   the information in the bitstream.
            // So decoder->image->yuvRange is kept because it was either the 'colr'
            // value set when the 'colr' box was parsed, or it was the AV1 OBU value
            // extracted from the sequence header OBU of the first tile of the first
            // frame (if no 'colr' box of colour_type 'nclx' was found).

            // Alpha plane with limited range is not allowed by the latest revision
            // of the specification. However, it was allowed in version 1.0.0 of the
            // specification. To allow such files, simply convert the alpha plane to
            // full range.
            if avif_is_alpha(input.item_category) && is_limited_range_alpha {
                let result = avif_image_limited_to_full_alpha(tile_image);
                if result != AvifResult::Ok {
                    diag_printf!(Some(diag), "avifImageLimitedToFullAlpha failed");
                    return result;
                }
            }
        }

        // Scale the decoded image so that it corresponds to this tile's output dimensions
        {
            let tile = &mut data.tiles[tile_at];
            if (tile.width != tile.image.width) || (tile.height != tile.image.height) {
                if avif_image_scale_with_limit(
                    &mut tile.image,
                    tile.width,
                    tile.height,
                    image_size_limit,
                    image_dimension_limit,
                    Some(diag),
                ) != AvifResult::Ok
                {
                    return avif_get_error_for_item_category(tile.input.item_category);
                }
            }
        }

        data.tile_infos[info_index].decoded_tile_count += 1;

        let info = data.tile_infos[info_index];
        let is_grid = (info.grid.rows > 0) && (info.grid.columns > 0);
        let steal_planes = !is_grid;

        if !steal_planes {
            if tile_index == 0 {
                avif_checkres!(avif_decoder_data_allocate_image_planes(
                    data,
                    &info,
                    dst_image,
                    Some(diag)
                ));
            }
            avif_checkres!(avif_decoder_data_copy_tile_to_image(
                data,
                &info,
                dst_image,
                tile_at,
                tile_index,
                Some(diag)
            ));
        } else {
            avif_assert_or_return!(info.tile_count == 1);
            avif_assert_or_return!(tile_index == 0);
            let tile = &mut data.tiles[tile_at];
            let src = &mut tile.image;

            // (switch (tile->input->itemCategory) { default: ... })
            if (dst_image.width != src.width)
                || (dst_image.height != src.height)
                || (dst_image.depth != src.depth)
            {
                if avif_is_alpha(tile.input.item_category) {
                    diag_printf!(
                        Some(diag),
                        "The color image item does not match the alpha image item in width, height, or bit depth"
                    );
                    return AvifResult::DecodeAlphaFailed;
                }
                avif_image_free_planes(dst_image, AVIF_PLANES_ALL);

                dst_image.width = src.width;
                dst_image.height = src.height;
                dst_image.depth = src.depth;
            }

            if avif_is_alpha(tile.input.item_category) {
                avif_image_steal_planes(dst_image, src, AVIF_PLANES_A);
            } else {
                // AVIF_ITEM_COLOR
                avif_image_steal_planes(dst_image, src, AVIF_PLANES_YUV);
            }
        }
    }
    AvifResult::Ok
}

/// Returns AVIF_FALSE if there is currently a partially decoded frame.
/// Translation of `avifDecoderDataFrameFullyDecoded()`.
fn avif_decoder_data_frame_fully_decoded(data: &AvifDecoderData) -> bool {
    for c in 0..AVIF_ITEM_CATEGORY_COUNT {
        if data.tile_infos[c].decoded_tile_count != data.tile_infos[c].tile_count {
            return false;
        }
    }
    true
}

/// Translation of `avifDecoderNextImage()`.
pub(crate) fn avif_decoder_next_image(
    decoder: &mut AvifDecoder,
    io: &mut dyn AvifIo,
) -> AvifResult {
    diag::clear_error(Some(&decoder.diag));

    match decoder.data.as_deref() {
        Some(data) if !data.tiles.is_empty() => {}
        _ => {
            // Nothing has been parsed yet
            return AvifResult::NoContent;
        }
    }

    // (the reader is always set: it is an argument)

    {
        let Some(data) = decoder.data.as_deref_mut() else {
            return AvifResult::NoContent;
        };
        if avif_decoder_data_frame_fully_decoded(data) {
            // A frame was decoded during the last avifDecoderNextImage() call.
            for c in 0..AVIF_ITEM_CATEGORY_COUNT {
                data.tile_infos[c].decoded_tile_count = 0;
            }
        }

        avif_assert_or_return!(
            data.tiles.len() as u32
                == (data.tile_infos[AVIF_ITEM_CATEGORY_COUNT - 1].first_tile_index
                    + data.tile_infos[AVIF_ITEM_CATEGORY_COUNT - 1].tile_count)
        );
    }

    let next_image_index = (decoder.image_index.wrapping_add(1)) as u32;

    // Ensure that we have created the codecs before proceeding with the decoding.
    if decoder
        .data
        .as_deref()
        .is_some_and(|d| matches!(d.tiles[0].codec, TileCodec::None))
    {
        avif_checkres!(avif_decoder_create_codecs(decoder));
    }

    // Acquire all sample data for the current image first, allowing for any read call to bail out
    // with AVIF_RESULT_WAITING_ON_IO harmlessly / idempotently, unless decoder->allowIncremental.
    let mut prepare_tile_result = [AvifResult::Ok; AVIF_ITEM_CATEGORY_COUNT];
    for c in 0..AVIF_ITEM_CATEGORY_COUNT {
        let Some(data) = decoder.data.as_deref_mut() else {
            return AvifResult::NoContent;
        };
        let info = data.tile_infos[c];
        prepare_tile_result[c] =
            avif_decoder_prepare_tiles(data, &decoder.diag, io, next_image_index, &info);
        if !decoder.allow_incremental || (prepare_tile_result[c] != AvifResult::WaitingOnIo) {
            avif_checkres!(prepare_tile_result[c]);
        }
    }

    // Decode all available color tiles now, then all available alpha tiles, then all available bit
    // depth extension tiles. The order of appearance of the tiles in the bitstream is left to the
    // encoder's choice, and decoding as many as possible of each category in parallel is beneficial
    // for incremental decoding, as pixel rows need all channels to be decoded before being
    // accessible to the user.
    for c in 0..AVIF_ITEM_CATEGORY_COUNT {
        avif_checkres!(avif_decoder_decode_tiles(decoder, next_image_index, c));
    }

    let Some(data) = decoder.data.as_deref() else {
        return AvifResult::NoContent;
    };
    if !avif_decoder_data_frame_fully_decoded(data) {
        avif_assert_or_return!(decoder.allow_incremental);
        // The image is not completely decoded. There should be no error unrelated to missing bytes,
        // and at least some missing bytes.
        let mut first_non_ok_result = AvifResult::Ok;
        for c in 0..AVIF_ITEM_CATEGORY_COUNT {
            avif_assert_or_return!(
                prepare_tile_result[c] == AvifResult::Ok
                    || prepare_tile_result[c] == AvifResult::WaitingOnIo
            );
            if first_non_ok_result == AvifResult::Ok {
                first_non_ok_result = prepare_tile_result[c];
            }
        }
        avif_assert_or_return!(first_non_ok_result != AvifResult::Ok);
        // Return the "not enough bytes" status now instead of moving on to the next frame.
        return AvifResult::WaitingOnIo;
    }
    for c in 0..AVIF_ITEM_CATEGORY_COUNT {
        avif_assert_or_return!(prepare_tile_result[c] == AvifResult::Ok);
    }

    // Only advance decoder->imageIndex once the image is completely decoded, so that
    // avifDecoderNthImage(decoder, decoder->imageIndex + 1) is equivalent to avifDecoderNextImage(decoder)
    // if the previous call to avifDecoderNextImage() returned AVIF_RESULT_WAITING_ON_IO.
    decoder.image_index = next_image_index as i32;
    // The decoded tile counts will be reset to 0 the next time avifDecoderNextImage() is called,
    // for avifDecoderDecodedRowCount() to work until then.
    if data.source_sample_table.is_some() {
        // Decoding from a track! Provide timing information.

        let mut timing = AvifImageTiming::default();
        let timing_result =
            avif_decoder_nth_image_timing(decoder, decoder.image_index as u32, &mut timing);
        if timing_result != AvifResult::Ok {
            return timing_result;
        }
        decoder.image_timing = timing;
    }
    AvifResult::Ok
}

/// Timing helper - This does not change the current image or invoke the codec (safe to call repeatedly)
/// This function may be used after a successful call (AVIF_RESULT_OK) to avifDecoderParse().
///
/// Translation of `avifDecoderNthImageTiming()`.
pub(crate) fn avif_decoder_nth_image_timing(
    decoder: &AvifDecoder,
    frame_index: u32,
    out_timing: &mut AvifImageTiming,
) -> AvifResult {
    let Some(data) = decoder.data.as_deref() else {
        // Nothing has been parsed yet
        return AvifResult::NoContent;
    };

    if (frame_index > i32::MAX as u32) || (frame_index as i32 >= decoder.image_count) {
        // Impossible index
        return AvifResult::NoImagesRemaining;
    }

    let Some(source_sample_table) = data
        .source_sample_table
        .and_then(|t| data.tracks[t].sample_table.as_deref())
    else {
        // There isn't any real timing associated with this decode, so
        // just hand back the defaults chosen in avifDecoderReset().
        *out_timing = decoder.image_timing;
        return AvifResult::Ok;
    };

    out_timing.timescale = decoder.timescale;
    out_timing.pts_in_timescales = 0;
    for image_index in 0..frame_index {
        out_timing.pts_in_timescales =
            out_timing
                .pts_in_timescales
                .wrapping_add(
                    avif_sample_table_get_image_delta(source_sample_table, image_index) as u64,
                );
    }
    out_timing.duration_in_timescales =
        avif_sample_table_get_image_delta(source_sample_table, frame_index) as u64;

    if out_timing.timescale > 0 {
        out_timing.pts = out_timing.pts_in_timescales as f64 / out_timing.timescale as f64;
        out_timing.duration =
            out_timing.duration_in_timescales as f64 / out_timing.timescale as f64;
    } else {
        out_timing.pts = 0.0;
        out_timing.duration = 0.0;
    }
    AvifResult::Ok
}

/// Translation of `avifDecoderNthImage()`.
pub(crate) fn avif_decoder_nth_image(
    decoder: &mut AvifDecoder,
    io: &mut dyn AvifIo,
    frame_index: u32,
) -> AvifResult {
    diag::clear_error(Some(&decoder.diag));

    if decoder.data.is_none() {
        // Nothing has been parsed yet
        return AvifResult::NoContent;
    }

    if (frame_index > i32::MAX as u32) || (frame_index as i32 >= decoder.image_count) {
        // Impossible index
        return AvifResult::NoImagesRemaining;
    }

    let requested_index = frame_index as i32;
    if requested_index == (decoder.image_index.wrapping_add(1)) {
        // It's just the next image (already partially decoded or not at all), nothing special here
        return avif_decoder_next_image(decoder, io);
    }

    if requested_index == decoder.image_index {
        if decoder
            .data
            .as_deref()
            .is_some_and(avif_decoder_data_frame_fully_decoded)
        {
            // The current fully decoded image (decoder->imageIndex) is requested, nothing to do
            return AvifResult::Ok;
        }
        // The next image (decoder->imageIndex + 1) is partially decoded but
        // the previous image (decoder->imageIndex) is requested.
        // Fall through to resetting the decoder data and start decoding from
        // the nearest key frame.
    }

    let nearest_key_frame = avif_decoder_nearest_keyframe(decoder, frame_index) as i32;
    if (nearest_key_frame > (decoder.image_index.wrapping_add(1)))
        || (requested_index <= decoder.image_index)
    {
        // If we get here, we need to start decoding from the nearest key frame.
        // So discard the unused decoder state and its previous frames. This
        // will force the setup of new AV1 decoder (avifCodec) instances in
        // avifDecoderNextImage().
        decoder.image_index = nearest_key_frame - 1; // prepare to read nearest keyframe
        if let Some(data) = decoder.data.as_deref_mut() {
            avif_decoder_data_reset_codec(data);
        }
    }
    loop {
        let result = avif_decoder_next_image(decoder, io);
        if result != AvifResult::Ok {
            return result;
        }

        if requested_index == decoder.image_index {
            break;
        }
    }
    AvifResult::Ok
}

/// Keyframe information. Translation of `avifDecoderIsKeyframe()`.
pub(crate) fn avif_decoder_is_keyframe(decoder: &AvifDecoder, frame_index: u32) -> bool {
    let Some(data) = decoder.data.as_deref().filter(|d| !d.tiles.is_empty()) else {
        // Nothing has been parsed yet
        return false;
    };

    // *All* tiles for the requested frameIndex must be keyframes in order for
    //  avifDecoderIsKeyframe() to return true, otherwise we may seek to a frame in which the color
    //  planes are a keyframe but the alpha plane isn't a keyframe, which will cause an alpha plane
    //  decode failure.
    for tile in &data.tiles {
        if (frame_index as usize >= tile.input.samples.len())
            || !tile.input.samples[frame_index as usize].sync
        {
            return false;
        }
    }
    true
}

/// "nearest" keyframe means the keyframe prior to this frame index
/// (returns frameIndex if it is a keyframe). Translation of
/// `avifDecoderNearestKeyframe()`.
pub(crate) fn avif_decoder_nearest_keyframe(decoder: &AvifDecoder, mut frame_index: u32) -> u32 {
    if decoder.data.is_none() {
        // Nothing has been parsed yet
        return 0;
    }

    while frame_index != 0 {
        if avif_decoder_is_keyframe(decoder, frame_index) {
            break;
        }
        frame_index -= 1;
    }
    frame_index
}

/// Returns the number of available rows in decoder->image given a color
/// or alpha subimage. Translation of `avifGetDecodedRowCount()`.
fn avif_get_decoded_row_count(
    data: &AvifDecoderData,
    info: &AvifTileInfo,
    image: &AvifImage,
) -> u32 {
    if info.decoded_tile_count == info.tile_count {
        return image.height;
    }
    if info.decoded_tile_count == 0 {
        return 0;
    }

    if (info.grid.rows > 0) && (info.grid.columns > 0) {
        // Grid of AVIF tiles (not to be confused with AV1 tiles).
        let tile_height = data.tiles[info.first_tile_index as usize].height;
        ((info.decoded_tile_count / info.grid.columns).wrapping_mul(tile_height)).min(image.height)
    } else {
        // Non-grid image.
        image.height
    }
}

/// Translation of `avifDecoderDecodedRowCount()`.
pub(crate) fn avif_decoder_decoded_row_count(decoder: &AvifDecoder) -> u32 {
    let (Some(image), Some(data)) = (decoder.image.as_deref(), decoder.data.as_deref()) else {
        return 0;
    };
    let mut min_row_count = image.height;
    for c in 0..AVIF_ITEM_CATEGORY_COUNT {
        let row_count = avif_get_decoded_row_count(data, &data.tile_infos[c], image);
        min_row_count = min_row_count.min(row_count);
    }
    min_row_count
}

/// Simple interface to decode a single image, independent of the decoder
/// afterwards. Translation of `avifDecoderRead()`.
pub(crate) fn avif_decoder_read(
    decoder: &mut AvifDecoder,
    io: &mut dyn AvifIo,
    image: &mut AvifImage,
) -> AvifResult {
    let mut result = avif_decoder_parse(decoder, io);
    if result != AvifResult::Ok {
        return result;
    }
    result = avif_decoder_next_image(decoder, io);
    if result != AvifResult::Ok {
        return result;
    }
    // If decoder->image->imageOwnsYUVPlanes is true and decoder->image is not used after this call,
    // the ownership of the planes in decoder->image could be transferred here instead of copied.
    // However most codec_*.c implementations allocate the output buffer themselves and return a
    // view, unless some postprocessing is applied (container-level grid reconstruction for
    // example), so the first condition rarely holds.
    // The second condition does not hold either: it is not required by the documentation in avif.h.
    match decoder.image.as_deref() {
        Some(src) => avif_image_copy(image, src, AVIF_PLANES_ALL),
        None => AvifResult::NoContent,
    }
}

// (avifDecoderReadMemory() and avifDecoderReadFile() use libavif's own
// readers, which are not translated.)
