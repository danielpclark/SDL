// Rust translation of src/psaux/psintrp.c and src/psaux/psintrp.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright 2007-2014 Adobe Systems Incorporated.
//
// This software, and all works of authorship, whether in source or
// object code form as indicated by the copyright notice(s) included
// herein (collectively, the "Work") is made available, and may only be
// used, modified, and distributed under the FreeType Project License,
// LICENSE.TXT.  Additionally, subject to the terms and conditions of the
// FreeType Project License, each contributor to the Work hereby grants
// to any individual or legal entity exercising permissions granted by
// the FreeType Project License and this section (hereafter, "You" or
// "Your") a perpetual, worldwide, non-exclusive, no-charge,
// royalty-free, irrevocable (except as stated in this section) patent
// license to make, have made, use, offer to sell, sell, import, and
// otherwise transfer the Work, where such license applies only to those
// patent claims licensable by such contributor that are necessarily
// infringed by their contribution(s) alone or by combination of their
// contribution(s) with the Work to which such contribution(s) was
// submitted.  If You institute patent litigation against any entity
// (including a cross-claim or counterclaim in a lawsuit) alleging that
// the Work or a contribution incorporated within the Work constitutes
// direct or contributory patent infringement, then any patent licenses
// granted to You under this License for that Work shall terminate as of
// the date such litigation is filed.
//
// By using, modifying, or distributing the Work you indicate that you
// have read and understood the terms and conditions of the
// FreeType Project License as well as those provided in this section,
// and you accept them fully.
//
// This is an altered (translated) version of the original software; the
// FreeType Project License is in FTL.TXT (see also LICENSE.txt).

//! Adobe's CFF Interpreter (body).
//!
//! The interpreter's charstring is an index into its subroutine stack
//! (C points into it), and its hint objects are kept together
//! ([`Cf2Hints`]) for the glyph path. Type 1 mode (`font->isT1`) is only
//! set by the Type 1 and CID drivers, which are not translated yet; its
//! branches (the hint replay of the first pass, `closepath`, `hsbw`,
//! `seac`, `sbw`, `callothersubr`, `pop`, `setcurrentpoint`, `hstem3`,
//! `vstem3` and the large integers of `div`) are left out where C has
//! them, as noted.

use super::super::base::ftcalc::*;
use super::super::cff::cffload::{cff_blend_build_vector, cff_blend_check_vector};
use super::super::cfftypes::CffBlendRec;
use super::super::fttypes::*;
use super::psarrst::*;
use super::pserror::*;
use super::psfixed::*;
use super::psfont::*;
use super::psft::*;
use super::pshints::*;
use super::psobjs::{cff_random, PsDecoder};
use super::psread::*;
use super::psstack::*;

/// `cf2_hintmask_init` (its error is the font instance's)
pub fn cf2_hintmask_init(hintmask: &mut Cf2HintMaskRec) {
    *hintmask = Cf2HintMaskRec::default();
}

/// `cf2_hintmask_isValid`
pub fn cf2_hintmask_is_valid(hintmask: &Cf2HintMaskRec) -> bool {
    hintmask.isValid
}

/// `cf2_hintmask_isNew`
pub fn cf2_hintmask_is_new(hintmask: &Cf2HintMaskRec) -> bool {
    hintmask.isNew
}

/// `cf2_hintmask_setNew`
pub fn cf2_hintmask_set_new(hintmask: &mut Cf2HintMaskRec, val: bool) {
    hintmask.isNew = val;
}

/// `cf2_hintmask_getMaskPtr`: clients call `getMaskPtr' in order to
/// iterate through hint mask (the index of its first byte)
pub fn cf2_hintmask_get_mask_ptr(_hintmask: &Cf2HintMaskRec) -> usize {
    0
}

fn cf2_hintmask_set_counts(
    hintmask: &mut Cf2HintMaskRec,
    bit_count: usize,
    error: &mut FtError,
) -> usize {
    if bit_count > CF2_MAX_HINTS {
        /* total of h and v stems must be <= 96 */
        cf2_set_error_code(error, FT_ERR_INVALID_GLYPH_FORMAT);
        return 0;
    }

    hintmask.bitCount = bit_count;
    hintmask.byteCount = hintmask.bitCount.div_ceil(8);

    hintmask.isValid = true;
    hintmask.isNew = true;

    bit_count
}

/* consume the hintmask bytes from the charstring, advancing the src */
/* pointer                                                           */
fn cf2_hintmask_read(
    hintmask: &mut Cf2HintMaskRec,
    charstring: &mut Cf2BufferRec,
    bit_count: usize,
    error: &mut FtError,
) {
    /* (CF2_NDEBUG is defined) */

    /* initialize counts and isValid */
    if cf2_hintmask_set_counts(hintmask, bit_count, error) == 0 {
        return;
    }

    /* set mask and advance interpreter's charstring pointer */
    for i in 0..hintmask.byteCount {
        hintmask.mask[i] = cf2_buf_read_byte(charstring, error) as FtByte;
    }

    /* assert any unused bits in last byte are zero unless there's a prior */
    /* error                                                               */
    /* bitCount -> mask, 0 -> 0, 1 -> 7f, 2 -> 3f, ... 6 -> 3, 7 -> 1      */
}

/// `cf2_hintmask_setAll`
pub fn cf2_hintmask_set_all(hintmask: &mut Cf2HintMaskRec, bit_count: usize, error: &mut FtError) {
    let mask: Cf2UInt = (1u32 << ((bit_count as Cf2Int).wrapping_neg() & 7)) - 1;

    /* initialize counts and isValid */
    if cf2_hintmask_set_counts(hintmask, bit_count, error) == 0 {
        return;
    }

    /* set mask to all ones */
    for i in 0..hintmask.byteCount {
        hintmask.mask[i] = 0xFF;
    }

    /* clear unused bits                                              */
    /* bitCount -> mask, 0 -> 0, 1 -> 7f, 2 -> 3f, ... 6 -> 3, 7 -> 1 */
    hintmask.mask[hintmask.byteCount - 1] &= !(mask as FtByte);
}

/* Type2 charstring opcodes */
const CF2_CMD_RESERVED_0: FtByte = 0;
const CF2_CMD_HSTEM: FtByte = 1;
const CF2_CMD_RESERVED_2: FtByte = 2;
const CF2_CMD_VSTEM: FtByte = 3;
const CF2_CMD_VMOVETO: FtByte = 4;
const CF2_CMD_RLINETO: FtByte = 5;
const CF2_CMD_HLINETO: FtByte = 6;
const CF2_CMD_VLINETO: FtByte = 7;
const CF2_CMD_RRCURVETO: FtByte = 8;
const CF2_CMD_CLOSEPATH: FtByte = 9; /* T1 only */
const CF2_CMD_CALLSUBR: FtByte = 10;
const CF2_CMD_RETURN: FtByte = 11;
const CF2_CMD_ESC: FtByte = 12;
const CF2_CMD_HSBW: FtByte = 13; /* T1 only */
const CF2_CMD_ENDCHAR: FtByte = 14;
const CF2_CMD_VSINDEX: FtByte = 15;
const CF2_CMD_BLEND: FtByte = 16;
const CF2_CMD_RESERVED_17: FtByte = 17;
const CF2_CMD_HSTEMHM: FtByte = 18;
const CF2_CMD_HINTMASK: FtByte = 19;
const CF2_CMD_CNTRMASK: FtByte = 20;
const CF2_CMD_RMOVETO: FtByte = 21;
const CF2_CMD_HMOVETO: FtByte = 22;
const CF2_CMD_VSTEMHM: FtByte = 23;
const CF2_CMD_RCURVELINE: FtByte = 24;
const CF2_CMD_RLINECURVE: FtByte = 25;
const CF2_CMD_VVCURVETO: FtByte = 26;
const CF2_CMD_HHCURVETO: FtByte = 27;
const CF2_CMD_EXTENDEDNMBR: FtByte = 28;
const CF2_CMD_CALLGSUBR: FtByte = 29;
const CF2_CMD_VHCURVETO: FtByte = 30;
const CF2_CMD_HVCURVETO: FtByte = 31;

const CF2_ESC_DOTSECTION: FtByte = 0;
const CF2_ESC_VSTEM3: FtByte = 1; /* T1 only */
const CF2_ESC_HSTEM3: FtByte = 2; /* T1 only */
const CF2_ESC_AND: FtByte = 3;
const CF2_ESC_OR: FtByte = 4;
const CF2_ESC_NOT: FtByte = 5;
const CF2_ESC_SEAC: FtByte = 6; /* T1 only */
const CF2_ESC_SBW: FtByte = 7; /* T1 only */
const CF2_ESC_RESERVED_8: FtByte = 8;
const CF2_ESC_ABS: FtByte = 9;
const CF2_ESC_ADD: FtByte = 10; /* like otherADD */
const CF2_ESC_SUB: FtByte = 11; /* like otherSUB */
const CF2_ESC_DIV: FtByte = 12;
const CF2_ESC_RESERVED_13: FtByte = 13;
const CF2_ESC_NEG: FtByte = 14;
const CF2_ESC_EQ: FtByte = 15;
const CF2_ESC_CALLOTHERSUBR: FtByte = 16; /* T1 only */
const CF2_ESC_POP: FtByte = 17; /* T1 only */
const CF2_ESC_DROP: FtByte = 18;
const CF2_ESC_RESERVED_19: FtByte = 19;
const CF2_ESC_PUT: FtByte = 20; /* like otherPUT    */
const CF2_ESC_GET: FtByte = 21; /* like otherGET    */
const CF2_ESC_IFELSE: FtByte = 22; /* like otherIFELSE */
const CF2_ESC_RANDOM: FtByte = 23; /* like otherRANDOM */
const CF2_ESC_MUL: FtByte = 24; /* like otherMUL    */
const CF2_ESC_RESERVED_25: FtByte = 25;
const CF2_ESC_SQRT: FtByte = 26;
const CF2_ESC_DUP: FtByte = 27; /* like otherDUP    */
const CF2_ESC_EXCH: FtByte = 28; /* like otherEXCH   */
const CF2_ESC_INDEX: FtByte = 29;
const CF2_ESC_ROLL: FtByte = 30;
const CF2_ESC_RESERVED_31: FtByte = 31;
const CF2_ESC_RESERVED_32: FtByte = 32;
const CF2_ESC_SETCURRENTPT: FtByte = 33; /* T1 only */
const CF2_ESC_HFLEX: FtByte = 34;
const CF2_ESC_FLEX: FtByte = 35;
const CF2_ESC_HFLEX1: FtByte = 36;
const CF2_ESC_FLEX1: FtByte = 37;
const CF2_ESC_RESERVED_38: FtByte = 38; /* & all higher     */

/// The glyph path's context (`font` and `decoder`) for one call.
macro_rules! ctx {
    ($font:expr, $decoder:expr) => {
        &mut Cf2Ctx {
            font: &mut *$font,
            decoder: &mut *$decoder,
        }
    };
}

/* `stemHintArray' does not change once we start drawing the outline. */
#[allow(clippy::too_many_arguments)]
fn cf2_do_stems(
    font: &mut Cf2FontRec,
    decoder: &PsDecoder<'_, '_>,
    op_stack: &mut Cf2StackRec,
    stem_hint_array: &mut Cf2ArrStackRec<Cf2StemHintRec>,
    width: &mut Cf2Fixed,
    have_width: &mut bool,
    hint_offset: Cf2Fixed,
) {
    let count: Cf2UInt = cf2_stack_count(op_stack);
    let has_width_arg = count & 1 != 0;

    /* variable accumulates delta values from operand stack */
    let mut position: Cf2Fixed = hint_offset;

    /* (Type 1 mode: the missing width is only reported) */

    if !font.isT1 && has_width_arg && !*have_width {
        *width = add_int32(
            cf2_stack_get_real(op_stack, 0, &mut font.error),
            cf2_get_nominal_width_x(decoder),
        );
    }

    if !decoder.width_only {
        let mut i: Cf2UInt = if has_width_arg { 1 } else { 0 };
        while i < count {
            /* construct a CF2_StemHint and push it onto the list */
            position = add_int32(position, cf2_stack_get_real(op_stack, i, &mut font.error));
            let min = position;
            position = add_int32(
                position,
                cf2_stack_get_real(op_stack, i + 1, &mut font.error),
            );
            let max = position;

            let stemhint = Cf2StemHintRec {
                used: false,
                min,
                max,
                minDS: 0,
                maxDS: 0,
            };

            cf2_arrstack_push(stem_hint_array, &stemhint, &mut font.error); /* defer error check */
            i += 2;
        }

        cf2_stack_clear(op_stack);
    }

    /* exit: */
    /* cf2_doStems must define a width (may be default) */
    *have_width = true;
}

#[allow(clippy::too_many_arguments)]
fn cf2_do_flex(
    font: &mut Cf2FontRec,
    decoder: &mut PsDecoder<'_, '_>,
    op_stack: &mut Cf2StackRec,
    cur_x: &mut Cf2Fixed,
    cur_y: &mut Cf2Fixed,
    glyph_path: &mut Cf2GlyphPathRec,
    hints: &mut Cf2Hints,
    read_from_stack: &[bool; 12],
    do_conditional_last_read: bool,
) {
    let mut vals: [Cf2Fixed; 14] = [0; 14];
    let mut idx: Cf2UInt;

    vals[0] = *cur_x;
    vals[1] = *cur_y;
    idx = 0;
    let is_hflex = !read_from_stack[9];
    let top: usize = if is_hflex { 9 } else { 10 };

    for i in 0..top {
        vals[i + 2] = vals[i];
        if read_from_stack[i] {
            vals[i + 2] = add_int32(
                vals[i + 2],
                cf2_stack_get_real(op_stack, idx, &mut font.error),
            );
            idx += 1;
        }
    }

    if is_hflex {
        vals[9 + 2] = *cur_y;
    }

    if do_conditional_last_read {
        let last_is_x =
            cf2_fixed_abs(sub_int32(vals[10], *cur_x)) > cf2_fixed_abs(sub_int32(vals[11], *cur_y));
        let last_val: Cf2Fixed = cf2_stack_get_real(op_stack, idx, &mut font.error);

        if last_is_x {
            vals[12] = add_int32(vals[10], last_val);
            vals[13] = *cur_y;
        } else {
            vals[12] = *cur_x;
            vals[13] = add_int32(vals[11], last_val);
        }
    } else {
        if read_from_stack[10] {
            vals[12] = add_int32(vals[10], cf2_stack_get_real(op_stack, idx, &mut font.error));
            idx += 1;
        } else {
            vals[12] = *cur_x;
        }

        if read_from_stack[11] {
            vals[13] = add_int32(vals[11], cf2_stack_get_real(op_stack, idx, &mut font.error));
        } else {
            vals[13] = *cur_y;
        }
    }

    for j in 0..2 {
        cf2_glyphpath_curve_to(
            glyph_path,
            ctx!(font, decoder),
            hints,
            vals[j * 6 + 2],
            vals[j * 6 + 3],
            vals[j * 6 + 4],
            vals[j * 6 + 5],
            vals[j * 6 + 6],
            vals[j * 6 + 7],
        );
    }

    cf2_stack_clear(op_stack);

    *cur_x = vals[12];
    *cur_y = vals[13];
}

/* Blend numOperands on the stack,                */
/* store results into the first numBlends values, */
/* then pop remaining arguments.                  */
fn cf2_do_blend(
    blend: &CffBlendRec,
    op_stack: &mut Cf2StackRec,
    num_blends: Cf2UInt,
    error: &mut FtError,
) {
    let num_operands: Cf2UInt = num_blends.wrapping_mul(blend.lenBV);

    let base: Cf2UInt = cf2_stack_count(op_stack).wrapping_sub(num_operands);
    let mut delta: Cf2UInt = base.wrapping_add(num_blends);

    for i in 0..num_blends {
        /* start with first term */
        let mut sum: Cf2Fixed = cf2_stack_get_real(op_stack, i.wrapping_add(base), error);

        for j in 1..blend.lenBV as usize {
            let weight = blend.BV.get(j).copied().unwrap_or(0);
            sum = add_int32(
                sum,
                ft_mul_fix(
                    weight as FtLong,
                    cf2_stack_get_real(op_stack, delta, error) as FtLong,
                ) as Cf2Fixed,
            );
            delta = delta.wrapping_add(1);
        }

        /* store blended result  */
        cf2_stack_set_real(op_stack, i.wrapping_add(base), sum, error);
    }

    /* leave only `numBlends' results on stack */
    cf2_stack_pop(op_stack, num_operands.wrapping_sub(num_blends), error);
}

/*
 * `error' is a shared error code used by many objects in this
 * routine.  Before the code continues from an error, it must check and
 * record the error in `*error'.  The idea is that this shared
 * error code will record the first error encountered.  If testing
 * for an error anyway, the cost of `goto exit' is small, so we do it,
 * even if continuing would be safe.  In this case, `lastError' is
 * set, so the testing and storing can be done in one place, at `exit'.
 *
 * Continuing after an error is intended for objects which do their own
 * testing of `*error', e.g., array stack functions.  This allows us to
 * avoid an extra test after the call.
 *
 * Unimplemented opcodes are ignored.
 *
 */

/// `cf2_interpT2CharString` (the outline callbacks are the font's
/// client outline, drawing into the decoder's builder)
#[allow(clippy::too_many_arguments)]
pub fn cf2_interp_t2_char_string(
    font: &mut Cf2FontRec,
    decoder: &mut PsDecoder<'_, '_>,
    buf: &Cf2BufferRec,
    translation: &FtVector,
    doing_seac: bool,
    mut cur_x: Cf2Fixed,
    mut cur_y: Cf2Fixed,
    width: &mut Cf2Fixed,
) {
    /* lastError is used for errors that are immediately tested */
    let mut last_error: FtError = FT_ERR_OK;

    /* pointer to parsed font object */

    let scale_y: Cf2Fixed = font.innerTransform.d;
    let nominal_width_x: Cf2Fixed = cf2_get_nominal_width_x(decoder);

    /* save this for hinting seac accents */
    let hint_origin_y: Cf2Fixed = cur_y;

    let mut op_stack: Option<Cf2StackRec> = None;
    let stack_size: FtUInt;
    let mut op1: FtByte; /* first opcode byte */

    let mut storage: [Cf2F16Dot16; CF2_STORAGE_SIZE] = [0; CF2_STORAGE_SIZE]; /* for `put' and `get' */

    /* instruction limit; 20,000,000 matches Avalon */
    let mut instruction_limit: FtUInt32 = 20000000;

    let mut subr_stack: Cf2ArrStackRec<Cf2BufferRec> = Cf2ArrStackRec::default();

    let mut have_width: bool;
    let mut charstring: usize; /* (an index into `subrStack') */

    let mut charstring_index: Cf2Int = -1; /* initialize to empty */

    /* TODO: placeholders for hint structures */

    /* objects used for hinting */
    let mut hints = Cf2Hints::default();
    let mut glyph_path = Cf2GlyphPathRec::default();

    /* initialize the remaining objects */
    cf2_arrstack_init(&mut subr_stack);
    cf2_arrstack_init(&mut hints.hStemHintArray);
    cf2_arrstack_init(&mut hints.vStemHintArray);

    /* initialize CF2_StemHint arrays */
    cf2_hintmask_init(&mut hints.hintMask);

    /* initialize path map to manage drawing operations */

    /* Note: last 4 params are used to handle `MoveToPermissive', which */
    /*       may need to call `hintMap.Build'                           */
    /* TODO: MoveToPermissive is gone; are these still needed?          */
    cf2_glyphpath_init(&mut glyph_path, font, scale_y, hint_origin_y, translation);

    /*
     * Initialize state for width parsing.  From the CFF Spec:
     *
     *   The first stack-clearing operator, which must be one of hstem,
     *   hstemhm, vstem, vstemhm, cntrmask, hintmask, hmoveto, vmoveto,
     *   rmoveto, or endchar, takes an additional argument - the width (as
     *   described earlier), which may be expressed as zero or one numeric
     *   argument.
     *
     * What we implement here uses the first validly specified width, but
     * does not detect errors for specifying more than one width.
     *
     * If one of the above operators occurs without explicitly specifying
     * a width, we assume the default width.
     *
     * CFF2 charstrings always return the default width (0).
     *
     */
    have_width = font.isCFF2;
    *width = cf2_get_default_width_x(decoder);

    /*
     * Note: At this point, all pointers to resources must be NULL
     *       and all local objects must be initialized.
     *       There must be no branches to `exit:' above this point.
     *
     */

    'exit: {
        /* allocate an operand stack */
        stack_size = if font.isCFF2 {
            cf2_get_maxstack(decoder)
        } else {
            CF2_OPERAND_STACK_SIZE
        };
        op_stack = cf2_stack_init(stack_size);

        let Some(op_stack) = op_stack.as_mut() else {
            last_error = FT_ERR_OUT_OF_MEMORY;
            break 'exit;
        };

        /* initialize subroutine stack by placing top level charstring as */
        /* first element (max depth plus one for the charstring)          */
        /* Note: Caller owns and must finalize the first charstring.      */
        /*       Our copy of it does not change that requirement.         */
        cf2_arrstack_set_count(&mut subr_stack, CF2_MAX_SUBR + 1, &mut font.error);

        charstring = cf2_arrstack_get_buffer(&subr_stack);

        /* catch errors so far */
        if font.error != 0 {
            break 'exit;
        }

        subr_stack.items[charstring] = buf.clone(); /* structure copy     */
        charstring_index = 0; /* entry is valid now */

        /* main interpreter loop */
        'main: loop {
            /* (Type 1 mode is not translated yet) */

            if cf2_buf_is_end(&subr_stack.items[charstring]) {
                /* If we've reached the end of the charstring, simulate a */
                /* cf2_cmdRETURN or cf2_cmdENDCHAR.                       */
                /* We do this for both CFF and CFF2.                      */
                if charstring_index != 0 {
                    op1 = CF2_CMD_RETURN; /* end of buffer for subroutine */
                } else {
                    op1 = CF2_CMD_ENDCHAR; /* end of buffer for top level charstring */
                }
            } else {
                op1 =
                    cf2_buf_read_byte(&mut subr_stack.items[charstring], &mut font.error) as FtByte;

                /* Explicit RETURN and ENDCHAR in CFF2 should be ignored. */
                /* Note: Trace message will report 0 instead of 11 or 14. */
                if (op1 == CF2_CMD_RETURN || op1 == CF2_CMD_ENDCHAR) && font.isCFF2 {
                    op1 = CF2_CMD_RESERVED_0;
                }
            }

            /* (Type 1 mode: skipping outline commands in the first pass, */
            /* and the othersubr results, are not translated yet)          */

            /* check for errors once per loop */
            if font.error != 0 {
                break 'exit;
            }

            instruction_limit -= 1;
            if instruction_limit == 0 {
                last_error = FT_ERR_INVALID_GLYPH_FORMAT;
                break 'exit;
            }

            match op1 {
                CF2_CMD_RESERVED_0 | CF2_CMD_RESERVED_2 | CF2_CMD_RESERVED_17 => {
                    /* we may get here if we have a prior error */
                }

                CF2_CMD_VSINDEX => {
                    if font.isCFF2 {
                        if font.blend.usedBV {
                            /* vsindex not allowed after blend */
                            last_error = FT_ERR_INVALID_GLYPH_FORMAT;
                            break 'exit;
                        }

                        let temp: FtInt = cf2_stack_pop_int(op_stack, &mut font.error);

                        if temp >= 0 {
                            font.vsindex = temp as FtUInt;
                        }
                    }
                    /* (otherwise clear stack & ignore) */
                }

                CF2_CMD_BLEND => {
                    if font.isCFF2 {
                        /* do we have a `blend' op in a non-variant font? */
                        if !font.blend.font {
                            last_error = FT_ERR_INVALID_GLYPH_FORMAT;
                            break 'exit;
                        }

                        /* check cached blend vector */
                        if cff_blend_check_vector(
                            &font.blend,
                            font.vsindex,
                            font.lenNDV,
                            font.NDV.as_deref(),
                        ) {
                            if let Err(e) = cff_blend_build_vector(
                                &mut font.blend,
                                &decoder.cff.vstore,
                                font.vsindex,
                                font.lenNDV,
                                font.NDV.as_deref(),
                            ) {
                                last_error = e;
                                break 'exit;
                            }
                        }

                        /* do the blend */
                        let num_blends: FtUInt =
                            cf2_stack_pop_int(op_stack, &mut font.error) as FtUInt;
                        if num_blends > stack_size {
                            last_error = FT_ERR_INVALID_GLYPH_FORMAT;
                            break 'exit;
                        }

                        cf2_do_blend(&font.blend, op_stack, num_blends, &mut font.error);

                        font.blend.usedBV = true;
                        continue 'main; /* do not clear the stack */
                    }
                    /* (otherwise clear stack & ignore) */
                }

                CF2_CMD_HSTEMHM | CF2_CMD_HSTEM => {
                    let mut skip = false;
                    if !font.isT1 {
                        /* never add hints after the mask is computed */
                        /* except if in Type 1 mode (no hintmask op)  */
                        if cf2_hintmask_is_valid(&hints.hintMask) {
                            skip = true;
                        }
                    }

                    if !skip {
                        /* add left-sidebearing correction in Type 1 mode */
                        cf2_do_stems(
                            font,
                            decoder,
                            op_stack,
                            &mut hints.hStemHintArray,
                            width,
                            &mut have_width,
                            0, /* (Type 1: the left side bearing's y) */
                        );

                        if decoder.width_only {
                            break 'exit;
                        }
                    }
                }

                CF2_CMD_VSTEMHM | CF2_CMD_VSTEM => {
                    let mut skip = false;
                    if !font.isT1 {
                        /* never add hints after the mask is computed */
                        /* except if in Type 1 mode (no hintmask op)  */
                        if cf2_hintmask_is_valid(&hints.hintMask) {
                            skip = true;
                        }
                    }

                    if !skip {
                        /* add left-sidebearing correction in Type 1 mode */
                        cf2_do_stems(
                            font,
                            decoder,
                            op_stack,
                            &mut hints.vStemHintArray,
                            width,
                            &mut have_width,
                            0, /* (Type 1: the left side bearing's x) */
                        );

                        if decoder.width_only {
                            break 'exit;
                        }
                    }
                }

                CF2_CMD_VMOVETO => {
                    /* (Type 1 mode: the missing width is only reported) */

                    if cf2_stack_count(op_stack) > 1 && !have_width {
                        *width = add_int32(
                            cf2_stack_get_real(op_stack, 0, &mut font.error),
                            nominal_width_x,
                        );
                    }

                    /* width is defined or default after this */
                    have_width = true;

                    if decoder.width_only {
                        break 'exit;
                    }

                    cur_y = add_int32(cur_y, cf2_stack_pop_fixed(op_stack, &mut font.error));

                    if decoder.flex_state == 0 {
                        cf2_glyphpath_move_to(
                            &mut glyph_path,
                            ctx!(font, decoder),
                            &mut hints,
                            cur_x,
                            cur_y,
                        );
                    }
                }

                CF2_CMD_RLINETO => {
                    let count: Cf2UInt = cf2_stack_count(op_stack);

                    let mut idx: Cf2UInt = 0;
                    while idx < count {
                        cur_x =
                            add_int32(cur_x, cf2_stack_get_real(op_stack, idx, &mut font.error));
                        cur_y = add_int32(
                            cur_y,
                            cf2_stack_get_real(op_stack, idx + 1, &mut font.error),
                        );

                        cf2_glyphpath_line_to(
                            &mut glyph_path,
                            ctx!(font, decoder),
                            &mut hints,
                            cur_x,
                            cur_y,
                        );
                        idx += 2;
                    }

                    cf2_stack_clear(op_stack);
                    continue 'main; /* no need to clear stack again */
                }

                CF2_CMD_HLINETO | CF2_CMD_VLINETO => {
                    let count: Cf2UInt = cf2_stack_count(op_stack);
                    let mut is_x = op1 == CF2_CMD_HLINETO;

                    for idx in 0..count {
                        let v: Cf2Fixed = cf2_stack_get_real(op_stack, idx, &mut font.error);

                        if is_x {
                            cur_x = add_int32(cur_x, v);
                        } else {
                            cur_y = add_int32(cur_y, v);
                        }

                        is_x = !is_x;

                        cf2_glyphpath_line_to(
                            &mut glyph_path,
                            ctx!(font, decoder),
                            &mut hints,
                            cur_x,
                            cur_y,
                        );
                    }

                    cf2_stack_clear(op_stack);
                    continue 'main;
                }

                CF2_CMD_RCURVELINE | CF2_CMD_RRCURVETO => {
                    let count: Cf2UInt = cf2_stack_count(op_stack);
                    let mut idx: Cf2UInt = 0;

                    while idx + 6 <= count {
                        let x1 =
                            add_int32(cf2_stack_get_real(op_stack, idx, &mut font.error), cur_x);
                        let y1 = add_int32(
                            cf2_stack_get_real(op_stack, idx + 1, &mut font.error),
                            cur_y,
                        );
                        let x2 =
                            add_int32(cf2_stack_get_real(op_stack, idx + 2, &mut font.error), x1);
                        let y2 =
                            add_int32(cf2_stack_get_real(op_stack, idx + 3, &mut font.error), y1);
                        let x3 =
                            add_int32(cf2_stack_get_real(op_stack, idx + 4, &mut font.error), x2);
                        let y3 =
                            add_int32(cf2_stack_get_real(op_stack, idx + 5, &mut font.error), y2);

                        cf2_glyphpath_curve_to(
                            &mut glyph_path,
                            ctx!(font, decoder),
                            &mut hints,
                            x1,
                            y1,
                            x2,
                            y2,
                            x3,
                            y3,
                        );

                        cur_x = x3;
                        cur_y = y3;
                        idx += 6;
                    }

                    if op1 == CF2_CMD_RCURVELINE {
                        cur_x =
                            add_int32(cur_x, cf2_stack_get_real(op_stack, idx, &mut font.error));
                        cur_y = add_int32(
                            cur_y,
                            cf2_stack_get_real(op_stack, idx + 1, &mut font.error),
                        );

                        cf2_glyphpath_line_to(
                            &mut glyph_path,
                            ctx!(font, decoder),
                            &mut hints,
                            cur_x,
                            cur_y,
                        );
                    }

                    cf2_stack_clear(op_stack);
                    continue 'main; /* no need to clear stack again */
                }

                CF2_CMD_CLOSEPATH => { /* (Type 1 only, which is not translated yet) */ }

                CF2_CMD_CALLGSUBR | CF2_CMD_CALLSUBR => {
                    if !font.isT1 && charstring_index > CF2_MAX_SUBR as Cf2Int {
                        /* max subr plus one for charstring */
                        last_error = FT_ERR_INVALID_GLYPH_FORMAT;
                        break 'exit; /* overflow of stack */
                    }

                    /* push our current CFF charstring region on subrStack */
                    charstring = cf2_arrstack_get_pointer(
                        &subr_stack,
                        (charstring_index + 1) as usize,
                        &mut font.error,
                    );

                    /* set up the new CFF region and pointer */
                    let subr_num: Cf2Int = cf2_stack_pop_int(op_stack, &mut font.error);

                    /* (Type 1 mode: the subroutine hash is not translated yet) */

                    match op1 {
                        CF2_CMD_CALLGSUBR => {
                            if cf2_init_global_region_buffer(
                                decoder,
                                subr_num,
                                &mut subr_stack.items[charstring],
                            ) {
                                last_error = FT_ERR_INVALID_GLYPH_FORMAT;
                                break 'exit; /* subroutine lookup or stream error */
                            }
                        }

                        _ => {
                            /* cf2_cmdCALLSUBR */
                            if cf2_init_local_region_buffer(
                                decoder,
                                subr_num,
                                &mut subr_stack.items[charstring],
                            ) {
                                last_error = FT_ERR_INVALID_GLYPH_FORMAT;
                                break 'exit; /* subroutine lookup or stream error */
                            }
                        }
                    }

                    charstring_index += 1; /* entry is valid now */
                    continue 'main; /* do not clear the stack */
                }

                CF2_CMD_RETURN => {
                    if charstring_index < 1 {
                        /* Note: cannot return from top charstring */
                        last_error = FT_ERR_INVALID_GLYPH_FORMAT;
                        break 'exit; /* underflow of stack */
                    }

                    /* restore position in previous charstring */
                    charstring_index -= 1;
                    charstring = cf2_arrstack_get_pointer(
                        &subr_stack,
                        charstring_index as usize,
                        &mut font.error,
                    );
                    continue 'main; /* do not clear the stack */
                }

                CF2_CMD_ESC => {
                    let op2: FtByte =
                        cf2_buf_read_byte(&mut subr_stack.items[charstring], &mut font.error)
                            as FtByte;

                    /* first switch for 2-byte operators handles CFF2      */
                    /* and opcodes that are reserved for both CFF and CFF2 */
                    match op2 {
                        CF2_ESC_HFLEX => {
                            static READ_FROM_STACK: [bool; 12] = [
                                true,  /* dx1 */
                                false, /* dy1 */
                                true,  /* dx2 */
                                true,  /* dy2 */
                                true,  /* dx3 */
                                false, /* dy3 */
                                true,  /* dx4 */
                                false, /* dy4 */
                                true,  /* dx5 */
                                false, /* dy5 */
                                true,  /* dx6 */
                                false, /* dy6 */
                            ];

                            cf2_do_flex(
                                font,
                                decoder,
                                op_stack,
                                &mut cur_x,
                                &mut cur_y,
                                &mut glyph_path,
                                &mut hints,
                                &READ_FROM_STACK,
                                false, /* doConditionalLastRead */
                            );
                            continue 'main;
                        }

                        CF2_ESC_FLEX => {
                            static READ_FROM_STACK: [bool; 12] = [true; 12];

                            cf2_do_flex(
                                font,
                                decoder,
                                op_stack,
                                &mut cur_x,
                                &mut cur_y,
                                &mut glyph_path,
                                &mut hints,
                                &READ_FROM_STACK,
                                false, /* doConditionalLastRead */
                            );
                            /* TODO: why is this not a continue? */
                        }

                        CF2_ESC_HFLEX1 => {
                            static READ_FROM_STACK: [bool; 12] = [
                                true,  /* dx1 */
                                true,  /* dy1 */
                                true,  /* dx2 */
                                true,  /* dy2 */
                                true,  /* dx3 */
                                false, /* dy3 */
                                true,  /* dx4 */
                                false, /* dy4 */
                                true,  /* dx5 */
                                true,  /* dy5 */
                                true,  /* dx6 */
                                false, /* dy6 */
                            ];

                            cf2_do_flex(
                                font,
                                decoder,
                                op_stack,
                                &mut cur_x,
                                &mut cur_y,
                                &mut glyph_path,
                                &mut hints,
                                &READ_FROM_STACK,
                                false, /* doConditionalLastRead */
                            );
                            continue 'main;
                        }

                        CF2_ESC_FLEX1 => {
                            static READ_FROM_STACK: [bool; 12] = [
                                true,  /* dx1 */
                                true,  /* dy1 */
                                true,  /* dx2 */
                                true,  /* dy2 */
                                true,  /* dx3 */
                                true,  /* dy3 */
                                true,  /* dx4 */
                                true,  /* dy4 */
                                true,  /* dx5 */
                                true,  /* dy5 */
                                false, /* dx6 */
                                false, /* dy6 */
                            ];

                            cf2_do_flex(
                                font,
                                decoder,
                                op_stack,
                                &mut cur_x,
                                &mut cur_y,
                                &mut glyph_path,
                                &mut hints,
                                &READ_FROM_STACK,
                                true, /* doConditionalLastRead */
                            );
                            continue 'main;
                        }

                        /* these opcodes are always reserved */
                        CF2_ESC_RESERVED_8 | CF2_ESC_RESERVED_13 | CF2_ESC_RESERVED_19
                        | CF2_ESC_RESERVED_25 | CF2_ESC_RESERVED_31 | CF2_ESC_RESERVED_32 => {}

                        _ => {
                            if font.isCFF2 || op2 >= CF2_ESC_RESERVED_38 {
                                /* unknown op */
                            } else {
                                /* (Type 1 mode: dropping the othersubr results */
                                /* is not translated yet)                        */

                                /* second switch for 2-byte operators handles */
                                /* CFF and Type 1                             */
                                match op2 {
                                    CF2_ESC_DOTSECTION => {
                                        /* something about `flip type of locking' -- ignore it */
                                    }

                                    CF2_ESC_VSTEM3 | CF2_ESC_HSTEM3 => {
                                        /*
                                         * Type 1:                          Type 2:
                                         *   x0 dx0 x1 dx1 x2 dx2 vstem3      x dx {dxa dxb}* vstem
                                         *   y0 dy0 y1 dy1 y2 dy2 hstem3      y dy {dya dyb}* hstem
                                         *   relative to lsb point            relative to zero
                                         *
                                         */
                                        /* (unknown op unless in Type 1 mode, */
                                        /* which is not translated yet)       */
                                    }

                                    CF2_ESC_AND => {
                                        let arg2 = cf2_stack_pop_fixed(op_stack, &mut font.error);
                                        let arg1 = cf2_stack_pop_fixed(op_stack, &mut font.error);

                                        cf2_stack_push_int(
                                            op_stack,
                                            (arg1 != 0 && arg2 != 0) as Cf2Int,
                                            &mut font.error,
                                        );
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_OR => {
                                        let arg2 = cf2_stack_pop_fixed(op_stack, &mut font.error);
                                        let arg1 = cf2_stack_pop_fixed(op_stack, &mut font.error);

                                        cf2_stack_push_int(
                                            op_stack,
                                            (arg1 != 0 || arg2 != 0) as Cf2Int,
                                            &mut font.error,
                                        );
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_NOT => {
                                        let arg = cf2_stack_pop_fixed(op_stack, &mut font.error);

                                        cf2_stack_push_int(
                                            op_stack,
                                            (arg == 0) as Cf2Int,
                                            &mut font.error,
                                        );
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_SEAC => {
                                        /* (unknown op unless in Type 1 mode, */
                                        /* which is not translated yet)       */
                                    }

                                    CF2_ESC_SBW => {
                                        /* (unknown op unless in Type 1 mode, */
                                        /* which is not translated yet)       */
                                    }

                                    CF2_ESC_ABS => {
                                        let arg = cf2_stack_pop_fixed(op_stack, &mut font.error);

                                        if arg < -CF2_FIXED_MAX {
                                            cf2_stack_push_fixed(
                                                op_stack,
                                                CF2_FIXED_MAX,
                                                &mut font.error,
                                            );
                                        } else {
                                            cf2_stack_push_fixed(
                                                op_stack,
                                                arg.abs(),
                                                &mut font.error,
                                            );
                                        }
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_ADD => {
                                        let summand2 =
                                            cf2_stack_pop_fixed(op_stack, &mut font.error);
                                        let summand1 =
                                            cf2_stack_pop_fixed(op_stack, &mut font.error);

                                        cf2_stack_push_fixed(
                                            op_stack,
                                            add_int32(summand1, summand2),
                                            &mut font.error,
                                        );
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_SUB => {
                                        let subtrahend =
                                            cf2_stack_pop_fixed(op_stack, &mut font.error);
                                        let minuend =
                                            cf2_stack_pop_fixed(op_stack, &mut font.error);

                                        cf2_stack_push_fixed(
                                            op_stack,
                                            sub_int32(minuend, subtrahend),
                                            &mut font.error,
                                        );
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_DIV => {
                                        /* (Type 1 mode: large integers are not */
                                        /* translated yet)                      */
                                        let divisor =
                                            cf2_stack_pop_fixed(op_stack, &mut font.error);
                                        let dividend =
                                            cf2_stack_pop_fixed(op_stack, &mut font.error);

                                        cf2_stack_push_fixed(
                                            op_stack,
                                            ft_div_fix(dividend as FtLong, divisor as FtLong)
                                                as Cf2Fixed,
                                            &mut font.error,
                                        );
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_NEG => {
                                        let arg = cf2_stack_pop_fixed(op_stack, &mut font.error);

                                        if arg < -CF2_FIXED_MAX {
                                            cf2_stack_push_fixed(
                                                op_stack,
                                                CF2_FIXED_MAX,
                                                &mut font.error,
                                            );
                                        } else {
                                            cf2_stack_push_fixed(op_stack, -arg, &mut font.error);
                                        }
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_EQ => {
                                        let arg2 = cf2_stack_pop_fixed(op_stack, &mut font.error);
                                        let arg1 = cf2_stack_pop_fixed(op_stack, &mut font.error);

                                        cf2_stack_push_int(
                                            op_stack,
                                            (arg1 == arg2) as Cf2Int,
                                            &mut font.error,
                                        );
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_CALLOTHERSUBR => {
                                        /* (unknown op unless in Type 1 mode, */
                                        /* which is not translated yet)       */
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_POP => {
                                        /* (unknown op unless in Type 1 mode, */
                                        /* which is not translated yet)       */
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_DROP => {
                                        let _ = cf2_stack_pop_fixed(op_stack, &mut font.error);
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_PUT => {
                                        let idx: Cf2UInt =
                                            cf2_stack_pop_int(op_stack, &mut font.error) as Cf2UInt;
                                        let val = cf2_stack_pop_fixed(op_stack, &mut font.error);

                                        if (idx as usize) < CF2_STORAGE_SIZE {
                                            storage[idx as usize] = val;
                                        }
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_GET => {
                                        let idx: Cf2UInt =
                                            cf2_stack_pop_int(op_stack, &mut font.error) as Cf2UInt;

                                        if (idx as usize) < CF2_STORAGE_SIZE {
                                            cf2_stack_push_fixed(
                                                op_stack,
                                                storage[idx as usize],
                                                &mut font.error,
                                            );
                                        }
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_IFELSE => {
                                        let cond2 = cf2_stack_pop_fixed(op_stack, &mut font.error);
                                        let cond1 = cf2_stack_pop_fixed(op_stack, &mut font.error);
                                        let arg2 = cf2_stack_pop_fixed(op_stack, &mut font.error);
                                        let arg1 = cf2_stack_pop_fixed(op_stack, &mut font.error);

                                        cf2_stack_push_fixed(
                                            op_stack,
                                            if cond1 <= cond2 { arg1 } else { arg2 },
                                            &mut font.error,
                                        );
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_RANDOM => {
                                        /* in spec */
                                        let sub = decoder.current_subfont;
                                        let subfont = decoder.cff.subfont_mut(sub);

                                        /* only use the lower 16 bits of `random'  */
                                        /* to generate a number in the range (0;1] */
                                        let r: Cf2F16Dot16 =
                                            ((subfont.random & 0xFFFF) + 1) as Cf2F16Dot16;

                                        subfont.random = cff_random(subfont.random);

                                        cf2_stack_push_fixed(op_stack, r, &mut font.error);
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_MUL => {
                                        let factor2 =
                                            cf2_stack_pop_fixed(op_stack, &mut font.error);
                                        let factor1 =
                                            cf2_stack_pop_fixed(op_stack, &mut font.error);

                                        cf2_stack_push_fixed(
                                            op_stack,
                                            ft_mul_fix(factor1 as FtLong, factor2 as FtLong)
                                                as Cf2Fixed,
                                            &mut font.error,
                                        );
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_SQRT => {
                                        let mut arg: Cf2F16Dot16 =
                                            cf2_stack_pop_fixed(op_stack, &mut font.error);

                                        if arg > 0 {
                                            /* use a start value that doesn't make */
                                            /* the algorithm's addition overflow   */
                                            let mut root: FtFixed = if arg < 10 {
                                                arg as FtFixed
                                            } else {
                                                (arg >> 1) as FtFixed
                                            };
                                            let mut new_root: FtFixed;

                                            /* Babylonian method */
                                            loop {
                                                new_root =
                                                    (root + ft_div_fix(arg as FtLong, root) + 1)
                                                        >> 1;
                                                if new_root == root {
                                                    break;
                                                }
                                                root = new_root;
                                            }
                                            arg = new_root as Cf2F16Dot16;
                                        } else {
                                            arg = 0;
                                        }

                                        cf2_stack_push_fixed(op_stack, arg, &mut font.error);
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_DUP => {
                                        let arg = cf2_stack_pop_fixed(op_stack, &mut font.error);

                                        cf2_stack_push_fixed(op_stack, arg, &mut font.error);
                                        cf2_stack_push_fixed(op_stack, arg, &mut font.error);
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_EXCH => {
                                        let arg2 = cf2_stack_pop_fixed(op_stack, &mut font.error);
                                        let arg1 = cf2_stack_pop_fixed(op_stack, &mut font.error);

                                        cf2_stack_push_fixed(op_stack, arg2, &mut font.error);
                                        cf2_stack_push_fixed(op_stack, arg1, &mut font.error);
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_INDEX => {
                                        let idx: Cf2Int =
                                            cf2_stack_pop_int(op_stack, &mut font.error);
                                        let size: Cf2UInt = cf2_stack_count(op_stack);

                                        if size > 0 {
                                            /* for `cf2_stack_getReal',   */
                                            /* index 0 is bottom of stack */
                                            let gr_idx: Cf2UInt = if idx < 0 {
                                                size - 1
                                            } else if idx as Cf2UInt >= size {
                                                0
                                            } else {
                                                size - 1 - idx as Cf2UInt
                                            };

                                            let v = cf2_stack_get_real(
                                                op_stack,
                                                gr_idx,
                                                &mut font.error,
                                            );
                                            cf2_stack_push_fixed(op_stack, v, &mut font.error);
                                        }
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_ROLL => {
                                        let idx: Cf2Int =
                                            cf2_stack_pop_int(op_stack, &mut font.error);
                                        let count: Cf2Int =
                                            cf2_stack_pop_int(op_stack, &mut font.error);

                                        cf2_stack_roll(op_stack, count, idx, &mut font.error);
                                        continue 'main; /* do not clear the stack */
                                    }

                                    CF2_ESC_SETCURRENTPT => {
                                        /* (unknown op unless in Type 1 mode, */
                                        /* which is not translated yet)       */
                                    }

                                    _ => {}
                                } /* end of 2nd switch checking op2 */
                            }
                        }
                    } /* end of 1st switch checking op2 */
                } /* case cf2_cmdESC */

                CF2_CMD_HSBW => { /* (Type 1 only, which is not translated yet) */ }

                CF2_CMD_ENDCHAR => {
                    /* (Type 1 mode: the initial hint map rewind is not */
                    /* translated yet)                                   */

                    if cf2_stack_count(op_stack) == 1 || cf2_stack_count(op_stack) == 5 {
                        if !have_width {
                            *width = add_int32(
                                cf2_stack_get_real(op_stack, 0, &mut font.error),
                                nominal_width_x,
                            );
                        }
                    }

                    /* width is defined or default after this */
                    have_width = true;

                    if decoder.width_only {
                        break 'exit;
                    }

                    /* close path if still open */
                    cf2_glyphpath_close_open_path(&mut glyph_path, ctx!(font, decoder), &mut hints);

                    /* disable seac for CFF2 and Type1        */
                    /* (charstring ending with args on stack) */
                    if !font.isCFF2 && !font.isT1 && cf2_stack_count(op_stack) > 1 {
                        /* must be either 4 or 5 --                       */
                        /* this is a (deprecated) implied `seac' operator */

                        let mut component = Cf2BufferRec::default();
                        let mut dummy_width: Cf2Fixed = 0; /* ignore component width */

                        if doing_seac {
                            last_error = FT_ERR_INVALID_GLYPH_FORMAT;
                            break 'exit; /* nested seac */
                        }

                        let achar: Cf2Int = cf2_stack_pop_int(op_stack, &mut font.error);
                        let bchar: Cf2Int = cf2_stack_pop_int(op_stack, &mut font.error);

                        cur_y = cf2_stack_pop_fixed(op_stack, &mut font.error);
                        cur_x = cf2_stack_pop_fixed(op_stack, &mut font.error);

                        if let Err(e) = cf2_get_seac_component(decoder, achar, &mut component) {
                            last_error = e; /* pass FreeType error through */
                            break 'exit;
                        }
                        cf2_interp_t2_char_string(
                            font,
                            decoder,
                            &component,
                            translation,
                            true,
                            cur_x,
                            cur_y,
                            &mut dummy_width,
                        );
                        cf2_free_seac_component(decoder, &mut component);

                        if let Err(e) = cf2_get_seac_component(decoder, bchar, &mut component) {
                            last_error = e; /* pass FreeType error through */
                            break 'exit;
                        }
                        cf2_interp_t2_char_string(
                            font,
                            decoder,
                            &component,
                            translation,
                            true,
                            0,
                            0,
                            &mut dummy_width,
                        );
                        cf2_free_seac_component(decoder, &mut component);
                    }
                    break 'exit;
                }

                CF2_CMD_CNTRMASK | CF2_CMD_HINTMASK => {
                    /* the final \n in the tracing message gets added in      */
                    /* `cf2_hintmask_read' (which also traces the mask bytes) */

                    /* never add hints after the mask is computed */
                    if cf2_stack_count(op_stack) > 1 && cf2_hintmask_is_valid(&hints.hintMask) {
                        /* invalid hint mask */
                    } else {
                        /* if there are arguments on the stack, there this is an */
                        /* implied cf2_cmdVSTEMHM                                */
                        cf2_do_stems(
                            font,
                            decoder,
                            op_stack,
                            &mut hints.vStemHintArray,
                            width,
                            &mut have_width,
                            0,
                        );

                        if decoder.width_only {
                            break 'exit;
                        }

                        let bit_count = cf2_arrstack_size(&hints.hStemHintArray)
                            + cf2_arrstack_size(&hints.vStemHintArray);

                        if op1 == CF2_CMD_HINTMASK {
                            /* consume the hint mask bytes which follow the operator */
                            cf2_hintmask_read(
                                &mut hints.hintMask,
                                &mut subr_stack.items[charstring],
                                bit_count,
                                &mut font.error,
                            );
                        } else {
                            /*
                             * Consume the counter mask bytes which follow the operator:
                             * Build a temporary hint map, just to place and lock those
                             * stems participating in the counter mask.  These are most
                             * likely the dominant hstems, and are grouped together in a
                             * few counter groups, not necessarily in correspondence
                             * with the hint groups.  This reduces the chances of
                             * conflicts between hstems that are initially placed in
                             * separate hint groups and then brought together.  The
                             * positions are copied back to `hStemHintArray', so we can
                             * discard `counterMask' and `counterHintMap'.
                             *
                             */
                            let mut counter_hint_map = Cf2HintMapRec::default();
                            let mut counter_mask = Cf2HintMaskRec::default();

                            cf2_hintmap_init(&mut counter_hint_map, font, scale_y);
                            cf2_hintmask_init(&mut counter_mask);

                            cf2_hintmask_read(
                                &mut counter_mask,
                                &mut subr_stack.items[charstring],
                                bit_count,
                                &mut font.error,
                            );
                            cf2_hintmap_build(
                                &mut counter_hint_map,
                                Some(&mut glyph_path.initialHintMap),
                                &mut glyph_path.hintMoves,
                                font,
                                &mut hints.hStemHintArray,
                                &hints.vStemHintArray,
                                &mut counter_mask,
                                0,
                                false,
                            );
                        }
                    }
                }

                CF2_CMD_RMOVETO => {
                    /* (Type 1 mode: the missing width is only reported) */

                    if cf2_stack_count(op_stack) > 2 && !have_width {
                        *width = add_int32(
                            cf2_stack_get_real(op_stack, 0, &mut font.error),
                            nominal_width_x,
                        );
                    }

                    /* width is defined or default after this */
                    have_width = true;

                    if decoder.width_only {
                        break 'exit;
                    }

                    cur_y = add_int32(cur_y, cf2_stack_pop_fixed(op_stack, &mut font.error));
                    cur_x = add_int32(cur_x, cf2_stack_pop_fixed(op_stack, &mut font.error));

                    if decoder.flex_state == 0 {
                        cf2_glyphpath_move_to(
                            &mut glyph_path,
                            ctx!(font, decoder),
                            &mut hints,
                            cur_x,
                            cur_y,
                        );
                    }
                }

                CF2_CMD_HMOVETO => {
                    /* (Type 1 mode: the missing width is only reported) */

                    if cf2_stack_count(op_stack) > 1 && !have_width {
                        *width = add_int32(
                            cf2_stack_get_real(op_stack, 0, &mut font.error),
                            nominal_width_x,
                        );
                    }

                    /* width is defined or default after this */
                    have_width = true;

                    if decoder.width_only {
                        break 'exit;
                    }

                    cur_x = add_int32(cur_x, cf2_stack_pop_fixed(op_stack, &mut font.error));

                    if decoder.flex_state == 0 {
                        cf2_glyphpath_move_to(
                            &mut glyph_path,
                            ctx!(font, decoder),
                            &mut hints,
                            cur_x,
                            cur_y,
                        );
                    }
                }

                CF2_CMD_RLINECURVE => {
                    let count: Cf2UInt = cf2_stack_count(op_stack);
                    let mut idx: Cf2UInt = 0;

                    while idx + 6 < count {
                        cur_x =
                            add_int32(cur_x, cf2_stack_get_real(op_stack, idx, &mut font.error));
                        cur_y = add_int32(
                            cur_y,
                            cf2_stack_get_real(op_stack, idx + 1, &mut font.error),
                        );

                        cf2_glyphpath_line_to(
                            &mut glyph_path,
                            ctx!(font, decoder),
                            &mut hints,
                            cur_x,
                            cur_y,
                        );
                        idx += 2;
                    }

                    while idx < count {
                        let x1 =
                            add_int32(cf2_stack_get_real(op_stack, idx, &mut font.error), cur_x);
                        let y1 = add_int32(
                            cf2_stack_get_real(op_stack, idx + 1, &mut font.error),
                            cur_y,
                        );
                        let x2 =
                            add_int32(cf2_stack_get_real(op_stack, idx + 2, &mut font.error), x1);
                        let y2 =
                            add_int32(cf2_stack_get_real(op_stack, idx + 3, &mut font.error), y1);
                        let x3 =
                            add_int32(cf2_stack_get_real(op_stack, idx + 4, &mut font.error), x2);
                        let y3 =
                            add_int32(cf2_stack_get_real(op_stack, idx + 5, &mut font.error), y2);

                        cf2_glyphpath_curve_to(
                            &mut glyph_path,
                            ctx!(font, decoder),
                            &mut hints,
                            x1,
                            y1,
                            x2,
                            y2,
                            x3,
                            y3,
                        );

                        cur_x = x3;
                        cur_y = y3;
                        idx += 6;
                    }

                    cf2_stack_clear(op_stack);
                    continue 'main; /* no need to clear stack again */
                }

                CF2_CMD_VVCURVETO | CF2_CMD_HHCURVETO => {
                    let count1: Cf2UInt = cf2_stack_count(op_stack);
                    let mut idx: Cf2UInt = 0;
                    let is_vv = op1 == CF2_CMD_VVCURVETO;

                    /* if `cf2_stack_count' isn't of the form 4n or 4n+1, */
                    /* we enforce it by clearing the second bit           */
                    /* (and sorting the stack indexing to suit)           */
                    let count: Cf2UInt = count1 & !2u32;
                    idx += count1 - count;

                    while idx < count {
                        let (x1, y1, x2, y2, x3, y3);

                        if is_vv {
                            if (count - idx) & 1 != 0 {
                                x1 = add_int32(
                                    cf2_stack_get_real(op_stack, idx, &mut font.error),
                                    cur_x,
                                );

                                idx += 1;
                            } else {
                                x1 = cur_x;
                            }

                            y1 = add_int32(
                                cf2_stack_get_real(op_stack, idx, &mut font.error),
                                cur_y,
                            );
                            x2 = add_int32(
                                cf2_stack_get_real(op_stack, idx + 1, &mut font.error),
                                x1,
                            );
                            y2 = add_int32(
                                cf2_stack_get_real(op_stack, idx + 2, &mut font.error),
                                y1,
                            );
                            x3 = x2;
                            y3 = add_int32(
                                cf2_stack_get_real(op_stack, idx + 3, &mut font.error),
                                y2,
                            );
                        } else {
                            if (count - idx) & 1 != 0 {
                                y1 = add_int32(
                                    cf2_stack_get_real(op_stack, idx, &mut font.error),
                                    cur_y,
                                );

                                idx += 1;
                            } else {
                                y1 = cur_y;
                            }

                            x1 = add_int32(
                                cf2_stack_get_real(op_stack, idx, &mut font.error),
                                cur_x,
                            );
                            x2 = add_int32(
                                cf2_stack_get_real(op_stack, idx + 1, &mut font.error),
                                x1,
                            );
                            y2 = add_int32(
                                cf2_stack_get_real(op_stack, idx + 2, &mut font.error),
                                y1,
                            );
                            x3 = add_int32(
                                cf2_stack_get_real(op_stack, idx + 3, &mut font.error),
                                x2,
                            );
                            y3 = y2;
                        }

                        cf2_glyphpath_curve_to(
                            &mut glyph_path,
                            ctx!(font, decoder),
                            &mut hints,
                            x1,
                            y1,
                            x2,
                            y2,
                            x3,
                            y3,
                        );

                        cur_x = x3;
                        cur_y = y3;
                        idx += 4;
                    }

                    cf2_stack_clear(op_stack);
                    continue 'main; /* no need to clear stack again */
                }

                CF2_CMD_VHCURVETO | CF2_CMD_HVCURVETO => {
                    let count1: Cf2UInt = cf2_stack_count(op_stack);
                    let mut idx: Cf2UInt = 0;

                    let mut alternate = op1 == CF2_CMD_HVCURVETO;

                    /* if `cf2_stack_count' isn't of the form 8n, 8n+1, */
                    /* 8n+4, or 8n+5, we enforce it by clearing the     */
                    /* second bit                                       */
                    /* (and sorting the stack indexing to suit)         */
                    let count: Cf2UInt = count1 & !2u32;
                    idx += count1 - count;

                    while idx < count {
                        let (x1, x2, x3, y1, y2, y3);

                        if alternate {
                            x1 = add_int32(
                                cf2_stack_get_real(op_stack, idx, &mut font.error),
                                cur_x,
                            );
                            y1 = cur_y;
                            x2 = add_int32(
                                cf2_stack_get_real(op_stack, idx + 1, &mut font.error),
                                x1,
                            );
                            y2 = add_int32(
                                cf2_stack_get_real(op_stack, idx + 2, &mut font.error),
                                y1,
                            );
                            y3 = add_int32(
                                cf2_stack_get_real(op_stack, idx + 3, &mut font.error),
                                y2,
                            );

                            if count - idx == 5 {
                                x3 = add_int32(
                                    cf2_stack_get_real(op_stack, idx + 4, &mut font.error),
                                    x2,
                                );

                                idx += 1;
                            } else {
                                x3 = x2;
                            }

                            alternate = false;
                        } else {
                            x1 = cur_x;
                            y1 = add_int32(
                                cf2_stack_get_real(op_stack, idx, &mut font.error),
                                cur_y,
                            );
                            x2 = add_int32(
                                cf2_stack_get_real(op_stack, idx + 1, &mut font.error),
                                x1,
                            );
                            y2 = add_int32(
                                cf2_stack_get_real(op_stack, idx + 2, &mut font.error),
                                y1,
                            );
                            x3 = add_int32(
                                cf2_stack_get_real(op_stack, idx + 3, &mut font.error),
                                x2,
                            );

                            if count - idx == 5 {
                                y3 = add_int32(
                                    cf2_stack_get_real(op_stack, idx + 4, &mut font.error),
                                    y2,
                                );

                                idx += 1;
                            } else {
                                y3 = y2;
                            }

                            alternate = true;
                        }

                        cf2_glyphpath_curve_to(
                            &mut glyph_path,
                            ctx!(font, decoder),
                            &mut hints,
                            x1,
                            y1,
                            x2,
                            y2,
                            x3,
                            y3,
                        );

                        cur_x = x3;
                        cur_y = y3;
                        idx += 4;
                    }

                    cf2_stack_clear(op_stack);
                    continue 'main; /* no need to clear stack again */
                }

                CF2_CMD_EXTENDEDNMBR => {
                    let byte1: Cf2Int =
                        cf2_buf_read_byte(&mut subr_stack.items[charstring], &mut font.error);
                    let byte2: Cf2Int =
                        cf2_buf_read_byte(&mut subr_stack.items[charstring], &mut font.error);

                    let v: Cf2Int = ((byte1 << 8) | byte2) as FtShort as Cf2Int;

                    cf2_stack_push_int(op_stack, v, &mut font.error);
                    continue 'main;
                }

                _ => {
                    /* numbers */
                    if
                    /* op1 >= 32 && */
                    op1 <= 246 {
                        let v: Cf2Int = op1 as Cf2Int - 139;

                        /* -107 .. 107 */
                        cf2_stack_push_int(op_stack, v, &mut font.error);
                    } else if
                    /* op1 >= 247 && */
                    op1 <= 250 {
                        let mut v: Cf2Int = op1 as Cf2Int;
                        v -= 247;
                        v *= 256;
                        v += cf2_buf_read_byte(&mut subr_stack.items[charstring], &mut font.error);
                        v += 108;

                        /* 108 .. 1131 */
                        cf2_stack_push_int(op_stack, v, &mut font.error);
                    } else if
                    /* op1 >= 251 && */
                    op1 <= 254 {
                        let mut v: Cf2Int = op1 as Cf2Int;
                        v -= 251;
                        v *= 256;
                        v += cf2_buf_read_byte(&mut subr_stack.items[charstring], &mut font.error);
                        v = -v - 108;

                        /* -1131 .. -108 */
                        cf2_stack_push_int(op_stack, v, &mut font.error);
                    } else {
                        /* op1 == 255 */
                        let cs = &mut subr_stack.items[charstring];
                        let byte1 = cf2_buf_read_byte(cs, &mut font.error) as FtUInt32;
                        let byte2 = cf2_buf_read_byte(cs, &mut font.error) as FtUInt32;
                        let byte3 = cf2_buf_read_byte(cs, &mut font.error) as FtUInt32;
                        let byte4 = cf2_buf_read_byte(cs, &mut font.error) as FtUInt32;

                        let v: Cf2Fixed =
                            ((byte1 << 24) | (byte2 << 16) | (byte3 << 8) | byte4) as Cf2Fixed;

                        /*
                         * For Type 1:
                         *
                         * According to the specification, values > 32000 or < -32000
                         * must be followed by a `div' operator to make the result be
                         * in the range [-32000;32000].  We expect that the second
                         * argument of `div' is not a large number.  Additionally, we
                         * don't handle stuff like `<large1> <large2> <num> div <num>
                         * div' or <large1> <large2> <num> div div'.  This is probably
                         * not allowed anyway.
                         *
                         * <large> <num> <num>+ div is not checked but should not be
                         * allowed as the large value remains untouched.
                         *
                         */
                        /* (Type 1 mode is not translated yet) */
                        cf2_stack_push_fixed(op_stack, v, &mut font.error);
                    }
                    continue 'main; /* don't clear stack */
                }
            } /* end of switch statement checking `op1' */

            cf2_stack_clear(op_stack);
        } /* end of main interpreter loop */
    }

    /* exit: */
    /* check whether last error seen is also the first one */
    cf2_set_error(&mut font.error, last_error);

    /* free resources from objects we've used */
    cf2_glyphpath_finalize(&mut glyph_path);
    cf2_arrstack_finalize(&mut hints.vStemHintArray);
    cf2_arrstack_finalize(&mut hints.hStemHintArray);
    cf2_arrstack_finalize(&mut subr_stack);
    cf2_stack_free(op_stack);
}
