// Rust translation of src/loaders/mtm_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The MultiTracker (MTM) loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

/// Translation of `struct mtm_file_header`.
#[derive(Clone, Copy, Default)]
struct MtmFileHeader {
    magic: [u8; 3], /* "MTM" */
    version: u8,    /* MSN=major, LSN=minor */
    name: [u8; 20], /* ASCIIZ Module name */
    tracks: u16,    /* Number of tracks saved */
    patterns: u8,   /* Number of patterns saved */
    modlen: u8,     /* Module length */
    extralen: u16,  /* Length of the comment field */
    samples: u8,    /* Number of samples */
    attr: u8,       /* Always zero */
    rows: u8,       /* Number rows per track */
    channels: u8,   /* Number of tracks per pattern */
    pan: [u8; 32],  /* Pan positions for each channel */
}

/// Translation of `struct mtm_instrument_header`.
#[derive(Clone, Copy, Default)]
struct MtmInstrumentHeader {
    name: [u8; 22],  /* Instrument name */
    length: u32,     /* Instrument length in bytes */
    loop_start: u32, /* Sample loop start */
    loopend: u32,    /* Sample loop end */
    finetune: u8,    /* Finetune */
    volume: u8,      /* Playback volume */
    attr: u8,        /* &0x01: 16bit sample */
}

pub(crate) const LIBXMP_LOADER_MTM: FormatLoader = FormatLoader {
    name: "Multitracker",
    test: mtm_test,
    loader: mtm_load,
};

/// Translation of `mtm_test()`.
fn mtm_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    let mut buf = [0u8; 4];

    if hio_read(&mut buf, 1, 4, f) < 4 {
        return -1;
    }
    if &buf[..3] != b"MTM" {
        return -1;
    }
    if buf[3] != 0x10 {
        return -1;
    }

    libxmp_read_title(f, t, 20);

    0
}

/// Translation of `mtm_load()`.
fn mtm_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut mfh = MtmFileHeader::default();
    let mut mih = MtmInstrumentHeader::default();
    let mut mt = [0u8; 192];
    let mut fxx = [false; 2];

    hio_read(&mut mfh.magic, 3, 1, f); /* "MTM" */
    mfh.version = hio_read8(f); /* MSN=major, LSN=minor */
    hio_read(&mut mfh.name, 20, 1, f); /* ASCIIZ Module name */
    mfh.tracks = hio_read16l(f); /* Number of tracks saved */
    mfh.patterns = hio_read8(f); /* Number of patterns saved */
    mfh.modlen = hio_read8(f); /* Module length */
    mfh.extralen = hio_read16l(f); /* Length of the comment field */

    mfh.samples = hio_read8(f); /* Number of samples */
    if mfh.samples > 63 {
        return -1;
    }

    mfh.attr = hio_read8(f); /* Always zero */

    mfh.rows = hio_read8(f); /* Number rows per track */
    if mfh.rows != 64 {
        return -1;
    }

    mfh.channels = hio_read8(f); /* Number of tracks per pattern */
    if mfh.channels as usize > 32.min(XMP_MAX_CHANNELS) {
        return -1;
    }

    hio_read(&mut mfh.pan, 32, 1, f); /* Pan positions for each channel */

    if hio_error(f) != 0 {
        return -1;
    }

    /* (upstream keeps a check of the magic here under #if 0) */

    let mod_ = &mut m.mod_;
    mod_.trk = mfh.tracks as i32 + 1;
    mod_.pat = mfh.patterns as i32 + 1;
    mod_.len = mfh.modlen as i32 + 1;
    mod_.ins = mfh.samples as i32;
    mod_.smp = mod_.ins;
    mod_.chn = mfh.channels as i32;
    mod_.spd = 6;
    mod_.bpm = 125;

    strncpy(&mut mod_.name, &mfh.name, 20);
    let s = format!(
        "MultiTracker {}.{:02} MTM",
        msn(mfh.version),
        lsn(mfh.version)
    );
    libxmp_set_type(m, s.as_bytes());

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    /* Read and convert instruments */
    for i in 0..mod_.ins {
        let iu = i as usize;
        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        hio_read(&mut mih.name, 22, 1, f); /* Instrument name */
        mih.length = hio_read32l(f); /* Instrument length in bytes */

        if mih.length > MAX_SAMPLE_SIZE as u32 {
            return -1;
        }

        mih.loop_start = hio_read32l(f); /* Sample loop start */
        mih.loopend = hio_read32l(f); /* Sample loop end */
        mih.finetune = hio_read8(f); /* Finetune */
        mih.volume = hio_read8(f); /* Playback volume */
        mih.attr = hio_read8(f); /* &0x01: 16bit sample */

        let xxs = &mut mod_.xxs[iu];
        xxs.len = mih.length as i32;
        xxs.lps = mih.loop_start as i32;
        xxs.lpe = mih.loopend as i32;
        xxs.flg = if xxs.lpe > 2 { XMP_SAMPLE_LOOP } else { 0 }; /* 1 == Forward loop */
        if mih.attr & 1 != 0 {
            xxs.flg |= XMP_SAMPLE_16BIT;
            xxs.len >>= 1;
            xxs.lps >>= 1;
            xxs.lpe >>= 1;
        }
        let len = xxs.len;

        let sub = &mut mod_.xxi[iu].sub[0];
        sub.vol = mih.volume as i32;
        sub.fin = (mih.finetune << 4) as i8 as i32;
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.sid = i;

        libxmp_instrument_name(mod_, i, &mih.name, 22);

        if len > 0 {
            mod_.xxi[iu].nsm = 1;
        }
    }

    hio_read(&mut mod_.xxo, 1, 128, f);

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    for i in 0..mod_.trk {
        if libxmp_alloc_track(mod_, i, mfh.rows as i32) < 0 {
            return -1;
        }

        if i == 0 {
            continue;
        }

        if hio_read(&mut mt, 3, 64, f) != 64 {
            return -1;
        }

        for j in 0..64 {
            let e = &mut mod_.track_mut(i).event[j];
            let d = &mt[j * 3..];

            e.note = d[0] >> 2;
            if e.note != 0 {
                e.note += 37;
            }
            e.ins = ((d[0] & 0x3) << 4) + msn(d[1]) as u8;
            e.fxt = lsn(d[1]) as u8;
            e.fxp = d[2];
            if e.fxt > FX_SPEED {
                e.fxt = 0;
                e.fxp = 0;
            }

            /* Break is hex (Sybaris/odyssey.mtm pos 14, 30). */
            if e.fxt == FX_BREAK {
                e.fxt = FX_IT_BREAK;
            }

            /* See tempo mode detection below. */
            if e.fxt == FX_SPEED {
                fxx[(e.fxp >= 0x20) as usize] = true;
            }

            /* Set pan effect translation */
            if e.fxt == FX_EXTENDED && msn(e.fxp) == 0x8 {
                e.fxt = FX_SETPAN;
                e.fxp <<= 4;
            }
        }
    }

    /* Read patterns */

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern(mod_, i) < 0 {
            return -1;
        }

        mod_.pattern_mut(i).rows = 64;
        for j in 0..32 {
            let mut track = hio_read16l(f) as i32;

            if track >= mod_.trk {
                track = 0;
            }

            if j < mod_.chn {
                mod_.pattern_mut(i).index[j as usize] = track;
            }
        }
    }

    /* Tempo mode detection.
     *
     * The MTM tempo effect has an unusual property: when speed is set, the
     * tempo resets to 125, and when tempo is set, the speed resets to 6.
     * Modules that use both speed and tempo effects need to emulate this.
     * See: Absolve the Ambience by Sybaris, Soma by Ranger Rick.
     *
     * Dual Module Player and other DOS players did not know about this and
     * did not implement support for it, and instead used Protracker Fxx.
     * Many MTM authors created modules that rely on this which are various
     * degrees of broken in the tracker they were made with! Several MTMs
     * by Phoenix and Silent Mode expect this. The majority of them can be
     * detected by checking for high Fxx and low Fxx on the same row.
     */
    'probably_dmp: {
        if fxx[0] && fxx[1] {
            /* Both used, check patterns. */
            for i in 0..mod_.pat {
                for j in 0..mfh.rows as i32 {
                    fxx = [false; 2];
                    for k in 0..mod_.chn {
                        let e = mod_.event(i, k, j);
                        if e.fxt == FX_SPEED {
                            fxx[(e.fxp >= 0x20) as usize] = true;
                        }
                    }
                    if fxx[0] && fxx[1] {
                        /* Same row, no change required */
                        break 'probably_dmp;
                    }
                }
            }
            /* probably MT; injecting speed/BPM reset effects */
            for i in 0..mod_.pat {
                for j in 0..mfh.rows as i32 {
                    for k in 0..mod_.chn {
                        let e = mod_.event_mut(i, k, j);
                        if e.fxt == FX_SPEED {
                            e.f2t = FX_SPEED;
                            e.f2p = if e.fxp < 0x20 { 125 } else { 6 };
                        }
                    }
                }
            }
        }
    }

    /* Comments */
    if mfh.extralen != 0 {
        if let Ok(mut comment) = try_vec(mfh.extralen as usize + 1, 0u8) {
            /* Comments are stored in 40 byte ASCIIZ lines. */
            let len = hio_read(&mut comment, 1, mfh.extralen as usize, f);
            let mut last_line = 0;

            let mut i = 0;
            while i + 40 <= len {
                if comment[i] != b'\0' {
                    last_line = i + 40;
                }
                i += 40;
            }
            let mut j = 0;
            let mut line = 0;
            while line < last_line {
                for i in 0..39 {
                    let c = comment[line + i];
                    if c == b'\0' {
                        break;
                    }
                    comment[j] = c;
                    j += 1;
                }
                comment[j] = b'\n';
                j += 1;
                line += 40;
            }
            comment[j] = b'\0';
            comment.truncate(j + 1);
            m.comment = Some(comment);
        } else {
            hio_seek(f, mfh.extralen as i64, SEEK_CUR);
        }
    }

    /* Read samples */

    for i in 0..m.mod_.ins as usize {
        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), SAMPLE_FLAG_UNS, &mut m.mod_.xxs[i], None) < 0 {
            return -1;
        }
    }

    let mod_ = &mut m.mod_;
    for i in 0..mod_.chn as usize {
        mod_.xxc[i].pan = (mfh.pan[i] as i32) << 4;
    }

    0
}
