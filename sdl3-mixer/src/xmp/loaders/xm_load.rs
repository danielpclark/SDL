// Rust translation of src/loaders/xm_load.c and src/loaders/xm.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/*
 * Fri, 26 Jun 1998 17:45:59 +1000  Andrew Leahy <alf@cit.nepean.uws.edu.au>
 * Finally got it working on the DEC Alpha running DEC UNIX! In the pattern
 * reading loop I found I was getting "0" for (p-patbuf) and "0" for
 * xph.datasize, the next if statement (where it tries to read the patbuf)
 * would then cause a seg_fault.
 *
 * Sun Sep 27 12:07:12 EST 1998  Claudio Matsuoka <claudio@pos.inf.ufpr.br>
 * Extended Module 1.02 stores data in a different order, we must handle
 * this accordingly. MAX_SAMP used as a workaround to check the number of
 * samples recognized by the player.
 */

//! The Fast Tracker II XM loader (and OXM, with Ogg Vorbis samples).

use super::super::common::*;
use super::super::dataio::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;
use crate::stb_vorbis::stb_vorbis_decode_memory;

const XM_EVENT_PACKING: u8 = 0x80;
const XM_EVENT_PACK_MASK: u8 = 0x7f;
const XM_EVENT_NOTE_FOLLOWS: u8 = 0x01;
const XM_EVENT_INSTRUMENT_FOLLOWS: u8 = 0x02;
const XM_EVENT_VOLUME_FOLLOWS: u8 = 0x04;
const XM_EVENT_FXTYPE_FOLLOWS: u8 = 0x08;
const XM_EVENT_FXPARM_FOLLOWS: u8 = 0x10;
const XM_LINEAR_FREQ: i32 = 0x01;
const XM_LOOP_MASK: u8 = 0x03;
const XM_LOOP_NONE: u8 = 0;
const XM_LOOP_FORWARD: u8 = 1;
const XM_LOOP_PINGPONG: u8 = 2;
const XM_SAMPLE_16BIT: u8 = 0x10;
const XM_SAMPLE_STEREO: u8 = 0x20;
const XM_ENVELOPE_ON: u8 = 0x01;
const XM_ENVELOPE_SUSTAIN: u8 = 0x02;
const XM_ENVELOPE_LOOP: u8 = 0x04;
const XM_LINEAR_PERIOD_MODE: u16 = 0x01;

/// Translation of `struct xm_file_header`.
struct XmFileHeader {
    id: [u8; 17],      /* ID text: "Extended module: " */
    name: [u8; 20],    /* Module name, padded with zeroes */
    tracker: [u8; 20], /* Tracker name */
    version: u16,      /* Version number, minor-major */
    headersz: u32,     /* Header size */
    songlen: u16,      /* Song length (in pattern order table) */
    restart: u16,      /* Restart position */
    channels: u16,     /* Number of channels (2,4,6,8,10,...,32) */
    patterns: u16,     /* Number of patterns (max 256) */
    instruments: u16,  /* Number of instruments (max 128) */
    flags: u16,        /* bit 0: 0=Amiga freq table, 1=Linear */
    tempo: u16,        /* Default tempo */
    bpm: u16,          /* Default BPM */
    order: [u8; 256],  /* Pattern order table */
}

/// Translation of `struct xm_pattern_header`.
#[derive(Default)]
struct XmPatternHeader {
    length: u32,   /* Pattern header length */
    packing: u8,   /* Packing type (always 0) */
    rows: u16,     /* Number of rows in pattern (1..256) */
    datasize: u16, /* Packed patterndata size */
}

/// Translation of `struct xm_instrument_header`.
#[derive(Default)]
struct XmInstrumentHeader {
    size: u32,      /* Instrument size */
    name: [u8; 22], /* Instrument name */
    type_: u8,      /* Instrument type (always 0) */
    samples: u16,   /* Number of samples in instrument */
    sh_size: u32,   /* Sample header size */
}

/// Translation of `struct xm_instrument`.
#[derive(Clone, Copy)]
struct XmInstrument {
    sample: [u8; 96], /* Sample number for all notes */
    v_env: [u16; 24], /* Points for volume envelope */
    p_env: [u16; 24], /* Points for panning envelope */
    v_pts: u8,        /* Number of volume points */
    p_pts: u8,        /* Number of panning points */
    v_sus: u8,        /* Volume sustain point */
    v_start: u8,      /* Volume loop start point */
    v_end: u8,        /* Volume loop end point */
    p_sus: u8,        /* Panning sustain point */
    p_start: u8,      /* Panning loop start point */
    p_end: u8,        /* Panning loop end point */
    v_type: u8,       /* Bit 0: On; 1: Sustain; 2: Loop */
    p_type: u8,       /* Bit 0: On; 1: Sustain; 2: Loop */
    y_wave: u8,       /* Vibrato waveform */
    y_sweep: u8,      /* Vibrato sweep */
    y_depth: u8,      /* Vibrato depth */
    y_rate: u8,       /* Vibrato rate */
    v_fade: u16,      /* Volume fadeout */
}

impl XmInstrument {
    const ZERO: XmInstrument = XmInstrument {
        sample: [0; 96],
        v_env: [0; 24],
        p_env: [0; 24],
        v_pts: 0,
        p_pts: 0,
        v_sus: 0,
        v_start: 0,
        v_end: 0,
        p_sus: 0,
        p_start: 0,
        p_end: 0,
        v_type: 0,
        p_type: 0,
        y_wave: 0,
        y_sweep: 0,
        y_depth: 0,
        y_rate: 0,
        v_fade: 0,
    };
}

/// Translation of `struct xm_sample_header`.
#[derive(Clone, Copy, Default)]
struct XmSampleHeader {
    length: u32,      /* Sample length */
    loop_start: u32,  /* Sample loop start */
    loop_length: u32, /* Sample loop length */
    volume: u8,       /* Volume */
    finetune: i8,     /* Finetune (signed byte -128..+127) */
    type_: u8,        /* 0=No loop,1=Fwd loop,2=Ping-pong,16-bit */
    pan: u8,          /* Panning (0-255) */
    relnote: i8,      /* Relative note number (signed byte) */
    reserved: u8,     /* Reserved */
    name: [u8; 22],   /* Sample_name */
}

pub(crate) const LIBXMP_LOADER_XM: FormatLoader = FormatLoader {
    name: "Fast Tracker II",
    test: xm_test,
    loader: xm_load,
};

/// Translation of `xm_test()`.
fn xm_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    let mut buf = [0u8; 20];

    if hio_read(&mut buf, 1, 17, f) < 17 {
        /* ID text */
        return -1;
    }

    if &buf[..17] != b"Extended Module: " {
        return -1;
    }

    libxmp_read_title(f, t, 20);

    0
}

/// Translation of `load_xm_pattern()`.
fn load_xm_pattern(
    m: &mut ModuleData,
    num: i32,
    version: i32,
    patbuf: &mut [u8],
    f: &mut HioHandle<'_>,
) -> i32 {
    let headsize: u32 = if version > 0x0102 { 9 } else { 8 };
    let mod_ = &mut m.mod_;
    let mut xph = XmPatternHeader::default();

    xph.length = hio_read32l(f);
    xph.packing = hio_read8(f);
    xph.rows = if version > 0x0102 {
        hio_read16l(f)
    } else {
        hio_read8(f) as u16 + 1
    };

    /* Sanity check */
    if xph.rows > 256 {
        return -1;
    }

    xph.datasize = hio_read16l(f);
    hio_seek(f, xph.length.wrapping_sub(headsize) as i64, SEEK_CUR);
    if hio_error(f) != 0 {
        return -1;
    }

    let mut r = xph.rows as i32;
    if r == 0 {
        r = 0x100;
    }

    if libxmp_alloc_pattern_tracks(mod_, num, r) < 0 {
        return -1;
    }

    if xph.datasize == 0 {
        return 0;
    }

    let mut size = xph.datasize as i32;
    let mut pat = 0usize;

    let size_read = hio_read(patbuf, 1, size as usize, f);
    if size_read < size as usize {
        patbuf[size_read..size as usize].fill(0);
    }

    for j in 0..r {
        for k in 0..mod_.chn {
            /* Some XMs have cleanly truncated patterns. See:
             * Balrog/f0rtify.xm; Decayer-9/purification.xm;
             * Falcon (PL)/eaten vinyl.xm; Headcrasher/microcosm.xm;
             * Jazztiz/ta-da-da-da.xm; Jisemdu/smile.xm;
             * Markus Plomgren/cool jazzy jeff!!!.xm;
             * Orange/optical.xm; Skyraver/spirit of life.xm;
             * Sonic (UK)'s atomic_subculture.xm, luvdup.xm,
             * phuture.xm; Teemu/speed.xm; Warhawk/anaconda.xm.
             */
            if pat == xph.datasize as usize {
                /* early_pattern_end: */
                return 0;
            }

            let event = mod_.event_mut(num, k, j);

            size -= 1;
            if size < 0 {
                return -1;
            }

            let b = patbuf[pat];
            pat += 1;
            if b & XM_EVENT_PACKING != 0 {
                if b & XM_EVENT_NOTE_FOLLOWS != 0 {
                    size -= 1;
                    if size < 0 {
                        return -1;
                    }
                    event.note = patbuf[pat];
                    pat += 1;
                }
                if b & XM_EVENT_INSTRUMENT_FOLLOWS != 0 {
                    size -= 1;
                    if size < 0 {
                        return -1;
                    }
                    event.ins = patbuf[pat];
                    pat += 1;
                }
                if b & XM_EVENT_VOLUME_FOLLOWS != 0 {
                    size -= 1;
                    if size < 0 {
                        return -1;
                    }
                    event.vol = patbuf[pat];
                    pat += 1;
                }
                if b & XM_EVENT_FXTYPE_FOLLOWS != 0 {
                    size -= 1;
                    if size < 0 {
                        return -1;
                    }
                    event.fxt = patbuf[pat];
                    pat += 1;
                }
                if b & XM_EVENT_FXPARM_FOLLOWS != 0 {
                    size -= 1;
                    if size < 0 {
                        return -1;
                    }
                    event.fxp = patbuf[pat];
                    pat += 1;
                }
            } else {
                size -= 4;
                if size < 0 {
                    return -1;
                }
                event.note = b;
                event.ins = patbuf[pat];
                event.vol = patbuf[pat + 1];
                event.fxt = patbuf[pat + 2];
                event.fxp = patbuf[pat + 3];
                pat += 4;
            }

            /* Sanity check */
            match event.fxt {
                18 | 19 | 22 | 23 | 24 | 26 | 28 | 30 | 31 | 32 => event.fxt = 0,
                _ => {}
            }
            if event.fxt > 34 {
                event.fxt = 0;
            }

            if event.note == 0x61 {
                event.note = XMP_KEY_OFF;
            } else if event.note > 0 {
                event.note = event.note.wrapping_add(12);
            }

            if event.fxt == 0x0e {
                if msn(event.fxp) == EX_FINETUNE {
                    let val = ((lsn(event.fxp) - 8) & 0xf) as u8;
                    event.fxp = ((EX_FINETUNE << 4) as u8) | val;
                }
                match event.fxp {
                    0x43 | 0x73 => event.fxp -= 1,
                    _ => {}
                }
            }
            if event.fxt == FX_XF_PORTA && msn(event.fxp) == 0x09 {
                /* Translate MPT hacks */
                match lsn(event.fxp) {
                    0x0 | 0x1 => {
                        /* Surround off */
                        /* Surround on */
                        event.fxt = FX_SURROUND;
                        event.fxp = lsn(event.fxp) as u8;
                    }
                    0xe | 0xf => {
                        /* Play forward */
                        /* Play reverse */
                        event.fxt = FX_REVERSE;
                        event.fxp = (lsn(event.fxp) - 0xe) as u8;
                    }
                    _ => {}
                }
            }

            if event.vol == 0 {
                continue;
            }

            /* Volume set */
            if event.vol >= 0x10 && event.vol <= 0x50 {
                event.vol -= 0x0f;
                continue;
            }

            /* Volume column effects */
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
                    event.f2p = (event.vol - 0xc0) << 4;
                }
                0x0d => {
                    /* Pan slide left */
                    event.f2t = FX_PANSL_NOMEM;
                    event.f2p = (event.vol - 0xd0) << 4;
                }
                0x0e => {
                    /* Pan slide right */
                    event.f2t = FX_PANSL_NOMEM;
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
        }
    }
    /* early_pattern_end: */
    0
}

/// Translation of `load_patterns()`.
fn load_patterns(m: &mut ModuleData, version: i32, f: &mut HioHandle<'_>) -> i32 {
    m.mod_.pat += 1;
    if libxmp_init_pattern(&mut m.mod_) < 0 {
        return -1;
    }

    let Ok(mut patbuf) = try_vec(65536, 0u8) else {
        return -1;
    };

    let mut i = 0;
    while i < m.mod_.pat - 1 {
        if load_xm_pattern(m, i, version, &mut patbuf, f) < 0 {
            return -1;
        }
        i += 1;
    }

    /* Alloc one extra pattern */
    {
        let mod_ = &mut m.mod_;
        let t = i * mod_.chn;

        if libxmp_alloc_pattern(mod_, i) < 0 {
            return -1;
        }

        mod_.pattern_mut(i).rows = 64;

        if libxmp_alloc_track(mod_, t, 64) < 0 {
            return -1;
        }

        for j in 0..mod_.chn as usize {
            mod_.pattern_mut(i).index[j] = t;
        }
    }

    0
}

/* Packed structures size */
const XM_INST_HEADER_SIZE: i32 = 29;
const XM_INST_SIZE: i32 = 212;

/* grass.near.the.house.xm defines 23 samples in instrument 1. FT2 docs
 * specify at most 16. See https://github.com/libxmp/libxmp/issues/168
 * for more details. */
const XM_MAX_SAMPLES_PER_INST: usize = 32;

const MAGIC_OGGS: u32 = 0x4f676753;

/// Translation of `is_ogg_sample()`.
fn is_ogg_sample(f: &mut HioHandle<'_>, xxs: &XmpSample) -> bool {
    /* Sample must be at least 4 bytes long to be an OGG sample.
     * Bonnie's Bookstore music.oxm contains zero length samples
     * followed immediately by OGG samples. */
    if xxs.len < 4 {
        return false;
    }

    /* size = */
    hio_read32l(f);
    let id = hio_read32b(f);
    if hio_error(f) != 0 || hio_seek(f, -8, SEEK_CUR) < 0 {
        return false;
    }

    if id != MAGIC_OGGS {
        /* copy input data if not Ogg file */
        return false;
    }

    true
}

/// Translation of `oggdec()`.
fn oggdec(smpctl: i32, f: &mut HioHandle<'_>, xxs: &mut XmpSample, len: i32) -> i32 {
    let mut flags = 0;

    let Ok(mut data) = try_vec(len.max(0) as usize, 0u8) else {
        return -1;
    };

    hio_read32b(f);
    if hio_error(f) != 0 || hio_read(&mut data, 1, (len - 4) as usize, f) != (len - 4) as usize {
        return -1;
    }

    let mut ch = 0;
    let mut rate = 0;
    let mut pcm16: Option<Vec<i16>> = None;
    let mut n = stb_vorbis_decode_memory(&data, &mut ch, &mut rate, &mut pcm16);
    drop(data);

    if n < 0 || ch != 1 {
        return -1;
    }
    let pcm16 = pcm16.unwrap_or_default();

    let pcm: Vec<u8> = if xxs.flg & XMP_SAMPLE_16BIT == 0 && n > 0 {
        let mut pcm = Vec::new();
        if pcm.try_reserve_exact(n as usize).is_err() {
            return -1;
        }
        pcm.extend(pcm16[..n as usize].iter().map(|&s| (s >> 8) as u8));
        pcm
    } else {
        let mut pcm = Vec::new();
        if pcm.try_reserve_exact(pcm16.len() * 2).is_err() {
            return -1;
        }
        for s in &pcm16 {
            pcm.extend_from_slice(&s.to_ne_bytes());
        }
        pcm
    };
    drop(pcm16);
    if xxs.flg & XMP_SAMPLE_STEREO != 0 {
        /* OXM stereo is a single channel non-interleaved stream. */
        n >>= 1;
    }
    xxs.len = n;

    flags |= SAMPLE_FLAG_NOLOAD;
    if cfg!(target_endian = "big") {
        flags |= SAMPLE_FLAG_BIGEND;
    }

    libxmp_load_sample(smpctl, None, flags, xxs, Some(&pcm))
}

/// Translation of `load_instruments()`.
fn load_instruments(
    m: &mut ModuleData,
    version: i32,
    mpt_ins_headers: &mut bool,
    f: &mut HioHandle<'_>,
) -> i32 {
    let mut xih = XmInstrumentHeader::default();
    let mut xi = XmInstrument::ZERO;
    let mut xsh = [XmSampleHeader::default(); XM_MAX_SAMPLES_PER_INST];
    let mut sample_num = 0;
    let mut buf = [0u8; 208];

    /* ESTIMATED value! We don't know the actual value at this point */
    m.mod_.smp = MAX_SAMPLES;

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    for i in 0..m.mod_.ins {
        let instr_pos = hio_tell(f);

        /* Modules converted with MOD2XM 1.0 always say we have 31
         * instruments, but file may end abruptly before that. Also covers
         * XMLiTE stripped modules and truncated files. This test will not
         * work if file has trailing garbage.
         *
         * Note: loading 4 bytes past the instrument header to get the
         * sample header size (if it exists). This is NOT considered to
         * be part of the instrument header.
         */
        if hio_read(&mut buf, (XM_INST_HEADER_SIZE + 4) as usize, 1, f) != 1 {
            break;
        }

        xih.size = readmem32l(&buf); /* Instrument size */
        xih.name.copy_from_slice(&buf[4..26]); /* Instrument name */
        xih.type_ = buf[26]; /* Instrument type (always 0) */
        xih.samples = readmem16l(&buf[27..]); /* Number of samples */
        xih.sh_size = readmem32l(&buf[29..]); /* Sample header size */

        /* Sanity check */
        if (xih.size as i32) < XM_INST_HEADER_SIZE {
            return -1;
        }

        if xih.samples as usize > XM_MAX_SAMPLES_PER_INST
            || (xih.samples > 0 && xih.sh_size > 0x100)
        {
            return -1;
        }

        /* Yet another Modplug Tracker tell: it saves huge zero-filled
         * instrument headers for unused instruments. */
        if xih.size == 0x107 && xih.samples == 0 && xih.sh_size == 0 {
            *mpt_ins_headers = true;
        }

        libxmp_instrument_name(&mut m.mod_, i, &xih.name, 22);

        m.mod_.xxi[i as usize].nsm = xih.samples as i32;

        if m.mod_.xxi[i as usize].nsm == 0 {
            /* Sample size should be in struct xm_instrument according to
             * the official format description, but FT2 actually puts it in
             * struct xm_instrument header. There's a tracker or converter
             * that follow the specs, so we must handle both cases (see
             * "Braintomb" by Jazztiz/ART).
             */

            /* Umm, Cyke O'Path <cyker@heatwave.co.uk> sent me a couple of
             * mods ("Breath of the Wind" and "Broken Dimension") that
             * reserve the instrument data space after the instrument header
             * even if the number of instruments is set to 0. In these modules
             * the instrument header size is marked as 263. The following
             * generalization should take care of both cases.
             */

            if hio_seek(
                f,
                (xih.size as i32 - (XM_INST_HEADER_SIZE + 4)) as i64,
                SEEK_CUR,
            ) < 0
            {
                return -1;
            }

            continue;
        }

        let nsm = m.mod_.xxi[i as usize].nsm;
        if libxmp_alloc_subinstrument(&mut m.mod_, i, nsm) < 0 {
            return -1;
        }

        /* for BoobieSqueezer (see http://boobie.rotfl.at/)
         * It works pretty much the same way as Impulse Tracker's sample
         * only mode, where it will strip off the instrument data.
         */
        if xih.size < (XM_INST_HEADER_SIZE + XM_INST_SIZE) as u32 {
            xi = XmInstrument::ZERO;
            hio_seek(
                f,
                xih.size.wrapping_sub((XM_INST_HEADER_SIZE + 4) as u32) as i64,
                SEEK_CUR,
            );
        } else {
            let mut b = 0;

            if hio_read(&mut buf, 208, 1, f) != 1 {
                return -1;
            }

            xi.sample.copy_from_slice(&buf[..96]); /* Sample map */
            b += 96;

            for j in 0..24 {
                xi.v_env[j] = readmem16l(&buf[b..]); /* Points for volume envelope */
                b += 2;
            }

            for j in 0..24 {
                xi.p_env[j] = readmem16l(&buf[b..]); /* Points for pan envelope */
                b += 2;
            }

            xi.v_pts = buf[b]; /* Number of volume points */
            xi.p_pts = buf[b + 1]; /* Number of pan points */
            xi.v_sus = buf[b + 2]; /* Volume sustain point */
            xi.v_start = buf[b + 3]; /* Volume loop start point */
            xi.v_end = buf[b + 4]; /* Volume loop end point */
            xi.p_sus = buf[b + 5]; /* Pan sustain point */
            xi.p_start = buf[b + 6]; /* Pan loop start point */
            xi.p_end = buf[b + 7]; /* Pan loop end point */
            xi.v_type = buf[b + 8]; /* Bit 0:On 1:Sustain 2:Loop */
            xi.p_type = buf[b + 9]; /* Bit 0:On 1:Sustain 2:Loop */
            xi.y_wave = buf[b + 10]; /* Vibrato waveform */
            xi.y_sweep = buf[b + 11]; /* Vibrato sweep */
            xi.y_depth = buf[b + 12]; /* Vibrato depth */
            xi.y_rate = buf[b + 13]; /* Vibrato rate */
            b += 14;
            xi.v_fade = readmem16l(&buf[b..]); /* Volume fadeout */

            /* Skip reserved space */
            if hio_seek(
                f,
                (xih.size as i32 - (XM_INST_HEADER_SIZE + XM_INST_SIZE)) as i64,
                SEEK_CUR,
            ) < 0
            {
                return -1;
            }

            let xxi = &mut m.mod_.xxi[i as usize];

            /* Envelope */
            xxi.rls = (xi.v_fade as i32) << 1;
            xxi.aei.npt = xi.v_pts as i32;
            xxi.aei.sus = xi.v_sus as i32;
            xxi.aei.lps = xi.v_start as i32;
            xxi.aei.lpe = xi.v_end as i32;
            xxi.aei.flg = xi.v_type as i32;
            xxi.pei.npt = xi.p_pts as i32;
            xxi.pei.sus = xi.p_sus as i32;
            xxi.pei.lps = xi.p_start as i32;
            xxi.pei.lpe = xi.p_end as i32;
            xxi.pei.flg = xi.p_type as i32;

            if xxi.aei.npt <= 0 || xxi.aei.npt > 12
            /*XMP_MAX_ENV_POINTS */
            {
                xxi.aei.flg &= !XMP_ENVELOPE_ON;
            } else {
                for k in 0..(xxi.aei.npt * 2) as usize {
                    xxi.aei.data[k] = xi.v_env[k] as i16;
                }
            }

            if xxi.pei.npt <= 0 || xxi.pei.npt > 12
            /*XMP_MAX_ENV_POINTS */
            {
                xxi.pei.flg &= !XMP_ENVELOPE_ON;
            } else {
                for k in 0..(xxi.pei.npt * 2) as usize {
                    xxi.pei.data[k] = xi.p_env[k] as i16;
                }
            }

            for j in 12..108 {
                xxi.map[j].ins = xi.sample[j - 12];
                if xxi.map[j].ins as i32 >= xxi.nsm {
                    xxi.map[j].ins = 0xff;
                }
            }
        }

        /* Read subinstrument and sample parameters */

        for j in 0..nsm as usize {
            if sample_num >= m.mod_.smp {
                let smp = m.mod_.smp;
                if libxmp_realloc_samples(m, smp * 3 / 2) < 0 {
                    return -1;
                }
            }

            if hio_read(&mut buf, 40, 1, f) != 1 {
                return -1;
            }

            let mut b = 0;
            xsh[j].length = readmem32l(&buf[b..]); /* Sample length */
            b += 4;

            /* Sanity check */
            if xsh[j].length > MAX_SAMPLE_SIZE as u32 {
                return -1;
            }

            xsh[j].loop_start = readmem32l(&buf[b..]); /* Sample loop start */
            b += 4;
            xsh[j].loop_length = readmem32l(&buf[b..]); /* Sample loop length */
            b += 4;
            xsh[j].volume = buf[b]; /* Volume */
            xsh[j].finetune = buf[b + 1] as i8; /* Finetune (-128..+127) */
            xsh[j].type_ = buf[b + 2]; /* Flags */
            xsh[j].pan = buf[b + 3]; /* Panning (0-255) */
            xsh[j].relnote = buf[b + 4] as i8; /* Relative note number */
            xsh[j].reserved = buf[b + 5];
            b += 6;
            xsh[j].name.copy_from_slice(&buf[b..b + 22]);

            let sub = &mut m.mod_.xxi[i as usize].sub[j];
            sub.vol = xsh[j].volume as i32;
            sub.pan = xsh[j].pan as i32;
            sub.xpo = xsh[j].relnote as i32;
            sub.fin = xsh[j].finetune as i32;
            sub.vwf = xi.y_wave as i32;
            sub.vde = (xi.y_depth as i32) << 2;
            sub.vra = xi.y_rate as i32;
            sub.vsw = xi.y_sweep as i32;
            sub.sid = sample_num;

            let xxs = &mut m.mod_.xxs[sample_num as usize];

            libxmp_copy_adjust(&mut xxs.name, &xsh[j].name, 22);

            xxs.len = xsh[j].length as i32;
            xxs.lps = xsh[j].loop_start as i32;
            xxs.lpe = xsh[j].loop_start.wrapping_add(xsh[j].loop_length) as i32;

            xxs.flg = 0;
            if xsh[j].type_ & XM_SAMPLE_16BIT != 0 {
                xxs.flg |= XMP_SAMPLE_16BIT;
                xxs.len >>= 1;
                xxs.lps >>= 1;
                xxs.lpe >>= 1;
            }
            if xsh[j].type_ & XM_SAMPLE_STEREO != 0 {
                xxs.flg |= XMP_SAMPLE_STEREO;
                xxs.len >>= 1;
                xxs.lps >>= 1;
                xxs.lpe >>= 1;
            }

            xxs.flg |= if xsh[j].type_ & XM_LOOP_FORWARD != 0 {
                XMP_SAMPLE_LOOP
            } else {
                0
            };
            xxs.flg |= if xsh[j].type_ & XM_LOOP_PINGPONG != 0 {
                XMP_SAMPLE_LOOP | XMP_SAMPLE_LOOP_BIDIR
            } else {
                0
            };

            sample_num += 1;
        }

        /* Read actual sample data */

        let mut total_sample_size: i64 = 0;
        for j in 0..nsm as usize {
            let sid = m.mod_.xxi[i as usize].sub[j].sid as usize;

            let mut flags = SAMPLE_FLAG_DIFF;
            if xsh[j].reserved == 0xad {
                flags = SAMPLE_FLAG_ADPCM;
            }

            if version > 0x0103 {
                let smpctl = m.smpctl;
                let xxs = &mut m.mod_.xxs[sid];
                if is_ogg_sample(f, xxs) {
                    if oggdec(smpctl, f, xxs, xsh[j].length as i32) < 0 {
                        return -1;
                    }

                    total_sample_size += xsh[j].length as i64;
                    continue;
                }

                if libxmp_load_sample(smpctl, Some(f), flags, xxs, None) < 0 {
                    return -1;
                }
                if flags & SAMPLE_FLAG_ADPCM != 0 {
                    total_sample_size += 16 + (xsh[j].length.wrapping_add(1) >> 1) as i64;
                } else {
                    total_sample_size += xsh[j].length as i64;
                }
            }
        }

        /* Reposition correctly in case of 16-bit sample having odd in-file length.
         * See "Lead Lined for '99", reported by Dennis Mulleneers.
         */
        if hio_seek(
            f,
            instr_pos + xih.size as i64 + 40 * xih.samples as i64 + total_sample_size,
            SEEK_SET,
        ) < 0
        {
            return -1;
        }
    }

    /* Final sample number adjustment */
    if libxmp_realloc_samples(m, sample_num) < 0 {
        return -1;
    }

    0
}

/// Translation of `xm_load()`.
fn xm_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut tracker_name = [0u8; 21];
    let mut claims_ft2 = false;
    let mut is_mpt_old = false;
    let mut is_mpt_116 = false;
    let mut mpt_ins_headers = false;
    let mut buf = [0u8; 80];

    if hio_read(&mut buf, 80, 1, f) != 1 {
        return -1;
    }

    let mut xfh = XmFileHeader {
        id: [0; 17],
        name: [0; 20],
        tracker: [0; 20],
        version: 0,
        headersz: 0,
        songlen: 0,
        restart: 0,
        channels: 0,
        patterns: 0,
        instruments: 0,
        flags: 0,
        tempo: 0,
        bpm: 0,
        order: [0; 256],
    };
    xfh.id.copy_from_slice(&buf[..17]); /* ID text */
    xfh.name.copy_from_slice(&buf[17..37]); /* Module name */
    /* */
    /* skip 0x1a */
    xfh.tracker.copy_from_slice(&buf[38..58]); /* Tracker name */
    xfh.version = readmem16l(&buf[58..]); /* Version number, minor-major */
    xfh.headersz = readmem32l(&buf[60..]); /* Header size */
    xfh.songlen = readmem16l(&buf[64..]); /* Song length */
    xfh.restart = readmem16l(&buf[66..]); /* Restart position */
    xfh.channels = readmem16l(&buf[68..]); /* Number of channels */
    xfh.patterns = readmem16l(&buf[70..]); /* Number of patterns */
    xfh.instruments = readmem16l(&buf[72..]); /* Number of instruments */
    xfh.flags = readmem16l(&buf[74..]); /* 0=Amiga freq table, 1=Linear */
    xfh.tempo = readmem16l(&buf[76..]); /* Default tempo */
    xfh.bpm = readmem16l(&buf[78..]); /* Default BPM */

    /* Sanity checks */
    if xfh.songlen > 256 {
        return -1;
    }
    if xfh.patterns > 256 {
        return -1;
    }
    if xfh.instruments > 255 {
        return -1;
    }

    if xfh.channels as usize > XMP_MAX_CHANNELS {
        return -1;
    }

    /* FT2 and MPT allow up to 255 BPM. OpenMPT allows up to 1000 BPM. */
    if (xfh.tempo >= 32 || xfh.bpm < 32 || xfh.bpm > 1000) && &xfh.tracker[..6] != b"MED2XM" {
        return -1;
    }

    /* Honor header size -- needed by BoobieSqueezer XMs */
    let len = xfh.headersz.wrapping_sub(0x14) as i32;
    if !(0..=256).contains(&len) {
        return -1;
    }

    xfh.order = [0; 256];
    if hio_read(&mut xfh.order, len as usize, 1, f) != 1 {
        /* Pattern order table */
        return -1;
    }

    let mod_ = &mut m.mod_;
    strncpy(&mut mod_.name, &xfh.name, 20);

    mod_.len = xfh.songlen as i32;
    mod_.chn = xfh.channels as i32;
    mod_.pat = xfh.patterns as i32;
    mod_.ins = xfh.instruments as i32;
    mod_.rst = if xfh.restart >= xfh.songlen {
        0
    } else {
        xfh.restart as i32
    };
    mod_.spd = xfh.tempo as i32;
    mod_.bpm = xfh.bpm as i32;
    mod_.trk = mod_.chn * mod_.pat + 1;

    m.c4rate = C4_NTSC_RATE;
    m.period_type = if xfh.flags & XM_LINEAR_PERIOD_MODE != 0 {
        PERIOD_LINEAR
    } else {
        PERIOD_AMIGA
    };

    let mod_ = &mut m.mod_;
    let n = mod_.len as usize;
    mod_.xxo[..n].copy_from_slice(&xfh.order[..n]);
    /*tracker_name[20] = 0;*/
    {
        /* snprintf(tracker_name, 21, "%-20.20s", xfh.tracker) */
        let t = &xfh.tracker[..strlen(&xfh.tracker)];
        tracker_name[..t.len()].copy_from_slice(t);
        for c in &mut tracker_name[t.len()..20] {
            *c = b' ';
        }
        tracker_name[20] = 0;
    }
    let mut i: i32 = 20;
    while i >= 0 {
        if tracker_name[i as usize] == 0x20 {
            tracker_name[i as usize] = 0;
        }
        if tracker_name[i as usize] != 0 {
            break;
        }
        i -= 1;
    }

    if strneq(&tracker_name, b"FastTracker v2.00", 17) {
        m.quirk |= QUIRK_FT2BUGS;
        claims_ft2 = true;
    } else if strneq(&tracker_name, b"Fasttracker II clone", 20) {
        m.quirk |= QUIRK_FT2BUGS;
    } else if strneq(&tracker_name, b"OpenMPT ", 8) {
        /* OpenMPT accurately emulates weird FT2 bugs */
        m.quirk |= QUIRK_FT2BUGS;
    }
    let set_name = |tracker_name: &mut [u8; 21], s: &[u8]| {
        /* strcpy() */
        tracker_name[..s.len()].copy_from_slice(s);
        tracker_name[s.len()] = 0;
    };
    if xfh.headersz == 0x0113 {
        set_name(&mut tracker_name, b"unknown tracker");
        m.quirk &= !QUIRK_FT2BUGS;
    } else if tracker_name[0] == 0 {
        set_name(&mut tracker_name, b"Digitrakker"); /* best guess */
        m.quirk &= !QUIRK_FT2BUGS;
    }

    /* See MMD1 loader for explanation */
    if strneq(&tracker_name, b"MED2XM by J.Pynnone", 19) {
        if m.mod_.bpm <= 10 {
            m.mod_.bpm = 125 * (0x35 - m.mod_.bpm * 2) / 33;
        }
        m.quirk &= !QUIRK_FT2BUGS;
    }

    if strneq(&tracker_name, b"FastTracker v 2.00", 18) {
        set_name(&mut tracker_name, b"old ModPlug Tracker");
        m.quirk &= !QUIRK_FT2BUGS;
        is_mpt_old = true;
    }

    let tn = &tracker_name[..strlen(&tracker_name)];
    if tn == b"Skale Tracker" || tn == b"Sk@le Tracker" {
        /* Skale Tracker allows Dxx Byy to jump to row X. */
        m.flow_mode |= FLOW_JUMP_NO_ROW_SET;
    }

    let mut type_ = tn.to_vec();
    type_.extend_from_slice(
        format!(" XM {}.{:02}", xfh.version >> 8, xfh.version & 0xff).as_bytes(),
    );
    libxmp_set_type(m, &type_);

    /* Honor header size */
    if hio_seek(f, start as i64 + xfh.headersz as i64 + 60, SEEK_SET) < 0 {
        return -1;
    }

    /* XM 1.02/1.03 has a different patterns and instruments order */
    if xfh.version <= 0x0103 {
        if load_instruments(m, xfh.version as i32, &mut mpt_ins_headers, f) < 0 {
            return -1;
        }
        if load_patterns(m, xfh.version as i32, f) < 0 {
            return -1;
        }
    } else {
        if load_patterns(m, xfh.version as i32, f) < 0 {
            return -1;
        }
        if load_instruments(m, xfh.version as i32, &mut mpt_ins_headers, f) < 0 {
            return -1;
        }
    }

    /* XM 1.02 stores all samples after the patterns */
    if xfh.version <= 0x0103 {
        for i in 0..m.mod_.ins as usize {
            for j in 0..m.mod_.xxi[i].nsm as usize {
                let sid = m.mod_.xxi[i].sub[j].sid as usize;
                let smpctl = m.smpctl;
                if libxmp_load_sample(
                    smpctl,
                    Some(f),
                    SAMPLE_FLAG_DIFF,
                    &mut m.mod_.xxs[sid],
                    None,
                ) < 0
                {
                    return -1;
                }
            }
        }
    }

    /* Load MPT properties from the end of the file. */
    loop {
        let ext = hio_read32b(f);
        let mut sz = hio_read32l(f);
        let mut known = false;

        if hio_error(f) != 0 || sz > 0x7fffffff
        /* INT32_MAX */
        {
            break;
        }

        match ext {
            e if e == magic4(b't', b'e', b'x', b't') => {
                /* Song comment */
                known = true;
                if m.comment.is_none() && (sz as i64) <= hio_size(f) {
                    if let Ok(mut comment) = try_vec(sz as usize + 1, 0u8) {
                        sz = hio_read(&mut comment, 1, sz as usize, f) as u32;
                        comment[sz as usize] = b'\0';

                        for b in &mut comment[..sz as usize] {
                            let c = *b as i8 as i32;
                            if c == b'\r' as i32 {
                                *b = b'\n';
                            } else if (c < 32 || c > 127) && c != b'\n' as i32 && c != b'\t' as i32 {
                                *b = b'.';
                            }
                        }
                        comment.truncate(sz as usize + 1);
                        m.comment = Some(comment);
                        sz = 0;
                    }
                }
            }

            e if e == magic4(b'M', b'I', b'D', b'I') /* MIDI config */
                || e == magic4(b'P', b'N', b'A', b'M') /* Pattern names */
                || e == magic4(b'C', b'N', b'A', b'M') /* Channel names */
                || e == magic4(b'C', b'H', b'F', b'X') /* Channel plugins */
                || e == magic4(b'X', b'T', b'P', b'M') /* Inst. extensions */ =>
            {
                known = true;
                /* fall-through */
                /* Plugin definition */
                if (ext & magic4(b'F', b'X', 0, 0)) == magic4(b'F', b'X', 0, 0) {
                    known = true;
                }
            }

            _ => {
                /* Plugin definition */
                if (ext & magic4(b'F', b'X', 0, 0)) == magic4(b'F', b'X', 0, 0) {
                    known = true;
                }
            }
        }

        if known && claims_ft2 {
            is_mpt_116 = true;
        }

        if sz != 0 && hio_seek(f, sz as i64, SEEK_CUR) < 0 {
            break;
        }
        if ext == magic4(b'X', b'T', b'P', b'M') {
            break;
        }
    }

    if claims_ft2 && mpt_ins_headers {
        is_mpt_116 = true;
    }

    if is_mpt_116 {
        let type_ = format!(
            "ModPlug Tracker 1.16 XM {}.{:02}",
            xfh.version >> 8,
            xfh.version & 0xff
        );
        libxmp_set_type(m, type_.as_bytes());
    }
    if is_mpt_116 || is_mpt_old {
        m.quirk &= !QUIRK_FT2BUGS;
        m.flow_mode = FLOW_MODE_MPT_116;
        m.mvolbase = 48;
        m.mvol = 48;
        libxmp_apply_mpt_preamp(m);
    }

    for i in 0..m.mod_.chn as usize {
        m.mod_.xxc[i].pan = 0x80;
    }

    m.quirk |= QUIRKS_FT2 | QUIRK_FT2ENV;
    m.read_event_type = READ_EVENT_FT2;

    0
}
