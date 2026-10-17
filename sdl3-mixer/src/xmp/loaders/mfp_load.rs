// Rust translation of src/loaders/mfp_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/*
 * A module packer created by Shaun Southern. Samples are stored in a
 * separate file. File prefixes are mfp for song and smp for samples. For
 * more information see http://www.exotica.org.uk/wiki/Magnetic_Fields_Packer
 */

//! The Magnetic Fields Packer loader.
//!
//! Upstream only accepts these from `HIO_HANDLE_TYPE_FILE` handles, which
//! come from the path and `FILE *` entry points; those aren't translated
//! (SDL_mixer loads modules from memory or callbacks), so the test always
//! fails here, as it does under SDL_mixer.

use super::super::common::*;
use super::super::dataio::readmem16b;
use super::super::hio::*;
use super::super::path::*;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

pub(crate) const LIBXMP_LOADER_MFP: FormatLoader = FormatLoader {
    name: "Magnetic Fields Packer",
    test: mfp_test,
    loader: mfp_load,
};

/// `HIO_HANDLE_TYPE(f) == HIO_HANDLE_TYPE_FILE`: never, as only memory and
/// callback handles are given to the loaders.
fn hio_handle_type_is_file(_f: &HioHandle<'_>) -> bool {
    false
}

/// Translation of `mfp_test()`.
fn mfp_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    let mut buf = [0u8; 384];

    if !hio_handle_type_is_file(f) {
        return -1;
    }

    if hio_read(&mut buf, 1, 384, f) < 384 {
        return -1;
    }

    /* check restart byte */
    if buf[249] != 0x7f {
        return -1;
    }

    for i in 0..31 {
        /* check size */
        let len = readmem16b(&buf[i * 8..]) as i32;
        if len > 0x7fff {
            return -1;
        }

        /* check finetune */
        if buf[i * 8 + 2] & 0xf0 != 0 {
            return -1;
        }

        /* check volume */
        if buf[i * 8 + 3] > 0x40 {
            return -1;
        }

        /* check loop start */
        let lps = readmem16b(&buf[i * 8 + 4..]) as i32;
        if lps > len {
            return -1;
        }

        /* check loop size */
        let lsz = readmem16b(&buf[i * 8 + 6..]) as i32;
        if lps + lsz - 1 > len {
            return -1;
        }

        if len > 0 && lsz == 0 {
            return -1;
        }
    }

    if buf[248] as u16 != readmem16b(&buf[378..]) {
        return -1;
    }

    if readmem16b(&buf[378..]) != readmem16b(&buf[380..]) {
        return -1;
    }

    libxmp_read_title(f, t, 0);

    0
}

/// Translation of `struct mfp_map`.
#[derive(Clone, Copy, Default)]
struct MfpMap {
    offset: u16,
    ord: u8,
    chn: u8,
}

/// Translation of `mfp_load()`.
fn mfp_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut sp = LibxmpPath::default();
    let mut buf = [0u8; 255 * 2 + 4];

    libxmp_set_type(m, b"Magnetic Fields Packer");

    let mod_ = &mut m.mod_;
    mod_.chn = 4;
    mod_.smp = 31;
    mod_.ins = mod_.smp;

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    for i in 0..31 {
        let iu = i as usize;

        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        let xxs = &mut mod_.xxs[iu];
        let xxi = &mut mod_.xxi[iu];
        xxs.len = 2 * hio_read16b(f) as i32;
        xxi.sub[0].fin = (hio_read8(f) << 4) as i8 as i32;
        xxi.sub[0].vol = hio_read8(f) as i32;
        xxs.lps = 2 * hio_read16b(f) as i32;
        let loop_size = hio_read16b(f) as i32;

        xxs.lpe = xxs.lps + 2 * loop_size;
        xxs.flg = if loop_size > 1 { XMP_SAMPLE_LOOP } else { 0 };
        xxi.sub[0].pan = XMP_INST_NO_DEFAULT_PAN;
        xxi.sub[0].sid = i;
        xxi.rls = 0xfff;

        if xxs.len > 0 {
            xxi.nsm = 1;
        }
    }

    mod_.pat = hio_read8(f) as i32;
    mod_.len = mod_.pat;
    hio_read8(f); /* restart */

    /*
    for (i = 0; i < 128; i++) {
        mod->xxo[i] = hio_read8(f);
    }
    if (hio_error(f)) {
        return -1;
    }
    */
    if hio_seek(f, 128, SEEK_CUR) != 0 {
        return -1;
    }
    for i in 0..mod_.len as usize {
        mod_.xxo[i] = i as u8;
    }

    /* Read and convert patterns */

    let size1 = hio_read16b(f) as i32;
    /* size2 = */
    hio_read16b(f);
    if size1 > mod_.len {
        return -1;
    }

    /* What follows is a list of order (not pattern) track offsets.
     * To avoid duplicating tracks, try to merge track offsets.
     *
     * It is probably possible to merge patterns to match the original
     * order list, but not really worth the effort.
     */
    let n = size1 as usize * 4;
    let Ok(mut mapping) = try_vec(n, MfpMap::default()) else {
        return -1;
    };
    let Ok(mut track_offs) = try_vec(n, 0u16) else {
        return -1;
    };

    let mut current = 0;
    for i in 0..size1 {
        /* Read pattern table */
        for j in 0..4 {
            mapping[current].offset = hio_read16b(f);
            mapping[current].ord = i as u8;
            mapping[current].chn = j;
            current += 1;
        }
    }

    /* (equal offsets get the same track below, so a stable sort gives the
     * same result as qsort()) */
    mapping.sort_by_key(|x| x.offset);

    mod_.trk = 0;
    let mut i = 0;
    while i < n {
        let offset = mapping[i].offset;
        track_offs[mod_.trk as usize] = offset;

        while i < n && mapping[i].offset == offset {
            /* Write track # to offset to save an extra field. */
            mapping[i].offset = mod_.trk as u16;
            i += 1;
        }
        mod_.trk += 1;
    }

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    for i in 0..mod_.len {
        if libxmp_alloc_pattern(mod_, i) < 0 {
            return -1;
        }
        mod_.pattern_mut(i).rows = 64;
    }

    for map in &mapping {
        /* offset was replaced with the track number. */
        mod_.pattern_mut(map.ord as i32).index[map.chn as usize] = map.offset as i32;
    }
    drop(mapping);

    let pat_addr = hio_tell(f);

    for i in 0..mod_.trk {
        let iu = i as usize;

        if libxmp_alloc_track(mod_, i, 64) < 0 {
            return -1;
        }

        if hio_seek(f, pat_addr + track_offs[iu] as i64, SEEK_SET) < 0 {
            return -1;
        }

        let len = if i < mod_.trk - 1 {
            track_offs[iu + 1] as usize - track_offs[iu] as usize
        } else {
            buf.len()
        };

        let len = hio_read(&mut buf, 1, len.min(255 * 2 + 4), f);

        let track = mod_.track_mut(i);
        let mut e = 0;
        for k in 0..4 {
            for x in 0..4 {
                for y in 0..4 {
                    if k >= len
                        || buf[k] as usize + x >= len
                        || buf[buf[k] as usize + x] as usize + y >= len
                        || buf[buf[buf[k] as usize + x] as usize + y] as usize * 2 + 4 > len
                    {
                        return -1;
                    }
                    let p = buf[buf[buf[k] as usize + x] as usize + y] as usize * 2;
                    libxmp_decode_protracker_event(&mut track.event[e], &buf[p..p + 4]);
                    e += 1;
                }
            }
        }
    }
    drop(track_offs);

    /* Read samples */

    libxmp_path_init(&mut sp);

    'err: {
        /* first check smp.filename */
        let Some(basename) = m.basename.as_mut() else {
            break 'err;
        };
        if strlen(basename) < 5 || basename[3] != b'.' {
            break 'err;
        }

        basename[0] = b's';
        basename[1] = b'm';
        basename[2] = b'p';

        if libxmp_path_join(&mut sp, m.dirname.as_deref(), m.basename.as_deref()) != 0 {
            break 'err;
        }

        let mut s = sp.path.as_deref().and_then(hio_open);
        if s.is_none() {
            /* handle .set filenames like in Kid Chaos*/
            let basename = m.basename.as_deref().unwrap_or_default();
            if basename[..strlen(basename)].contains(&b'-') {
                let path = sp.path.as_deref().unwrap_or_default();
                if let Some(ppos) = path[..sp.length].iter().rposition(|&c| c == b'-') {
                    if libxmp_path_suffix_at(&mut sp, ppos, Some(b".set")) != 0 {
                        break 'err;
                    }
                }
            }
            s = sp.path.as_deref().and_then(hio_open);
            if s.is_none() {
                break 'err;
            }
        }
        libxmp_path_free(&mut sp);
        let mut s = s.unwrap();

        for i in 0..m.mod_.ins as usize {
            let sid = m.mod_.xxi[i].sub[0].sid as usize;
            let smpctl = m.smpctl;
            if libxmp_load_sample(
                smpctl,
                Some(&mut s),
                SAMPLE_FLAG_FULLREP,
                &mut m.mod_.xxs[sid],
                None,
            ) < 0
            {
                return -1;
            }
        }

        m.period_type = PERIOD_MODRNG;

        return 0;
    }

    /* err: */
    libxmp_path_free(&mut sp);
    for i in 0..m.mod_.ins as usize {
        m.mod_.xxi[i].nsm = 0;
        m.mod_.xxs[i] = XmpSample::default();
    }

    0
}
