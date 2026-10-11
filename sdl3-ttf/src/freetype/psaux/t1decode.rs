// Rust translation of src/psaux/t1decode.c and src/psaux/t1decode.h, and
// of the Type 1 decoder records of include/freetype/internal/psaux.h,
// from FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2000-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! PostScript Type 1 decoding routines (body).
//!
//! As SDL_ttf's bundled build configures it, `T1_CONFIG_OPTION_OLD_ENGINE`
//! is undefined: the old charstring interpreter
//! (`t1_decoder_parse_charstrings`) is not compiled, and the Adobe engine
//! renders Type 1 charstrings (`cf2_decoder_parse_charstrings`); the
//! decoder only parses the metrics of a glyph itself
//! (`t1_decoder_parse_metrics`). The decoder's zones are offsets into
//! the charstring or the subroutines they are in; its parse callback is
//! the driver's, which the driver calls directly.

use std::collections::HashMap;
use std::sync::Arc;

use super::super::base::ftcalc::ft_div_fix;
use super::super::cfftypes::CffGlyphSlotRec;
use super::super::fttypes::*;
use super::super::psnames::psmodule::ps_get_standard_strings;
use super::super::psnames::pstables::T1_STANDARD_ENCODING;
use super::super::t1tables::PsBlendRec;
use super::psfont::Cf2FontRec;
use super::psobjs::*;

/* ensure proper sign extension */
#[inline]
fn fix2int(f: FtLong) -> FtInt {
    (f >> 16) as FtShort as FtInt
}

/// `T1_Operator`
type T1Operator = usize;

const OP_NONE: T1Operator = 0;
const OP_ENDCHAR: T1Operator = 1;
const OP_HSBW: T1Operator = 2;
const OP_SEAC: T1Operator = 3;
const OP_SBW: T1Operator = 4;
const OP_CLOSEPATH: T1Operator = 5;
const OP_HLINETO: T1Operator = 6;
const OP_HMOVETO: T1Operator = 7;
const OP_HVCURVETO: T1Operator = 8;
const OP_RLINETO: T1Operator = 9;
const OP_RMOVETO: T1Operator = 10;
const OP_RRCURVETO: T1Operator = 11;
const OP_VHCURVETO: T1Operator = 12;
const OP_VLINETO: T1Operator = 13;
const OP_VMOVETO: T1Operator = 14;
const OP_DOTSECTION: T1Operator = 15;
const OP_HSTEM: T1Operator = 16;
const OP_HSTEM3: T1Operator = 17;
const OP_VSTEM: T1Operator = 18;
const OP_VSTEM3: T1Operator = 19;
const OP_DIV: T1Operator = 20;
const OP_CALLOTHERSUBR: T1Operator = 21;
const OP_CALLSUBR: T1Operator = 22;
const OP_POP: T1Operator = 23;
const OP_RETURN: T1Operator = 24;
const OP_SETCURRENTPOINT: T1Operator = 25;
const OP_UNKNOWN15: T1Operator = 26;

const OP_MAX: usize = 27; /* never remove this one */

/// `t1_args_count`
static T1_ARGS_COUNT: [FtInt; OP_MAX] = [
    0,  /* none */
    0,  /* endchar */
    2,  /* hsbw */
    5,  /* seac */
    4,  /* sbw */
    0,  /* closepath */
    1,  /* hlineto */
    1,  /* hmoveto */
    4,  /* hvcurveto */
    2,  /* rlineto */
    2,  /* rmoveto */
    6,  /* rrcurveto */
    4,  /* vhcurveto */
    1,  /* vlineto */
    1,  /* vmoveto */
    0,  /* dotsection */
    2,  /* hstem */
    6,  /* hstem3 */
    2,  /* vstem */
    6,  /* vstem3 */
    2,  /* div */
    -1, /* callothersubr */
    1,  /* callsubr */
    0,  /* pop */
    0,  /* return */
    2,  /* setcurrentpoint */
    2,  /* opcode 15 (undocumented and obsolete) */
];

/* psaux.h */

/* (`#if 0'ed T1_MAX_SUBRS_CALLS and T1_MAX_CHARSTRINGS_OPERANDS, which */
/* are defined in ftoption.h)                                          */
pub const T1_MAX_SUBRS_CALLS: usize = 16;
pub const T1_MAX_CHARSTRINGS_OPERANDS: usize = 256;

/// Where the bytes of a decoder zone are: the charstring being parsed,
/// the Type 1 subroutines or the CID subroutines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum T1ZoneSource {
    #[default]
    Charstring,
    Subrs,
    CidSubrs,
}

/// `T1_Decoder_ZoneRec` (offsets into the bytes of `source`)
#[derive(Debug, Clone, Copy, Default)]
pub struct T1DecoderZoneRec {
    pub source: T1ZoneSource,
    pub cursor: usize,
    pub base: usize,
    pub limit: usize,
}

/// `T1_DecoderRec`
///
/// The subroutines are a Type 1 font's table (`subrs`, with C's
/// `subrs_len`) or a CID font's (`cid_subrs`, offsets into
/// `cid_subrs_bytes` with C's `subrs[idx + 1]` ends, and no
/// `subrs_len`); `charstrings` is the Type 1 font's, for the seac
/// components of the Adobe engine.
#[derive(Debug)]
#[allow(non_snake_case)]
pub struct T1DecoderRec<'a> {
    pub builder: T1BuilderRec<'a>,

    pub stack: [FtLong; T1_MAX_CHARSTRINGS_OPERANDS],
    pub top: usize,

    pub zones: [T1DecoderZoneRec; T1_MAX_SUBRS_CALLS + 1],
    pub zone: usize,

    /* (`psnames' for seac: the module's functions are called directly) */
    pub num_glyphs: FtUInt,
    pub glyph_names: Option<PsTableData>,
    pub charstrings: Option<PsTableData>,

    pub lenIV: FtInt, /* internal for sub routine calls */
    pub num_subrs: FtInt,
    pub subrs: Option<PsTableData>, /* (with `subrs_len') */
    pub cid_subrs: Option<Arc<[usize]>>,
    pub cid_subrs_bytes: Option<Arc<[u8]>>,
    pub subrs_hash: Option<&'a HashMap<FtInt, usize>>, /* used if `num_subrs' was massaged */

    pub font_matrix: FtMatrix,
    pub font_offset: FtVector,

    pub flex_state: FtInt,
    pub num_flex_vectors: FtInt,
    pub flex_vectors: [FtVector; 7],

    pub blend: Option<&'a PsBlendRec>, /* for multiple master support */

    pub hint_mode: FtRenderMode,

    /* (`parse_callback' and `funcs': the driver's and these functions */
    /* are called directly)                                              */
    pub buildchar: &'a mut [FtLong],
    pub len_buildchar: FtUInt,

    pub seac: bool,

    pub cf2_instance: Option<Box<Cf2FontRec>>,
}

/// `t1_lookup_glyph_by_stdcharcode_ps`: looks up a given glyph by its
/// StandardEncoding charcode.  Used to implement the SEAC Type 1 operator
/// in the Adobe engine.
///
/// Returns the glyph index, -1 if not found.
pub fn t1_lookup_glyph_by_stdcharcode_ps(decoder: &PsDecoder<'_, '_>, charcode: FtInt) -> FtInt {
    /* check range of standard char code */
    if !(0..=255).contains(&charcode) {
        return -1;
    }

    let glyph_name =
        ps_get_standard_strings(T1_STANDARD_ENCODING[charcode as usize] as FtUInt).unwrap_or(b"");

    let Some(glyph_names) = decoder.t1().and_then(|t1| t1.glyph_names.as_ref()) else {
        return -1;
    };

    for n in 0..decoder.num_glyphs as usize {
        if let Some(name) = glyph_names.name(n) {
            if name.first() == glyph_name.first() && name == glyph_name {
                return n as FtInt;
            }
        }
    }

    -1
}

/* (T1_CONFIG_OPTION_OLD_ENGINE is not defined) */

impl T1DecoderRec<'_> {
    /// The bytes of a zone's source (`charstring` is the charstring being
    /// parsed).
    fn zone_bytes<'c>(&'c self, source: T1ZoneSource, charstring: &'c [u8]) -> &'c [u8] {
        match source {
            T1ZoneSource::Charstring => charstring,
            T1ZoneSource::Subrs => self.subrs.as_ref().map(|s| &s.block[..]).unwrap_or(&[]),
            T1ZoneSource::CidSubrs => self.cid_subrs_bytes.as_deref().unwrap_or(&[]),
        }
    }
}

/// `t1_decoder_parse_metrics`: parses a given Type 1 charstrings program.
/// Only the Type 1 `hsbw` and `sbw` operators are processed.
///
/// * `decoder`: the current Type 1 decoder.
/// * `charstring_base`: the base of the charstring stream.
/// * `charstring_len`: the length in bytes of the charstring stream.
pub fn t1_decoder_parse_metrics(
    decoder: &mut T1DecoderRec<'_>,
    charstring_base: &[u8],
    charstring_len: FtUInt,
) -> FtResult<()> {
    let mut zone: usize;
    let mut ip: usize;
    let mut limit: usize;
    let mut source: T1ZoneSource;
    let mut large_int: bool;

    /* First of all, initialize the decoder */
    decoder.top = 0;
    decoder.zone = 0;
    zone = 0;

    decoder.builder.parse_state = T1_PARSE_START;

    decoder.zones[zone].source = T1ZoneSource::Charstring;
    decoder.zones[zone].base = 0;
    limit = charstring_len as usize;
    decoder.zones[zone].limit = limit;
    ip = 0;
    decoder.zones[zone].cursor = 0;
    source = T1ZoneSource::Charstring;

    large_int = false;

    macro_rules! byte {
        ($i:expr) => {
            decoder
                .zone_bytes(source, charstring_base)
                .get($i)
                .copied()
                .unwrap_or(0)
        };
    }

    'syntax_error: {
        /* now, execute loop */
        while ip < limit {
            let mut top = decoder.top;
            let mut op: T1Operator = OP_NONE;
            let mut value: FtInt32 = 0;

            /**********************************************************************
             *
             * Decode operator or operand
             *
             */

            /* first of all, decompress operator or value */
            let b = byte!(ip);
            ip += 1;
            match b {
                1 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 14 | 15 | 21 | 22 | 30 | 31 => {
                    /* No_Width: */
                    break 'syntax_error;
                }

                10 => {
                    op = OP_CALLSUBR;
                }
                11 => {
                    op = OP_RETURN;
                }

                13 => {
                    op = OP_HSBW;
                }

                12 => {
                    if ip >= limit {
                        break 'syntax_error;
                    }

                    let b2 = byte!(ip);
                    ip += 1;
                    match b2 {
                        7 => {
                            op = OP_SBW;
                        }
                        12 => {
                            op = OP_DIV;
                        }

                        _ => {
                            /* No_Width: */
                            break 'syntax_error;
                        }
                    }
                }

                255 => {
                    /* four bytes integer */
                    if ip + 4 > limit {
                        break 'syntax_error;
                    }

                    value = (((byte!(ip) as FtUInt32) << 24)
                        | ((byte!(ip + 1) as FtUInt32) << 16)
                        | ((byte!(ip + 2) as FtUInt32) << 8)
                        | (byte!(ip + 3) as FtUInt32)) as FtInt32;
                    ip += 4;

                    /* According to the specification, values > 32000 or < -32000 must */
                    /* be followed by a `div' operator to make the result be in the    */
                    /* range [-32000;32000].  We expect that the second argument of    */
                    /* `div' is not a large number.  Additionally, we don't handle     */
                    /* stuff like `<large1> <large2> <num> div <num> div' or           */
                    /* <large1> <large2> <num> div div'.  This is probably not allowed */
                    /* anyway.                                                         */
                    if !(-32000..=32000).contains(&value) {
                        if large_int {
                            break 'syntax_error;
                        } else {
                            large_int = true;
                        }
                    } else if !large_int {
                        value = ((value as FtUInt32) << 16) as FtInt32;
                    }
                }

                _ => {
                    let b0 = byte!(ip - 1);
                    if b0 >= 32 {
                        if b0 < 247 {
                            value = b0 as FtInt32 - 139;
                        } else {
                            ip += 1;
                            if ip > limit {
                                break 'syntax_error;
                            }

                            let b1 = byte!(ip - 1) as FtInt32;
                            if b0 < 251 {
                                value = ((b0 as FtInt32 - 247) * 256) + b1 + 108;
                            } else {
                                value = -(((b0 as FtInt32 - 251) * 256) + b1 + 108);
                            }
                        }

                        if !large_int {
                            value = ((value as FtUInt32) << 16) as FtInt32;
                        }
                    } else {
                        break 'syntax_error;
                    }
                }
            }

            if large_int && !(op == OP_NONE || op == OP_DIV) {
                break 'syntax_error;
            }

            /**********************************************************************
             *
             * Push value on stack, or process operator
             *
             */
            if op == OP_NONE {
                if top >= T1_MAX_CHARSTRINGS_OPERANDS {
                    break 'syntax_error;
                }

                decoder.stack[top] = value as FtLong;
                top += 1;
                decoder.top = top;
            } else {
                /* general operator */
                let num_args = T1_ARGS_COUNT[op];

                if (top as FtInt) < num_args {
                    /* Stack_Underflow: */
                    return Err(FT_ERR_STACK_UNDERFLOW);
                }

                top -= num_args as usize;

                match op {
                    OP_HSBW => {
                        let builder = &mut decoder.builder;

                        builder.parse_state = T1_PARSE_HAVE_WIDTH;

                        builder.builder.left_bearing.x = builder
                            .builder
                            .left_bearing
                            .x
                            .wrapping_add(decoder.stack[top]);

                        builder.builder.advance.x = decoder.stack[top + 1];
                        builder.builder.advance.y = 0;

                        /* we only want to compute the glyph's metrics */
                        /* (lsb + advance width) without loading the   */
                        /* rest of it; so exit immediately             */
                        return Ok(());
                    }

                    OP_SBW => {
                        let builder = &mut decoder.builder;

                        builder.parse_state = T1_PARSE_HAVE_WIDTH;

                        builder.builder.left_bearing.x = builder
                            .builder
                            .left_bearing
                            .x
                            .wrapping_add(decoder.stack[top]);
                        builder.builder.left_bearing.y = builder
                            .builder
                            .left_bearing
                            .y
                            .wrapping_add(decoder.stack[top + 1]);

                        builder.builder.advance.x = decoder.stack[top + 2];
                        builder.builder.advance.y = decoder.stack[top + 3];

                        /* we only want to compute the glyph's metrics */
                        /* (lsb + advance width), without loading the  */
                        /* rest of it; so exit immediately             */
                        return Ok(());
                    }

                    OP_DIV => {
                        /* if `large_int' is set, we divide unscaled numbers; */
                        /* otherwise, we divide numbers in 16.16 format --    */
                        /* in both cases, it is the same operation            */
                        decoder.stack[top] = ft_div_fix(decoder.stack[top], decoder.stack[top + 1]);
                        top += 1;

                        large_int = false;
                    }

                    OP_CALLSUBR => {
                        let mut idx: FtInt = fix2int(decoder.stack[top]);

                        if let Some(hash) = decoder.subrs_hash {
                            match hash.get(&idx) {
                                Some(&val) => idx = val as FtInt,
                                None => idx = -1,
                            }
                        }

                        if idx < 0 || idx >= decoder.num_subrs {
                            break 'syntax_error;
                        }

                        if zone >= T1_MAX_SUBRS_CALLS {
                            break 'syntax_error;
                        }

                        decoder.zones[zone].cursor = ip; /* save current instruction pointer */

                        zone += 1;

                        /* The Type 1 driver stores subroutines without the seed bytes. */
                        /* The CID driver stores subroutines with seed bytes.  This     */
                        /* case is taken care of when decoder->subrs_len == 0.          */
                        let base: Option<usize>;
                        let zlimit: usize;
                        let zsource: T1ZoneSource;
                        if let Some(subrs) = decoder.subrs.as_ref() {
                            zsource = T1ZoneSource::Subrs;
                            base = subrs.elements.get(idx as usize).copied().flatten();
                            zlimit = base.unwrap_or(0) + subrs.length(idx as usize) as usize;
                        } else {
                            /* We are using subroutines from a CID font.  We must adjust */
                            /* for the seed bytes.                                       */
                            zsource = T1ZoneSource::CidSubrs;
                            let code = decoder.cid_subrs.as_deref().unwrap_or(&[]);
                            base = code.get(idx as usize).map(|&b| {
                                b + if decoder.lenIV >= 0 {
                                    decoder.lenIV as usize
                                } else {
                                    0
                                }
                            });
                            zlimit = code.get(idx as usize + 1).copied().unwrap_or(0);
                        }

                        let Some(base) = base else {
                            break 'syntax_error;
                        };

                        decoder.zones[zone] = T1DecoderZoneRec {
                            source: zsource,
                            cursor: base,
                            base,
                            limit: zlimit,
                        };

                        decoder.zone = zone;
                        ip = base;
                        limit = zlimit;
                        source = zsource;
                    }

                    OP_RETURN => {
                        if zone == 0 {
                            break 'syntax_error;
                        }

                        zone -= 1;
                        ip = decoder.zones[zone].cursor;
                        limit = decoder.zones[zone].limit;
                        source = decoder.zones[zone].source;
                        decoder.zone = zone;
                    }

                    _ => {
                        break 'syntax_error;
                    }
                }

                decoder.top = top;
            } /* general operator processing */
        } /* while ip < limit */

        /* No_Width: */
    }

    /* Syntax_Error: */
    Err(FT_ERR_SYNTAX_ERROR)
}

/// `t1_decoder_init`: initialize T1 decoder (`has_psnames` says whether
/// the `psnames' module is available; `face`, `size`, `glyph` and
/// `hinting` are the builder's; `buildchar` is the face's BuildCharArray,
/// whose length the caller sets)
#[allow(clippy::too_many_arguments)]
pub fn t1_decoder_init<'a>(
    has_psnames: bool,
    num_glyphs: FtLong,
    face: CffBuilderFace,
    size: Option<bool>,
    glyph: Option<CffGlyph<'a>>,
    glyph_names: Option<PsTableData>,
    blend: Option<&'a PsBlendRec>,
    hinting: bool,
    hint_mode: FtRenderMode,
    buildchar: &'a mut [FtLong],
) -> FtResult<T1DecoderRec<'a>> {
    /* retrieve `psnames' interface from list of current modules */
    if !has_psnames {
        return Err(FT_ERR_UNIMPLEMENTED_FEATURE);
    }

    let builder = t1_builder_init(face, size, glyph, hinting);

    /* decoder->buildchar and decoder->len_buildchar have to be  */
    /* initialized by the caller since we cannot know the length */
    /* of the BuildCharArray                                     */

    Ok(T1DecoderRec {
        builder,

        stack: [0; T1_MAX_CHARSTRINGS_OPERANDS],
        top: 0,

        zones: [T1DecoderZoneRec::default(); T1_MAX_SUBRS_CALLS + 1],
        zone: 0,

        num_glyphs: num_glyphs as FtUInt,
        glyph_names,
        charstrings: None,

        lenIV: 0,
        num_subrs: 0,
        subrs: None,
        cid_subrs: None,
        cid_subrs_bytes: None,
        subrs_hash: None,

        font_matrix: FtMatrix::default(),
        font_offset: FtVector::default(),

        flex_state: 0,
        num_flex_vectors: 0,
        flex_vectors: [FtVector::default(); 7],

        blend,

        hint_mode,

        len_buildchar: 0,
        buildchar,

        seac: false,

        cf2_instance: None,
    })
}

/// `t1_decoder_done`: finalize T1 decoder
pub fn t1_decoder_done(decoder: &mut T1DecoderRec<'_>) {
    t1_builder_done(&mut decoder.builder);

    /* (the font instance goes with it) */
    decoder.cf2_instance = None;
}

/// The Type 1 and CID drivers' glyph slot additions (`T1_GlyphSlotRec` and
/// `CID_GlyphSlotRec`, minus their root, which share
/// `CFF_GlyphSlotRec`'s first fields).
pub type T1GlyphSlotExt = CffGlyphSlotRec;
