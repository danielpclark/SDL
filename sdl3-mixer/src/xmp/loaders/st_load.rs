// Rust translation of src/loaders/st_load.c (and struct st_header of src/loaders/mod.h) from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr (st_load.c)
// Copyright (C) 1996-2021 Claudio Matsuoka and Hipolito Carraro Jr (mod.h)
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Ultimate Soundtracker support based on the module format description
 * written by Michael Schwendt <sidplay@geocities.com>
 */

//! The 15 instrument Soundtracker loader.

use super::super::common::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::mod_load::ModInstrument;
use super::sample::libxmp_load_sample;
use super::*;

/// Translation of `struct st_header`.
#[derive(Clone, Copy)]
struct StHeader {
    name: [u8; 20],
    ins: [ModInstrument; 15],
    len: u8,
    restart: u8,
    order: [u8; 128],
}

impl Default for StHeader {
    fn default() -> Self {
        StHeader {
            name: [0; 20],
            ins: [ModInstrument::default(); 15],
            len: 0,
            restart: 0,
            order: [0; 128],
        }
    }
}

pub(crate) const LIBXMP_LOADER_ST: FormatLoader = FormatLoader {
    name: "Soundtracker",
    test: st_test,
    loader: st_load,
};

/* musanx.mod contains 22 period and instrument errors */
const ST_MAX_PATTERN_ERRORS: i32 = 22;
/* Allow some degree of sample truncation for ST modules.
 * The worst known module currently is u2.mod with 7% truncation. */
const ST_TRUNCATION_LIMIT: i32 = 93;

#[rustfmt::skip]
static PERIOD: [i32; 52] = [
    856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453,
    428, 404, 381, 360, 339, 320, 302, 285, 269, 254, 240, 226,
    214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120, 113,
    /* Off-by-one period values found in blueberry.mod, snd.mod,
     * quite a lot.mod, sweet dreams.mod, and bar----fringdus.mod */
    763, 679, 641, 571, 539, 509, 429, 340, 321, 300, 286, 270,
    227, 191, 162,
    -1,
];

/// Translation of `st_expected_size()`.
fn st_expected_size(smp_size: i32, pat: i32) -> i32 {
    600 + smp_size + 1024 * pat
}

/// Reads the instruments of the header (as both functions do).
fn read_st_instrument(ins: &mut ModInstrument, f: &mut HioHandle<'_>) {
    hio_read(&mut ins.name, 1, 22, f);
    ins.size = hio_read16b(f);
    ins.finetune = hio_read8(f) as i8;
    ins.volume = hio_read8(f) as i8;
    ins.loop_start = hio_read16b(f);
    ins.loop_size = hio_read16b(f);
}

/// Translation of `st_test()`.
fn st_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, start: i32) -> i32 {
    let mut mh = StHeader::default();
    let mut pat_buf = [0u8; 1024];
    let mut test_flags = TEST_NAME_IGNORE_AFTER_CR;

    let size = hio_size(f);

    if size < 600 {
        return -1;
    }

    let mut smp_size = 0;

    hio_seek(f, start as i64, SEEK_SET);
    hio_read(&mut mh.name, 1, 20, f);
    /* The Super Ski 2 modules have unusual "SONG\x13\x88" names. */
    if mh.name[5] == 0x88 {
        mh.name[5] = b'X';
        if mh.name[4] == 0x13 {
            mh.name[4] = b'X';
        }
    }
    if libxmp_test_name(&mh.name, 20, 0) < 0 {
        return -1;
    }

    for i in 0..15 {
        read_st_instrument(&mut mh.ins[i], f);
    }
    mh.len = hio_read8(f);
    mh.restart = hio_read8(f);
    hio_read(&mut mh.order, 1, 128, f);

    let mut pat = 0;
    let mut pat_short = 0;
    for i in 0..128 {
        if mh.order[i] > 0x7f {
            return -1;
        }
        if mh.order[i] as i32 > pat {
            pat = mh.order[i] as i32;
            if i < mh.len as usize {
                pat_short = pat;
            }
        }
    }
    pat += 1;
    pat_short += 1;

    if pat > 0x7f || mh.len == 0 || mh.len > 0x80 {
        return -1;
    }

    for i in 0..15 {
        let ins = &mut mh.ins[i];
        smp_size += 2 * ins.size as i32;

        /* pennylane.mod and heymusic-sssexremix.mod have unusual
         * values after the \0. */
        if i == 0 && (&ins.name[..9] == b"funbass\0\r" || &ins.name[..17] == b"st-69:baseline\0R\0")
        {
            test_flags |= TEST_NAME_IGNORE_AFTER_0;
        }

        /* Crepequs.mod has random values in first byte */
        ins.name[0] = b'X';

        if libxmp_test_name(&ins.name, 22, test_flags) < 0 {
            return -1;
        }

        if ins.volume as i32 > 0x40 {
            return -1;
        }

        if ins.finetune as i32 & 0xf0 != 0 {
            return -1;
        }

        if ins.size > 0x8000 {
            return -1;
        }

        /* This test is always false, disable it
         *
         * if ((mh.ins[i].loop_start >> 1) > 0x8000)
         *    return -1;
         */

        if ins.loop_size > 0x8000 {
            return -1;
        }

        /* This test fails in atmosfer.mod, disable it
         *
         * if (mh.ins[i].loop_size > 1 && mh.ins[i].loop_size > mh.ins[i].size)
         *    return -1;
         */

        /* Bad rip of fin-nv1.mod has this unused instrument. */
        if ins.size == 0 && ins.loop_start == 4462 && ins.loop_size == 2078 {
            continue;
        }

        if (ins.loop_start >> 1) > ins.size {
            return -1;
        }

        if ins.size != 0 && (ins.loop_start >> 1) == ins.size {
            return -1;
        }

        if ins.size == 0 && ins.loop_start > 0 {
            return -1;
        }
    }

    if smp_size < 8 {
        return -1;
    }

    /* If the file size is correct when counting only patterns prior to the
     * module length, use the shorter count. This quirk is found in some
     * ST modules, most of them authored by Jean Baudlot. See razor-1911.mod,
     * the Operation Wolf soundtrack, or the Bad Dudes soundtrack.
     */
    if size < st_expected_size(smp_size, pat) as i64
        && size == st_expected_size(smp_size, pat_short) as i64
    {
        pat = pat_short;
    }

    let mut pattern_errors = 0;
    let mut ins = 0;
    for _ in 0..pat {
        if hio_read(&mut pat_buf, 1, 1024, f) < 1024 {
            return -1;
        }
        for mod_event in pat_buf.chunks_exact(4) {
            let s = (mod_event[0] & 0xf0) as i32 | msn(mod_event[2]);

            if s > 15 {
                /* sample number > 15 */
                pattern_errors += 1;
                if pattern_errors > ST_MAX_PATTERN_ERRORS {
                    /* too many pattern errors; not ST */
                    return -1;
                }
            }

            if s > ins {
                /* find highest used sample */
                ins = s;
            }

            let p = 256 * lsn(mod_event[0]) + mod_event[1] as i32;

            if p == 0 {
                continue;
            }

            let mut k = 0;
            while PERIOD[k] >= 0 {
                if p == PERIOD[k] {
                    break;
                }
                k += 1;
            }
            if PERIOD[k] < 0 {
                pattern_errors += 1;
                if pattern_errors > ST_MAX_PATTERN_ERRORS {
                    /* too many pattern errors; not ST */
                    return -1;
                }
            }
        }
    }

    /* Check if file was cut before any unused samples */
    if size < st_expected_size(smp_size, pat) as i64 {
        let mut ss = 0;
        let mut i = 0;
        while i < 15 && (i as i32) < ins {
            ss += 2 * mh.ins[i].size as i32;
            i += 1;
        }

        let limit = st_expected_size(ss, pat) * ST_TRUNCATION_LIMIT / 100;
        if size < limit as i64 {
            return -1;
        }
    }

    hio_seek(f, start as i64, SEEK_SET);
    libxmp_read_title(f, t, 20);

    0
}

/// Translation of `st_load()`.
fn st_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut mh = StHeader::default();
    let mut pat_buf = [0u8; 1024];
    let mut ust = true;
    /* int lps_mult = m->fetch & XMP_CTL_FIXLOOP ? 1 : 2; */

    let mod_ = &mut m.mod_;
    mod_.chn = 4;
    mod_.ins = 15;
    mod_.smp = mod_.ins;

    let mut smp_size = 0;

    hio_read(&mut mh.name, 1, 20, f);
    for i in 0..15 {
        read_st_instrument(&mut mh.ins[i], f);
        smp_size += 2 * mh.ins[i].size as i32;
    }
    mh.len = hio_read8(f);
    mh.restart = hio_read8(f);
    hio_read(&mut mh.order, 1, 128, f);

    mod_.len = mh.len as i32;
    mod_.rst = mh.restart as i32;

    /* UST: The byte at module offset 471 is BPM, not the song restart
     *      The default for UST modules is 0x78 = 120 BPM = 48 Hz.
     */
    if mod_.rst < 0x40 {
        /* should be 0x20 */
        ust = false;
    }

    mod_.xxo[..128].copy_from_slice(&mh.order);

    let mut pat_short = 0;
    for i in 0..128 {
        if mod_.xxo[i] as i32 > mod_.pat {
            mod_.pat = mod_.xxo[i] as i32;
            if i < mh.len as usize {
                pat_short = mod_.pat;
            }
        }
    }
    mod_.pat += 1;
    pat_short += 1;

    /* If the file size is correct when counting only patterns prior to the
     * module length, use the shorter count. See test function for info.
     */
    let size = hio_size(f);
    if size < st_expected_size(smp_size, mod_.pat) as i64
        && size == st_expected_size(smp_size, pat_short) as i64
    {
        mod_.pat = pat_short;
    }

    for i in 0..mod_.ins as usize {
        /* UST: Volume word does not contain a "Finetuning" value in its
         * high-byte.
         */
        if mh.ins[i].finetune != 0 {
            ust = false;
        }

        /* if (mh.ins[i].size == 0 && mh.ins[i].loop_size == 1)
        nt = 1; */

        /* UST: Maximum sample length is 9999 bytes decimal, but 1387
         * words hexadecimal. Longest samples on original sample disk
         * ST-01 were 9900 bytes.
         */
        if mh.ins[i].size > 0x1387 || mh.ins[i].loop_start > 9999 || mh.ins[i].loop_size > 0x1387 {
            ust = false;
        }
    }

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    for i in 0..mod_.ins {
        let iu = i as usize;
        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        let ins = &mh.ins[iu];
        let xxs = &mut mod_.xxs[iu];
        xxs.len = 2 * ins.size as i32 - ins.loop_start as i32;
        xxs.lps = 0;
        xxs.lpe = xxs.lps + 2 * ins.loop_size as i32;
        xxs.flg = if ins.loop_size > 1 {
            XMP_SAMPLE_LOOP
        } else {
            0
        };
        let len = xxs.len;
        let xxi = &mut mod_.xxi[iu];
        let sub = &mut xxi.sub[0];
        sub.fin = ((ins.finetune as u8) << 4) as i8 as i32;
        sub.vol = ins.volume as i32;
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.sid = i;
        strncpy(&mut xxi.name, &ins.name, 22);

        if len > 0 {
            xxi.nsm = 1;
        }
    }

    mod_.trk = mod_.chn * mod_.pat;

    strncpy(&mut mod_.name, &mh.name, 20);

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Load and convert patterns */
    /* Also scan patterns for tracker detection */
    let mut fxused = 0;

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        if hio_read(&mut pat_buf, 1, 1024, f) < 1024 {
            return -1;
        }

        for (j, mod_event) in pat_buf.chunks_exact(4).enumerate() {
            let j = j as i32;
            let event = mod_.event_mut(i, j % 4, j / 4);
            libxmp_decode_protracker_event(event, mod_event);

            if event.fxt != 0 {
                fxused |= 1 << event.fxt;
            } else if event.fxp != 0 {
                fxused |= 1;
            }

            /* UST: Only effects 1 (arpeggio) and 2 (pitchbend) are
             * available.
             */
            if event.fxt != 0 && event.fxt != 1 && event.fxt != 2 {
                ust = false;
            }

            /* Karsten Obarski's sleepwalk uses arpeggio 30 and 40 */
            if event.fxt == 1 {
                /* unlikely arpeggio */
                if event.fxp == 0x00 {
                    ust = false;
                }
                /*if ((ev.fxp & 0x0f) == 0 || (ev.fxp & 0xf0) == 0)
                ust = 0; */
            }

            if event.fxt == 2 {
                /* bend up and down at same time? */
                if (event.fxp & 0x0f) != 0 && (event.fxp & 0xf0) != 0 {
                    ust = false;
                }
            }
        }
    }

    if fxused & !0x0006 != 0 {
        ust = false;
    }

    let modtype: &[u8] = if ust {
        b"Ultimate Soundtracker"
    } else if (fxused & !0xd007) == 0 {
        b"Soundtracker IX" /* or MasterSoundtracker? */
    } else if (fxused & !0xf807) == 0 {
        b"D.O.C Soundtracker 2.0"
    } else {
        b"unknown tracker 15 instrument"
    };

    snprintf_bytes(&mut mod_.type_, XMP_NAME_SIZE, modtype);

    m.quirk |= QUIRK_NOBPM;
    m.period_type = PERIOD_MODRNG;

    let mod_ = &mut m.mod_;
    /* Perform the necessary conversions for Ultimate Soundtracker */
    if ust {
        /* Fix restart & bpm */
        mod_.bpm = mod_.rst;
        mod_.rst = 0;

        /* Fix sample loops */
        for _i in 0..mod_.ins { /* FIXME */ }

        /* Fix effects (arpeggio and pitchbending) */
        for i in 0..mod_.pat {
            for j in 0..(64 * 4) {
                let event = mod_.event_mut(i, j % 4, j / 4);
                if event.fxt == 1 {
                    event.fxt = 0;
                } else if event.fxt == 2 && (event.fxp & 0xf0) == 0 {
                    event.fxt = 1;
                } else if event.fxt == 2 && (event.fxp & 0x0f) == 0 {
                    event.fxp >>= 4;
                }
            }
        }
    } else if mod_.rst >= mod_.len {
        mod_.rst = 0;
    }

    /* Load samples */

    for i in 0..m.mod_.ins as usize {
        if m.mod_.xxs[i].len == 0 {
            continue;
        }

        /* Skip transient part of sample.
         *
         * Dennis Lindroos <denafcm@gmail.com> reports: One main thing is
         * sample-looping which on all trackers up to Noisetracker 1 (i
         * think it was Mahoney who actually changed the loopstart to be
         * in WORDS) never play looped samples from the beginning, i.e.
         * only plays the looped part. This can be heard in old modules
         * especially with "analogstring", "strings2" or "strings3"
         * samples because these have "slow attack" that is not part of
         * the loop and thus they sound "sharper"..
         */
        hio_seek(f, mh.ins[i].loop_start as i64, SEEK_CUR);

        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[i], None) < 0 {
            return -1;
        }
    }

    0
}
