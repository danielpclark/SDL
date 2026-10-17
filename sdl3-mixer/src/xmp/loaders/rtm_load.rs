// Rust translation of src/loaders/rtm_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Data structures from the specification of the RTM format version 1.10 by
 * Arnaud Hasenfratz
 */

//! The Real Tracker (RTM) loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_c2spd_to_note;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

/// Translation of `struct ObjectHeader`.
#[derive(Clone, Copy, Default)]
struct ObjectHeader {
    id: [u8; 4],      /* "RTMM", "RTND", "RTIN" or "RTSM" */
    rc: u8,           /* 0x20 */
    name: [u8; 32],   /* object name */
    eof: u8,          /* "\x1A" */
    version: u16,     /* version of the format (actual : 0x110) */
    header_size: u16, /* object header size */
}

/// Translation of `struct RTMMHeader`.
#[derive(Clone, Copy, Default)]
struct RtmmHeader {
    /* Real Tracker Music Module */
    flags: u16, /* song flags */
    /* bit 0 : linear table,
    bit 1 : track names present */
    ntrack: u8,        /* number of tracks */
    ninstr: u8,        /* number of instruments */
    nposition: u16,    /* number of positions */
    npattern: u16,     /* number of patterns */
    speed: u8,         /* initial speed */
    tempo: u8,         /* initial tempo */
    panning: [u8; 32], /* initial pannings (for S3M compatibility) */
    extra_data_size: u32, /* length of data after the header */
                       /* (the software, composer and, from version 1.12, original name, are
                        * read into locals or skipped) */
}

const RTMM_FLAG_LINEAR_TABLE: u16 = 1 << 0;
const RTMM_FLAG_TRACK_NAMES: u16 = 1 << 1;

/// Translation of `struct RTNDHeader`.
#[derive(Clone, Copy, Default)]
struct RtndHeader {
    /* Real Tracker Note Data */
    flags: u16, /* Always 1 */
    ntrack: u8,
    nrows: u16,
    datasize: u32, /* Size of packed data */
}

/// Translation of `struct EnvelopePoint`.
#[derive(Clone, Copy, Default)]
struct EnvelopePoint {
    x: i32,
    y: i32,
}

/// Translation of `struct Envelope`.
#[derive(Clone, Copy, Default)]
struct Envelope {
    npoint: u8,
    point: [EnvelopePoint; 12],
    sustain: u8,
    loopstart: u8,
    loopend: u8,
    flags: u16, /* bit 0 : enable envelope,
                bit 1 : sustain, bit 2 : loop */
}

const RTENV_FLAG_ENABLE: u16 = 1 << 0;
const RTENV_FLAG_SUSTAIN: u16 = 1 << 1;
const RTENV_FLAG_LOOP: u16 = 1 << 2;

/// Translation of `struct RTINHeader`.
#[derive(Clone, Copy)]
struct RtinHeader {
    /* Real Tracker Instrument */
    nsample: u8,
    flags: u16,       /* bit 0 : default panning enabled
                      bit 1 : mute samples */
    table: [u8; 120], /* sample number for each note */
    volume_env: Envelope,
    panning_env: Envelope,
    vibflg: i8,   /* vibrato type */
    vibsweep: i8, /* vibrato sweep */
    vibdepth: i8, /* vibrato depth */
    vibrate: i8,  /* vibrato rate */
    volfade: u16,

    /* version 1.10 */
    midi_port: u8,
    midi_channel: u8,
    midi_program: u8,
    midi_enable: u8,

    /* version 1.12 */
    midi_transpose: i8,
    midi_bender_range: u8,
    midi_base_volume: u8,
    midi_use_velocity: i8,
}

const RTIN_FLAG_DEFAULT_PAN: u16 = 1 << 0;
const RTIN_FLAG_MUTE_SAMPLES: u16 = 1 << 1;

/// Translation of `struct RTSMHeader`.
#[derive(Clone, Copy, Default)]
struct RtsmHeader {
    /* Real Tracker Sample */
    flags: u16, /* bit 1 : 16 bits,
                bit 2 : delta encoded (always) */
    basevolume: u8,
    defaultvolume: u8,
    length: u32,
    loop_: u8, /* =0:no loop, =1:forward loop,
               =2:bi-directional loop */
    loopbegin: u32,
    loopend: u32,
    basefreq: u32,
    basenote: u8,
    panning: i8, /* Panning from -64 to 64 */
}

const RTSM_FLAG_16_BIT: u16 = 1 << 1;
const RTSM_FLAG_DELTA_CODING: u16 = 1 << 2;
const RTSM_LOOP_ON: u8 = 1;
const RTSM_LOOP_BIDIR: u8 = 2;

pub(crate) const LIBXMP_LOADER_RTM: FormatLoader = FormatLoader {
    name: "Real Tracker",
    test: rtm_test,
    loader: rtm_load,
};

/// Translation of `rtm_test()`.
fn rtm_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    let mut buf = [0u8; 4];

    if hio_read(&mut buf, 1, 4, f) < 4 {
        return -1;
    }
    if &buf != b"RTMM" {
        return -1;
    }

    if hio_read8(f) != 0x20 {
        return -1;
    }

    libxmp_read_title(f, t, 32);

    0
}

const MAX_SAMP: i32 = 1024;

const FX_NONE: u8 = 0xff;
const FX_EXTENDED_IT: u8 = 0xfe;
const FX_PORTA_UP_MOD: u8 = 0xf1;
const FX_PORTA_DN_MOD: u8 = 0xf2;
const FX_TONE_VSLIDE_MOD: u8 = 0xf5;
const FX_VIBRA_VSLIDE_MOD: u8 = 0xf6;
const FX_VOLSLIDE_MOD: u8 = 0xfa;

/* MOD effects and S3M effects have separate effects memory.
 *
 * TODO: Axy/5xy/6xy have shared memory and dxy/kxy have shared memory.
 * Additionally, dxy will set the Axy memory if the dxy slide is NOT fine,
 * but Axy will never set the memory of dxy. Effects memory is positional,
 * i.e. effect 1 and effect 2 of a given channel have their own memory slots.
 * Simulating this would be a lot of work for a format that was barely used,
 * so just strip "fine" effects from Axy/5xy/6xy and let the two sets share
 * memory for now.
 *
 * TODO: 1xx has separate memory from fxx; 2xx has separate memory from exx.
 */
static RTM_FX: [u8; 41] = [
    /* 0 */ FX_ARPEGGIO, /* Arpeggio (MOD) */
    /* 1 */ FX_PORTA_UP_MOD, /* Portamento up (MOD) */
    /* 2 */ FX_PORTA_DN_MOD, /* Portamento down (MOD) */
    /* 3 */ FX_TONEPORTA, /* Toneporta */
    /* 4 */ FX_VIBRATO, /* Vibrato */
    /* 5 */ FX_TONE_VSLIDE_MOD, /* Toneporta + volume slide (MOD) */
    /* 6 */ FX_VIBRA_VSLIDE_MOD, /* Vibrato + volume slide (MOD) */
    /* 7 */ FX_TREMOLO, /* Tremolo */
    /* 8 */ FX_SETPAN, /* Set panning (S3M) */
    /* 9 */ FX_OFFSET, /* Sample offset */
    /* A */ FX_VOLSLIDE_MOD, /* Volume slide (MOD) */
    /* B */ FX_JUMP, /* Position jump */
    /* C */ FX_VOLSET, /* Set volume */
    /* D */ FX_BREAK, /* Pattern break */
    /* E */ FX_EXTENDED, /* Extended effects (MOD) */
    /* F */ FX_SPEED, /* Set speed/tempo */
    /* G */ FX_GLOBALVOL, /* Set global volume */
    /* H */ FX_GVOL_SLIDE, /* Global volume slide */
    /* I */ FX_NONE,
    /* J */ FX_NONE,
    /* K */ FX_KEYOFF, /* Key off */
    /* L */ FX_ENVPOS, /* Set volume envelope position */
    /* M */ FX_NONE, /* Select MIDI controller */
    /* N */ FX_NONE,
    /* O */ FX_NONE,
    /* P */ FX_PANSLIDE, /* Panning slide */
    /* Q */ FX_NONE,
    /* R */ FX_MULTI_RETRIG, /* Retrig + volume slide */
    /* S */ FX_EXTENDED_IT, /* SA Set high sample offset */
    /* T */ FX_TREMOR, /* Tremor */
    /* U */ FX_NONE,
    /* V */ FX_NONE, /* Set MIDI controller value */
    /* W */ FX_NONE,
    /* X */ FX_XF_PORTA, /* Extra fine portamento */
    /* Y */ FX_NONE,
    /* Z */ FX_NONE,
    /* d */ FX_VOLSLIDE, /* Volume slide (S3M) */
    /* f */ FX_PORTA_UP, /* Portamento up (S3M) */
    /* e */ FX_PORTA_DN, /* Portamento down (S3M) */
    /* k */ FX_VIBRA_VSLIDE, /* Vibrato + volume slide (S3M) */
    /* a */ FX_S3M_SPEED, /* Set speed (S3M) */
];

/// Translation of `rtm_translate_effect()`.
fn rtm_translate_effect(fxt: &mut u8, fxp: &mut u8) {
    if *fxt as usize >= RTM_FX.len() {
        *fxt = 0;
        *fxp = 0;
        return;
    }

    *fxt = RTM_FX[*fxt as usize];
    match *fxt {
        FX_NONE => {
            *fxt = 0;
            *fxp = 0;
        }

        FX_SETPAN => {
            /* Set panning (S3M) */
            if *fxp == 0xa4 {
                *fxt = FX_SURROUND;
                *fxp = 1;
            } else {
                let p = (*fxp as i32) << 1;
                *fxp = p.min(255) as u8;
            }
        }

        FX_VOLSLIDE_MOD /* Volume slide (MOD) */
        | FX_TONE_VSLIDE_MOD /* Toneporta + volslide (MOD) */
        | FX_VIBRA_VSLIDE_MOD /* Vibrato + volslide (MOD) */ => {
            /* TODO: this format has very strange memory quirks that
             * are not emulated, see above. */
            *fxt &= 0x0f;
            /* Disable fine effects */
            if lsn(*fxp) != 0 && msn(*fxp) != 0 {
                *fxp &= 0xf0;
            }
        }

        FX_PORTA_UP_MOD /* Portamento up (MOD) */ | FX_PORTA_DN_MOD /* Portamento down (MOD) */ => {
            /* TODO: 1xx has separate memory from fxx,
             * 2xx has separate memory from exx.
             * Clamp down values that would be interpreted as fine.
             * Values this high are essentially indistinguishable. */
            *fxt &= 0x0f;
            *fxp = (*fxp).min(0xdf);
        }

        FX_EXTENDED => {
            /* Extended effects (MOD) */
            match msn(*fxp) {
                0x0 /* Not implemented */ | 0xf => {
                    *fxt = 0;
                    *fxp = 0;
                }
                EX_VIBRATO_WF /* Set vibrato control */ | EX_TREMOLO_WF /* Set tremolo control */ => {
                    /* TODO: 0=sine, 1=ramp-up, 2=ramp-down */
                }
                _ => {}
            }
        }

        FX_EXTENDED_IT => {
            /* Extended effects (IT) */
            match msn(*fxp) {
                0xa => {
                    /* Set high sample offset */
                    *fxt = FX_HIOFFSET;
                    *fxp = lsn(*fxp) as u8;
                }
                _ => {
                    /* Not implemented */
                    *fxt = 0;
                    *fxp = 0;
                }
            }
        }
        _ => {}
    }
}

/* Convert Real Tracker -64..64 pan values into xmp 0..255 pan values. */
/// Translation of `rtm_convert_pan()`.
fn rtm_convert_pan(pan: i8) -> i32 {
    let mut v = (pan as i32 * 2) + 0x80;
    clamp(&mut v, 0, 255);
    v
}

/// Translation of `rtm_convert_envelope_flags()`.
fn rtm_convert_envelope_flags(flags: u16) -> i32 {
    /* RTM envelope flags are XM-compatible */
    (flags & (RTENV_FLAG_ENABLE | RTENV_FLAG_SUSTAIN | RTENV_FLAG_LOOP)) as i32
}

/// Translation of `read_object_header()`.
fn read_object_header(f: &mut HioHandle<'_>, h: &mut ObjectHeader, id: &[u8; 4]) -> i32 {
    hio_read(&mut h.id, 4, 1, f);

    if *id != h.id {
        return -1;
    }

    h.rc = hio_read8(f);
    if h.rc != 0x20 {
        return -1;
    }
    if hio_read(&mut h.name, 1, 32, f) != 32 {
        return -1;
    }
    h.eof = hio_read8(f);
    h.version = hio_read16l(f);
    h.header_size = hio_read16l(f);

    0
}

/// Reads an envelope of `struct RTINHeader`.
fn read_envelope(env: &mut Envelope, f: &mut HioHandle<'_>) -> i32 {
    env.npoint = hio_read8(f);

    /* Sanity check */
    if env.npoint >= 12 {
        return -1;
    }

    for p in env.point.iter_mut() {
        p.x = hio_read32l(f) as i32;
        p.y = hio_read32l(f) as i32;
    }
    env.sustain = hio_read8(f);
    env.loopstart = hio_read8(f);
    env.loopend = hio_read8(f);
    env.flags = hio_read16l(f);
    0
}

/// Translation of `rtm_load()`.
fn rtm_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut oh = ObjectHeader::default();
    let mut rh = RtmmHeader::default();
    let mut rp = RtndHeader::default();
    let mut ri = RtinHeader {
        nsample: 0,
        flags: 0,
        table: [0; 120],
        volume_env: Envelope::default(),
        panning_env: Envelope::default(),
        vibflg: 0,
        vibsweep: 0,
        vibdepth: 0,
        vibrate: 0,
        volfade: 0,
        midi_port: 0,
        midi_channel: 0,
        midi_program: 0,
        midi_enable: 0,
        midi_transpose: 0,
        midi_bender_range: 0,
        midi_base_volume: 0,
        midi_use_velocity: 0,
    };
    let mut rs = RtsmHeader::default();
    let mut tracker_name = [0u8; 21];
    let mut composer = [0u8; 33];

    if read_object_header(f, &mut oh, b"RTMM") < 0 {
        return -1;
    }

    let version = oh.version as i32;

    hio_read(&mut tracker_name, 1, 20, f);
    tracker_name[20] = 0;
    hio_read(&mut composer, 1, 32, f);
    composer[32] = 0;
    let _ = &composer; /* strncpy(m->author, composer, XMP_NAME_SIZE); */
    rh.flags = hio_read16l(f);
    rh.ntrack = hio_read8(f);
    rh.ninstr = hio_read8(f);
    rh.nposition = hio_read16l(f);
    rh.npattern = hio_read16l(f);
    rh.speed = hio_read8(f);
    rh.tempo = hio_read8(f);
    hio_read(&mut rh.panning, 32, 1, f);
    rh.extra_data_size = hio_read32l(f);

    /* Sanity check */
    if hio_error(f) != 0 || rh.nposition > 255 || rh.ntrack > 32 || rh.npattern > 255 {
        return -1;
    }

    if version >= 0x0112 {
        hio_seek(f, 32, SEEK_CUR); /* skip original name */
    }

    let mod_ = &mut m.mod_;
    for i in 0..rh.nposition as usize {
        mod_.xxo[i] = hio_read16l(f) as u8;
        if mod_.xxo[i] as u16 >= rh.npattern {
            return -1;
        }
    }

    strncpy(&mut mod_.name, &oh.name, 32);
    mod_.name[32] = b'\0';
    let mut type_ = cstr(&tracker_name).to_vec();
    type_.extend_from_slice(format!(" RTM {:x}.{:02x}", version >> 8, version & 0xff).as_bytes());
    snprintf_bytes(&mut mod_.type_, XMP_NAME_SIZE, &type_);
    /* strncpy(m->author, composer, XMP_NAME_SIZE); */

    mod_.len = rh.nposition as i32;
    mod_.pat = rh.npattern as i32;
    mod_.chn = rh.ntrack as i32;
    mod_.trk = mod_.chn * mod_.pat;
    mod_.ins = rh.ninstr as i32;
    mod_.spd = rh.speed as i32;
    mod_.bpm = rh.tempo as i32;

    m.c4rate = C4_NTSC_RATE;
    m.period_type = if rh.flags & RTMM_FLAG_LINEAR_TABLE != 0 {
        PERIOD_LINEAR
    } else {
        PERIOD_AMIGA
    };

    let mod_ = &mut m.mod_;
    for i in 0..mod_.chn as usize {
        mod_.xxc[i].pan = rtm_convert_pan(rh.panning[i] as i8);
    }

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* (the header sizes are added as unsigned 32 bit numbers) */
    let mut offset = 42u32
        .wrapping_add(oh.header_size as u32)
        .wrapping_add(rh.extra_data_size) as i32;

    for i in 0..mod_.pat {
        hio_seek(f, start.wrapping_add(offset) as i64, SEEK_SET);

        if read_object_header(f, &mut oh, b"RTND") < 0 {
            return -1;
        }

        rp.flags = hio_read16l(f);
        rp.ntrack = hio_read8(f);
        rp.nrows = hio_read16l(f);
        rp.datasize = hio_read32l(f);

        /* Sanity check */
        if rp.ntrack > rh.ntrack || rp.nrows > 999 {
            return -1;
        }

        offset = (offset as u32)
            .wrapping_add(42)
            .wrapping_add(oh.header_size as u32)
            .wrapping_add(rp.datasize) as i32;

        if libxmp_alloc_pattern_tracks_long(mod_, i, rp.nrows as i32) < 0 {
            return -1;
        }

        for r in 0..rp.nrows as i32 {
            let mut j: i32 = 0;
            loop {
                let c = hio_read8(f);
                if c == 0 {
                    /* next row */
                    break;
                }

                /* Sanity check */
                if j >= rp.ntrack as i32 {
                    return -1;
                }

                if c & 0x01 != 0 {
                    /* set track */
                    j = hio_read8(f) as i32;

                    /* Sanity check */
                    if j >= rp.ntrack as i32 {
                        return -1;
                    }
                }
                let event = mod_.event_mut(i, j, r);
                if c & 0x02 != 0 {
                    /* read note */
                    event.note = hio_read8(f).wrapping_add(1);
                    if event.note == 0xff {
                        event.note = XMP_KEY_OFF;
                    } else {
                        event.note = event.note.wrapping_add(12);
                    }
                }
                if c & 0x04 != 0 {
                    /* read instrument */
                    event.ins = hio_read8(f);
                }
                if c & 0x08 != 0 {
                    /* read effect */
                    event.fxt = hio_read8(f);
                }
                if c & 0x10 != 0 {
                    /* read parameter */
                    event.fxp = hio_read8(f);
                }
                if c & 0x20 != 0 {
                    /* read effect 2 */
                    event.f2t = hio_read8(f);
                }
                if c & 0x40 != 0 {
                    /* read parameter 2 */
                    event.f2p = hio_read8(f);
                }

                if c & 0x18 != 0 {
                    rtm_translate_effect(&mut event.fxt, &mut event.fxp);
                }
                if c & 0x60 != 0 {
                    rtm_translate_effect(&mut event.f2t, &mut event.f2p);
                }
                j += 1;
            }
        }
    }

    /*
     * load instruments
     */

    hio_seek(f, start.wrapping_add(offset) as i64, SEEK_SET);

    /* ESTIMATED value! We don't know the actual value at this point */
    mod_.smp = MAX_SAMP;

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mut smpnum = 0;
    for i in 0..m.mod_.ins {
        let iu = i as usize;

        if read_object_header(f, &mut oh, b"RTIN") < 0 {
            return -1;
        }

        libxmp_instrument_name(&mut m.mod_, i, &oh.name, 32);

        if oh.header_size == 0 {
            ri.nsample = 0;
            continue;
        }

        ri.nsample = hio_read8(f);
        ri.flags = hio_read16l(f);
        if hio_read(&mut ri.table, 1, 120, f) != 120 {
            return -1;
        }

        if read_envelope(&mut ri.volume_env, f) < 0 {
            return -1;
        }
        if read_envelope(&mut ri.panning_env, f) < 0 {
            return -1;
        }

        ri.vibflg = hio_read8(f) as i8;
        ri.vibsweep = hio_read8(f) as i8;
        ri.vibdepth = hio_read8(f) as i8;
        ri.vibrate = hio_read8(f) as i8;
        ri.volfade = hio_read16l(f);

        if version >= 0x0110 {
            ri.midi_port = hio_read8(f);
            ri.midi_channel = hio_read8(f);
            ri.midi_program = hio_read8(f);
            ri.midi_enable = hio_read8(f);
        }
        if version >= 0x0112 {
            ri.midi_transpose = hio_read8(f) as i8;
            ri.midi_bender_range = hio_read8(f);
            ri.midi_base_volume = hio_read8(f);
            ri.midi_use_velocity = hio_read8(f) as i8;
        }

        let volbase = m.volbase;
        let mod_ = &mut m.mod_;
        let xxi = &mut mod_.xxi[iu];
        xxi.nsm = ri.nsample as i32;
        xxi.vol = if !ri.flags & RTIN_FLAG_MUTE_SAMPLES != 0 {
            volbase
        } else {
            0
        };

        if xxi.nsm > 16 {
            xxi.nsm = 16;
        }

        let nsm = xxi.nsm;
        if libxmp_alloc_subinstrument(mod_, i, nsm) < 0 {
            return -1;
        }

        let xxi = &mut mod_.xxi[iu];
        for j in 0..120 {
            xxi.map[j].ins = ri.table[j];
        }

        /* Envelope */
        xxi.rls = ri.volfade as i32;
        xxi.aei.npt = ri.volume_env.npoint as i32;
        xxi.aei.sus = ri.volume_env.sustain as i32;
        xxi.aei.lps = ri.volume_env.loopstart as i32;
        xxi.aei.lpe = ri.volume_env.loopend as i32;
        xxi.aei.flg = rtm_convert_envelope_flags(ri.volume_env.flags);
        xxi.pei.npt = ri.panning_env.npoint as i32;
        xxi.pei.sus = ri.panning_env.sustain as i32;
        xxi.pei.lps = ri.panning_env.loopstart as i32;
        xxi.pei.lpe = ri.panning_env.loopend as i32;
        xxi.pei.flg = rtm_convert_envelope_flags(ri.panning_env.flags);

        for j in 0..xxi.aei.npt as usize {
            xxi.aei.data[j * 2] = ri.volume_env.point[j].x as i16;
            xxi.aei.data[j * 2 + 1] = (ri.volume_env.point[j].y / 2) as i16;
        }
        for j in 0..xxi.pei.npt as usize {
            xxi.pei.data[j * 2] = ri.panning_env.point[j].x as i16;
            xxi.pei.data[j * 2 + 1] = (32 + ri.panning_env.point[j].y / 2) as i16;
        }

        /* For each sample */
        for j in 0..nsm as usize {
            let mut flags = 0;

            if read_object_header(f, &mut oh, b"RTSM") < 0 {
                return -1;
            }

            rs.flags = hio_read16l(f);
            rs.basevolume = hio_read8(f);
            rs.defaultvolume = hio_read8(f);
            rs.length = hio_read32l(f);
            rs.loop_ = hio_read32l(f) as u8;
            rs.loopbegin = hio_read32l(f);
            rs.loopend = hio_read32l(f);
            rs.basefreq = hio_read32l(f);
            rs.basenote = hio_read8(f);
            rs.panning = hio_read8(f) as i8;

            let sub = &mut m.mod_.xxi[iu].sub[j];
            libxmp_c2spd_to_note(rs.basefreq as i32, &mut sub.xpo, &mut sub.fin);
            sub.xpo += 48 - rs.basenote as i32;
            sub.vol = rs.defaultvolume as i32;
            sub.gvl = rs.basevolume as i32;
            sub.pan = if ri.flags & RTIN_FLAG_DEFAULT_PAN != 0 {
                rtm_convert_pan(rs.panning)
            } else {
                -1
            };
            /* Autovibrato oddities:
             * Wave:  TODO: 0 sine, 1 square, 2 ramp down, 3 ramp up
             *        All invalid values are also ramp up.
             * Depth: the UI limits it 0-15, but higher values
             *        work. Negatives are very broken.
             * Rate:  the UI limits 0-63; but higher and negative
             *        values actually work how you would expect!
             *        Rate is half as fast as libxmp currently.
             * Sweep: the UI limits 0-255 but loads as signed.
             *        During playback, it is treated as unsigned.
             */
            sub.vwf = (ri.vibflg as u8).min(3) as i32;
            sub.vde = (ri.vibdepth as i32).max(0) << 2;
            sub.vra = (ri.vibrate as i32 + (ri.vibrate > 0) as i32) >> 1;
            sub.vsw = ri.vibsweep as u8 as i32;
            sub.sid = smpnum;

            if smpnum >= m.mod_.smp {
                let n = m.mod_.smp * 3 / 2;
                if libxmp_realloc_samples(m, n) < 0 {
                    return -1;
                }
            }
            let xxs = &mut m.mod_.xxs[smpnum as usize];

            libxmp_copy_adjust(&mut xxs.name, &oh.name, 31);

            xxs.len = rs.length as i32;
            xxs.lps = rs.loopbegin as i32;
            xxs.lpe = rs.loopend as i32;

            if rs.flags & RTSM_FLAG_16_BIT != 0 {
                xxs.flg |= XMP_SAMPLE_16BIT;
                xxs.len >>= 1;
                xxs.lps >>= 1;
                xxs.lpe >>= 1;
            }
            if rs.flags & RTSM_FLAG_DELTA_CODING != 0 {
                flags |= SAMPLE_FLAG_DIFF;
            }
            /* Any non-zero enables looping */
            if rs.loop_ >= RTSM_LOOP_ON {
                xxs.flg |= XMP_SAMPLE_LOOP;
            }
            /* Only RTSM_LOOP_BIDIR enables; only works for GUS? */
            if rs.loop_ == RTSM_LOOP_BIDIR {
                xxs.flg |= XMP_SAMPLE_LOOP_BIDIR;
            }

            let smpctl = m.smpctl;
            if libxmp_load_sample(
                smpctl,
                Some(f),
                flags,
                &mut m.mod_.xxs[smpnum as usize],
                None,
            ) < 0
            {
                return -1;
            }
            smpnum += 1;
        }
    }

    /* Final sample number adjustment */
    if libxmp_realloc_samples(m, smpnum) < 0 {
        return -1;
    }

    m.quirk |= QUIRK_FINEFX | QUIRK_INSVOL;
    m.read_event_type = READ_EVENT_FT2;

    0
}
