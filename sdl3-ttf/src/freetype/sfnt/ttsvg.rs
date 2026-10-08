// Rust translation of src/sfnt/ttsvg.c from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2022-2023 by David Turner, Robert Wilhelm, Werner Lemberg, and
// Moazin Khatti.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! OpenType SVG Color (specification).
//!
//! 'SVG' table specification:
//!
//!    <https://docs.microsoft.com/en-us/typography/opentype/spec/svg>

use super::super::base::ftobjs::*;
use super::super::base::ftstream::*;
use super::super::fttypes::*;
use super::super::gzip::ftgzip::ft_gzip_uncompress;
use super::super::tttables::*;
use super::super::tttypes::*;
use super::ttload::tt_face_goto_table;

/* NOTE: These table sizes are given by the specification. */
const SVG_TABLE_HEADER_SIZE: FtULong = 10;
const SVG_DOCUMENT_RECORD_SIZE: FtULong = 12;
const SVG_DOCUMENT_LIST_MINIMUM_SIZE: FtULong = 2 + SVG_DOCUMENT_RECORD_SIZE;
const SVG_MINIMUM_SIZE: FtULong = SVG_TABLE_HEADER_SIZE + SVG_DOCUMENT_LIST_MINIMUM_SIZE;

/// `Svg`
#[derive(Debug, Default)]
pub struct Svg {
    pub version: FtUShort,     /* table version (starting at 0)  */
    pub num_entries: FtUShort, /* number of SVG document records */

    /// offset of the start of SVG Document List in `table`
    pub svg_doc_list: usize,

    pub table: Vec<u8>, /* memory that backs up SVG */
    pub table_size: FtULong,
}

/// `tt_face_load_svg`
pub fn tt_face_load_svg(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    let r = (|| -> FtResult<()> {
        let table_size = tt_face_goto_table(face, TTAG_SVG as FtULong, stream)?;

        if table_size < SVG_MINIMUM_SIZE {
            return Err(FT_ERR_INVALID_TABLE);
        }

        let table = stream.extract_frame(table_size)?;

        /* Allocate memory for the SVG object */
        let mut svg = Box::<Svg>::default();

        let mut p = 0usize;
        svg.version = ft_next_ushort(&table, &mut p);
        let offset_to_svg_document_list = ft_next_ulong(&table, &mut p) as FtULong;

        if offset_to_svg_document_list < SVG_TABLE_HEADER_SIZE
            || offset_to_svg_document_list > table_size - SVG_DOCUMENT_LIST_MINIMUM_SIZE
        {
            return Err(FT_ERR_INVALID_TABLE);
        }

        svg.svg_doc_list = offset_to_svg_document_list as usize;

        let mut p = svg.svg_doc_list;
        svg.num_entries = ft_next_ushort(&table, &mut p);

        if offset_to_svg_document_list + 2 + svg.num_entries as FtULong * SVG_DOCUMENT_RECORD_SIZE
            > table_size
        {
            return Err(FT_ERR_INVALID_TABLE);
        }

        svg.table = table;
        svg.table_size = table_size;

        face.svg = Some(svg);
        face.root.face_flags |= FT_FACE_FLAG_SVG;

        Ok(())
    })();

    if r.is_err() {
        /* NoSVG: */
        face.svg = None;
    }

    r
}

/// `tt_face_free_svg`
pub fn tt_face_free_svg(face: &mut TtFaceRec) {
    face.svg = None;
}

/// `Svg_doc`
#[derive(Debug, Clone, Copy, Default)]
struct SvgDoc {
    start_glyph_id: FtUShort,
    end_glyph_id: FtUShort,

    offset: FtULong,
    length: FtULong,
}

/// `extract_svg_doc`
fn extract_svg_doc(stream: &[u8], mut p: usize) -> SvgDoc {
    SvgDoc {
        start_glyph_id: ft_next_ushort(stream, &mut p),
        end_glyph_id: ft_next_ushort(stream, &mut p),
        offset: ft_next_ulong(stream, &mut p) as FtULong,
        length: ft_next_ulong(stream, &mut p) as FtULong,
    }
}

/// `compare_svg_doc`
fn compare_svg_doc(doc: SvgDoc, glyph_index: FtUInt) -> FtInt {
    if glyph_index < doc.start_glyph_id as FtUInt {
        -1
    } else if glyph_index > doc.end_glyph_id as FtUInt {
        1
    } else {
        0
    }
}

/// `find_doc`; returns `(doc_offset, doc_length, start_glyph, end_glyph)`.
fn find_doc(
    table: &[u8],
    document_records: usize,
    num_entries: FtUShort,
    glyph_index: FtUInt,
) -> FtResult<(FtULong, FtULong, FtUShort, FtUShort)> {
    let mut mid_doc = SvgDoc::default(); /* pacify compiler */

    let mut found = false;
    let mut start_index: FtUInt = 0;
    let mut end_index: FtUInt = (num_entries as FtUInt).wrapping_sub(1);

    let rec =
        |i: FtUInt, size: usize| document_records.wrapping_add((i as usize).wrapping_mul(size));

    /* search algorithm */
    if num_entries == 0 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    let start_doc = extract_svg_doc(table, rec(start_index, 12));
    let end_doc = extract_svg_doc(table, rec(end_index, 12));

    if compare_svg_doc(start_doc, glyph_index) == -1 || compare_svg_doc(end_doc, glyph_index) == 1 {
        return Err(FT_ERR_INVALID_GLYPH_INDEX);
    }

    while start_index <= end_index {
        let i = start_index.wrapping_add(end_index) / 2;
        mid_doc = extract_svg_doc(table, rec(i, 12));
        let comp_res = compare_svg_doc(mid_doc, glyph_index);

        /* FIXME (upstream): C re-reads `start_doc` and `end_doc` here with */
        /* a record stride of 4 instead of 12 (the values are unused), and  */
        /* `i - 1` wraps around for unsorted records, making C read out of  */
        /* bounds; the reads here return zero beyond the table instead.     */
        if comp_res == 1 {
            start_index = i.wrapping_add(1);
        } else if comp_res == -1 {
            end_index = i.wrapping_sub(1);
        } else {
            found = true;
            break;
        }
    }
    /* search algorithm end */

    if !found {
        Err(FT_ERR_INVALID_GLYPH_INDEX)
    } else {
        Ok((
            mid_doc.offset,
            mid_doc.length,
            mid_doc.start_glyph_id,
            mid_doc.end_glyph_id,
        ))
    }
}

/// `tt_face_load_svg_doc`: loads the SVG document of `glyph_index` into
/// the face's glyph slot (`glyph->other`).
pub fn tt_face_load_svg_doc(face: &mut TtFaceRec, glyph_index: FtUInt) -> FtResult<()> {
    let svg = face.svg.as_ref().expect("face without an SVG table");

    let doc_list = svg.svg_doc_list;

    let (doc_offset, mut doc_length, doc_start_glyph_id, doc_end_glyph_id) =
        find_doc(&svg.table, doc_list + 2, svg.num_entries, glyph_index)?;

    let doc_limit = svg.table_size - doc_list as FtULong;
    if doc_offset > doc_limit || doc_length > doc_limit - doc_offset {
        return Err(FT_ERR_INVALID_TABLE);
    }

    let start = doc_list + doc_offset as usize;
    let mut doc: Vec<u8>;
    let docs = &svg.table[start..start + doc_length as usize];

    if doc_length > 6 && docs[0] == 0x1F && docs[1] == 0x8B && docs[2] == 0x08 {
        /* FT_CONFIG_OPTION_USE_ZLIB */
        /*
         * Get the size of the original document.  This helps in allotting the
         * buffer to accommodate the uncompressed version.  The last 4 bytes
         * of the compressed document are equal to the original size modulo
         * 2^32.  Since the size of SVG documents is less than 2^32 bytes we
         * can use this accurately.  The four bytes are stored in
         * little-endian format.
         */
        let n = doc_length as usize;
        let mut uncomp_size: FtULong = (docs[n - 1] as FtULong) << 24
            | (docs[n - 2] as FtULong) << 16
            | (docs[n - 3] as FtULong) << 8
            | docs[n - 4] as FtULong;

        let mut uncomp_buffer = super::super::base::ftmemory::ft_qalloc(uncomp_size as FtLong)?;

        if ft_gzip_uncompress(&mut uncomp_buffer, &mut uncomp_size, docs).is_err() {
            return Err(FT_ERR_INVALID_TABLE);
        }

        face.root.glyph.internal.flags |= FT_GLYPH_OWN_GZIP_SVG;

        uncomp_buffer.truncate(uncomp_size as usize);
        doc = uncomp_buffer;
        doc_length = uncomp_size;
    } else {
        doc = Vec::new();
        if doc.try_reserve_exact(docs.len()).is_err() {
            return Err(FT_ERR_OUT_OF_MEMORY);
        }
        doc.extend_from_slice(docs);
    }

    let svg_document = FtSvgDocumentRec {
        svg_document: doc,
        svg_document_length: doc_length,

        metrics: face.root.size.metrics,
        units_per_EM: face.root.units_per_EM,

        start_glyph_id: doc_start_glyph_id,
        end_glyph_id: doc_end_glyph_id,

        transform: FtMatrix {
            xx: 0x10000,
            xy: 0,
            yx: 0,
            yy: 0x10000,
        },

        delta: FtVector { x: 0, y: 0 },
    };

    face.root.glyph.other = Some(Box::new(svg_document));

    Ok(())
}
