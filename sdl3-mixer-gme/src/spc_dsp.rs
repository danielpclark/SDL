// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/

/* Copyright (C) 2007 Shay Green. This module is free software; you
can redistribute it and/or modify it under the terms of the GNU Lesser
General Public License as published by the Free Software Foundation; either
version 2.1 of the License, or (at your option) any later version. This
module is distributed in the hope that it will be useful, but WITHOUT ANY
WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
FOR A PARTICULAR PURPOSE. See the GNU Lesser General Public License for more
details. You should have received a copy of the GNU Lesser General Public
License along with this module; if not, write to the Free Software Foundation,
Inc., 51 Franklin Street, Fifth Floor, Boston, MA 02110-1301 USA */
// Modified 2026-10-17: translated into Rust from SDL_mixer 3.3.0's Game_Music_Emu 0.6.5 (see NOTICE).

//! Fast SNES SPC-700 DSP emulator (about 3x speed of accurate one)
//! (`Spc_Dsp.h`, `Spc_Dsp.cpp`).
//!
//! The 64K RAM it shares with the SPC-700 is passed to the functions that
//! use it (upstream keeps a pointer to it), with the samples' main output
//! buffer (upstream's output pointer can point into it or into the DSP's
//! own `extra` buffer, which [`Out`] says).

pub type Sample = i16;

/// Mutes voices corresponding to non-zero bits in mask (overrides VxVOL with 0).
/// Reduces emulation accuracy.
pub const VOICE_COUNT: usize = 8;

/// Resets DSP and uses supplied values to initialize registers
pub const REGISTER_COUNT: usize = 128;

// Global registers
pub const R_MVOLL: usize = 0x0C;
pub const R_MVOLR: usize = 0x1C;
pub const R_EVOLL: usize = 0x2C;
pub const R_EVOLR: usize = 0x3C;
pub const R_KON: usize = 0x4C;
pub const R_KOFF: usize = 0x5C;
pub const R_FLG: usize = 0x6C;
pub const R_ENDX: usize = 0x7C;
pub const R_EFB: usize = 0x0D;
pub const R_PMON: usize = 0x2D;
pub const R_NON: usize = 0x3D;
pub const R_EON: usize = 0x4D;
pub const R_DIR: usize = 0x5D;
pub const R_ESA: usize = 0x6D;
pub const R_EDL: usize = 0x7D;
pub const R_FIR: usize = 0x0F; // 8 coefficients at 0x0F, 0x1F ... 0x7F

// Voice registers
pub const V_VOLL: usize = 0x00;
pub const V_VOLR: usize = 0x01;
pub const V_PITCHL: usize = 0x02;
pub const V_PITCHH: usize = 0x03;
pub const V_SRCN: usize = 0x04;
pub const V_ADSR0: usize = 0x05;
pub const V_ADSR1: usize = 0x06;
pub const V_GAIN: usize = 0x07;
pub const V_ENVX: usize = 0x08;
pub const V_OUTX: usize = 0x09;

pub const EXTRA_SIZE: usize = 16;

const ECHO_HIST_SIZE: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
enum EnvMode {
    #[default]
    Release,
    Attack,
    Decay,
    Sustain,
}

const BRR_BUF_SIZE: usize = 12;

#[derive(Clone, Copy, Debug, Default)]
struct Voice {
    buf: [i32; BRR_BUF_SIZE * 2], // decoded samples (twice the size to simplify wrap handling)
    buf_pos: usize,               // place in buffer where next samples will be decoded
    interp_pos: i32,              // relative fractional position in sample (0x1000 = 1.0)
    brr_addr: i32,                // address of current BRR block
    brr_offset: i32,              // current decoding offset in BRR block
    kon_delay: i32,               // KON delay/current setup phase
    env_mode: EnvMode,
    env: i32,         // current envelope level
    hidden_env: i32,  // used by GAIN mode 7, very obscure quirk
    volume: [i32; 2], // copy of volume from DSP registers, with surround disabled
    enabled: i32,     // -1 if enabled, 0 if muted
}

/// A place the DSP writes samples to: the main output buffer, or its own
/// `extra` buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Out {
    pub extra: bool,
    pub pos: usize,
}

#[derive(Clone, Debug)]
pub struct SpcDsp {
    regs: [u8; REGISTER_COUNT],

    // Echo history keeps most recent 8 samples (twice the size to simplify wrap handling)
    echo_hist: [[i32; 2]; ECHO_HIST_SIZE * 2],
    echo_hist_pos: usize, // &echo_hist [0 to 7]

    every_other_sample: i32, // toggles every sample
    kon: i32,                // KON value when last checked
    noise: i32,
    echo_offset: i32, // offset from ESA in echo buffer
    echo_length: i32, // number of bytes that echo_offset will stop at
    phase: i32,       // next clock cycle to run (0-31)
    counters: [u32; 4],

    new_kon: i32,
    t_koff: i32,

    voices: [Voice; VOICE_COUNT],

    counter_select: [usize; 32],

    // non-emulation state
    mute_mask: i32,
    surround_threshold: i32,
    echo_enable: i32,
    out: Out,
    out_end: Out,
    out_begin: Out,
    pub extra: [Sample; EXTRA_SIZE],
}

static INITIAL_REGS: [u8; REGISTER_COUNT] = [
    0x45, 0x8B, 0x5A, 0x9A, 0xE4, 0x82, 0x1B, 0x78, 0x00, 0x00, 0xAA, 0x96, 0x89, 0x0E, 0xE0, 0x80,
    0x2A, 0x49, 0x3D, 0xBA, 0x14, 0xA0, 0xAC, 0xC5, 0x00, 0x00, 0x51, 0xBB, 0x9C, 0x4E, 0x7B, 0xFF,
    0xF4, 0xFD, 0x57, 0x32, 0x37, 0xD9, 0x42, 0x22, 0x00, 0x00, 0x5B, 0x3C, 0x9F, 0x1B, 0x87, 0x9A,
    0x6F, 0x27, 0xAF, 0x7B, 0xE5, 0x68, 0x0A, 0xD9, 0x00, 0x00, 0x9A, 0xC5, 0x9C, 0x4E, 0x7B, 0xFF,
    0xEA, 0x21, 0x78, 0x4F, 0xDD, 0xED, 0x24, 0x14, 0x00, 0x00, 0x77, 0xB1, 0xD1, 0x36, 0xC1, 0x67,
    0x52, 0x57, 0x46, 0x3D, 0x59, 0xF4, 0x87, 0xA4, 0x00, 0x00, 0x7E, 0x44, 0x9C, 0x4E, 0x7B, 0xFF,
    0x75, 0xF5, 0x06, 0x97, 0x10, 0xC3, 0x24, 0xBB, 0x00, 0x00, 0x7B, 0x7A, 0xE0, 0x60, 0x12, 0x0F,
    0xF7, 0x74, 0x1C, 0xE5, 0x39, 0x3D, 0x73, 0xC1, 0x00, 0x00, 0x7A, 0xB3, 0xFF, 0x4E, 0x7B, 0xFF,
];

// if ( io < -32768 ) io = -32768;
// if ( io >  32767 ) io =  32767;
macro_rules! clamp16 {
    ($io:expr) => {
        if $io as i16 as i32 != $io {
            $io = ($io >> 31) ^ 0x7FFF;
        }
    };
}

// Interleved gauss table (to improve cache coherency)
// interleved_gauss [i] = gauss [(i & 1) * 256 + 255 - (i >> 1 & 0xFF)]
static INTERLEVED_GAUSS: [i16; 512] = [
    370, 1305, 366, 1305, 362, 1304, 358, 1304, 354, 1304, 351, 1304, 347, 1304, 343, 1303, //
    339, 1303, 336, 1303, 332, 1302, 328, 1302, 325, 1301, 321, 1300, 318, 1300, 314, 1299, //
    311, 1298, 307, 1297, 304, 1297, 300, 1296, 297, 1295, 293, 1294, 290, 1293, 286, 1292, //
    283, 1291, 280, 1290, 276, 1288, 273, 1287, 270, 1286, 267, 1284, 263, 1283, 260, 1282, //
    257, 1280, 254, 1279, 251, 1277, 248, 1275, 245, 1274, 242, 1272, 239, 1270, 236, 1269, //
    233, 1267, 230, 1265, 227, 1263, 224, 1261, 221, 1259, 218, 1257, 215, 1255, 212, 1253, //
    210, 1251, 207, 1248, 204, 1246, 201, 1244, 199, 1241, 196, 1239, 193, 1237, 191, 1234, //
    188, 1232, 186, 1229, 183, 1227, 180, 1224, 178, 1221, 175, 1219, 173, 1216, 171, 1213, //
    168, 1210, 166, 1207, 163, 1205, 161, 1202, 159, 1199, 156, 1196, 154, 1193, 152, 1190, //
    150, 1186, 147, 1183, 145, 1180, 143, 1177, 141, 1174, 139, 1170, 137, 1167, 134, 1164, //
    132, 1160, 130, 1157, 128, 1153, 126, 1150, 124, 1146, 122, 1143, 120, 1139, 118, 1136, //
    117, 1132, 115, 1128, 113, 1125, 111, 1121, 109, 1117, 107, 1113, 106, 1109, 104, 1106, //
    102, 1102, 100, 1098, 99, 1094, 97, 1090, 95, 1086, 94, 1082, 92, 1078, 90, 1074, //
    89, 1070, 87, 1066, 86, 1061, 84, 1057, 83, 1053, 81, 1049, 80, 1045, 78, 1040, //
    77, 1036, 76, 1032, 74, 1027, 73, 1023, 71, 1019, 70, 1014, 69, 1010, 67, 1005, //
    66, 1001, 65, 997, 64, 992, 62, 988, 61, 983, 60, 978, 59, 974, 58, 969, //
    56, 965, 55, 960, 54, 955, 53, 951, 52, 946, 51, 941, 50, 937, 49, 932, //
    48, 927, 47, 923, 46, 918, 45, 913, 44, 908, 43, 904, 42, 899, 41, 894, //
    40, 889, 39, 884, 38, 880, 37, 875, 36, 870, 36, 865, 35, 860, 34, 855, //
    33, 851, 32, 846, 32, 841, 31, 836, 30, 831, 29, 826, 29, 821, 28, 816, //
    27, 811, 27, 806, 26, 802, 25, 797, 24, 792, 24, 787, 23, 782, 23, 777, //
    22, 772, 21, 767, 21, 762, 20, 757, 20, 752, 19, 747, 19, 742, 18, 737, //
    17, 732, 17, 728, 16, 723, 16, 718, 15, 713, 15, 708, 15, 703, 14, 698, //
    14, 693, 13, 688, 13, 683, 12, 678, 12, 674, 11, 669, 11, 664, 11, 659, //
    10, 654, 10, 649, 10, 644, 9, 640, 9, 635, 9, 630, 8, 625, 8, 620, //
    8, 615, 7, 611, 7, 606, 7, 601, 6, 596, 6, 592, 6, 587, 6, 582, //
    5, 577, 5, 573, 5, 568, 5, 563, 4, 559, 4, 554, 4, 550, 4, 545, //
    4, 540, 3, 536, 3, 531, 3, 527, 3, 522, 3, 517, 2, 513, 2, 508, //
    2, 504, 2, 499, 2, 495, 2, 491, 2, 486, 1, 482, 1, 477, 1, 473, //
    1, 469, 1, 464, 1, 460, 1, 456, 1, 451, 1, 447, 1, 443, 1, 439, //
    0, 434, 0, 430, 0, 426, 0, 422, 0, 418, 0, 414, 0, 410, 0, 405, //
    0, 401, 0, 397, 0, 393, 0, 389, 0, 385, 0, 381, 0, 378, 0, 374,
];

// Counters

const fn rate(rate: u32, div: u32) -> u32 {
    if rate >= div {
        rate / div * 8 - 1
    } else {
        rate - 1
    }
}

static COUNTER_MASK: [u32; 32] = [
    rate(2, 2),
    rate(2048, 4),
    rate(1536, 3),
    rate(1280, 5),
    rate(1024, 4),
    rate(768, 3),
    rate(640, 5),
    rate(512, 4),
    rate(384, 3),
    rate(320, 5),
    rate(256, 4),
    rate(192, 3),
    rate(160, 5),
    rate(128, 4),
    rate(96, 3),
    rate(80, 5),
    rate(64, 4),
    rate(48, 3),
    rate(40, 5),
    rate(32, 4),
    rate(24, 3),
    rate(20, 5),
    rate(16, 4),
    rate(12, 3),
    rate(10, 5),
    rate(8, 4),
    rate(6, 3),
    rate(5, 5),
    rate(4, 4),
    rate(3, 3),
    rate(2, 4),
    rate(1, 4),
];

#[inline]
fn get_le16a(ram: &[u8], i: usize) -> i32 {
    ram[i] as i32 | (ram[i + 1] as i32) << 8
}

impl Default for SpcDsp {
    fn default() -> Self {
        Self::new()
    }
}

impl SpcDsp {
    pub fn new() -> Self {
        // memset(&m, 0, sizeof(state_t));
        SpcDsp {
            regs: [0; REGISTER_COUNT],
            echo_hist: [[0; 2]; ECHO_HIST_SIZE * 2],
            echo_hist_pos: 0,
            every_other_sample: 0,
            kon: 0,
            noise: 0,
            echo_offset: 0,
            echo_length: 0,
            phase: 0,
            counters: [0; 4],
            new_kon: 0,
            t_koff: 0,
            voices: [Voice::default(); VOICE_COUNT],
            counter_select: [0; 32],
            mute_mask: 0,
            surround_threshold: 0,
            echo_enable: 0,
            out: Out::default(),
            out_end: Out::default(),
            out_begin: Out::default(),
            extra: [0; EXTRA_SIZE],
        }
    }

    // Setup

    /// Initializes DSP (and has it use the 64K RAM passed to the functions
    /// using it)
    pub fn init(&mut self) {
        self.mute_voices(0);
        self.disable_surround(false);
        self.disable_echo(false);
        self.set_output(None);
        self.reset();

        // (the checks that the compiler sign-extends and that the clamp
        // macro works)
    }

    /// Sets destination for output samples (`Some((the place in the main
    /// buffer, its size))`). If out is NULL or out_size is 0, doesn't
    /// generate any.
    pub fn set_output(&mut self, out: Option<(usize, usize)>) {
        let (out, size) = match out {
            // require( (size & 1) == 0 ); // must be even
            Some((pos, size)) => (Out { extra: false, pos }, size),
            None => (
                Out {
                    extra: true,
                    pos: 0,
                },
                EXTRA_SIZE,
            ),
        };
        self.out_begin = out;
        self.out = out;
        self.out_end = Out {
            extra: out.extra,
            pos: out.pos + size,
        };
    }

    /// Sets destination for output samples in the DSP's own extra buffer
    /// (`set_output( extra () + pos, size )`).
    pub fn set_output_extra(&mut self, pos: usize, size: usize) {
        let out = Out { extra: true, pos };
        self.out_begin = out;
        self.out = out;
        self.out_end = Out {
            extra: true,
            pos: pos + size,
        };
    }

    /// Number of samples written to output since it was last set, always
    /// a multiple of 2. Undefined if more samples were generated than
    /// output buffer could hold.
    pub fn sample_count(&self) -> i32 {
        self.out.pos as i32 - self.out_begin.pos as i32
    }

    pub fn out_pos(&self) -> Out {
        self.out
    }

    // Emulation

    /// Reads/writes DSP registers. For accuracy, you must first call spc_run_dsp()
    /// to catch the DSP up to present.
    pub fn read(&self, addr: usize) -> i32 {
        // assert( (unsigned) addr < register_count );
        self.regs[addr] as i32
    }

    pub fn write(&mut self, addr: usize, data: i32) {
        // assert( (unsigned) addr < register_count );

        self.regs[addr] = data as u8;
        let low = addr & 0x0F;
        if low < 0x2 {
            // voice volumes
            self.update_voice_vol(low ^ addr);
        } else if low == 0xC {
            if addr == R_KON {
                self.new_kon = data as u8 as i32;
            }

            if addr == R_ENDX {
                // always cleared, regardless of data written
                self.regs[R_ENDX] = 0;
            }
        }
    }

    #[inline]
    fn update_voice_vol(&mut self, addr: usize) {
        let mut l = self.regs[addr + V_VOLL] as i8 as i32;
        let mut r = self.regs[addr + V_VOLR] as i8 as i32;

        if l * r < self.surround_threshold {
            // signs differ, so negate those that are negative
            l ^= l >> 7;
            r ^= r >> 7;
        }

        let v = &mut self.voices[addr >> 4];
        let enabled = v.enabled;
        v.volume[0] = l & enabled;
        v.volume[1] = r & enabled;
    }

    /// If true, prevents channels and global volumes from being phase-negated
    pub fn disable_surround(&mut self, disable: bool) {
        self.surround_threshold = if disable { 0 } else { -0x4000 };
    }

    pub fn disable_echo(&mut self, disable: bool) {
        self.echo_enable = !disable as i32;
    }

    // Counters

    #[inline]
    fn init_counter(&mut self) {
        // counters start out with this synchronization
        self.counters[0] = 1;
        self.counters[1] = 0;
        self.counters[2] = 0x20u32.wrapping_neg();
        self.counters[3] = 0x0B;

        let mut n = 2;
        for i in 1..32 {
            self.counter_select[i] = n;
            n -= 1;
            if n == 0 {
                n = 3;
            }
        }
        self.counter_select[0] = 0;
        self.counter_select[30] = 2;
    }

    #[inline]
    fn run_counter(&mut self, i: usize) {
        let mut n = self.counters[i] as i32;
        let old = n;
        n = n.wrapping_sub(1);
        if old & 7 == 0 {
            n = n.wrapping_sub(6 - i as i32);
        }
        self.counters[i] = n as u32;
    }

    #[inline]
    fn read_counter(&self, rate: usize) -> u32 {
        self.counters[self.counter_select[rate]] & COUNTER_MASK[rate]
    }

    #[inline]
    fn write_samples(&mut self, main: &mut [Sample], l: i32, r: i32) {
        let mut out = self.out;
        {
            let buf: &mut [Sample] = if out.extra { &mut self.extra } else { main };
            buf[out.pos] = l as Sample;
            buf[out.pos + 1] = r as Sample;
        }
        out.pos += 2;
        if out.pos >= self.out_end.pos {
            // check( out == m.out_end );
            // check( m.out_end != &m.extra [extra_size] || ... );
            out = Out {
                extra: true,
                pos: 0,
            };
            self.out_end = Out {
                extra: true,
                pos: EXTRA_SIZE,
            };
        }
        self.out = out;
    }

    // Emulation

    /// Runs DSP for specified number of clocks (~1024000 per second). Every 32 clocks
    /// a pair of samples is be generated.
    pub fn run(&mut self, ram: &mut [u8], main: &mut [Sample], clock_count: i32) {
        let new_phase = self.phase + clock_count;
        let mut count = new_phase >> 5;
        self.phase = new_phase & 31;
        if count == 0 {
            return;
        }

        let dir = self.regs[R_DIR] as usize * 0x100;
        let slow_gaussian = (self.regs[R_PMON] as i32 >> 1) | self.regs[R_NON] as i32;
        let noise_rate = (self.regs[R_FLG] & 0x1F) as usize;

        // Global volume
        let mut mvoll = self.regs[R_MVOLL] as i8 as i32;
        let mut mvolr = self.regs[R_MVOLR] as i8 as i32;
        let mut evoll = self.regs[R_EVOLL] as i8 as i32;
        let mut evolr = self.regs[R_EVOLR] as i8 as i32;

        if self.echo_enable == 0 {
            mvoll = 127;
            mvolr = 127;
            evoll = 0;
            evolr = 0;
        }

        if mvoll * mvolr < self.surround_threshold {
            mvoll = -mvoll; // eliminate surround
        }

        loop {
            // KON/KOFF reading
            self.every_other_sample ^= 1;
            if self.every_other_sample != 0 {
                self.new_kon &= !self.kon;
                self.kon = self.new_kon;
                self.t_koff = self.regs[R_KOFF] as i32;
            }

            self.run_counter(1);
            self.run_counter(2);
            self.run_counter(3);

            // Noise
            if self.read_counter(noise_rate) == 0 {
                let feedback = (self.noise << 13) ^ (self.noise << 14);
                self.noise = (feedback & 0x4000) ^ (self.noise >> 1);
            }

            // Voices
            let mut pmon_input = 0;
            let mut main_out_l: i32 = 0;
            let mut main_out_r: i32 = 0;
            let mut echo_out_l: i32 = 0;
            let mut echo_out_r: i32 = 0;
            let mut vi = 0usize;
            let mut v_regs = 0usize;
            let mut vbit: i32 = 1;
            loop {
                macro_rules! sample_ptr {
                    ($i:expr) => {
                        get_le16a(ram, dir + self.regs[v_regs + V_SRCN] as usize * 4 + $i * 2)
                    };
                }

                let v = &mut self.voices[vi];
                let mut brr_header = ram[v.brr_addr as usize] as i32;
                let mut kon_delay = v.kon_delay;

                // Pitch
                let mut pitch = get_le16a(&self.regs, v_regs + V_PITCHL) & 0x3FFF;
                if self.regs[R_PMON] as i32 & vbit != 0 {
                    pitch += ((pmon_input >> 5) * pitch) >> 10;
                }

                // KON phases
                kon_delay -= 1;
                if kon_delay >= 0 {
                    v.kon_delay = kon_delay;

                    // Get ready to start BRR decoding on next sample
                    if kon_delay == 4 {
                        v.brr_addr = get_le16a(ram, dir + self.regs[v_regs + V_SRCN] as usize * 4);
                        v.brr_offset = 1;
                        v.buf_pos = 0;
                        brr_header = 0; // header is ignored on this sample
                    }

                    // Envelope is never run during KON
                    v.env = 0;
                    v.hidden_env = 0;

                    // Disable BRR decoding until last three samples
                    v.interp_pos = if kon_delay & 3 != 0 { 0x4000 } else { 0 };

                    // Pitch is never added during KON
                    pitch = 0;
                }

                let mut env = v.env;

                // Gaussian interpolation
                {
                    let mut output: i32 = 0;
                    self.regs[v_regs + V_ENVX] = (env >> 4) as u8;
                    let v = &mut self.voices[vi];
                    if env != 0 {
                        // Make pointers into gaussian based on fractional position between samples
                        let offset = (v.interp_pos as u32 >> 3 & 0x1FE) as usize;
                        let fwd = &INTERLEVED_GAUSS[offset..];
                        let rev = &INTERLEVED_GAUSS[510 - offset..]; // mirror left half of gaussian

                        let inp = &v.buf[v.buf_pos + (v.interp_pos as u32 >> 12) as usize..];

                        if slow_gaussian & vbit == 0 {
                            // 99%
                            // Faster approximation when exact sample value isn't necessary for pitch mod
                            output = (fwd[0] as i32 * inp[0]
                                + fwd[1] as i32 * inp[1]
                                + rev[1] as i32 * inp[2]
                                + rev[0] as i32 * inp[3])
                                >> 11;
                            output = (output * env) >> 11;
                        } else {
                            output = (self.noise * 2) as i16 as i32;
                            if self.regs[R_NON] as i32 & vbit == 0 {
                                output = (fwd[0] as i32 * inp[0]) >> 11;
                                output += (fwd[1] as i32 * inp[1]) >> 11;
                                output += (rev[1] as i32 * inp[2]) >> 11;
                                output = output as i16 as i32;
                                output += (rev[0] as i32 * inp[3]) >> 11;

                                clamp16!(output);
                                output &= !1;
                            }
                            output = (output * env) >> 11 & !1;
                        }

                        // Output
                        let l = output * v.volume[0];
                        let r = output * v.volume[1];

                        main_out_l = main_out_l.wrapping_add(l);
                        main_out_r = main_out_r.wrapping_add(r);

                        if self.regs[R_EON] as i32 & vbit != 0 {
                            echo_out_l = echo_out_l.wrapping_add(l);
                            echo_out_r = echo_out_r.wrapping_add(r);
                        }
                    }

                    pmon_input = output;
                    self.regs[v_regs + V_OUTX] = (output >> 8) as u8;
                }

                let flg = self.regs[R_FLG];
                let v = &mut self.voices[vi];

                // Soft reset or end of sample
                if flg & 0x80 != 0 || (brr_header & 3) == 1 {
                    v.env_mode = EnvMode::Release;
                    env = 0;
                }

                if self.every_other_sample != 0 {
                    // KOFF
                    if self.t_koff & vbit != 0 {
                        v.env_mode = EnvMode::Release;
                    }

                    // KON
                    if self.kon & vbit != 0 {
                        v.kon_delay = 5;
                        v.env_mode = EnvMode::Attack;
                        self.regs[R_ENDX] &= !vbit as u8;
                    }
                }

                let v = &mut self.voices[vi];
                let mut skip_brr = false;

                // Envelope
                if v.kon_delay == 0 {
                    if v.env_mode == EnvMode::Release {
                        // 97%
                        env -= 0x8;
                        v.env = env;
                        if env <= 0 {
                            v.env = 0;
                            skip_brr = true; // no BRR decoding for you!
                        }
                    } else {
                        // 3%
                        let adsr0 = self.regs[v_regs + V_ADSR0] as i32;
                        let mut env_data = self.regs[v_regs + V_ADSR1] as i32;
                        let mut exit_env = false;
                        let rate: usize = if adsr0 >= 0x80 {
                            // 97% ADSR
                            if v.env_mode > EnvMode::Decay {
                                // 89%
                                env -= 1;
                                env -= env >> 8;
                                let rate = (env_data & 0x1F) as usize;

                                // optimized handling
                                v.hidden_env = env;
                                if self.counters[self.counter_select[rate]] & COUNTER_MASK[rate]
                                    == 0
                                {
                                    v.env = env;
                                }
                                exit_env = true;
                                0
                            } else if v.env_mode == EnvMode::Decay {
                                env -= 1;
                                env -= env >> 8;
                                ((adsr0 >> 3 & 0x0E) + 0x10) as usize
                            } else {
                                // env_attack
                                let rate = ((adsr0 & 0x0F) * 2 + 1) as usize;
                                env += if rate < 31 { 0x20 } else { 0x400 };
                                rate
                            }
                        } else {
                            // GAIN
                            env_data = self.regs[v_regs + V_GAIN] as i32;
                            let mode = env_data >> 5;
                            if mode < 4 {
                                // direct
                                env = env_data * 0x10;
                                31
                            } else {
                                let rate = (env_data & 0x1F) as usize;
                                if mode == 4 {
                                    // 4: linear decrease
                                    env -= 0x20;
                                } else if mode < 6 {
                                    // 5: exponential decrease
                                    env -= 1;
                                    env -= env >> 8;
                                } else {
                                    // 6,7: linear increase
                                    env += 0x20;
                                    if mode > 6 && v.hidden_env as u32 >= 0x600 {
                                        env += 0x8 - 0x20; // 7: two-slope linear increase
                                    }
                                }
                                rate
                            }
                        };

                        if !exit_env {
                            // Sustain level
                            if (env >> 8) == (env_data >> 5) && v.env_mode == EnvMode::Decay {
                                v.env_mode = EnvMode::Sustain;
                            }

                            v.hidden_env = env;

                            // unsigned cast because linear decrease going negative also triggers this
                            if env as u32 > 0x7FF {
                                env = if env < 0 { 0 } else { 0x7FF };
                                if v.env_mode == EnvMode::Attack {
                                    v.env_mode = EnvMode::Decay;
                                }
                            }

                            if self.counters[self.counter_select[rate]] & COUNTER_MASK[rate] == 0 {
                                v.env = env; // nothing else is controlled by the counter
                            }
                        }
                    }
                }
                // exit_env:

                if !skip_brr {
                    // Apply pitch
                    let old_pos = v.interp_pos;
                    let mut interp_pos = (old_pos & 0x3FFF) + pitch;
                    if interp_pos > 0x7FFF {
                        interp_pos = 0x7FFF;
                    }
                    v.interp_pos = interp_pos;

                    // BRR decode if necessary
                    if old_pos >= 0x4000 {
                        // Arrange the four input nybbles in 0xABCD order for easy decoding
                        let mut nybbles: i32 =
                            ram[((v.brr_addr + v.brr_offset) & 0xFFFF) as usize] as i32 * 0x100
                                + ram[((v.brr_addr + v.brr_offset + 1) & 0xFFFF) as usize] as i32;

                        // Advance read position
                        const BRR_BLOCK_SIZE: i32 = 9;
                        let mut brr_offset = v.brr_offset;
                        brr_offset += 2;
                        if brr_offset >= BRR_BLOCK_SIZE {
                            // Next BRR block
                            let mut brr_addr = (v.brr_addr + BRR_BLOCK_SIZE) & 0xFFFF;
                            // assert( brr_offset == brr_block_size );
                            if brr_header & 1 != 0 {
                                brr_addr = sample_ptr!(1);
                                if v.kon_delay == 0 {
                                    self.regs[R_ENDX] |= vbit as u8;
                                }
                            }
                            let v = &mut self.voices[vi];
                            v.brr_addr = brr_addr;
                            brr_offset = 1;
                        }
                        let v = &mut self.voices[vi];
                        v.brr_offset = brr_offset;

                        // Decode

                        // 0: >>1  1: <<0  2: <<1 ... 12: <<11  13-15: >>4 <<11
                        static SHIFTS: [u8; 16 * 2] = [
                            13, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 16, 16, 16, //
                            0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 11, 11, 11,
                        ];
                        let scale = (brr_header >> 4) as usize;
                        let right_shift = SHIFTS[scale] as u32;
                        let left_shift = SHIFTS[scale + 16] as u32;

                        // Write to next four samples in circular buffer
                        let mut pos = v.buf_pos;
                        let end = pos + 4;

                        // Decode four samples
                        while pos < end {
                            // Extract upper nybble and scale appropriately. Every cast is
                            // necessary to maintain correctness and avoid undef behavior
                            let mut s = ((((nybbles as i16 as i32) >> right_shift) as u16 as u32)
                                << left_shift) as u16 as i16
                                as i32;

                            // Apply IIR filter (8 is the most commonly used)
                            let filter = brr_header & 0x0C;
                            let p1 = v.buf[pos + BRR_BUF_SIZE - 1];
                            let p2 = v.buf[pos + BRR_BUF_SIZE - 2] >> 1;
                            if filter >= 8 {
                                s += p1;
                                s -= p2;
                                if filter == 8 {
                                    // s += p1 * 0.953125 - p2 * 0.46875
                                    s += p2 >> 4;
                                    s += (p1 * -3) >> 6;
                                } else {
                                    // s += p1 * 0.8984375 - p2 * 0.40625
                                    s += (p1 * -13) >> 7;
                                    s += (p2 * 3) >> 4;
                                }
                            } else if filter != 0 {
                                // s += p1 * 0.46875
                                s += p1 >> 1;
                                s += (-p1) >> 5;
                            }

                            // Adjust and write sample
                            clamp16!(s);
                            s = (s * 2) as i16 as i32;
                            v.buf[pos + BRR_BUF_SIZE] = s;
                            v.buf[pos] = s; // second copy simplifies wrap-around

                            pos += 1;
                            nybbles <<= 4;
                        }

                        if pos >= BRR_BUF_SIZE {
                            pos = 0;
                        }
                        v.buf_pos = pos;
                    }
                }
                // skip_brr:
                // Next voice
                vbit <<= 1;
                v_regs += 0x10;
                vi += 1;
                if vbit >= 0x100 {
                    break;
                }
            }

            // Echo position
            let mut echo_offset = self.echo_offset;
            let echo_ptr = ((self.regs[R_ESA] as i32 * 0x100 + echo_offset) & 0xFFFF) as usize;
            if echo_offset == 0 {
                self.echo_length = (self.regs[R_EDL] & 0x0F) as i32 * 0x800;
            }
            echo_offset += 4;
            if echo_offset >= self.echo_length {
                echo_offset = 0;
            }
            self.echo_offset = echo_offset;

            // FIR
            let mut echo_in_l = get_le16a(ram, echo_ptr) as i16 as i32;
            let mut echo_in_r = get_le16a(ram, echo_ptr + 2) as i16 as i32;

            let mut echo_hist_pos = self.echo_hist_pos + 1;
            if echo_hist_pos >= ECHO_HIST_SIZE {
                echo_hist_pos = 0;
            }
            self.echo_hist_pos = echo_hist_pos;

            self.echo_hist[echo_hist_pos][0] = echo_in_l;
            self.echo_hist[echo_hist_pos + 8][0] = echo_in_l;
            self.echo_hist[echo_hist_pos][1] = echo_in_r;
            self.echo_hist[echo_hist_pos + 8][1] = echo_in_r;

            macro_rules! calc_fir_ {
                ($i:expr, $in:expr) => {
                    ($in).wrapping_mul(self.regs[R_FIR + $i * 0x10] as i8 as i32)
                };
            }
            echo_in_l = calc_fir_!(7, echo_in_l);
            echo_in_r = calc_fir_!(7, echo_in_r);

            for i in 0..7 {
                // DO_FIR( i )
                echo_in_l =
                    echo_in_l.wrapping_add(calc_fir_!(i, self.echo_hist[echo_hist_pos + i + 1][0]));
                echo_in_r =
                    echo_in_r.wrapping_add(calc_fir_!(i, self.echo_hist[echo_hist_pos + i + 1][1]));
            }

            // Echo out
            if self.regs[R_FLG] & 0x20 == 0 {
                let efb = self.regs[R_EFB] as i8 as i32;
                let mut l = (echo_out_l >> 7).wrapping_add(echo_in_l.wrapping_mul(efb) >> 14);
                let mut r = (echo_out_r >> 7).wrapping_add(echo_in_r.wrapping_mul(efb) >> 14);

                // just to help pass more validation tests
                // (#if SPC_MORE_ACCURACY: l &= ~1; r &= ~1;)

                clamp16!(l);
                clamp16!(r);

                ram[echo_ptr] = l as u8;
                ram[echo_ptr + 1] = (l >> 8) as u8;
                ram[echo_ptr + 2] = r as u8;
                ram[echo_ptr + 3] = (r >> 8) as u8;
            }

            // Sound out
            let mut l = main_out_l
                .wrapping_mul(mvoll)
                .wrapping_add(echo_in_l.wrapping_mul(evoll))
                >> 14;
            let mut r = main_out_r
                .wrapping_mul(mvolr)
                .wrapping_add(echo_in_r.wrapping_mul(evolr))
                >> 14;

            clamp16!(l);
            clamp16!(r);

            if self.regs[R_FLG] & 0x40 != 0 {
                l = 0;
                r = 0;
            }

            self.write_samples(main, l, r);

            count -= 1;
            if count == 0 {
                break;
            }
        }
    }

    // Setup

    pub fn mute_voices(&mut self, mask: i32) {
        self.mute_mask = mask;
        for i in 0..VOICE_COUNT {
            self.voices[i].enabled = (mask >> i & 1) - 1;
            self.update_voice_vol(i * 0x10);
        }
    }

    fn soft_reset_common(&mut self) {
        // require( m.ram ); // init() must have been called already

        self.noise = 0x4000;
        self.echo_hist_pos = 0;
        self.every_other_sample = 1;
        self.echo_offset = 0;
        self.phase = 0;

        self.init_counter();
    }

    /// Emulates pressing reset switch on SNES
    pub fn soft_reset(&mut self) {
        self.regs[R_FLG] = 0xE0;
        self.soft_reset_common();
    }

    pub fn load(&mut self, regs: &[u8]) {
        self.regs.copy_from_slice(&regs[..REGISTER_COUNT]);
        // memset( &m.regs [register_count], 0, offsetof (state_t,ram) - register_count );
        self.echo_hist = [[0; 2]; ECHO_HIST_SIZE * 2];
        self.echo_hist_pos = 0;
        self.every_other_sample = 0;
        self.kon = 0;
        self.noise = 0;
        self.echo_offset = 0;
        self.echo_length = 0;
        self.phase = 0;
        self.counters = [0; 4];
        self.new_kon = 0;
        self.t_koff = 0;
        self.voices = [Voice::default(); VOICE_COUNT];
        self.counter_select = [0; 32];

        // Internal state
        for i in (0..VOICE_COUNT).rev() {
            let v = &mut self.voices[i];
            v.brr_offset = 1;
            v.buf_pos = 0;
        }
        self.new_kon = self.regs[R_KON] as i32;

        let mask = self.mute_mask;
        self.mute_voices(mask);
        self.soft_reset_common();
    }

    /// Resets DSP to power-on state
    pub fn reset(&mut self) {
        self.load(&INITIAL_REGS);
    }
}
