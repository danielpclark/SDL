// Rust translation of src/loaders/mgt_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Megatracker (MGT) loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_c2spd_to_note;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

const MAGIC_MGT: u32 = magic4(0x00, b'M', b'G', b'T');
const MAGIC_MCS: u32 = magic4(0xbd, b'M', b'C', b'S');

pub(crate) const LIBXMP_LOADER_MGT: FormatLoader = FormatLoader {
    name: "Megatracker",
    test: mgt_test,
    loader: mgt_load,
};

/// Translation of `mgt_test()`.
fn mgt_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, start: i32) -> i32 {
    if hio_read24b(f) != MAGIC_MGT {
        return -1;
    }
    hio_read8(f);
    if hio_read32b(f) != MAGIC_MCS {
        return -1;
    }

    hio_seek(f, 18, SEEK_CUR);
    let sng_ptr = hio_read32b(f) as i32;
    hio_seek(f, start.wrapping_add(sng_ptr) as i64, SEEK_SET);

    libxmp_read_title(f, t, 32);

    0
}

/// Translation of `mgt_load()`.
fn mgt_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut sdata = [0i32; 64];
    let at = |p: i32| start.wrapping_add(p) as i64;

    hio_read24b(f); /* MGT */
    let ver = hio_read8(f);
    hio_read32b(f); /* MCS */

    let s = format!("Megatracker MGT v{}.{}", msn(ver), lsn(ver));
    libxmp_set_type(m, s.as_bytes());

    let mod_ = &mut m.mod_;
    mod_.chn = hio_read16b(f) as i32;
    hio_read16b(f); /* number of songs */
    mod_.len = hio_read16b(f) as i32;
    mod_.pat = hio_read16b(f) as i32;
    mod_.trk = hio_read16b(f) as i32;
    mod_.ins = hio_read16b(f) as i32;
    mod_.smp = mod_.ins;
    hio_read16b(f); /* reserved */
    hio_read32b(f); /* reserved */

    /* Sanity check */
    if mod_.chn > XMP_MAX_CHANNELS as i32 || mod_.pat > MAX_PATTERNS || mod_.ins > 64 {
        return -1;
    }

    let sng_ptr = hio_read32b(f) as i32;
    hio_read32b(f); /* seq_ptr (read again from the song) */
    let ins_ptr = hio_read32b(f) as i32;
    let pat_ptr = hio_read32b(f) as i32;
    let trk_ptr = hio_read32b(f) as i32;
    hio_read32b(f); /* sample offset */
    hio_read32b(f); /* total smp len */
    hio_read32b(f); /* unpacked trk size */

    hio_seek(f, at(sng_ptr), SEEK_SET);

    hio_read(&mut mod_.name, 1, 32, f);
    let seq_ptr = hio_read32b(f) as i32;
    mod_.len = hio_read16b(f) as i32;
    mod_.rst = hio_read16b(f) as i32;
    mod_.bpm = hio_read8(f) as i32;
    mod_.spd = hio_read8(f) as i32;
    hio_read16b(f); /* global volume */
    hio_read8(f); /* master L */
    hio_read8(f); /* master R */

    /* Sanity check */
    if mod_.len > XMP_MAX_MOD_LENGTH as i32 || mod_.rst > 255 {
        return -1;
    }

    for _ in 0..mod_.chn {
        hio_read16b(f); /* pan */
    }

    m.c4rate = C4_NTSC_RATE;

    /* Sequence */

    let mod_ = &mut m.mod_;
    hio_seek(f, at(seq_ptr), SEEK_SET);
    for i in 0..mod_.len as usize {
        let pos = hio_read16b(f) as i32;

        /* Sanity check */
        if pos >= mod_.pat {
            return -1;
        }
        mod_.xxo[i] = pos as u8;
    }

    /* Instruments */

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    hio_seek(f, at(ins_ptr), SEEK_SET);

    let mod_ = &mut m.mod_;
    for i in 0..mod_.ins {
        let iu = i as usize;

        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        // Note (upstream): a name of 32 bytes fills the name without its
        // terminator, which C's string functions then read past; here it
        // is read within the name's array.
        hio_read(&mut mod_.xxi[iu].name, 1, 32, f);
        sdata[iu] = hio_read32b(f) as i32;
        let xxs = &mut mod_.xxs[iu];
        xxs.len = hio_read32b(f) as i32;

        /* Sanity check */
        if xxs.len > MAX_SAMPLE_SIZE {
            return -1;
        }

        xxs.lps = hio_read32b(f) as i32;
        xxs.lpe = (xxs.lps as u32).wrapping_add(hio_read32b(f)) as i32;
        hio_read32b(f);
        hio_read32b(f);
        let c2spd = hio_read32b(f) as i32;
        let sub = &mut mod_.xxi[iu].sub[0];
        libxmp_c2spd_to_note(c2spd, &mut sub.xpo, &mut sub.fin);
        sub.vol = (hio_read16b(f) >> 4) as i32;
        hio_read8(f); /* vol L */
        hio_read8(f); /* vol R */
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        let flags = hio_read8(f);
        let xxs = &mut mod_.xxs[iu];
        xxs.flg = if flags & 0x03 != 0 {
            XMP_SAMPLE_LOOP
        } else {
            0
        };
        xxs.flg |= if flags & 0x02 != 0 {
            XMP_SAMPLE_LOOP_BIDIR
        } else {
            0
        };
        mod_.xxi[iu].sub[0].fin += 0 * hio_read8(f) as i32; // FIXME
        hio_read8(f); /* unused */
        hio_read8(f);
        hio_read8(f);
        hio_read8(f);
        hio_read16b(f);
        hio_read32b(f);
        hio_read32b(f);

        let len = mod_.xxs[iu].len;
        mod_.xxi[iu].nsm = (len != 0) as i32;
        mod_.xxi[iu].sub[0].sid = i;
    }

    /* PATTERN_INIT - alloc extra track*/
    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Tracks */

    /* Sanity check */
    if trk_ptr as i64 >= hio_size(f) {
        return -1;
    }

    for i in 1..mod_.trk {
        hio_seek(f, at(trk_ptr.wrapping_add(i * 4)), SEEK_SET);
        let offset = hio_read32b(f) as i32;
        hio_seek(f, at(offset), SEEK_SET);

        let rows = hio_read16b(f) as i32;

        /* Sanity check */
        if rows > 255 {
            return -1;
        }

        if libxmp_alloc_track(mod_, i, rows) < 0 {
            return -1;
        }

        let mut j = 0;
        while j < rows {
            /* TODO libxmp can't really support the wide effect
             * params Megatracker uses right now, but less bad
             * conversions of certain effects could be attempted. */
            /* uint8 f2p ;*/

            let b = hio_read8(f);
            j += (b & 0x03) as i32;

            /* Sanity check */
            if j >= rows {
                return -1;
            }

            let mut note = 0u8;
            let event = &mut mod_.track_mut(i).event[j as usize];
            if b & 0x04 != 0 {
                note = hio_read8(f);
            }
            if b & 0x08 != 0 {
                event.ins = hio_read8(f);
            }
            if b & 0x10 != 0 {
                event.vol = hio_read8(f);
            }
            if b & 0x20 != 0 {
                event.fxt = hio_read8(f);
            }
            if b & 0x40 != 0 {
                event.fxp = hio_read8(f);
            }
            if b & 0x80 != 0 {
                /*f2p =*/
                hio_read8(f);
            }

            if note == 1 {
                event.note = XMP_KEY_OFF;
            } else if note > 11 {
                /* adjusted to play codeine.mgt */
                event.note = note.wrapping_add(1);
            }

            /* effects */
            if event.fxt < 0x10 {
                /* like amiga */
            } else {
                match event.fxt {
                    0x13 | 0x14 | 0x15 | 0x17 | 0x1c | 0x1d | 0x1e => {
                        // Note (upstream): C takes the extended effect's
                        // number from the new effect type (making them all
                        // EEx, pattern delays), not the old one.
                        event.fxp = ((event.fxt & 0x0f) << 4) | (event.fxp & 0x0f);
                        event.fxt = FX_EXTENDED;
                    }
                    _ => {
                        event.fxt = 0;
                        event.fxp = 0;
                    }
                }
            }

            /* volume and volume column effects */
            if event.vol >= 0x10 && event.vol <= 0x50 {
                event.vol -= 0x0f;
                j += 1;
                continue;
            }

            match event.vol >> 4 {
                0x06 => {
                    /* Volume slide down */
                    event.f2t = FX_VOLSLIDE_2;
                    event.f2p = event.vol - 0x60;
                }
                0x07 => {
                    /* Volume slide up */
                    event.f2t = FX_VOLSLIDE_2;
                    event.f2p = (event.vol - 0x70) << 4;
                }
                0x08 => {
                    /* Fine volume slide down */
                    event.f2t = FX_EXTENDED;
                    event.f2p = ((EX_F_VSLIDE_DN << 4) as u8) | (event.vol - 0x80);
                }
                0x09 => {
                    /* Fine volume slide up */
                    event.f2t = FX_EXTENDED;
                    event.f2p = ((EX_F_VSLIDE_UP << 4) as u8) | (event.vol - 0x90);
                }
                0x0a => {
                    /* Set vibrato speed */
                    event.f2t = FX_VIBRATO;
                    event.f2p = (event.vol - 0xa0) << 4;
                }
                0x0b => {
                    /* Vibrato */
                    event.f2t = FX_VIBRATO;
                    event.f2p = event.vol - 0xb0;
                }
                0x0c => {
                    /* Set panning */
                    event.f2t = FX_SETPAN;
                    event.f2p = ((event.vol - 0xc0) << 4) + 8;
                }
                0x0d => {
                    /* Pan slide left */
                    event.f2t = FX_PANSLIDE;
                    event.f2p = (event.vol - 0xd0) << 4;
                }
                0x0e => {
                    /* Pan slide right */
                    event.f2t = FX_PANSLIDE;
                    event.f2p = event.vol - 0xe0;
                }
                0x0f => {
                    /* Tone portamento */
                    event.f2t = FX_TONEPORTA;
                    event.f2p = (event.vol - 0xf0) << 4;
                }
                _ => {}
            }

            event.vol = 0;
            j += 1;
        }
    }

    /* Extra track */
    if mod_.trk > 0 {
        if let Some(xxt) = mod_.xxt.as_mut() {
            xxt[0] = Some(Box::new(XmpTrack {
                rows: 64,
                event: vec![XmpEvent::default(); 64],
            }));
        }
    }

    /* Read and convert patterns */

    hio_seek(f, at(pat_ptr), SEEK_SET);

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern(mod_, i) < 0 {
            return -1;
        }

        let rows = hio_read16b(f) as i32;

        /* Sanity check */
        if rows > 256 {
            return -1;
        }

        mod_.pattern_mut(i).rows = rows;

        for j in 0..mod_.chn as usize {
            let track = hio_read16b(f) as i32 - 1;

            /* Sanity check */
            if track >= mod_.trk {
                return -1;
            }

            mod_.pattern_mut(i).index[j] = track;
        }
    }

    /* Read samples */

    for i in 0..m.mod_.ins as usize {
        if m.mod_.xxi[i].nsm == 0 {
            continue;
        }

        hio_seek(f, at(sdata[i]), SEEK_SET);
        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[i], None) < 0 {
            return -1;
        }
    }

    0
}
