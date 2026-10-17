/*
 * emu2413 v1.5.9
 * https://github.com/digital-sound-antiques/emu2413
 * Copyright (C) 2020 Mitsutaka Okazaki
 *
 * This source refers to the following documents. The author would like to thank all the authors who have
 * contributed to the writing of them.
 * - [YM2413 notes](http://www.smspower.org/Development/YM2413) by andete
 * - ymf262.c by Jarek Burczynski
 * - [VRC7 presets](https://siliconpr0n.org/archive/doku.php?id=vendor:yamaha:opl2#opll_vrc7_patch_format) by Nuke.YKT
 * - YMF281B presets by Chabin
 */
// emu2413 is under the MIT license (see COPYING.emu2413).
// Modified 2026-10-17: translated into Rust from SDL_mixer 3.3.0's Game_Music_Emu 0.6.5 (gme/ext/; see NOTICE).

//! A YM2413 (OPLL) emulator (`ext/emu2413.h`, `ext/emu2413.c`), which
//! libgme uses for the NES VRC7.
//!
//! What the VRC7 reaches is translated: creating the chip at its own rate
//! (clock / 72, at which the internal rate converter is off; the converter
//! is left out), resetting it, the VRC7 chip type and patch set, register
//! writes and stereo output. The patch dump, pan and mask functions are
//! left out. The tables upstream builds once in statics are built once
//! here too.

use std::sync::OnceLock;

const OPLL_TONE_NUM: usize = 3;

/// voice data
#[derive(Clone, Copy, Debug, Default)]
pub struct OpllPatch {
    pub tl: u32,
    pub fb: u32,
    pub eg: u32,
    pub ml: u32,
    pub ar: u32,
    pub dr: u32,
    pub sl: u32,
    pub rr: u32,
    pub kr: u32,
    pub kl: u32,
    pub am: u32,
    pub pm: u32,
    pub ws: u32,
}

/// The index of the `null_patch` slots point to before a patch is set.
const NULL_PATCH: usize = 19 * 2;

/// slot
#[derive(Clone, Copy, Debug, Default)]
struct OpllSlot {
    number: u8,
    /// type flags:
    /// 000000SM
    ///       |+-- M: 0:modulator 1:carrier
    ///       +--- S: 0:normal 1:single slot mode (sd, tom, hh or cym)
    type_: u8,
    /// voice parameter (an index into the chip's patches)
    patch: usize,
    /// slot output: output value, latest and previous.
    output: [i32; 2],
    // phase generator (pg)
    /// wave table (0: full sine, 1: half sine)
    wave_table: usize,
    /// pg phase
    pg_phase: u32,
    /// pg output, as index of wave table
    pg_out: u32,
    /// if 1, pg_phase is preserved when key-on
    pg_keep: u8,
    /// (block << 9) | f-number
    blk_fnum: u16,
    /// f-number (9 bits)
    fnum: u16,
    /// block (3 bits)
    blk: u8,
    // envelope generator (eg)
    /// current state
    eg_state: u8,
    /// current volume
    volume: i32,
    /// key-on flag 1:on 0:off
    key_flag: u8,
    /// key-sus option 1:on 0:off
    sus_flag: u8,
    /// total level + key scale level
    tll: u16,
    /// key scale offset (rks) for eg speed
    rks: u8,
    /// eg speed rate high 4bits
    eg_rate_h: u8,
    /// eg speed rate low 2bits
    eg_rate_l: u8,
    /// shift for eg global counter, controls envelope speed
    eg_shift: u32,
    /// eg output
    eg_out: u32,
    /// flags to debounce update
    update_requests: u32,
}

/* clang-format off */
static DEFAULT_INST: [[u8; (16 + 3) * 8]; OPLL_TONE_NUM] = [
    [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 0: User
        0x71, 0x61, 0x1e, 0x17, 0xd0, 0x78, 0x00, 0x17, // 1: Violin
        0x13, 0x41, 0x1a, 0x0d, 0xd8, 0xf7, 0x23, 0x13, // 2: Guitar
        0x13, 0x01, 0x99, 0x00, 0xf2, 0xc4, 0x11, 0x23, // 3: Piano
        0x31, 0x61, 0x0e, 0x07, 0xa8, 0x64, 0x70, 0x27, // 4: Flute
        0x32, 0x21, 0x1e, 0x06, 0xe0, 0x76, 0x00, 0x28, // 5: Clarinet
        0x31, 0x22, 0x16, 0x05, 0xe0, 0x71, 0x00, 0x18, // 6: Oboe
        0x21, 0x61, 0x1d, 0x07, 0x82, 0x81, 0x10, 0x07, // 7: Trumpet
        0x23, 0x21, 0x2d, 0x14, 0xa2, 0x72, 0x00, 0x07, // 8: Organ
        0x61, 0x61, 0x1b, 0x06, 0x64, 0x65, 0x10, 0x17, // 9: Horn
        0x41, 0x61, 0x0b, 0x18, 0x85, 0xf7, 0x71, 0x07, // A: Synthesizer
        0x13, 0x01, 0x83, 0x11, 0xfa, 0xe4, 0x10, 0x04, // B: Harpsichord
        0x17, 0xc1, 0x24, 0x07, 0xf8, 0xf8, 0x22, 0x12, // C: Vibraphone
        0x61, 0x50, 0x0c, 0x05, 0xc2, 0xf5, 0x20, 0x42, // D: Synthesizer Bass
        0x01, 0x01, 0x55, 0x03, 0xc9, 0x95, 0x03, 0x02, // E: Acoustic Bass
        0x61, 0x41, 0x89, 0x03, 0xf1, 0xe4, 0x40, 0x13, // F: Electric Guitar
        0x01, 0x01, 0x18, 0x0f, 0xdf, 0xf8, 0x6a, 0x6d, // R: Bass Drum
        0x01, 0x01, 0x00, 0x00, 0xc8, 0xd8, 0xa7, 0x48, // R: High-Hat(M) / Snare Drum(C)
        0x05, 0x01, 0x00, 0x00, 0xf8, 0xaa, 0x59, 0x55, // R: Tom-tom(M) / Top Cymbal(C)
    ],
    [
        /* VRC7 presets from Nuke.YKT */
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x21, 0x05, 0x06, 0xe8, 0x81, 0x42,
        0x27, 0x13, 0x41, 0x14, 0x0d, 0xd8, 0xf6, 0x23, 0x12, 0x11, 0x11, 0x08, 0x08, 0xfa, 0xb2,
        0x20, 0x12, 0x31, 0x61, 0x0c, 0x07, 0xa8, 0x64, 0x61, 0x27, 0x32, 0x21, 0x1e, 0x06, 0xe1,
        0x76, 0x01, 0x28, 0x02, 0x01, 0x06, 0x00, 0xa3, 0xe2, 0xf4, 0xf4, 0x21, 0x61, 0x1d, 0x07,
        0x82, 0x81, 0x11, 0x07, 0x23, 0x21, 0x22, 0x17, 0xa2, 0x72, 0x01, 0x17, 0x35, 0x11, 0x25,
        0x00, 0x40, 0x73, 0x72, 0x01, 0xb5, 0x01, 0x0f, 0x0F, 0xa8, 0xa5, 0x51, 0x02, 0x17, 0xc1,
        0x24, 0x07, 0xf8, 0xf8, 0x22, 0x12, 0x71, 0x23, 0x11, 0x06, 0x65, 0x74, 0x18, 0x16, 0x01,
        0x02, 0xd3, 0x05, 0xc9, 0x95, 0x03, 0x02, 0x61, 0x63, 0x0c, 0x00, 0x94, 0xC0, 0x33, 0xf6,
        0x21, 0x72, 0x0d, 0x00, 0xc1, 0xd5, 0x56, 0x06, 0x01, 0x01, 0x18, 0x0f, 0xdf, 0xf8, 0x6a,
        0x6d, 0x01, 0x01, 0x00, 0x00, 0xc8, 0xd8, 0xa7, 0x68, 0x05, 0x01, 0x00, 0x00, 0xf8, 0xaa,
        0x59, 0x55,
    ],
    [
        /* YMF281B presets */
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 0: User
        0x62, 0x21, 0x1a, 0x07, 0xf0, 0x6f, 0x00,
        0x16, // 1: Electric Strings (form Chabin's patch)
        0x40, 0x10, 0x45, 0x00, 0xf6, 0x83, 0x73,
        0x63, // 2: Bow Wow (based on plgDavid's patch, KSL fixed)
        0x13, 0x01, 0x99, 0x00, 0xf2, 0xc3, 0x21,
        0x23, // 3: Electric Guitar (similar to YM2413 but different DR(C))
        0x01, 0x61, 0x0b, 0x0f, 0xf9, 0x64, 0x70,
        0x17, // 4: Organ (based on Chabin, TL/DR fixed)
        0x32, 0x21, 0x1e, 0x06, 0xe1, 0x76, 0x01, 0x28, // 5: Clarinet (identical to YM2413)
        0x60, 0x01, 0x82, 0x0e, 0xf9, 0x61, 0x20,
        0x27, // 6: Saxophone (based on plgDavid, PM/EG fixed)
        0x21, 0x61, 0x1c, 0x07, 0x84, 0x81, 0x11,
        0x07, // 7: Trumpet (similar to YM2413 but different TL/DR(M))
        0x37, 0x32, 0xc9, 0x01, 0x66, 0x64, 0x40, 0x28, // 8: Street Organ (from Chabin)
        0x01, 0x21, 0x07, 0x03, 0xa5, 0x71, 0x51,
        0x07, // 9: Synth Brass (based on Chabin, TL fixed)
        0x06, 0x01, 0x5e, 0x07, 0xf3, 0xf3, 0xf6,
        0x13, // A: Electric Piano (based on Chabin, DR/RR/KR fixed)
        0x00, 0x00, 0x18, 0x06, 0xf5, 0xf3, 0x20, 0x23, // B: Bass (based on Chabin, EG fixed)
        0x17, 0xc1, 0x24, 0x07, 0xf8, 0xf8, 0x22, 0x12, // C: Vibraphone (identical to YM2413)
        0x35, 0x64, 0x00, 0x00, 0xff, 0xf3, 0x77, 0xf5, // D: Chimes (from plgDavid)
        0x11, 0x31, 0x00, 0x07, 0xdd, 0xf3, 0xff, 0xfb, // E: Tom Tom II (from plgDavid)
        0x3a, 0x21, 0x00, 0x07, 0x80, 0x84, 0x0f,
        0xf5, // F: Noise (based on plgDavid, AR fixed)
        0x01, 0x01, 0x18, 0x0f, 0xdf, 0xf8, 0x6a, 0x6d, // R: Bass Drum (identical to YM2413)
        0x01, 0x01, 0x00, 0x00, 0xc8, 0xd8, 0xa7,
        0x68, // R: High-Hat(M) / Snare Drum(C) (identical to YM2413)
        0x05, 0x01, 0x00, 0x00, 0xf8, 0xaa, 0x59,
        0x55, // R: Tom-tom(M) / Top Cymbal(C) (identical to YM2413)
    ],
];
/* clang-format on */

/* phase increment counter */
const DP_BITS: u32 = 19;
const DP_WIDTH: u32 = 1 << DP_BITS;
const DP_BASE_BITS: u32 = DP_BITS - PG_BITS;

/* dynamic range of envelope output */
const EG_STEP: f64 = 0.375;
const EG_BITS: u32 = 7;
const EG_MUTE: u32 = (1 << EG_BITS) - 1;
const EG_MAX: u32 = EG_MUTE - 4;

/* dynamic range of total level */
const TL_BITS: u32 = 6;

/* damper speed before key-on. key-scale affects. */
const DAMPER_RATE: u32 = 12;

#[inline]
const fn tl2eg(d: u32) -> u32 {
    d << 1
}

/* sine table */
const PG_BITS: u32 = 10; /* 2^10 = 1024 length sine table */
const PG_WIDTH: usize = 1 << PG_BITS;

/* clang-format off */
/* exp_table[x] = round((exp2((double)x / 256.0) - 1) * 1024) */
static EXP_TABLE: [u16; 256] = [
    0, 3, 6, 8, 11, 14, 17, 20, 22, 25, 28, 31, 34, 37, 40, 42, 45, 48, 51, 54, 57, 60, 63, 66, 69,
    72, 75, 78, 81, 84, 87, 90, 93, 96, 99, 102, 105, 108, 111, 114, 117, 120, 123, 126, 130, 133,
    136, 139, 142, 145, 148, 152, 155, 158, 161, 164, 168, 171, 174, 177, 181, 184, 187, 190, 194,
    197, 200, 204, 207, 210, 214, 217, 220, 224, 227, 231, 234, 237, 241, 244, 248, 251, 255, 258,
    262, 265, 268, 272, 276, 279, 283, 286, 290, 293, 297, 300, 304, 308, 311, 315, 318, 322, 326,
    329, 333, 337, 340, 344, 348, 352, 355, 359, 363, 367, 370, 374, 378, 382, 385, 389, 393, 397,
    401, 405, 409, 412, 416, 420, 424, 428, 432, 436, 440, 444, 448, 452, 456, 460, 464, 468, 472,
    476, 480, 484, 488, 492, 496, 501, 505, 509, 513, 517, 521, 526, 530, 534, 538, 542, 547, 551,
    555, 560, 564, 568, 572, 577, 581, 585, 590, 594, 599, 603, 607, 612, 616, 621, 625, 630, 634,
    639, 643, 648, 652, 657, 661, 666, 670, 675, 680, 684, 689, 693, 698, 703, 708, 712, 717, 722,
    726, 731, 736, 741, 745, 750, 755, 760, 765, 770, 774, 779, 784, 789, 794, 799, 804, 809, 814,
    819, 824, 829, 834, 839, 844, 849, 854, 859, 864, 869, 874, 880, 885, 890, 895, 900, 906, 911,
    916, 921, 927, 932, 937, 942, 948, 953, 959, 964, 969, 975, 980, 986, 991, 996, 1002, 1007,
    1013, 1018,
];
/* fullsin_table[x] = round(-log2(sin((x + 0.5) * PI / (PG_WIDTH / 4) / 2)) * 256) */
/* (its first quarter; makeSinTable() fills in the rest) */
static FULLSIN_QUARTER: [u16; PG_WIDTH / 4] = [
    2137, 1731, 1543, 1419, 1326, 1252, 1190, 1137, 1091, 1050, 1013, 979, 949, 920, 894, 869, 846,
    825, 804, 785, 767, 749, 732, 717, 701, 687, 672, 659, 646, 633, 621, 609, 598, 587, 576, 566,
    556, 546, 536, 527, 518, 509, 501, 492, 484, 476, 468, 461, 453, 446, 439, 432, 425, 418, 411,
    405, 399, 392, 386, 380, 375, 369, 363, 358, 352, 347, 341, 336, 331, 326, 321, 316, 311, 307,
    302, 297, 293, 289, 284, 280, 276, 271, 267, 263, 259, 255, 251, 248, 244, 240, 236, 233, 229,
    226, 222, 219, 215, 212, 209, 205, 202, 199, 196, 193, 190, 187, 184, 181, 178, 175, 172, 169,
    167, 164, 161, 159, 156, 153, 151, 148, 146, 143, 141, 138, 136, 134, 131, 129, 127, 125, 122,
    120, 118, 116, 114, 112, 110, 108, 106, 104, 102, 100, 98, 96, 94, 92, 91, 89, 87, 85, 83, 82,
    80, 78, 77, 75, 74, 72, 70, 69, 67, 66, 64, 63, 62, 60, 59, 57, 56, 55, 53, 52, 51, 49, 48, 47,
    46, 45, 43, 42, 41, 40, 39, 38, 37, 36, 35, 34, 33, 32, 31, 30, 29, 28, 27, 26, 25, 24, 23, 23,
    22, 21, 20, 20, 19, 18, 17, 17, 16, 15, 15, 14, 13, 13, 12, 12, 11, 10, 10, 9, 9, 8, 8, 7, 7,
    7, 6, 6, 5, 5, 5, 4, 4, 4, 3, 3, 3, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0,
];
/* clang-format on */

/* pitch modulator */
/* offset to fnum, rough approximation of 14 cents depth. */
static PM_TABLE: [[i8; 8]; 8] = [
    [0, 0, 0, 0, 0, 0, 0, 0],    // fnum = 000xxxxxx
    [0, 0, 1, 0, 0, 0, -1, 0],   // fnum = 001xxxxxx
    [0, 1, 2, 1, 0, -1, -2, -1], // fnum = 010xxxxxx
    [0, 1, 3, 1, 0, -1, -3, -1], // fnum = 011xxxxxx
    [0, 2, 4, 2, 0, -2, -4, -2], // fnum = 100xxxxxx
    [0, 2, 5, 2, 0, -2, -5, -2], // fnum = 101xxxxxx
    [0, 3, 6, 3, 0, -3, -6, -3], // fnum = 110xxxxxx
    [0, 3, 7, 3, 0, -3, -7, -3], // fnum = 111xxxxxx
];

/* amplitude lfo table */
/* The following envelop pattern is verified on real YM2413. */
/* each element repeates 64 cycles */
static AM_TABLE: [u8; 210] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, //
    2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, //
    4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, //
    6, 6, 6, 6, 6, 6, 6, 6, 7, 7, 7, 7, 7, 7, 7, 7, //
    8, 8, 8, 8, 8, 8, 8, 8, 9, 9, 9, 9, 9, 9, 9, 9, //
    10, 10, 10, 10, 10, 10, 10, 10, 11, 11, 11, 11, 11, 11, 11, 11, //
    12, 12, 12, 12, 12, 12, 12, 12, //
    13, 13, 13, //
    12, 12, 12, 12, 12, 12, 12, 12, //
    11, 11, 11, 11, 11, 11, 11, 11, 10, 10, 10, 10, 10, 10, 10, 10, //
    9, 9, 9, 9, 9, 9, 9, 9, 8, 8, 8, 8, 8, 8, 8, 8, //
    7, 7, 7, 7, 7, 7, 7, 7, 6, 6, 6, 6, 6, 6, 6, 6, //
    5, 5, 5, 5, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4, //
    3, 3, 3, 3, 3, 3, 3, 3, 2, 2, 2, 2, 2, 2, 2, 2, //
    1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0,
];

/* envelope decay increment step table */
/* based on andete's research */
static EG_STEP_TABLES: [[u8; 8]; 4] = [
    [0, 1, 0, 1, 0, 1, 0, 1],
    [0, 1, 0, 1, 1, 1, 0, 1],
    [0, 1, 1, 1, 0, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 1],
];

const ATTACK: u8 = 0;
const DECAY: u8 = 1;
const SUSTAIN: u8 = 2;
const RELEASE: u8 = 3;
const DAMP: u8 = 4;

static ML_TABLE: [u32; 16] = [
    1,
    1 * 2,
    2 * 2,
    3 * 2,
    4 * 2,
    5 * 2,
    6 * 2,
    7 * 2,
    8 * 2,
    9 * 2,
    10 * 2,
    10 * 2,
    12 * 2,
    12 * 2,
    15 * 2,
    15 * 2,
];

const fn db2(x: f64) -> f64 {
    x * 2.0
}
static KL_TABLE: [f64; 16] = [
    db2(0.000),
    db2(9.000),
    db2(12.000),
    db2(13.875),
    db2(15.000),
    db2(16.125),
    db2(16.875),
    db2(17.625),
    db2(18.000),
    db2(18.750),
    db2(19.125),
    db2(19.500),
    db2(19.875),
    db2(20.250),
    db2(20.625),
    db2(21.000),
];

static NULL_PATCH_VALUE: OpllPatch = OpllPatch {
    tl: 0,
    fb: 0,
    eg: 0,
    ml: 0,
    ar: 0,
    dr: 0,
    sl: 0,
    rr: 0,
    kr: 0,
    kl: 0,
    am: 0,
    pm: 0,
    ws: 0,
};

/// The tables `initializeTables()` makes.
struct Tables {
    tll_table: Vec<[[u32; 4]; 1 << TL_BITS]>,
    rks_table: [[i32; 2]; 8 * 2],
    fullsin_table: [u16; PG_WIDTH],
    halfsin_table: [u16; PG_WIDTH],
    default_patch: [[OpllPatch; (16 + 3) * 2]; OPLL_TONE_NUM],
}

/***************************************************

                  Create tables

****************************************************/

fn make_sin_table(fullsin_table: &mut [u16; PG_WIDTH], halfsin_table: &mut [u16; PG_WIDTH]) {
    fullsin_table[..PG_WIDTH / 4].copy_from_slice(&FULLSIN_QUARTER);

    for x in 0..PG_WIDTH / 4 {
        fullsin_table[PG_WIDTH / 4 + x] = fullsin_table[PG_WIDTH / 4 - x - 1];
    }

    for x in 0..PG_WIDTH / 2 {
        fullsin_table[PG_WIDTH / 2 + x] = 0x8000 | fullsin_table[x];
    }

    halfsin_table[..PG_WIDTH / 2].copy_from_slice(&fullsin_table[..PG_WIDTH / 2]);

    for x in halfsin_table.iter_mut().skip(PG_WIDTH / 2) {
        *x = 0xfff;
    }
}

fn make_tll_table(tll_table: &mut [[[u32; 4]; 1 << TL_BITS]]) {
    for fnum in 0..16usize {
        for block in 0..8usize {
            for tl in 0..64u32 {
                for kl in 0..4usize {
                    let e = &mut tll_table[(block << 4) | fnum][tl as usize][kl];
                    if kl == 0 {
                        *e = tl2eg(tl);
                    } else {
                        let tmp = (KL_TABLE[fnum] - db2(3.000) * (7 - block as i32) as f64) as i32;
                        if tmp <= 0 {
                            *e = tl2eg(tl);
                        } else {
                            *e = ((tmp >> (3 - kl)) as f64 / EG_STEP) as u32 + tl2eg(tl);
                        }
                    }
                }
            }
        }
    }
}

fn make_rks_table(rks_table: &mut [[i32; 2]; 16]) {
    for fnum8 in 0..2i32 {
        for block in 0..8i32 {
            rks_table[((block << 1) | fnum8) as usize][1] = (block << 1) + fnum8;
            rks_table[((block << 1) | fnum8) as usize][0] = block >> 1;
        }
    }
}

fn make_default_patch(default_patch: &mut [[OpllPatch; 38]; OPLL_TONE_NUM]) {
    for (i, patches) in default_patch.iter_mut().enumerate() {
        for j in 0..19 {
            opll_get_default_patch(i as i32, j, &mut patches[j as usize * 2..]);
        }
    }
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut t = Tables {
            tll_table: vec![[[0; 4]; 1 << TL_BITS]; 8 * 16],
            rks_table: [[0; 2]; 16],
            fullsin_table: [0; PG_WIDTH],
            halfsin_table: [0; PG_WIDTH],
            default_patch: [[OpllPatch::default(); 38]; OPLL_TONE_NUM],
        };
        make_tll_table(&mut t.tll_table);
        make_rks_table(&mut t.rks_table);
        make_sin_table(&mut t.fullsin_table, &mut t.halfsin_table);
        make_default_patch(&mut t.default_patch);
        t
    })
}

/*********************************************************

                      Synthesizing

*********************************************************/
const SLOT_BD1: usize = 12;
const SLOT_HH: usize = 14;
const SLOT_SD: usize = 15;
const SLOT_TOM: usize = 16;
const SLOT_CYM: usize = 17;

/* utility macros */
#[inline]
const fn md(x: usize) -> usize {
    x << 1
}
#[inline]
const fn car(x: usize) -> usize {
    (x << 1) | 1
}
#[inline]
const fn bit(s: u32, b: u32) -> u32 {
    (s >> b) & 1
}

const UPDATE_WS: u32 = 1;
const UPDATE_TLL: u32 = 2;
const UPDATE_RKS: u32 = 4;
const UPDATE_EG: u32 = 8;
const UPDATE_ALL: u32 = 255;

/// `OPLL`
#[derive(Clone, Debug)]
pub struct Opll {
    clk: u32,
    rate: u32,
    chip_type: u8,
    adr: u32,
    inp_step: f64,
    out_step: f64,
    out_time: f64,
    reg: [u8; 0x40],
    test_flag: u8,
    slot_key_status: u32,
    rhythm_mode: u8,
    eg_counter: u32,
    pm_phase: u32,
    am_phase: i32,
    lfo_am: u8,
    noise: u32,
    short_noise: u8,
    patch_number: [i32; 9],
    slot: [OpllSlot; 18],
    /// (with `null_patch` at the end)
    patch: [OpllPatch; 19 * 2 + 1],
    pan: [u8; 16],
    pan_fine: [[f32; 2]; 16],
    mask: u32,
    /// channel output
    /// 0..8:tone 9:bd 10:hh 11:sd 12:tom 13:cym
    pub ch_out: [i16; 14],
    mix_out: [i16; 2],
}

#[inline]
fn get_parameter_rate(slot: &OpllSlot, patch: &OpllPatch) -> u32 {
    if (slot.type_ & 1) == 0 && slot.key_flag == 0 {
        return 0;
    }

    match slot.eg_state {
        ATTACK => patch.ar,
        DECAY => patch.dr,
        SUSTAIN => {
            if patch.eg != 0 {
                0
            } else {
                patch.rr
            }
        }
        RELEASE => {
            if slot.sus_flag != 0 {
                5
            } else if patch.eg != 0 {
                patch.rr
            } else {
                7
            }
        }
        DAMP => DAMPER_RATE,
        _ => 0,
    }
}

#[inline]
fn request_update(slot: &mut OpllSlot, flag: u32) {
    slot.update_requests |= flag;
}

fn reset_slot(slot: &mut OpllSlot, number: i32) {
    slot.number = number as u8;
    slot.type_ = (number % 2) as u8;
    slot.pg_keep = 0;
    slot.wave_table = 0;
    slot.pg_phase = 0;
    slot.output[0] = 0;
    slot.output[1] = 0;
    slot.eg_state = RELEASE;
    slot.eg_shift = 0;
    slot.rks = 0;
    slot.tll = 0;
    slot.key_flag = 0;
    slot.sus_flag = 0;
    slot.blk_fnum = 0;
    slot.blk = 0;
    slot.fnum = 0;
    slot.volume = 0;
    slot.pg_out = 0;
    slot.eg_out = EG_MUTE;
    slot.patch = NULL_PATCH;
}

#[inline]
fn lookup_attack_step(slot: &OpllSlot, counter: u32) -> u8 {
    let index;

    match slot.eg_rate_h {
        12 => {
            index = (counter & 0xc) >> 1;
            4 - EG_STEP_TABLES[slot.eg_rate_l as usize][index as usize]
        }
        13 => {
            index = (counter & 0xc) >> 1;
            3 - EG_STEP_TABLES[slot.eg_rate_l as usize][index as usize]
        }
        14 => {
            index = (counter & 0xc) >> 1;
            2 - EG_STEP_TABLES[slot.eg_rate_l as usize][index as usize]
        }
        0 | 15 => 0,
        _ => {
            index = counter >> slot.eg_shift;
            if EG_STEP_TABLES[slot.eg_rate_l as usize][(index & 7) as usize] != 0 {
                4
            } else {
                0
            }
        }
    }
}

#[inline]
fn lookup_decay_step(slot: &OpllSlot, counter: u32) -> u8 {
    let index;

    match slot.eg_rate_h {
        0 => 0,
        13 => {
            index = ((counter & 0xc) >> 1) | (counter & 1);
            EG_STEP_TABLES[slot.eg_rate_l as usize][index as usize]
        }
        14 => {
            index = (counter & 0xc) >> 1;
            EG_STEP_TABLES[slot.eg_rate_l as usize][index as usize] + 1
        }
        15 => 2,
        _ => {
            index = counter >> slot.eg_shift;
            EG_STEP_TABLES[slot.eg_rate_l as usize][(index & 7) as usize]
        }
    }
}

#[inline]
fn start_envelope(slot: &mut OpllSlot, patch: &OpllPatch) {
    if (15u32).min(patch.ar + (slot.rks as u32 >> 2)) == 15 {
        slot.eg_state = DECAY;
        slot.eg_out = 0;
    } else {
        slot.eg_state = ATTACK;
    }
    request_update(slot, UPDATE_EG);
}

/* output: -4095...4095 */
#[inline]
fn lookup_exp_table(i: u16) -> i16 {
    /* from andete's expression */
    let t: i16 = (EXP_TABLE[((i & 0xff) ^ 0xff) as usize] + 1024) as i16;
    let res: i16 = ((t as i32) >> ((i & 0x7f00) >> 8)) as i16;
    (((if (i & 0x8000) != 0 { !res } else { res }) as i32) << 1) as i16
}

#[inline]
fn to_linear(h: u16, slot: &OpllSlot, am: i16) -> i16 {
    if slot.eg_out > EG_MAX {
        return 0;
    }

    let att: u16 =
        ((EG_MUTE as i32).min(slot.eg_out as i32 + slot.tll as i32 + am as i32) << 4) as u16;
    lookup_exp_table(h.wrapping_add(att))
}

/// Specify phase offset directly based on 10-bit (1024-length) sine table
#[inline]
const fn pd(phase: u32) -> u32 {
    phase
}

#[inline]
fn mo(x: i16) -> i16 {
    (-(x as i32) >> 1) as i16
}

impl Opll {
    fn patch_of(&self, slot: &OpllSlot) -> &OpllPatch {
        if slot.patch == NULL_PATCH {
            &NULL_PATCH_VALUE
        } else {
            &self.patch[slot.patch]
        }
    }

    fn wave(&self, slot: &OpllSlot) -> &'static [u16; PG_WIDTH] {
        let t = tables();
        if slot.wave_table == 0 {
            &t.fullsin_table
        } else {
            &t.halfsin_table
        }
    }

    fn commit_slot_update(&mut self, i: usize) {
        let t = tables();
        let patch = *self.patch_of(&self.slot[i]);
        let slot = &mut self.slot[i];

        if slot.update_requests & UPDATE_WS != 0 {
            slot.wave_table = patch.ws as usize;
        }

        if slot.update_requests & UPDATE_TLL != 0 {
            if (slot.type_ & 1) == 0 {
                slot.tll = t.tll_table[(slot.blk_fnum >> 5) as usize][patch.tl as usize]
                    [patch.kl as usize] as u16;
            } else {
                slot.tll = t.tll_table[(slot.blk_fnum >> 5) as usize][slot.volume as usize]
                    [patch.kl as usize] as u16;
            }
        }

        if slot.update_requests & UPDATE_RKS != 0 {
            slot.rks = t.rks_table[(slot.blk_fnum >> 8) as usize][patch.kr as usize] as u8;
        }

        if slot.update_requests & (UPDATE_RKS | UPDATE_EG) != 0 {
            let p_rate = get_parameter_rate(slot, &patch);

            if p_rate == 0 {
                slot.eg_shift = 0;
                slot.eg_rate_h = 0;
                slot.eg_rate_l = 0;
                return;
            }

            slot.eg_rate_h = 15u32.min(p_rate + (slot.rks as u32 >> 2)) as u8;
            slot.eg_rate_l = slot.rks & 3;
            if slot.eg_state == ATTACK {
                slot.eg_shift = if 0 < slot.eg_rate_h && slot.eg_rate_h < 12 {
                    13 - slot.eg_rate_h as u32
                } else {
                    0
                };
            } else {
                slot.eg_shift = if slot.eg_rate_h < 13 {
                    13 - slot.eg_rate_h as u32
                } else {
                    0
                };
            }
        }

        slot.update_requests = 0;
    }

    #[inline]
    fn slot_on(&mut self, i: usize) {
        let slot = &mut self.slot[i];
        slot.key_flag = 1;
        slot.eg_state = DAMP;
        request_update(slot, UPDATE_EG);
    }

    #[inline]
    fn slot_off(&mut self, i: usize) {
        let slot = &mut self.slot[i];
        slot.key_flag = 0;
        if slot.type_ & 1 != 0 {
            slot.eg_state = RELEASE;
            request_update(slot, UPDATE_EG);
        }
    }

    #[inline]
    fn update_key_status(&mut self) {
        let r14 = self.reg[0x0e];
        let rhythm_mode = bit(r14 as u32, 5);
        let mut new_slot_key_status: u32 = 0;

        for ch in 0..9 {
            if self.reg[0x20 + ch] & 0x10 != 0 {
                new_slot_key_status |= 3 << (ch * 2);
            }
        }

        if rhythm_mode != 0 {
            if r14 & 0x10 != 0 {
                new_slot_key_status |= 3 << SLOT_BD1;
            }

            if r14 & 0x01 != 0 {
                new_slot_key_status |= 1 << SLOT_HH;
            }

            if r14 & 0x08 != 0 {
                new_slot_key_status |= 1 << SLOT_SD;
            }

            if r14 & 0x04 != 0 {
                new_slot_key_status |= 1 << SLOT_TOM;
            }

            if r14 & 0x02 != 0 {
                new_slot_key_status |= 1 << SLOT_CYM;
            }
        }

        let updated_status = self.slot_key_status ^ new_slot_key_status;

        if updated_status != 0 {
            for i in 0..18 {
                if bit(updated_status, i) != 0 {
                    if bit(new_slot_key_status, i) != 0 {
                        self.slot_on(i as usize);
                    } else {
                        self.slot_off(i as usize);
                    }
                }
            }
        }

        self.slot_key_status = new_slot_key_status;
    }

    #[inline]
    fn set_patch(&mut self, ch: usize, num: i32) {
        self.patch_number[ch] = num;
        self.slot[md(ch)].patch = (num * 2) as usize;
        self.slot[car(ch)].patch = (num * 2 + 1) as usize;
        request_update(&mut self.slot[md(ch)], UPDATE_ALL);
        request_update(&mut self.slot[car(ch)], UPDATE_ALL);
    }

    #[inline]
    fn set_sus_flag(&mut self, ch: usize, flag: i32) {
        self.slot[car(ch)].sus_flag = flag as u8;
        request_update(&mut self.slot[car(ch)], UPDATE_EG);
        if self.slot[md(ch)].type_ & 1 != 0 {
            self.slot[md(ch)].sus_flag = flag as u8;
            request_update(&mut self.slot[md(ch)], UPDATE_EG);
        }
    }

    /// set volume ( volume : 6bit, register value << 2 )
    #[inline]
    fn set_volume(&mut self, ch: usize, volume: i32) {
        self.slot[car(ch)].volume = volume;
        request_update(&mut self.slot[car(ch)], UPDATE_TLL);
    }

    #[inline]
    fn set_slot_volume(&mut self, i: usize, volume: i32) {
        self.slot[i].volume = volume;
        request_update(&mut self.slot[i], UPDATE_TLL);
    }

    /// set f-Nnmber ( fnum : 9bit )
    #[inline]
    fn set_fnumber(&mut self, ch: usize, fnum: i32) {
        for s in [car(ch), md(ch)] {
            let slot = &mut self.slot[s];
            slot.fnum = fnum as u16;
            slot.blk_fnum = (slot.blk_fnum & 0xe00) | (fnum as u16 & 0x1ff);
            request_update(slot, UPDATE_EG | UPDATE_RKS | UPDATE_TLL);
        }
    }

    /// set block data (blk : 3bit )
    #[inline]
    fn set_block(&mut self, ch: usize, blk: i32) {
        for s in [car(ch), md(ch)] {
            let slot = &mut self.slot[s];
            slot.blk = blk as u8;
            slot.blk_fnum = (((blk & 7) << 9) as u16) | (slot.blk_fnum & 0x1ff);
            request_update(slot, UPDATE_EG | UPDATE_RKS | UPDATE_TLL);
        }
    }

    #[inline]
    fn update_rhythm_mode(&mut self) {
        let new_rhythm_mode = (self.reg[0x0e] >> 5) & 1;

        if self.rhythm_mode != new_rhythm_mode {
            if new_rhythm_mode != 0 {
                self.slot[SLOT_HH].type_ = 3;
                self.slot[SLOT_HH].pg_keep = 1;
                self.slot[SLOT_SD].type_ = 3;
                self.slot[SLOT_TOM].type_ = 3;
                self.slot[SLOT_CYM].type_ = 3;
                self.slot[SLOT_CYM].pg_keep = 1;
                self.set_patch(6, 16);
                self.set_patch(7, 17);
                self.set_patch(8, 18);
                self.set_slot_volume(SLOT_HH, (((self.reg[0x37] >> 4) & 15) as i32) << 2);
                self.set_slot_volume(SLOT_TOM, (((self.reg[0x38] >> 4) & 15) as i32) << 2);
            } else {
                self.slot[SLOT_HH].type_ = 0;
                self.slot[SLOT_HH].pg_keep = 0;
                self.slot[SLOT_SD].type_ = 1;
                self.slot[SLOT_TOM].type_ = 0;
                self.slot[SLOT_CYM].type_ = 1;
                self.slot[SLOT_CYM].pg_keep = 0;
                self.set_patch(6, (self.reg[0x36] >> 4) as i32);
                self.set_patch(7, (self.reg[0x37] >> 4) as i32);
                self.set_patch(8, (self.reg[0x38] >> 4) as i32);
            }
        }

        self.rhythm_mode = new_rhythm_mode;
    }

    fn update_ampm(&mut self) {
        if self.test_flag & 2 != 0 {
            self.pm_phase = 0;
            self.am_phase = 0;
        } else {
            self.pm_phase =
                self.pm_phase
                    .wrapping_add(if self.test_flag & 8 != 0 { 1024 } else { 1 });
            self.am_phase =
                self.am_phase
                    .wrapping_add(if self.test_flag & 8 != 0 { 64 } else { 1 });
        }
        self.lfo_am =
            AM_TABLE[((self.am_phase >> 6) as i64 as u64 % AM_TABLE.len() as u64) as usize];
    }

    fn update_noise(&mut self, cycle: i32) {
        for _ in 0..cycle {
            if self.noise & 1 != 0 {
                self.noise ^= 0x800200;
            }
            self.noise >>= 1;
        }
    }

    fn update_short_noise(&mut self) {
        let pg_hh = self.slot[SLOT_HH].pg_out;
        let pg_cym = self.slot[SLOT_CYM].pg_out;

        let h_bit2 = bit(pg_hh, PG_BITS - 8);
        let h_bit7 = bit(pg_hh, PG_BITS - 3);
        let h_bit3 = bit(pg_hh, PG_BITS - 7);

        let c_bit3 = bit(pg_cym, PG_BITS - 7);
        let c_bit5 = bit(pg_cym, PG_BITS - 5);

        self.short_noise = ((h_bit2 ^ h_bit7) | (h_bit3 ^ c_bit5) | (c_bit3 ^ c_bit5)) as u8;
    }

    #[inline]
    fn calc_phase(&mut self, i: usize, pm_phase: i32, reset: u8) {
        let patch = *self.patch_of(&self.slot[i]);
        let slot = &mut self.slot[i];
        let pm: i8 = if patch.pm != 0 {
            PM_TABLE[((slot.fnum >> 6) & 7) as usize][((pm_phase >> 10) & 7) as usize]
        } else {
            0
        };
        if reset != 0 {
            slot.pg_phase = 0;
        }
        let inc = (((slot.fnum as i32 & 0x1ff) * 2 + pm as i32) as u32)
            .wrapping_mul(ML_TABLE[patch.ml as usize]);
        slot.pg_phase = slot
            .pg_phase
            .wrapping_add(inc.wrapping_shl(slot.blk as u32) >> 2);
        slot.pg_phase &= DP_WIDTH - 1;
        slot.pg_out = slot.pg_phase >> DP_BASE_BITS;
    }

    #[inline]
    fn calc_envelope(&mut self, i: usize, buddy: Option<usize>, eg_counter: u16, test: u8) {
        let patch = *self.patch_of(&self.slot[i]);
        let slot = &mut self.slot[i];
        let mask: u32 = (1u32 << slot.eg_shift) - 1;
        let s: u8;

        if slot.eg_state == ATTACK {
            if 0 < slot.eg_out && 0 < slot.eg_rate_h && (eg_counter as u32 & mask & !3) == 0 {
                s = lookup_attack_step(slot, eg_counter as u32);
                if 0 < s {
                    slot.eg_out =
                        0i32.max(slot.eg_out as i32 - (slot.eg_out >> s) as i32 - 1) as u32;
                }
            }
        } else if slot.eg_rate_h > 0 && (eg_counter as u32 & mask) == 0 {
            slot.eg_out = (EG_MUTE as i32)
                .min((slot.eg_out + lookup_decay_step(slot, eg_counter as u32) as u32) as i32)
                as u32;
        }

        match slot.eg_state {
            DAMP => {
                // DAMP to ATTACK transition is occured when the envelope reaches EG_MAX (max attenuation but it's not mute).
                // Do not forget to check (eg_counter & mask) == 0 to synchronize it with the progress of the envelope.
                if slot.eg_out >= EG_MAX && (eg_counter as u32 & mask) == 0 {
                    start_envelope(slot, &patch);
                    if slot.type_ & 1 != 0 {
                        if slot.pg_keep == 0 {
                            slot.pg_phase = 0;
                        }
                        if let Some(b) = buddy {
                            if self.slot[b].pg_keep == 0 {
                                self.slot[b].pg_phase = 0;
                            }
                        }
                    }
                }
            }

            ATTACK => {
                if slot.eg_out == 0 {
                    slot.eg_state = DECAY;
                    request_update(slot, UPDATE_EG);
                }
            }

            DECAY => {
                // DECAY to SUSTAIN transition must be checked at every cycle regardless of the conditions of the envelope rate and
                // counter. i.e. the transition is not synchronized with the progress of the envelope.
                if (slot.eg_out >> 3) == patch.sl {
                    slot.eg_state = SUSTAIN;
                    request_update(slot, UPDATE_EG);
                }
            }

            _ => {}
        }

        if test != 0 {
            self.slot[i].eg_out = 0;
        }
    }

    fn update_slots(&mut self) {
        self.eg_counter = self.eg_counter.wrapping_add(1);

        for i in 0..18 {
            let mut buddy = None;
            if self.slot[i].type_ == 0 {
                buddy = Some(i + 1);
            }
            if self.slot[i].type_ == 1 {
                buddy = Some(i - 1);
            }
            if self.slot[i].update_requests != 0 {
                self.commit_slot_update(i);
            }
            self.calc_envelope(i, buddy, self.eg_counter as u16, self.test_flag & 1);
            self.calc_phase(i, self.pm_phase as i32, self.test_flag & 4);
        }
    }

    #[inline]
    fn calc_slot_car(&mut self, ch: usize, fm: i16) -> i16 {
        let i = car(ch);
        let am: u8 = if self.patch_of(&self.slot[i]).am != 0 {
            self.lfo_am
        } else {
            0
        };
        let wave = self.wave(&self.slot[i]);
        let slot = &mut self.slot[i];

        slot.output[1] = slot.output[0];
        let idx = (slot.pg_out as i32 + 2 * (fm as i32 >> 1)) as u32 as usize & (PG_WIDTH - 1);
        slot.output[0] = to_linear(wave[idx], slot, am as i16) as i32;

        slot.output[0] as i16
    }

    #[inline]
    fn calc_slot_mod(&mut self, ch: usize) -> i16 {
        let i = md(ch);
        let patch = *self.patch_of(&self.slot[i]);
        let wave = self.wave(&self.slot[i]);
        let slot = &mut self.slot[i];

        let fm: i16 = if patch.fb > 0 {
            ((slot.output[1] + slot.output[0]) >> (9 - patch.fb)) as i16
        } else {
            0
        };
        let am: u8 = if patch.am != 0 { self.lfo_am } else { 0 };

        slot.output[1] = slot.output[0];
        let idx = (slot.pg_out as i32 + fm as i32) as u32 as usize & (PG_WIDTH - 1);
        slot.output[0] = to_linear(wave[idx], slot, am as i16) as i32;

        slot.output[0] as i16
    }

    #[inline]
    fn calc_slot_tom(&self) -> i16 {
        let slot = &self.slot[md(8)];

        to_linear(self.wave(slot)[slot.pg_out as usize], slot, 0)
    }

    #[inline]
    fn calc_slot_snare(&self) -> i16 {
        let slot = &self.slot[car(7)];

        let phase: u32 = if bit(slot.pg_out, PG_BITS - 2) != 0 {
            if self.noise & 1 != 0 {
                pd(0x300)
            } else {
                pd(0x200)
            }
        } else if self.noise & 1 != 0 {
            pd(0x0)
        } else {
            pd(0x100)
        };

        to_linear(self.wave(slot)[phase as usize], slot, 0)
    }

    #[inline]
    fn calc_slot_cym(&self) -> i16 {
        let slot = &self.slot[car(8)];

        let phase: u32 = if self.short_noise != 0 {
            pd(0x300)
        } else {
            pd(0x100)
        };

        to_linear(self.wave(slot)[phase as usize], slot, 0)
    }

    #[inline]
    fn calc_slot_hat(&self) -> i16 {
        let slot = &self.slot[md(7)];

        let phase: u32 = if self.short_noise != 0 {
            if self.noise & 1 != 0 {
                pd(0x2d0)
            } else {
                pd(0x234)
            }
        } else if self.noise & 1 != 0 {
            pd(0x34)
        } else {
            pd(0xd0)
        };

        to_linear(self.wave(slot)[phase as usize], slot, 0)
    }

    fn update_output(&mut self) {
        self.update_ampm();
        self.update_short_noise();
        self.update_slots();

        /* CH1-6 */
        for i in 0..6 {
            if self.mask & (1 << i) == 0 {
                let m = self.calc_slot_mod(i);
                self.ch_out[i] = mo(self.calc_slot_car(i, m));
            }
        }

        /* CH7 */
        if self.rhythm_mode == 0 {
            if self.mask & (1 << 6) == 0 {
                let m = self.calc_slot_mod(6);
                self.ch_out[6] = mo(self.calc_slot_car(6, m));
            }
        } else if self.mask & OPLL_MASK_BD == 0 {
            let m = self.calc_slot_mod(6);
            self.ch_out[9] = self.calc_slot_car(6, m);
        }
        self.update_noise(14);

        /* CH8 */
        if self.rhythm_mode == 0 {
            if self.mask & (1 << 7) == 0 {
                let m = self.calc_slot_mod(7);
                self.ch_out[7] = mo(self.calc_slot_car(7, m));
            }
        } else {
            if self.mask & OPLL_MASK_HH == 0 {
                self.ch_out[10] = self.calc_slot_hat();
            }
            if self.mask & OPLL_MASK_SD == 0 {
                self.ch_out[11] = self.calc_slot_snare();
            }
        }
        self.update_noise(2);

        /* CH9 */
        if self.rhythm_mode == 0 {
            if self.mask & (1 << 8) == 0 {
                let m = self.calc_slot_mod(8);
                self.ch_out[8] = mo(self.calc_slot_car(8, m));
            }
        } else {
            if self.mask & OPLL_MASK_TOM == 0 {
                self.ch_out[12] = self.calc_slot_tom();
            }
            if self.mask & OPLL_MASK_CYM == 0 {
                self.ch_out[13] = self.calc_slot_cym();
            }
        }
        self.update_noise(2);
    }

    #[inline]
    fn mix_output_stereo(&mut self) {
        let mut out = [0i16; 2];
        for i in 0..14 {
            if self.pan[i] & 2 != 0 {
                out[0] = out[0].wrapping_add((self.ch_out[i] as f32 * self.pan_fine[i][0]) as i16);
            }
            if self.pan[i] & 1 != 0 {
                out[1] = out[1].wrapping_add((self.ch_out[i] as f32 * self.pan_fine[i][1]) as i16);
            }
        }
        self.mix_out = out;
        // (no rate converter: see OPLL_new())
    }

    /***********************************************************

                       External Interfaces

    ***********************************************************/

    /// `OPLL_new()`
    pub fn new(clk: u32, rate: u32) -> Box<Opll> {
        tables();

        let mut opll = Box::new(Opll {
            clk,
            rate,
            chip_type: 0,
            adr: 0,
            inp_step: 0.0,
            out_step: 0.0,
            out_time: 0.0,
            reg: [0; 0x40],
            test_flag: 0,
            slot_key_status: 0,
            rhythm_mode: 0,
            eg_counter: 0,
            pm_phase: 0,
            am_phase: 0,
            lfo_am: 0,
            noise: 0,
            short_noise: 0,
            patch_number: [0; 9],
            slot: [OpllSlot::default(); 18],
            patch: [NULL_PATCH_VALUE; 19 * 2 + 1],
            pan: [0; 16],
            pan_fine: [[0.0; 2]; 16],
            mask: 0,
            ch_out: [0; 14],
            mix_out: [0; 2],
        });

        opll.reset();
        opll.set_chip_type(0);
        opll.reset_patch(0);
        opll
    }

    fn reset_rate_conversion_params(&mut self) {
        let f_out = self.rate as f64;
        let f_inp = self.clk as f64 / 72.0;

        self.out_time = 0.0;
        self.out_step = f_inp;
        self.inp_step = f_out;

        // The internal rate converter (OPLL_RateConv) is left out: it is
        // only made when the rate isn't the chip's own (clock / 72), and
        // the VRC7 asks for that one.
        debug_assert!(
            !(sdl3::stdlib::math::floor(f_inp) != f_out
                && sdl3::stdlib::math::floor(f_inp + 0.5) != f_out)
        );
    }

    /// `OPLL_reset()`
    pub fn reset(&mut self) {
        self.adr = 0;

        self.pm_phase = 0;
        self.am_phase = 0;

        self.noise = 0x1;
        self.mask = 0;

        self.rhythm_mode = 0;
        self.slot_key_status = 0;
        self.eg_counter = 0;

        self.reset_rate_conversion_params();

        for i in 0..18 {
            reset_slot(&mut self.slot[i], i as i32);
        }

        for i in 0..9 {
            self.set_patch(i, 0);
        }

        for i in 0..0x40 {
            self.write_reg(i, 0);
        }

        for i in 0..15 {
            self.pan[i] = 3;
            self.pan_fine[i][1] = 1.0;
            self.pan_fine[i][0] = 1.0;
        }

        self.ch_out = [0; 14];
    }

    /// `OPLL_setChipType()`: Set chip type. If vrc7 is selected, r#14 is ignored.
    /// This method not change the current ROM patch set.
    /// To change ROM patch set, use OPLL_resetPatch.
    /// @param type 0:YM2413 1:VRC7
    pub fn set_chip_type(&mut self, type_: u8) {
        self.chip_type = type_;
    }

    /// `OPLL_writeReg()`
    pub fn write_reg(&mut self, mut reg: u32, data: u8) {
        /* mirror registers */
        if (0x19..=0x1f).contains(&reg)
            || (0x29..=0x2f).contains(&reg)
            || (0x39..=0x3f).contains(&reg)
        {
            reg -= 9;
        }

        /* The boundary check must be placed at the previous line to the array
        assignment to avoid the false -Wstringop-overflow warning. */
        if reg >= 0x40 {
            return;
        }

        self.reg[reg as usize] = data;

        macro_rules! update_user {
            ($slot:expr, $flags:expr) => {
                for i in 0..9 {
                    if self.patch_number[i] == 0 {
                        request_update(&mut self.slot[$slot(i)], $flags);
                    }
                }
            };
        }

        let data32 = data as u32;
        match reg {
            0x00 => {
                self.patch[0].am = (data32 >> 7) & 1;
                self.patch[0].pm = (data32 >> 6) & 1;
                self.patch[0].eg = (data32 >> 5) & 1;
                self.patch[0].kr = (data32 >> 4) & 1;
                self.patch[0].ml = data32 & 15;
                update_user!(md, UPDATE_RKS | UPDATE_EG);
            }

            0x01 => {
                self.patch[1].am = (data32 >> 7) & 1;
                self.patch[1].pm = (data32 >> 6) & 1;
                self.patch[1].eg = (data32 >> 5) & 1;
                self.patch[1].kr = (data32 >> 4) & 1;
                self.patch[1].ml = data32 & 15;
                update_user!(car, UPDATE_RKS | UPDATE_EG);
            }

            0x02 => {
                self.patch[0].kl = (data32 >> 6) & 3;
                self.patch[0].tl = data32 & 63;
                update_user!(md, UPDATE_TLL);
            }

            0x03 => {
                self.patch[1].kl = (data32 >> 6) & 3;
                self.patch[1].ws = (data32 >> 4) & 1;
                self.patch[0].ws = (data32 >> 3) & 1;
                self.patch[0].fb = data32 & 7;
                for i in 0..9 {
                    if self.patch_number[i] == 0 {
                        request_update(&mut self.slot[md(i)], UPDATE_WS);
                        request_update(&mut self.slot[car(i)], UPDATE_WS | UPDATE_TLL);
                    }
                }
            }

            0x04 => {
                self.patch[0].ar = (data32 >> 4) & 15;
                self.patch[0].dr = data32 & 15;
                update_user!(md, UPDATE_EG);
            }

            0x05 => {
                self.patch[1].ar = (data32 >> 4) & 15;
                self.patch[1].dr = data32 & 15;
                update_user!(car, UPDATE_EG);
            }

            0x06 => {
                self.patch[0].sl = (data32 >> 4) & 15;
                self.patch[0].rr = data32 & 15;
                update_user!(md, UPDATE_EG);
            }

            0x07 => {
                self.patch[1].sl = (data32 >> 4) & 15;
                self.patch[1].rr = data32 & 15;
                update_user!(car, UPDATE_EG);
            }

            0x0e => {
                if self.chip_type == 1 {
                    return;
                }
                self.update_rhythm_mode();
                self.update_key_status();
            }

            0x0f => {
                self.test_flag = data;
            }

            0x10..=0x18 => {
                let ch = (reg - 0x10) as usize;
                self.set_fnumber(ch, data as i32 + ((self.reg[0x20 + ch] as i32 & 1) << 8));
            }

            0x20..=0x28 => {
                let ch = (reg - 0x20) as usize;
                self.set_fnumber(ch, ((data as i32 & 1) << 8) + self.reg[0x10 + ch] as i32);
                self.set_block(ch, (data as i32 >> 1) & 7);
                self.set_sus_flag(ch, (data as i32 >> 5) & 1);
                self.update_key_status();
            }

            0x30..=0x38 => {
                if (self.reg[0x0e] & 32) != 0 && (reg >= 0x36) {
                    match reg {
                        0x37 => self.set_slot_volume(md(7), (((data >> 4) & 15) as i32) << 2),
                        0x38 => self.set_slot_volume(md(8), (((data >> 4) & 15) as i32) << 2),
                        _ => {}
                    }
                } else {
                    self.set_patch((reg - 0x30) as usize, ((data >> 4) & 15) as i32);
                }
                self.set_volume((reg - 0x30) as usize, ((data & 15) as i32) << 2);
            }

            _ => {}
        }
    }

    /// `OPLL_writeIO()`
    pub fn write_io(&mut self, adr: u32, val: u8) {
        if adr & 1 != 0 {
            let a = self.adr;
            self.write_reg(a, val);
        } else {
            self.adr = val as u32;
        }
    }

    /// `OPLL_copyPatch()`
    fn copy_patch(&mut self, num: usize, patch: &OpllPatch) {
        self.patch[num] = *patch;
    }

    /// `OPLL_resetPatch()`
    pub fn reset_patch(&mut self, type_: u8) {
        let t = tables();
        for i in 0..19 * 2 {
            self.copy_patch(i, &t.default_patch[type_ as usize % OPLL_TONE_NUM][i]);
        }
    }

    /// `OPLL_calcStereo()`: Calulate stereo sample
    pub fn calc_stereo(&mut self, out: &mut [i32; 2]) {
        while self.out_step > self.out_time {
            self.out_time += self.inp_step;
            self.update_output();
            self.mix_output_stereo();
        }
        self.out_time -= self.out_step;
        out[0] = self.mix_out[0] as i32;
        out[1] = self.mix_out[1] as i32;
    }
}

/* mask */
const OPLL_MASK_HH: u32 = 1 << 9;
const OPLL_MASK_CYM: u32 = 1 << 10;
const OPLL_MASK_TOM: u32 = 1 << 11;
const OPLL_MASK_SD: u32 = 1 << 12;
const OPLL_MASK_BD: u32 = 1 << 13;

/// `OPLL_dumpToPatch()`
fn opll_dump_to_patch(dump: &[u8], patch: &mut [OpllPatch]) {
    if dump.len() < 8 {
        return;
    }
    let d = |i: usize| dump[i] as u32;
    patch[0].am = (d(0) >> 7) & 1;
    patch[1].am = (d(1) >> 7) & 1;
    patch[0].pm = (d(0) >> 6) & 1;
    patch[1].pm = (d(1) >> 6) & 1;
    patch[0].eg = (d(0) >> 5) & 1;
    patch[1].eg = (d(1) >> 5) & 1;
    patch[0].kr = (d(0) >> 4) & 1;
    patch[1].kr = (d(1) >> 4) & 1;
    patch[0].ml = d(0) & 15;
    patch[1].ml = d(1) & 15;
    patch[0].kl = (d(2) >> 6) & 3;
    patch[1].kl = (d(3) >> 6) & 3;
    patch[0].tl = d(2) & 63;
    patch[1].tl = 0;
    patch[0].fb = d(3) & 7;
    patch[1].fb = 0;
    patch[0].ws = (d(3) >> 3) & 1;
    patch[1].ws = (d(3) >> 4) & 1;
    patch[0].ar = (d(4) >> 4) & 15;
    patch[1].ar = (d(5) >> 4) & 15;
    patch[0].dr = d(4) & 15;
    patch[1].dr = d(5) & 15;
    patch[0].sl = (d(6) >> 4) & 15;
    patch[1].sl = (d(7) >> 4) & 15;
    patch[0].rr = d(6) & 15;
    patch[1].rr = d(7) & 15;
}

/// `OPLL_getDefaultPatch()`
fn opll_get_default_patch(type_: i32, num: i32, patch: &mut [OpllPatch]) {
    if type_ < 0 || type_ >= OPLL_TONE_NUM as i32 || !(0..19).contains(&num) {
        return;
    }
    opll_dump_to_patch(
        &DEFAULT_INST[type_ as usize][num as usize * 8..num as usize * 8 + 8],
        patch,
    );
}
