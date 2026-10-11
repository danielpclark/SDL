// Rust translation of src/pcf/pcfread.c and src/pcf/pcfread.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
//
// FreeType font driver for pcf fonts
//
// Copyright 2000-2010, 2012-2014 by
// Francesco Zappa Nardelli
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.  IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
//
// This is an altered (translated) version of the original software.

//! The PCF font loader.
//!
//! The `FT_READ_*` macros set `error` and read 0 when they fail, and the
//! code relies on that in places; `rd` does the same.

use std::sync::Arc;

use super::super::base::ftcalc::ft_mul_div;
use super::super::base::ftmemory::{ft_new_array, ft_qalloc};
use super::super::base::ftstream::{ft_peek_ushort, ft_peek_ushort_le, FtStreamRec};
use super::super::fttypes::*;
use super::pcf::*;

/// The value of an `FT_READ_*` macro, setting `error` as it does.
fn rd<T: Default>(r: FtResult<T>, error: &mut FtError) -> T {
    match r {
        Ok(v) => {
            *error = FT_ERR_OK;
            v
        }
        Err(e) => {
            *error = e;
            T::default()
        }
    }
}

/// `pcf_toc_header`
fn read_pcf_toc_header(stream: &mut FtStreamRec, toc: &mut PcfTocRec) -> FtResult<()> {
    stream.enter_frame(8)?;
    toc.version = stream.get_ulong_le() as FtULong;
    toc.count = stream.get_ulong_le() as FtULong;
    stream.exit_frame();
    Ok(())
}

/// `pcf_table_header`
fn read_pcf_table_header(stream: &mut FtStreamRec, table: &mut PcfTableRec) -> FtResult<()> {
    stream.enter_frame(16)?;
    table.type_ = stream.get_ulong_le() as FtULong;
    table.format = stream.get_ulong_le() as FtULong;
    table.size = stream.get_ulong_le() as FtULong; /* rounded up to a multiple of 4 */
    table.offset = stream.get_ulong_le() as FtULong;
    stream.exit_frame();
    Ok(())
}

/// `pcf_read_TOC`
fn pcf_read_toc(stream: &mut FtStreamRec, face: &mut PcfFaceRec) -> FtResult<()> {
    let toc = &mut face.toc;

    if stream.seek(0).is_err() || read_pcf_toc_header(stream, toc).is_err() {
        return Err(FT_ERR_CANNOT_OPEN_RESOURCE);
    }

    if toc.version != PCF_FILE_VERSION || toc.count == 0 {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    if stream.size < 16 {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    /* we need 16 bytes per TOC entry, */
    /* and there can be most 9 tables  */
    if toc.count > (stream.size >> 4) || toc.count > 9 {
        toc.count = (stream.size >> 4).min(9);
    }

    toc.tables = ft_new_array(toc.count as FtLong)?;

    let error = (|| -> FtResult<()> {
        let toc = &mut face.toc;
        for n in 0..toc.count as usize {
            read_pcf_table_header(stream, &mut toc.tables[n])?;
        }

        /* Sort tables and check for overlaps.  Because they are almost      */
        /* always ordered already, an in-place bubble sort with simultaneous */
        /* boundary checking seems appropriate.                              */
        let tables = &mut toc.tables;

        let count = toc.count as usize;
        for n in 0..count - 1 {
            let mut have_change = false;

            for i in 0..count - 1 - n {
                if tables[i].offset > tables[i + 1].offset {
                    tables.swap(i, i + 1);

                    have_change = true;
                }

                if (tables[i].size > tables[i + 1].offset)
                    || (tables[i].offset > tables[i + 1].offset - tables[i].size)
                {
                    return Err(FT_ERR_INVALID_OFFSET);
                }
            }

            if !have_change {
                break;
            }
        }

        /*
         * We now check whether the `size' and `offset' values are reasonable:
         * `offset' + `size' must not exceed the stream size.
         *
         * Note, however, that X11's `pcfWriteFont' routine (used by the
         * `bdftopcf' program to create PCF font files) has two special
         * features.
         *
         * - It always assigns the accelerator table a size of 100 bytes in the
         *   TOC, regardless of its real size, which can vary between 34 and 72
         *   bytes.
         *
         * - Due to the way the routine is designed, it ships out the last font
         *   table with its real size, ignoring the TOC's size value.  Since
         *   the TOC size values are always rounded up to a multiple of 4, the
         *   difference can be up to three bytes for all tables except the
         *   accelerator table, for which the difference can be as large as 66
         *   bytes.
         *
         */

        let size = stream.size;

        for table in tables.iter().take(count - 1) {
            /* we need two checks to avoid overflow */
            if (table.size > size) || (table.offset > size - table.size) {
                return Err(FT_ERR_INVALID_TABLE);
            }
        }

        /* only check `tables->offset' for last table element ... */
        let last = &mut tables[count - 1];
        if last.offset > size {
            return Err(FT_ERR_INVALID_TABLE);
        }
        /* ... and adjust `tables->size' to the real value if necessary */
        if last.size > size - last.offset {
            last.size = size - last.offset;
        }

        Ok(())
    })();

    if error.is_err() {
        /* Exit: */
        face.toc.tables = Vec::new();
    }
    error
}

const PCF_METRIC_SIZE: FtULong = 12;

/// `pcf_metric_header` and `pcf_metric_msb_header`
fn read_pcf_metric(stream: &mut FtStreamRec, msb: bool, metric: &mut PcfMetricRec) -> FtResult<()> {
    stream.enter_frame(PCF_METRIC_SIZE)?;
    let get = |stream: &mut FtStreamRec| -> FtShort {
        if msb {
            stream.get_short()
        } else {
            stream.get_ushort_le() as FtShort
        }
    };
    metric.leftSideBearing = get(stream);
    metric.rightSideBearing = get(stream);
    metric.characterWidth = get(stream);
    metric.ascent = get(stream);
    metric.descent = get(stream);
    metric.attributes = get(stream);
    stream.exit_frame();
    Ok(())
}

const PCF_COMPRESSED_METRIC_SIZE: FtULong = 5;

/// `pcf_compressed_metric_header`
fn read_pcf_compressed_metric(
    stream: &mut FtStreamRec,
    compr: &mut PcfCompressedMetricRec,
) -> FtResult<()> {
    stream.enter_frame(PCF_COMPRESSED_METRIC_SIZE)?;
    compr.leftSideBearing = stream.get_byte();
    compr.rightSideBearing = stream.get_byte();
    compr.characterWidth = stream.get_byte();
    compr.ascent = stream.get_byte();
    compr.descent = stream.get_byte();
    stream.exit_frame();
    Ok(())
}

/// `pcf_get_metric`
fn pcf_get_metric(
    stream: &mut FtStreamRec,
    format: FtULong,
    metric: &mut PcfMetricRec,
) -> FtResult<()> {
    if pcf_format_match(format, PCF_DEFAULT_FORMAT) {
        /* parsing normal metrics */
        let msb = pcf_byte_order(format) == MSBFirst;

        /* the following sets `error' but doesn't return in case of failure */
        read_pcf_metric(stream, msb, metric)
    } else {
        let mut compr = PcfCompressedMetricRec::default();

        /* parsing compressed metrics */
        read_pcf_compressed_metric(stream, &mut compr)?;

        metric.leftSideBearing = (compr.leftSideBearing as i32 - 0x80) as FtShort;
        metric.rightSideBearing = (compr.rightSideBearing as i32 - 0x80) as FtShort;
        metric.characterWidth = (compr.characterWidth as i32 - 0x80) as FtShort;
        metric.ascent = (compr.ascent as i32 - 0x80) as FtShort;
        metric.descent = (compr.descent as i32 - 0x80) as FtShort;
        metric.attributes = 0;

        Ok(())
    }
}

/// `pcf_seek_to_table_type`
fn pcf_seek_to_table_type(
    stream: &mut FtStreamRec,
    tables: &[PcfTableRec],
    ntables: FtULong, /* same as PCF_Toc->count */
    type_: FtULong,
    aformat: &mut FtULong,
    asize: &mut FtULong,
) -> FtResult<()> {
    let mut error = FT_ERR_INVALID_FILE_FORMAT;

    for i in 0..ntables as usize {
        let Some(table) = tables.get(i) else {
            break;
        };
        if table.type_ == type_ {
            if stream.pos > table.offset {
                error = FT_ERR_INVALID_STREAM_SKIP;
                break;
            }

            if stream.skip((table.offset - stream.pos) as FtLong).is_err() {
                error = FT_ERR_INVALID_STREAM_SKIP;
                break;
            }

            *asize = table.size;
            *aformat = table.format;

            return Ok(());
        }
    }

    /* Fail: */
    *asize = 0;
    Err(error)
}

/// `pcf_has_table_type`
fn pcf_has_table_type(
    tables: &[PcfTableRec],
    ntables: FtULong, /* same as PCF_Toc->count */
    type_: FtULong,
) -> bool {
    for i in 0..ntables as usize {
        if tables.get(i).is_some_and(|t| t.type_ == type_) {
            return true;
        }
    }

    false
}

const PCF_PROPERTY_SIZE: FtULong = 9;

/// `pcf_property_header` and `pcf_property_msb_header`
fn read_pcf_property(
    stream: &mut FtStreamRec,
    msb: bool,
    prop: &mut PcfParsePropertyRec,
) -> FtResult<()> {
    stream.enter_frame(PCF_PROPERTY_SIZE)?;
    if msb {
        prop.name = stream.get_long() as FtLong;
        prop.isString = stream.get_byte();
        prop.value = stream.get_long() as FtLong;
    } else {
        prop.name = stream.get_ulong_le() as i32 as FtLong;
        prop.isString = stream.get_byte();
        prop.value = stream.get_ulong_le() as i32 as FtLong;
    }
    stream.exit_frame();
    Ok(())
}

/// The C string at `s` (up to its NUL).
fn cstr(s: &[u8]) -> &[u8] {
    let n = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    &s[..n]
}

/// `pcf_find_property`
pub fn pcf_find_property<'a>(face: &'a PcfFaceRec, prop: &[u8]) -> Option<&'a PcfPropertyRec> {
    let properties = &face.properties;

    for i in 0..face.nprops.max(0) as usize {
        let p = properties.get(i)?;
        if p.name.as_deref().map(cstr) == Some(prop) {
            return Some(p);
        }
    }

    None
}

/// `pcf_get_properties`
fn pcf_get_properties(stream: &mut FtStreamRec, face: &mut PcfFaceRec) -> FtResult<()> {
    let mut error: FtError;
    let mut format: FtULong = 0;
    let mut size: FtULong = 0;

    pcf_seek_to_table_type(
        stream,
        &face.toc.tables,
        face.toc.count,
        PCF_PROPERTIES,
        &mut format,
        &mut size,
    )?;

    let format = stream.read_ulong_le()? as FtULong;

    if !pcf_format_match(format, PCF_DEFAULT_FORMAT) {
        /* Bail: */
        return Ok(());
    }

    let msb = pcf_byte_order(format) == MSBFirst;

    error = FT_ERR_OK;
    let orig_nprops = if msb {
        rd(stream.read_ulong(), &mut error)
    } else {
        rd(stream.read_ulong_le(), &mut error)
    } as FtULong;
    if error != 0 {
        return Err(error);
    }

    /* rough estimate */
    if orig_nprops > size / PCF_PROPERTY_SIZE {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* as a heuristic limit to avoid excessive allocation in */
    /* gzip bombs (i.e., very small, invalid input data that */
    /* pretends to expand to an insanely large file) we only */
    /* load the first 256 properties                         */
    let nprops: FtULong = if orig_nprops > 256 { 256 } else { orig_nprops };

    face.nprops = nprops as i32;

    let mut props: Vec<PcfParsePropertyRec> = ft_new_array(nprops as FtLong)?;

    for prop in props.iter_mut() {
        read_pcf_property(stream, msb, prop)?;
    }

    /* this skip will only work if we really have an extremely large */
    /* number of properties; it will fail for fake data, avoiding an */
    /* unnecessarily large allocation later on                       */
    if stream
        .skip(((orig_nprops - nprops) * PCF_PROPERTY_SIZE) as FtLong)
        .is_err()
    {
        return Err(FT_ERR_INVALID_STREAM_SKIP);
    }

    /* pad the property array                                            */
    /*                                                                   */
    /* clever here - nprops is the same as the number of odd-units read, */
    /* as only isStringProp are odd length   (Keith Packard)             */
    /*                                                                   */
    if orig_nprops & 3 != 0 {
        let i = 4 - (orig_nprops & 3);
        if stream.skip(i as FtLong).is_err() {
            return Err(FT_ERR_INVALID_STREAM_SKIP);
        }
    }

    let mut string_size = if msb {
        rd(stream.read_ulong(), &mut error)
    } else {
        rd(stream.read_ulong_le(), &mut error)
    } as FtULong;
    if error != 0 {
        return Err(error);
    }

    /* rough estimate */
    if string_size > size.wrapping_sub(orig_nprops * PCF_PROPERTY_SIZE) {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* the strings in the `strings' array are PostScript strings, */
    /* which can have a maximum length of 65536 characters each   */
    if string_size > 16777472 {
        /* 256 * (65536 + 1) */
        string_size = 16777472;
    }

    /* allocate one more byte so that we have a final null byte */
    let mut strings = ft_qalloc(string_size as FtLong + 1)?;
    stream.read(&mut strings[..string_size as usize])?;

    strings[string_size as usize] = b'\0';

    /* zero out in case of failure */
    face.properties = ft_new_array(nprops as FtLong)?;

    for i in 0..nprops as usize {
        let name_offset: FtLong = props[i].name;

        if (name_offset < 0) || (name_offset as FtULong > string_size) {
            return Err(FT_ERR_INVALID_OFFSET);
        }

        face.properties[i].name = Some(cstr(&strings[name_offset as usize..]).to_vec());

        face.properties[i].isString = props[i].isString;

        if props[i].isString != 0 {
            let value_offset: FtLong = props[i].value;

            if (value_offset < 0) || (value_offset as FtULong > string_size) {
                return Err(FT_ERR_INVALID_OFFSET);
            }

            face.properties[i].atom = Some(cstr(&strings[value_offset as usize..]).to_vec());
        } else {
            face.properties[i].l = props[i].value;
        }
    }

    /* Bail: */
    Ok(())
}

/// `pcf_get_metrics`
fn pcf_get_metrics(stream: &mut FtStreamRec, face: &mut PcfFaceRec) -> FtResult<()> {
    let mut error: FtError = FT_ERR_OK;
    let mut format: FtULong = 0;
    let mut size: FtULong = 0;

    pcf_seek_to_table_type(
        stream,
        &face.toc.tables,
        face.toc.count,
        PCF_METRICS,
        &mut format,
        &mut size,
    )?;

    let format = stream.read_ulong_le()? as FtULong;

    if !pcf_format_match(format, PCF_DEFAULT_FORMAT)
        && !pcf_format_match(format, PCF_COMPRESSED_METRICS)
    {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    let msb = pcf_byte_order(format) == MSBFirst;
    let orig_nmetrics: FtULong = if pcf_format_match(format, PCF_DEFAULT_FORMAT) {
        if msb {
            rd(stream.read_ulong(), &mut error) as FtULong
        } else {
            rd(stream.read_ulong_le(), &mut error) as FtULong
        }
    } else if msb {
        rd(stream.read_ushort(), &mut error) as FtULong
    } else {
        rd(stream.read_ushort_le(), &mut error) as FtULong
    };
    if error != 0 {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    /* rough estimate */
    if pcf_format_match(format, PCF_DEFAULT_FORMAT) {
        if orig_nmetrics > size / PCF_METRIC_SIZE {
            return Err(FT_ERR_INVALID_TABLE);
        }
    } else if orig_nmetrics > size / PCF_COMPRESSED_METRIC_SIZE {
        return Err(FT_ERR_INVALID_TABLE);
    }

    if orig_nmetrics == 0 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /*
     * PCF is a format from ancient times; Unicode was in its infancy, and
     * widely used two-byte character sets for CJK scripts (Big 5, GB 2312,
     * JIS X 0208, etc.) did have at most 15000 characters.  Even the more
     * exotic CNS 11643 and CCCII standards, which were essentially
     * three-byte character sets, provided less then 65536 assigned
     * characters.
     *
     * While technically possible to have a larger number of glyphs in PCF
     * files, we thus limit the number to 65535, taking into account that we
     * synthesize the metrics of glyph 0 to be a copy of the `default
     * character', and that 0xFFFF in the encodings array indicates a
     * missing glyph.
     */
    let nmetrics: FtULong = if orig_nmetrics > 65534 {
        65534
    } else {
        orig_nmetrics
    };

    face.nmetrics = nmetrics + 1;

    face.metrics = ft_new_array(face.nmetrics as FtLong)?;

    /* we handle glyph index 0 later on */
    let mut r = Ok(());
    for i in 1..face.nmetrics as usize {
        let metrics = &mut face.metrics[i];
        r = pcf_get_metric(stream, format, metrics);

        metrics.bits = 0;

        if r.is_err() {
            break;
        }

        /* sanity checks -- those values are used in `PCF_Glyph_Load' to     */
        /* compute a glyph's bitmap dimensions, thus setting them to zero in */
        /* case of an error disables this particular glyph only              */
        if metrics.rightSideBearing < metrics.leftSideBearing
            || (metrics.ascent as i32) < -(metrics.descent as i32)
        {
            metrics.characterWidth = 0;
            metrics.leftSideBearing = 0;
            metrics.rightSideBearing = 0;
            metrics.ascent = 0;
            metrics.descent = 0;
        }
    }

    if r.is_err() {
        face.metrics = Vec::new();
    }

    /* Bail: */
    r
}

/// `pcf_get_bitmaps`
fn pcf_get_bitmaps(stream: &mut FtStreamRec, face: &mut PcfFaceRec) -> FtResult<()> {
    let mut error: FtError = FT_ERR_OK;
    let mut bitmap_sizes: [FtULong; GLYPHPADOPTIONS] = [0; GLYPHPADOPTIONS];
    let mut format: FtULong = 0;
    let mut size: FtULong = 0;

    pcf_seek_to_table_type(
        stream,
        &face.toc.tables,
        face.toc.count,
        PCF_BITMAPS,
        &mut format,
        &mut size,
    )?;

    stream.enter_frame(8)?;

    let format = stream.get_ulong_le() as FtULong;
    let orig_nbitmaps: FtULong = if pcf_byte_order(format) == MSBFirst {
        stream.get_ulong() as FtULong
    } else {
        stream.get_ulong_le() as FtULong
    };

    stream.exit_frame();

    if !pcf_format_match(format, PCF_DEFAULT_FORMAT) {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    /* see comment in `pcf_get_metrics' */
    let nbitmaps: FtULong = if orig_nbitmaps > 65534 {
        65534
    } else {
        orig_nbitmaps
    };

    /* no extra bitmap for glyph 0 */
    if nbitmaps != face.nmetrics.wrapping_sub(1) {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    /* start position of bitmap data */
    let pos: FtULong = stream.pos + nbitmaps * 4 + 4 * 4;

    let msb = pcf_byte_order(format) == MSBFirst;
    for i in 1..=nbitmaps as usize {
        let offset: FtULong = if msb {
            rd(stream.read_ulong(), &mut error) as FtULong
        } else {
            rd(stream.read_ulong_le(), &mut error) as FtULong
        };

        /* right now, we only check the offset with a rough estimate; */
        /* actual bitmaps are only loaded on demand                   */
        if let Some(m) = face.metrics.get_mut(i) {
            if offset > size {
                m.bits = pos;
            } else {
                m.bits = pos + offset;
            }
        }
    }
    if error != 0 {
        return Err(error);
    }

    for i in 0..GLYPHPADOPTIONS {
        bitmap_sizes[i] = if msb {
            rd(stream.read_ulong(), &mut error) as FtULong
        } else {
            rd(stream.read_ulong_le(), &mut error) as FtULong
        };
        if error != 0 {
            return Err(error);
        }

        /* (`sizebitmaps', only used for debugging) */
    }

    face.bitmapsFormat = format;

    /* Bail: */
    Ok(())
}

/*
 * This file uses X11 terminology for PCF data; an `encoding' in X11 speak
 * is the same as a character code in FreeType speak.
 */
const PCF_ENC_SIZE: FtULong = 10;

/// `pcf_enc_header` and `pcf_enc_msb_header`
fn read_pcf_enc(stream: &mut FtStreamRec, msb: bool, enc: &mut PcfEncRec) -> FtResult<()> {
    stream.enter_frame(PCF_ENC_SIZE)?;
    let get = |stream: &mut FtStreamRec| -> FtUShort {
        if msb {
            stream.get_ushort()
        } else {
            stream.get_ushort_le()
        }
    };
    enc.firstCol = get(stream);
    enc.lastCol = get(stream);
    enc.firstRow = get(stream);
    enc.lastRow = get(stream);
    enc.defaultChar = get(stream);
    stream.exit_frame();
    Ok(())
}

/// `pcf_get_encodings`
fn pcf_get_encodings(stream: &mut FtStreamRec, face: &mut PcfFaceRec) -> FtResult<()> {
    let mut format: FtULong = 0;
    let mut size: FtULong = 0;

    pcf_seek_to_table_type(
        stream,
        &face.toc.tables,
        face.toc.count,
        PCF_BDF_ENCODINGS,
        &mut format,
        &mut size,
    )?;

    let format = stream.read_ulong_le()? as FtULong;

    if !pcf_format_match(format, PCF_DEFAULT_FORMAT) && !pcf_format_match(format, PCF_BDF_ENCODINGS)
    {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    let msb = pcf_byte_order(format) == MSBFirst;
    read_pcf_enc(stream, msb, &mut face.enc)?;

    let enc = &mut face.enc;

    /* sanity checks; we limit numbers of rows and columns to 256 */
    if enc.firstCol > enc.lastCol
        || enc.lastCol > 0xFF
        || enc.firstRow > enc.lastRow
        || enc.lastRow > 0xFF
    {
        return Err(FT_ERR_INVALID_TABLE);
    }

    let mut default_char_row: FtUShort = enc.defaultChar >> 8;
    let mut default_char_col: FtUShort = enc.defaultChar & 0xFF;

    /* validate default character */
    if default_char_row < enc.firstRow
        || default_char_row > enc.lastRow
        || default_char_col < enc.firstCol
        || default_char_col > enc.lastCol
    {
        enc.defaultChar = (enc.firstRow as u32 * 256 + enc.firstCol as u32) as FtUShort;

        default_char_row = enc.firstRow;
        default_char_col = enc.firstCol;
    }

    let nencoding: FtULong =
        (enc.lastCol - enc.firstCol + 1) as FtULong * (enc.lastRow - enc.firstRow + 1) as FtULong;

    stream.enter_frame(2 * nencoding)?;

    /*
     * FreeType mandates that glyph index 0 is the `undefined glyph', which
     * PCF calls the `default character'.  However, FreeType needs glyph
     * index 0 to be used for the undefined glyph only, which is is not the
     * case for PCF.  For this reason, we add one slot for glyph index 0 and
     * simply copy the default character to it.
     *
     * `stream->cursor' still points to the beginning of the frame; we can
     * thus easily get the offset to the default character.
     */
    let pos = stream.cursor()
        + 2 * ((default_char_row - enc.firstRow) as usize
            * (enc.lastCol - enc.firstCol + 1) as usize
            + (default_char_col - enc.firstCol) as usize);

    let mut default_char_encoding_offset: FtUShort = if msb {
        ft_peek_ushort(stream.frame_data(), pos)
    } else {
        ft_peek_ushort_le(stream.frame_data(), pos)
    };

    if default_char_encoding_offset == 0xFFFF {
        /* No glyph for default character, */
        /* setting it to the first glyph of the font */
        default_char_encoding_offset = 1;
    } else {
        default_char_encoding_offset += 1;

        if default_char_encoding_offset as FtULong >= face.nmetrics {
            /* Invalid glyph index for default character, */
            /* setting it to the first glyph of the font */
            default_char_encoding_offset = 1;
        }
    }

    /* copy metrics of default character to index 0 */
    if let Some(m) = face
        .metrics
        .get(default_char_encoding_offset as usize)
        .copied()
    {
        face.metrics[0] = m;
    }

    let mut offsets: Vec<FtUShort> = match ft_new_array(nencoding as FtLong) {
        Ok(v) => v,
        Err(e) => {
            /* (C leaves the frame entered) */
            stream.exit_frame();
            return Err(e);
        }
    };

    /* now loop over all values */
    let enc = &mut face.enc;
    let mut k = 0usize;
    for _i in enc.firstRow..=enc.lastRow {
        for _j in enc.firstCol..=enc.lastCol {
            /* X11's reference implementation uses the equivalent to  */
            /* `FT_GET_SHORT', however PCF fonts with more than 32768 */
            /* characters (e.g., `unifont.pcf') clearly show that an  */
            /* unsigned value is needed.                              */
            let encoding_offset: FtUShort = if msb {
                stream.get_ushort()
            } else {
                stream.get_ushort_le()
            };

            /* everything is off by 1 due to the artificial glyph 0 */
            offsets[k] = if encoding_offset == 0xFFFF {
                0xFFFF
            } else {
                encoding_offset.wrapping_add(1)
            };
            k += 1;
        }
    }
    stream.exit_frame();

    enc.offset = Arc::from(offsets);

    /* Bail: */
    Ok(())
}

/// `pcf_accel_header` and `pcf_accel_msb_header`
fn read_pcf_accel(stream: &mut FtStreamRec, msb: bool, accel: &mut PcfAccelRec) -> FtResult<()> {
    stream.enter_frame(20)?;
    accel.noOverlap = stream.get_byte();
    accel.constantMetrics = stream.get_byte();
    accel.terminalFont = stream.get_byte();
    accel.constantWidth = stream.get_byte();
    accel.inkInside = stream.get_byte();
    accel.inkMetrics = stream.get_byte();
    accel.drawDirection = stream.get_byte();
    let _ = stream.skip_bytes(1);
    let get = |stream: &mut FtStreamRec| -> FtLong {
        if msb {
            stream.get_long() as FtLong
        } else {
            stream.get_ulong_le() as i32 as FtLong
        }
    };
    accel.fontAscent = get(stream);
    accel.fontDescent = get(stream);
    accel.maxOverlap = get(stream);
    stream.exit_frame();
    Ok(())
}

/// `pcf_get_accel`
fn pcf_get_accel(stream: &mut FtStreamRec, face: &mut PcfFaceRec, type_: FtULong) -> FtResult<()> {
    let mut format: FtULong = 0;
    let mut size: FtULong = 0;

    pcf_seek_to_table_type(
        stream,
        &face.toc.tables,
        face.toc.count,
        type_,
        &mut format,
        &mut size,
    )?;

    let format = stream.read_ulong_le()? as FtULong;

    if !pcf_format_match(format, PCF_DEFAULT_FORMAT)
        && !pcf_format_match(format, PCF_ACCEL_W_INKBOUNDS)
    {
        /* Bail: */
        return Ok(());
    }

    let accel = &mut face.accel;
    read_pcf_accel(stream, pcf_byte_order(format) == MSBFirst, accel)?;

    /* sanity checks */
    if accel.fontAscent.wrapping_abs() > 0x7FFF {
        accel.fontAscent = if accel.fontAscent < 0 {
            -0x7FFF
        } else {
            0x7FFF
        };
    }
    if accel.fontDescent.wrapping_abs() > 0x7FFF {
        accel.fontDescent = if accel.fontDescent < 0 {
            -0x7FFF
        } else {
            0x7FFF
        };
    }

    pcf_get_metric(stream, format & !PCF_FORMAT_MASK, &mut accel.minbounds)?;

    pcf_get_metric(stream, format & !PCF_FORMAT_MASK, &mut accel.maxbounds)?;

    if pcf_format_match(format, PCF_ACCEL_W_INKBOUNDS) {
        pcf_get_metric(stream, format & !PCF_FORMAT_MASK, &mut accel.ink_minbounds)?;

        pcf_get_metric(stream, format & !PCF_FORMAT_MASK, &mut accel.ink_maxbounds)?;
    } else {
        accel.ink_minbounds = accel.minbounds;
        accel.ink_maxbounds = accel.maxbounds;
    }

    /* Bail: */
    Ok(())
}

/// `pcf_interpret_style`
fn pcf_interpret_style(pcf: &mut PcfFaceRec) -> FtResult<()> {
    let mut strings: [Option<Vec<u8>>; 4] = [None, None, None, None];
    let mut lengths: [usize; 4] = [0; 4];

    pcf.root.style_flags = 0;

    let first = |p: &PcfPropertyRec| {
        p.atom
            .as_ref()
            .and_then(|a| a.first())
            .copied()
            .unwrap_or(0)
    };

    let mut style_flags = 0;
    if let Some(prop) = pcf_find_property(pcf, b"SLANT") {
        let c = first(prop);
        if prop.isString != 0 && (c == b'O' || c == b'o' || c == b'I' || c == b'i') {
            style_flags |= FT_STYLE_FLAG_ITALIC;
            strings[2] = Some(if c == b'O' || c == b'o' {
                b"Oblique".to_vec()
            } else {
                b"Italic".to_vec()
            });
        }
    }

    if let Some(prop) = pcf_find_property(pcf, b"WEIGHT_NAME") {
        let c = first(prop);
        if prop.isString != 0 && (c == b'B' || c == b'b') {
            style_flags |= FT_STYLE_FLAG_BOLD;
            strings[1] = Some(b"Bold".to_vec());
        }
    }

    if let Some(prop) = pcf_find_property(pcf, b"SETWIDTH_NAME") {
        let c = first(prop);
        if prop.isString != 0 && c != 0 && !(c == b'N' || c == b'n') {
            strings[3] = prop.atom.clone();
        }
    }

    if let Some(prop) = pcf_find_property(pcf, b"ADD_STYLE_NAME") {
        let c = first(prop);
        if prop.isString != 0 && c != 0 && !(c == b'N' || c == b'n') {
            strings[0] = prop.atom.clone();
        }
    }

    let mut len = 0usize;
    for nn in 0..4 {
        lengths[nn] = 0;
        if let Some(s) = &strings[nn] {
            lengths[nn] = cstr(s).len();
            len += lengths[nn] + 1;
        }
    }

    if len == 0 {
        strings[0] = Some(b"Regular".to_vec());
        lengths[0] = 7;
    }

    {
        let mut s: Vec<u8> = Vec::new();

        for nn in 0..4 {
            let Some(src) = &strings[nn] else {
                continue;
            };

            let len = lengths[nn];

            /* separate elements with a space */
            if !s.is_empty() {
                s.push(b' ');
            }

            let start = s.len();
            s.extend_from_slice(&src[..len]);

            /* need to convert spaces to dashes for */
            /* add_style_name and setwidth_name     */
            if nn == 0 || nn == 3 {
                for c in s[start..].iter_mut() {
                    if *c == b' ' {
                        *c = b'-';
                    }
                }
            }
        }

        pcf.root.style_flags = style_flags;
        pcf.root.style_name = Some(String::from_utf8_lossy(&s).into_owned());
    }

    Ok(())
}

/// `pcf_load_font`
pub fn pcf_load_font(
    stream: &mut FtStreamRec,
    face: &mut PcfFaceRec,
    face_index: FtLong,
) -> FtResult<()> {
    let error = pcf_load_font_body(stream, face, face_index);

    /* Exit: */
    if error.is_err() {
        /* This is done to respect the behaviour of the original */
        /* PCF font driver.                                      */
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    error
}

fn pcf_load_font_body(
    stream: &mut FtStreamRec,
    face: &mut PcfFaceRec,
    face_index: FtLong,
) -> FtResult<()> {
    pcf_read_toc(stream, face)?;

    face.root.num_faces = 1;
    face.root.face_index = 0;

    /* If we are performing a simple font format check, exit immediately. */
    if face_index < 0 {
        return Ok(());
    }

    pcf_get_properties(stream, face)?;

    /* Use the old accelerators if no BDF accelerators are in the file. */
    let has_bdf_accelerators =
        pcf_has_table_type(&face.toc.tables, face.toc.count, PCF_BDF_ACCELERATORS);
    if !has_bdf_accelerators {
        pcf_get_accel(stream, face, PCF_ACCELERATORS)?;
    }

    /* metrics */
    pcf_get_metrics(stream, face)?;

    /* bitmaps */
    pcf_get_bitmaps(stream, face)?;

    /* encodings */
    pcf_get_encodings(stream, face)?;

    /* BDF style accelerators (i.e. bounds based on encoded glyphs) */
    if has_bdf_accelerators {
        pcf_get_accel(stream, face, PCF_BDF_ACCELERATORS)?;
    }

    /* XXX: TO DO: inkmetrics and glyph_names are missing */

    /* now construct the face object */
    {
        face.root.face_flags |= FT_FACE_FLAG_FIXED_SIZES | FT_FACE_FLAG_HORIZONTAL;

        if face.accel.constantWidth != 0 {
            face.root.face_flags |= FT_FACE_FLAG_FIXED_WIDTH;
        }

        pcf_interpret_style(face)?;

        match pcf_find_property(face, b"FAMILY_NAME") {
            Some(prop) if prop.isString != 0 => {
                /* (`PCF_CONFIG_OPTION_LONG_FAMILY_NAMES' is undefined) */
                let atom = prop.atom.clone().unwrap_or_default();
                face.root.family_name = Some(String::from_utf8_lossy(&atom).into_owned());
            }
            _ => face.root.family_name = None,
        }

        face.root.num_glyphs = face.nmetrics as FtLong;

        face.root.num_fixed_sizes = 1;

        {
            let mut bsize = FtBitmapSize::default();
            let mut resolution_x: FtShort = 0;
            let mut resolution_y: FtShort = 0;

            /* for simplicity, we take absolute values of integer properties */

            let sum = face.accel.fontAscent.wrapping_add(face.accel.fontDescent);
            if sum.wrapping_abs() > 0x7FFF {
                bsize.height = 0x7FFF;
            } else {
                bsize.height = (sum as FtShort).wrapping_abs();
            }

            if let Some(prop) = pcf_find_property(face, b"AVERAGE_WIDTH") {
                let l = prop.value_l();
                if l.wrapping_abs() > 0x7FFF * 10 - 5 {
                    bsize.width = 0x7FFF;
                } else {
                    bsize.width = (((l + 5) / 10) as FtShort).wrapping_abs();
                }
            } else {
                /* this is a heuristical value */
                bsize.width = ((bsize.height as i32 * 2 + 1) / 3) as FtShort;
            }

            if let Some(prop) = pcf_find_property(face, b"POINT_SIZE") {
                let l = prop.value_l();
                /* convert from 722.7 decipoints to 72 points per inch */
                if l.wrapping_abs() > 0x504C2 {
                    /* 0x7FFF * 72270/7200 */
                    bsize.size = 0x7FFF;
                } else {
                    bsize.size = ft_mul_div(l.wrapping_abs(), 64 * 7200, 72270);
                }
            }

            if let Some(prop) = pcf_find_property(face, b"PIXEL_SIZE") {
                let l = prop.value_l();
                if l.wrapping_abs() > 0x7FFF {
                    bsize.y_ppem = 0x7FFF << 6;
                } else {
                    bsize.y_ppem = ((l as FtShort).wrapping_abs() as FtPos) << 6;
                }
            }

            if let Some(prop) = pcf_find_property(face, b"RESOLUTION_X") {
                let l = prop.value_l();
                if l.wrapping_abs() > 0x7FFF {
                    resolution_x = 0x7FFF;
                } else {
                    resolution_x = (l as FtShort).wrapping_abs();
                }
            }

            if let Some(prop) = pcf_find_property(face, b"RESOLUTION_Y") {
                let l = prop.value_l();
                if l.wrapping_abs() > 0x7FFF {
                    resolution_y = 0x7FFF;
                } else {
                    resolution_y = (l as FtShort).wrapping_abs();
                }
            }

            if bsize.y_ppem == 0 {
                bsize.y_ppem = bsize.size;
                if resolution_y != 0 {
                    bsize.y_ppem = ft_mul_div(bsize.y_ppem, resolution_y as FtLong, 72);
                }
            }
            if resolution_x != 0 && resolution_y != 0 {
                bsize.x_ppem =
                    ft_mul_div(bsize.y_ppem, resolution_x as FtLong, resolution_y as FtLong);
            } else {
                bsize.x_ppem = bsize.y_ppem;
            }

            face.root.available_sizes = vec![bsize];
        }

        /* set up charset */
        {
            let charset_registry = pcf_find_property(face, b"CHARSET_REGISTRY");
            let charset_encoding = pcf_find_property(face, b"CHARSET_ENCODING");

            if let (Some(reg), Some(enc)) = (charset_registry, charset_encoding) {
                if reg.isString != 0 && enc.isString != 0 {
                    let (e, r) = (enc.atom.clone(), reg.atom.clone());
                    face.charset_encoding = e;
                    face.charset_registry = r;
                }
            }
        }
    }

    Ok(())
}
