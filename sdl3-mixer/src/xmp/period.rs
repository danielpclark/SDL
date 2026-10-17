// Rust translation of src/period.c and src/period.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Notes, periods and pitch bends. The math is sdl3's (SDL's fdlibm).

use sdl3::stdlib::math::{ceil, floor, log, pow};

use super::common::*;

pub(crate) const PERIOD_BASE: f64 = 13696.0; /* C0 period */

/* Macros for period conversion */
pub(crate) const NOTE_B0: i32 = 11;
pub(crate) const NOTE_Bb0: i32 = NOTE_B0 + 1;
pub(crate) const MAX_NOTE: i32 = NOTE_B0 * 8;
pub(crate) const MAX_PERIOD: i32 = 0x1c56;
pub(crate) const MIN_PERIOD_A: i32 = 0x0071;
pub(crate) const MAX_PERIOD_A: i32 = 0x0358;
pub(crate) const MIN_PERIOD_L: i32 = 0x0000;
pub(crate) const MAX_PERIOD_L: i32 = 0x1e00;
pub(crate) const MIN_NOTE_MOD: i32 = 48;
pub(crate) const MAX_NOTE_MOD: i32 = 83;
pub(crate) const FT2_NOTE_BN1: i32 = 11; /* FT2 B-(-1) */
pub(crate) const FT2_NOTE_C0: i32 = 12; /* FT2 C-0 */
pub(crate) const FT2_NOTE_AS9: i32 = 130; /* FT2 A#9 */

/*
 * Period table from the Protracker V2.1A play routine
 */
#[rustfmt::skip]
static PT_PERIOD_TABLE: [[u16; 36]; 16] = [
    /* Tuning 0, Normal */
    [856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453, 428, 404, 381, 360, 339, 320, 302, 285, 269, 254, 240, 226, 214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120, 113],
    /* Tuning 1 */
    [850, 802, 757, 715, 674, 637, 601, 567, 535, 505, 477, 450, 425, 401, 379, 357, 337, 318, 300, 284, 268, 253, 239, 225, 213, 201, 189, 179, 169, 159, 150, 142, 134, 126, 119, 113],
    /* Tuning 2 */
    [844, 796, 752, 709, 670, 632, 597, 563, 532, 502, 474, 447, 422, 398, 376, 355, 335, 316, 298, 282, 266, 251, 237, 224, 211, 199, 188, 177, 167, 158, 149, 141, 133, 125, 118, 112],
    /* Tuning 3 */
    [838, 791, 746, 704, 665, 628, 592, 559, 528, 498, 470, 444, 419, 395, 373, 352, 332, 314, 296, 280, 264, 249, 235, 222, 209, 198, 187, 176, 166, 157, 148, 140, 132, 125, 118, 111],
    /* Tuning 4 */
    [832, 785, 741, 699, 660, 623, 588, 555, 524, 495, 467, 441, 416, 392, 370, 350, 330, 312, 294, 278, 262, 247, 233, 220, 208, 196, 185, 175, 165, 156, 147, 139, 131, 124, 117, 110],
    /* Tuning 5 */
    [826, 779, 736, 694, 655, 619, 584, 551, 520, 491, 463, 437, 413, 390, 368, 347, 328, 309, 292, 276, 260, 245, 232, 219, 206, 195, 184, 174, 164, 155, 146, 138, 130, 123, 116, 109],
    /* Tuning 6 */
    [820, 774, 730, 689, 651, 614, 580, 547, 516, 487, 460, 434, 410, 387, 365, 345, 325, 307, 290, 274, 258, 244, 230, 217, 205, 193, 183, 172, 163, 154, 145, 137, 129, 122, 115, 109],
    /* Tuning 7 */
    [814, 768, 725, 684, 646, 610, 575, 543, 513, 484, 457, 431, 407, 384, 363, 342, 323, 305, 288, 272, 256, 242, 228, 216, 204, 192, 181, 171, 161, 152, 144, 136, 128, 121, 114, 108],
    /* Tuning -8 */
    [907, 856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453, 428, 404, 381, 360, 339, 320, 302, 285, 269, 254, 240, 226, 214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120],
    /* Tuning -7 */
    [900, 850, 802, 757, 715, 675, 636, 601, 567, 535, 505, 477, 450, 425, 401, 379, 357, 337, 318, 300, 284, 268, 253, 238, 225, 212, 200, 189, 179, 169, 159, 150, 142, 134, 126, 119],
    /* Tuning -6 */
    [894, 844, 796, 752, 709, 670, 632, 597, 563, 532, 502, 474, 447, 422, 398, 376, 355, 335, 316, 298, 282, 266, 251, 237, 223, 211, 199, 188, 177, 167, 158, 149, 141, 133, 125, 118],
    /* Tuning -5 */
    [887, 838, 791, 746, 704, 665, 628, 592, 559, 528, 498, 470, 444, 419, 395, 373, 352, 332, 314, 296, 280, 264, 249, 235, 222, 209, 198, 187, 176, 166, 157, 148, 140, 132, 125, 118],
    /* Tuning -4 */
    [881, 832, 785, 741, 699, 660, 623, 588, 555, 524, 494, 467, 441, 416, 392, 370, 350, 330, 312, 294, 278, 262, 247, 233, 220, 208, 196, 185, 175, 165, 156, 147, 139, 131, 123, 117],
    /* Tuning -3 */
    [875, 826, 779, 736, 694, 655, 619, 584, 551, 520, 491, 463, 437, 413, 390, 368, 347, 328, 309, 292, 276, 260, 245, 232, 219, 206, 195, 184, 174, 164, 155, 146, 138, 130, 123, 116],
    /* Tuning -2 */
    [868, 820, 774, 730, 689, 651, 614, 580, 547, 516, 487, 460, 434, 410, 387, 365, 345, 325, 307, 290, 274, 258, 244, 230, 217, 205, 193, 183, 172, 163, 154, 145, 137, 129, 122, 115],
    /* Tuning -1 */
    [862, 814, 768, 725, 684, 646, 610, 575, 543, 513, 484, 457, 431, 407, 384, 363, 342, 323, 305, 288, 272, 256, 242, 228, 216, 203, 192, 181, 171, 161, 152, 144, 136, 128, 121, 114],

];

const M_LN2: f64 = 0.69314718055994530942;

/// Translation of `libxmp_round()`.
#[inline]
fn libxmp_round(val: f64) -> f64 {
    if val >= 0.0 {
        floor(val + 0.5)
    } else {
        ceil(val - 0.5)
    }
}

/* Get period from note using Protracker tuning */
/// Translation of `libxmp_note_to_period_pt()`.
#[inline]
fn libxmp_note_to_period_pt(mut n: i32, mut f: i32) -> i32 {
    if n < MIN_NOTE_MOD || n > MAX_NOTE_MOD {
        return -1;
    }

    n -= 48;
    f >>= 4;
    if f < -8 || f > 7 {
        return 0;
    }

    if f < 0 {
        f += 16;
    }

    PT_PERIOD_TABLE[f as usize][n as usize] as i32
}

/* Get period from note */
/// Translation of `libxmp_note_to_period()`.
pub(crate) fn libxmp_note_to_period(ctx: &ContextData, n: i32, f: i32, adj: f64) -> f64 {
    let m = &ctx.m;
    let p = &ctx.p;

    /* If mod replayer, modrng and Amiga mixing are active */
    if p.flags & super::XMP_FLAGS_A500 != 0 && m.is_amiga_mod() {
        return libxmp_note_to_period_pt(n, f) as f64;
    }

    let d = n as f64 + f as f64 / 128.0;

    let mut per = match m.period_type {
        PERIOD_LINEAR => (240.0 - d) * 16.0, /* Linear */
        PERIOD_CSPD => 8363.0 * pow(2.0, n as f64 / 12.0) / 32.0 + f as f64, /* Hz */
        _ => PERIOD_BASE / pow(2.0, d / 12.0), /* Amiga */
    };

    if adj > 0.1 {
        per *= adj;
    }

    per
}

/* For the software mixer */
/// Translation of `libxmp_note_to_period_mix()`.
pub(crate) fn libxmp_note_to_period_mix(n: i32, b: i32) -> f64 {
    let d = n as f64 + b as f64 / 12800.0;
    PERIOD_BASE / pow(2.0, d / 12.0)
}

/* Get note from period */
/* This function is used only by the MOD loader */
/// Translation of `libxmp_period_to_note()`.
pub(crate) fn libxmp_period_to_note(p: i32) -> i32 {
    if p <= 0 {
        return 0;
    }

    (libxmp_round(12.0 * log(PERIOD_BASE / p as f64) / M_LN2) + 1.0) as i32
}

/* Get pitchbend from base note and amiga period */
/// Translation of `libxmp_period_to_bend()`.
pub(crate) fn libxmp_period_to_bend(ctx: &ContextData, p: f64, n: i32, adj: f64) -> i32 {
    let m = &ctx.m;

    if n == 0 || p < 0.1 {
        return 0;
    }

    /* FIXME (upstream): with the Amiga mixer, notes out of the Protracker
     * range have a period of -1, and the log() of a negative number is
     * converted to int as x86-64 converts NaN (see c_int()). */
    match m.period_type {
        PERIOD_LINEAR => c_int(100.0 * (8.0 * (((240 - n) * 16) as f64 - p))),
        PERIOD_CSPD => {
            let d = libxmp_note_to_period(ctx, n, 0, adj);
            c_int(libxmp_round(100.0 * (1536.0 / M_LN2) * log(p / d)))
        }
        _ => {
            /* Amiga */
            let d = libxmp_note_to_period(ctx, n, 0, adj);
            c_int(libxmp_round(100.0 * (1536.0 / M_LN2) * log(d / p)))
        }
    }
}

/* Convert finetune = 1200 * log2(C2SPD/8363))
 *
 *      c = (1200.0 * log(c2spd) - 1200.0 * log(c4_rate)) / M_LN2;
 *      xpo = c/100;
 *      fin = 128 * (c%100) / 100;
 */
/// Translation of `libxmp_c2spd_to_note()`.
pub(crate) fn libxmp_c2spd_to_note(c2spd: i32, n: &mut i32, f: &mut i32) {
    if c2spd <= 0 {
        *n = 0;
        *f = 0;
        return;
    }

    let c = (1536.0 * log(c2spd as f64 / 8363.0) / M_LN2) as i32;
    *n = c / 128;
    *f = c % 128;
}

/* Gravis Ultrasound frequency increments in steps of Hz/1024, where Hz is the
 * current rate of the card and is dependent on the active channel count.
 * For <=14 channels, the rate is 44100. For 15 to 32 channels, the rate is
 * round(14 * 44100 / active_channels).
 */
static GUS_RATES: [f64; 19] = [
    /* <= 14 */ 44100.0, /* 15-20 */ 41160.0, 38587.5, 36317.65, 34300.0, 32494.74,
    30870.0, /* 21-26 */ 29400.0, 28063.64, 26843.48, 25725.0, 24696.0, 23746.15,
    /* 27-32 */ 22866.67, 22050.0, 21289.66, 20580.0, 19916.13, 19294.75,
];

/* Get a Gravis Ultrasound frequency offset in Hz for a given number of steps.
 */
/// Translation of `libxmp_gus_frequency_steps()`.
pub(crate) fn libxmp_gus_frequency_steps(num_steps: i32, mut num_channels_active: i32) -> f64 {
    clamp(&mut num_channels_active, 14, 32);
    (num_steps as f64 * GUS_RATES[(num_channels_active - 14) as usize]) / 1024.0
}
