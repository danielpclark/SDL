// Rust translation of the decoding half of lib/res0.c from libvorbis
// 1.3.7 (https://xiph.org/vorbis/, at the revision SDL_mixer's
// external/vorbis pins).
// THE OggVorbis SOURCE CODE IS (C) COPYRIGHT 1994-2015
// by the Xiph.Org Foundation https://xiph.org/
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! function: residue backend 0, 1 and 2 implementation
//!
//! Slow, slow, slow, simpleminded and did I mention it was slow?  The
//! encode/decode loops are coded for clarity and performance is not yet
//! even a nagging little idea lurking in the shadows.  Oh and BTW, it's
//! slow.
//!
//! The encoder's half (`res0_pack()`, the `_01class()`/`_2class()`
//! classifiers, `_encodepart()`, `_01forward()` and the `res*_forward()`
//! and `res*_class()` entry points, the `TRAIN_RES` training) isn't
//! translated. The look keeps a copy of its setup and the indices of its
//! books in the `codec_setup_info`'s `fullbooks` (C points at them), and
//! C's `decodemap` array of arrays is one array of `partvals` rows of
//! `dim` entries.

use super::codebook::*;
use super::sharedbook::ov_ilog;
use super::*;
use crate::ogg::bitwise::*;

/// Translation of `vorbis_info_residue0` (backends.h): block-partitioned
/// VQ coded straight residue (the encoder's class metrics aren't kept).
#[derive(Clone, Copy, Debug)]
pub(crate) struct VorbisInfoResidue0 {
    /* block-partitioned VQ coded straight residue */
    pub begin: i64,
    pub end: i64,

    /* first stage (lossless partitioning) */
    pub grouping: i32,           /* group n vectors per partition */
    pub partitions: i32,         /* possible codebooks for a partition */
    pub partvals: i32,           /* partitions ^ groupbook dim */
    pub groupbook: i32,          /* huffbook for partitioning */
    pub secondstages: [i32; 64], /* expanded out to pointers in lookup */
    pub booklist: [i32; 512],    /* list of second stage books */
}

impl Default for VorbisInfoResidue0 {
    fn default() -> Self {
        VorbisInfoResidue0 {
            begin: 0,
            end: 0,
            grouping: 0,
            partitions: 0,
            partvals: 0,
            groupbook: 0,
            secondstages: [0; 64],
            booklist: [0; 512],
        }
    }
}

/// Translation of `vorbis_look_residue0`.
#[derive(Debug, Default)]
pub(crate) struct VorbisLookResidue0 {
    pub info: VorbisInfoResidue0,

    pub parts: i32,
    pub stages: i32,
    /* (fullbooks is the codec_setup_info's) */
    pub phrasebook: usize,
    pub partbooks: Vec<Vec<Option<usize>>>,

    pub partvals: i32,
    pub decodemap: Vec<i32>,
    pub decodemap_dim: usize,
    /* (postbits, phrasebits and frames are never counted) */
}

/// Translation of `icount()`.
fn icount(mut v: u32) -> i32 {
    let mut ret = 0;
    while v != 0 {
        ret += (v & 1) as i32;
        v >>= 1;
    }
    ret
}

/* vorbis_info is for range checking */
/// Translation of `res0_unpack()`.
pub(crate) fn res0_unpack(
    ci: &CodecSetupInfo,
    opb: &mut OggpackBuffer<'_>,
) -> Option<Box<VorbisInfoResidue0>> {
    let mut acc: i32 = 0;
    let mut info = Box::<VorbisInfoResidue0>::default();

    info.begin = oggpack_read(opb, 24);
    info.end = oggpack_read(opb, 24);
    info.grouping = oggpack_read(opb, 24) as i32 + 1;
    info.partitions = oggpack_read(opb, 6) as i32 + 1;
    info.groupbook = oggpack_read(opb, 8) as i32;

    /* check for premature EOP */
    if info.groupbook < 0 {
        return None;
    }

    for j in 0..info.partitions as usize {
        let mut cascade = oggpack_read(opb, 3) as i32;
        let cflag = oggpack_read(opb, 1) as i32;
        if cflag < 0 {
            return None;
        }
        if cflag != 0 {
            let c = oggpack_read(opb, 5) as i32;
            if c < 0 {
                return None;
            }
            cascade |= c << 3;
        }
        info.secondstages[j] = cascade;

        acc += icount(cascade as u32);
    }
    for j in 0..acc as usize {
        let book = oggpack_read(opb, 8) as i32;
        if book < 0 {
            return None;
        }
        info.booklist[j] = book;
    }

    if info.groupbook >= ci.books {
        return None;
    }
    for j in 0..acc as usize {
        if info.booklist[j] >= ci.books {
            return None;
        }
        if ci.book_param[info.booklist[j] as usize].as_ref()?.maptype == 0 {
            return None;
        }
    }

    /* verify the phrasebook is not specifying an impossible or
    inconsistent partitioning scheme. */
    /* modify the phrasebook ranging check from r16327; an early beta
    encoder had a bug where it used an oversized phrasebook by
    accident.  These files should continue to be playable, but don't
    allow an exploit */
    {
        let groupbook = ci.book_param[info.groupbook as usize].as_ref()?;
        let entries = groupbook.entries as i32;
        let mut dim = groupbook.dim as i32;
        let mut partvals: i32 = 1;
        if dim < 1 {
            return None;
        }
        while dim > 0 {
            partvals *= info.partitions;
            if partvals > entries {
                return None;
            }
            dim -= 1;
        }
        info.partvals = partvals;
    }

    Some(info)
}

/// Translation of `res0_look()`: `None` is the allocation failure (of a
/// `decodemap` sized from the stream) that C would crash on.
pub(crate) fn res0_look(
    ci: &CodecSetupInfo,
    info: &VorbisInfoResidue0,
) -> Option<VorbisLookResidue0> {
    let fullbooks = ci.fullbooks.as_deref().unwrap();
    let mut look = VorbisLookResidue0::default();

    let mut acc: usize = 0;
    let dim: i32;
    let mut maxstage: i32 = 0;
    look.info = *info;

    look.parts = info.partitions;
    look.phrasebook = info.groupbook as usize;
    dim = fullbooks[look.phrasebook].dim as i32;

    look.partbooks = vec![Vec::new(); look.parts as usize];

    for j in 0..look.parts as usize {
        let stages = ov_ilog(info.secondstages[j] as u32);
        if stages != 0 {
            if stages > maxstage {
                maxstage = stages;
            }
            look.partbooks[j] = vec![None; stages as usize];
            for k in 0..stages as usize {
                if info.secondstages[j] & (1 << k) != 0 {
                    look.partbooks[j][k] = Some(info.booklist[acc] as usize);
                    acc += 1;
                }
            }
        }
    }

    look.partvals = 1;
    for _ in 0..dim {
        look.partvals *= look.parts;
    }

    look.stages = maxstage;
    look.decodemap_dim = dim as usize;
    look.decodemap = try_vec::<i32>(look.partvals as usize * dim as usize)?;
    for j in 0..look.partvals as usize {
        let mut val: i64 = j as i64;
        let mut mult: i64 = (look.partvals / look.parts) as i64;
        for k in 0..dim as usize {
            let deco = val / mult;
            val -= deco * mult;
            mult /= look.parts as i64;
            look.decodemap[j * dim as usize + k] = deco as i32;
        }
    }
    Some(look)
}

/// The decode function `_01inverse()` is given: `vorbis_book_decodevs_add`
/// (residue 0) or `vorbis_book_decodev_add` (residue 1).
type DecodePart = fn(&Codebook, &mut [f32], &mut OggpackBuffer<'_>, i32) -> i64;

/* a truncated packet here just means 'stop working'; it's not an error */
/// Translation of `_01inverse()`: `chans` are the channels of `pcm` that
/// C's `in` points at.
fn _01inverse(
    vb: &mut VorbisBlock,
    opb: &mut OggpackBuffer<'_>,
    fullbooks: &[Codebook],
    look: &VorbisLookResidue0,
    chans: &[usize],
    ch: i32,
    decodepart: DecodePart,
) -> i32 {
    let info = &look.info;
    let phrasebook = &fullbooks[look.phrasebook];

    /* move all this setup out later */
    let samples_per_partition = info.grouping;
    let partitions_per_word = phrasebook.dim as i32;
    let max = vb.pcmend >> 1;
    let end = if info.end < max as i64 {
        info.end as i32
    } else {
        max
    };
    let n = end - info.begin as i32;

    if n > 0 {
        let partvals = n / samples_per_partition;
        let partwords = (partvals + partitions_per_word - 1) / partitions_per_word;
        /* (partword[j][l] is the row of decodemap) */
        let mut partword = vec![vec![0usize; partwords.max(0) as usize]; ch as usize];

        for s in 0..look.stages {
            /* each loop decodes on partition codeword containing
            partitions_per_word partitions */
            let mut i: i64 = 0;
            let mut l: usize = 0;
            while i < partvals as i64 {
                if s == 0 {
                    /* fetch the partition word for each channel */
                    for j in 0..ch as usize {
                        let temp = vorbis_book_decode(phrasebook, opb) as i32;

                        if temp == -1 || temp >= info.partvals {
                            return 0; /* eopbreak */
                        }
                        partword[j][l] = temp as usize;
                    }
                }

                /* now we decode residual values for the partitions */
                let mut k: i32 = 0;
                while k < partitions_per_word && i < partvals as i64 {
                    for j in 0..ch as usize {
                        let offset = info.begin + i * samples_per_partition as i64;
                        let part = look.decodemap[partword[j][l] * look.decodemap_dim + k as usize]
                            as usize;
                        if info.secondstages[part] & (1 << s) != 0 {
                            if let Some(stagebook) = look.partbooks[part][s as usize] {
                                let a = &mut vb.pcm[chans[j]][offset as usize..];
                                if decodepart(&fullbooks[stagebook], a, opb, samples_per_partition)
                                    == -1
                                {
                                    return 0; /* eopbreak */
                                }
                            }
                        }
                    }
                    k += 1;
                    i += 1;
                }
                l += 1;
            }
        }
    }
    /* errout: */
    /* eopbreak: */
    0
}

/// Translation of `res0_inverse()`: `chans` are the channels of the
/// submap's bundle (C's `in`).
pub(crate) fn res0_inverse(
    vb: &mut VorbisBlock,
    opb: &mut OggpackBuffer<'_>,
    fullbooks: &[Codebook],
    look: &VorbisLookResidue0,
    chans: &[usize],
    nonzero: &[i32],
    ch: i32,
) -> i32 {
    let mut used = Vec::with_capacity(ch as usize);
    for i in 0..ch as usize {
        if nonzero[i] != 0 {
            used.push(chans[i]);
        }
    }
    if !used.is_empty() {
        _01inverse(
            vb,
            opb,
            fullbooks,
            look,
            &used,
            used.len() as i32,
            vorbis_book_decodevs_add,
        )
    } else {
        0
    }
}

/// Translation of `res1_inverse()`.
pub(crate) fn res1_inverse(
    vb: &mut VorbisBlock,
    opb: &mut OggpackBuffer<'_>,
    fullbooks: &[Codebook],
    look: &VorbisLookResidue0,
    chans: &[usize],
    nonzero: &[i32],
    ch: i32,
) -> i32 {
    let mut used = Vec::with_capacity(ch as usize);
    for i in 0..ch as usize {
        if nonzero[i] != 0 {
            used.push(chans[i]);
        }
    }
    if !used.is_empty() {
        _01inverse(
            vb,
            opb,
            fullbooks,
            look,
            &used,
            used.len() as i32,
            vorbis_book_decodev_add,
        )
    } else {
        0
    }
}

/* duplicate code here as speed is somewhat more important */
/// Translation of `res2_inverse()`.
pub(crate) fn res2_inverse(
    vb: &mut VorbisBlock,
    opb: &mut OggpackBuffer<'_>,
    fullbooks: &[Codebook],
    look: &VorbisLookResidue0,
    chans: &[usize],
    nonzero: &[i32],
    ch: i32,
) -> i32 {
    let info = &look.info;
    let phrasebook = &fullbooks[look.phrasebook];

    /* move all this setup out later */
    let samples_per_partition = info.grouping;
    let partitions_per_word = phrasebook.dim as i32;
    let max = (vb.pcmend * ch) >> 1;
    let end = if info.end < max as i64 {
        info.end as i32
    } else {
        max
    };
    let n = end - info.begin as i32;

    if n > 0 {
        let partvals = n / samples_per_partition;
        let partwords = (partvals + partitions_per_word - 1) / partitions_per_word;
        let mut partword = vec![0usize; partwords.max(0) as usize];

        let mut i = 0usize;
        while i < ch as usize {
            if nonzero[i] != 0 {
                break;
            }
            i += 1;
        }
        if i == ch as usize {
            return 0; /* no nonzero vectors */
        }

        for s in 0..look.stages {
            let mut i: i64 = 0;
            let mut l: usize = 0;
            while i < partvals as i64 {
                if s == 0 {
                    /* fetch the partition word */
                    let temp = vorbis_book_decode(phrasebook, opb) as i32;
                    if temp == -1 || temp >= info.partvals {
                        return 0; /* eopbreak */
                    }
                    partword[l] = temp as usize;
                }

                /* now we decode residual values for the partitions */
                let mut k: i32 = 0;
                while k < partitions_per_word && i < partvals as i64 {
                    let part =
                        look.decodemap[partword[l] * look.decodemap_dim + k as usize] as usize;
                    if info.secondstages[part] & (1 << s) != 0 {
                        if let Some(stagebook) = look.partbooks[part][s as usize] {
                            if vorbis_book_decodevv_add(
                                &fullbooks[stagebook],
                                &mut vb.pcm,
                                chans,
                                i * samples_per_partition as i64 + info.begin,
                                ch,
                                opb,
                                samples_per_partition,
                            ) == -1
                            {
                                return 0; /* eopbreak */
                            }
                        }
                    }
                    k += 1;
                    i += 1;
                }
                l += 1;
            }
        }
    }
    /* errout: */
    /* eopbreak: */
    0
}
