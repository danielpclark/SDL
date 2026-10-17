// Rust translation of src/loaders/flt_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Startrekker (FLT4, FLT8, EXO4, EXO8) loader, with Startrekker AM
//! synth instruments from the module's `.NT` (or AudioSculpture `.AS`)
//! file.

use super::super::common::*;
use super::super::dataio::*;
use super::super::extras::InstrumentExtras;
use super::super::flt_extras::*;
use super::super::hio::*;
use super::super::path::*;
use super::super::rng::*;
use super::super::*;
use super::common::*;
use super::mod_load::{ModHeader, ModInstrument};
use super::sample::libxmp_load_sample;
use super::*;

pub(crate) const LIBXMP_LOADER_FLT: FormatLoader = FormatLoader {
    name: "Startrekker",
    test: flt_test,
    loader: flt_load,
};

/// Translation of `flt_test()`.
fn flt_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, start: i32) -> i32 {
    let mut buf = [0u8; 4];

    hio_seek(f, start as i64 + 1080, SEEK_SET);
    if hio_read(&mut buf, 1, 4, f) < 4 {
        return -1;
    }

    /* Also RASP? */
    if &buf[..3] != b"FLT" && &buf[..3] != b"EXO" {
        return -1;
    }

    if buf[3] != b'4' && buf[3] != b'8' && buf[3] != b'M' {
        return -1;
    }

    hio_seek(f, start as i64, SEEK_SET);
    libxmp_read_title(f, t, 20);

    0
}

/* Waveforms from the Startrekker 1.2 AM synth replayer code */

#[rustfmt::skip]
static AM_WAVEFORM: [[i8; 32]; 3] = [
    [    0,   25,   49,   71,   90,  106,  117,  125, /* Sine */
       127,  125,  117,  106,   90,   71,   49,   25,
         0,  -25,  -49,  -71,  -90, -106, -117, -125,
      -127, -125, -117, -106,  -90,  -71,  -49,  -25
    ],

    [ -128, -120, -112, -104,  -96,  -88,  -80,  -72, /* Ramp */
       -64,  -56,  -48,  -40,  -32,  -24,  -16,   -8,
         0,    8,   16,   24,   32,   40,   48,   56,
        64,   72,   80,   88,   96,  104,  112,  120
    ],

    [ -128, -128, -128, -128, -128, -128, -128, -128, /* Square */
      -128, -128, -128, -128, -128, -128, -128, -128,
       127,  127,  127,  127,  127,  127,  127,  127,
       127,  127,  127,  127,  127,  127,  127,  127
    ],
];

/// Translation of `struct am_instrument`.
#[derive(Clone, Copy, Default)]
struct AmInstrument {
    l0: u16,     /* start amplitude */
    a1l: u16,    /* attack level */
    a1s: u16,    /* attack speed */
    a2l: u16,    /* secondary attack level */
    a2s: u16,    /* secondary attack speed */
    sl: u16,     /* sustain level */
    ds: u16,     /* decay speed */
    st: u16,     /* sustain time */
    rs: u16,     /* release speed */
    wf: i16,     /* waveform */
    p_fall: i16, /* "pitch" fall (add to period each tick) */
    v_amp: i16,  /* vibrato amplitude */
    v_spd: i16,  /* vibrato speed */
    fq: i16,     /* base frequency */
}

/// Translation of `is_am_instrument()`.
fn is_am_instrument(nt: &mut HioHandle<'_>, i: i32) -> bool {
    let mut buf = [0u8; 28];

    hio_seek(nt, 144 + i as i64 * 120, SEEK_SET);
    if hio_read(&mut buf, 1, 28, nt) < 28 {
        return false;
    }
    if &buf[..2] != b"AM" {
        return false;
    }
    if readmem16b(&buf[26..]) /* WF */ > 3 {
        return false;
    }

    true
}

/* (upstream keeps convert_am_envelope(), a conversion of the AM synth
 * envelope parameters to an XM/IT envelope based on the Startrekker 1.2
 * docs, here under #if 0, as it may be useful if libxmp ever needs to
 * convert StarTrekker envelopes again) */

/// Translation of `read_am_instrument()`.
fn read_am_instrument(m: &mut ModuleData, nt: &mut HioHandle<'_>, i: i32) -> i32 {
    let iu = i as usize;
    let mut am = AmInstrument::default();
    let mut rng = RngState::default();
    let mut buf = [0u8; 30];
    let mut am_noise = [0u8; 1024];

    hio_seek(nt, 144 + i as i64 * 120 + 2 + 4, SEEK_SET);
    /* Allow partial/missing AM instruments (GTS/fa.worse face.mod). */
    if hio_read(&mut buf, 1, 30, nt) < 30 { /* (upstream logs a warning) */ }
    am.l0 = readmem16b(&buf[0..]);
    am.a1l = readmem16b(&buf[2..]);
    am.a1s = readmem16b(&buf[4..]);
    am.a2l = readmem16b(&buf[6..]);
    am.a2s = readmem16b(&buf[8..]);
    am.sl = readmem16b(&buf[10..]);
    am.ds = readmem16b(&buf[12..]);
    am.st = readmem16b(&buf[14..]);
    am.rs = readmem16b(&buf[18..]);
    am.wf = readmem16b(&buf[20..]) as i16;
    am.p_fall = readmem16b(&buf[22..]) as i16;
    am.v_amp = readmem16b(&buf[24..]) as i16;
    am.v_spd = readmem16b(&buf[26..]) as i16;
    am.fq = readmem16b(&buf[28..]) as i16;

    let mod_ = &mut m.mod_;
    let wave: &[u8];
    let mut wave_buf = [0u8; 32];
    {
        let xxs = &mut mod_.xxs[iu];
        if am.wf < 3 {
            xxs.len = 32;
            xxs.lps = 0;
            xxs.lpe = 32;
            for (w, &v) in wave_buf.iter_mut().zip(AM_WAVEFORM[am.wf as usize].iter()) {
                *w = v as u8;
            }
            wave = &wave_buf;
        } else {
            xxs.len = 1024;
            xxs.lps = 0;
            xxs.lpe = 1024;

            libxmp_init_random(&mut rng);
            for n in am_noise.iter_mut() {
                *n = libxmp_get_random(&mut rng, 256) as u8;
            }

            wave = &am_noise;
        }

        xxs.flg = XMP_SAMPLE_LOOP;
    }
    let xxi = &mut mod_.xxi[iu];
    xxi.nsm = 1;
    xxi.sub[0].xpo = -12 * am.fq as i32;
    xxi.sub[0].vwf = 0;
    xxi.sub[0].vde = am.v_amp as i32 * 4;
    xxi.sub[0].vra = am.v_spd as i32;

    if libxmp_flt_new_instrument_extras(xxi) < 0 {
        return -1;
    }

    let InstrumentExtras::Flt(extra) = &mut xxi.extra else {
        return -1;
    };
    extra.l0 = am.l0;
    extra.a1l = am.a1l;
    extra.a1s = am.a1s;
    extra.a2l = am.a2l;
    extra.a2s = am.a2s;
    extra.sl = am.sl;
    extra.ds = am.ds;
    extra.st = am.st;
    extra.rs = am.rs;
    extra.p_fall = am.p_fall;
    extra.fq = am.fq as u16; /* Required to fix toneporta */

    /* (upstream keeps an implementation of P.FALL using a pitch envelope
     * here under #if 0) */

    let smpctl = m.smpctl;
    if libxmp_load_sample(
        smpctl,
        None,
        SAMPLE_FLAG_NOLOAD,
        &mut m.mod_.xxs[iu],
        Some(wave),
    ) != 0
    {
        return -1;
    }

    0
}

/// Translation of `flt_check_sample_file()`.
fn flt_check_sample_file(
    sp: &mut LibxmpPath,
    ext_pos: usize,
    ext: &[u8],
) -> Option<HioHandle<'static>> {
    if libxmp_path_suffix_at(sp, ext_pos, Some(ext)) != 0 {
        return None;
    }

    hio_open(sp.path.as_deref()?)
}

/// Translation of `flt_load()`.
fn flt_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut mod_event = [0u8; 4];
    let mut sp = LibxmpPath::default();
    let mut buf = [0u8; 16];
    let mut nt: Option<HioHandle<'static>> = None;

    /* See if we have the synth parameters file */
    let mut am_synth = false;
    libxmp_path_init(&mut sp);
    if libxmp_path_join(&mut sp, m.dirname.as_deref(), m.basename.as_deref()) == 0 {
        let sp_length = sp.length;

        nt = flt_check_sample_file(&mut sp, sp_length, b".NT");
        if nt.is_none() {
            nt = flt_check_sample_file(&mut sp, sp_length, b".nt");
        }
        if nt.is_none() {
            nt = flt_check_sample_file(&mut sp, sp_length, b".AS");
        }
        if nt.is_none() {
            nt = flt_check_sample_file(&mut sp, sp_length, b".as");
        }
    }
    libxmp_path_free(&mut sp);

    let mut tracker: &[u8] = b"Startrekker";

    if let Some(nt) = nt.as_mut() {
        if hio_read(&mut buf, 1, 16, nt) != 16 {
            return -1;
        }
        if &buf == b"ST1.2 ModuleINFO" {
            am_synth = true;
            tracker = b"Startrekker 1.2";
        } else if &buf == b"ST1.3 ModuleINFO" {
            am_synth = true;
            tracker = b"Startrekker 1.3";
        } else if &buf == b"AudioSculpture10" {
            am_synth = true;
            tracker = b"AudioSculpture 1.0";
        }
    }

    let mut mh = ModHeader {
        name: [0; 20],
        ins: [ModInstrument::default(); 31],
        len: 0,
        restart: 0,
        order: [0; 128],
        magic: [0; 4],
    };
    hio_read(&mut mh.name, 20, 1, f);
    for i in 0..31 {
        hio_read(&mut mh.ins[i].name, 22, 1, f);
        mh.ins[i].size = hio_read16b(f);
        mh.ins[i].finetune = hio_read8(f) as i8;
        mh.ins[i].volume = hio_read8(f) as i8;
        mh.ins[i].loop_start = hio_read16b(f);
        mh.ins[i].loop_size = hio_read16b(f);
    }
    mh.len = hio_read8(f);
    mh.restart = hio_read8(f);
    hio_read(&mut mh.order, 128, 1, f);
    hio_read(&mut mh.magic, 4, 1, f);

    let mod_ = &mut m.mod_;
    if mh.magic[3] == b'4' {
        mod_.chn = 4;
    } else {
        mod_.chn = 8;
    }

    mod_.ins = 31;
    mod_.smp = mod_.ins;
    mod_.len = mh.len as i32;
    mod_.rst = mh.restart as i32;
    mod_.xxo[..128].copy_from_slice(&mh.order);

    for i in 0..128 {
        if mod_.chn > 4 {
            mod_.xxo[i] >>= 1;
        }
        if mod_.xxo[i] as i32 > mod_.pat {
            mod_.pat = mod_.xxo[i] as i32;
        }
    }

    mod_.pat += 1;

    mod_.trk = mod_.chn * mod_.pat;

    if am_synth && libxmp_flt_new_module_extras(m) < 0 {
        return -1;
    }

    strncpy(&mut m.mod_.name, &mh.name, 20);
    let mut type_ = tracker.to_vec();
    type_.push(b' ');
    type_.extend_from_slice(&printf_s(&mh.magic, 4, 4));
    libxmp_set_type(m, &type_);

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
        xxs.len = 2 * ins.size as i32;
        xxs.lps = 2 * ins.loop_start as i32;
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
        xxi.rls = 0xfff;

        if len > 0 {
            xxi.nsm = 1;
        }

        libxmp_instrument_name(mod_, i, &ins.name, 22);
    }

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Load and convert patterns */

    /* "The format you are looking for is FLT8, and the ONLY two
     *  differences are: It says FLT8 instead of FLT4 or M.K., AND, the
     *  patterns are PAIRED. I thought this was the easiest 8 track
     *  format possible, since it can be loaded in a normal 4 channel
     *  tracker if you should want to rip sounds or patterns. So, in a
     *  8 track FLT8 module, patterns 00 and 01 is "really" pattern 00.
     *  Patterns 02 and 03 together is "really" pattern 01. That's it.
     *  Oh well, I didn't have the time to implement all effect commands
     *  either, so some FLT8 modules would play back badly (I think
     *  especially the portamento command uses a different "scale" than
     *  the normal portamento command, that would be hard to patch).
     */
    for i in 0..mod_.pat {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        for j in 0..(64 * 4) {
            let event = mod_.event_mut(i, j % 4, j / 4);
            if hio_read(&mut mod_event, 1, 4, f) < 4 {
                return -1;
            }
            libxmp_decode_noisetracker_event(event, &mod_event);
        }
        if mod_.chn > 4 {
            for j in 0..(64 * 4) {
                let event = mod_.event_mut(i, (j % 4) + 4, j / 4);
                if hio_read(&mut mod_event, 1, 4, f) < 4 {
                    return -1;
                }
                libxmp_decode_noisetracker_event(event, &mod_event);

                /* no macros */
                if event.fxt == 0x0e {
                    event.fxt = 0;
                    event.fxp = 0;
                }
            }
        }
    }

    /* no such limit for synth instruments
     * mod->flg |= XXM_FLG_MODRNG;
     */

    /* Load samples */

    for i in 0..m.mod_.smp {
        if am_synth {
            if let Some(nt) = nt.as_mut() {
                if is_am_instrument(nt, i) {
                    if hio_seek(f, m.mod_.xxs[i as usize].len as i64, SEEK_CUR) < 0 {
                        return -1;
                    }
                    if read_am_instrument(m, nt, i) < 0 {
                        return -1;
                    }
                    continue;
                }
            }
        }
        let smpctl = m.smpctl;
        if libxmp_load_sample(
            smpctl,
            Some(f),
            SAMPLE_FLAG_FULLREP,
            &mut m.mod_.xxs[i as usize],
            None,
        ) < 0
        {
            return -1;
        }
    }

    0
}
