// Rust translation of src/pshinter/pshglob.c, src/pshinter/pshglob.h and
// the globals part of include/freetype/internal/pshints.h from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2001-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! PostScript hinter global hinting management (body).
//!
//! C allocates the globals with `FT_QNEW`, leaving the members it does not
//! set uninitialized; they start at zero here.

#![allow(non_snake_case)]

use super::super::base::ftcalc::*;
use super::super::fttypes::*;

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                    GLOBAL HINTS INTERNALS                     *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `PS_GLOBALS_MAX_BLUE_ZONES`: the maximum number of blue zones in a font
/// global hints structure.
pub const PS_GLOBALS_MAX_BLUE_ZONES: usize = 16;

/// `PS_GLOBALS_MAX_STD_WIDTHS`: the maximum number of standard and snap
/// widths in either the horizontal or vertical direction.
pub const PS_GLOBALS_MAX_STD_WIDTHS: usize = 16;

/// `PSH_WidthRec`: standard and snap width
#[derive(Debug, Clone, Copy, Default)]
pub struct PshWidthRec {
    pub org: FtInt,
    pub cur: FtPos,
    pub fit: FtPos,
}

/// `PSH_WidthsRec`: standard and snap widths table
#[derive(Debug, Clone, Copy, Default)]
pub struct PshWidthsRec {
    pub count: FtUInt,
    pub widths: [PshWidthRec; PS_GLOBALS_MAX_STD_WIDTHS],
}

/// `PSH_DimensionRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct PshDimensionRec {
    pub stdw: PshWidthsRec,
    pub scale_mult: FtFixed,
    pub scale_delta: FtFixed,
}

/// `PSH_Blue_ZoneRec`: blue zone descriptor
#[derive(Debug, Clone, Copy, Default)]
pub struct PshBlueZoneRec {
    pub org_ref: FtInt,
    pub org_delta: FtInt,
    pub org_top: FtInt,
    pub org_bottom: FtInt,

    pub cur_ref: FtPos,
    pub cur_delta: FtPos,
    pub cur_bottom: FtPos,
    pub cur_top: FtPos,
}

/// `PSH_Blue_TableRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct PshBlueTableRec {
    pub count: FtUInt,
    pub zones: [PshBlueZoneRec; PS_GLOBALS_MAX_BLUE_ZONES],
}

/// `PSH_BluesRec`: blue zones table
#[derive(Debug, Clone, Copy, Default)]
pub struct PshBluesRec {
    pub normal_top: PshBlueTableRec,
    pub normal_bottom: PshBlueTableRec,
    pub family_top: PshBlueTableRec,
    pub family_bottom: PshBlueTableRec,

    pub blue_scale: FtFixed,
    pub blue_shift: FtInt,
    pub blue_threshold: FtInt,
    pub blue_fuzz: FtInt,
    pub no_overshoots: bool,
}

/// `PSH_GlobalsRec`: font globals.
/// dimension 0 => X coordinates + vertical hints/stems
/// dimension 1 => Y coordinates + horizontal hints/stems
#[derive(Debug, Clone, Copy, Default)]
pub struct PshGlobalsRec {
    pub dimension: [PshDimensionRec; 2],
    pub blues: PshBluesRec,
}

pub const PSH_BLUE_ALIGN_NONE: i32 = 0;
pub const PSH_BLUE_ALIGN_TOP: i32 = 1;
pub const PSH_BLUE_ALIGN_BOT: i32 = 2;

/// `PSH_AlignmentRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct PshAlignmentRec {
    pub align: i32,
    pub align_top: FtPos,
    pub align_bot: FtPos,
}

/// `PS_PrivateRec` (t1tables.h): a Type 1 private dictionary, as the PS
/// hinter reads it.
#[derive(Debug, Clone, Copy, Default)]
pub struct PsPrivateRec {
    pub unique_id: FtInt,
    pub lenIV: FtInt,

    pub num_blue_values: FtByte,
    pub num_other_blues: FtByte,
    pub num_family_blues: FtByte,
    pub num_family_other_blues: FtByte,

    pub blue_values: [FtShort; 14],
    pub other_blues: [FtShort; 10],

    pub family_blues: [FtShort; 14],
    pub family_other_blues: [FtShort; 10],

    pub blue_scale: FtFixed,
    pub blue_shift: FtInt,
    pub blue_fuzz: FtInt,

    pub standard_width: [FtUShort; 1],
    pub standard_height: [FtUShort; 1],

    pub num_snap_widths: FtByte,
    pub num_snap_heights: FtByte,
    pub force_bold: FtByte,
    pub round_stem_up: FtByte,

    pub snap_widths: [FtShort; 13],  /* including std width  */
    pub snap_heights: [FtShort; 13], /* including std height */

    pub expansion_factor: FtFixed,

    pub language_group: FtLong,
    pub password: FtLong,

    pub min_feature: [FtShort; 2],
}

/// `PSH_Globals_FuncsRec`
#[derive(Debug)]
pub struct PshGlobalsFuncsRec {
    pub create: fn(private_dict: &PsPrivateRec) -> FtResult<Box<PshGlobalsRec>>,
    pub set_scale: fn(
        globals: &mut PshGlobalsRec,
        x_scale: FtFixed,
        y_scale: FtFixed,
        x_delta: FtFixed,
        y_delta: FtFixed,
    ),
    pub destroy: fn(globals: Option<Box<PshGlobalsRec>>),
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                       STANDARD WIDTHS                         *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/* scale the widths/heights table */
fn psh_globals_scale_widths(globals: &mut PshGlobalsRec, direction: usize) {
    let dim = &mut globals.dimension[direction];
    let stdw = &mut dim.stdw;
    let mut count: FtUInt = stdw.count;
    let scale: FtFixed = dim.scale_mult;

    if count > 0 {
        let mut w = 0usize;
        stdw.widths[w].cur = ft_mul_fix(stdw.widths[w].org as FtLong, scale);
        stdw.widths[w].fit = ft_pix_round(stdw.widths[w].cur);
        let stand_cur = stdw.widths[0].cur; /* standard width/height */

        w += 1;
        count -= 1;

        while count > 0 && w < PS_GLOBALS_MAX_STD_WIDTHS {
            let mut wv: FtPos = ft_mul_fix(stdw.widths[w].org as FtLong, scale);
            let mut dist: FtPos = wv - stand_cur;

            if dist < 0 {
                dist = -dist;
            }

            if dist < 128 {
                wv = stand_cur;
            }

            stdw.widths[w].cur = wv;
            stdw.widths[w].fit = ft_pix_round(wv);

            count -= 1;
            w += 1;
        }
    }
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                       BLUE ZONES                              *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

fn psh_blues_set_zones_0(
    is_others: bool,
    mut read_count: FtUInt,
    read: &[FtShort],
    top_table: &mut PshBlueTableRec,
    bot_table: &mut PshBlueTableRec,
) {
    let mut count_top: FtUInt = top_table.count;
    let mut count_bot: FtUInt = bot_table.count;
    let mut first = true;
    let mut r = 0usize;

    let at = |i: usize| read.get(i).copied().unwrap_or(0) as FtInt;

    while read_count > 1 {
        let reference: FtInt;
        let delta: FtInt;
        let mut count: FtUInt;
        let top: bool;

        /* read blue zone entry, and select target top/bottom zone */
        let zones: &mut [PshBlueZoneRec; PS_GLOBALS_MAX_BLUE_ZONES];
        if first || is_others {
            reference = at(r + 1);
            delta = at(r).wrapping_sub(reference);

            zones = &mut bot_table.zones;
            count = count_bot;
            first = false;
            top = false;
        } else {
            reference = at(r);
            delta = at(r + 1).wrapping_sub(reference);

            zones = &mut top_table.zones;
            count = count_top;
            top = true;
        }

        /* insert into sorted table */
        let mut zone = 0usize;
        let mut skip = false;
        while count > 0 {
            if reference < zones[zone].org_ref {
                break;
            }

            if reference == zones[zone].org_ref {
                let delta0: FtInt = zones[zone].org_delta;

                /* we have two zones on the same reference position -- */
                /* only keep the largest one                           */
                if delta < 0 {
                    if delta < delta0 {
                        zones[zone].org_delta = delta;
                    }
                } else if delta > delta0 {
                    zones[zone].org_delta = delta;
                }
                skip = true;
                break;
            }
            count -= 1;
            zone += 1;
        }

        if !skip {
            while count > 0 {
                let c = count as usize;
                if zone + c < PS_GLOBALS_MAX_BLUE_ZONES {
                    zones[zone + c] = zones[zone + c - 1];
                }
                count -= 1;
            }

            if zone < PS_GLOBALS_MAX_BLUE_ZONES {
                zones[zone].org_ref = reference;
                zones[zone].org_delta = delta;
            }

            if top {
                count_top += 1;
            } else {
                count_bot += 1;
            }
        }

        /* Skip: */
        r += 2;
        read_count -= 2;
    }

    top_table.count = count_top;
    bot_table.count = count_bot;
}

/* Re-read blue zones from the original fonts and store them into our */
/* private structure.  This function re-orders, sanitizes, and        */
/* fuzz-expands the zones as well.                                    */
#[allow(clippy::too_many_arguments)]
fn psh_blues_set_zones(
    target: &mut PshBluesRec,
    count: FtUInt,
    blues: &[FtShort],
    count_others: FtUInt,
    other_blues: &[FtShort],
    fuzz: FtInt,
    family: FtInt,
) {
    let (top_table, bot_table) = if family != 0 {
        (&mut target.family_top, &mut target.family_bottom)
    } else {
        (&mut target.normal_top, &mut target.normal_bottom)
    };

    /* read the input blue zones, and build two sorted tables  */
    /* (one for the top zones, the other for the bottom zones) */
    top_table.count = 0;
    bot_table.count = 0;

    /* first, the blues */
    psh_blues_set_zones_0(false, count, blues, top_table, bot_table);
    psh_blues_set_zones_0(true, count_others, other_blues, top_table, bot_table);

    let count_top: FtUInt = top_table.count;
    let count_bot: FtUInt = bot_table.count;

    let n_top = (count_top as usize).min(PS_GLOBALS_MAX_BLUE_ZONES);
    let n_bot = (count_bot as usize).min(PS_GLOBALS_MAX_BLUE_ZONES);

    /* sanitize top table */
    if count_top > 0 {
        let zones = &mut top_table.zones;
        for z in 0..n_top {
            let count = count_top as usize - z;

            if count > 1 && z + 1 < PS_GLOBALS_MAX_BLUE_ZONES {
                let delta: FtInt = zones[z + 1].org_ref.wrapping_sub(zones[z].org_ref);
                if zones[z].org_delta > delta {
                    zones[z].org_delta = delta;
                }
            }

            zones[z].org_bottom = zones[z].org_ref;
            zones[z].org_top = zones[z].org_delta.wrapping_add(zones[z].org_ref);
        }
    }

    /* sanitize bottom table */
    if count_bot > 0 {
        let zones = &mut bot_table.zones;
        for z in 0..n_bot {
            let count = count_bot as usize - z;

            if count > 1 && z + 1 < PS_GLOBALS_MAX_BLUE_ZONES {
                let delta: FtInt = zones[z].org_ref.wrapping_sub(zones[z + 1].org_ref);
                if zones[z].org_delta < delta {
                    zones[z].org_delta = delta;
                }
            }

            zones[z].org_top = zones[z].org_ref;
            zones[z].org_bottom = zones[z].org_delta.wrapping_add(zones[z].org_ref);
        }
    }

    /* expand top and bottom tables with blue fuzz */
    {
        let mut top: FtInt;
        let mut bot: FtInt;
        let mut delta: FtInt;

        for dim in [1, 0] {
            let (zones, mut count) = if dim == 1 {
                (&mut top_table.zones, count_top)
            } else {
                (&mut bot_table.zones, count_bot)
            };
            let mut zone = 0usize;

            if count > 0 {
                /* expand the bottom of the lowest zone normally */
                zones[zone].org_bottom = zones[zone].org_bottom.wrapping_sub(fuzz);

                /* expand the top and bottom of intermediate zones;    */
                /* checking that the interval is smaller than the fuzz */
                top = zones[zone].org_top;

                count -= 1;
                while count > 0 && zone + 1 < PS_GLOBALS_MAX_BLUE_ZONES {
                    bot = zones[zone + 1].org_bottom;
                    delta = bot.wrapping_sub(top);

                    if delta / 2 < fuzz {
                        let v = top.wrapping_add(delta / 2);
                        zones[zone + 1].org_bottom = v;
                        zones[zone].org_top = v;
                    } else {
                        zones[zone].org_top = top.wrapping_add(fuzz);
                        zones[zone + 1].org_bottom = bot.wrapping_sub(fuzz);
                    }

                    zone += 1;
                    top = zones[zone].org_top;
                    count -= 1;
                }

                /* expand the top of the highest zone normally */
                zones[zone].org_top = top.wrapping_add(fuzz);
            }
        }
    }
}

/* reset the blues table when the device transform changes */
fn psh_blues_scale_zones(blues: &mut PshBluesRec, scale: FtFixed, delta: FtPos) {
    /*                                                        */
    /* Determine whether we need to suppress overshoots or    */
    /* not.  We simply need to compare the vertical scale     */
    /* parameter to the raw bluescale value.  Here is why:    */
    /*                                                        */
    /*   We need to suppress overshoots for all pointsizes.   */
    /*   At 300dpi that satisfies:                            */
    /*                                                        */
    /*      pointsize < 240*bluescale + 0.49                  */
    /*                                                        */
    /*   This corresponds to:                                 */
    /*                                                        */
    /*      pixelsize < 1000*bluescale + 49/24                */
    /*                                                        */
    /*      scale*EM_Size < 1000*bluescale + 49/24            */
    /*                                                        */
    /*   However, for normal Type 1 fonts, EM_Size is 1000!   */
    /*   We thus only check:                                  */
    /*                                                        */
    /*      scale < bluescale + 49/24000                      */
    /*                                                        */
    /*   which we shorten to                                  */
    /*                                                        */
    /*      "scale < bluescale"                               */
    /*                                                        */
    /* Note that `blue_scale' is stored 1000 times its real   */
    /* value, and that `scale' converts from font units to    */
    /* fractional pixels.                                     */
    /*                                                        */

    /* 1000 / 64 = 125 / 8 */
    if scale >= 0x20C49BA {
        blues.no_overshoots = scale < blues.blue_scale.wrapping_mul(8) / 125;
    } else {
        blues.no_overshoots = scale.wrapping_mul(125) < blues.blue_scale.wrapping_mul(8);
    }

    /*                                                        */
    /*  The blue threshold is the font units distance under   */
    /*  which overshoots are suppressed due to the BlueShift  */
    /*  even if the scale is greater than BlueScale.          */
    /*                                                        */
    /*  It is the smallest distance such that                 */
    /*                                                        */
    /*    dist <= BlueShift && dist*scale <= 0.5 pixels       */
    /*                                                        */
    {
        let mut threshold: FtInt = blues.blue_shift;

        while threshold > 0 && ft_mul_fix(threshold as FtLong, scale) > 32 {
            threshold -= 1;
        }

        blues.blue_threshold = threshold;
    }

    for num in 0..4 {
        let table = match num {
            0 => &mut blues.normal_top,
            1 => &mut blues.normal_bottom,
            2 => &mut blues.family_top,
            _ => &mut blues.family_bottom,
        };

        let n = (table.count as usize).min(PS_GLOBALS_MAX_BLUE_ZONES);
        for zone in table.zones[..n].iter_mut() {
            zone.cur_top = ft_mul_fix(zone.org_top as FtLong, scale).wrapping_add(delta);
            zone.cur_bottom = ft_mul_fix(zone.org_bottom as FtLong, scale).wrapping_add(delta);
            zone.cur_ref = ft_mul_fix(zone.org_ref as FtLong, scale).wrapping_add(delta);
            zone.cur_delta = ft_mul_fix(zone.org_delta as FtLong, scale);

            /* round scaled reference position */
            zone.cur_ref = ft_pix_round(zone.cur_ref);
        }
    }

    /* process the families now */

    for num in 0..2 {
        let (normal, family) = match num {
            0 => (&mut blues.normal_top, &blues.family_top),
            _ => (&mut blues.normal_bottom, &blues.family_bottom),
        };

        let n1 = (normal.count as usize).min(PS_GLOBALS_MAX_BLUE_ZONES);
        let n2 = (family.count as usize).min(PS_GLOBALS_MAX_BLUE_ZONES);
        for zone1 in normal.zones[..n1].iter_mut() {
            /* try to find a family zone whose reference position is less */
            /* than 1 pixel far from the current zone                     */
            for zone2 in family.zones[..n2].iter() {
                let mut delta: FtPos = zone1.org_ref as FtPos - zone2.org_ref as FtPos;
                if delta < 0 {
                    delta = -delta;
                }

                if ft_mul_fix(delta, scale) < 64 {
                    zone1.cur_top = zone2.cur_top;
                    zone1.cur_bottom = zone2.cur_bottom;
                    zone1.cur_ref = zone2.cur_ref;
                    zone1.cur_delta = zone2.cur_delta;
                    break;
                }
            }
        }
    }
}

/* calculate the maximum height of given blue zones */
fn psh_calc_max_height(num: FtUInt, values: &[FtShort], mut cur_max: FtShort) -> FtShort {
    let at = |i: usize| values.get(i).copied().unwrap_or(0);

    let mut count = 0;
    while count < num as usize {
        let cur_height: FtShort = at(count + 1).wrapping_sub(at(count));

        if cur_height > cur_max {
            cur_max = cur_height;
        }
        count += 2;
    }

    cur_max
}

/// `psh_blues_snap_stem`: snap a stem to one or two blue zones
pub fn psh_blues_snap_stem(
    blues: &PshBluesRec,
    stem_top: FtInt,
    stem_bot: FtInt,
    alignment: &mut PshAlignmentRec,
) {
    alignment.align = PSH_BLUE_ALIGN_NONE;

    let no_shoots: bool = blues.no_overshoots;

    /* look up stem top in top zones table */
    let table = &blues.normal_top;
    let n = (table.count as usize).min(PS_GLOBALS_MAX_BLUE_ZONES);
    for zone in table.zones[..n].iter() {
        let delta: FtPos = sub_long(stem_top as FtLong, zone.org_bottom as FtLong);
        if delta < -(blues.blue_fuzz as FtPos) {
            break;
        }

        if stem_top <= zone.org_top.wrapping_add(blues.blue_fuzz) {
            if no_shoots || delta <= blues.blue_threshold as FtPos {
                alignment.align |= PSH_BLUE_ALIGN_TOP;
                alignment.align_top = zone.cur_ref;
            }
            break;
        }
    }

    /* look up stem bottom in bottom zones table */
    let table = &blues.normal_bottom;
    let n = (table.count as usize).min(PS_GLOBALS_MAX_BLUE_ZONES);
    for zone in table.zones[..n].iter().rev() {
        let delta: FtPos = sub_long(zone.org_top as FtLong, stem_bot as FtLong);
        if delta < -(blues.blue_fuzz as FtPos) {
            break;
        }

        if stem_bot >= zone.org_bottom.wrapping_sub(blues.blue_fuzz) {
            if no_shoots || delta < blues.blue_threshold as FtPos {
                alignment.align |= PSH_BLUE_ALIGN_BOT;
                alignment.align_bot = zone.cur_ref;
            }
            break;
        }
    }
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                        GLOBAL HINTS                           *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

fn psh_globals_destroy(globals: Option<Box<PshGlobalsRec>>) {
    if let Some(mut globals) = globals {
        globals.dimension[0].stdw.count = 0;
        globals.dimension[1].stdw.count = 0;

        globals.blues.normal_top.count = 0;
        globals.blues.normal_bottom.count = 0;
        globals.blues.family_top.count = 0;
        globals.blues.family_bottom.count = 0;
    }
}

fn psh_globals_new(priv_: &PsPrivateRec) -> FtResult<Box<PshGlobalsRec>> {
    let mut globals = Box::new(PshGlobalsRec::default());

    /* copy standard widths */
    {
        let dim = &mut globals.dimension[1];
        dim.stdw.widths[0].org = priv_.standard_width[0] as FtInt;

        let n = priv_.num_snap_widths as usize;
        for (k, &v) in priv_.snap_widths.iter().take(n).enumerate() {
            if k + 1 < PS_GLOBALS_MAX_STD_WIDTHS {
                dim.stdw.widths[k + 1].org = v as FtInt;
            }
        }

        dim.stdw.count = priv_.num_snap_widths as FtUInt + 1;
    }

    /* copy standard heights */
    {
        let dim = &mut globals.dimension[0];
        dim.stdw.widths[0].org = priv_.standard_height[0] as FtInt;

        let n = priv_.num_snap_heights as usize;
        for (k, &v) in priv_.snap_heights.iter().take(n).enumerate() {
            if k + 1 < PS_GLOBALS_MAX_STD_WIDTHS {
                dim.stdw.widths[k + 1].org = v as FtInt;
            }
        }

        dim.stdw.count = priv_.num_snap_heights as FtUInt + 1;
    }

    /* copy blue zones */
    psh_blues_set_zones(
        &mut globals.blues,
        priv_.num_blue_values as FtUInt,
        &priv_.blue_values,
        priv_.num_other_blues as FtUInt,
        &priv_.other_blues,
        priv_.blue_fuzz,
        0,
    );

    psh_blues_set_zones(
        &mut globals.blues,
        priv_.num_family_blues as FtUInt,
        &priv_.family_blues,
        priv_.num_family_other_blues as FtUInt,
        &priv_.family_other_blues,
        priv_.blue_fuzz,
        1,
    );

    /* limit the BlueScale value to `1 / max_of_blue_zone_heights' */
    {
        let mut max_height: FtShort = 1;

        max_height = psh_calc_max_height(
            priv_.num_blue_values as FtUInt,
            &priv_.blue_values,
            max_height,
        );
        max_height = psh_calc_max_height(
            priv_.num_other_blues as FtUInt,
            &priv_.other_blues,
            max_height,
        );
        max_height = psh_calc_max_height(
            priv_.num_family_blues as FtUInt,
            &priv_.family_blues,
            max_height,
        );
        max_height = psh_calc_max_height(
            priv_.num_family_other_blues as FtUInt,
            &priv_.family_other_blues,
            max_height,
        );

        /* BlueScale is scaled 1000 times */
        let max_scale: FtFixed = ft_div_fix(1000, max_height as FtLong);
        globals.blues.blue_scale = if priv_.blue_scale < max_scale {
            priv_.blue_scale
        } else {
            max_scale
        };
    }

    globals.blues.blue_shift = priv_.blue_shift;
    globals.blues.blue_fuzz = priv_.blue_fuzz;

    globals.dimension[0].scale_mult = 0;
    globals.dimension[0].scale_delta = 0;
    globals.dimension[1].scale_mult = 0;
    globals.dimension[1].scale_delta = 0;

    Ok(globals)
}

/// `psh_globals_set_scale`
pub fn psh_globals_set_scale(
    globals: &mut PshGlobalsRec,
    x_scale: FtFixed,
    y_scale: FtFixed,
    x_delta: FtFixed,
    y_delta: FtFixed,
) {
    let dim = &mut globals.dimension[0];
    if x_scale != dim.scale_mult || x_delta != dim.scale_delta {
        dim.scale_mult = x_scale;
        dim.scale_delta = x_delta;

        psh_globals_scale_widths(globals, 0);
    }

    let dim = &mut globals.dimension[1];
    if y_scale != dim.scale_mult || y_delta != dim.scale_delta {
        dim.scale_mult = y_scale;
        dim.scale_delta = y_delta;

        psh_globals_scale_widths(globals, 1);
        psh_blues_scale_zones(&mut globals.blues, y_scale, y_delta);
    }
}

/// `psh_globals_funcs_init`
pub fn psh_globals_funcs_init() -> PshGlobalsFuncsRec {
    PshGlobalsFuncsRec {
        create: psh_globals_new,
        set_scale: psh_globals_set_scale,
        destroy: psh_globals_destroy,
    }
}

/// The globals functions record (the module's `globals_funcs`).
pub static PSH_GLOBALS_FUNCS: PshGlobalsFuncsRec = PshGlobalsFuncsRec {
    create: psh_globals_new,
    set_scale: psh_globals_set_scale,
    destroy: psh_globals_destroy,
};
