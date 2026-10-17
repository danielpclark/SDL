// Rust translation of the decoding half of lib/floor1.c from libvorbis
// 1.3.7 (https://xiph.org/vorbis/, at the revision SDL_mixer's
// external/vorbis pins).
// THE OggVorbis SOURCE CODE IS (C) COPYRIGHT 1994-2015
// by the Xiph.Org Foundation https://xiph.org/
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! function: floor backend 1 implementation
//!
//! The encoder's half (`floor1_pack()`, `floor1_fit()` and its line
//! fitting, `floor1_interpolate_fit()`, `floor1_encode()`,
//! `render_line0()`, `vorbis_dBquant()`) isn't translated. The look keeps a
//! copy of its setup (C points at it in the `codec_setup_info`).

use super::codebook::vorbis_book_decode;
use super::sharedbook::ov_ilog;
use super::*;
use crate::ogg::bitwise::*;

pub(crate) const VIF_POSIT: usize = 63;
pub(crate) const VIF_CLASS: usize = 16;
pub(crate) const VIF_PARTS: usize = 31;

/// Translation of `vorbis_info_floor1` (backends.h): the encoder's
/// analysis parameters aren't kept.
#[derive(Clone, Copy, Debug)]
pub(crate) struct VorbisInfoFloor1 {
    pub partitions: i32,                      /* 0 to 31 */
    pub partitionclass: [i32; VIF_PARTS],     /* 0 to 15 */
    pub class_dim: [i32; VIF_CLASS],          /* 1 to 8 */
    pub class_subs: [i32; VIF_CLASS],         /* 0,1,2,3 (bits: 1<<n poss) */
    pub class_book: [i32; VIF_CLASS],         /* subs ^ dim entries */
    pub class_subbook: [[i32; 8]; VIF_CLASS], /* [VIF_CLASS][subs] */

    pub mult: i32,                      /* 1 2 3 or 4 */
    pub postlist: [i32; VIF_POSIT + 2], /* first two implicit */
}

impl Default for VorbisInfoFloor1 {
    fn default() -> Self {
        VorbisInfoFloor1 {
            partitions: 0,
            partitionclass: [0; VIF_PARTS],
            class_dim: [0; VIF_CLASS],
            class_subs: [0; VIF_CLASS],
            class_book: [0; VIF_CLASS],
            class_subbook: [[0; 8]; VIF_CLASS],
            mult: 0,
            postlist: [0; VIF_POSIT + 2],
        }
    }
}

/// Translation of `vorbis_look_floor1` (codec_internal.h).
#[derive(Clone, Copy, Debug)]
pub(crate) struct VorbisLookFloor1 {
    pub sorted_index: [i32; VIF_POSIT + 2],
    pub forward_index: [i32; VIF_POSIT + 2],
    pub reverse_index: [i32; VIF_POSIT + 2],

    pub hineighbor: [i32; VIF_POSIT],
    pub loneighbor: [i32; VIF_POSIT],
    pub posts: i32,

    pub n: i32,
    pub quant_q: i32,
    pub vi: VorbisInfoFloor1,
    /* (phrasebits, postbits and frames are never counted) */
}

/***********************************************/

/// Sorting the post list as C's `qsort()` with `icomp()` does (the
/// values are compared; the order of equal ones doesn't matter, as both
/// callers either reject them or never see them).
fn sort_posts(postlist: &[i32], n: usize) -> Vec<usize> {
    let mut sortpointer: Vec<usize> = (0..n).collect();
    sortpointer.sort_by(|&a, &b| postlist[a].cmp(&postlist[b]));
    sortpointer
}

/// Translation of `floor1_unpack()`.
pub(crate) fn floor1_unpack(
    ci: &CodecSetupInfo,
    opb: &mut OggpackBuffer<'_>,
) -> Option<Box<VorbisInfoFloor1>> {
    let mut count: i32 = 0;
    let mut maxclass: i32 = -1;
    let rangebits: i32;

    let mut info = Box::<VorbisInfoFloor1>::default();
    /* read partitions */
    info.partitions = oggpack_read(opb, 5) as i32; /* only 0 to 31 legal */
    for j in 0..info.partitions.max(0) as usize {
        info.partitionclass[j] = oggpack_read(opb, 4) as i32; /* only 0 to 15 legal */
        if info.partitionclass[j] < 0 {
            return None;
        }
        if maxclass < info.partitionclass[j] {
            maxclass = info.partitionclass[j];
        }
    }

    /* read partition classes */
    for j in 0..(maxclass + 1) as usize {
        info.class_dim[j] = oggpack_read(opb, 3) as i32 + 1; /* 1 to 8 */
        info.class_subs[j] = oggpack_read(opb, 2) as i32; /* 0,1,2,3 bits */
        if info.class_subs[j] < 0 {
            return None;
        }
        if info.class_subs[j] != 0 {
            info.class_book[j] = oggpack_read(opb, 8) as i32;
        }
        if info.class_book[j] < 0 || info.class_book[j] >= ci.books {
            return None;
        }
        for k in 0..(1usize << info.class_subs[j]) {
            info.class_subbook[j][k] = oggpack_read(opb, 8) as i32 - 1;
            if info.class_subbook[j][k] < -1 || info.class_subbook[j][k] >= ci.books {
                return None;
            }
        }
    }

    /* read the post list */
    info.mult = oggpack_read(opb, 2) as i32 + 1; /* only 1,2,3,4 legal now */
    rangebits = oggpack_read(opb, 4) as i32;
    if rangebits < 0 {
        return None;
    }

    let mut k = 0usize;
    for j in 0..info.partitions.max(0) as usize {
        count += info.class_dim[info.partitionclass[j] as usize];
        if count > VIF_POSIT as i32 {
            return None;
        }
        while k < count as usize {
            let t = oggpack_read(opb, rangebits) as i32;
            info.postlist[k + 2] = t;
            if t < 0 || t >= (1 << rangebits) {
                return None;
            }
            k += 1;
        }
    }
    info.postlist[0] = 0;
    info.postlist[1] = 1 << rangebits;

    /* don't allow repeated values in post list as they'd result in
    zero-length segments */
    {
        let sortpointer = sort_posts(&info.postlist, (count + 2) as usize);

        for j in 1..(count + 2) as usize {
            if info.postlist[sortpointer[j - 1]] == info.postlist[sortpointer[j]] {
                return None;
            }
        }
    }

    Some(info)
}

/// Translation of `floor1_look()`.
pub(crate) fn floor1_look(info: &VorbisInfoFloor1) -> Box<VorbisLookFloor1> {
    let mut look = Box::new(VorbisLookFloor1 {
        sorted_index: [0; VIF_POSIT + 2],
        forward_index: [0; VIF_POSIT + 2],
        reverse_index: [0; VIF_POSIT + 2],
        hineighbor: [0; VIF_POSIT],
        loneighbor: [0; VIF_POSIT],
        posts: 0,
        n: 0,
        quant_q: 0,
        vi: *info,
    });
    let mut n: i32 = 0;

    look.n = info.postlist[1];

    /* we drop each position value in-between already decoded values,
    and use linear interpolation to predict each new value past the
    edges.  The positions are read in the order of the position
    list... we precompute the bounding positions in the lookup.  Of
    course, the neighbors can change (if a position is declined), but
    this is an initial mapping */

    for i in 0..info.partitions as usize {
        n += info.class_dim[info.partitionclass[i] as usize];
    }
    n += 2;
    look.posts = n;

    /* also store a sorted position index */
    let sortpointer = sort_posts(&info.postlist, n as usize);

    /* points from sort order back to range number */
    for i in 0..n as usize {
        look.forward_index[i] = sortpointer[i] as i32;
    }
    /* points from range order to sorted position */
    for i in 0..n as usize {
        look.reverse_index[look.forward_index[i] as usize] = i as i32;
    }
    /* we actually need the post values too */
    for i in 0..n as usize {
        look.sorted_index[i] = info.postlist[look.forward_index[i] as usize];
    }

    /* quantize values to multiplier spec */
    match info.mult {
        1 => {
            /* 1024 -> 256 */
            look.quant_q = 256;
        }
        2 => {
            /* 1024 -> 128 */
            look.quant_q = 128;
        }
        3 => {
            /* 1024 -> 86 */
            look.quant_q = 86;
        }
        4 => {
            /* 1024 -> 64 */
            look.quant_q = 64;
        }
        _ => {}
    }

    /* discover our neighbors for decode where we don't use fit flags
    (that would push the neighbors outward) */
    for i in 0..(n - 2).max(0) as usize {
        let mut lo: i32 = 0;
        let mut hi: i32 = 1;
        let mut lx: i32 = 0;
        let mut hx: i32 = look.n;
        let currentx = info.postlist[i + 2];
        for j in 0..i + 2 {
            let x = info.postlist[j];
            if x > lx && x < currentx {
                lo = j as i32;
                lx = x;
            }
            if x < hx && x > currentx {
                hi = j as i32;
                hx = x;
            }
        }
        look.loneighbor[i] = lo;
        look.hineighbor[i] = hi;
    }

    look
}

/// Translation of `render_point()`.
fn render_point(x0: i32, x1: i32, mut y0: i32, mut y1: i32, x: i32) -> i32 {
    y0 &= 0x7fff; /* mask off flag */
    y1 &= 0x7fff;

    {
        let dy = y1 - y0;
        let adx = x1 - x0;
        let ady = dy.abs();
        let err = ady * (x - x0);

        let off = err / adx;
        if dy < 0 {
            return y0 - off;
        }
        y0 + off
    }
}

#[rustfmt::skip]
static FLOOR1_FROMDB_LOOKUP: [f32; 256] = [
    1.0649863e-07, 1.1341951e-07, 1.2079015e-07, 1.2863978e-07,
    1.3699951e-07, 1.4590251e-07, 1.5538408e-07, 1.6548181e-07,
    1.7623575e-07, 1.8768855e-07, 1.9988561e-07, 2.128753e-07,
    2.2670913e-07, 2.4144197e-07, 2.5713223e-07, 2.7384213e-07,
    2.9163793e-07, 3.1059021e-07, 3.3077411e-07, 3.5226968e-07,
    3.7516214e-07, 3.9954229e-07, 4.2550680e-07, 4.5315863e-07,
    4.8260743e-07, 5.1396998e-07, 5.4737065e-07, 5.8294187e-07,
    6.2082472e-07, 6.6116941e-07, 7.0413592e-07, 7.4989464e-07,
    7.9862701e-07, 8.5052630e-07, 9.0579828e-07, 9.6466216e-07,
    1.0273513e-06, 1.0941144e-06, 1.1652161e-06, 1.2409384e-06,
    1.3215816e-06, 1.4074654e-06, 1.4989305e-06, 1.5963394e-06,
    1.7000785e-06, 1.8105592e-06, 1.9282195e-06, 2.0535261e-06,
    2.1869758e-06, 2.3290978e-06, 2.4804557e-06, 2.6416497e-06,
    2.8133190e-06, 2.9961443e-06, 3.1908506e-06, 3.3982101e-06,
    3.6190449e-06, 3.8542308e-06, 4.1047004e-06, 4.3714470e-06,
    4.6555282e-06, 4.9580707e-06, 5.2802740e-06, 5.6234160e-06,
    5.9888572e-06, 6.3780469e-06, 6.7925283e-06, 7.2339451e-06,
    7.7040476e-06, 8.2047000e-06, 8.7378876e-06, 9.3057248e-06,
    9.9104632e-06, 1.0554501e-05, 1.1240392e-05, 1.1970856e-05,
    1.2748789e-05, 1.3577278e-05, 1.4459606e-05, 1.5399272e-05,
    1.6400004e-05, 1.7465768e-05, 1.8600792e-05, 1.9809576e-05,
    2.1096914e-05, 2.2467911e-05, 2.3928002e-05, 2.5482978e-05,
    2.7139006e-05, 2.8902651e-05, 3.0780908e-05, 3.2781225e-05,
    3.4911534e-05, 3.7180282e-05, 3.9596466e-05, 4.2169667e-05,
    4.4910090e-05, 4.7828601e-05, 5.0936773e-05, 5.4246931e-05,
    5.7772202e-05, 6.1526565e-05, 6.5524908e-05, 6.9783085e-05,
    7.4317983e-05, 7.9147585e-05, 8.4291040e-05, 8.9768747e-05,
    9.5602426e-05, 0.00010181521, 0.00010843174, 0.00011547824,
    0.00012298267, 0.00013097477, 0.00013948625, 0.00014855085,
    0.00015820453, 0.00016848555, 0.00017943469, 0.00019109536,
    0.00020351382, 0.00021673929, 0.00023082423, 0.00024582449,
    0.00026179955, 0.00027881276, 0.00029693158, 0.00031622787,
    0.00033677814, 0.00035866388, 0.00038197188, 0.00040679456,
    0.00043323036, 0.00046138411, 0.00049136745, 0.00052329927,
    0.00055730621, 0.00059352311, 0.00063209358, 0.00067317058,
    0.00071691700, 0.00076350630, 0.00081312324, 0.00086596457,
    0.00092223983, 0.00098217216, 0.0010459992, 0.0011139742,
    0.0011863665, 0.0012634633, 0.0013455702, 0.0014330129,
    0.0015261382, 0.0016253153, 0.0017309374, 0.0018434235,
    0.0019632195, 0.0020908006, 0.0022266726, 0.0023713743,
    0.0025254795, 0.0026895994, 0.0028643847, 0.0030505286,
    0.0032487691, 0.0034598925, 0.0036847358, 0.0039241906,
    0.0041792066, 0.0044507950, 0.0047400328, 0.0050480668,
    0.0053761186, 0.0057254891, 0.0060975636, 0.0064938176,
    0.0069158225, 0.0073652516, 0.0078438871, 0.0083536271,
    0.0088964928, 0.009474637, 0.010090352, 0.010746080,
    0.011444421, 0.012188144, 0.012980198, 0.013823725,
    0.014722068, 0.015678791, 0.016697687, 0.017782797,
    0.018938423, 0.020169149, 0.021479854, 0.022875735,
    0.024362330, 0.025945531, 0.027631618, 0.029427276,
    0.031339626, 0.033376252, 0.035545228, 0.037855157,
    0.040315199, 0.042935108, 0.045725273, 0.048696758,
    0.051861348, 0.055231591, 0.058820850, 0.062643361,
    0.066714279, 0.071049749, 0.075666962, 0.080584227,
    0.085821044, 0.091398179, 0.097337747, 0.10366330,
    0.11039993, 0.11757434, 0.12521498, 0.13335215,
    0.14201813, 0.15124727, 0.16107617, 0.17154380,
    0.18269168, 0.19456402, 0.20720788, 0.22067342,
    0.23501402, 0.25028656, 0.26655159, 0.28387361,
    0.30232132, 0.32196786, 0.34289114, 0.36517414,
    0.38890521, 0.41417847, 0.44109412, 0.46975890,
    0.50028648, 0.53279791, 0.56742212, 0.60429640,
    0.64356699, 0.68538959, 0.72993007, 0.77736504,
    0.82788260, 0.88168307, 0.9389798, 1.,
];

/// Translation of `render_line()`.
fn render_line(mut n: i32, x0: i32, x1: i32, y0: i32, y1: i32, d: &mut [f32]) {
    let dy = y1 - y0;
    let adx = x1 - x0;
    let mut ady = dy.abs();
    let base = dy / adx;
    let sy = if dy < 0 { base - 1 } else { base + 1 };
    let mut x = x0;
    let mut y = y0;
    let mut err = 0;

    ady -= (base * adx).abs();

    if n > x1 {
        n = x1;
    }

    if x < n {
        d[x as usize] *= FLOOR1_FROMDB_LOOKUP[y as usize];
    }

    loop {
        x += 1;
        if x >= n {
            break;
        }
        err += ady;
        if err >= adx {
            err -= adx;
            y += sy;
        } else {
            y += base;
        }
        d[x as usize] *= FLOOR1_FROMDB_LOOKUP[y as usize];
    }
}

/// Translation of `floor1_inverse1()`.
pub(crate) fn floor1_inverse1(
    opb: &mut OggpackBuffer<'_>,
    ci: &CodecSetupInfo,
    look: &VorbisLookFloor1,
) -> Option<FloorMemo> {
    let info = &look.vi;

    let books = ci.fullbooks.as_deref().unwrap();

    /* unpack wrapped/predicted values from stream */
    if oggpack_read(opb, 1) == 1 {
        let mut fit_value = vec![0i32; look.posts as usize];

        fit_value[0] = oggpack_read(opb, ov_ilog((look.quant_q - 1) as u32)) as i32;
        fit_value[1] = oggpack_read(opb, ov_ilog((look.quant_q - 1) as u32)) as i32;

        /* partition by partition */
        let mut j = 2usize;
        for i in 0..info.partitions as usize {
            let class = info.partitionclass[i] as usize;
            let cdim = info.class_dim[class];
            let csubbits = info.class_subs[class];
            let csub = 1 << csubbits;
            let mut cval: i32 = 0;

            /* decode the partition's first stage cascade value */
            if csubbits != 0 {
                cval = vorbis_book_decode(&books[info.class_book[class] as usize], opb) as i32;

                if cval == -1 {
                    return None;
                }
            }

            for k in 0..cdim as usize {
                let book = info.class_subbook[class][(cval & (csub - 1)) as usize];
                cval >>= csubbits;
                if book >= 0 {
                    fit_value[j + k] = vorbis_book_decode(&books[book as usize], opb) as i32;
                    if fit_value[j + k] == -1 {
                        return None;
                    }
                } else {
                    fit_value[j + k] = 0;
                }
            }
            j += cdim as usize;
        }

        /* unwrap positive values and reconsitute via linear interpolation */
        for i in 2..look.posts as usize {
            let lo = look.loneighbor[i - 2] as usize;
            let hi = look.hineighbor[i - 2] as usize;
            let predicted = render_point(
                info.postlist[lo],
                info.postlist[hi],
                fit_value[lo],
                fit_value[hi],
                info.postlist[i],
            );
            let hiroom = look.quant_q - predicted;
            let loroom = predicted;
            let room = (if hiroom < loroom { hiroom } else { loroom }) << 1;
            let mut val = fit_value[i];

            if val != 0 {
                if val >= room {
                    if hiroom > loroom {
                        val -= loroom;
                    } else {
                        val = -1 - (val - hiroom);
                    }
                } else if val & 1 != 0 {
                    val = -((val + 1) >> 1);
                } else {
                    val >>= 1;
                }

                fit_value[i] = (val + predicted) & 0x7fff;
                fit_value[lo] &= 0x7fff;
                fit_value[hi] &= 0x7fff;
            } else {
                fit_value[i] = predicted | 0x8000;
            }
        }

        return Some(FloorMemo::Floor1(fit_value));
    }
    /* eop: */
    None
}

/// Translation of `floor1_inverse2()`.
pub(crate) fn floor1_inverse2(
    vb: &VorbisBlock,
    ci: &CodecSetupInfo,
    look: &VorbisLookFloor1,
    memo: Option<&[i32]>,
    out: &mut [f32],
) -> i32 {
    let info = &look.vi;

    let n = (ci.blocksizes[vb.W as usize] / 2) as i32;

    if let Some(fit_value) = memo {
        /* render the lines */
        let mut hx: i32 = 0;
        let mut lx: i32 = 0;
        let mut ly: i32 = fit_value[0].wrapping_mul(info.mult);
        /* guard lookup against out-of-range values */
        ly = ly.clamp(0, 255);

        for j in 1..look.posts as usize {
            let current = look.forward_index[j] as usize;
            let mut hy = fit_value[current] & 0x7fff;
            if hy == fit_value[current] {
                hx = info.postlist[current];
                hy *= info.mult;
                /* guard lookup against out-of-range values */
                hy = hy.clamp(0, 255);

                render_line(n, lx, hx, ly, hy, out);

                lx = hx;
                ly = hy;
            }
        }
        for j in hx.max(0)..n {
            out[j as usize] *= FLOOR1_FROMDB_LOOKUP[ly as usize]; /* be certain */
        }
        return 1;
    }
    for v in out.iter_mut().take(n as usize) {
        *v = 0.0;
    }
    0
}
