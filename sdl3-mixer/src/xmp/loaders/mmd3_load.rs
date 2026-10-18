// Rust translation of src/loaders/mmd3_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2025 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The OctaMED MMD2 and MMD3 loader.

use super::super::common::*;
use super::super::hio::*;
use super::super::med_extras::*;
use super::super::*;
use super::common::*;
use super::med::*;
use super::mmd1_load::{at, at_u, mmd_test};
use super::mmd_common::*;
use super::*;

pub(crate) const LIBXMP_LOADER_MMD3: FormatLoader = FormatLoader {
    name: "OctaMED",
    test: mmd3_test,
    loader: mmd3_load,
};

/// Translation of `mmd3_test()`.
fn mmd3_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, start: i32) -> i32 {
    mmd_test(f, t, start, &[b"MMD2", b"MMD3"])
}

/// Translation of `mmd3_load()`.
fn mmd3_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut header = Mmd0::default();
    let mut song = Mmd2Song::default();
    let mut block = Mmd1Block::default();
    let mut expdata = Mmd0Exp::default();

    hio_read(&mut header.id, 4, 1, f);

    let ver = header.id[3] as i8 as i32 - b'1' as i32 + 1;

    header.modlen = hio_read32b(f);
    let song_offset = hio_read32b(f) as i32;
    hio_read16b(f);
    hio_read16b(f);
    let blockarr_offset = hio_read32b(f) as i32;
    hio_read32b(f);
    let smplarr_offset = hio_read32b(f) as i32;
    hio_read32b(f);
    let expdata_offset = hio_read32b(f) as i32;
    hio_read32b(f);
    header.pstate = hio_read16b(f);
    header.pblock = hio_read16b(f);
    header.pline = hio_read16b(f);
    header.pseqnum = hio_read16b(f);
    header.actplayline = hio_read16b(f) as i16;
    header.counter = hio_read8(f);
    header.extra_songs = hio_read8(f);
    let _ = &header; /* (its fields aren't used further) */

    /*
     * song structure
     */
    hio_seek(f, at(start, song_offset), SEEK_SET);
    for i in 0..63 {
        let s = &mut song.sample[i];
        s.rep = hio_read16b(f);
        s.replen = hio_read16b(f);
        s.midich = hio_read8(f);
        s.midipreset = hio_read8(f);
        s.svol = hio_read8(f);
        s.strans = hio_read8s(f);
    }
    song.numblocks = hio_read16b(f);
    song.songlen = hio_read16b(f);
    let seqtable_offset = hio_read32b(f) as i32;
    hio_read32b(f);
    let trackvols_offset = hio_read32b(f) as i32;
    song.numtracks = hio_read16b(f);
    song.numpseqs = hio_read16b(f);
    let trackpans_offset = hio_read32b(f) as i32;
    song.flags3 = hio_read32b(f);
    song.voladj = hio_read16b(f);
    song.channels = hio_read16b(f);
    song.mix_echotype = hio_read8(f);
    song.mix_echodepth = hio_read8(f);
    song.mix_echolen = hio_read16b(f);
    song.mix_stereosep = hio_read8(f) as i8;

    hio_seek(f, 223, SEEK_CUR);

    song.deftempo = hio_read16b(f);
    song.playtransp = hio_read8(f) as i8;
    song.flags = hio_read8(f);
    song.flags2 = hio_read8(f);
    song.tempo2 = hio_read8(f);
    for _ in 0..16 {
        hio_read8(f); /* reserved */
    }
    song.mastervol = hio_read8(f);
    song.numsamples = hio_read8(f);

    /* Sanity check */
    if song.numsamples > 63 {
        return -1;
    }

    /*
     * read sequence
     */
    hio_seek(f, at(start, seqtable_offset), SEEK_SET);
    let playseq_offset = hio_read32b(f) as i32;
    hio_seek(f, at(start, playseq_offset), SEEK_SET);
    hio_seek(f, 32, SEEK_CUR); /* skip name */
    hio_read32b(f);
    hio_read32b(f);
    let mod_ = &mut m.mod_;
    mod_.len = hio_read16b(f) as i32;

    /* Sanity check */
    if mod_.len > 255 {
        return -1;
    }

    for i in 0..mod_.len as usize {
        mod_.xxo[i] = hio_read16b(f) as u8;
    }

    /*
     * convert header
     */
    m.c4rate = C4_NTSC_RATE;
    m.quirk |= QUIRK_RTONCE; /* FF1 */
    m.quirk |= if song.flags & FLAG_STSLIDE != 0 {
        0
    } else {
        QUIRK_VSALL | QUIRK_PBALL
    };
    let hexvol = song.flags & FLAG_VOLHEX != 0;
    let med_8ch = song.flags & FLAG_8CHANNEL != 0;
    let bpm_on = song.flags2 & FLAG2_BPM != 0;
    let bpmlen = 1 + (song.flags2 & FLAG2_BMASK) as i32;
    m.time_factor = MED_TIME_FACTOR;

    mmd_set_bpm(m, med_8ch, song.deftempo as i32, bpm_on, bpmlen);

    let mod_ = &mut m.mod_;
    mod_.spd = song.tempo2 as i32;
    mod_.pat = song.numblocks as i32;
    mod_.ins = song.numsamples as i32;
    mod_.rst = 0;
    mod_.chn = 0;
    mod_.name[0] = 0;

    /*
     * Read smplarr
     */
    let Ok(mut smplarr) = try_vec(mod_.ins as usize, 0u32) else {
        return -1;
    };
    if hio_seek(f, at(start, smplarr_offset), SEEK_SET) != 0 {
        return -1;
    }
    for s in smplarr.iter_mut() {
        *s = hio_read32b(f);
        if hio_eof(f) != 0 {
            return -1;
        }
    }

    /*
     * Obtain number of samples from each instrument
     */
    mod_.smp = 0;
    for &s in &smplarr {
        if s == 0 {
            continue;
        }
        hio_seek(f, at_u(start, s), SEEK_SET);
        hio_read32b(f); /* length */
        let type_ = hio_read16b(f) as i16;
        if type_ == -1 || type_ == -2 {
            /* type is synth? */
            hio_seek(f, 14, SEEK_CUR);
            mod_.smp += hio_read16b(f) as i32; /* wforms */
        } else if (1..=6).contains(&type_) {
            /* octave samples */
            mod_.smp += MMD_NUM_OCT[type_ as usize - 1];
        } else {
            mod_.smp += 1;
        }
        if hio_error(f) != 0 {
            return -1;
        }
    }

    /*
     * expdata
     */
    expdata.s_ext_entries = 0;
    expdata.s_ext_entrsz = 0;
    expdata.i_ext_entries = 0;
    expdata.i_ext_entrsz = 0;
    let mut expsmp_offset = 0;
    let mut iinfo_offset = 0;

    if expdata_offset != 0 {
        if hio_seek(f, at(start, expdata_offset), SEEK_SET) != 0 {
            return -1;
        }
        hio_read32b(f);
        expsmp_offset = hio_read32b(f) as i32;
        expdata.s_ext_entries = hio_read16b(f);
        expdata.s_ext_entrsz = hio_read16b(f);
        hio_read32b(f); /* annotxt */
        hio_read32b(f); /* annolen */
        iinfo_offset = hio_read32b(f) as i32;
        expdata.i_ext_entries = hio_read16b(f);
        expdata.i_ext_entrsz = hio_read16b(f);

        /* Sanity check */
        if expsmp_offset < 0 || iinfo_offset < 0 {
            return -1;
        }

        hio_read32b(f);
        hio_read32b(f);
        hio_read32b(f);
        hio_read32b(f);
        let songname_offset = hio_read32b(f) as i32;
        expdata.songnamelen = hio_read32b(f);
        hio_read32b(f); /* dumps */
        let mmdinfo_offset = hio_read32b(f) as i32;

        if hio_error(f) != 0 {
            return -1;
        }

        hio_seek(f, at(start, songname_offset), SEEK_SET);
        // Note (upstream): a song name of XMP_NAME_SIZE or more bytes
        // fills the name without its terminator, which C's string
        // functions then read past; here it is read within the name's
        // array.
        for i in 0..expdata.songnamelen as usize {
            if i >= XMP_NAME_SIZE {
                break;
            }
            m.mod_.name[i] = hio_read8(f);
        }

        if mmdinfo_offset != 0 {
            hio_seek(f, at(start, mmdinfo_offset), SEEK_SET);
            mmd_info_text(f, m, mmdinfo_offset);
        }
    }

    /*
     * Read blockarr.
     */
    let mod_ = &mut m.mod_;
    let Ok(mut blockarr) = try_vec(mod_.pat as usize, 0u32) else {
        return -1;
    };
    if hio_seek(f, at(start, blockarr_offset), SEEK_SET) != 0 {
        return -1;
    }
    for b in blockarr.iter_mut() {
        *b = hio_read32b(f);
        if hio_error(f) != 0 {
            return -1;
        }
    }

    /*
     * Quickly scan patterns to check the number of channels
     */

    let mut max_lines = 1;
    for &b in &blockarr {
        if b == 0 {
            continue;
        }

        hio_seek(f, at_u(start, b), SEEK_SET);

        block.numtracks = hio_read16b(f);
        block.lines = hio_read16b(f);
        if hio_error(f) != 0 {
            return -1;
        }

        /* Sanity check--Amiga OctaMED files have an upper bound of 3200 lines per block,
         * but MED Soundstudio for Windows allows up to 9999 lines.
         */
        if block.lines as i32 + 1 > 9999 {
            return -1;
        }

        if block.numtracks as i32 > mod_.chn {
            mod_.chn = block.numtracks as i32;
        }
        if block.lines as i32 + 1 > max_lines {
            max_lines = block.lines as i32 + 1;
        }
    }

    /* Sanity check */
    if mod_.chn <= 0 || mod_.chn > XMP_MAX_CHANNELS as i32 {
        return -1;
    }

    mod_.trk = mod_.pat * mod_.chn;

    let tracker_ver = mmd_tracker_version(
        m,
        ver,
        false,
        med_8ch,
        if expdata_offset != 0 {
            Some(&expdata)
        } else {
            None
        },
    );

    /*
     * Read and convert patterns
     */

    let mod_ = &mut m.mod_;
    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    let Ok(mut patbuf) = try_vec((mod_.chn * max_lines * 4) as usize, 0u8) else {
        return -1;
    };

    for i in 0..mod_.pat {
        let b = blockarr[i as usize];
        if b == 0 {
            continue;
        }

        hio_seek(f, at_u(start, b), SEEK_SET);

        block.numtracks = hio_read16b(f);
        block.lines = hio_read16b(f);
        hio_read32b(f); /* FIXME: should try to load extra command pages when they exist. */

        let size = block.numtracks as usize * (block.lines as usize + 1) * 4;
        /* (the scan above made room for the blocks) */
        if size > patbuf.len() {
            return -1;
        }
        if hio_read(&mut patbuf[..size], 1, size, f) < size {
            return -1;
        }

        if libxmp_alloc_pattern_tracks_long(mod_, i, block.lines as i32 + 1) < 0 {
            return -1;
        }

        let mut pos = 0;
        for j in 0..mod_.pattern(i).rows {
            for k in 0..block.numtracks as i32 {
                let p = &patbuf[pos..pos + 4];
                let event = mod_.event_mut(i, k, j);
                event.note = p[0] & 0x7f;
                if event.note != 0 {
                    event.note = event.note.wrapping_add((12 + song.playtransp as i32) as u8);
                }

                if event.note as usize >= XMP_MAX_KEYS {
                    event.note = 0;
                }

                event.ins = p[1] & 0x3f;

                event.fxt = p[2];
                event.fxp = p[3];
                mmd_xlat_fx(event, bpm_on, bpmlen, med_8ch, hexvol);
                pos += 4;
            }
        }
    }
    drop(patbuf);

    if libxmp_med_new_module_extras(m) != 0 {
        return -1;
    }

    if let Some(me) = med_module_extras_mut(m) {
        me.tracker_version = tracker_ver;
    }

    /*
     * Read and convert instruments and samples
     */

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    /* Instrument extras */
    let Ok(mut exp_smp) = try_vec(m.mod_.ins as usize, InstrExt::default()) else {
        return -1;
    };

    if expsmp_offset != 0 {
        if hio_seek(f, at(start, expsmp_offset), SEEK_SET) != 0 {
            return -1;
        }

        let mut i = 0;
        while i < m.mod_.ins as usize && i < expdata.s_ext_entries as usize {
            let mut skip = expdata.s_ext_entrsz as i32;
            let e = &mut exp_smp[i];

            if expdata.s_ext_entrsz >= 2 {
                /* MED 3.00 / OctaMED V1 */
                e.hold = hio_read8(f);
                e.decay = hio_read8(f);
                skip -= 2;
            }
            if expdata.s_ext_entrsz >= 4 {
                /* MED 3.20 / OctaMED V2 */
                e.suppress_midi_off = hio_read8(f);
                e.finetune = hio_read8(f) as i8;
                skip -= 2;
            }
            if expdata.s_ext_entrsz >= 8 {
                /* Octamed V5 */
                e.default_pitch = hio_read8(f);
                e.instr_flags = hio_read8(f);
                hio_read16b(f);
                skip -= 4;
            }
            if expdata.s_ext_entrsz >= 10 {
                /* OctaMED V5.02 */
                hio_read16b(f);
                skip -= 2;
            }
            if expdata.s_ext_entrsz >= 18 {
                /* OctaMED V7 */
                e.long_repeat = hio_read32b(f);
                e.long_replen = hio_read32b(f);
                skip -= 8;
            }

            if hio_error(f) != 0 {
                return -1;
            }

            if skip != 0 && hio_seek(f, skip as i64, SEEK_CUR) != 0 {
                return -1;
            }
            i += 1;
        }
    }

    /* Instrument names */
    if iinfo_offset != 0 {
        let mut name = [0u8; 40];

        if hio_seek(f, at(start, iinfo_offset), SEEK_SET) != 0 {
            return -1;
        }

        let mut i = 0;
        while i < m.mod_.ins && i < expdata.i_ext_entries as i32 {
            let skip = expdata.i_ext_entrsz as i32 - 40;

            if hio_read(&mut name, 40, 1, f) < 1 {
                return -1;
            }
            libxmp_instrument_name(&mut m.mod_, i, &name, 40);

            if skip != 0 && hio_seek(f, skip as i64, SEEK_CUR) != 0 {
                return -1;
            }
            i += 1;
        }
    }

    /* Sample data */
    let mut smp_idx = 0;
    for i in 0..m.mod_.ins {
        let iu = i as usize;
        if smplarr[iu] == 0 {
            continue;
        }

        if hio_seek(f, at_u(start, smplarr[iu]), SEEK_SET) < 0 {
            return -1;
        }

        smp_idx = mmd_load_instrument(
            f,
            m,
            i,
            smp_idx,
            &expdata,
            &exp_smp[iu],
            &song.sample[iu],
            ver,
        );

        if smp_idx < 0 {
            return -1;
        }
    }

    let mod_ = &mut m.mod_;
    hio_seek(f, at(start, trackvols_offset), SEEK_SET);
    for i in 0..mod_.chn as usize {
        mod_.xxc[i].vol = hio_read8(f) as i32;
    }

    if trackpans_offset != 0 {
        hio_seek(f, at(start, trackpans_offset), SEEK_SET);
        for i in 0..mod_.chn as usize {
            let p = 8 * hio_read8s(f) as i32;
            mod_.xxc[i].pan = 0x80 + if p > 127 { 127 } else { p };
        }
    } else {
        for i in 0..mod_.chn as usize {
            mod_.xxc[i].pan = 0x80;
        }
    }

    m.read_event_type = READ_EVENT_MED;
    0
}
