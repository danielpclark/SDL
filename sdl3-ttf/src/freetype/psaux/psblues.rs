// Rust translation of src/psaux/psblues.c and src/psaux/psblues.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright 2009-2014 Adobe Systems Incorporated.
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

//! Adobe's code for handling Blue Zones (body).
//!
//! A `CF2_Blues' object stores the blue zones (horizontal alignment
//! zones) of a font.  These are specified in the CFF private dictionary
//! by `BlueValues', `OtherBlues', `FamilyBlues', and `FamilyOtherBlues'.
//! Each zone is defined by a top and bottom edge in character space.
//! Further, each zone is either a top zone or a bottom zone, as recorded
//! by `bottomZone'.
//!
//! The maximum number of `BlueValues' and `FamilyBlues' is 7 each.
//! However, these are combined to produce a total of 7 zones.
//! Similarly, the maximum number of `OtherBlues' and `FamilyOtherBlues'
//! is 5 and these are combined to produce an additional 5 zones.
//!
//! Blue zones are used to `capture' hints and force them to a common
//! alignment point.  This alignment is recorded in device space in
//! `dsFlatEdge'.  Except for this value, a `CF2_Blues' object could be
//! constructed independently of scaling.  Construction may occur once
//! the matrix is known.  Other features implemented in the Capture
//! method are overshoot suppression, overshoot enforcement, and Blue
//! Boost.
//!
//! Capture is determined by `BlueValues' and `OtherBlues', but the
//! alignment point may be adjusted to the scaled flat edge of
//! `FamilyBlues' or `FamilyOtherBlues'.  No alignment is done to the
//! curved edge of a zone.

use super::super::base::ftcalc::*;
use super::super::fttypes::*;
use super::psfixed::*;
use super::psfont::Cf2FontRec;
use super::psft::*;
use super::pshints::{cf2_hint_is_bottom, cf2_hint_is_top, cf2_hint_is_valid, cf2_hint_lock};
use super::psobjs::PsDecoder;

/*
 * `CF2_Hint' is shared by `cf2hints.h' and
 * `cf2blues.h', but `cf2blues.h' depends on
 * `cf2hints.h', so define it here.  Note: The typedef is in
 * `cf2glue.h'.
 *
 */
pub const CF2_GHOST_BOTTOM: Cf2UInt = 0x1; /* a single bottom edge           */
pub const CF2_GHOST_TOP: Cf2UInt = 0x2; /* a single top edge              */
pub const CF2_PAIR_BOTTOM: Cf2UInt = 0x4; /* the bottom edge of a stem hint */
pub const CF2_PAIR_TOP: Cf2UInt = 0x8; /* the top edge of a stem hint    */
pub const CF2_LOCKED: Cf2UInt = 0x10; /* this edge has been aligned     */
/* by a blue zone                 */
pub const CF2_SYNTHETIC: Cf2UInt = 0x20; /* this edge was synthesized      */

/*
 * Default value for OS/2 typoAscender/Descender when their difference
 * is not equal to `unitsPerEm'.  The default is based on -250 and 1100
 * in `CF2_Blues', assuming 1000 units per em here.
 *
 */
pub const CF2_ICF_TOP: Cf2Fixed = cf2_int_to_fixed(880);
pub const CF2_ICF_BOTTOM: Cf2Fixed = cf2_int_to_fixed(-120);

/*
 * Constant used for hint adjustment and for synthetic em box hint
 * placement.
 */
/// `CF2_MIN_COUNTER` (`cf2_doubleToFixed( 0.5 )`)
pub const CF2_MIN_COUNTER: Cf2Fixed = 0x8000;

/// `CF2_HintRec` (shared typedef is in cf2glue.h)
#[derive(Debug, Clone, Copy, Default)]
pub struct Cf2HintRec {
    pub flags: Cf2UInt, /* attributes of the edge            */
    pub index: usize,   /* index in original stem hint array */
    /* (if not synthetic)                */
    pub csCoord: Cf2Fixed,
    pub dsCoord: Cf2Fixed,
    pub scale: Cf2Fixed,
}

/// `CF2_BlueRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct Cf2BlueRec {
    pub csBottomEdge: Cf2Fixed,
    pub csTopEdge: Cf2Fixed,
    pub csFlatEdge: Cf2Fixed, /* may be from either local or Family zones */
    pub dsFlatEdge: Cf2Fixed, /* top edge of bottom zone or bottom edge   */
    /* of top zone (rounded)                    */
    pub bottomZone: bool,
}

/* max total blue zones is 12 */
pub const CF2_MAX_BLUES: usize = 7;
pub const CF2_MAX_OTHERBLUES: usize = 5;

/// `CF2_BluesRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct Cf2BluesRec {
    pub scale: Cf2Fixed,
    pub count: Cf2UInt,
    pub suppressOvershoot: bool,
    pub doEmBoxHints: bool,

    pub blueScale: Cf2Fixed,
    pub blueShift: Cf2Fixed,
    pub blueFuzz: Cf2Fixed,

    pub boost: Cf2Fixed,

    pub emBoxTopEdge: Cf2HintRec,
    pub emBoxBottomEdge: Cf2HintRec,

    pub zone: [Cf2BlueRec; CF2_MAX_BLUES + CF2_MAX_OTHERBLUES],
}

/*
 * For blue values, the FreeType parser produces an array of integers,
 * while the Adobe CFF engine produces an array of fixed.
 * Define a macro to convert FreeType to fixed.
 */
fn cf2_blue_to_fixed(x: FtPos) -> Cf2Fixed {
    cf2_int_to_fixed(x as i32)
}

/// `cf2_blues_init` (the font's `blues`)
pub fn cf2_blues_init(font: &mut Cf2FontRec, decoder: &PsDecoder<'_, '_>) {
    /* pointer to parsed font object */
    let mut blues = Cf2BluesRec::default();

    let mut zone_height: Cf2Fixed;
    let mut max_zone_height: Cf2Fixed = 0;
    let cs_units_per_pixel: Cf2Fixed;

    let em_box_bottom: Cf2Fixed;
    let em_box_top: Cf2Fixed;

    blues.scale = font.innerTransform.d;

    cf2_get_blue_metrics(
        decoder,
        &mut blues.blueScale,
        &mut blues.blueShift,
        &mut blues.blueFuzz,
    );

    let (num_blue_values, blue_values) = cf2_get_blue_values(decoder);
    let (num_other_blues, other_blues) = cf2_get_other_blues(decoder);
    let (num_family_blues, family_blues) = cf2_get_family_blues(decoder);
    let (num_family_other_blues, family_other_blues) = cf2_get_family_other_blues(decoder);

    /* the arrays have fixed sizes; C reads past an odd count's last */
    /* value into the array's next element                           */
    let bv = |i: usize| blue_values.get(i).copied().unwrap_or(0);
    let ob = |i: usize| other_blues.get(i).copied().unwrap_or(0);
    let fb = |i: usize| family_blues.get(i).copied().unwrap_or(0);
    let fob = |i: usize| family_other_blues.get(i).copied().unwrap_or(0);

    /*
     * synthetic em box hint heuristic
     *
     * Apply this when ideographic dictionary (LanguageGroup 1) has no
     * real alignment zones.  Adobe tools generate dummy zones at -250 and
     * 1100 for a 1000 unit em.  Fonts with ICF-based alignment zones
     * should not enable the heuristic.  When the heuristic is enabled,
     * the font's blue zones are ignored.
     *
     */

    /* get em box from OS/2 typoAscender/Descender                      */
    /* TODO: FreeType does not parse these metrics.  Skip them for now. */
    {
        em_box_bottom = CF2_ICF_BOTTOM;
        em_box_top = CF2_ICF_TOP;
    }

    if cf2_get_language_group(decoder) == 1
        && (num_blue_values == 0
            || (num_blue_values == 4
                && cf2_blue_to_fixed(bv(0)) < em_box_bottom
                && cf2_blue_to_fixed(bv(1)) < em_box_bottom
                && cf2_blue_to_fixed(bv(2)) > em_box_top
                && cf2_blue_to_fixed(bv(3)) > em_box_top))
    {
        /*
         * Construct hint edges suitable for synthetic ghost hints at top
         * and bottom of em box.  +-CF2_MIN_COUNTER allows for unhinted
         * features above or below the last hinted edge.  This also gives a
         * net 1 pixel boost to the height of ideographic glyphs.
         *
         * Note: Adjust synthetic hints outward by epsilon (0x.0001) to
         *       avoid interference.  E.g., some fonts have real hints at
         *       880 and -120.
         */

        blues.emBoxBottomEdge.csCoord = em_box_bottom - CF2_FIXED_EPSILON;
        blues.emBoxBottomEdge.dsCoord = cf2_fixed_round(ft_mul_fix(
            blues.emBoxBottomEdge.csCoord as FtLong,
            blues.scale as FtLong,
        ) as Cf2Fixed)
        .wrapping_sub(CF2_MIN_COUNTER);
        blues.emBoxBottomEdge.scale = blues.scale;
        blues.emBoxBottomEdge.flags = CF2_GHOST_BOTTOM | CF2_LOCKED | CF2_SYNTHETIC;

        blues.emBoxTopEdge.csCoord = em_box_top
            .wrapping_add(CF2_FIXED_EPSILON)
            .wrapping_add(font.darkenY.wrapping_mul(2));
        blues.emBoxTopEdge.dsCoord = cf2_fixed_round(ft_mul_fix(
            blues.emBoxTopEdge.csCoord as FtLong,
            blues.scale as FtLong,
        ) as Cf2Fixed)
        .wrapping_add(CF2_MIN_COUNTER);
        blues.emBoxTopEdge.scale = blues.scale;
        blues.emBoxTopEdge.flags = CF2_GHOST_TOP | CF2_LOCKED | CF2_SYNTHETIC;

        blues.doEmBoxHints = true; /* enable the heuristic */

        font.blues = blues;
        return;
    }

    /* copy `BlueValues' and `OtherBlues' to a combined array of top and */
    /* bottom zones                                                      */
    let mut i = 0;
    while i < num_blue_values {
        let c = blues.count as usize;
        blues.zone[c].csBottomEdge = cf2_blue_to_fixed(bv(i));
        blues.zone[c].csTopEdge = cf2_blue_to_fixed(bv(i + 1));

        zone_height = sub_int32(blues.zone[c].csTopEdge, blues.zone[c].csBottomEdge);

        if zone_height < 0 {
            i += 2;
            continue; /* reject this zone */
        }

        if zone_height > max_zone_height {
            /* take maximum before darkening adjustment      */
            /* so overshoot suppression point doesn't change */
            max_zone_height = zone_height;
        }

        /* adjust both edges of top zone upward by twice darkening amount */
        if i != 0 {
            blues.zone[c].csTopEdge = blues.zone[c]
                .csTopEdge
                .wrapping_add(font.darkenY.wrapping_mul(2));
            blues.zone[c].csBottomEdge = blues.zone[c]
                .csBottomEdge
                .wrapping_add(font.darkenY.wrapping_mul(2));
        }

        /* first `BlueValue' is bottom zone; others are top */
        if i == 0 {
            blues.zone[c].bottomZone = true;
            blues.zone[c].csFlatEdge = blues.zone[c].csTopEdge;
        } else {
            blues.zone[c].bottomZone = false;
            blues.zone[c].csFlatEdge = blues.zone[c].csBottomEdge;
        }

        blues.count += 1;
        i += 2;
    }

    let mut i = 0;
    while i < num_other_blues {
        let c = blues.count as usize;
        blues.zone[c].csBottomEdge = cf2_blue_to_fixed(ob(i));
        blues.zone[c].csTopEdge = cf2_blue_to_fixed(ob(i + 1));

        zone_height = sub_int32(blues.zone[c].csTopEdge, blues.zone[c].csBottomEdge);

        if zone_height < 0 {
            i += 2;
            continue; /* reject this zone */
        }

        if zone_height > max_zone_height {
            /* take maximum before darkening adjustment      */
            /* so overshoot suppression point doesn't change */
            max_zone_height = zone_height;
        }

        /* Note: bottom zones are not adjusted for darkening amount */

        /* all OtherBlues are bottom zone */
        blues.zone[c].bottomZone = true;
        blues.zone[c].csFlatEdge = blues.zone[c].csTopEdge;

        blues.count += 1;
        i += 2;
    }

    /* Adjust for FamilyBlues */

    /* Search for the nearest flat edge in `FamilyBlues' or                */
    /* `FamilyOtherBlues'.  According to the Black Book, any matching edge */
    /* must be within one device pixel                                     */

    cs_units_per_pixel =
        ft_div_fix(cf2_int_to_fixed(1) as FtLong, blues.scale as FtLong) as Cf2Fixed;

    /* loop on all zones in this font */
    for i in 0..blues.count as usize {
        let mut min_diff: Cf2Fixed;
        let mut flat_family_edge: Cf2Fixed;
        let mut diff: Cf2Fixed;
        /* value for this font */
        let flat_edge: Cf2Fixed = blues.zone[i].csFlatEdge;

        if blues.zone[i].bottomZone {
            /* In a bottom zone, the top edge is the flat edge.             */
            /* Search `FamilyOtherBlues' for bottom zones; look for closest */
            /* Family edge that is within the one pixel threshold.          */

            min_diff = CF2_FIXED_MAX;

            let mut j = 0;
            while j < num_family_other_blues {
                /* top edge */
                flat_family_edge = cf2_blue_to_fixed(fob(j + 1));

                diff = cf2_fixed_abs(sub_int32(flat_edge, flat_family_edge));

                if diff < min_diff && diff < cs_units_per_pixel {
                    blues.zone[i].csFlatEdge = flat_family_edge;
                    min_diff = diff;

                    if diff == 0 {
                        break;
                    }
                }
                j += 2;
            }

            /* check the first member of FamilyBlues, which is a bottom zone */
            if num_family_blues >= 2 {
                /* top edge */
                flat_family_edge = cf2_blue_to_fixed(fb(1));

                diff = cf2_fixed_abs(sub_int32(flat_edge, flat_family_edge));

                if diff < min_diff && diff < cs_units_per_pixel {
                    blues.zone[i].csFlatEdge = flat_family_edge;
                }
            }
        } else {
            /* In a top zone, the bottom edge is the flat edge.                */
            /* Search `FamilyBlues' for top zones; skip first zone, which is a */
            /* bottom zone; look for closest Family edge that is within the    */
            /* one pixel threshold                                             */

            min_diff = CF2_FIXED_MAX;

            let mut j = 2;
            while j < num_family_blues {
                /* bottom edge */
                flat_family_edge = cf2_blue_to_fixed(fb(j));

                /* adjust edges of top zone upward by twice darkening amount */
                flat_family_edge = flat_family_edge.wrapping_add(font.darkenY.wrapping_mul(2)); /* bottom edge */

                diff = cf2_fixed_abs(sub_int32(flat_edge, flat_family_edge));

                if diff < min_diff && diff < cs_units_per_pixel {
                    blues.zone[i].csFlatEdge = flat_family_edge;
                    min_diff = diff;

                    if diff == 0 {
                        break;
                    }
                }
                j += 2;
            }
        }
    }

    /* TODO: enforce separation of zones, including BlueFuzz */

    /* Adjust BlueScale; similar to AdjustBlueScale() in coretype */
    /* `bcsetup.c'.                                               */

    if max_zone_height > 0 {
        if blues.blueScale as FtLong
            > ft_div_fix(cf2_int_to_fixed(1) as FtLong, max_zone_height as FtLong)
        {
            /* clamp at maximum scale */
            blues.blueScale =
                ft_div_fix(cf2_int_to_fixed(1) as FtLong, max_zone_height as FtLong) as Cf2Fixed;
        }

        /*
         * TODO: Revisit the bug fix for 613448.  The minimum scale
         *       requirement catches a number of library fonts.  For
         *       example, with default BlueScale (.039625) and 0.4 minimum,
         *       the test below catches any font with maxZoneHeight < 10.1.
         *       There are library fonts ranging from 2 to 10 that get
         *       caught, including e.g., Eurostile LT Std Medium with
         *       maxZoneHeight of 6.
         *
         */
    }

    /*
     * Suppress overshoot and boost blue zones at small sizes.  Boost
     * amount varies linearly from 0.5 pixel near 0 to 0 pixel at
     * blueScale cutoff.
     * Note: This boost amount is different from the coretype heuristic.
     *
     */

    if blues.scale < blues.blueScale {
        blues.suppressOvershoot = true;

        /* Change rounding threshold for `dsFlatEdge'.                    */
        /* Note: constant changed from 0.5 to 0.6 to avoid a problem with */
        /*       10ppem Arial                                             */

        blues.boost = (cf2_double_to_fixed(0.6) as FtLong
            - ft_mul_div(
                cf2_double_to_fixed(0.6) as FtLong,
                blues.scale as FtLong,
                blues.blueScale as FtLong,
            )) as Cf2Fixed;
        if blues.boost > 0x7FFF {
            /* boost must remain less than 0.5, or baseline could go negative */
            blues.boost = 0x7FFF;
        }
    }

    /* boost and darkening have similar effects; don't do both */
    if font.stemDarkened {
        blues.boost = 0;
    }

    /* set device space alignment for each zone;    */
    /* apply boost amount before rounding flat edge */

    for i in 0..blues.count as usize {
        let m = ft_mul_fix(blues.zone[i].csFlatEdge as FtLong, blues.scale as FtLong);
        if blues.zone[i].bottomZone {
            blues.zone[i].dsFlatEdge = cf2_fixed_round((m - blues.boost as FtLong) as Cf2Fixed);
        } else {
            blues.zone[i].dsFlatEdge = cf2_fixed_round((m + blues.boost as FtLong) as Cf2Fixed);
        }
    }

    font.blues = blues;
}

/*
 * Check whether `stemHint' is captured by one of the blue zones.
 *
 * Zero, one or both edges may be valid; only valid edges can be
 * captured.  For compatibility with CoolType, search top and bottom
 * zones in the same pass (see `BlueLock').  If a hint is captured,
 * return true and position the edge(s) in one of 3 ways:
 *
 * 1) If `BlueScale' suppresses overshoot, position the captured edge
 *    at the flat edge of the zone.
 * 2) If overshoot is not suppressed and `BlueShift' requires
 *    overshoot, position the captured edge a minimum of 1 device pixel
 *    from the flat edge.
 * 3) If overshoot is not suppressed or required, position the captured
 *    edge at the nearest device pixel.
 *
 */

/// `cf2_blues_capture`
pub fn cf2_blues_capture(
    blues: &Cf2BluesRec,
    bottom_hint_edge: &mut Cf2HintRec,
    top_hint_edge: &mut Cf2HintRec,
) -> bool {
    /* TODO: validate? */
    let cs_fuzz: Cf2Fixed = blues.blueFuzz;

    /* new position of captured edge */
    let ds_new: Cf2Fixed;

    /* amount that hint is moved when positioned */
    let mut ds_move: Cf2Fixed = 0;

    let mut captured = false;

    /* assert edge flags are consistent */

    /* TODO: search once without blue fuzz for compatibility with coretype? */
    for i in 0..blues.count as usize {
        if blues.zone[i].bottomZone && cf2_hint_is_bottom(bottom_hint_edge) {
            if sub_int32(blues.zone[i].csBottomEdge, cs_fuzz) <= bottom_hint_edge.csCoord
                && bottom_hint_edge.csCoord <= add_int32(blues.zone[i].csTopEdge, cs_fuzz)
            {
                /* bottom edge captured by bottom zone */

                if blues.suppressOvershoot {
                    ds_new = blues.zone[i].dsFlatEdge;
                } else if sub_int32(blues.zone[i].csTopEdge, bottom_hint_edge.csCoord)
                    >= blues.blueShift
                {
                    /* guarantee minimum of 1 pixel overshoot */
                    ds_new = cf2_fixed_round(bottom_hint_edge.dsCoord)
                        .min(sub_int32(blues.zone[i].dsFlatEdge, cf2_int_to_fixed(1)));
                } else {
                    /* simply round captured edge */
                    ds_new = cf2_fixed_round(bottom_hint_edge.dsCoord);
                }

                ds_move = sub_int32(ds_new, bottom_hint_edge.dsCoord);
                captured = true;

                break;
            }
        }

        if !blues.zone[i].bottomZone && cf2_hint_is_top(top_hint_edge) {
            if sub_int32(blues.zone[i].csBottomEdge, cs_fuzz) <= top_hint_edge.csCoord
                && top_hint_edge.csCoord <= add_int32(blues.zone[i].csTopEdge, cs_fuzz)
            {
                /* top edge captured by top zone */

                if blues.suppressOvershoot {
                    ds_new = blues.zone[i].dsFlatEdge;
                } else if sub_int32(top_hint_edge.csCoord, blues.zone[i].csBottomEdge)
                    >= blues.blueShift
                {
                    /* guarantee minimum of 1 pixel overshoot */
                    ds_new = cf2_fixed_round(top_hint_edge.dsCoord)
                        .max(blues.zone[i].dsFlatEdge.wrapping_add(cf2_int_to_fixed(1)));
                } else {
                    /* simply round captured edge */
                    ds_new = cf2_fixed_round(top_hint_edge.dsCoord);
                }

                ds_move = sub_int32(ds_new, top_hint_edge.dsCoord);
                captured = true;

                break;
            }
        }
    }

    if captured {
        /* move both edges and flag them `locked' */
        if cf2_hint_is_valid(bottom_hint_edge) {
            bottom_hint_edge.dsCoord = add_int32(bottom_hint_edge.dsCoord, ds_move);

            cf2_hint_lock(bottom_hint_edge);
        }

        if cf2_hint_is_valid(top_hint_edge) {
            top_hint_edge.dsCoord = add_int32(top_hint_edge.dsCoord, ds_move);

            cf2_hint_lock(top_hint_edge);
        }
    }

    captured
}
