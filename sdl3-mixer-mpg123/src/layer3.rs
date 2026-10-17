/*
    layer3.c: the layer 3 decoder

    copyright 1995-2021 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Michael Hipp

    Dear visitor:
    If you feel you don't understand fully the works of this file, your feeling might be correct.

    Optimize-TODO: put short bands into the band-field without the stride of 3 reals
    Length-optimze: unify long and short band code where it is possible

    The int-vs-pointer situation has to be cleaned up.
*/
// Modified 2026-10-17: translated into Rust from layer3.c in SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

//! The layer III decoder, with the new Huffman tables (`USE_NEW_HUFFTABLE`)
//! and the plain C `INT123_dct36()` (no `OPT_THE_DCT36` in a build of the
//! generic decoder).

use crate::frame::*;
use crate::getbits::*;
use crate::l3bandgain::BAND_INFO;
use crate::l3tabs::*;
use crate::newhuffman::{HT, HTC};
use crate::parse::set_pointer;

/* define CUT_SFB21 if you want to cut-off the frequency above 16kHz */

/* Decoder state data, living on the stack of INT123_do_layer3. */

/// Translation of `struct gr_info_s`. `full_gain` and `pow2gain` are
/// offsets into `fr->gainpow2`.
#[derive(Clone, Copy, Default)]
struct GrInfo {
    scfsi: i32,
    part2_3_length: u32,
    big_values: u32,
    scalefac_compress: u32,
    block_type: u32,
    mixed_block_flag: u32,
    table_select: [u32; 3],
    /* Making those two signed int as workaround for open64/pathscale/sun compilers, and also for consistency, since they're worked on together with other signed variables. */
    maxband: [i32; 3],
    maxbandl: i32,
    maxb: u32,
    region1start: u32,
    region2start: u32,
    preflag: u32,
    scalefac_scale: u32,
    count1table_select: u32,
    full_gain: [usize; 3],
    pow2gain: usize,
}

/// Translation of `struct III_sideinfo`: `ch[ch][gr]`.
#[derive(Clone, Copy, Default)]
struct IIISideinfo {
    main_data_begin: u32,
    private_bits: u32,
    /* Hm, funny... struct inside struct... */
    ch: [[GrInfo; 2]; 2],
}

/// Translation of `INT123_init_layer3_gainpow2()`. `pow()` is SDL's
/// (fdlibm's), which gives the same table as the C library's.
pub(crate) fn init_layer3_gainpow2(_fr: &mut Mpg123Handle<'_>, i: i32) -> f32 {
    sdl3::stdlib::math::pow(2.0, -0.25 * (i + 210) as f64) as f32
}

/// Translation of `INT123_init_layer3_stuff()`.
pub(crate) fn init_layer3_stuff(
    fr: &mut Mpg123Handle<'_>,
    gainpow2_func: fn(&mut Mpg123Handle<'_>, i32) -> f32,
) {
    for i in -256..118 + 4 {
        let v = gainpow2_func(fr, i);
        fr.gainpow2_muls[(i + 256) as usize] = v;
    }

    for j in 0..9 {
        for i in 0..23 {
            fr.long_limit[j][i] = (BAND_INFO[j].long_idx[i] as i32 - 1 + 8) / 18 + 1;
            if fr.long_limit[j][i] > fr.down_sample_sblimit {
                fr.long_limit[j][i] = fr.down_sample_sblimit;
            }
        }
        for i in 0..14 {
            fr.short_limit[j][i] = (BAND_INFO[j].short_idx[i] as i32 - 1) / 18 + 1;
            if fr.short_limit[j][i] > fr.down_sample_sblimit {
                fr.short_limit[j][i] = fr.down_sample_sblimit;
            }
        }
    }
}

/*
    Observe!
    Now come the actualy decoding routines.
*/

/* read additional side information (for MPEG 1 and MPEG 2) */
fn iii_get_side_info(
    fr: &mut Mpg123Handle<'_>,
    si: &mut IIISideinfo,
    stereo: i32,
    ms_stereo: i32,
    sfreq: usize,
    single: i32,
) -> i32 {
    let powdiff = if single == SINGLE_MIX { 4 } else { 0 };

    const TABS: [[i32; 5]; 2] = [[2, 9, 5, 3, 4], [1, 8, 1, 2, 9]];
    let tab = &TABS[fr.hdr.lsf as usize];

    {
        /* First ensure we got enough bits available. */
        let mut needbits: u32 = 0;
        needbits += tab[1] as u32; /* main_data_begin */
        needbits += if stereo == 1 { tab[2] } else { tab[3] } as u32; /* private */
        if fr.hdr.lsf == 0 {
            needbits += stereo as u32 * 4; /* scfsi */
        }
        /* For each granule for each channel ... */
        needbits +=
            (tab[0] * stereo * (29 + tab[4] + 1 + 22 + (if fr.hdr.lsf == 0 { 1 } else { 0 }) + 2))
                as u32;
        if fr.bits_avail < needbits as i64 {
            // %u bits for side info needed, only %li available
            return 1;
        }
    }

    si.main_data_begin = getbits(fr, tab[1]);

    if si.main_data_begin > fr.bitreservoir {
        /*  overwrite main_data_begin for the really available bit reservoir */
        backbits(fr, tab[1]);
        let w = fr.wordpointer;
        if fr.hdr.lsf == 0 {
            fr.bsspace[w] = (fr.bitreservoir >> 1) as u8;
            fr.bsspace[w + 1] = ((fr.bitreservoir & 1) << 7) as u8;
        } else {
            fr.bsspace[w] = fr.bitreservoir as u8;
        }

        /* zero "side-info" data for a silence-frame
        without touching audio data used as bit reservoir for following frame */
        let n = (fr.hdr.ssize - 2) as usize;
        fr.bsspace[w + 2..w + 2 + n].fill(0);

        /* reread the new bit reservoir offset */
        si.main_data_begin = getbits(fr, tab[1]);
    }

    /* Keep track of the available data bytes for the bit reservoir.
    CRC is included in ssize already. */
    fr.bitreservoir = fr
        .bitreservoir
        .wrapping_add((fr.hdr.framesize - fr.hdr.ssize) as u32);

    /* Limit the reservoir to the max for MPEG 1.0 or 2.x . */
    let lim = if fr.hdr.lsf == 0 { 511 } else { 255 };
    if fr.bitreservoir > lim {
        fr.bitreservoir = lim;
    }

    /* Now back into less commented territory. It's code. It works. */

    if stereo == 1 {
        si.private_bits = getbits(fr, tab[2]);
    } else {
        si.private_bits = getbits(fr, tab[3]);
    }

    if fr.hdr.lsf == 0 {
        for ch in 0..stereo as usize {
            si.ch[ch][0].scfsi = -1;
            si.ch[ch][1].scfsi = getbits(fr, 4) as i32;
        }
    }

    for gr in 0..tab[0] as usize {
        for ch in 0..stereo as usize {
            let mut gr_info = si.ch[ch][gr];
            gr_info.part2_3_length = getbits(fr, 12);
            gr_info.big_values = getbits(fr, 9);
            if gr_info.big_values > 288 {
                // big_values too large!
                gr_info.big_values = 288;
            }
            let qss = getbits_fast(fr, 8);
            gr_info.pow2gain = (256 - qss as i32 + powdiff) as usize;
            if ms_stereo != 0 {
                gr_info.pow2gain += 2;
            }
            gr_info.scalefac_compress = getbits(fr, tab[4]);
            if gr_info.part2_3_length == 0 {
                gr_info.scalefac_compress = 0;
            }

            /* 22 bits for if/else block */
            if getbits(fr, 1) != 0 {
                /* window switch flag  */
                gr_info.block_type = getbits_fast(fr, 2);
                gr_info.mixed_block_flag = get1bit(fr);
                gr_info.table_select[0] = getbits_fast(fr, 5);
                gr_info.table_select[1] = getbits_fast(fr, 5);
                /*
                    table_select[2] not needed, because there is no region2,
                    but to satisfy some verification tools we set it either.
                */
                gr_info.table_select[2] = 0;
                for i in 0..3 {
                    let sbg = getbits_fast(fr, 3) << 3;
                    gr_info.full_gain[i] = gr_info.pow2gain + sbg as usize;
                }

                if gr_info.block_type == 0 {
                    // Blocktype == 0 and window-switching == 1 not allowed.
                    si.ch[ch][gr] = gr_info;
                    return 1;
                }

                /* region_count/start parameters are implicit in this case. */
                if (fr.hdr.lsf == 0 || (gr_info.block_type == 2)) && fr.hdr.mpeg25 == 0 {
                    gr_info.region1start = 36 >> 1;
                    gr_info.region2start = 576 >> 1;
                } else if fr.hdr.mpeg25 != 0 {
                    let r0c = if (gr_info.block_type == 2) && gr_info.mixed_block_flag == 0 {
                        5
                    } else {
                        7
                    };

                    /* r0c+1+r1c+1 == 22, always. */
                    let r1c = 20 - r0c;
                    gr_info.region1start = (BAND_INFO[sfreq].long_idx[r0c + 1] >> 1) as u32;
                    gr_info.region2start =
                        (BAND_INFO[sfreq].long_idx[r0c + 1 + r1c + 1] >> 1) as u32;
                } else {
                    gr_info.region1start = 54 >> 1;
                    gr_info.region2start = 576 >> 1;
                }
            } else {
                for i in 0..3 {
                    gr_info.table_select[i] = getbits_fast(fr, 5);
                }

                let r0c = getbits_fast(fr, 4) as usize; /* 0 .. 15 */
                let r1c = getbits_fast(fr, 3) as usize; /* 0 .. 7 */
                gr_info.region1start = (BAND_INFO[sfreq].long_idx[r0c + 1] >> 1) as u32;

                /* max(r0c+r1c+2) = 15+7+2 = 24 */
                if r0c + 1 + r1c + 1 > 22 {
                    gr_info.region2start = 576 >> 1;
                } else {
                    gr_info.region2start =
                        (BAND_INFO[sfreq].long_idx[r0c + 1 + r1c + 1] >> 1) as u32;
                }

                gr_info.block_type = 0;
                gr_info.mixed_block_flag = 0;
            }
            if fr.hdr.lsf == 0 {
                gr_info.preflag = get1bit(fr);
            }

            gr_info.scalefac_scale = get1bit(fr);
            gr_info.count1table_select = get1bit(fr);
            si.ch[ch][gr] = gr_info;
        }
    }
    0
}

/* read scalefactors */
fn iii_get_scale_factors_1(
    fr: &mut Mpg123Handle<'_>,
    scf_buf: &mut [i32; 39],
    gr_info: &GrInfo,
    _ch: i32,
    _gr: i32,
) -> i32 {
    const SLEN: [[u8; 16]; 2] = [
        [0, 0, 0, 0, 3, 1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4],
        [0, 1, 2, 3, 0, 1, 2, 3, 1, 2, 3, 1, 2, 3, 2, 3],
    ];
    let numbits;
    let num0 = SLEN[0][gr_info.scalefac_compress as usize] as i32;
    let num1 = SLEN[1][gr_info.scalefac_compress as usize] as i32;
    let mut scf = 0usize;

    if gr_info.block_type == 2 {
        let mut i = 18;
        numbits = (num0 + num1) * 18 /* num0 * (17+1?) + num1 * 18 */
            - (if gr_info.mixed_block_flag != 0 { num0 } else { 0 });
        if fr.bits_avail < numbits as i64 {
            return -1;
        }

        if gr_info.mixed_block_flag != 0 {
            for _i in 0..8 {
                scf_buf[scf] = getbits_fast(fr, num0) as i32;
                scf += 1;
            }

            i = 9;
        }

        for _i in 0..i {
            scf_buf[scf] = getbits_fast(fr, num0) as i32;
            scf += 1;
        }

        for _i in 0..18 {
            scf_buf[scf] = getbits_fast(fr, num1) as i32;
            scf += 1;
        }

        /* short[13][0..2] = 0 */
        scf_buf[scf] = 0;
        scf_buf[scf + 1] = 0;
        scf_buf[scf + 2] = 0;
    } else {
        let scfsi = gr_info.scfsi;

        if scfsi < 0 {
            /* scfsi < 0 => granule == 0 */
            numbits = (num0 + num1) * 10 + num0;
            if fr.bits_avail < numbits as i64 {
                return -1;
            }

            for _i in 0..11 {
                scf_buf[scf] = getbits_fast(fr, num0) as i32;
                scf += 1;
            }

            for _i in 0..10 {
                scf_buf[scf] = getbits_fast(fr, num1) as i32;
                scf += 1;
            }

            scf_buf[scf] = 0;
        } else {
            let nb = |flag: i32, num: i32, n: i32| if scfsi & flag == 0 { num * n } else { 0 };
            numbits = nb(0x8, num0, 6) + nb(0x4, num0, 5) + nb(0x2, num1, 5) + nb(0x1, num1, 5);
            if fr.bits_avail < numbits as i64 {
                return -1;
            }

            if scfsi & 0x8 == 0 {
                for _i in 0..6 {
                    scf_buf[scf] = getbits_fast(fr, num0) as i32;
                    scf += 1;
                }
            } else {
                scf += 6;
            }

            if scfsi & 0x4 == 0 {
                for _i in 0..5 {
                    scf_buf[scf] = getbits_fast(fr, num0) as i32;
                    scf += 1;
                }
            } else {
                scf += 5;
            }

            if scfsi & 0x2 == 0 {
                for _i in 0..5 {
                    scf_buf[scf] = getbits_fast(fr, num1) as i32;
                    scf += 1;
                }
            } else {
                scf += 5;
            }

            if scfsi & 0x1 == 0 {
                for _i in 0..5 {
                    scf_buf[scf] = getbits_fast(fr, num1) as i32;
                    scf += 1;
                }
            } else {
                scf += 5;
            }

            scf_buf[scf] = 0; /* no l[21] in original sources */
        }
    }

    numbits
}

fn iii_get_scale_factors_2(
    fr: &mut Mpg123Handle<'_>,
    scf_buf: &mut [i32; 39],
    gr_info: &mut GrInfo,
    i_stereo: i32,
) -> i32 {
    let mut numbits: i32 = 0;

    const STAB: [[[u8; 4]; 6]; 3] = [
        [
            [6, 5, 5, 5],
            [6, 5, 7, 3],
            [11, 10, 0, 0],
            [7, 7, 7, 0],
            [6, 6, 6, 3],
            [8, 8, 5, 0],
        ],
        [
            [9, 9, 9, 9],
            [9, 9, 12, 6],
            [18, 18, 0, 0],
            [12, 12, 12, 0],
            [12, 9, 9, 6],
            [15, 12, 9, 0],
        ],
        [
            [6, 9, 9, 9],
            [6, 9, 12, 6],
            [15, 18, 0, 0],
            [6, 15, 12, 0],
            [6, 12, 9, 6],
            [6, 18, 9, 0],
        ],
    ];

    let mut slen: u32 = if i_stereo != 0 {
        /* i_stereo AND second channel -> INT123_do_layer3() checks this */
        I_SLEN2[(gr_info.scalefac_compress >> 1) as usize] as u32
    } else {
        N_SLEN2[gr_info.scalefac_compress as usize] as u32
    };

    gr_info.preflag = (slen >> 15) & 0x1;

    let mut n = 0usize;
    if gr_info.block_type == 2 {
        n += 1;
        if gr_info.mixed_block_flag != 0 {
            n += 1;
        }
    }

    let pnt = &STAB[n][((slen >> 12) & 0x7) as usize];

    let mut slen2 = slen;
    for i in 0..4 {
        let num = (slen2 & 0x7) as i32;
        slen2 >>= 3;
        if num != 0 {
            numbits += pnt[i] as i32 * num;
        }
    }
    if numbits as i64 > gr_info.part2_3_length as i64 {
        return -1;
    }

    let mut scf = 0usize;
    for i in 0..4 {
        let num = (slen & 0x7) as i32;
        slen >>= 3;
        if num != 0 {
            for _j in 0..pnt[i] {
                scf_buf[scf] = getbits_fast(fr, num) as i32;
                scf += 1;
            }
        } else {
            for _j in 0..pnt[i] {
                scf_buf[scf] = 0;
                scf += 1;
            }
        }
    }

    n = (n << 1) + 1;
    for _i in 0..n {
        scf_buf[scf] = 0;
        scf += 1;
    }

    numbits
}

static PRETAB_CHOICE: [[u8; 22]; 2] = [
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 3, 2, 0,
    ],
];

/*
    Dequantize samples
    ...includes Huffman decoding
*/

/* 24 is enough because tab13 has max. a 19 bit huffvector */
/* The old code played games with shifting signed integers around in not quite */
/* legal ways. Also, it used long where just 32 bits are required. This could */
/* be good or bad on 64 bit architectures ... anyway, making clear that */
/* 32 bits suffice is a benefit. */
const BITSHIFT: i32 = 24;

/// `MSB_MASK`
#[inline]
fn msb_mask(mask: u32) -> bool {
    mask & (1u32 << 31) != 0
}

/// A `const short *` into one of the `map` tables (the `mapbuf` arrays,
/// which upstream's pointers may walk from one sampling rate's row into
/// the next).
struct MapPtr {
    tab: &'static [i16],
    pos: usize,
}

impl MapPtr {
    /// `*m++`
    #[inline]
    fn next(&mut self) -> i32 {
        let v = self.tab.get(self.pos).copied().unwrap_or(0) as i32;
        self.pos += 1;
        v
    }
}

/// `REFRESH_MASK`
macro_rules! refresh_mask {
    ($fr:expr, $mask:ident, $num:ident, $part2remain:ident) => {
        while $num < BITSHIFT {
            $mask |= (getbyte($fr) as u32) << (BITSHIFT - $num);
            $num += 8;
            $part2remain -= 8;
        }
    };
}

fn iii_dequantize_sample(
    fr: &mut Mpg123Handle<'_>,
    xr: &mut [[f32; SSLIMIT]; SBLIMIT],
    scf_buf: &[i32; 39],
    gr_info: &mut GrInfo,
    sfreq: usize,
    part2bits: i32,
) -> i32 {
    let shift = 1 + gr_info.scalefac_scale;
    // Pointer cast to make pedantic compilers happy.
    let xr = xr.as_flattened_mut();
    let mut xrpnt: usize = 0;
    // Some compiler freaks out over &xr[SBLIMIT][0], which is the same.
    let xrpntlimit = SBLIMIT * SSLIMIT;
    let mut l = [0i32; 3];
    let mut l3;
    let mut part2remain = (gr_info.part2_3_length as i32).wrapping_sub(part2bits);
    let mut scf = 0usize;

    // Writes to xr jump around wildly with corrupted files. It is not guaranteed
    // that all values are correctly written to. Zero all values beforehand
    // to have a clean slate, instead of trying to do that afterwards.
    // All branches that zero a value can also be dropped, then.
    // After some micro optimization here and there, the resulting code seems
    // to be up to 3 % faster than before with AVX decoder, but this depends a
    // lot on the target CPU, could give a small hit on some.
    // This loop here also seems to be a bit faster than calling memset(0).
    xr.fill(0.0);

    /// `CHECK_XRPNT_USE`
    macro_rules! check_xrpnt_use {
        () => {
            if xrpnt >= xrpntlimit {
                // attempted xrpnt overflow (%p !< %p)
                return 1;
            }
        };
    }

    let gp = |fr: &Mpg123Handle<'_>, i: usize| fr.gainpow2(i);

    /* Assumption: If there is some part2_3_length at all, there should be
    enough of it to work with properly. In case of zero length we silently
    zero things. */
    if gr_info.part2_3_length > 0 {
        /* mhipp tree has this split up a bit... */
        let mut num = getbitoffset(fr);
        /* We must split this, because for num==0 the shift is undefined if you do it in one step. */
        let mut mask: u32 = getbits(fr, num) << BITSHIFT;
        mask <<= 8 - num;
        part2remain -= num;

        /* Bitindex is zero now, we are allowed to use getbyte(). */

        {
            let bv = gr_info.big_values as i32;
            let region1 = gr_info.region1start as i32;
            let region2 = gr_info.region2start as i32;
            l3 = ((576 >> 1) - bv) >> 1;

            /* we may lose the 'odd' bit here !! check this later again */
            if bv <= region1 {
                l[0] = bv;
                l[1] = 0;
                l[2] = 0;
            } else {
                l[0] = region1;
                if bv <= region2 {
                    l[1] = bv - l[0];
                    l[2] = 0;
                } else {
                    l[1] = region2 - l[0];
                    l[2] = bv - region2;
                }
            }
        }

        // About 2/3 of checks when checking on use.
        //#define CHECK_XRPNT_ON_SET 1

        if gr_info.block_type == 2 {
            /* decoding with short or mixed mode BandIndex table */
            let mut max = [0i32; 4];
            let mut step = 0usize;
            let mut lwin = 3usize;
            let mut cb = 0i32;
            let mut v: f32 = 0.0;
            let mut mc: i32;
            let mut m;
            let me;

            if gr_info.mixed_block_flag != 0 {
                max[3] = -1;
                max[0] = 2;
                max[1] = 2;
                max[2] = 2;
                m = MapPtr {
                    tab: MAPBUF0.as_flattened(),
                    pos: sfreq * 152,
                };
                me = sfreq * 152 + MAPEND_LEN[sfreq][0];
            } else {
                max[0] = -1;
                max[1] = -1;
                max[2] = -1;
                max[3] = -1;
                /* max[3] not really needed in this case */
                m = MapPtr {
                    tab: MAPBUF1.as_flattened(),
                    pos: sfreq * 156,
                };
                me = sfreq * 156 + MAPEND_LEN[sfreq][1];
            }

            mc = 0;
            for i in 0..2 {
                let mut lp = l[i];
                let h = &HT[gr_info.table_select[i] as usize];
                while lp != 0 {
                    let mut x: i32;
                    let mut y: i32;
                    if mc == 0 {
                        mc = m.next();
                        xrpnt = m.next() as usize;
                        lwin = m.next() as usize;
                        cb = m.next();
                        if lwin == 3 {
                            v = gp(
                                fr,
                                gr_info.pow2gain + ((scf_buf[scf] as u32) << shift) as usize,
                            );
                            scf += 1;
                            step = 1;
                        } else {
                            v = gp(
                                fr,
                                gr_info.full_gain[lwin] + ((scf_buf[scf] as u32) << shift) as usize,
                            );
                            scf += 1;
                            step = 3;
                        }
                    }
                    {
                        let mut val = 0usize;
                        refresh_mask!(fr, mask, num, part2remain);
                        loop {
                            y = h.table[val + (mask >> (BITSHIFT + 4)) as usize] as i32;
                            if y >= 0 {
                                break;
                            }
                            val = (val as isize - y as isize) as usize;
                            num -= 4;
                            mask <<= 4;
                        }
                        num -= y >> 8;
                        mask <<= y >> 8;
                        x = (y >> 4) & 0xf;
                        y &= 0xf;
                    }
                    if x == 15 && h.linbits != 0 {
                        max[lwin] = cb;
                        refresh_mask!(fr, mask, num, part2remain);
                        x += (mask >> (BITSHIFT as u32 + 8 - h.linbits)) as i32;
                        num -= h.linbits as i32 + 1;
                        mask <<= h.linbits;
                        check_xrpnt_use!();
                        if msb_mask(mask) {
                            xr[xrpnt] = -ISPOW[x as usize] * v;
                        } else {
                            xr[xrpnt] = ISPOW[x as usize] * v;
                        }

                        mask <<= 1;
                    } else if x != 0 {
                        max[lwin] = cb;
                        check_xrpnt_use!();
                        if msb_mask(mask) {
                            xr[xrpnt] = -ISPOW[x as usize] * v;
                        } else {
                            xr[xrpnt] = ISPOW[x as usize] * v;
                        }

                        num -= 1;
                        mask <<= 1;
                    }

                    xrpnt = xrpnt.wrapping_add(step);
                    if y == 15 && h.linbits != 0 {
                        max[lwin] = cb;
                        refresh_mask!(fr, mask, num, part2remain);
                        y += (mask >> (BITSHIFT as u32 + 8 - h.linbits)) as i32;
                        num -= h.linbits as i32 + 1;
                        mask <<= h.linbits;
                        check_xrpnt_use!();
                        if msb_mask(mask) {
                            xr[xrpnt] = -ISPOW[y as usize] * v;
                        } else {
                            xr[xrpnt] = ISPOW[y as usize] * v;
                        }

                        mask <<= 1;
                    } else if y != 0 {
                        max[lwin] = cb;
                        check_xrpnt_use!();
                        if msb_mask(mask) {
                            xr[xrpnt] = -ISPOW[y as usize] * v;
                        } else {
                            xr[xrpnt] = ISPOW[y as usize] * v;
                        }

                        num -= 1;
                        mask <<= 1;
                    }

                    xrpnt = xrpnt.wrapping_add(step);
                    lp -= 1;
                    mc -= 1;
                }
            }

            while l3 != 0 && (part2remain + num > 0) {
                let h = &HTC[gr_info.count1table_select as usize];
                let mut val = 0usize;
                let mut a: i16;

                refresh_mask!(fr, mask, num, part2remain);
                loop {
                    a = h.table[val];
                    val += 1;
                    if a >= 0 {
                        break;
                    }
                    if msb_mask(mask) {
                        val = (val as isize - a as isize) as usize;
                    }

                    num -= 1;
                    mask <<= 1;
                }
                if part2remain + num <= 0 {
                    num -= part2remain + num;
                    break;
                }

                for i in 0..4 {
                    if i & 1 == 0 {
                        if mc == 0 {
                            mc = m.next();
                            xrpnt = m.next() as usize;
                            lwin = m.next() as usize;
                            cb = m.next();
                            if lwin == 3 {
                                v = gp(
                                    fr,
                                    gr_info.pow2gain + ((scf_buf[scf] as u32) << shift) as usize,
                                );
                                scf += 1;
                                step = 1;
                            } else {
                                v = gp(
                                    fr,
                                    gr_info.full_gain[lwin]
                                        + ((scf_buf[scf] as u32) << shift) as usize,
                                );
                                scf += 1;
                                step = 3;
                            }
                        }
                        mc -= 1;
                    }
                    if (a & (0x8 >> i)) != 0 {
                        max[lwin] = cb;
                        if part2remain + num <= 0 {
                            break;
                        }

                        check_xrpnt_use!();
                        if msb_mask(mask) {
                            xr[xrpnt] = -v;
                        } else {
                            xr[xrpnt] = v;
                        }

                        num -= 1;
                        mask <<= 1;
                    }

                    xrpnt = xrpnt.wrapping_add(step);
                }
                l3 -= 1;
            }

            if lwin < 3 {
                /* short band? */
                loop {
                    if m.pos >= me {
                        if mc > 0 {
                            // No actual writes here because xr has been zeroed before.
                            // short band -> step=3, two steps per mc round
                            xrpnt = xrpnt.wrapping_add((6 * mc) as usize);
                            mc = 0;
                        }
                        break;
                    }

                    mc = m.next();
                    xrpnt = m.next() as usize;
                    if m.next() == 0 {
                        break; /* optimized: zero fields zero-initialized */
                    }

                    m.next(); /* cb */
                }
            }
            let _ = (mc, xrpnt);

            gr_info.maxband[0] = max[0] + 1;
            gr_info.maxband[1] = max[1] + 1;
            gr_info.maxband[2] = max[2] + 1;
            gr_info.maxbandl = max[3] + 1;

            {
                let mut rmax = if max[0] > max[1] { max[0] } else { max[1] };
                rmax = (if rmax > max[2] { rmax } else { max[2] }) + 1;
                gr_info.maxb = if rmax != 0 {
                    fr.short_limit[sfreq][rmax as usize]
                } else {
                    fr.long_limit[sfreq][(max[3] + 1) as usize]
                } as u32;
            }
        } else {
            /* decoding with 'long' BandIndex table (block_type != 2) */
            let pretab = &PRETAB_CHOICE[gr_info.preflag as usize];
            let mut pt = 0usize;
            let mut max = -1i32;
            let mut cb = 0i32;
            let mut m = MapPtr {
                tab: MAPBUF2.as_flattened(),
                pos: sfreq * 44,
            };
            let mut v: f32 = 0.0;
            let mut mc: i32 = 0;

            /* long hash table values */
            for i in 0..3 {
                let mut lp = l[i];
                let h = &HT[gr_info.table_select[i] as usize];

                while lp != 0 {
                    let mut x: i32;
                    let mut y: i32;
                    if mc == 0 {
                        mc = m.next();
                        cb = m.next();
                        {
                            let p = pretab.get(pt).copied().unwrap_or(0) as i32;
                            pt += 1;
                            v = gp(
                                fr,
                                gr_info.pow2gain + (((scf_buf[scf] + p) as u32) << shift) as usize,
                            );
                            scf += 1;
                        }
                    }
                    {
                        let mut val = 0usize;
                        refresh_mask!(fr, mask, num, part2remain);
                        loop {
                            y = h.table[val + (mask >> (BITSHIFT + 4)) as usize] as i32;
                            if y >= 0 {
                                break;
                            }
                            val = (val as isize - y as isize) as usize;
                            num -= 4;
                            mask <<= 4;
                        }
                        num -= y >> 8;
                        mask <<= y >> 8;
                        x = (y >> 4) & 0xf;
                        y &= 0xf;
                    }

                    if x == 15 && h.linbits != 0 {
                        max = cb;
                        refresh_mask!(fr, mask, num, part2remain);
                        x += (mask >> (BITSHIFT as u32 + 8 - h.linbits)) as i32;
                        num -= h.linbits as i32 + 1;
                        mask <<= h.linbits;
                        check_xrpnt_use!();
                        if msb_mask(mask) {
                            xr[xrpnt] = -ISPOW[x as usize] * v;
                        } else {
                            xr[xrpnt] = ISPOW[x as usize] * v;
                        }
                        xrpnt += 1;

                        mask <<= 1;
                    } else if x != 0 {
                        max = cb;
                        check_xrpnt_use!();
                        if msb_mask(mask) {
                            xr[xrpnt] = -ISPOW[x as usize] * v;
                        } else {
                            xr[xrpnt] = ISPOW[x as usize] * v;
                        }
                        xrpnt += 1;
                        num -= 1;

                        mask <<= 1;
                    } else {
                        xrpnt += 1;
                    }

                    if y == 15 && h.linbits != 0 {
                        max = cb;
                        refresh_mask!(fr, mask, num, part2remain);
                        y += (mask >> (BITSHIFT as u32 + 8 - h.linbits)) as i32;
                        num -= h.linbits as i32 + 1;
                        mask <<= h.linbits;
                        check_xrpnt_use!();
                        if msb_mask(mask) {
                            xr[xrpnt] = -ISPOW[y as usize] * v;
                        } else {
                            xr[xrpnt] = ISPOW[y as usize] * v;
                        }
                        xrpnt += 1;

                        mask <<= 1;
                    } else if y != 0 {
                        max = cb;
                        check_xrpnt_use!();
                        if msb_mask(mask) {
                            xr[xrpnt] = -ISPOW[y as usize] * v;
                        } else {
                            xr[xrpnt] = ISPOW[y as usize] * v;
                        }
                        xrpnt += 1;

                        num -= 1;
                        mask <<= 1;
                    } else {
                        xrpnt += 1;
                    }
                    lp -= 1;
                    mc -= 1;
                }
            }

            /* short (count1table) values */
            while l3 != 0 && (part2remain + num > 0) {
                let h = &HTC[gr_info.count1table_select as usize];
                let mut val = 0usize;
                let mut a: i16;

                refresh_mask!(fr, mask, num, part2remain);
                loop {
                    a = h.table[val];
                    val += 1;
                    if a >= 0 {
                        break;
                    }
                    if msb_mask(mask) {
                        val = (val as isize - a as isize) as usize;
                    }

                    num -= 1;
                    mask <<= 1;
                }
                if part2remain + num <= 0 {
                    num -= part2remain + num;
                    break;
                }

                for i in 0..4 {
                    if i & 1 == 0 {
                        if mc == 0 {
                            mc = m.next();
                            cb = m.next();
                            {
                                let p = pretab.get(pt).copied().unwrap_or(0) as i32;
                                pt += 1;
                                v = gp(
                                    fr,
                                    gr_info.pow2gain
                                        + (((scf_buf[scf] + p) as u32) << shift) as usize,
                                );
                                scf += 1;
                            }
                        }
                        mc -= 1;
                    }
                    if (a & (0x8 >> i)) != 0 {
                        max = cb;
                        if part2remain + num <= 0 {
                            break;
                        }

                        check_xrpnt_use!();
                        if msb_mask(mask) {
                            xr[xrpnt] = -v;
                        } else {
                            xr[xrpnt] = v;
                        }
                        xrpnt += 1;

                        num -= 1;
                        mask <<= 1;
                    } else {
                        xrpnt += 1;
                    }
                }
                l3 -= 1;
            }

            gr_info.maxbandl = max + 1;
            gr_info.maxb = fr.long_limit[sfreq][gr_info.maxbandl as usize] as u32;
        }

        part2remain += num;
        backbits(fr, num);
    } else {
        part2remain = 0;
        /* Not entirely sure what good values are, must be > 0. */
        gr_info.maxband[0] = 1;
        gr_info.maxband[1] = 1;
        gr_info.maxband[2] = 1;
        gr_info.maxbandl = 1; /* sfb=maxband[lwin]*3 + lwin - mixed_block_flag must be >= 0 */
        gr_info.maxb = 1;
    }

    // Relying on initial zeroing if xr has not been written to fully.

    while part2remain > 16 {
        skipbits(fr, 16); /* Dismiss stuffing Bits */
        part2remain -= 16;
    }
    if part2remain > 0 {
        skipbits(fr, part2remain);
    } else if part2remain < 0 {
        // Can't rewind stream by %d bits!
        return 1; /* -> error */
    }
    0
}

/* calculate real channel values for Joint-I-Stereo-mode */
fn iii_i_stereo(
    xr_buf: &mut [[[f32; SSLIMIT]; SBLIMIT]; 2],
    scalefac: &[i32; 39],
    gr_info: &GrInfo,
    sfreq: usize,
    ms_stereo: i32,
    lsf: i32,
) {
    let (xr0, xr1) = xr_buf.split_at_mut(1);
    let xr0 = xr0[0].as_flattened_mut();
    let xr1 = xr1[0].as_flattened_mut();
    let bi = &BAND_INFO[sfreq];

    /* TODO: optimize as static */
    let tabs: [[[&[f32]; 2]; 2]; 3] = [
        [[&TAN1_1, &TAN2_1], [&TAN1_2, &TAN2_2]],
        [[&POW1_1[0], &POW2_1[0]], [&POW1_2[0], &POW2_2[0]]],
        [[&POW1_1[1], &POW2_1[1]], [&POW1_2[1], &POW2_2[1]]],
    ];

    let tab = (lsf + (gr_info.scalefac_compress as i32 & lsf)) as usize;
    let tab1 = tabs[tab][ms_stereo as usize][0];
    let tab2 = tabs[tab][ms_stereo as usize][1];

    if gr_info.block_type == 2 {
        let mut do_l = 0;
        if gr_info.mixed_block_flag != 0 {
            do_l = 1;
        }

        for lwin in 0..3 {
            /* process each window */
            /* get first band with zero values */
            let mut sfb = gr_info.maxband[lwin]; /* sfb is minimal 3 for mixed mode */
            if sfb > 3 {
                do_l = 0;
            }

            while sfb < 12 {
                let is_p =
                    scalefac[(sfb * 3 + lwin as i32 - gr_info.mixed_block_flag as i32) as usize]; /* scale: 0-15 */
                if is_p != 7 {
                    let mut sb = bi.short_diff[sfb as usize] as i32;
                    let mut idx = bi.short_idx[sfb as usize] as usize + lwin;
                    let t1 = tab1[is_p as usize];
                    let t2 = tab2[is_p as usize];
                    while sb > 0 {
                        let v = xr0[idx];
                        xr0[idx] = v * t1;
                        xr1[idx] = v * t2;
                        sb -= 1;
                        idx += 3;
                    }
                }
                sfb += 1;
            }

            /* in the original: copy 10 to 11 , here: copy 11 to 12
            maybe still wrong??? (copy 12 to 13?) */
            let is_p = scalefac[(11 * 3 + lwin as i32 - gr_info.mixed_block_flag as i32) as usize]; /* scale: 0-15 */
            let mut sb = bi.short_diff[12] as i32;
            let mut idx = bi.short_idx[12] as usize + lwin;
            if is_p != 7 {
                let t1 = tab1[is_p as usize];
                let t2 = tab2[is_p as usize];
                while sb > 0 {
                    let v = xr0[idx];
                    xr0[idx] = v * t1;
                    xr1[idx] = v * t2;
                    sb -= 1;
                    idx += 3;
                }
            }
        } /* end for(lwin; .. ; . ) */

        /* also check l-part, if ALL bands in the three windows are 'empty' and mode = mixed_mode */
        if do_l != 0 {
            let mut sfb = gr_info.maxbandl;
            if sfb > 21 {
                return; /* similarity fix related to CVE-2006-1655 */
            }

            let mut idx = bi.long_idx[sfb as usize] as usize;
            while sfb < 8 {
                let mut sb = bi.long_diff[sfb as usize] as i32;
                let is_p = scalefac[sfb as usize]; /* scale: 0-15 */
                if is_p != 7 {
                    let t1 = tab1[is_p as usize];
                    let t2 = tab2[is_p as usize];
                    while sb > 0 {
                        let v = xr0[idx];
                        xr0[idx] = v * t1;
                        xr1[idx] = v * t2;
                        sb -= 1;
                        idx += 1;
                    }
                } else {
                    idx += sb as usize;
                }
                sfb += 1;
            }
        }
    } else {
        /* ((gr_info->block_type != 2)) */
        let mut sfb = gr_info.maxbandl;
        if sfb > 21 {
            return; /* tightened fix for CVE-2006-1655 */
        }

        let mut idx = bi.long_idx[sfb as usize] as usize;
        while sfb < 21 {
            let mut sb = bi.long_diff[sfb as usize] as i32;
            let is_p = scalefac[sfb as usize]; /* scale: 0-15 */
            if is_p != 7 {
                let t1 = tab1[is_p as usize];
                let t2 = tab2[is_p as usize];
                while sb > 0 {
                    let v = xr0[idx];
                    xr0[idx] = v * t1;
                    xr1[idx] = v * t2;
                    sb -= 1;
                    idx += 1;
                }
            } else {
                idx += sb as usize;
            }
            sfb += 1;
        }

        let is_p = scalefac[20];
        if is_p != 7 {
            /* copy l-band 20 to l-band 21 */
            let t1 = tab1[is_p as usize];
            let t2 = tab2[is_p as usize];

            let mut sb = bi.long_diff[21] as i32;
            while sb > 0 {
                let v = xr0[idx];
                xr0[idx] = v * t1;
                xr1[idx] = v * t2;
                sb -= 1;
                idx += 1;
            }
        }
    }
}

fn iii_antialias(xr: &mut [[f32; SSLIMIT]; SBLIMIT], gr_info: &GrInfo) {
    let sblim;

    if gr_info.block_type == 2 {
        if gr_info.mixed_block_flag == 0 {
            return;
        }

        sblim = 1;
    } else {
        sblim = gr_info.maxb.wrapping_sub(1);
    }

    /* 31 alias-reduction operations between each pair of sub-bands */
    /* with 8 butterflies between each pair                         */

    {
        let xr = xr.as_flattened_mut();
        let mut xr1 = SSLIMIT; /* xr[1] */

        for _sb in 0..sblim {
            let mut cs = 0usize;
            let mut ca = 0usize;
            let mut xr2 = xr1;

            for _ss in 0..8 {
                /* upper and lower butterfly inputs */
                xr2 -= 1;
                let bu = xr[xr2];
                let bd = xr[xr1];
                xr[xr2] = bu * AA_CS[cs] - bd * AA_CA[ca];
                xr[xr1] = bd * AA_CS[cs] + bu * AA_CA[ca];
                xr1 += 1;
                cs += 1;
                ca += 1;
            }
            xr1 += 10;
        }
    }
}

/*
    This is an optimized DCT from Jeff Tsay's maplay 1.2+ package.
    Saved one multiplication by doing the 'twiddle factor' stuff
    together with the window mul. (MH)

    This uses Byeong Gi Lee's Fast Cosine Transform algorithm, but the
    9 point IDCT needs to be reduced further. Unfortunately, I don't
    know how to do that, because 9 is not an even number. - Jeff.

    Original Message:

    9 Point Inverse Discrete Cosine Transform

    This piece of code is Copyright 1997 Mikko Tommila and is freely usable
    by anybody. The algorithm itself is of course in the public domain.

    Again derived heuristically from the 9-point WFTA.

    The algorithm is optimized (?) for speed, not for small rounding errors or
    good readability.

    36 additions, 11 multiplications

    Again this is very likely sub-optimal.

    The code is optimized to use a minimum number of temporary variables,
    so it should compile quite well even on 8-register Intel x86 processors.
    This makes the code quite obfuscated and very difficult to understand.

    References:
    [1] S. Winograd: "On Computing the Discrete Fourier Transform",
        Mathematics of Computation, Volume 32, Number 141, January 1978,
        Pages 175-199
*/
/// Translation of `INT123_dct36()`: `tsbuf` is indexed at strides of
/// `SBLIMIT`.
fn dct36(
    inbuf: &mut [f32; SSLIMIT],
    o1: &[f32],
    o2: &mut [f32],
    wintab: &[f32; 36],
    tsbuf: &mut [f32],
) {
    let mut tmp = [0.0f32; 18];

    {
        let inp = inbuf;

        inp[17] += inp[16];
        inp[16] += inp[15];
        inp[15] += inp[14];
        inp[14] += inp[13];
        inp[13] += inp[12];
        inp[12] += inp[11];
        inp[11] += inp[10];
        inp[10] += inp[9];
        inp[9] += inp[8];
        inp[8] += inp[7];
        inp[7] += inp[6];
        inp[6] += inp[5];
        inp[5] += inp[4];
        inp[4] += inp[3];
        inp[3] += inp[2];
        inp[2] += inp[1];
        inp[1] += inp[0];

        inp[17] += inp[15];
        inp[15] += inp[13];
        inp[13] += inp[11];
        inp[11] += inp[9];
        inp[9] += inp[7];
        inp[7] += inp[5];
        inp[5] += inp[3];
        inp[3] += inp[1];

        {
            let mut t3;
            {
                let t0 = COS6_2 * (inp[8] + inp[16] - inp[4]);
                let t1 = COS6_2 * inp[12];

                t3 = inp[0];
                let t2 = t3 - t1 - t1;
                tmp[1] = t2 - t0;
                tmp[7] = tmp[1];
                tmp[4] = t2 + t0 + t0;
                t3 += t1;

                let t2 = COS6_1 * (inp[10] + inp[14] - inp[2]);
                tmp[1] -= t2;
                tmp[7] += t2;
            }
            {
                let t0 = COS9[0] * (inp[4] + inp[8]);
                let t1 = COS9[1] * (inp[8] - inp[16]);
                let t2 = COS9[2] * (inp[4] + inp[16]);

                tmp[2] = t3 - t0 - t2;
                tmp[6] = tmp[2];
                tmp[0] = t3 + t0 + t1;
                tmp[8] = tmp[0];
                tmp[3] = t3 - t1 + t2;
                tmp[5] = tmp[3];
            }
        }
        {
            let mut t1 = COS18[0] * (inp[2] + inp[10]);
            let mut t2 = COS18[1] * (inp[10] - inp[14]);
            let mut t3 = COS6_1 * inp[6];

            {
                let t0 = t1 + t2 + t3;
                tmp[0] += t0;
                tmp[8] -= t0;
            }

            t2 -= t3;
            t1 -= t3;

            t3 = COS18[2] * (inp[2] + inp[14]);

            t1 += t3;
            tmp[3] += t1;
            tmp[5] -= t1;

            t2 -= t3;
            tmp[2] += t2;
            tmp[6] -= t2;
        }

        {
            let t1 = COS6_2 * inp[13];
            let mut t2 = COS6_2 * (inp[9] + inp[17] - inp[5]);

            let mut t3 = inp[1] + t1;
            let mut t4 = inp[1] - t1 - t1;
            let t5 = t4 - t2;

            let mut t0 = COS9[0] * (inp[5] + inp[9]);
            let mut t1 = COS9[1] * (inp[9] - inp[17]);

            tmp[13] = (t4 + t2 + t2) * INT123_TFCOS36[17 - 13];
            t2 = COS9[2] * (inp[5] + inp[17]);

            let t6 = t3 - t0 - t2;
            t0 += t3 + t1;
            t3 += t2 - t1;

            t2 = COS18[0] * (inp[3] + inp[11]);
            t4 = COS18[1] * (inp[11] - inp[15]);
            let t7 = COS6_1 * inp[7];

            t1 = t2 + t4 + t7;
            tmp[17] = (t0 + t1) * INT123_TFCOS36[17 - 17];
            tmp[9] = (t0 - t1) * INT123_TFCOS36[17 - 9];
            t1 = COS18[2] * (inp[3] + inp[15]);
            t2 += t1 - t7;

            tmp[14] = (t3 + t2) * INT123_TFCOS36[17 - 14];
            t0 = COS6_1 * (inp[11] + inp[15] - inp[3]);
            tmp[12] = (t3 - t2) * INT123_TFCOS36[17 - 12];

            t4 -= t1 + t7;

            tmp[16] = (t5 - t0) * INT123_TFCOS36[17 - 16];
            tmp[10] = (t5 + t0) * INT123_TFCOS36[17 - 10];
            tmp[15] = (t6 + t4) * INT123_TFCOS36[17 - 15];
            tmp[11] = (t6 - t4) * INT123_TFCOS36[17 - 11];
        }

        let out2 = o2;
        let w = wintab;
        let out1 = o1;
        let ts = tsbuf;
        macro_rules! dct36_macro {
            ($v:expr) => {{
                let v: usize = $v;
                let mut tmpval;
                tmpval = tmp[v] + tmp[17 - v];
                out2[9 + v] = tmpval * w[27 + v];
                out2[8 - v] = tmpval * w[26 - v];
                tmpval = tmp[v] - tmp[17 - v];
                ts[SBLIMIT * (8 - v)] = out1[8 - v] + tmpval * w[8 - v];
                ts[SBLIMIT * (9 + v)] = out1[9 + v] + tmpval * w[9 + v];
            }};
        }

        dct36_macro!(0);
        dct36_macro!(1);
        dct36_macro!(2);
        dct36_macro!(3);
        dct36_macro!(4);
        dct36_macro!(5);
        dct36_macro!(6);
        dct36_macro!(7);
        dct36_macro!(8);
    }
}

/* new DCT12 */
fn dct12(inp: &[f32], rawout1: &[f32], rawout2: &mut [f32], wi: &[f32; 36], ts: &mut [f32]) {
    let mut base = 0usize;
    let (mut in0, mut in1, mut in2, mut in3, mut in4, mut in5);

    macro_rules! dct12_part1 {
        () => {
            in5 = inp[base + 5 * 3];
            in4 = inp[base + 4 * 3];
            in5 += in4;
            in3 = inp[base + 3 * 3];
            in4 += in3;
            in2 = inp[base + 2 * 3];
            in3 += in2;
            in1 = inp[base + 3];
            in2 += in1;
            in0 = inp[base];
            in1 += in0;

            in5 += in3;
            in3 += in1;

            in2 *= COS6_1;
            in3 *= COS6_1;
        };
    }

    macro_rules! dct12_part2 {
        () => {
            in0 += in4 * COS6_2;

            in4 = in0 + in2;
            in0 -= in2;

            in1 += in5 * COS6_2;

            in5 = (in1 + in3) * TFCOS12[0];
            in1 = (in1 - in3) * TFCOS12[2];

            in3 = in4 + in5;
            in4 -= in5;

            in2 = in0 + in1;
            in0 -= in1;
        };
    }

    {
        let out1 = rawout1;
        ts[0] = out1[0];
        ts[SBLIMIT] = out1[1];
        ts[SBLIMIT * 2] = out1[2];
        ts[SBLIMIT * 3] = out1[3];
        ts[SBLIMIT * 4] = out1[4];
        ts[SBLIMIT * 5] = out1[5];

        dct12_part1!();

        {
            let tmp0;
            let mut tmp1 = in0 - in4;
            {
                let tmp2 = (in1 - in5) * TFCOS12[1];
                tmp0 = tmp1 + tmp2;
                tmp1 -= tmp2;
            }
            ts[(17 - 1) * SBLIMIT] = out1[17 - 1] + tmp0 * wi[11 - 1];
            ts[(12 + 1) * SBLIMIT] = out1[12 + 1] + tmp0 * wi[6 + 1];
            ts[(6 + 1) * SBLIMIT] = out1[6 + 1] + tmp1 * wi[1];
            ts[(11 - 1) * SBLIMIT] = out1[11 - 1] + tmp1 * wi[5 - 1];
        }

        dct12_part2!();

        ts[17 * SBLIMIT] = out1[17] + in2 * wi[11];
        ts[12 * SBLIMIT] = out1[12] + in2 * wi[6];
        ts[(12 + 2) * SBLIMIT] = out1[12 + 2] + in3 * wi[6 + 2];
        ts[(17 - 2) * SBLIMIT] = out1[17 - 2] + in3 * wi[11 - 2];

        ts[6 * SBLIMIT] = out1[6] + in0 * wi[0];
        ts[11 * SBLIMIT] = out1[11] + in0 * wi[5];
        ts[(6 + 2) * SBLIMIT] = out1[6 + 2] + in4 * wi[2];
        ts[(11 - 2) * SBLIMIT] = out1[11 - 2] + in4 * wi[5 - 2];
    }

    base += 1;

    {
        let out2 = &mut *rawout2;

        dct12_part1!();

        {
            let tmp0;
            let mut tmp1 = in0 - in4;
            {
                let tmp2 = (in1 - in5) * TFCOS12[1];
                tmp0 = tmp1 + tmp2;
                tmp1 -= tmp2;
            }
            out2[5 - 1] = tmp0 * wi[11 - 1];
            out2[1] = tmp0 * wi[6 + 1];
            ts[(12 + 1) * SBLIMIT] += tmp1 * wi[1];
            ts[(17 - 1) * SBLIMIT] += tmp1 * wi[5 - 1];
        }

        dct12_part2!();

        out2[5] = in2 * wi[11];
        out2[0] = in2 * wi[6];
        out2[2] = in3 * wi[6 + 2];
        out2[5 - 2] = in3 * wi[11 - 2];

        ts[12 * SBLIMIT] += in0 * wi[0];
        ts[17 * SBLIMIT] += in0 * wi[5];
        ts[(12 + 2) * SBLIMIT] += in4 * wi[2];
        ts[(17 - 2) * SBLIMIT] += in4 * wi[5 - 2];
    }

    base += 1;

    {
        let out2 = &mut *rawout2;
        out2[12] = 0.0;
        out2[13] = 0.0;
        out2[14] = 0.0;
        out2[15] = 0.0;
        out2[16] = 0.0;
        out2[17] = 0.0;

        dct12_part1!();

        {
            let tmp0;
            let mut tmp1 = in0 - in4;
            {
                let tmp2 = (in1 - in5) * TFCOS12[1];
                tmp0 = tmp1 + tmp2;
                tmp1 -= tmp2;
            }
            out2[11 - 1] = tmp0 * wi[11 - 1];
            out2[6 + 1] = tmp0 * wi[6 + 1];
            out2[1] += tmp1 * wi[1];
            out2[5 - 1] += tmp1 * wi[5 - 1];
        }

        dct12_part2!();

        out2[11] = in2 * wi[11];
        out2[6] = in2 * wi[6];
        out2[6 + 2] = in3 * wi[6 + 2];
        out2[11 - 2] = in3 * wi[11 - 2];

        out2[0] += in0 * wi[0];
        out2[5] += in0 * wi[5];
        out2[2] += in4 * wi[2];
        out2[5 - 2] += in4 * wi[5 - 2];
    }
}

fn iii_hybrid(
    fs_in: &mut [[f32; SSLIMIT]; SBLIMIT],
    ts_out: &mut [[f32; SBLIMIT]; SSLIMIT],
    ch: usize,
    gr_info: &GrInfo,
    block: &mut [[[f32; SBLIMIT * SSLIMIT]; 2]; 2],
    blc: &mut [i32; 2],
) {
    let tspnt_buf = ts_out.as_flattened_mut();
    let mut tspnt = 0usize;
    let mut sb = 0usize;

    let b = blc[ch] as usize;
    let (b0, b1) = block.split_at_mut(1);
    let (rawout1_buf, rawout2_buf) = if b == 0 {
        (&mut b0[0][ch], &mut b1[0][ch])
    } else {
        (&mut b1[0][ch], &mut b0[0][ch])
    };
    blc[ch] = (1 - b) as i32;
    let mut rawout1 = 0usize;
    let mut rawout2 = 0usize;

    if gr_info.mixed_block_flag != 0 {
        sb = 2;
        dct36(
            &mut fs_in[0],
            &rawout1_buf[rawout1..],
            &mut rawout2_buf[rawout2..],
            &WIN[0],
            &mut tspnt_buf[tspnt..],
        );
        dct36(
            &mut fs_in[1],
            &rawout1_buf[rawout1 + 18..],
            &mut rawout2_buf[rawout2 + 18..],
            &WIN1[0],
            &mut tspnt_buf[tspnt + 1..],
        );
        rawout1 += 36;
        rawout2 += 36;
        tspnt += 2;
    }

    let bt = gr_info.block_type as usize;
    if bt == 2 {
        while (sb as u32) < gr_info.maxb {
            dct12(
                &fs_in[sb],
                &rawout1_buf[rawout1..],
                &mut rawout2_buf[rawout2..],
                &WIN[2],
                &mut tspnt_buf[tspnt..],
            );
            dct12(
                &fs_in[sb + 1],
                &rawout1_buf[rawout1 + 18..],
                &mut rawout2_buf[rawout2 + 18..],
                &WIN1[2],
                &mut tspnt_buf[tspnt + 1..],
            );
            sb += 2;
            tspnt += 2;
            rawout1 += 36;
            rawout2 += 36;
        }
    } else {
        while (sb as u32) < gr_info.maxb {
            dct36(
                &mut fs_in[sb],
                &rawout1_buf[rawout1..],
                &mut rawout2_buf[rawout2..],
                &WIN[bt],
                &mut tspnt_buf[tspnt..],
            );
            dct36(
                &mut fs_in[sb + 1],
                &rawout1_buf[rawout1 + 18..],
                &mut rawout2_buf[rawout2 + 18..],
                &WIN1[bt],
                &mut tspnt_buf[tspnt + 1..],
            );
            sb += 2;
            tspnt += 2;
            rawout1 += 36;
            rawout2 += 36;
        }
    }

    while sb < SBLIMIT {
        for i in 0..SSLIMIT {
            tspnt_buf[tspnt + i * SBLIMIT] = rawout1_buf[rawout1];
            rawout1 += 1;
            rawout2_buf[rawout2] = 0.0;
            rawout2 += 1;
        }
        sb += 1;
        tspnt += 1;
    }
}

/* And at the end... the main layer3 handler */
/// Translation of `INT123_do_layer3()`.
pub(crate) fn do_layer3(fr: &mut Mpg123Handle<'_>) -> i32 {
    let Some(mut scratch) = fr.layerscratch.take() else {
        return 0;
    };
    let clip = do_layer3_on(fr, &mut scratch);
    fr.layerscratch = Some(scratch);
    clip
}

fn do_layer3_on(fr: &mut Mpg123Handle<'_>, scratch: &mut LayerScratch) -> i32 {
    let mut clip = 0;
    let mut scalefacs = [[0i32; 39]; 2]; /* max 39 for short[13][3] mode, mixed: 38, long: 22 */
    let mut sideinfo = IIISideinfo::default();
    let stereo = fr.stereo;
    let mut single = fr.single;
    let ms_stereo;
    let i_stereo;
    let sfreq = fr.hdr.sampling_frequency as usize;
    let stereo1;

    if stereo == 1 {
        /* stream is mono */
        stereo1 = 1;
        single = SINGLE_LEFT;
    } else if single != SINGLE_STEREO {
        /* stream is stereo, but force to mono */
        stereo1 = 1;
    } else {
        stereo1 = 2;
    }

    if fr.hdr.mode == MPG_MD_JOINT_STEREO {
        ms_stereo = (fr.hdr.mode_ext & 0x2) >> 1;
        i_stereo = fr.hdr.mode_ext & 0x1;
    } else {
        ms_stereo = 0;
        i_stereo = 0;
    }

    let granules = if fr.hdr.lsf != 0 { 1 } else { 2 };

    /* quick hack to keep the music playing */
    /* after having seen this nasty test file... */
    if iii_get_side_info(fr, &mut sideinfo, stereo, ms_stereo, sfreq, single) != 0 {
        // bad frame - unable to get valid sideinfo
        return clip;
    }

    set_pointer(fr, 1, sideinfo.main_data_begin as i64);
    for gr in 0..granules {
        /*  hybridIn[2][SBLIMIT][SSLIMIT] */
        let hybrid_in = &mut scratch.hybrid_in;
        /*  hybridOut[2][SSLIMIT][SBLIMIT] */
        let hybrid_out = &mut scratch.hybrid_out;

        {
            let gr_info = &mut sideinfo.ch[0][gr];
            if gr_info.part2_3_length as i64 > fr.bits_avail {
                // part2_3_length (%u) too large for available bit count (%li)
                return clip;
            }
            let part2bits = if fr.hdr.lsf != 0 {
                iii_get_scale_factors_2(fr, &mut scalefacs[0], gr_info, 0)
            } else {
                iii_get_scale_factors_1(fr, &mut scalefacs[0], gr_info, 0, gr as i32)
            };

            if part2bits < 0 {
                // not enough bits for scale factors
                return clip;
            }

            if iii_dequantize_sample(
                fr,
                &mut hybrid_in[0],
                &scalefacs[0],
                gr_info,
                sfreq,
                part2bits,
            ) != 0
            {
                // dequantization failed!
                return clip;
            }
            if fr.bits_avail < 0 {
                // bit deficit after dequant
                return clip;
            }
        }

        if stereo == 2 {
            let mut gr_info = sideinfo.ch[1][gr];
            let part2bits = if fr.hdr.lsf != 0 {
                iii_get_scale_factors_2(fr, &mut scalefacs[1], &mut gr_info, i_stereo)
            } else {
                iii_get_scale_factors_1(fr, &mut scalefacs[1], &gr_info, 1, gr as i32)
            };

            if part2bits < 0 {
                // not enough bits for scale factors
                return clip;
            }

            if iii_dequantize_sample(
                fr,
                &mut hybrid_in[1],
                &scalefacs[1],
                &mut gr_info,
                sfreq,
                part2bits,
            ) != 0
            {
                // dequantization failed!
                return clip;
            }
            if fr.bits_avail < 0 {
                // bit deficit after dequant
                return clip;
            }

            if ms_stereo != 0 {
                let mut maxb = sideinfo.ch[0][gr].maxb;
                if gr_info.maxb > maxb {
                    maxb = gr_info.maxb;
                }

                let (h0, h1) = hybrid_in.split_at_mut(1);
                let h0 = h0[0].as_flattened_mut();
                let h1 = h1[0].as_flattened_mut();
                for i in 0..(SSLIMIT as i32).wrapping_mul(maxb as i32).max(0) as usize {
                    let tmp0 = h0[i];
                    let tmp1 = h1[i];
                    h0[i] = tmp0 + tmp1;
                    h1[i] = tmp0 - tmp1;
                }
            }

            if i_stereo != 0 {
                iii_i_stereo(
                    hybrid_in,
                    &scalefacs[1],
                    &gr_info,
                    sfreq,
                    ms_stereo,
                    fr.hdr.lsf,
                );
            }

            if ms_stereo != 0 || i_stereo != 0 || (single == SINGLE_MIX) {
                if gr_info.maxb > sideinfo.ch[0][gr].maxb {
                    sideinfo.ch[0][gr].maxb = gr_info.maxb;
                } else {
                    gr_info.maxb = sideinfo.ch[0][gr].maxb;
                }
            }

            match single {
                SINGLE_MIX => {
                    let (h0, h1) = hybrid_in.split_at_mut(1);
                    let h0 = h0[0].as_flattened_mut();
                    let h1 = h1[0].as_flattened_mut();
                    for i in 0..(SSLIMIT as i32).wrapping_mul(gr_info.maxb as i32).max(0) as usize {
                        h0[i] += h1[i]; /* *0.5 done by pow-scale */
                    }
                }
                SINGLE_RIGHT => {
                    let (h0, h1) = hybrid_in.split_at_mut(1);
                    let h0 = h0[0].as_flattened_mut();
                    let h1 = h1[0].as_flattened_mut();
                    for i in 0..(SSLIMIT as i32).wrapping_mul(gr_info.maxb as i32).max(0) as usize {
                        h0[i] = h1[i];
                    }
                }
                _ => {}
            }
            sideinfo.ch[1][gr] = gr_info;
        }

        for ch in 0..stereo1 {
            let gr_info = sideinfo.ch[ch][gr];
            iii_antialias(&mut hybrid_in[ch], &gr_info);
            iii_hybrid(
                &mut hybrid_in[ch],
                &mut hybrid_out[ch],
                ch,
                &gr_info,
                &mut fr.hybrid_block,
                &mut fr.hybrid_blc,
            );
        }

        for ss in 0..SSLIMIT {
            if single != SINGLE_STEREO {
                let f = fr.synth_mono.expect("synth set up");
                clip += f(&mut hybrid_out[0][ss], fr);
            } else {
                let f = fr.synth_stereo.expect("synth set up");
                let (o0, o1) = hybrid_out.split_at_mut(1);
                clip += f(&mut o0[0][ss], &mut o1[0][ss], fr);
            }
        }
    }

    clip
}
