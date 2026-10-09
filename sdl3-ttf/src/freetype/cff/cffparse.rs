// Rust translation of src/cff/cffparse.c, src/cff/cffparse.h and
// src/cff/cfftoken.h from FreeType (2.13.2, as SDL_ttf's external/freetype
// pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! CFF token stream parser (body).
//!
//! The parser's stack holds the positions of the operands: in the
//! dictionary's bytes, or (for CFF2 blend results) in the subfont's
//! `blend_stack`, which C points into directly. The field table generated
//! from `cfftoken.h` names each field, and storing a number keeps C's
//! conversion to the field's type. `CFF_CONFIG_OPTION_OLD_ENGINE` is not
//! defined, so opcode 31 (Type 2 charstrings in a DICT) is an unknown
//! operator.

use super::super::base::ftcalc::*;
use super::super::cfftypes::*;
use super::super::fttypes::*;
use super::cffload::{cff_blend_build_vector, cff_blend_check_vector, cff_blend_do_blend};

/* CFF uses constant parser stack size; */
/* CFF2 can increase from default 193   */
pub const CFF_MAX_STACK_DEPTH: FtUInt = 96;

/*
 * There are plans to remove the `maxstack' operator in a forthcoming
 * revision of the CFF2 specification, increasing the (then static) stack
 * size to 513.  By making the default stack size equal to the maximum
 * stack size, the operator is essentially disabled, which has the
 * desired effect in FreeType.
 */
pub const CFF2_MAX_STACK: FtUInt = 513;
pub const CFF2_DEFAULT_STACK: FtUInt = 513;

pub const CFF_CODE_TOPDICT: FtUInt = 0x1000;
pub const CFF_CODE_PRIVATE: FtUInt = 0x2000;
pub const CFF2_CODE_TOPDICT: FtUInt = 0x3000;
pub const CFF2_CODE_FONTDICT: FtUInt = 0x4000;
pub const CFF2_CODE_PRIVATE: FtUInt = 0x5000;

/// The position of an operand (C's `FT_Byte*` on the parser stack).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CffOperand {
    /// a position in the dictionary being parsed
    Dict(usize),
    /// a position in the subfont's `blend_stack`
    Blend(usize),
}

/// The object a parser fills (`parser->object`).
#[derive(Debug)]
pub enum CffParserObject<'o> {
    /// a `CFF_FontRecDict` (Top DICT or Font DICT)
    FontDict(&'o mut CffFontRecDictRec),
    /// a `CFF_Private`, with its subfont (`priv->subfont`) and the font's
    /// variation store (`blend->font->vstore`)
    Private {
        subfont: &'o mut CffSubFontRec,
        vstore: &'o CffVStoreRec,
    },
}

/// `CFF_ParserRec`
#[derive(Debug)]
pub struct CffParserRec<'o, 'd> {
    /// the dictionary being parsed (`start` is its first byte)
    pub dict: &'d [u8],
    pub limit: usize,
    pub cursor: usize,

    pub stack: Vec<CffOperand>,
    /// `top`, as an index into `stack`
    pub top: usize,
    pub stackSize: FtUInt, /* allocated size */

    pub object_code: FtUInt,
    pub object: CffParserObject<'o>,

    pub num_designs: FtUShort, /* a copy of `CFF_FontRecDict->num_designs' */
    pub num_axes: FtUShort,    /* a copy of `CFF_FontRecDict->num_axes'    */
}

impl CffParserRec<'_, '_> {
    /// The byte at `k` past the operand `d` (`d[0][k]`); bytes past the end
    /// of the buffer read as zero.
    fn byte(&self, d: CffOperand, k: usize) -> u8 {
        match d {
            CffOperand::Dict(p) => self.dict.get(p + k).copied().unwrap_or(0),
            CffOperand::Blend(p) => match &self.object {
                CffParserObject::Private { subfont, .. } => {
                    subfont.blend_stack.get(p + k).copied().unwrap_or(0)
                }
                CffParserObject::FontDict(_) => 0,
            },
        }
    }
}

/// `cff_parser_init`
pub fn cff_parser_init<'o, 'd>(
    code: FtUInt,
    object: CffParserObject<'o>,
    stack_size: FtUInt,
    num_designs: FtUShort,
    num_axes: FtUShort,
) -> FtResult<CffParserRec<'o, 'd>> {
    /* allocate the stack buffer */
    let mut stack = Vec::new();
    if stack.try_reserve_exact(stack_size as usize).is_err() {
        return Err(FT_ERR_OUT_OF_MEMORY);
    }
    stack.resize(stack_size as usize, CffOperand::Dict(0));

    Ok(CffParserRec {
        dict: &[],
        limit: 0,
        cursor: 0,
        stack,
        top: 0, /* empty stack */
        stackSize: stack_size,
        object_code: code,
        object,
        num_designs,
        num_axes,
    })
}

/// `cff_parser_done`: the parser's stack goes with it.
pub fn cff_parser_done(_parser: CffParserRec<'_, '_>) {}

/* The parser limit checks in the next two functions are supposed */
/* to detect the immediate crossing of the stream boundary.  They */
/* shall not be triggered from the distant t2_strings buffers.    */

/* read an integer */
fn cff_parse_integer(parser: &CffParserRec<'_, '_>, start: CffOperand, limit: usize) -> FtLong {
    let at = |k: usize| parser.byte(start, k);
    /* the limit checks compare positions in the dictionary; operands in */
    /* the blend stack are never integers                                */
    let p = match start {
        CffOperand::Dict(p) => p + 1,
        CffOperand::Blend(_) => usize::MAX,
    };
    let v = at(0) as FtInt;
    let val: FtLong;

    /* `p + n > limit && limit >= p' */
    let bad = |n: usize| p != usize::MAX && p + n > limit && limit >= p;

    if v == 28 {
        if bad(2) {
            return 0; /* Bad */
        }

        val = (((at(1) as FtUShort) << 8) | at(2) as FtUShort) as FtShort as FtLong;
    } else if v == 29 {
        if bad(4) {
            return 0; /* Bad */
        }

        val = (((at(1) as FtULong) << 24)
            | ((at(2) as FtULong) << 16)
            | ((at(3) as FtULong) << 8)
            | at(4) as FtULong) as u32 as i32 as FtLong;
    } else if v < 247 {
        val = (v - 139) as FtLong;
    } else if v < 251 {
        if bad(1) {
            return 0; /* Bad */
        }

        val = ((v - 247) * 256 + at(1) as FtInt + 108) as FtLong;
    } else {
        if bad(1) {
            return 0; /* Bad */
        }

        val = (-(v - 251) * 256 - at(1) as FtInt - 108) as FtLong;
    }

    val
}

static POWER_TENS: [FtLong; 10] = [
    1, 10, 100, 1000, 10000, 100000, 1000000, 10000000, 100000000, 1000000000,
];

/* maximum values allowed for multiplying      */
/* with the corresponding `power_tens' element */
static POWER_TEN_LIMITS: [FtLong; 10] = [
    i64::MAX,
    i64::MAX / 10,
    i64::MAX / 100,
    i64::MAX / 1000,
    i64::MAX / 10000,
    i64::MAX / 100000,
    i64::MAX / 1000000,
    i64::MAX / 10000000,
    i64::MAX / 100000000,
    i64::MAX / 1000000000,
];

/// `power_tens[i]` (the indices are within the table, as the digit counts
/// keep them; out-of-range ones read as 1 rather than past the table)
fn power_tens(i: FtLong) -> FtLong {
    POWER_TENS.get(i as usize).copied().unwrap_or(1)
}

/* read a real */
fn cff_parse_real(
    data: &[u8],
    start: usize,
    limit: usize,
    power_ten: FtLong,
    mut scaling: Option<&mut FtLong>,
) -> FtFixed {
    let mut p = start;
    let mut nib: FtInt;
    let mut phase: FtUInt;

    let mut result: FtLong;
    let mut number: FtLong;
    let mut exponent: FtLong;
    let mut sign: FtInt = 0;
    let mut exponent_sign: FtInt = 0;
    let mut have_overflow: FtInt = 0;
    let exponent_add: FtLong;
    let mut integer_length: FtLong;
    let mut fraction_length: FtLong;

    let at = |p: usize| data.get(p).copied().unwrap_or(0);

    if let Some(s) = scaling.as_deref_mut() {
        *s = 0;
    }

    result = 0;

    number = 0;
    exponent = 0;

    let mut exp_add: FtLong = 0;
    integer_length = 0;
    fraction_length = 0;

    'parse: {
        /* First of all, read the integer part. */
        phase = 4;

        loop {
            /* If we entered this iteration with phase == 4, we need to */
            /* read a new byte.  This also skips past the initial 0x1E. */
            if phase != 0 {
                p += 1;

                /* Make sure we don't read past the end. */
                if p + 1 > limit && limit >= p {
                    /* Bad: */
                    result = 0;
                    break 'parse;
                }
            }

            /* Get the nibble. */
            nib = (at(p) >> phase) as FtInt & 0xF;
            phase = 4 - phase;

            if nib == 0xE {
                sign = 1;
            } else if nib > 9 {
                break;
            } else {
                /* Increase exponent if we can't add the digit. */
                if number >= 0xCCCCCCC {
                    exp_add += 1;
                }
                /* Skip leading zeros. */
                else if nib != 0 || number != 0 {
                    integer_length += 1;
                    number = number * 10 + nib as FtLong;
                }
            }
        }

        /* Read fraction part, if any. */
        if nib == 0xA {
            loop {
                /* If we entered this iteration with phase == 4, we need */
                /* to read a new byte.                                   */
                if phase != 0 {
                    p += 1;

                    /* Make sure we don't read past the end. */
                    if p + 1 > limit && limit >= p {
                        /* Bad: */
                        result = 0;
                        break 'parse;
                    }
                }

                /* Get the nibble. */
                nib = ((at(p) >> phase) & 0xF) as FtInt;
                phase = 4 - phase;
                if nib >= 10 {
                    break;
                }

                /* Skip leading zeros if possible. */
                if nib == 0 && number == 0 {
                    exp_add -= 1;
                }
                /* Only add digit if we don't overflow. */
                else if number < 0xCCCCCCC && fraction_length < 9 {
                    fraction_length += 1;
                    number = number * 10 + nib as FtLong;
                }
            }
        }

        /* Read exponent, if any. */
        if nib == 12 {
            exponent_sign = 1;
            nib = 11;
        }

        if nib == 11 {
            loop {
                /* If we entered this iteration with phase == 4, */
                /* we need to read a new byte.                   */
                if phase != 0 {
                    p += 1;

                    /* Make sure we don't read past the end. */
                    if p + 1 > limit && limit >= p {
                        /* Bad: */
                        result = 0;
                        break 'parse;
                    }
                }

                /* Get the nibble. */
                nib = ((at(p) >> phase) & 0xF) as FtInt;
                phase = 4 - phase;
                if nib >= 10 {
                    break;
                }

                /* Arbitrarily limit exponent. */
                if exponent > 1000 {
                    have_overflow = 1;
                } else {
                    exponent = exponent * 10 + nib as FtLong;
                }
            }

            if exponent_sign != 0 {
                exponent = -exponent;
            }
        }

        exponent_add = exp_add;

        if number == 0 {
            break 'parse;
        }

        if have_overflow != 0 {
            if exponent_sign != 0 {
                /* Underflow: */
                result = 0;
            } else {
                /* Overflow: */
                result = 0x7FFFFFFF;
            }
            break 'parse;
        }

        /* We don't check `power_ten' and `exponent_add'. */
        exponent += power_ten + exponent_add;

        if let Some(scaling) = scaling {
            /* Only use `fraction_length'. */
            fraction_length += integer_length;
            exponent += integer_length;

            if fraction_length <= 5 {
                if number > 0x7FFF {
                    result = ft_div_fix(number, 10);
                    *scaling = exponent - fraction_length + 1;
                } else {
                    if exponent > 0 {
                        /* Make `scaling' as small as possible. */
                        let new_fraction_length = exponent.min(5);
                        let shift = new_fraction_length - fraction_length;

                        if shift > 0 {
                            exponent -= new_fraction_length;
                            number *= power_tens(shift);
                            if number > 0x7FFF {
                                number /= 10;
                                exponent += 1;
                            }
                        } else {
                            exponent -= fraction_length;
                        }
                    } else {
                        exponent -= fraction_length;
                    }

                    result = ((number as FtULong) << 16) as FtLong;
                    *scaling = exponent;
                }
            } else if (number / power_tens(fraction_length - 5)) > 0x7FFF {
                result = ft_div_fix(number, power_tens(fraction_length - 4));
                *scaling = exponent - 4;
            } else {
                result = ft_div_fix(number, power_tens(fraction_length - 5));
                *scaling = exponent - 5;
            }
        } else {
            integer_length += exponent;
            fraction_length -= exponent;

            if integer_length > 5 {
                /* Overflow: */
                result = 0x7FFFFFFF;
                break 'parse;
            }
            if integer_length < -5 {
                /* Underflow: */
                result = 0;
                break 'parse;
            }

            /* Remove non-significant digits. */
            if integer_length < 0 {
                number /= power_tens(-integer_length);
                fraction_length += integer_length;
            }

            /* this can only happen if exponent was non-zero */
            if fraction_length == 10 {
                number /= 10;
                fraction_length -= 1;
            }

            /* Convert into 16.16 format. */
            if fraction_length > 0 {
                if (number / power_tens(fraction_length)) > 0x7FFF {
                    break 'parse;
                }

                result = ft_div_fix(number, power_tens(fraction_length));
            } else {
                number *= power_tens(-fraction_length);

                if number > 0x7FFF {
                    /* Overflow: */
                    result = 0x7FFFFFFF;
                    break 'parse;
                }

                result = ((number as FtULong) << 16) as FtLong;
            }
        }
    }

    /* Exit: */
    if sign != 0 {
        result = -result;
    }

    result
}

/// The 32-bit value of a blend result (`255` and four bytes).
fn blend_bytes(parser: &CffParserRec<'_, '_>, d: CffOperand) -> FtUInt32 {
    ((parser.byte(d, 1) as FtUInt32) << 24)
        | ((parser.byte(d, 2) as FtUInt32) << 16)
        | ((parser.byte(d, 3) as FtUInt32) << 8)
        | parser.byte(d, 4) as FtUInt32
}

/// The start of a real number operand in the dictionary.
fn real_start(d: CffOperand) -> usize {
    match d {
        CffOperand::Dict(p) => p,
        CffOperand::Blend(_) => usize::MAX,
    }
}

/// `cff_parse_num`: read a number, either integer or real
pub fn cff_parse_num(parser: &CffParserRec<'_, '_>, d: CffOperand) -> FtLong {
    if parser.byte(d, 0) == 30 {
        /* binary-coded decimal is truncated to integer */
        cff_parse_real(parser.dict, real_start(d), parser.limit, 0, None) >> 16
    } else if parser.byte(d, 0) == 255 {
        /* 16.16 fixed-point is used internally for CFF2 blend results. */
        /* Since these are trusted values, a limit check is not needed. */

        /* After the 255, 4 bytes give the number.                 */
        /* The blend value is converted to integer, with rounding; */
        /* due to the right-shift we don't need the lowest byte.   */
        ((((parser.byte(d, 1) as FtUInt32) << 16)
            | ((parser.byte(d, 2) as FtUInt32) << 8)
            | parser.byte(d, 3) as FtUInt32)
            .wrapping_add(0x80)
            >> 8) as FtShort as FtLong
    } else {
        cff_parse_integer(parser, d, parser.limit)
    }
}

/* read a floating point number, either integer or real */
fn do_fixed(parser: &CffParserRec<'_, '_>, d: CffOperand, scaling: FtLong) -> FtFixed {
    if parser.byte(d, 0) == 30 {
        cff_parse_real(parser.dict, real_start(d), parser.limit, scaling, None)
    } else if parser.byte(d, 0) == 255 {
        /* FIXME (upstream): the four bytes are read as an unsigned 32-bit */
        /* value, so that a negative blend result becomes a large positive */
        /* `FT_Fixed' (C's conversion is kept here)                        */
        let mut val: FtFixed = blend_bytes(parser, d) as FtFixed;

        if scaling != 0 {
            if val.abs() > POWER_TEN_LIMITS[scaling as usize] {
                return if val > 0 { 0x7FFFFFFF } else { -0x7FFFFFFF };
            }
            val *= power_tens(scaling);
        }
        val
    } else {
        let mut val: FtLong = cff_parse_integer(parser, d, parser.limit);

        if scaling != 0 {
            if (val.abs() << 16) > POWER_TEN_LIMITS[scaling as usize] {
                val = if val > 0 { 0x7FFFFFFF } else { -0x7FFFFFFF };
                /* Overflow: */
                return val;
            }

            val *= power_tens(scaling);
        }

        if val > 0x7FFF {
            val = 0x7FFFFFFF;
            /* Overflow: */
            return val;
        } else if val < -0x7FFF {
            val = -0x7FFFFFFF;
            /* Overflow: */
            return val;
        }

        ((val as FtULong) << 16) as FtLong
    }
}

/// `cff_parse_fixed`: read a floating point number, either integer or real
pub fn cff_parse_fixed(parser: &CffParserRec<'_, '_>, d: CffOperand) -> FtFixed {
    do_fixed(parser, d, 0)
}

/* read a floating point number, either integer or real, */
/* but return `10^scaling' times the number read in      */
fn cff_parse_fixed_scaled(
    parser: &CffParserRec<'_, '_>,
    d: CffOperand,
    scaling: FtLong,
) -> FtFixed {
    do_fixed(parser, d, scaling)
}

/* read a floating point number, either integer or real,     */
/* and return it as precise as possible -- `scaling' returns */
/* the scaling factor (as a power of 10)                     */
fn cff_parse_fixed_dynamic(
    parser: &CffParserRec<'_, '_>,
    d: CffOperand,
    scaling: &mut FtLong,
) -> FtFixed {
    if parser.byte(d, 0) == 30 {
        cff_parse_real(parser.dict, real_start(d), parser.limit, 0, Some(scaling))
    } else {
        let number: FtLong = cff_parse_integer(parser, d, parser.limit);
        let mut integer_length: FtInt;

        if number > 0x7FFF {
            integer_length = 5;
            while integer_length < 10 {
                if number < POWER_TENS[integer_length as usize] {
                    break;
                }
                integer_length += 1;
            }

            if (number / power_tens(integer_length as FtLong - 5)) > 0x7FFF {
                *scaling = integer_length as FtLong - 4;
                ft_div_fix(number, power_tens(integer_length as FtLong - 4))
            } else {
                *scaling = integer_length as FtLong - 5;
                ft_div_fix(number, power_tens(integer_length as FtLong - 5))
            }
        } else {
            *scaling = 0;
            ((number as FtULong) << 16) as FtLong
        }
    }
}

/// The `CFF_FontRecDict` a parser fills.
fn font_dict<'a>(parser: &'a mut CffParserRec<'_, '_>) -> Option<&'a mut CffFontRecDictRec> {
    match &mut parser.object {
        CffParserObject::FontDict(d) => Some(d),
        CffParserObject::Private { .. } => None,
    }
}

fn cff_parse_font_matrix(parser: &mut CffParserRec<'_, '_>) -> FtResult<()> {
    if parser.top >= 6 {
        let mut values: [FtFixed; 6] = [0; 6];
        let mut scalings: [FtLong; 6] = [0; 6];

        let mut min_scaling: FtLong;
        let mut max_scaling: FtLong;

        /* We expect a well-formed font matrix, that is, the matrix elements */
        /* `xx' and `yy' are of approximately the same magnitude.  To avoid  */
        /* loss of precision, we use the magnitude of the largest matrix     */
        /* element to scale all other elements.  The scaling factor is then  */
        /* contained in the `units_per_em' value.                            */

        max_scaling = FtLong::MIN;
        min_scaling = FtLong::MAX;

        for i in 0..6 {
            values[i] = cff_parse_fixed_dynamic(parser, parser.stack[i], &mut scalings[i]);
            if values[i] != 0 {
                if scalings[i] > max_scaling {
                    max_scaling = scalings[i];
                }
                if scalings[i] < min_scaling {
                    min_scaling = scalings[i];
                }
            }
        }

        let Some(dict) = font_dict(parser) else {
            return Ok(());
        };
        dict.has_font_matrix = true;

        let unlikely = max_scaling < -9
            || max_scaling > 0
            || max_scaling.wrapping_sub(min_scaling) < 0
            || max_scaling.wrapping_sub(min_scaling) > 9;

        if !unlikely {
            for i in 0..6 {
                let value = values[i];

                if value == 0 {
                    continue;
                }

                let divisor = power_tens(max_scaling - scalings[i]);
                let half_divisor = divisor >> 1;

                if value < 0 {
                    if FtLong::MIN + half_divisor < value {
                        values[i] = (value - half_divisor) / divisor;
                    } else {
                        values[i] = FtLong::MIN / divisor;
                    }
                } else if FtLong::MAX - half_divisor > value {
                    values[i] = (value + half_divisor) / divisor;
                } else {
                    values[i] = FtLong::MAX / divisor;
                }
            }

            dict.font_matrix.xx = values[0];
            dict.font_matrix.yx = values[1];
            dict.font_matrix.xy = values[2];
            dict.font_matrix.yy = values[3];
            dict.font_offset.x = values[4];
            dict.font_offset.y = values[5];

            dict.units_per_em = power_tens(-max_scaling) as FtULong;

            if ft_matrix_check(&dict.font_matrix) {
                return Ok(());
            }
        }

        /* Unlikely: */
        /* Return default matrix in case of unlikely values. */

        dict.font_matrix.xx = 0x10000;
        dict.font_matrix.yx = 0;
        dict.font_matrix.xy = 0;
        dict.font_matrix.yy = 0x10000;
        dict.font_offset.x = 0;
        dict.font_offset.y = 0;
        dict.units_per_em = 1;

        Ok(())
    } else {
        Err(FT_ERR_STACK_UNDERFLOW)
    }
}

fn cff_parse_font_bbox(parser: &mut CffParserRec<'_, '_>) -> FtResult<()> {
    if parser.top >= 4 {
        let x_min = ft_round_fix(cff_parse_fixed(parser, parser.stack[0]));
        let y_min = ft_round_fix(cff_parse_fixed(parser, parser.stack[1]));
        let x_max = ft_round_fix(cff_parse_fixed(parser, parser.stack[2]));
        let y_max = ft_round_fix(cff_parse_fixed(parser, parser.stack[3]));

        if let Some(dict) = font_dict(parser) {
            dict.font_bbox.xMin = x_min;
            dict.font_bbox.yMin = y_min;
            dict.font_bbox.xMax = x_max;
            dict.font_bbox.yMax = y_max;
        }
        return Ok(());
    }

    Err(FT_ERR_STACK_UNDERFLOW)
}

fn cff_parse_private_dict(parser: &mut CffParserRec<'_, '_>) -> FtResult<()> {
    if parser.top >= 2 {
        let tmp = cff_parse_num(parser, parser.stack[0]);
        if tmp < 0 {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }
        if let Some(dict) = font_dict(parser) {
            dict.private_size = tmp as FtULong;
        }

        let tmp = cff_parse_num(parser, parser.stack[1]);
        if tmp < 0 {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }
        if let Some(dict) = font_dict(parser) {
            dict.private_offset = tmp as FtULong;
        }

        return Ok(());
    }

    Err(FT_ERR_STACK_UNDERFLOW)
}

/* The `MultipleMaster' operator comes before any  */
/* top DICT operators that contain T2 charstrings. */

fn cff_parse_multiple_master(parser: &mut CffParserRec<'_, '_>) -> FtResult<()> {
    /* currently, we handle only the first argument */
    if parser.top >= 5 {
        let num_designs = cff_parse_num(parser, parser.stack[0]);

        if !(2..=16).contains(&num_designs) {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        let num_axes = (parser.top - 4) as FtUShort;
        if let Some(dict) = font_dict(parser) {
            dict.num_designs = num_designs as FtUShort;
            dict.num_axes = num_axes;
        }

        parser.num_designs = num_designs as FtUShort;
        parser.num_axes = num_axes;

        return Ok(());
    }

    Err(FT_ERR_STACK_UNDERFLOW)
}

fn cff_parse_cid_ros(parser: &mut CffParserRec<'_, '_>) -> FtResult<()> {
    if parser.top >= 3 {
        let registry = cff_parse_num(parser, parser.stack[0]) as FtUInt;
        let ordering = cff_parse_num(parser, parser.stack[1]) as FtUInt;
        /* (a real supplement is rounded) */
        let supplement = cff_parse_num(parser, parser.stack[2]);

        if let Some(dict) = font_dict(parser) {
            dict.cid_registry = registry;
            dict.cid_ordering = ordering;
            dict.cid_supplement = supplement;
        }

        return Ok(());
    }

    Err(FT_ERR_STACK_UNDERFLOW)
}

fn cff_parse_vsindex(parser: &mut CffParserRec<'_, '_>) -> FtResult<()> {
    /* vsindex operator can only be used in a Private DICT */
    let vsindex = cff_parse_num(parser, parser.stack[0]) as FtUInt;

    let CffParserObject::Private { subfont, .. } = &mut parser.object else {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    };
    if !subfont.private_dict.subfont {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    if subfont.blend.usedBV {
        return Err(FT_ERR_SYNTAX_ERROR);
    }

    subfont.private_dict.vsindex = vsindex;

    Ok(())
}

fn cff_parse_blend(parser: &mut CffParserRec<'_, '_>) -> FtResult<()> {
    /* blend operator can only be used in a Private DICT */
    {
        let CffParserObject::Private { subfont, vstore } = &mut parser.object else {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        };
        if !subfont.private_dict.subfont {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        let sub: &mut CffSubFontRec = subfont;
        if cff_blend_check_vector(
            &sub.blend,
            sub.private_dict.vsindex,
            sub.lenNDV,
            sub.NDV.as_deref(),
        ) {
            cff_blend_build_vector(
                &mut sub.blend,
                vstore,
                sub.private_dict.vsindex,
                sub.lenNDV,
                sub.NDV.as_deref(),
            )?;
        }
    }

    let num_blends = cff_parse_num(parser, parser.stack[parser.top - 1]) as FtUInt;
    if num_blends > parser.stackSize {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    let error = cff_blend_do_blend(parser, num_blends);

    if let CffParserObject::Private { subfont, .. } = &mut parser.object {
        subfont.blend.usedBV = true;
    }

    error
}

/* maxstack operator increases parser and operand stacks for CFF2 */
fn cff_parse_maxstack(parser: &mut CffParserRec<'_, '_>) -> FtResult<()> {
    /* maxstack operator can only be used in a Top DICT */
    let maxstack = cff_parse_num(parser, parser.stack[0]) as FtUInt;

    let Some(dict) = font_dict(parser) else {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    };

    dict.maxstack = maxstack;
    if dict.maxstack > CFF2_MAX_STACK {
        dict.maxstack = CFF2_MAX_STACK;
    }
    if dict.maxstack < CFF2_DEFAULT_STACK {
        dict.maxstack = CFF2_DEFAULT_STACK;
    }

    Ok(())
}

/* the kinds of fields */
const CFF_KIND_NONE: FtInt = 0;
const CFF_KIND_NUM: FtInt = 1;
const CFF_KIND_FIXED: FtInt = 2;
const CFF_KIND_FIXED_THOUSAND: FtInt = 3;
const CFF_KIND_STRING: FtInt = 4;
const CFF_KIND_BOOL: FtInt = 5;
const CFF_KIND_DELTA: FtInt = 6;
const CFF_KIND_CALLBACK: FtInt = 7;
const CFF_KIND_BLEND: FtInt = 8;

/// The fields of `CFF_FontRecDictRec` and `CFF_PrivateRec` that the
/// table stores numbers into (C's `offset` and `size`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CffField {
    None,
    /* CFF_FontRecDictRec */
    Version,
    Notice,
    Copyright,
    FullName,
    FamilyName,
    Weight,
    IsFixedPitch,
    ItalicAngle,
    UnderlinePosition,
    UnderlineThickness,
    PaintType,
    CharstringType,
    UniqueId,
    StrokeWidth,
    CharsetOffset,
    EncodingOffset,
    CharstringsOffset,
    SyntheticBase,
    EmbeddedPostscript,
    CidFontVersion,
    CidFontRevision,
    CidFontType,
    CidCount,
    CidUidBase,
    CidFdArrayOffset,
    CidFdSelectOffset,
    CidFontName,
    VstoreOffset,
    /* CFF_PrivateRec */
    BlueValues,
    OtherBlues,
    FamilyBlues,
    FamilyOtherBlues,
    BlueScale,
    BlueShift,
    BlueFuzz,
    StandardWidth,
    StandardHeight,
    SnapWidths,
    SnapHeights,
    ForceBold,
    ForceBoldThreshold,
    LenIV,
    LanguageGroup,
    ExpansionFactor,
    InitialRandomSeed,
    LocalSubrsOffset,
    DefaultWidth,
    NominalWidth,
}

type CffFieldReader = fn(parser: &mut CffParserRec<'_, '_>) -> FtResult<()>;

/// `CFF_Field_Handler`
struct CffFieldHandler {
    kind: FtInt,
    code: FtInt,
    field: CffField,
    reader: Option<CffFieldReader>,
    array_max: FtUInt,
}

macro_rules! field {
    ($kind:expr, $code:expr, $cffcode:expr, $field:ident) => {
        CffFieldHandler {
            kind: $kind,
            code: ($code | $cffcode) as FtInt,
            field: CffField::$field,
            reader: None,
            array_max: 0,
        }
    };
}
macro_rules! callback {
    ($code:expr, $cffcode:expr, $reader:ident) => {
        CffFieldHandler {
            kind: CFF_KIND_CALLBACK,
            code: ($code | $cffcode) as FtInt,
            field: CffField::None,
            reader: Some($reader),
            array_max: 0,
        }
    };
}
macro_rules! delta {
    ($code:expr, $cffcode:expr, $field:ident, $max:expr) => {
        CffFieldHandler {
            kind: CFF_KIND_DELTA,
            code: ($code | $cffcode) as FtInt,
            field: CffField::$field,
            reader: None,
            array_max: $max,
        }
    };
}

/* cfftoken.h */
static CFF_FIELD_HANDLERS: [CffFieldHandler; 77] = [
    /* FT_STRUCTURE CFF_FontRecDictRec, CFFCODE CFF_CODE_TOPDICT */
    field!(CFF_KIND_STRING, 0, CFF_CODE_TOPDICT, Version),
    field!(CFF_KIND_STRING, 1, CFF_CODE_TOPDICT, Notice),
    field!(CFF_KIND_STRING, 0x100, CFF_CODE_TOPDICT, Copyright),
    field!(CFF_KIND_STRING, 2, CFF_CODE_TOPDICT, FullName),
    field!(CFF_KIND_STRING, 3, CFF_CODE_TOPDICT, FamilyName),
    field!(CFF_KIND_STRING, 4, CFF_CODE_TOPDICT, Weight),
    field!(CFF_KIND_BOOL, 0x101, CFF_CODE_TOPDICT, IsFixedPitch),
    field!(CFF_KIND_FIXED, 0x102, CFF_CODE_TOPDICT, ItalicAngle),
    field!(CFF_KIND_FIXED, 0x103, CFF_CODE_TOPDICT, UnderlinePosition),
    field!(CFF_KIND_FIXED, 0x104, CFF_CODE_TOPDICT, UnderlineThickness),
    field!(CFF_KIND_NUM, 0x105, CFF_CODE_TOPDICT, PaintType),
    field!(CFF_KIND_NUM, 0x106, CFF_CODE_TOPDICT, CharstringType),
    callback!(0x107, CFF_CODE_TOPDICT, cff_parse_font_matrix),
    field!(CFF_KIND_NUM, 13, CFF_CODE_TOPDICT, UniqueId),
    callback!(5, CFF_CODE_TOPDICT, cff_parse_font_bbox),
    field!(CFF_KIND_NUM, 0x108, CFF_CODE_TOPDICT, StrokeWidth),
    field!(CFF_KIND_NUM, 15, CFF_CODE_TOPDICT, CharsetOffset),
    field!(CFF_KIND_NUM, 16, CFF_CODE_TOPDICT, EncodingOffset),
    field!(CFF_KIND_NUM, 17, CFF_CODE_TOPDICT, CharstringsOffset),
    callback!(18, CFF_CODE_TOPDICT, cff_parse_private_dict),
    field!(CFF_KIND_NUM, 0x114, CFF_CODE_TOPDICT, SyntheticBase),
    field!(CFF_KIND_STRING, 0x115, CFF_CODE_TOPDICT, EmbeddedPostscript),
    /* the next two operators were removed from the Type2 specification */
    /* in version 16-March-2000                                         */
    callback!(0x118, CFF_CODE_TOPDICT, cff_parse_multiple_master),
    callback!(0x11E, CFF_CODE_TOPDICT, cff_parse_cid_ros),
    field!(CFF_KIND_NUM, 0x11F, CFF_CODE_TOPDICT, CidFontVersion),
    field!(CFF_KIND_NUM, 0x120, CFF_CODE_TOPDICT, CidFontRevision),
    field!(CFF_KIND_NUM, 0x121, CFF_CODE_TOPDICT, CidFontType),
    field!(CFF_KIND_NUM, 0x122, CFF_CODE_TOPDICT, CidCount),
    field!(CFF_KIND_NUM, 0x123, CFF_CODE_TOPDICT, CidUidBase),
    field!(CFF_KIND_NUM, 0x124, CFF_CODE_TOPDICT, CidFdArrayOffset),
    field!(CFF_KIND_NUM, 0x125, CFF_CODE_TOPDICT, CidFdSelectOffset),
    field!(CFF_KIND_STRING, 0x126, CFF_CODE_TOPDICT, CidFontName),
    /* FT_STRUCTURE CFF_PrivateRec, CFFCODE CFF_CODE_PRIVATE */
    delta!(6, CFF_CODE_PRIVATE, BlueValues, 14),
    delta!(7, CFF_CODE_PRIVATE, OtherBlues, 10),
    delta!(8, CFF_CODE_PRIVATE, FamilyBlues, 14),
    delta!(9, CFF_CODE_PRIVATE, FamilyOtherBlues, 10),
    field!(CFF_KIND_FIXED_THOUSAND, 0x109, CFF_CODE_PRIVATE, BlueScale),
    field!(CFF_KIND_NUM, 0x10A, CFF_CODE_PRIVATE, BlueShift),
    field!(CFF_KIND_NUM, 0x10B, CFF_CODE_PRIVATE, BlueFuzz),
    field!(CFF_KIND_NUM, 10, CFF_CODE_PRIVATE, StandardWidth),
    field!(CFF_KIND_NUM, 11, CFF_CODE_PRIVATE, StandardHeight),
    delta!(0x10C, CFF_CODE_PRIVATE, SnapWidths, 13),
    delta!(0x10D, CFF_CODE_PRIVATE, SnapHeights, 13),
    field!(CFF_KIND_BOOL, 0x10E, CFF_CODE_PRIVATE, ForceBold),
    field!(CFF_KIND_FIXED, 0x10F, CFF_CODE_PRIVATE, ForceBoldThreshold),
    field!(CFF_KIND_NUM, 0x110, CFF_CODE_PRIVATE, LenIV),
    field!(CFF_KIND_NUM, 0x111, CFF_CODE_PRIVATE, LanguageGroup),
    field!(CFF_KIND_FIXED, 0x112, CFF_CODE_PRIVATE, ExpansionFactor),
    field!(CFF_KIND_NUM, 0x113, CFF_CODE_PRIVATE, InitialRandomSeed),
    field!(CFF_KIND_NUM, 19, CFF_CODE_PRIVATE, LocalSubrsOffset),
    field!(CFF_KIND_NUM, 20, CFF_CODE_PRIVATE, DefaultWidth),
    field!(CFF_KIND_NUM, 21, CFF_CODE_PRIVATE, NominalWidth),
    /* FT_STRUCTURE CFF_FontRecDictRec, CFFCODE CFF2_CODE_TOPDICT */
    callback!(0x107, CFF2_CODE_TOPDICT, cff_parse_font_matrix),
    field!(CFF_KIND_NUM, 17, CFF2_CODE_TOPDICT, CharstringsOffset),
    field!(CFF_KIND_NUM, 0x124, CFF2_CODE_TOPDICT, CidFdArrayOffset),
    field!(CFF_KIND_NUM, 0x125, CFF2_CODE_TOPDICT, CidFdSelectOffset),
    field!(CFF_KIND_NUM, 24, CFF2_CODE_TOPDICT, VstoreOffset),
    callback!(25, CFF2_CODE_TOPDICT, cff_parse_maxstack),
    /* FT_STRUCTURE CFF_FontRecDictRec, CFFCODE CFF2_CODE_FONTDICT */
    callback!(18, CFF2_CODE_FONTDICT, cff_parse_private_dict),
    callback!(0x107, CFF2_CODE_FONTDICT, cff_parse_font_matrix),
    /* FT_STRUCTURE CFF_PrivateRec, CFFCODE CFF2_CODE_PRIVATE */
    delta!(6, CFF2_CODE_PRIVATE, BlueValues, 14),
    delta!(7, CFF2_CODE_PRIVATE, OtherBlues, 10),
    delta!(8, CFF2_CODE_PRIVATE, FamilyBlues, 14),
    delta!(9, CFF2_CODE_PRIVATE, FamilyOtherBlues, 10),
    field!(CFF_KIND_FIXED_THOUSAND, 0x109, CFF2_CODE_PRIVATE, BlueScale),
    field!(CFF_KIND_NUM, 0x10A, CFF2_CODE_PRIVATE, BlueShift),
    field!(CFF_KIND_NUM, 0x10B, CFF2_CODE_PRIVATE, BlueFuzz),
    field!(CFF_KIND_NUM, 10, CFF2_CODE_PRIVATE, StandardWidth),
    field!(CFF_KIND_NUM, 11, CFF2_CODE_PRIVATE, StandardHeight),
    delta!(0x10C, CFF2_CODE_PRIVATE, SnapWidths, 13),
    delta!(0x10D, CFF2_CODE_PRIVATE, SnapHeights, 13),
    field!(CFF_KIND_NUM, 0x111, CFF2_CODE_PRIVATE, LanguageGroup),
    field!(CFF_KIND_FIXED, 0x112, CFF2_CODE_PRIVATE, ExpansionFactor),
    callback!(22, CFF2_CODE_PRIVATE, cff_parse_vsindex),
    CffFieldHandler {
        kind: CFF_KIND_BLEND,
        code: (23 | CFF2_CODE_PRIVATE) as FtInt,
        field: CffField::None,
        reader: Some(cff_parse_blend),
        array_max: 0,
    },
    field!(CFF_KIND_NUM, 19, CFF2_CODE_PRIVATE, LocalSubrsOffset),
    /* (end marker) */
    CffFieldHandler {
        kind: CFF_KIND_NONE,
        code: 0,
        field: CffField::None,
        reader: None,
        array_max: 0,
    },
];

/// The `Store_Number` part of `cff_parser_run`: stores `val` in `field`
/// as C's assignment to the field's type does (`FT_Bool` fields keep the
/// low byte, `FT_Int`/`FT_UInt` ones the low 32 bits).
fn cff_store_number(object: &mut CffParserObject<'_>, field: CffField, val: FtLong) {
    use CffField::*;

    let as_uint = val as FtInt as FtUInt;
    match object {
        CffParserObject::FontDict(d) => match field {
            Version => d.version = as_uint,
            Notice => d.notice = as_uint,
            Copyright => d.copyright = as_uint,
            FullName => d.full_name = as_uint,
            FamilyName => d.family_name = as_uint,
            Weight => d.weight = as_uint,
            IsFixedPitch => d.is_fixed_pitch = val as FtByte,
            ItalicAngle => d.italic_angle = val,
            UnderlinePosition => d.underline_position = val,
            UnderlineThickness => d.underline_thickness = val,
            PaintType => d.paint_type = val as FtInt,
            CharstringType => d.charstring_type = val as FtInt,
            UniqueId => d.unique_id = val as FtULong,
            StrokeWidth => d.stroke_width = val,
            CharsetOffset => d.charset_offset = val as FtULong,
            EncodingOffset => d.encoding_offset = val as FtULong,
            CharstringsOffset => d.charstrings_offset = val as FtULong,
            SyntheticBase => d.synthetic_base = val,
            EmbeddedPostscript => d.embedded_postscript = as_uint,
            CidFontVersion => d.cid_font_version = val,
            CidFontRevision => d.cid_font_revision = val,
            CidFontType => d.cid_font_type = val,
            CidCount => d.cid_count = val as FtULong,
            CidUidBase => d.cid_uid_base = val as FtULong,
            CidFdArrayOffset => d.cid_fd_array_offset = val as FtULong,
            CidFdSelectOffset => d.cid_fd_select_offset = val as FtULong,
            CidFontName => d.cid_font_name = as_uint,
            VstoreOffset => d.vstore_offset = val as FtULong,
            _ => {}
        },
        CffParserObject::Private { subfont, .. } => {
            let p = &mut subfont.private_dict;
            match field {
                BlueScale => p.blue_scale = val,
                BlueShift => p.blue_shift = val,
                BlueFuzz => p.blue_fuzz = val,
                StandardWidth => p.standard_width = val,
                StandardHeight => p.standard_height = val,
                ForceBold => p.force_bold = val as FtByte,
                ForceBoldThreshold => p.force_bold_threshold = val,
                LenIV => p.lenIV = val as FtInt,
                LanguageGroup => p.language_group = val as FtInt,
                ExpansionFactor => p.expansion_factor = val,
                InitialRandomSeed => p.initial_random_seed = val,
                LocalSubrsOffset => p.local_subrs_offset = val as FtULong,
                DefaultWidth => p.default_width = val,
                NominalWidth => p.nominal_width = val,
                _ => {}
            }
        }
    }
}

/// The array and count of a delta field (`CFF_FIELD_DELTA`).
fn cff_delta_field<'a>(
    object: &'a mut CffParserObject<'_>,
    field: CffField,
) -> Option<(&'a mut [FtPos], &'a mut FtByte)> {
    use CffField::*;

    match object {
        CffParserObject::Private { subfont, .. } => {
            let p = &mut subfont.private_dict;
            match field {
                BlueValues => Some((&mut p.blue_values[..], &mut p.num_blue_values)),
                OtherBlues => Some((&mut p.other_blues[..], &mut p.num_other_blues)),
                FamilyBlues => Some((&mut p.family_blues[..], &mut p.num_family_blues)),
                FamilyOtherBlues => {
                    Some((&mut p.family_other_blues[..], &mut p.num_family_other_blues))
                }
                SnapWidths => Some((&mut p.snap_widths[..], &mut p.num_snap_widths)),
                SnapHeights => Some((&mut p.snap_heights[..], &mut p.num_snap_heights)),
                _ => Option::None,
            }
        }
        CffParserObject::FontDict(_) => Option::None,
    }
}

/// `cff_parser_run`: parses the dictionary `dict` (from `start` to
/// `limit`).
pub fn cff_parser_run<'d>(parser: &mut CffParserRec<'_, 'd>, dict: &'d [u8]) -> FtResult<()> {
    let start = 0usize;
    let limit = dict.len();
    let mut p = start;

    parser.top = 0;
    parser.dict = dict;
    parser.limit = limit;
    parser.cursor = start;

    let at = |p: usize| dict.get(p).copied().unwrap_or(0);

    while p < limit {
        let mut v = at(p) as FtUInt;

        /* Opcode 31 is legacy MM T2 operator, not a number.      */
        /* Opcode 255 is reserved and should not appear in fonts; */
        /* it is used internally for CFF2 blends.                 */
        if v >= 27 && v != 31 && v != 255 {
            /* it's a number; we will push its position on the stack */
            if parser.top as FtUInt >= parser.stackSize {
                /* Stack_Overflow: */
                return Err(FT_ERR_INVALID_ARGUMENT);
            }

            parser.stack[parser.top] = CffOperand::Dict(p);
            parser.top += 1;

            /* now, skip it */
            if v == 30 {
                /* skip real number */
                p += 1;
                loop {
                    /* An unterminated floating point number at the */
                    /* end of a dictionary is invalid but harmless. */
                    if p >= limit {
                        return Ok(());
                    }
                    v = (at(p) >> 4) as FtUInt;
                    if v == 15 {
                        break;
                    }
                    v = (at(p) & 0xF) as FtUInt;
                    if v == 15 {
                        break;
                    }
                    p += 1;
                }
            } else if v == 28 {
                p += 2;
            } else if v == 29 {
                p += 4;
            } else if v > 246 {
                p += 1;
            }
        } else {
            /* This is not a number, hence it's an operator.  Compute its code */
            /* and look for it in our current list.                            */

            if parser.top as FtUInt >= parser.stackSize {
                /* Stack_Overflow: */
                return Err(FT_ERR_INVALID_ARGUMENT);
            }

            let mut num_args = parser.top as FtUInt;
            parser.stack[parser.top] = CffOperand::Dict(p);
            let mut code = v;

            if v == 12 {
                /* two byte operator */
                p += 1;
                if p >= limit {
                    /* Syntax_Error: */
                    return Err(FT_ERR_INVALID_ARGUMENT);
                }

                code = 0x100 | at(p) as FtUInt;
            }
            code |= parser.object_code;

            let mut fi = 0;
            while CFF_FIELD_HANDLERS[fi].kind != CFF_KIND_NONE {
                let field = &CFF_FIELD_HANDLERS[fi];
                if field.code == code as FtInt {
                    /* we found our field's handler; read it */

                    /* check that we have enough arguments -- except for */
                    /* delta encoded arrays, which can be empty          */
                    if field.kind != CFF_KIND_DELTA && num_args < 1 {
                        /* Stack_Underflow: */
                        return Err(FT_ERR_INVALID_ARGUMENT);
                    }

                    match field.kind {
                        CFF_KIND_BOOL | CFF_KIND_STRING | CFF_KIND_NUM => {
                            let val = cff_parse_num(parser, parser.stack[0]);
                            cff_store_number(&mut parser.object, field.field, val);
                        }

                        CFF_KIND_FIXED => {
                            let val = cff_parse_fixed(parser, parser.stack[0]);
                            cff_store_number(&mut parser.object, field.field, val);
                        }

                        CFF_KIND_FIXED_THOUSAND => {
                            let val = cff_parse_fixed_scaled(parser, parser.stack[0], 3);
                            cff_store_number(&mut parser.object, field.field, val);
                        }

                        CFF_KIND_DELTA => {
                            if num_args > field.array_max {
                                num_args = field.array_max;
                            }

                            let mut vals = [0 as FtLong; 14];
                            let mut val: FtLong = 0;
                            for (k, slot) in vals.iter_mut().enumerate().take(num_args as usize) {
                                val = add_long(val, cff_parse_num(parser, parser.stack[k]));
                                *slot = val;
                            }

                            if let Some((array, qcount)) =
                                cff_delta_field(&mut parser.object, field.field)
                            {
                                /* store count */
                                *qcount = num_args as FtByte;

                                array[..num_args as usize]
                                    .copy_from_slice(&vals[..num_args as usize]);
                            }
                        }

                        _ => {
                            /* callback or blend */
                            (field.reader.unwrap())(parser)?;
                        }
                    }
                    break; /* goto Found */
                }
                fi += 1;
            }

            /* this is an unknown operator, or it is unsupported; */
            /* we will ignore it for now.                         */

            /* Found: */
            /* clear stack */
            /* TODO: could clear blend stack here,       */
            /*       but we don't have access to subFont */
            if CFF_FIELD_HANDLERS[fi].kind != CFF_KIND_BLEND {
                parser.top = 0;
            }
        }
        p += 1;
    } /* while ( p < limit ) */

    Ok(())
}
