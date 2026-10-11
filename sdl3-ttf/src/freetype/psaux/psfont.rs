// Rust translation of src/psaux/psfont.c and src/psaux/psfont.h from
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

//! Adobe's code for font instances (body).
//!
//! The font instance is kept in the CFF font record across glyphs
//! (`cf2_instance`); the decoder it renders with (C's `font->decoder` and
//! `font->outline.decoder`) is given to the functions that use it.

use super::super::base::ftcalc::*;
use super::super::cff::cffload::{cff_blend_check_vector, cff_load_private_dict};
use super::super::cfftypes::*;
use super::super::fttypes::*;
use super::psblues::*;
use super::pserror::cf2_set_error;
use super::psfixed::*;
use super::psft::*;
use super::psglue::*;
use super::psintrp::cf2_interp_t2_char_string;
use super::psobjs::{PsDecoder, PsDecoderFont};
use super::psread::Cf2BufferRec;

pub const CF2_OPERAND_STACK_SIZE: FtUInt = 48;
pub const CF2_MAX_SUBR: usize = 16; /* maximum subroutine nesting;         */
/* only 10 are allowed but there exist */
/* fonts like `HiraKakuProN-W3.ttf'    */
/* (Hiragino Kaku Gothic ProN W3;      */
/* 8.2d6e1; 2014-12-19) that exceed    */
/* this limit                          */
pub const CF2_STORAGE_SIZE: usize = 32;

/// `CF2_FontRec` (typedef is in `cf2glue.h')
#[derive(Debug, Clone, Default)]
pub struct Cf2FontRec {
    pub error: FtError, /* shared error for this instance */

    pub isT1: bool,
    pub isCFF2: bool,
    pub renderingFlags: Cf2RenderingFlags,

    /* variables that depend on Transform:  */
    /* the following have zero translation; */
    /* inner * outer = font * original      */
    pub currentTransform: Cf2Matrix, /* original client matrix           */
    pub innerTransform: Cf2Matrix,   /* for hinting; erect, scaled       */
    pub outerTransform: Cf2Matrix,   /* post hinting; includes rotations */
    pub ppem: Cf2Fixed,              /* transform-dependent              */

    /* variation data */
    pub blend: CffBlendRec,        /* cached charstring blend vector  */
    pub vsindex: Cf2UInt,          /* current vsindex                 */
    pub lenNDV: Cf2UInt,           /* current length NDV or zero      */
    pub NDV: Option<Vec<FtFixed>>, /* ptr to current NDV or NULL      */

    pub unitsPerEm: Cf2Int,

    pub syntheticEmboldeningAmountX: Cf2Fixed, /* character space units */
    pub syntheticEmboldeningAmountY: Cf2Fixed, /* character space units */

    /* FreeType related members */
    pub outline: Cf2OutlineCallbacksRec, /* freetype glyph outline functions */
    /// `lastSubfont` (FreeType parsed data; top font or subfont)
    pub lastSubfont: Option<CffSubFontId>,

    /* these flags can vary from one call to the next */
    pub hinted: bool,
    pub darkened: bool, /* true if stemDarkened or synthetic bold */
    /* i.e. darkenX != 0 || darkenY != 0      */
    pub stemDarkened: bool,

    pub darkenParams: [FtInt; 8], /* 1000 unit character space */

    /* variables that depend on both FontDict and Transform */
    pub stdVW: Cf2Fixed,   /* in character space; depends on dict entry */
    pub stdHW: Cf2Fixed,   /* in character space; depends on dict entry */
    pub darkenX: Cf2Fixed, /* character space units    */
    pub darkenY: Cf2Fixed, /* depends on transform     */
    /* and private dict (StdVW) */
    pub reverseWinding: bool, /* darken assuming          */
    /* counterclockwise winding */
    pub blues: Cf2BluesRec, /* computed zone data */

    /// `cffload` (pointer to cff functions; set: the CFF driver's, which
    /// are called directly)
    pub cffload: bool,
}

/* Compute a stem darkening amount in character space. */
#[allow(clippy::too_many_arguments)]
fn cf2_compute_darkening(
    em_ratio: Cf2Fixed,
    ppem: Cf2Fixed,
    stem_width: Cf2Fixed,
    darken_amount: &mut Cf2Fixed,
    bolden_amount: Cf2Fixed,
    stem_darkened: bool,
    darken_params: &[FtInt; 8],
) {
    /*
     * Total darkening amount is computed in 1000 unit character space
     * using the modified 5 part curve as Adobe's Avalon rasterizer.
     * The darkening amount is smaller for thicker stems.
     * It becomes zero when the stem is thicker than 2.333 pixels.
     *
     * By default, we use
     *
     *   darkenAmount = 0.4 pixels   if scaledStem <= 0.5 pixels,
     *   darkenAmount = 0.275 pixels if 1 <= scaledStem <= 1.667 pixels,
     *   darkenAmount = 0 pixel      if scaledStem >= 2.333 pixels,
     *
     * and piecewise linear in-between:
     *
     *
     *   darkening
     *       ^
     *       |
     *       |      (x1,y1)
     *       |--------+
     *       |         \
     *       |          \
     *       |           \          (x3,y3)
     *       |            +----------+
     *       |        (x2,y2)         \
     *       |                         \
     *       |                          \
     *       |                           +-----------------
     *       |                         (x4,y4)
     *       +--------------------------------------------->   stem
     *                                                       thickness
     *
     *
     * This corresponds to the following values for the
     * `darkening-parameters' property:
     *
     *   (x1, y1) = (500, 400)
     *   (x2, y2) = (1000, 275)
     *   (x3, y3) = (1667, 275)
     *   (x4, y4) = (2333, 0)
     *
     */

    /* Internal calculations are done in units per thousand for */
    /* convenience. The x axis is scaled stem width in          */
    /* thousandths of a pixel. That is, 1000 is 1 pixel.        */
    /* The y axis is darkening amount in thousandths of a pixel.*/
    /* In the code, below, dividing by ppem and                 */
    /* adjusting for emRatio converts darkenAmount to character */
    /* space (font units).                                      */
    *darken_amount = 0;

    if bolden_amount == 0 && !stem_darkened {
        return;
    }

    /* protect against range problems and divide by zero */
    if em_ratio < cf2_double_to_fixed(0.01) {
        return;
    }

    let mul_fix = |a: i64, b: i64| ft_mul_fix(a, b);
    let div_fix = |a: i64, b: i64| ft_div_fix(a, b);

    if stem_darkened {
        let x1: FtInt = darken_params[0];
        let y1: FtInt = darken_params[1];
        let x2: FtInt = darken_params[2];
        let y2: FtInt = darken_params[3];
        let x3: FtInt = darken_params[4];
        let y3: FtInt = darken_params[5];
        let x4: FtInt = darken_params[6];
        let y4: FtInt = darken_params[7];

        /* convert from true character space to 1000 unit character space; */
        /* add synthetic emboldening effect                                */

        /* `stemWidthPer1000' will not overflow for a legitimate font      */

        let stem_width_per_1000: Cf2Fixed = mul_fix(
            stem_width.wrapping_add(bolden_amount) as i64,
            em_ratio as i64,
        ) as Cf2Fixed;

        /* `scaledStem' can easily overflow, so we must clamp its maximum  */
        /* value; the test doesn't need to be precise, but must be         */
        /* conservative.  The clamp value (default 2333) where             */
        /* `darkenAmount' is zero is well below the overflow value of      */
        /* 32767.                                                          */
        /*                                                                 */
        /* FT_MSB computes the integer part of the base 2 logarithm.  The  */
        /* number of bits for the product is 1 or 2 more than the sum of   */
        /* logarithms; remembering that the 16 lowest bits of the fraction */
        /* are dropped this is correct to within a factor of almost 4.     */
        /* For example, 0x80.0000 * 0x80.0000 = 0x4000.0000 is 23+23 and   */
        /* is flagged as possible overflow because 0xFF.FFFF * 0xFF.FFFF = */
        /* 0xFFFF.FE00 is also 23+23.                                      */

        let log_base2: FtInt = ft_msb(stem_width_per_1000 as FtUInt32) + ft_msb(ppem as FtUInt32);

        let scaled_stem: Cf2Fixed = if log_base2 >= 46 {
            /* possible overflow */
            cf2_int_to_fixed(x4)
        } else {
            mul_fix(stem_width_per_1000 as i64, ppem as i64) as Cf2Fixed
        };

        /* now apply the darkening parameters */

        /* (the C code jumps between the segments with `goto') */
        let mut segment = if scaled_stem < cf2_int_to_fixed(x1) {
            0
        } else if scaled_stem < cf2_int_to_fixed(x2) {
            1
        } else if scaled_stem < cf2_int_to_fixed(x3) {
            2
        } else if scaled_stem < cf2_int_to_fixed(x4) {
            3
        } else {
            4
        };

        loop {
            match segment {
                0 => {
                    *darken_amount = div_fix(cf2_int_to_fixed(y1) as i64, ppem as i64) as Cf2Fixed;
                }

                1 => {
                    let xdelta: FtInt = x2.wrapping_sub(x1);
                    let ydelta: FtInt = y2.wrapping_sub(y1);
                    let x: FtInt = stem_width_per_1000
                        .wrapping_sub(div_fix(cf2_int_to_fixed(x1) as i64, ppem as i64) as FtInt);

                    if xdelta == 0 {
                        /* goto Try_x3 */
                        segment = 2;
                        continue;
                    }

                    *darken_amount = (ft_mul_div(x as i64, ydelta as i64, xdelta as i64)
                        + div_fix(cf2_int_to_fixed(y1) as i64, ppem as i64))
                        as Cf2Fixed;
                }

                2 => {
                    /* Try_x3: */
                    let xdelta: FtInt = x3.wrapping_sub(x2);
                    let ydelta: FtInt = y3.wrapping_sub(y2);
                    let x: FtInt = stem_width_per_1000
                        .wrapping_sub(div_fix(cf2_int_to_fixed(x2) as i64, ppem as i64) as FtInt);

                    if xdelta == 0 {
                        /* goto Try_x4 */
                        segment = 3;
                        continue;
                    }

                    *darken_amount = (ft_mul_div(x as i64, ydelta as i64, xdelta as i64)
                        + div_fix(cf2_int_to_fixed(y2) as i64, ppem as i64))
                        as Cf2Fixed;
                }

                3 => {
                    /* Try_x4: */
                    let xdelta: FtInt = x4.wrapping_sub(x3);
                    let ydelta: FtInt = y4.wrapping_sub(y3);
                    let x: FtInt = stem_width_per_1000
                        .wrapping_sub(div_fix(cf2_int_to_fixed(x3) as i64, ppem as i64) as FtInt);

                    if xdelta == 0 {
                        /* goto Use_y4 */
                        segment = 4;
                        continue;
                    }

                    *darken_amount = (ft_mul_div(x as i64, ydelta as i64, xdelta as i64)
                        + div_fix(cf2_int_to_fixed(y3) as i64, ppem as i64))
                        as Cf2Fixed;
                }

                _ => {
                    /* Use_y4: */
                    *darken_amount = div_fix(cf2_int_to_fixed(y4) as i64, ppem as i64) as Cf2Fixed;
                }
            }
            break;
        }

        /* use half the amount on each side and convert back to true */
        /* character space                                           */
        *darken_amount =
            div_fix(*darken_amount as i64, em_ratio.wrapping_mul(2) as i64) as Cf2Fixed;
    }

    /* add synthetic emboldening effect in character space */
    *darken_amount = darken_amount.wrapping_add(bolden_amount / 2);
}

/* set up values for the current FontDict and matrix; */
/* called for each glyph to be rendered               */

/* caller's transform is adjusted for subpixel positioning */
fn cf2_font_setup(font: &mut Cf2FontRec, decoder: &mut PsDecoder<'_, '_>, transform: &Cf2Matrix) {
    /* pointer to parsed font object */
    let mut need_extra_setup = false;

    /* character space units */
    let mut bolden_x: Cf2Fixed = font.syntheticEmboldeningAmountX;
    let bolden_y: Cf2Fixed = font.syntheticEmboldeningAmountY;

    let mut ppem: Cf2Fixed;

    let mut len_normalized_v: Cf2UInt = 0;
    let mut normalized_v: Option<Vec<FtFixed>> = None;

    /* clear previous error */
    font.error = FT_ERR_OK;

    /* if a CID fontDict has changed, we need to recompute some cached */
    /* data                                                            */
    let sub_font = cf2_get_subfont(decoder);
    if font.lastSubfont != Some(sub_font) {
        font.lastSubfont = Some(sub_font);
        need_extra_setup = true;
    }

    if !font.isT1 {
        /* check for variation vectors */
        let vstore = cf2_get_vstore(decoder);
        let has_variations = vstore.dataCount != 0;

        if has_variations {
            /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
            /* check whether Private DICT in this subfont needs to be reparsed */
            match cf2_get_normalized_vector(decoder) {
                Ok((len, vec)) => {
                    len_normalized_v = len;
                    normalized_v = vec;
                }
                Err(e) => {
                    font.error = e;
                    return;
                }
            }

            let subfont = decoder.subfont();
            if cff_blend_check_vector(
                &subfont.blend,
                subfont.private_dict.vsindex,
                len_normalized_v,
                normalized_v.as_deref(),
            ) {
                /* blend has changed, reparse */
                if let PsDecoderFont::Cff { cff, stream, .. } = &mut decoder.font {
                    let _ = cff_load_private_dict(
                        cff,
                        sub_font,
                        stream,
                        len_normalized_v,
                        normalized_v.as_deref(),
                    );
                }
                need_extra_setup = true;
            }

            let subfont = decoder.subfont();

            /* copy from subfont */
            font.blend.font = subfont.blend.font;

            /* clear state of charstring blend */
            font.blend.usedBV = false;

            /* initialize value for charstring */
            font.vsindex = subfont.private_dict.vsindex;

            /* store vector inputs for blends in charstring */
            font.lenNDV = len_normalized_v;
            font.NDV = normalized_v;
        }
    }

    /* if ppem has changed, we need to recompute some cached data         */
    /* note: because of CID font matrix concatenation, ppem and transform */
    /*       do not necessarily track.                                    */
    ppem = cf2_get_ppem_y(decoder);
    if font.ppem != ppem {
        font.ppem = ppem;
        need_extra_setup = true;
    }

    /* copy hinted flag on each call */
    font.hinted = font.renderingFlags & CF2_FLAGS_HINTED != 0;

    /* determine if transform has changed;       */
    /* include Fontmatrix but ignore translation */
    if transform.a != font.currentTransform.a
        || transform.b != font.currentTransform.b
        || transform.c != font.currentTransform.c
        || transform.d != font.currentTransform.d
    {
        /* save `key' information for `cache of one' matrix data; */
        /* save client transform, without the translation         */
        font.currentTransform = *transform;
        font.currentTransform.tx = cf2_int_to_fixed(0);
        font.currentTransform.ty = cf2_int_to_fixed(0);

        /* TODO: FreeType transform is simple scalar; for now, use identity */
        /*       for outer                                                  */
        font.innerTransform = *transform;
        font.outerTransform.a = cf2_int_to_fixed(1);
        font.outerTransform.d = cf2_int_to_fixed(1);
        font.outerTransform.b = cf2_int_to_fixed(0);
        font.outerTransform.c = cf2_int_to_fixed(0);

        need_extra_setup = true;
    }

    /*
     * font->darkened is set to true if there is a stem darkening request or
     * the font is synthetic emboldened.
     * font->darkened controls whether to adjust blue zones, winding order,
     * and hinting.
     *
     */
    if font.stemDarkened as Cf2Int != (font.renderingFlags & CF2_FLAGS_DARKENED) {
        font.stemDarkened = font.renderingFlags & CF2_FLAGS_DARKENED != 0;

        /* blue zones depend on darkened flag */
        need_extra_setup = true;
    }

    /* recompute variables that are dependent on transform or FontDict or */
    /* darken flag                                                        */
    if need_extra_setup {
        /* StdVW is found in the private dictionary;                       */
        /* recompute darkening amounts whenever private dictionary or      */
        /* transform change                                                */
        /* Note: a rendering flag turns darkening on or off, so we want to */
        /*       store the `on' amounts;                                   */
        /*       darkening amount is computed in character space           */
        /* TODO: testing size-dependent darkening here;                    */
        /*       what to do for rotations?                                 */

        let mut units_per_em: Cf2Int = font.unitsPerEm;

        if units_per_em == 0 {
            units_per_em = 1000;
        }

        ppem = cf2_int_to_fixed(4).max(font.ppem); /* use minimum ppem of 4 */

        /* Freetype does not preserve the fontMatrix when parsing; use */
        /* unitsPerEm instead.                                         */
        /* TODO: check precision of this                               */
        let em_ratio: Cf2Fixed = cf2_int_to_fixed(1000).wrapping_div(units_per_em);
        font.stdVW = cf2_get_std_vw(decoder);

        if font.stdVW <= 0 {
            font.stdVW = ft_div_fix(cf2_int_to_fixed(75) as i64, em_ratio as i64) as Cf2Fixed;
        }

        let darken_params = font.darkenParams;
        if bolden_x > 0 {
            /* Ensure that boldenX is at least 1 pixel for synthetic bold font */
            /* (similar to what Avalon does)                                   */
            bolden_x = bolden_x
                .max(ft_div_fix(cf2_int_to_fixed(units_per_em) as i64, ppem as i64) as Cf2Fixed);

            /* Synthetic emboldening adds at least 1 pixel to darkenX, while */
            /* stem darkening adds at most half pixel.  Since the purpose of */
            /* stem darkening (readability at small sizes) is met with       */
            /* synthetic emboldening, no need to add stem darkening for a    */
            /* synthetic bold font.                                          */
            cf2_compute_darkening(
                em_ratio,
                ppem,
                font.stdVW,
                &mut font.darkenX,
                bolden_x,
                false,
                &darken_params,
            );
        } else {
            cf2_compute_darkening(
                em_ratio,
                ppem,
                font.stdVW,
                &mut font.darkenX,
                0,
                font.stemDarkened,
                &darken_params,
            );
        }

        /* set the default stem width, because it must be the same for all */
        /* family members;                                                 */
        /* choose a constant for StdHW that depends on font contrast       */
        let std_hw: Cf2Fixed = cf2_get_std_hw(decoder);

        if std_hw > 0 && font.stdVW > mul_int32(2, std_hw) {
            font.stdHW = ft_div_fix(cf2_int_to_fixed(75) as i64, em_ratio as i64) as Cf2Fixed;
        } else {
            /* low contrast font gets less hstem darkening */
            font.stdHW = ft_div_fix(cf2_int_to_fixed(110) as i64, em_ratio as i64) as Cf2Fixed;
        }

        cf2_compute_darkening(
            em_ratio,
            ppem,
            font.stdHW,
            &mut font.darkenY,
            bolden_y,
            font.stemDarkened,
            &darken_params,
        );

        font.darkened = font.darkenX != 0 || font.darkenY != 0;

        font.reverseWinding = false; /* initial expectation is CCW */

        /* compute blue zones for this instance */
        cf2_blues_init(font, decoder);
    } /* needExtraSetup */
}

/// `cf2_getGlyphOutline`: equivalent to AdobeGetOutline
pub fn cf2_get_glyph_outline(
    font: &mut Cf2FontRec,
    decoder: &mut PsDecoder<'_, '_>,
    charstring: &Cf2BufferRec,
    transform: &Cf2Matrix,
    glyph_width: &mut Cf2F16Dot16,
) -> FtResult<()> {
    let last_error: FtError = FT_ERR_OK;

    let mut adv_width: Cf2Fixed = 0;

    /* Note: use both integer and fraction for outlines.  This allows bbox */
    /*       to come out directly.                                         */

    let translation = FtVector {
        x: transform.tx as FtPos,
        y: transform.ty as FtPos,
    };

    'exit: {
        /* set up values based on transform */
        cf2_font_setup(font, decoder, transform);
        if font.error != 0 {
            break 'exit; /* setup encountered an error */
        }

        /* reset darken direction */
        font.reverseWinding = false;

        /* winding order only affects darkening */
        let mut need_winding = font.darkened;

        loop {
            /* reset output buffer */
            cf2_outline_reset(font, decoder);

            /* build the outline, passing the full translation */
            cf2_interp_t2_char_string(
                font,
                decoder,
                charstring,
                &translation,
                false,
                0,
                0,
                &mut adv_width,
            );

            if font.error != 0 {
                break 'exit;
            }

            if !need_winding {
                break;
            }

            /* check winding order */
            if font.outline.windingMomentum >= 0 {
                /* CFF is CCW */
                break;
            }

            /* invert darkening and render again                            */
            /* TODO: this should be a parameter to getOutline-computeOffset */
            font.reverseWinding = true;

            need_winding = false; /* exit after next iteration */
        }

        /* finish storing client outline */
        cf2_outline_close(decoder);
    }

    /* exit: */
    /* FreeType just wants the advance width; there is no translation */
    *glyph_width = adv_width;

    /* free resources and collect errors from objects we've used */
    cf2_set_error(&mut font.error, last_error);

    if font.error != 0 {
        Err(font.error)
    } else {
        Ok(())
    }
}
