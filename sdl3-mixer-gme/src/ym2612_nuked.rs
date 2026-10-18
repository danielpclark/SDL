// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/

// Based on Nuked OPN2 ym3438.c and ym3438.h

/*
 * Copyright (C) 2017 Alexey Khokholov (Nuke.YKT)
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, write to the Free Software
 * Foundation, Inc., 51 Franklin Street, Fifth Floor, Boston, MA 02110-1301 USA
 *
 *
 *  Nuked OPN2(Yamaha YM3438) emulator.
 *  Thanks:
 *      Silicon Pr0n:
 *          Yamaha YM3438 decap and die shot(digshadow).
 *      OPLx decapsulated(Matthew Gambrell, Olli Niemitalo):
 *          OPL2 ROMs.
 *
 * version: 1.0.7
 */
// Modified 2026-10-18: translated into Rust from SDL_mixer 3.3.0's Game_Music_Emu 0.6.5 (see NOTICE).

//! YM2612 FM sound chip emulator interface (`Ym2612_Emu.h`, which picks
//! the Nuked OPN2 core as SDL_mixer's build does, `VGM_YM2612_NUKED`;
//! `Ym2612_Nuked.h`, `Ym2612_Nuked.cpp`). The `OPN2_*` functions are
//! the `opn2_*` functions here, on a [`Ym3438`].

use std::sync::atomic::{AtomicU32, Ordering};

use crate::blargg_common::BlarggErr;

/*EXTRA*/
const RSM_FRAC: u32 = 10;
const OPN_WRITEBUF_SIZE: usize = 2048;
const OPN_WRITEBUF_DELAY: u64 = 15;

/// Discrete YM3438 (Teradrive)
pub const YM3438_TYPE_DISCRETE: u32 = 0;
/// ASIC YM3438 (MD1 VA7, MD2, MD3, etc)
pub const YM3438_TYPE_ASIC: u32 = 1;
/// YM2612 (MD1, MD2 VA2)
pub const YM3438_TYPE_YM2612: u32 = 2;

/*EXTRA*/
#[derive(Clone, Copy, Debug, Default)]
struct Opn2Writebuf {
    time: u64,
    port: u8,
    data: u8,
    // reserved[6]
}

/// The chip's write buffer (`writebuf[OPN_WRITEBUF_SIZE]`)
#[derive(Clone, Debug)]
struct Writebuf(Vec<Opn2Writebuf>);

impl Default for Writebuf {
    fn default() -> Self {
        Writebuf(vec![Opn2Writebuf::default(); OPN_WRITEBUF_SIZE])
    }
}

/// `ym3438_t`: the chip's state.
#[derive(Clone, Default)]
pub struct Ym3438 {
    cycles: u32,
    channel: u32,
    mol: i16,
    mor: i16,
    /* IO */
    write_data: u16,
    write_a: u8,
    write_d: u8,
    write_a_en: u8,
    write_d_en: u8,
    write_busy: u8,
    write_busy_cnt: u8,
    write_fm_address: u8,
    write_fm_data: u8,
    write_fm_mode_a: u8,
    address: u16,
    data: u8,
    pin_test_in: u8,
    #[allow(dead_code)] // (never read, as upstream)
    pin_irq: u8,
    busy: u8,
    /* LFO */
    lfo_en: u8,
    lfo_freq: u8,
    lfo_pm: u8,
    lfo_am: u8,
    lfo_cnt: u8,
    lfo_inc: u8,
    lfo_quotient: u8,
    /* Phase generator */
    pg_fnum: u16,
    pg_block: u8,
    pg_kcode: u8,
    pg_inc: [u32; 24],
    pg_phase: [u32; 24],
    pg_reset: [u8; 24],
    pg_read: u32,
    /* Envelope generator */
    eg_cycle: u8,
    eg_cycle_stop: u8,
    eg_shift: u8,
    eg_shift_lock: u8,
    eg_timer_low_lock: u8,
    eg_timer: u16,
    eg_timer_inc: u8,
    eg_quotient: u16,
    eg_custom_timer: u8,
    eg_rate: u8,
    eg_ksv: u8,
    eg_inc: u8,
    eg_ratemax: u8,
    eg_sl: [u8; 2],
    eg_lfo_am: u8,
    eg_tl: [u8; 2],
    eg_state: [u8; 24],
    eg_level: [u16; 24],
    eg_out: [u16; 24],
    eg_kon: [u8; 24],
    eg_kon_csm: [u8; 24],
    eg_kon_latch: [u8; 24],
    #[allow(dead_code)] // (never read, as upstream)
    eg_csm_mode: [u8; 24],
    eg_ssg_enable: [u8; 24],
    eg_ssg_pgrst_latch: [u8; 24],
    eg_ssg_repeat_latch: [u8; 24],
    eg_ssg_hold_up_latch: [u8; 24],
    eg_ssg_dir: [u8; 24],
    eg_ssg_inv: [u8; 24],
    eg_read: [u32; 2],
    eg_read_inc: u8,
    /* FM */
    fm_op1: [[i16; 2]; 6],
    fm_op2: [i16; 6],
    fm_out: [i16; 24],
    fm_mod: [u16; 24],
    /* Channel */
    ch_acc: [i16; 6],
    ch_out: [i16; 6],
    ch_lock: i16,
    ch_lock_l: u8,
    ch_lock_r: u8,
    ch_read: i16,
    /* Timer */
    timer_a_cnt: u16,
    timer_a_reg: u16,
    timer_a_load_lock: u8,
    timer_a_load: u8,
    timer_a_enable: u8,
    timer_a_reset: u8,
    timer_a_load_latch: u8,
    timer_a_overflow_flag: u8,
    timer_a_overflow: u8,

    timer_b_cnt: u16,
    timer_b_subcnt: u8,
    timer_b_reg: u16,
    timer_b_load_lock: u8,
    timer_b_load: u8,
    timer_b_enable: u8,
    timer_b_reset: u8,
    timer_b_load_latch: u8,
    timer_b_overflow_flag: u8,
    timer_b_overflow: u8,

    /* Register set */
    mode_test_21: [u8; 8],
    mode_test_2c: [u8; 8],
    mode_ch3: u8,
    mode_kon_channel: u8,
    mode_kon_operator: [u8; 4],
    mode_kon: [u8; 24],
    mode_csm: u8,
    mode_kon_csm: u8,
    dacen: u8,
    dacdata: i16,

    ks: [u8; 24],
    ar: [u8; 24],
    sr: [u8; 24],
    dt: [u8; 24],
    multi: [u8; 24],
    sl: [u8; 24],
    rr: [u8; 24],
    dr: [u8; 24],
    am: [u8; 24],
    tl: [u8; 24],
    ssg_eg: [u8; 24],

    fnum: [u16; 6],
    block: [u8; 6],
    kcode: [u8; 6],
    fnum_3ch: [u16; 6],
    block_3ch: [u8; 6],
    kcode_3ch: [u8; 6],
    reg_a4: u8,
    reg_ac: u8,
    connect: [u8; 6],
    fb: [u8; 6],
    pan_l: [u8; 6],
    pan_r: [u8; 6],
    ams: [u8; 6],
    pms: [u8; 6],

    /*EXTRA*/
    mute: [u32; 7],
    rateratio: i32,
    samplecnt: i32,
    oldsamples: [i32; 2],
    samples: [i32; 2],

    writebuf_samplecnt: u64,
    writebuf_cur: u32,
    writebuf_last: u32,
    writebuf_lasttime: u64,
    writebuf: Writebuf,
}

impl std::fmt::Debug for Ym3438 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ym3438")
            .field("cycles", &self.cycles)
            .field("rateratio", &self.rateratio)
            .finish_non_exhaustive()
    }
}

const EG_NUM_ATTACK: u8 = 0;
const EG_NUM_DECAY: u8 = 1;
const EG_NUM_SUSTAIN: u8 = 2;
const EG_NUM_RELEASE: u8 = 3;

/* logsin table */
static LOGSINROM: [u16; 256] = [
    0x859, 0x6c3, 0x607, 0x58b, 0x52e, 0x4e4, 0x4a6, 0x471, 0x443, 0x41a, 0x3f5, 0x3d3, 0x3b5,
    0x398, 0x37e, 0x365, 0x34e, 0x339, 0x324, 0x311, 0x2ff, 0x2ed, 0x2dc, 0x2cd, 0x2bd, 0x2af,
    0x2a0, 0x293, 0x286, 0x279, 0x26d, 0x261, 0x256, 0x24b, 0x240, 0x236, 0x22c, 0x222, 0x218,
    0x20f, 0x206, 0x1fd, 0x1f5, 0x1ec, 0x1e4, 0x1dc, 0x1d4, 0x1cd, 0x1c5, 0x1be, 0x1b7, 0x1b0,
    0x1a9, 0x1a2, 0x19b, 0x195, 0x18f, 0x188, 0x182, 0x17c, 0x177, 0x171, 0x16b, 0x166, 0x160,
    0x15b, 0x155, 0x150, 0x14b, 0x146, 0x141, 0x13c, 0x137, 0x133, 0x12e, 0x129, 0x125, 0x121,
    0x11c, 0x118, 0x114, 0x10f, 0x10b, 0x107, 0x103, 0x0ff, 0x0fb, 0x0f8, 0x0f4, 0x0f0, 0x0ec,
    0x0e9, 0x0e5, 0x0e2, 0x0de, 0x0db, 0x0d7, 0x0d4, 0x0d1, 0x0cd, 0x0ca, 0x0c7, 0x0c4, 0x0c1,
    0x0be, 0x0bb, 0x0b8, 0x0b5, 0x0b2, 0x0af, 0x0ac, 0x0a9, 0x0a7, 0x0a4, 0x0a1, 0x09f, 0x09c,
    0x099, 0x097, 0x094, 0x092, 0x08f, 0x08d, 0x08a, 0x088, 0x086, 0x083, 0x081, 0x07f, 0x07d,
    0x07a, 0x078, 0x076, 0x074, 0x072, 0x070, 0x06e, 0x06c, 0x06a, 0x068, 0x066, 0x064, 0x062,
    0x060, 0x05e, 0x05c, 0x05b, 0x059, 0x057, 0x055, 0x053, 0x052, 0x050, 0x04e, 0x04d, 0x04b,
    0x04a, 0x048, 0x046, 0x045, 0x043, 0x042, 0x040, 0x03f, 0x03e, 0x03c, 0x03b, 0x039, 0x038,
    0x037, 0x035, 0x034, 0x033, 0x031, 0x030, 0x02f, 0x02e, 0x02d, 0x02b, 0x02a, 0x029, 0x028,
    0x027, 0x026, 0x025, 0x024, 0x023, 0x022, 0x021, 0x020, 0x01f, 0x01e, 0x01d, 0x01c, 0x01b,
    0x01a, 0x019, 0x018, 0x017, 0x017, 0x016, 0x015, 0x014, 0x014, 0x013, 0x012, 0x011, 0x011,
    0x010, 0x00f, 0x00f, 0x00e, 0x00d, 0x00d, 0x00c, 0x00c, 0x00b, 0x00a, 0x00a, 0x009, 0x009,
    0x008, 0x008, 0x007, 0x007, 0x007, 0x006, 0x006, 0x005, 0x005, 0x005, 0x004, 0x004, 0x004,
    0x003, 0x003, 0x003, 0x002, 0x002, 0x002, 0x002, 0x001, 0x001, 0x001, 0x001, 0x001, 0x001,
    0x001, 0x000, 0x000, 0x000, 0x000, 0x000, 0x000, 0x000, 0x000,
];

/* exp table */
static EXPROM: [u16; 256] = [
    0x000, 0x003, 0x006, 0x008, 0x00b, 0x00e, 0x011, 0x014, 0x016, 0x019, 0x01c, 0x01f, 0x022,
    0x025, 0x028, 0x02a, 0x02d, 0x030, 0x033, 0x036, 0x039, 0x03c, 0x03f, 0x042, 0x045, 0x048,
    0x04b, 0x04e, 0x051, 0x054, 0x057, 0x05a, 0x05d, 0x060, 0x063, 0x066, 0x069, 0x06c, 0x06f,
    0x072, 0x075, 0x078, 0x07b, 0x07e, 0x082, 0x085, 0x088, 0x08b, 0x08e, 0x091, 0x094, 0x098,
    0x09b, 0x09e, 0x0a1, 0x0a4, 0x0a8, 0x0ab, 0x0ae, 0x0b1, 0x0b5, 0x0b8, 0x0bb, 0x0be, 0x0c2,
    0x0c5, 0x0c8, 0x0cc, 0x0cf, 0x0d2, 0x0d6, 0x0d9, 0x0dc, 0x0e0, 0x0e3, 0x0e7, 0x0ea, 0x0ed,
    0x0f1, 0x0f4, 0x0f8, 0x0fb, 0x0ff, 0x102, 0x106, 0x109, 0x10c, 0x110, 0x114, 0x117, 0x11b,
    0x11e, 0x122, 0x125, 0x129, 0x12c, 0x130, 0x134, 0x137, 0x13b, 0x13e, 0x142, 0x146, 0x149,
    0x14d, 0x151, 0x154, 0x158, 0x15c, 0x160, 0x163, 0x167, 0x16b, 0x16f, 0x172, 0x176, 0x17a,
    0x17e, 0x181, 0x185, 0x189, 0x18d, 0x191, 0x195, 0x199, 0x19c, 0x1a0, 0x1a4, 0x1a8, 0x1ac,
    0x1b0, 0x1b4, 0x1b8, 0x1bc, 0x1c0, 0x1c4, 0x1c8, 0x1cc, 0x1d0, 0x1d4, 0x1d8, 0x1dc, 0x1e0,
    0x1e4, 0x1e8, 0x1ec, 0x1f0, 0x1f5, 0x1f9, 0x1fd, 0x201, 0x205, 0x209, 0x20e, 0x212, 0x216,
    0x21a, 0x21e, 0x223, 0x227, 0x22b, 0x230, 0x234, 0x238, 0x23c, 0x241, 0x245, 0x249, 0x24e,
    0x252, 0x257, 0x25b, 0x25f, 0x264, 0x268, 0x26d, 0x271, 0x276, 0x27a, 0x27f, 0x283, 0x288,
    0x28c, 0x291, 0x295, 0x29a, 0x29e, 0x2a3, 0x2a8, 0x2ac, 0x2b1, 0x2b5, 0x2ba, 0x2bf, 0x2c4,
    0x2c8, 0x2cd, 0x2d2, 0x2d6, 0x2db, 0x2e0, 0x2e5, 0x2e9, 0x2ee, 0x2f3, 0x2f8, 0x2fd, 0x302,
    0x306, 0x30b, 0x310, 0x315, 0x31a, 0x31f, 0x324, 0x329, 0x32e, 0x333, 0x338, 0x33d, 0x342,
    0x347, 0x34c, 0x351, 0x356, 0x35b, 0x360, 0x365, 0x36a, 0x370, 0x375, 0x37a, 0x37f, 0x384,
    0x38a, 0x38f, 0x394, 0x399, 0x39f, 0x3a4, 0x3a9, 0x3ae, 0x3b4, 0x3b9, 0x3bf, 0x3c4, 0x3c9,
    0x3cf, 0x3d4, 0x3da, 0x3df, 0x3e4, 0x3ea, 0x3ef, 0x3f5, 0x3fa,
];

/* Note table */
static FN_NOTE: [u32; 16] = [0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 3, 3, 3, 3, 3, 3];

/* Envelope generator */
static EG_STEPHI: [[u32; 4]; 4] = [[0, 0, 0, 0], [1, 0, 0, 0], [1, 0, 1, 0], [1, 1, 1, 0]];

static EG_AM_SHIFT: [u8; 4] = [7, 3, 1, 0];

/* Phase generator */
static PG_DETUNE: [u32; 8] = [16, 17, 19, 20, 22, 24, 27, 29];

static PG_LFO_SH1: [[u32; 8]; 8] = [
    [7, 7, 7, 7, 7, 7, 7, 7],
    [7, 7, 7, 7, 7, 7, 7, 7],
    [7, 7, 7, 7, 7, 7, 1, 1],
    [7, 7, 7, 7, 1, 1, 1, 1],
    [7, 7, 7, 1, 1, 1, 1, 0],
    [7, 7, 1, 1, 0, 0, 0, 0],
    [7, 7, 1, 1, 0, 0, 0, 0],
    [7, 7, 1, 1, 0, 0, 0, 0],
];

static PG_LFO_SH2: [[u32; 8]; 8] = [
    [7, 7, 7, 7, 7, 7, 7, 7],
    [7, 7, 7, 7, 2, 2, 2, 2],
    [7, 7, 7, 2, 2, 2, 7, 7],
    [7, 7, 2, 2, 7, 7, 2, 2],
    [7, 7, 2, 7, 7, 7, 2, 7],
    [7, 7, 7, 2, 7, 7, 2, 1],
    [7, 7, 7, 2, 7, 7, 2, 1],
    [7, 7, 7, 2, 7, 7, 2, 1],
];

/* Address decoder */
static OP_OFFSET: [u32; 12] = [
    0x000, /* Ch1 OP1/OP2 */
    0x001, /* Ch2 OP1/OP2 */
    0x002, /* Ch3 OP1/OP2 */
    0x100, /* Ch4 OP1/OP2 */
    0x101, /* Ch5 OP1/OP2 */
    0x102, /* Ch6 OP1/OP2 */
    0x004, /* Ch1 OP3/OP4 */
    0x005, /* Ch2 OP3/OP4 */
    0x006, /* Ch3 OP3/OP4 */
    0x104, /* Ch4 OP3/OP4 */
    0x105, /* Ch5 OP3/OP4 */
    0x106, /* Ch6 OP3/OP4 */
];

static CH_OFFSET: [u32; 6] = [
    0x000, /* Ch1 */
    0x001, /* Ch2 */
    0x002, /* Ch3 */
    0x100, /* Ch4 */
    0x101, /* Ch5 */
    0x102, /* Ch6 */
];

/* LFO */
static LFO_CYCLES: [u32; 8] = [108, 77, 71, 67, 62, 44, 8, 5];

/* FM algorithm */
static FM_ALGORITHM: [[[u32; 8]; 6]; 4] = [
    [
        [1, 1, 1, 1, 1, 1, 1, 1], /* OP1_0         */
        [1, 1, 1, 1, 1, 1, 1, 1], /* OP1_1         */
        [0, 0, 0, 0, 0, 0, 0, 0], /* OP2           */
        [0, 0, 0, 0, 0, 0, 0, 0], /* Last operator */
        [0, 0, 0, 0, 0, 0, 0, 0], /* Last operator */
        [0, 0, 0, 0, 0, 0, 0, 1], /* Out           */
    ],
    [
        [0, 1, 0, 0, 0, 1, 0, 0], /* OP1_0         */
        [0, 0, 0, 0, 0, 0, 0, 0], /* OP1_1         */
        [1, 1, 1, 0, 0, 0, 0, 0], /* OP2           */
        [0, 0, 0, 0, 0, 0, 0, 0], /* Last operator */
        [0, 0, 0, 0, 0, 0, 0, 0], /* Last operator */
        [0, 0, 0, 0, 0, 1, 1, 1], /* Out           */
    ],
    [
        [0, 0, 0, 0, 0, 0, 0, 0], /* OP1_0         */
        [0, 0, 0, 0, 0, 0, 0, 0], /* OP1_1         */
        [0, 0, 0, 0, 0, 0, 0, 0], /* OP2           */
        [1, 0, 0, 1, 1, 1, 1, 0], /* Last operator */
        [0, 0, 0, 0, 0, 0, 0, 0], /* Last operator */
        [0, 0, 0, 0, 1, 1, 1, 1], /* Out           */
    ],
    [
        [0, 0, 1, 0, 0, 1, 0, 0], /* OP1_0         */
        [0, 0, 0, 0, 0, 0, 0, 0], /* OP1_1         */
        [0, 0, 0, 1, 0, 0, 0, 0], /* OP2           */
        [1, 1, 0, 1, 1, 0, 0, 0], /* Last operator */
        [0, 0, 1, 0, 0, 0, 0, 0], /* Last operator */
        [1, 1, 1, 1, 1, 1, 1, 1], /* Out           */
    ],
];

/// `chip_type` (a global, as upstream's is; every emulator sets it to the
/// ASIC when made)
static CHIP_TYPE: AtomicU32 = AtomicU32::new(YM3438_TYPE_DISCRETE);

fn chip_type() -> u32 {
    CHIP_TYPE.load(Ordering::Relaxed)
}

fn opn2_do_io(chip: &mut Ym3438) {
    /* Write signal check */
    chip.write_a_en = ((chip.write_a & 0x03) == 0x01) as u8;
    chip.write_d_en = ((chip.write_d & 0x03) == 0x01) as u8;
    chip.write_a <<= 1;
    chip.write_d <<= 1;
    /* Busy counter */
    chip.busy = chip.write_busy;
    chip.write_busy_cnt = chip.write_busy_cnt.wrapping_add(chip.write_busy);
    chip.write_busy =
        ((chip.write_busy != 0 && (chip.write_busy_cnt >> 5) == 0) || chip.write_d_en != 0) as u8;
    chip.write_busy_cnt &= 0x1f;
}

fn opn2_do_reg_write(chip: &mut Ym3438) {
    let mut slot = (chip.cycles % 12) as usize;
    let channel = chip.channel as usize;
    /* Update registers */
    if chip.write_fm_data != 0 {
        /* Slot */
        if OP_OFFSET[slot] == (chip.address as u32 & 0x107) {
            if chip.address & 0x08 != 0 {
                /* OP2, OP4 */
                slot += 12;
            }
            let address = chip.address & 0xf0;
            match address {
                0x30 => {
                    /* DT, MULTI */
                    chip.multi[slot] = chip.data & 0x0f;
                    if chip.multi[slot] == 0 {
                        chip.multi[slot] = 1;
                    } else {
                        chip.multi[slot] <<= 1;
                    }
                    chip.dt[slot] = (chip.data >> 4) & 0x07;
                }
                0x40 => {
                    /* TL */
                    chip.tl[slot] = chip.data & 0x7f;
                }
                0x50 => {
                    /* KS, AR */
                    chip.ar[slot] = chip.data & 0x1f;
                    chip.ks[slot] = (chip.data >> 6) & 0x03;
                }
                0x60 => {
                    /* AM, DR */
                    chip.dr[slot] = chip.data & 0x1f;
                    chip.am[slot] = (chip.data >> 7) & 0x01;
                }
                0x70 => {
                    /* SR */
                    chip.sr[slot] = chip.data & 0x1f;
                }
                0x80 => {
                    /* SL, RR */
                    chip.rr[slot] = chip.data & 0x0f;
                    chip.sl[slot] = (chip.data >> 4) & 0x0f;
                    chip.sl[slot] |= (chip.sl[slot] + 1) & 0x10;
                }
                0x90 => {
                    /* SSG-EG */
                    chip.ssg_eg[slot] = chip.data & 0x0f;
                }
                _ => {}
            }
        }

        /* Channel */
        if CH_OFFSET[channel] == (chip.address as u32 & 0x103) {
            let address = chip.address & 0xfc;
            match address {
                0xa0 => {
                    chip.fnum[channel] =
                        (chip.data as u16 & 0xff) | ((chip.reg_a4 as u16 & 0x07) << 8);
                    chip.block[channel] = (chip.reg_a4 >> 3) & 0x07;
                    chip.kcode[channel] = (chip.block[channel] << 2)
                        | FN_NOTE[(chip.fnum[channel] >> 7) as usize] as u8;
                }
                0xa4 => {
                    chip.reg_a4 = chip.data;
                }
                0xa8 => {
                    chip.fnum_3ch[channel] =
                        (chip.data as u16 & 0xff) | ((chip.reg_ac as u16 & 0x07) << 8);
                    chip.block_3ch[channel] = (chip.reg_ac >> 3) & 0x07;
                    chip.kcode_3ch[channel] = (chip.block_3ch[channel] << 2)
                        | FN_NOTE[(chip.fnum_3ch[channel] >> 7) as usize] as u8;
                }
                0xac => {
                    chip.reg_ac = chip.data;
                }
                0xb0 => {
                    chip.connect[channel] = chip.data & 0x07;
                    chip.fb[channel] = (chip.data >> 3) & 0x07;
                }
                0xb4 => {
                    chip.pms[channel] = chip.data & 0x07;
                    chip.ams[channel] = (chip.data >> 4) & 0x03;
                    chip.pan_l[channel] = (chip.data >> 7) & 0x01;
                    chip.pan_r[channel] = (chip.data >> 6) & 0x01;
                }
                _ => {}
            }
        }
    }

    if chip.write_a_en != 0 || chip.write_d_en != 0 {
        /* Data */
        if chip.write_a_en != 0 {
            chip.write_fm_data = 0;
        }

        if chip.write_fm_address != 0 && chip.write_d_en != 0 {
            chip.write_fm_data = 1;
        }

        /* Address */
        if chip.write_a_en != 0 {
            if (chip.write_data & 0xf0) != 0x00 {
                /* FM Write */
                chip.address = chip.write_data;
                chip.write_fm_address = 1;
            } else {
                /* SSG write */
                chip.write_fm_address = 0;
            }
        }

        /* FM Mode */
        /* Data */
        if chip.write_d_en != 0 && (chip.write_data & 0x100) == 0 {
            let wd = chip.write_data;
            match chip.address {
                0x21 => {
                    /* LSI test 1 */
                    for i in 0..8 {
                        chip.mode_test_21[i] = ((wd >> i) & 0x01) as u8;
                    }
                }
                0x22 => {
                    /* LFO control */
                    if (wd >> 3) & 0x01 != 0 {
                        chip.lfo_en = 0x7f;
                    } else {
                        chip.lfo_en = 0;
                    }
                    chip.lfo_freq = (wd & 0x07) as u8;
                }
                0x24 => {
                    /* Timer A */
                    chip.timer_a_reg &= 0x03;
                    chip.timer_a_reg |= (wd & 0xff) << 2;
                }
                0x25 => {
                    chip.timer_a_reg &= 0x3fc;
                    chip.timer_a_reg |= wd & 0x03;
                }
                0x26 => {
                    /* Timer B */
                    chip.timer_b_reg = wd & 0xff;
                }
                0x27 => {
                    /* CSM, Timer control */
                    chip.mode_ch3 = ((wd & 0xc0) >> 6) as u8;
                    chip.mode_csm = (chip.mode_ch3 == 2) as u8;
                    chip.timer_a_load = (wd & 0x01) as u8;
                    chip.timer_a_enable = ((wd >> 2) & 0x01) as u8;
                    chip.timer_a_reset = ((wd >> 4) & 0x01) as u8;
                    chip.timer_b_load = ((wd >> 1) & 0x01) as u8;
                    chip.timer_b_enable = ((wd >> 3) & 0x01) as u8;
                    chip.timer_b_reset = ((wd >> 5) & 0x01) as u8;
                }
                0x28 => {
                    /* Key on/off */
                    for i in 0..4 {
                        chip.mode_kon_operator[i] = ((wd >> (4 + i)) & 0x01) as u8;
                    }
                    if (wd & 0x03) == 0x03 {
                        /* Invalid address */
                        chip.mode_kon_channel = 0xff;
                    } else {
                        chip.mode_kon_channel = ((wd & 0x03) + ((wd >> 2) & 1) * 3) as u8;
                    }
                }
                0x2a => {
                    /* DAC data */
                    chip.dacdata &= 0x01;
                    chip.dacdata |= ((wd ^ 0x80) << 1) as i16;
                }
                0x2b => {
                    /* DAC enable */
                    chip.dacen = (wd >> 7) as u8;
                }
                0x2c => {
                    /* LSI test 2 */
                    for i in 0..8 {
                        chip.mode_test_2c[i] = ((wd >> i) & 0x01) as u8;
                    }
                    chip.dacdata &= 0x1fe;
                    chip.dacdata |= chip.mode_test_2c[3] as i16;
                    chip.eg_custom_timer =
                        (chip.mode_test_2c[7] == 0 && chip.mode_test_2c[6] != 0) as u8;
                }
                _ => {}
            }
        }

        /* Address */
        if chip.write_a_en != 0 {
            chip.write_fm_mode_a = (chip.write_data & 0xff) as u8;
        }
    }

    if chip.write_fm_data != 0 {
        chip.data = (chip.write_data & 0xff) as u8;
    }
}

fn opn2_phase_calc_increment(chip: &mut Ym3438) {
    let chan = chip.channel as usize;
    let slot = chip.cycles as usize;
    let mut fnum: u32 = chip.pg_fnum as u32;
    let fnum_h: u32 = fnum >> 4;
    let lfo: u8 = chip.lfo_pm;
    let mut lfo_l: u8 = lfo & 0x0f;
    let pms: u8 = chip.pms[chan];
    let dt: u8 = chip.dt[slot];
    let dt_l: u8 = dt & 0x03;
    let mut detune: u8 = 0;
    let mut kcode: u8 = chip.pg_kcode;

    fnum <<= 1;
    /* Apply LFO */
    if lfo_l & 0x08 != 0 {
        lfo_l ^= 0x0f;
    }
    let mut fm: u32 = (fnum_h >> PG_LFO_SH1[pms as usize][lfo_l as usize])
        + (fnum_h >> PG_LFO_SH2[pms as usize][lfo_l as usize]);
    if pms > 5 {
        fm <<= pms - 5;
    }
    fm >>= 2;
    if lfo & 0x10 != 0 {
        fnum = fnum.wrapping_sub(fm);
    } else {
        fnum = fnum.wrapping_add(fm);
    }
    fnum &= 0xfff;

    let mut basefreq: u32 = (fnum << chip.pg_block) >> 2;

    /* Apply detune */
    if dt_l != 0 {
        if kcode > 0x1c {
            kcode = 0x1c;
        }
        let block: u8 = kcode >> 2;
        let note: u8 = kcode & 0x03;
        let sum: u8 = block + 9 + (((dt_l == 3) as u8) | (dt_l & 0x02));
        let sum_h: u8 = sum >> 1;
        let sum_l: u8 = sum & 0x01;
        detune = (PG_DETUNE[((sum_l << 2) | note) as usize] >> (9 - sum_h)) as u8;
    }
    if dt & 0x04 != 0 {
        basefreq = basefreq.wrapping_sub(detune as u32);
    } else {
        basefreq = basefreq.wrapping_add(detune as u32);
    }
    basefreq &= 0x1ffff;
    chip.pg_inc[slot] = (basefreq * chip.multi[slot] as u32) >> 1;
    chip.pg_inc[slot] &= 0xfffff;
}

fn opn2_phase_generate(chip: &mut Ym3438) {
    /* Mask increment */
    let slot = ((chip.cycles + 20) % 24) as usize;
    if chip.pg_reset[slot] != 0 {
        chip.pg_inc[slot] = 0;
    }
    /* Phase step */
    let slot = ((chip.cycles + 19) % 24) as usize;
    chip.pg_phase[slot] = chip.pg_phase[slot].wrapping_add(chip.pg_inc[slot]);
    chip.pg_phase[slot] &= 0xfffff;
    if chip.pg_reset[slot] != 0 || chip.mode_test_21[3] != 0 {
        chip.pg_phase[slot] = 0;
    }
}

fn opn2_envelope_ssg_eg(chip: &mut Ym3438) {
    let slot = chip.cycles as usize;
    let mut direction: u8 = 0;
    chip.eg_ssg_pgrst_latch[slot] = 0;
    chip.eg_ssg_repeat_latch[slot] = 0;
    chip.eg_ssg_hold_up_latch[slot] = 0;
    chip.eg_ssg_inv[slot] = 0;
    if chip.ssg_eg[slot] & 0x08 != 0 {
        direction = chip.eg_ssg_dir[slot];
        if chip.eg_level[slot] & 0x200 != 0 {
            /* Reset */
            if (chip.ssg_eg[slot] & 0x03) == 0x00 {
                chip.eg_ssg_pgrst_latch[slot] = 1;
            }
            /* Repeat */
            if (chip.ssg_eg[slot] & 0x01) == 0x00 {
                chip.eg_ssg_repeat_latch[slot] = 1;
            }
            /* Inverse */
            if (chip.ssg_eg[slot] & 0x03) == 0x02 {
                direction ^= 1;
            }
            if (chip.ssg_eg[slot] & 0x03) == 0x03 {
                direction = 1;
            }
        }
        /* Hold up */
        if chip.eg_kon_latch[slot] != 0
            && ((chip.ssg_eg[slot] & 0x07) == 0x05 || (chip.ssg_eg[slot] & 0x07) == 0x03)
        {
            chip.eg_ssg_hold_up_latch[slot] = 1;
        }
        direction &= chip.eg_kon[slot];
        chip.eg_ssg_inv[slot] =
            (chip.eg_ssg_dir[slot] ^ ((chip.ssg_eg[slot] >> 2) & 0x01)) & chip.eg_kon[slot];
    }
    chip.eg_ssg_dir[slot] = direction;
    chip.eg_ssg_enable[slot] = (chip.ssg_eg[slot] >> 3) & 0x01;
}

/// `(~level << eg_inc) >> 5`, on `int` (the left shift of a negative value
/// is two's complement, as GCC does it and C++20 defines it)
#[inline]
fn attack_inc(level: i16, eg_inc: u8) -> i16 {
    ((!(level as i32)).wrapping_shl(eg_inc as u32) >> 5) as i16
}

fn opn2_envelope_adsr(chip: &mut Ym3438) {
    let slot = ((chip.cycles + 22) % 24) as usize;

    let nkon: u8 = chip.eg_kon_latch[slot];
    let okon: u8 = chip.eg_kon[slot];
    let mut nextlevel: i16;
    let mut nextstate: u8 = chip.eg_state[slot];
    let mut inc: i16 = 0;
    chip.eg_read[0] = chip.eg_read_inc as u32;
    chip.eg_read_inc = (chip.eg_inc > 0) as u8;

    /* Reset phase generator */
    chip.pg_reset[slot] = ((nkon != 0 && okon == 0) || chip.eg_ssg_pgrst_latch[slot] != 0) as u8;

    /* KeyOn/Off */
    let kon_event: bool =
        (nkon != 0 && okon == 0) || (okon != 0 && chip.eg_ssg_repeat_latch[slot] != 0);
    let koff_event: bool = okon != 0 && nkon == 0;

    let mut level: i16 = chip.eg_level[slot] as i16;
    let mut ssg_level: i16 = level;

    if chip.eg_ssg_inv[slot] != 0 {
        /* Inverse */
        ssg_level = (512 - level as i32) as i16;
        ssg_level &= 0x3ff;
    }
    if koff_event {
        level = ssg_level;
    }
    let eg_off: u8 = if chip.eg_ssg_enable[slot] != 0 {
        (level >> 9) as u8
    } else {
        ((level & 0x3f0) == 0x3f0) as u8
    };
    nextlevel = level;
    if kon_event {
        nextstate = EG_NUM_ATTACK;
        /* Instant attack */
        if chip.eg_ratemax != 0 {
            nextlevel = 0;
        } else if chip.eg_state[slot] == EG_NUM_ATTACK
            && level != 0
            && chip.eg_inc != 0
            && nkon != 0
        {
            inc = attack_inc(level, chip.eg_inc);
        }
    } else {
        match chip.eg_state[slot] {
            EG_NUM_ATTACK => {
                if level == 0 {
                    nextstate = EG_NUM_DECAY;
                } else if chip.eg_inc != 0 && chip.eg_ratemax == 0 && nkon != 0 {
                    inc = attack_inc(level, chip.eg_inc);
                }
            }
            EG_NUM_DECAY => {
                if (level >> 5) as i32 == chip.eg_sl[1] as i32 {
                    nextstate = EG_NUM_SUSTAIN;
                } else if eg_off == 0 && chip.eg_inc != 0 {
                    inc = 1 << (chip.eg_inc - 1);
                    if chip.eg_ssg_enable[slot] != 0 {
                        inc <<= 2;
                    }
                }
            }
            EG_NUM_SUSTAIN | EG_NUM_RELEASE => {
                if eg_off == 0 && chip.eg_inc != 0 {
                    inc = 1 << (chip.eg_inc - 1);
                    if chip.eg_ssg_enable[slot] != 0 {
                        inc <<= 2;
                    }
                }
            }
            _ => {}
        }
        if nkon == 0 {
            nextstate = EG_NUM_RELEASE;
        }
    }
    if chip.eg_kon_csm[slot] != 0 {
        nextlevel |= (chip.eg_tl[1] as i16) << 3;
    }

    /* Envelope off */
    if !kon_event
        && chip.eg_ssg_hold_up_latch[slot] == 0
        && chip.eg_state[slot] != EG_NUM_ATTACK
        && eg_off != 0
    {
        nextstate = EG_NUM_RELEASE;
        nextlevel = 0x3ff;
    }

    nextlevel = nextlevel.wrapping_add(inc);

    chip.eg_kon[slot] = chip.eg_kon_latch[slot];
    chip.eg_level[slot] = (nextlevel as u16) & 0x3ff;
    chip.eg_state[slot] = nextstate;
}

fn opn2_envelope_prepare(chip: &mut Ym3438) {
    let mut inc: u8 = 0;
    let slot = chip.cycles as usize;

    /* Prepare increment */
    let mut rate: u8 = (chip.eg_rate << 1).wrapping_add(chip.eg_ksv);

    if rate > 0x3f {
        rate = 0x3f;
    }

    let sum: u8 = ((rate >> 2).wrapping_add(chip.eg_shift_lock)) & 0x0f;
    if chip.eg_rate != 0 && chip.eg_quotient == 2 {
        if rate < 48 {
            match sum {
                12 => inc = 1,
                13 => inc = (rate >> 1) & 0x01,
                14 => inc = rate & 0x01,
                _ => {}
            }
        } else {
            inc = (EG_STEPHI[(rate & 0x03) as usize][chip.eg_timer_low_lock as usize]
                + (rate >> 2) as u32
                - 11) as u8;
            if inc > 4 {
                inc = 4;
            }
        }
    }
    chip.eg_inc = inc;
    chip.eg_ratemax = ((rate >> 1) == 0x1f) as u8;

    /* Prepare rate & ksv */
    let mut rate_sel: u8 = chip.eg_state[slot];
    if (chip.eg_kon[slot] != 0 && chip.eg_ssg_repeat_latch[slot] != 0)
        || (chip.eg_kon[slot] == 0 && chip.eg_kon_latch[slot] != 0)
    {
        rate_sel = EG_NUM_ATTACK;
    }
    match rate_sel {
        EG_NUM_ATTACK => chip.eg_rate = chip.ar[slot],
        EG_NUM_DECAY => chip.eg_rate = chip.dr[slot],
        EG_NUM_SUSTAIN => chip.eg_rate = chip.sr[slot],
        EG_NUM_RELEASE => chip.eg_rate = (chip.rr[slot] << 1) | 0x01,
        _ => {}
    }
    chip.eg_ksv = chip.pg_kcode >> (chip.ks[slot] ^ 0x03);
    if chip.am[slot] != 0 {
        chip.eg_lfo_am = chip.lfo_am >> EG_AM_SHIFT[chip.ams[chip.channel as usize] as usize];
    } else {
        chip.eg_lfo_am = 0;
    }
    /* Delay TL & SL value */
    chip.eg_tl[1] = chip.eg_tl[0];
    chip.eg_tl[0] = chip.tl[slot];
    chip.eg_sl[1] = chip.eg_sl[0];
    chip.eg_sl[0] = chip.sl[slot];
}

fn opn2_envelope_generate(chip: &mut Ym3438) {
    let slot = ((chip.cycles + 23) % 24) as usize;

    let mut level: u16 = chip.eg_level[slot];

    if chip.eg_ssg_inv[slot] != 0 {
        /* Inverse */
        level = (512 - level as i32) as u16;
    }
    if chip.mode_test_21[5] != 0 {
        level = 0;
    }
    level &= 0x3ff;

    /* Apply AM LFO */
    level = level.wrapping_add(chip.eg_lfo_am as u16);

    /* Apply TL */
    if !(chip.mode_csm != 0 && chip.channel == 2 + 1) {
        level = level.wrapping_add((chip.eg_tl[0] as u16) << 3);
    }
    if level > 0x3ff {
        level = 0x3ff;
    }
    chip.eg_out[slot] = level;
}

fn opn2_update_lfo(chip: &mut Ym3438) {
    let cycles = LFO_CYCLES[chip.lfo_freq as usize];
    if (chip.lfo_quotient as u32 & cycles) == cycles {
        chip.lfo_quotient = 0;
        chip.lfo_cnt = chip.lfo_cnt.wrapping_add(1);
    } else {
        chip.lfo_quotient = chip.lfo_quotient.wrapping_add(chip.lfo_inc);
    }
    chip.lfo_cnt &= chip.lfo_en;
}

fn opn2_fm_prepare(chip: &mut Ym3438) {
    let mut slot = ((chip.cycles + 6) % 24) as usize;
    let channel = chip.channel as usize;
    let op = slot / 6;
    let connect = chip.connect[channel] as usize;
    let prevslot = ((chip.cycles + 18) % 24) as usize;

    /* Calculate modulation */
    let mut mod1: i16 = 0;
    let mut mod2: i16 = 0;

    if FM_ALGORITHM[op][0][connect] != 0 {
        mod2 |= chip.fm_op1[channel][0];
    }
    if FM_ALGORITHM[op][1][connect] != 0 {
        mod1 |= chip.fm_op1[channel][1];
    }
    if FM_ALGORITHM[op][2][connect] != 0 {
        mod1 |= chip.fm_op2[channel];
    }
    if FM_ALGORITHM[op][3][connect] != 0 {
        mod2 |= chip.fm_out[prevslot];
    }
    if FM_ALGORITHM[op][4][connect] != 0 {
        mod1 |= chip.fm_out[prevslot];
    }
    let mut mod_: i16 = mod1.wrapping_add(mod2);
    if op == 0 {
        /* Feedback */
        mod_ >>= 10 - chip.fb[channel];
        if chip.fb[channel] == 0 {
            mod_ = 0;
        }
    } else {
        mod_ >>= 1;
    }
    chip.fm_mod[slot] = mod_ as u16;

    slot = ((chip.cycles + 18) % 24) as usize;
    /* OP1 */
    if slot / 6 == 0 {
        chip.fm_op1[channel][1] = chip.fm_op1[channel][0];
        chip.fm_op1[channel][0] = chip.fm_out[slot];
    }
    /* OP2 */
    if slot / 6 == 2 {
        chip.fm_op2[channel] = chip.fm_out[slot];
    }
}

fn opn2_ch_generate(chip: &mut Ym3438) {
    let slot = ((chip.cycles + 18) % 24) as usize;
    let channel = chip.channel as usize;
    let op = slot / 6;
    let test_dac: u32 = chip.mode_test_2c[5] as u32;
    let mut acc: i16 = chip.ch_acc[channel];
    let mut add: i16 = test_dac as i16;
    if op == 0 && test_dac == 0 {
        acc = 0;
    }
    if FM_ALGORITHM[op][5][chip.connect[channel] as usize] != 0 && test_dac == 0 {
        add = add.wrapping_add(chip.fm_out[slot] >> 5);
    }
    let mut sum: i16 = acc.wrapping_add(add);
    /* Clamp */
    if sum > 255 {
        sum = 255;
    } else if sum < -256 {
        sum = -256;
    }

    if op == 0 || test_dac != 0 {
        chip.ch_out[channel] = chip.ch_acc[channel];
    }
    chip.ch_acc[channel] = sum;
}

fn opn2_ch_output(chip: &mut Ym3438) {
    let cycles = chip.cycles;
    let slot = chip.cycles;
    let mut channel = chip.channel as usize;
    let test_dac: u32 = chip.mode_test_2c[5] as u32;
    let mut out: i16;
    let mut sign: i16;
    let out_en: u32;
    chip.ch_read = chip.ch_lock;
    if slot < 12 {
        /* Ch 4,5,6 */
        channel += 1;
    }
    if (cycles & 3) == 0 {
        if test_dac == 0 {
            /* Lock value */
            chip.ch_lock = chip.ch_out[channel];
        }
        chip.ch_lock_l = chip.pan_l[channel];
        chip.ch_lock_r = chip.pan_r[channel];
    }
    /* Ch 6 */
    if ((cycles >> 2) == 1 && chip.dacen != 0) || test_dac != 0 {
        out = chip.dacdata;
        out = out.wrapping_shl(7);
        out >>= 7;
    } else {
        out = chip.ch_lock;
    }
    chip.mol = 0;
    chip.mor = 0;

    if chip_type() == YM3438_TYPE_YM2612 {
        out_en = ((cycles & 3) == 3 || test_dac != 0) as u32;
        /* YM2612 DAC emulation(not verified) */
        sign = out >> 8;
        if out >= 0 {
            out += 1;
            sign += 1;
        }
        if chip.ch_lock_l != 0 && out_en != 0 {
            chip.mol = out;
        } else {
            chip.mol = sign;
        }
        if chip.ch_lock_r != 0 && out_en != 0 {
            chip.mor = out;
        } else {
            chip.mor = sign;
        }
        /* Amplify signal */
        chip.mol = chip.mol.wrapping_mul(3);
        chip.mor = chip.mor.wrapping_mul(3);
    } else {
        out_en = ((cycles & 3) != 0 || test_dac != 0) as u32;
        /* Discrete YM3438 seems has the ladder effect too */
        if out >= 0 && chip_type() == YM3438_TYPE_DISCRETE {
            out += 1;
        }
        if chip.ch_lock_l != 0 && out_en != 0 {
            chip.mol = out;
        }
        if chip.ch_lock_r != 0 && out_en != 0 {
            chip.mor = out;
        }
    }
}

fn opn2_fm_generate(chip: &mut Ym3438) {
    let slot = ((chip.cycles + 19) % 24) as usize;
    /* Calculate phase */
    let phase: u16 = ((chip.fm_mod[slot] as u32 + (chip.pg_phase[slot] >> 10)) & 0x3ff) as u16;
    let quarter: u16 = if phase & 0x100 != 0 {
        (phase ^ 0xff) & 0xff
    } else {
        phase & 0xff
    };
    let mut level: u16 = LOGSINROM[quarter as usize];
    /* Apply envelope */
    level = level.wrapping_add(chip.eg_out[slot] << 2);
    /* Transform */
    if level > 0x1fff {
        level = 0x1fff;
    }
    let mut output: i16 =
        (((EXPROM[((level & 0xff) ^ 0xff) as usize] as i32 | 0x400) << 2) >> (level >> 8)) as i16;
    if phase & 0x200 != 0 {
        output =
            ((!(output as i32)) ^ ((chip.mode_test_21[4] as i32) << 13)).wrapping_add(1) as i16;
    } else {
        output = ((output as i32) ^ ((chip.mode_test_21[4] as i32) << 13)) as i16;
    }
    output = output.wrapping_shl(2);
    output >>= 2;
    chip.fm_out[slot] = output;
}

fn opn2_do_timer_a(chip: &mut Ym3438) {
    let mut load: u8 = chip.timer_a_overflow;
    if chip.cycles == 2 {
        /* Lock load value */
        load |= (chip.timer_a_load_lock == 0 && chip.timer_a_load != 0) as u8;
        chip.timer_a_load_lock = chip.timer_a_load;
        if chip.mode_csm != 0 {
            /* CSM KeyOn */
            chip.mode_kon_csm = load;
        } else {
            chip.mode_kon_csm = 0;
        }
    }
    /* Load counter */
    let mut time: u16 = if chip.timer_a_load_latch != 0 {
        chip.timer_a_reg
    } else {
        chip.timer_a_cnt
    };
    chip.timer_a_load_latch = load;
    /* Increase counter */
    if (chip.cycles == 1 && chip.timer_a_load_lock != 0) || chip.mode_test_21[2] != 0 {
        time = time.wrapping_add(1);
    }
    /* Set overflow flag */
    if chip.timer_a_reset != 0 {
        chip.timer_a_reset = 0;
        chip.timer_a_overflow_flag = 0;
    } else {
        chip.timer_a_overflow_flag |= chip.timer_a_overflow & chip.timer_a_enable;
    }
    chip.timer_a_overflow = (time >> 10) as u8;
    chip.timer_a_cnt = time & 0x3ff;
}

fn opn2_do_timer_b(chip: &mut Ym3438) {
    let mut load: u8 = chip.timer_b_overflow;
    if chip.cycles == 2 {
        /* Lock load value */
        load |= (chip.timer_b_load_lock == 0 && chip.timer_b_load != 0) as u8;
        chip.timer_b_load_lock = chip.timer_b_load;
    }
    /* Load counter */
    let mut time: u16 = if chip.timer_b_load_latch != 0 {
        chip.timer_b_reg
    } else {
        chip.timer_b_cnt
    };
    chip.timer_b_load_latch = load;
    /* Increase counter */
    if chip.cycles == 1 {
        chip.timer_b_subcnt = chip.timer_b_subcnt.wrapping_add(1);
    }
    if (chip.timer_b_subcnt == 0x10 && chip.timer_b_load_lock != 0) || chip.mode_test_21[2] != 0 {
        time = time.wrapping_add(1);
    }
    chip.timer_b_subcnt &= 0x0f;
    /* Set overflow flag */
    if chip.timer_b_reset != 0 {
        chip.timer_b_reset = 0;
        chip.timer_b_overflow_flag = 0;
    } else {
        chip.timer_b_overflow_flag |= chip.timer_b_overflow & chip.timer_b_enable;
    }
    chip.timer_b_overflow = (time >> 8) as u8;
    chip.timer_b_cnt = time & 0xff;
}

fn opn2_key_on(chip: &mut Ym3438) {
    let slot = chip.cycles as usize;
    let chan = chip.channel as usize;
    /* Key On */
    chip.eg_kon_latch[slot] = chip.mode_kon[slot];
    chip.eg_kon_csm[slot] = 0;
    if chip.channel == 2 && chip.mode_kon_csm != 0 {
        /* CSM Key On */
        chip.eg_kon_latch[slot] = 1;
        chip.eg_kon_csm[slot] = 1;
    }
    if chip.cycles == chip.mode_kon_channel as u32 {
        /* OP1 */
        chip.mode_kon[chan] = chip.mode_kon_operator[0];
        /* OP2 */
        chip.mode_kon[chan + 12] = chip.mode_kon_operator[1];
        /* OP3 */
        chip.mode_kon[chan + 6] = chip.mode_kon_operator[2];
        /* OP4 */
        chip.mode_kon[chan + 18] = chip.mode_kon_operator[3];
    }
}

/* EXTRA, original was "void OPN2_Reset(ym3438_t *chip)" */
pub fn opn2_reset(chip: &mut Ym3438, rate: u32, clock: u32) {
    let rateratio: u32 = chip.rateratio as u32;
    *chip = Ym3438::default();
    for i in 0..24 {
        chip.eg_out[i] = 0x3ff;
        chip.eg_level[i] = 0x3ff;
        chip.eg_state[i] = EG_NUM_RELEASE;
        chip.multi[i] = 1;
    }
    for i in 0..6 {
        chip.pan_l[i] = 1;
        chip.pan_r[i] = 1;
    }

    // FIXME (upstream): a clock rate of 0 divides by zero; it keeps the
    // ratio then, as a rate of 0 does (which set_rate() refuses), and
    // upstream's build for the tests is patched the same.
    if rate != 0 && clock != 0 {
        chip.rateratio = ((((144u64 * rate as u64) << RSM_FRAC) / clock as u64) as u32) as i32;
    } else {
        chip.rateratio = rateratio as i32;
    }
}

pub fn opn2_set_chip_type(type_: u32) {
    CHIP_TYPE.store(type_, Ordering::Relaxed);
}

pub fn opn2_clock(chip: &mut Ym3438, buffer: &mut [i16; 2]) {
    let slot = chip.cycles;
    chip.lfo_inc = chip.mode_test_21[1];
    chip.pg_read >>= 1;
    chip.eg_read[1] >>= 1;
    chip.eg_cycle = chip.eg_cycle.wrapping_add(1);
    /* Lock envelope generator timer value */
    if chip.cycles == 1 && chip.eg_quotient == 2 {
        if chip.eg_cycle_stop != 0 {
            chip.eg_shift_lock = 0;
        } else {
            chip.eg_shift_lock = chip.eg_shift.wrapping_add(1);
        }
        chip.eg_timer_low_lock = (chip.eg_timer & 0x03) as u8;
    }
    /* Cycle specific functions */
    match chip.cycles {
        0 => {
            chip.lfo_pm = chip.lfo_cnt >> 2;
            if chip.lfo_cnt & 0x40 != 0 {
                chip.lfo_am = chip.lfo_cnt & 0x3f;
            } else {
                chip.lfo_am = chip.lfo_cnt ^ 0x3f;
            }
            chip.lfo_am <<= 1;
        }
        1 => {
            chip.eg_quotient += 1;
            chip.eg_quotient %= 3;
            chip.eg_cycle = 0;
            chip.eg_cycle_stop = 1;
            chip.eg_shift = 0;
            chip.eg_timer_inc |= (chip.eg_quotient >> 1) as u8;
            chip.eg_timer = chip.eg_timer.wrapping_add(chip.eg_timer_inc as u16);
            chip.eg_timer_inc = (chip.eg_timer >> 12) as u8;
            chip.eg_timer &= 0xfff;
        }
        2 => {
            chip.pg_read = chip.pg_phase[21] & 0x3ff;
            chip.eg_read[1] = chip.eg_out[0] as u32;
        }
        13 => {
            chip.eg_cycle = 0;
            chip.eg_cycle_stop = 1;
            chip.eg_shift = 0;
            chip.eg_timer = chip.eg_timer.wrapping_add(chip.eg_timer_inc as u16);
            chip.eg_timer_inc = (chip.eg_timer >> 12) as u8;
            chip.eg_timer &= 0xfff;
        }
        23 => {
            chip.lfo_inc |= 1;
        }
        _ => {}
    }
    chip.eg_timer &= !((chip.mode_test_21[5] as u32).wrapping_shl(chip.eg_cycle as u32) as u16);
    if (((chip.eg_timer as u32).wrapping_shr(chip.eg_cycle as u32))
        | (chip.pin_test_in & chip.eg_custom_timer) as u32)
        & chip.eg_cycle_stop as u32
        != 0
    {
        chip.eg_shift = chip.eg_cycle;
        chip.eg_cycle_stop = 0;
    }

    opn2_do_io(chip);

    opn2_do_timer_a(chip);
    opn2_do_timer_b(chip);
    opn2_key_on(chip);

    opn2_ch_output(chip);
    opn2_ch_generate(chip);

    opn2_fm_prepare(chip);
    opn2_fm_generate(chip);

    opn2_phase_generate(chip);
    opn2_phase_calc_increment(chip);

    opn2_envelope_adsr(chip);
    opn2_envelope_generate(chip);
    opn2_envelope_ssg_eg(chip);
    opn2_envelope_prepare(chip);

    /* Prepare fnum & block */
    let next = ((chip.channel + 1) % 6) as usize;
    if chip.mode_ch3 != 0 {
        /* Channel 3 special mode */
        match slot {
            1 => {
                /* OP1 */
                chip.pg_fnum = chip.fnum_3ch[1];
                chip.pg_block = chip.block_3ch[1];
                chip.pg_kcode = chip.kcode_3ch[1];
            }
            7 => {
                /* OP3 */
                chip.pg_fnum = chip.fnum_3ch[0];
                chip.pg_block = chip.block_3ch[0];
                chip.pg_kcode = chip.kcode_3ch[0];
            }
            13 => {
                /* OP2 */
                chip.pg_fnum = chip.fnum_3ch[2];
                chip.pg_block = chip.block_3ch[2];
                chip.pg_kcode = chip.kcode_3ch[2];
            }
            /* 19: OP4 */
            _ => {
                chip.pg_fnum = chip.fnum[next];
                chip.pg_block = chip.block[next];
                chip.pg_kcode = chip.kcode[next];
            }
        }
    } else {
        chip.pg_fnum = chip.fnum[next];
        chip.pg_block = chip.block[next];
        chip.pg_kcode = chip.kcode[next];
    }

    opn2_update_lfo(chip);
    opn2_do_reg_write(chip);
    chip.cycles = (chip.cycles + 1) % 24;
    chip.channel = chip.cycles % 6;

    buffer[0] = chip.mol;
    buffer[1] = chip.mor;
}

pub fn opn2_write(chip: &mut Ym3438, mut port: u32, data: u8) {
    port &= 3;
    chip.write_data = (((port << 7) & 0x100) | data as u32) as u16;
    if port & 1 != 0 {
        /* Data */
        chip.write_d |= 1;
    } else {
        /* Address */
        chip.write_a |= 1;
    }
}

pub fn opn2_set_test_pin(chip: &mut Ym3438, value: u32) {
    chip.pin_test_in = (value & 1) as u8;
}

pub fn opn2_read_test_pin(chip: &Ym3438) -> u32 {
    if chip.mode_test_2c[7] == 0 {
        return 0;
    }
    (chip.cycles == 23) as u32
}

pub fn opn2_read_irq_pin(chip: &Ym3438) -> u32 {
    (chip.timer_a_overflow_flag | chip.timer_b_overflow_flag) as u32
}

pub fn opn2_read(chip: &Ym3438, port: u32) -> u8 {
    if (port & 3) == 0 || chip_type() == YM3438_TYPE_ASIC {
        if chip.mode_test_21[6] != 0 {
            /* Read test data */
            let slot = ((chip.cycles + 18) % 24) as usize;
            let mut testdata: u16 = (((chip.pg_read & 0x01) << 15)
                | ((chip.eg_read[chip.mode_test_21[0] as usize] & 0x01) << 14))
                as u16;
            if chip.mode_test_2c[4] != 0 {
                testdata |= (chip.ch_read & 0x1ff) as u16;
            } else {
                testdata |= (chip.fm_out[slot] & 0x3fff) as u16;
            }
            if chip.mode_test_21[7] != 0 {
                return (testdata & 0xff) as u8;
            } else {
                return (testdata >> 8) as u8;
            }
        } else {
            return (chip.busy << 7)
                | (chip.timer_b_overflow_flag << 1)
                | chip.timer_a_overflow_flag;
        }
    }
    0
}

pub fn opn2_write_buffered(chip: &mut Ym3438, port: u32, data: u8) {
    let mut buffer = [0i16; 2];
    let last = chip.writebuf_last as usize;

    if chip.writebuf.0[last].port & 0x04 != 0 {
        let w = chip.writebuf.0[last];
        opn2_write(chip, (w.port & 0x03) as u32, w.data);

        chip.writebuf_cur = (chip.writebuf_last + 1) % OPN_WRITEBUF_SIZE as u32;
        let mut skip: u64 = w.time.wrapping_sub(chip.writebuf_samplecnt);
        chip.writebuf_samplecnt = w.time;
        while skip != 0 {
            skip -= 1;
            opn2_clock(chip, &mut buffer);
        }
    }

    chip.writebuf.0[last].port = ((port & 0x03) | 0x04) as u8;
    chip.writebuf.0[last].data = data;
    let mut time1: u64 = chip.writebuf_lasttime.wrapping_add(OPN_WRITEBUF_DELAY);
    let time2: u64 = chip.writebuf_samplecnt;

    if time1 < time2 {
        time1 = time2;
    }

    chip.writebuf.0[last].time = time1;
    chip.writebuf_lasttime = time1;
    chip.writebuf_last = (chip.writebuf_last + 1) % OPN_WRITEBUF_SIZE as u32;
}

pub fn opn2_generate(chip: &mut Ym3438, buf: &mut [i16; 2]) {
    let mut buffer = [0i16; 2];

    buf[0] = 0;
    buf[1] = 0;

    for _ in 0..24 {
        let mute: u32 = match chip.cycles >> 2 {
            0 => chip.mute[1],                       /* Ch 2 */
            1 => chip.mute[5 + chip.dacen as usize], /* Ch 6, DAC */
            2 => chip.mute[3],                       /* Ch 4 */
            3 => chip.mute[0],                       /* Ch 1 */
            4 => chip.mute[4],                       /* Ch 5 */
            5 => chip.mute[2],                       /* Ch 3 */
            _ => 0,
        };
        opn2_clock(chip, &mut buffer);
        if mute == 0 {
            buf[0] = buf[0].wrapping_add(buffer[0]);
            buf[1] = buf[1].wrapping_add(buffer[1]);
        }

        loop {
            let cur = chip.writebuf_cur as usize;
            if chip.writebuf.0[cur].time > chip.writebuf_samplecnt {
                break;
            }
            if chip.writebuf.0[cur].port & 0x04 == 0 {
                break;
            }
            chip.writebuf.0[cur].port &= 0x03;
            let w = chip.writebuf.0[cur];
            opn2_write(chip, w.port as u32, w.data);
            chip.writebuf_cur = (chip.writebuf_cur + 1) % OPN_WRITEBUF_SIZE as u32;
        }
        chip.writebuf_samplecnt = chip.writebuf_samplecnt.wrapping_add(1);
    }
}

pub fn opn2_generate_resampled(chip: &mut Ym3438, buf: &mut [i16; 2]) {
    let mut buffer = [0i16; 2];

    while chip.samplecnt >= chip.rateratio {
        chip.oldsamples[0] = chip.samples[0];
        chip.oldsamples[1] = chip.samples[1];
        opn2_generate(chip, &mut buffer);
        chip.samples[0] = buffer[0] as i32 * 11;
        chip.samples[1] = buffer[1] as i32 * 11;
        chip.samplecnt -= chip.rateratio;
    }
    // FIXME (upstream): the products overflow (int) at ratios past about
    // 6000 (clock rates under about a megahertz); they're in 64 bits here,
    // and in upstream's build for the tests.
    let (r, c) = (chip.rateratio as i64, chip.samplecnt as i64);
    buf[0] = (((chip.oldsamples[0] as i64 * (r - c) + chip.samples[0] as i64 * c) / r) >> 1) as i16;
    buf[1] = (((chip.oldsamples[1] as i64 * (r - c) + chip.samples[1] as i64 * c) / r) >> 1) as i16;
    chip.samplecnt += 1 << RSM_FRAC;
}

pub fn opn2_generate_stream(chip: &mut Ym3438, output: &mut [i16], numsamples: u32) {
    let mut buffer = [0i16; 2];

    for o in output.chunks_exact_mut(2).take(numsamples as usize) {
        opn2_generate_resampled(chip, &mut buffer);
        o[0] = buffer[0];
        o[1] = buffer[1];
    }
}

pub fn opn2_generate_stream_mix(chip: &mut Ym3438, output: &mut [i16], numsamples: u32) {
    let mut buffer = [0i16; 2];

    for o in output.chunks_exact_mut(2).take(numsamples as usize) {
        opn2_generate_resampled(chip, &mut buffer);
        o[0] = o[0].wrapping_add(buffer[0]);
        o[1] = o[1].wrapping_add(buffer[1]);
    }
}

pub fn opn2_set_options(flags: u8) {
    match (flags >> 3) & 0x03 {
        0x01 => {
            /* ASIC YM3438 */
            opn2_set_chip_type(YM3438_TYPE_ASIC);
        }
        0x02 => {
            /* Discrete YM3438 */
            opn2_set_chip_type(YM3438_TYPE_DISCRETE);
        }
        /* 0x00: YM2612 */
        _ => {
            opn2_set_chip_type(YM3438_TYPE_YM2612);
        }
    }
}

pub fn opn2_set_mute(chip: &mut Ym3438, mute: u32) {
    for i in 0..7 {
        chip.mute[i] = (mute >> i) & 0x01;
    }
}

/// `Ym2612_Emu`: the YM2612 emulator SDL_mixer's build uses.
pub type Ym2612Emu = Ym2612NukedEmu;

/// `Ym2612_Nuked_Emu`
#[derive(Debug)]
pub struct Ym2612NukedEmu {
    impl_: Box<Ym3438>,
    prev_sample_rate: f64,
    prev_clock_rate: f64,
}

impl Default for Ym2612NukedEmu {
    fn default() -> Self {
        Self::new()
    }
}

impl Ym2612NukedEmu {
    // Mute voice n if bit n (1 << n) of mask is set
    pub const CHANNEL_COUNT: i32 = 6;
    // stereo
    pub const OUT_CHAN_COUNT: i32 = 2;

    pub fn new() -> Self {
        opn2_set_chip_type(YM3438_TYPE_ASIC);
        Ym2612NukedEmu {
            // FIXME (upstream): the chip is left uninitialized until
            // set_rate(), where OPN2_Reset() reads its ratio first (to keep
            // it when the rate is 0); it starts zeroed here, and in
            // upstream's build for the tests.
            impl_: Box::default(),
            prev_sample_rate: 0.0,
            prev_clock_rate: 0.0,
        }
    }

    /// Set output sample rate and chip clock rates, in Hz. Returns non-zero
    /// if error.
    pub fn set_rate(&mut self, sample_rate: f64, clock_rate: f64) -> BlarggErr {
        let chip_r = &mut *self.impl_;
        self.prev_sample_rate = sample_rate;
        self.prev_clock_rate = clock_rate;
        opn2_reset(chip_r, sample_rate as u32, clock_rate as u32);
        // FIXME (upstream): a resampling ratio (from the rates) under 1
        // (none was made, or the rate is too low) divides by zero or never
        // ends, and one past what the sample count holds overflows it; the
        // rates fail then, here and in upstream's build for the tests.
        if chip_r.rateratio < 1 || chip_r.rateratio > 0x7FFF_FFFF - (1 << RSM_FRAC) {
            return Err("Unsupported YM2612 FM sound rate");
        }
        Ok(())
    }

    /// Reset to power-up state
    pub fn reset(&mut self) {
        let chip_r = &mut *self.impl_;
        opn2_reset(
            chip_r,
            self.prev_sample_rate as u32,
            self.prev_clock_rate as u32,
        );
    }

    /// Mute voice n if bit n (1 << n) of mask is set
    pub fn mute_voices(&mut self, mask: i32) {
        opn2_set_mute(&mut self.impl_, mask as u32);
    }

    /// Write addr to register 0 then data to register 1
    pub fn write0(&mut self, addr: i32, data: i32) {
        let chip_r = &mut *self.impl_;
        opn2_write_buffered(chip_r, 0, addr as u8);
        opn2_write_buffered(chip_r, 1, data as u8);
    }

    /// Write addr to register 2 then data to register 3
    pub fn write1(&mut self, addr: i32, data: i32) {
        let chip_r = &mut *self.impl_;
        opn2_write_buffered(chip_r, 2, addr as u8);
        opn2_write_buffered(chip_r, 1 + 2, data as u8);
    }

    /// Run and add pair_count samples into current output buffer contents
    pub fn run(&mut self, pair_count: i32, out: &mut [i16]) {
        opn2_generate_stream_mix(&mut self.impl_, out, pair_count as u32);
    }
}
