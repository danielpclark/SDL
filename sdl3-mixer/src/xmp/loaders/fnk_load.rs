// Rust translation of src/loaders/fnk_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2025 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Funktracker loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

const MAGIC_FUNK: u32 = magic4(b'F', b'u', b'n', b'k');

pub(crate) const LIBXMP_LOADER_FNK: FormatLoader = FormatLoader {
    name: "Funktracker",
    test: fnk_test,
    loader: fnk_load,
};

/// Translation of `fnk_test()`.
fn fnk_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    if hio_read32b(f) != MAGIC_FUNK {
        return -1;
    }

    hio_read8(f);
    let a = hio_read8(f);
    let b = hio_read8(f);
    hio_read8(f);

    if (a >> 1) < 10 {
        /* creation year (-1980) */
        return -1;
    }

    if msn(b) > 7 || lsn(b) > 9 {
        /* CPU and card */
        return -1;
    }

    let size = hio_read32l(f) as i32;
    if size < 1024 {
        return -1;
    }

    if hio_size(f) != size as i64 {
        return -1;
    }

    libxmp_read_title(f, t, 0);

    0
}

/// Translation of `struct fnk_instrument`.
#[derive(Clone, Copy, Default)]
struct FnkInstrument {
    name: [u8; 19],  /* ASCIIZ instrument name */
    loop_start: u32, /* Instrument loop start */
    length: u32,     /* Instrument length */
    volume: u8,      /* Volume (0-255) */
    pan: u8,         /* Pan (0-255) */
    shifter: u8,     /* Portamento and offset shift */
    waveform: u8,    /* Vibrato and tremolo waveforms */
    retrig: u8,      /* Retrig and arpeggio speed */
}

/// Translation of `struct fnk_header`.
struct FnkHeader {
    marker: [u8; 4],          /* 'Funk' */
    info: [u8; 4],            /* */
    filesize: u32,            /* File size */
    fmt: [u8; 4],             /* F2xx, Fkxx or Fvxx */
    loop_: u8,                /* Loop order number */
    order: [u8; 256],         /* Order list */
    pbrk: [u8; 128],          /* Break list for patterns */
    fih: [FnkInstrument; 64], /* Instruments */
}

/// Translation of `fnk_translate_event()`.
fn fnk_translate_event(event: &mut XmpEvent, ev: &[u8; 3], _ffh: &FnkHeader) {
    let note = (ev[0] >> 2) as i32;

    /* 0x3f - "null slot": load command only */
    /* 0x3e - "sample-only slot": load instrument and command only */
    /* 0x3d - "reload sample attrs": load instrument and command only,
     *        TODO: naming seems completely backwards--this event is supposed
     *        to NOT load most sample fields like volume/pan from the new ins
     *        (fnk_smpl_setpan.fnk row 28 plays wrong)?
     */
    if note < 0x3d {
        event.note = (37 + note) as u8;
    }
    if note < 0x3f {
        event.ins = (1 + msn(ev[1]) + (((ev[0] & 0x03) as i32) << 4)) as u8;
    }

    match lsn(ev[1]) {
        0x00 => {
            event.fxt = FX_PER_PORTA_UP;
            event.fxp = ev[2];
        }
        0x01 => {
            event.fxt = FX_PER_PORTA_DN;
            event.fxp = ev[2];
        }
        0x02 => {
            event.fxt = FX_PER_TPORTA;
            event.fxp = ev[2];
        }
        0x03 => {
            event.fxt = FX_PER_VIBRATO;
            event.fxp = ev[2];
        }
        /* TODO: 0x04 Vibrato Fanin */
        /* TODO: 0x05 Vibrato Fanout */
        0x06 => {
            /* TODO: this effect is implemented completely wrong -
             * supposed to be gXY -> every (X+1) ticks, increase by table[Y] */
            event.fxt = FX_PER_VSLD_UP;
            event.fxp = ev[2] << 1;
        }
        0x07 => {
            /* TODO: this effect is implemented completely wrong -
             * supposed to be hXY -> every (X+1) ticks, decrease by table[Y] */
            event.fxt = FX_PER_VSLD_DN;
            event.fxp = ev[2] << 1;
        }
        /* TODO: 0x08 volume porta */
        /* TODO: 0x09 decaying reverb */
        /* TODO: 0x0a tremolo */
        0x0b => {
            event.fxt = FX_ARPEGGIO;
            event.fxp = ev[2];
        }
        /* TODO: 0x0c sample offset */
        0x0d => {
            event.fxt = FX_VOLSET;
            event.fxp = ev[2];
        }
        0x0e => {
            if ev[2] == 0x0a || ev[2] == 0x0b || ev[2] == 0x0c {
                event.fxt = FX_PER_CANCEL;
                return;
            }

            match msn(ev[2]) {
                /* TODO: 0x0 Misc Control */
                0x1 => {
                    event.fxt = FX_EXTENDED;
                    event.fxp = ((EX_CUT << 4) | lsn(ev[2])) as u8;
                }
                /* TODO: 0x2 Real Frequency Adjust ????? */
                0x2 => {
                    event.fxt = FX_EXTENDED;
                    event.fxp = ((EX_DELAY << 4) | lsn(ev[2])) as u8;
                }
                /* TODO: 0x3 Set Arpeggio Speed */
                /* TODO: 0x4 Fine Port Up */
                /* TODO: 0x5 Fine Port Down */
                /* TODO: 0x6 Fine Volume Slide Up */
                /* TODO: 0x7 Fine Volume Slide Down */
                /* TODO: 0x8 Volume Crest */
                /* TODO: 0x9 Volume Trough */
                /* TODO: 0xa Set Master Volume */
                /* TODO: 0xb Expand Loop */
                /* TODO: 0xc Collapse Loop */
                0xd => {
                    event.fxt = FX_EXTENDED;
                    event.fxp = ((EX_RETRIG << 4) | lsn(ev[2])) as u8;
                }
                0xe => {
                    /* TODO: This doesn't work on a line with note 0x3f? */
                    event.fxt = FX_SETPAN;
                    event.fxp = (8 + (lsn(ev[2]) << 4)) as u8;
                }
                0xf => {
                    event.fxt = FX_SPEED;
                    event.fxp = lsn(ev[2]) as u8;
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// Translation of `fnk_load()`.
fn fnk_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    /* int day, month, year; */
    let mut ffh = FnkHeader {
        marker: [0; 4],
        info: [0; 4],
        filesize: 0,
        fmt: [0; 4],
        loop_: 0,
        order: [0; 256],
        pbrk: [0; 128],
        fih: [FnkInstrument::default(); 64],
    };
    let mut ev = [0u8; 3];
    let mut allow_panning = true;

    hio_read(&mut ffh.marker, 4, 1, f);
    hio_read(&mut ffh.info, 4, 1, f);
    ffh.filesize = hio_read32l(f);
    hio_read(&mut ffh.fmt, 4, 1, f);
    ffh.loop_ = hio_read8(f);
    hio_read(&mut ffh.order, 256, 1, f);
    hio_read(&mut ffh.pbrk, 128, 1, f);

    for i in 0..128 {
        if ffh.pbrk[i] >= 64 {
            return -1;
        }
    }

    for i in 0..64 {
        let fih = &mut ffh.fih[i];
        hio_read(&mut fih.name, 19, 1, f);
        fih.loop_start = hio_read32l(f);
        fih.length = hio_read32l(f);
        fih.volume = hio_read8(f);
        fih.pan = hio_read8(f);
        fih.shifter = hio_read8(f);
        fih.waveform = hio_read8(f);
        fih.retrig = hio_read8(f);
        /* Sanity check */
        if fih.length >= ffh.filesize {
            return -1;
        }
    }

    /* day = ffh.info[0] & 0x1f;
    month = ((ffh.info[1] & 0x01) << 3) | ((ffh.info[0] & 0xe0) >> 5);
    year = 1980 + ((ffh.info[1] & 0xfe) >> 1); */

    let mod_ = &mut m.mod_;
    mod_.ins = 64;
    mod_.smp = mod_.ins;

    let mut i = 0;
    while i < 256 && ffh.order[i] != 0xff {
        if ffh.order[i] as i32 > mod_.pat {
            mod_.pat = ffh.order[i] as i32;
        }
        i += 1;
    }
    mod_.pat += 1;

    /* Sanity check */
    if mod_.pat > 128 {
        return -1;
    }

    mod_.len = i as i32;
    mod_.xxo[..i].copy_from_slice(&ffh.order[..i]);

    mod_.spd = 4;
    mod_.bpm = 125;
    mod_.chn = 0;

    /*
     * If an R1 fmt (funktype = Fk** or Fv**), then ignore byte 3. It's
     * unreliable. It used to store the (GUS) sample memory requirement.
     */
    if ffh.fmt[0] == b'F' && ffh.fmt[1] == b'2' {
        /* TODO: 16-bit sample precision (bit 0) */
        if ((ffh.info[3] as i8) >> 1) & 0x40 != 0 {
            mod_.bpm -= ((ffh.info[3] >> 1) & 0x3f) as i32;
        } else {
            mod_.bpm += ((ffh.info[3] >> 1) & 0x3f) as i32;
        }

        libxmp_set_type(m, b"FunktrackerGOLD");
    } else if ffh.fmt[0] == b'F' && (ffh.fmt[1] == b'v' || ffh.fmt[1] == b'k') {
        /* "Fk**" for fixed channeling or
         * "Fv**" for variable channeling
         *
         * Translated: Fk** has fixed 669 panning, Fv** has GUS panning.
         */
        libxmp_set_type(m, b"Funktracker");
        if ffh.fmt[1] == b'k' {
            allow_panning = false;
        }
    } else {
        mod_.chn = 8;
        allow_panning = false;
        libxmp_set_type(m, b"Funktracker DOS32");
    }

    let mod_ = &mut m.mod_;
    if mod_.chn == 0 {
        mod_.chn = if !ffh.fmt[2].is_ascii_digit() || !ffh.fmt[3].is_ascii_digit() {
            8
        } else {
            (ffh.fmt[2] - b'0') as i32 * 10 + (ffh.fmt[3] - b'0') as i32
        };

        /* Sanity check */
        if mod_.chn <= 0 || mod_.chn > XMP_MAX_CHANNELS as i32 {
            return -1;
        }
    }

    /* TODO: switch to time factor, verify whether or not this is DOS-only */
    mod_.bpm = 4 * mod_.bpm / 5;
    mod_.trk = mod_.chn * mod_.pat;

    /* FNK allows mode per instrument but we don't, so use linear for all */
    m.period_type = PERIOD_LINEAR;

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    /* Convert instruments */
    for i in 0..mod_.ins {
        let iu = i as usize;
        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        let fih = &ffh.fih[iu];
        let xxs = &mut mod_.xxs[iu];
        xxs.len = fih.length as i32;
        xxs.lps = fih.loop_start as i32;
        if xxs.lps == -1 {
            xxs.lps = 0;
        }
        /* TODO: GUS driver *only* seems to cut loop length by 64;
         * this needs further investigation. */
        xxs.lpe = fih.length as i32;
        xxs.flg = if fih.loop_start != u32::MAX {
            XMP_SAMPLE_LOOP
        } else {
            0
        };
        let len = xxs.len;
        let xxi = &mut mod_.xxi[iu];
        xxi.sub[0].vol = fih.volume as i32;
        xxi.sub[0].pan = fih.pan as i32;
        xxi.sub[0].sid = i;

        if len > 0 {
            xxi.nsm = 1;
        }

        libxmp_instrument_name(mod_, i, &ffh.fih[iu].name, 19);
    }

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Read and convert patterns */

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        mod_.event_mut(i, 0, ffh.pbrk[i as usize] as i32).f2t = FX_BREAK;

        for j in 0..64 {
            for k in 0..mod_.chn {
                let event = mod_.event_mut(i, k, j);
                if hio_read(&mut ev, 1, 3, f) < 3 {
                    return -1;
                }

                fnk_translate_event(event, &ev, &ffh);
            }
        }
    }

    /* Read samples */

    for i in 0..m.mod_.ins as usize {
        if m.mod_.xxs[i].len <= 2 {
            continue;
        }

        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[i], None) < 0 {
            return -1;
        }
    }

    for i in 0..m.mod_.chn {
        let pan = if allow_panning {
            0x80
        } else {
            defpan(m, (i % 2) * 0xff)
        };
        m.mod_.xxc[i as usize].pan = pan;
    }

    m.volbase = 0xff;
    m.quirk = QUIRK_VSALL;

    0
}
