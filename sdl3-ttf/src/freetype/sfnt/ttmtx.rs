// Rust translation of src/sfnt/ttmtx.c from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2006-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Load the metrics tables common to TTF and OTF fonts (body).

use super::super::base::ftstream::FtStreamRec;
use super::super::fttypes::*;
use super::super::tttables::*;
use super::super::tttypes::*;
use super::ttload::tt_face_goto_table;

/* IMPORTANT: The TT_HoriHeader and TT_VertHeader structures should   */
/*            be identical except for the names of their fields,      */
/*            which are different.                                    */
/*                                                                    */
/*            This ensures that `tt_face_load_hmtx' is able to read   */
/*            both the horizontal and vertical headers.               */

/// `tt_face_load_hmtx`: Load the `hmtx' or `vmtx' table into a face
/// object.
pub fn tt_face_load_hmtx(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    vertical: bool,
) -> FtResult<()> {
    let tag = if vertical { TTAG_vmtx } else { TTAG_hmtx } as FtULong;

    let table_size = tt_face_goto_table(face, tag, stream)?;

    let pos = stream.pos();
    if vertical {
        face.vert_metrics_size = table_size;
        face.vert_metrics_offset = pos;
    } else {
        face.horz_metrics_size = table_size;
        face.horz_metrics_offset = pos;
    }

    Ok(())
}

/// `tt_face_load_hhea`: Load the `hhea' or 'vhea' table into a face
/// object.
pub fn tt_face_load_hhea(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    vertical: bool,
) -> FtResult<()> {
    if vertical {
        tt_face_goto_table(face, TTAG_vhea as FtULong, stream)?;
    } else {
        tt_face_goto_table(face, TTAG_hhea as FtULong, stream)?;
    }

    stream.enter_frame(36)?;
    let version = stream.get_ulong() as FtFixed;
    let ascender = stream.get_short();
    let descender = stream.get_short();
    let line_gap = stream.get_short();
    let advance_max = stream.get_ushort();
    let min_lsb = stream.get_short();
    let min_rsb = stream.get_short();
    let max_extent = stream.get_short();
    let caret_slope_rise = stream.get_short();
    let caret_slope_run = stream.get_short();
    let caret_offset = stream.get_short();
    let reserved = [
        stream.get_short(),
        stream.get_short(),
        stream.get_short(),
        stream.get_short(),
    ];
    let metric_data_format = stream.get_short();
    let number_of_metrics = stream.get_ushort();
    stream.exit_frame();

    if vertical {
        let header = &mut face.vertical;
        header.Version = version;
        header.Ascender = ascender;
        header.Descender = descender;
        header.Line_Gap = line_gap;
        header.advance_Height_Max = advance_max;
        header.min_Top_Side_Bearing = min_lsb;
        header.min_Bottom_Side_Bearing = min_rsb;
        header.yMax_Extent = max_extent;
        header.caret_Slope_Rise = caret_slope_rise;
        header.caret_Slope_Run = caret_slope_run;
        header.caret_Offset = caret_offset;
        header.Reserved = reserved;
        header.metric_Data_Format = metric_data_format;
        header.number_Of_VMetrics = number_of_metrics;
    } else {
        let header = &mut face.horizontal;
        header.Version = version;
        header.Ascender = ascender;
        header.Descender = descender;
        header.Line_Gap = line_gap;
        header.advance_Width_Max = advance_max;
        header.min_Left_Side_Bearing = min_lsb;
        header.min_Right_Side_Bearing = min_rsb;
        header.xMax_Extent = max_extent;
        header.caret_Slope_Rise = caret_slope_rise;
        header.caret_Slope_Run = caret_slope_run;
        header.caret_Offset = caret_offset;
        header.Reserved = reserved;
        header.metric_Data_Format = metric_data_format;
        header.number_Of_HMetrics = number_of_metrics;
    }

    Ok(())
}

/// `tt_face_get_metrics`: Return the horizontal or vertical metrics in
/// font units for a given glyph.  The values are the left side bearing
/// (top side bearing for vertical metrics) and advance width (advance
/// height for vertical layout); returns `(bearing, advance)`.
pub fn tt_face_get_metrics(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    vertical: bool,
    gindex: FtUInt,
) -> (FtShort, FtUShort) {
    let mut abearing: FtShort = 0;
    let mut aadvance: FtUShort = 0;

    let (k, mut table_pos, table_size) = if vertical {
        (
            face.vertical.number_Of_VMetrics,
            face.vert_metrics_offset,
            face.vert_metrics_size,
        )
    } else {
        (
            face.horizontal.number_Of_HMetrics,
            face.horz_metrics_offset,
            face.horz_metrics_size,
        )
    };

    let table_end = table_pos + table_size;

    'nodata: {
        if k > 0 {
            if gindex < k as FtUInt {
                table_pos += 4 * gindex as FtULong;
                if table_pos + 4 > table_end {
                    break 'nodata;
                }

                if stream.seek(table_pos).is_err() {
                    break 'nodata;
                }
                match stream.read_ushort() {
                    Ok(v) => aadvance = v,
                    Err(_) => break 'nodata,
                }
                match stream.read_short() {
                    Ok(v) => abearing = v,
                    Err(_) => break 'nodata,
                }
            } else {
                table_pos += 4 * (k as FtULong - 1);
                if table_pos + 2 > table_end {
                    break 'nodata;
                }

                if stream.seek(table_pos).is_err() {
                    break 'nodata;
                }
                match stream.read_ushort() {
                    Ok(v) => aadvance = v,
                    Err(_) => break 'nodata,
                }

                table_pos += 4 + 2 * (gindex as FtULong - k as FtULong);
                if table_pos + 2 > table_end {
                    abearing = 0;
                } else if stream.seek(table_pos).is_err() {
                    abearing = 0;
                } else if let Ok(v) = stream.read_short() {
                    abearing = v;
                }
            }

            return finish(face, stream, vertical, gindex, abearing, aadvance);
        }
    }

    /* NoData: */
    abearing = 0;
    aadvance = 0;

    finish(face, stream, vertical, gindex, abearing, aadvance)
}

fn finish(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    vertical: bool,
    gindex: FtUInt,
    abearing: FtShort,
    aadvance: FtUShort,
) -> (FtShort, FtUShort) {
    use super::super::truetype::ttgxvar;

    if face.blend.is_some() {
        let mut a = aadvance as FtInt;
        let b = abearing as FtInt;

        /* (the TrueType driver's metrics variations service sets only */
        /* `hadvance_adjust' and `vadvance_adjust'; the bearing        */
        /* adjustment functions are NULL)                              */
        if vertical {
            let _ = ttgxvar::tt_vadvance_adjust(face, stream, gindex, &mut a);
        } else {
            let _ = ttgxvar::tt_hadvance_adjust(face, stream, gindex, &mut a);
        }

        return (b as FtShort, a as FtUShort);
    }

    (abearing, aadvance)
}
