// Rust translation of src/loaders/dt_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* TODO: Digital Home Studio features (SV19 extensions, IENV, 2.06 format, etc).
 * TODO: Digital Home Studio DTM 2.1 test; 2.03, 2.04, 1.9, 2.1 effects tests.
 * TODO: libxmp needs a new pan law for modules from 2.04 until 1.9.
 * TODO: libxmp does not support track or pattern names.
 * TODO: libxmp does not support SV19 fractional BPM.
 * TODO: libxmp does not support horrible DTM stereo hacks (see below).
 * TODO: libxmp can't tell 2.04 and pre-VERS commercial modules apart.
 * TODO: were external samples ever implemented?
 */

//! The Digital Tracker (DTM) loader.

use super::super::common::*;
use super::super::dataio::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_c2spd_to_note;
use super::super::*;
use super::common::*;
use super::iff::*;
use super::sample::libxmp_load_sample;
use super::*;

const MAGIC_D_T_: u32 = magic4(b'D', b'.', b'T', b'.');

/* Values to compare against version_derived for compatibility.
 * These don't directly correspond to real values in the format,
 * they were just picked so the "commercial" formats with VERS would
 * have higher values than those determined by the pattern format.
 * Note that commercial versions 1.0 through 1.9 come after 2.04, and
 * versions prior to 2.015 did not support the DTM format. VERS and
 * SV19 are not emitted up through at least version DT 1.901, so most
 * of the commercial versions will have their modules IDed as 2.04.
 * Later versions (1.914 and 1.917 confirmed) are aware of VERS and SV19.
 *
 * Note that there's a later Digital Tracker release series for BeOS
 * using a completely new format that this loader doesn't support.
 */
const DTM_V2015: u32 = 2015; /* Digital Tracker 2.015 and 2.02 */
const DTM_V203: u32 = 2030; /* Digital Tracker 2.03 */
const DTM_V204: u32 = 2040; /* Digital Tracker 2.04 thru 1.9 */
const DTM_V19: u32 = 19 << 8; /* Digital Tracker 1.9 (later vers) */
const DTM_V21: u32 = 21 << 8; /* Digital Home Studio */

/* Pattern format versions, which don't correspond directly to file
 * format versions. Versions 2.015 thru 2.03 have all zero bytes here and
 * store patterns using the Protracker format; 2.04 through 1.9 use ASCII
 * "2.04" and store note numbers instead of periods; Digital Home Studio
 * uses an ASCII "2.06" and stores completely unpacked pattern fields.
 */
const FORMAT_MOD: u32 = 0;
const FORMAT_V204: u32 = magic4(b'2', b'.', b'0', b'4');
const FORMAT_V206: u32 = magic4(b'2', b'.', b'0', b'6');

/* "Mono" mode has always been panoramic stereo, but this wasn't clearly
 * communicated in the documentation or UI until Digital Home Studio. */
const DTM_OLD_STEREO: i32 = 0x00;
const DTM_PANORAMIC_STEREO: i32 = 0xff;

pub(crate) const LIBXMP_LOADER_DT: FormatLoader = FormatLoader {
    name: "Digital Tracker",
    test: dt_test,
    loader: dt_load,
};

/// Translation of `dt_test()`.
fn dt_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    if hio_read32b(f) != MAGIC_D_T_ {
        return -1;
    }

    let mut size = hio_read32b(f); /* chunk size */
    hio_read16b(f); /* type */
    hio_read16b(f); /* stereo mode; global depth (pre-2.04) */
    hio_read16b(f); /* reserved */
    hio_read16b(f); /* tempo */
    hio_read16b(f); /* bpm */
    hio_read32b(f); /* global sample rate (pre-2.04) */

    clamp(&mut size, 14, XMP_NAME_SIZE as u32 + 14);
    size -= 14;
    libxmp_read_title(f, t, size as i32);

    0
}

/// Translation of `struct local_data`.
#[derive(Default)]
struct LocalData {
    vers_flag: bool,
    patt_flag: bool,
    sv19_flag: bool,
    pflag: bool,
    sflag: bool,
    stereo: i32, /* 0=old stereo, ff=panoramic stereo (>=2.04) */
    depth: i32,  /* global sample depth used by pre-2.04 modules */
    c2spd: i32,  /* global sample rate used by pre-2.04 modules */
    version: u32,
    version_derived: u32,
    format: u32,
    realpat: i32,
    last_pat: i32,
    insnum: i32,
    chn: i32,

    patbuf: Vec<u8>,
}

/// Translation of `dtm_translate_effect()`.
fn dtm_translate_effect(event: &mut XmpEvent, data: &LocalData) {
    match event.fxt {
        0x1 /* portamento up */
        | 0x2 /* portamento down */
        | 0x3 /* tone portamento */
        | 0x4 /* vibrato */
        | 0x5 /* tone portamento + volslide */
        | 0x6 /* vibrato + volslide */
        | 0x7 /* tremolo */
        | 0x9 /* offset */
        | 0xa /* volslide */
        | 0xc /* set volume */ => {
            /* Protracker compatible */
        }

        0x0 => {
            /* arpeggio */
            /* DT beta through 2.04: does nothing. */
            if data.version_derived <= DTM_V203 {
                event.fxp = 0;
            }
        }

        0x8 => {
            /* set panning */
            /* DT 2.04+: only supported in panoramic stereo mode. */
            if data.version_derived >= DTM_V204 && data.stereo == DTM_PANORAMIC_STEREO {
                /* DT 1.9 and up have 800 as full right and 8FF as
                 * full left. 2.04 through 1.1 use the high nibble to
                 * control the channel's left mix value (0:low, F:high)
                 * and the low nibble control the channel's right mix
                 * level. 0 is not completely silent. The default
                 * setting is 15 (full) for each channel.
                 *
                 * For the older behavior, see:
                 *   Bitmaps/no happy end !!!!!!.dtm
                 *   Tyan/fruchtix 1997.dtm
                 */
                if data.version_derived >= DTM_V19 {
                    event.fxp ^= 0xff;
                } else {
                    /* TODO: solve DT's old pan law into libxmp's.
                     * would be nice to have pan law support instead.
                     *
                     * L = 0x80 * (left + 1) = vol * (0x80 - pan)
                     * R = 0x80 * (right + 1) = vol * (0x80 + pan)
                     **/
                    let left = msn(event.fxp);
                    let right = lsn(event.fxp);
                    let pan = 0x80 * (right - left) / (left + right + 2);
                    let vol = if right > left {
                        0x80 * ((right + 1) << 2) / (0x80 + pan)
                    } else if left > right {
                        0x80 * ((left + 1) << 2) / (0x80 - pan)
                    } else {
                        (left + 1) << 2
                    };
                    event.fxt = FX_SETPAN;
                    event.fxp = (pan + 0x80) as u8;
                    event.f2t = FX_TRK_VOL;
                    event.f2p = vol as u8;
                }
            } else {
                event.fxt = 0;
                event.fxp = 0;
            }
        }

        0xb => {
            /* position jump */
            if data.version_derived <= DTM_V203 && data.chn > 4 {
                /* DT 2.04 and under: in 6 and 8 channel modules,
                 * the break position is somewhere in the middle of
                 * a pattern unless followed by a Dxx. This can also
                 * do things like jump to position "255".
                 * FIXME: try to more closely approximate this? */
                event.f2t = FX_BREAK;
                event.f2p = 0x32;
            } else if data.version_derived == DTM_V204 {
                /* DT commercial 1.0 through 1.2: effect is off-by-one;
                 * B01 will jump to 0, B02 will jump to 1, etc.
                 * This works as expected from 1.901 onward.
                 * TODO: does anything rely on the broken version?
                 */
                if event.fxp > 0 {
                    event.fxp -= 1;
                }
            }
        }

        0xd => {
            /* pattern break */
            /* DT beta through commercial 1.2: parameter is ignored.
             * This works as expected from 1.901 onward.
             * TODO: does anything rely on the broken version?
             */
            if data.version_derived <= DTM_V203 {
                event.fxp = 0;
            }
        }

        0xf => {
            /* set speed/tempo */
            /* DT beta: all parameters set speed, 0 acts like 1
             * DT 1.01 thru 2.04: 0 and 20h act like speed 1 (no clamp)
             * DT commercial 1.0: 0 and 20h act like speed 1 (clamped to 66)
             * DT commercial 1.1: 0 and 20h act like speed 1 (no clamp)
             * DT commercial 1.2: 0 is ignored, 20h is a BPM (clamped to 66)
             * DT commercial 1.901: 0 is ignored, 20h is a BPM (no clamp)
             * The 66 BPM clamp in some versions seems to be to avoid DSP
             * bugs with slow BPMs. Whatever issue was fixed by 1.901.
             * TODO: what does anything actually rely on, if at all?
             */
            if data.version_derived <= DTM_V203 && (event.fxp == 0 || event.fxp == 0x20) {
                event.fxp = 1;
            }
        }

        0xe => {
            match msn(event.fxp) {
                0x1 /* fine portamento up */
                | 0x2 /* fine portamento down */
                | 0x3 /* glissando control */
                | 0x6 /* pattern loop */
                | 0xa /* fine volslide up */
                | 0xb /* fine volslide down */
                | 0xc /* note cut */
                | 0xd /* note delay */
                | 0xe /* pattern delay */ => {
                    /* Protracker compatible */
                }

                0x4 => {
                    /* vibrato waveform */
                    /* DT 2.04 and up: sine rampup(period) square square */
                    if (event.fxp & 3) == 3 {
                        event.fxp -= 1;
                    }
                    /* Continuous variants are missing from all versions. */
                    event.fxp &= 3;
                }

                0x5 => {
                    /* set finetune */
                    /* DT 2.04: only works if a note is present */
                    if event.note == 0 {
                        event.fxt = 0;
                        event.fxp = 0;
                    }
                }

                /* 0x8: set panning */
                /* Missing from all versions except commercial 1.1,
                 * where it seems to be a global volume effect? */
                /* fall-through */
                _ => {
                    event.fxt = 0;
                    event.fxp = 0;
                }
            }
        }

        _ => {
            event.fxt = 0;
            event.fxp = 0;
        }
    }
}

/// Translation of `dtm_event_size()`.
fn dtm_event_size(format: u32) -> i32 {
    match format {
        FORMAT_MOD | FORMAT_V204 => 4,
        FORMAT_V206 => 6,
        _ => -1,
    }
}

/// Translation of `dtm_translate_event()`.
fn dtm_translate_event(event: &mut XmpEvent, in_: &[u8], data: &LocalData) {
    match data.format {
        FORMAT_MOD => {
            libxmp_decode_protracker_event(event, in_);
        }

        FORMAT_V204 => {
            if in_[0] > 0 && in_[0] <= 0x7c {
                event.note = 12 * (in_[0] >> 4) + (in_[0] & 0x0f) + 12;
            }
            event.vol = (in_[1] & 0xfc) >> 2;
            event.ins = ((in_[1] & 0x03) << 4) + (in_[2] >> 4);
            event.fxt = in_[2] & 0xf;
            event.fxp = in_[3];
        }

        FORMAT_V206 => { /* FIXME Digital Home Studio */ }
        _ => {}
    }

    dtm_translate_effect(event, data);
}

/// Translation of `get_d_t_()`.
fn get_d_t_(m: &mut ModuleData, size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mut name_len = size.wrapping_sub(14);

    if !(14..=142).contains(&size) {
        return -1;
    }

    let mod_ = &mut m.mod_;
    hio_read16b(f); /* type */
    data.stereo = hio_read8(f) as i32; /* 0:old stereo, ff:panoramic stereo */
    data.depth = hio_read8(f) as i32; /* global sample bit depth (pre-2.04) */
    hio_read16b(f); /* reserved */
    mod_.spd = hio_read16b(f) as i32;
    let b = hio_read16b(f) as i32;
    if b > 0 {
        /* RAMBO.DTM has bpm 0 */
        mod_.bpm = b; /* Not clamped by Digital Tracker. */
    }
    data.c2spd = hio_read32b(f) as i32; /* global sample rate (pre-2.04) */

    if data.stereo != DTM_OLD_STEREO && data.stereo != DTM_PANORAMIC_STEREO {
        data.stereo = DTM_OLD_STEREO;
    }

    /* Global sample depth is applied to all samples pre-2.04.
     * Later Digital Tracker versions incorrectly ignore this field when
     * importing pre-2.04 modules.
     *
     * DT 2.015 and 2.02 will save a value of 0 in this field.
     */
    if data.depth != 0 && data.depth != 8 && data.depth != 16 {
        data.depth = 8;
    }

    /* Only known used values for global sample rate are 8400 and 24585,
     * but 12292 and 49170 are also supported and referenced in the UI.
     * 2.04 will replace the sample rates with whatever was in this field.
     * The only values that do not crash here besides 8400 are Atari Falcon
     * hardware frequencies:
     *
     * f = floor((25175040 >> 8) / div) = floor(98340 / div)
     */
    match data.c2spd {
        49170 | 24585 | 12292 | 8400 => {}
        _ => {
            data.c2spd = 8400;
        }
    }

    name_len = name_len.min(XMP_NAME_SIZE as u32);
    // Note (upstream): a name of XMP_NAME_SIZE bytes fills the name
    // without its terminator, which C's string functions then read past;
    // here it is read within the name's array.
    hio_read(&mut mod_.name, name_len as usize, 1, f);
    libxmp_set_type(m, b"Digital Tracker DTM");

    0
}

/// Translation of `get_s_q_()`.
fn get_s_q_(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, _data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;

    /* Sanity check */
    if mod_.pat != 0 {
        return -1;
    }

    mod_.len = hio_read16b(f) as i32;
    mod_.rst = hio_read16b(f) as i32;

    /* Sanity check */
    if mod_.len > 256 || mod_.rst > 255 {
        return -1;
    }

    hio_read32b(f); /* reserved */

    let mut maxpat = 0;
    for i in 0..128 {
        mod_.xxo[i] = hio_read8(f);
        if mod_.xxo[i] as i32 > maxpat {
            maxpat = mod_.xxo[i] as i32;
        }
    }
    mod_.pat = maxpat + 1;

    0
}

/// Translation of `get_vers()`.
fn get_vers(_m: &mut ModuleData, size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    if data.vers_flag || size < 4 {
        return 0;
    }
    data.vers_flag = true;

    data.version = hio_read32b(f);
    0
}

/// Translation of `get_sv19()`.
fn get_sv19(m: &mut ModuleData, size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;
    let mut buf = [0u8; 32 * 2];

    if !data.vers_flag || data.version < 19 || size < 86 {
        /* ignore in the extreme off chance a module with this exists. */
        return 0;
    }
    data.sv19_flag = true;

    hio_read16b(f); /* ticks per beat, by default 24 */
    let bpm_frac = hio_read32b(f); /* initial BPM (fractional portion) */

    /* Round up to nearest for now. */
    if bpm_frac >= 0x80000000 {
        mod_.bpm += 1;
    }

    /* Initial pan table. 0=left 90=center 180=right
     * Do not load in old stereo mode for now. */
    if hio_read(&mut buf, 1, 64, f) < 64 {
        return -1;
    }
    for i in 0..32 {
        let val = readmem16b(&buf[2 * i..]) as i32;
        if val <= 180 && data.stereo == DTM_PANORAMIC_STEREO {
            mod_.xxc[i].pan = val * 0x100 / 180;
        }
    }

    /* Instrument type table.
     * The file format claims 32 bytes but it's only 16.
     * It's not clear how to influence this table, if possible. */

    0
}

/// Translation of `get_patt()`.
fn get_patt(m: &mut ModuleData, size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mut ver = [0u8; 4];

    /* Sanity check */
    if data.patt_flag || data.pflag || size < 8 {
        return -1;
    }
    data.patt_flag = true;

    let mod_ = &mut m.mod_;
    mod_.chn = hio_read16b(f) as i32;
    data.chn = mod_.chn;
    data.realpat = hio_read16b(f) as i32;
    if hio_read(&mut ver, 1, 4, f) < 4 {
        return -1;
    }
    data.format = readmem32b(&ver);
    mod_.trk = mod_.chn * mod_.pat;

    if data.format != FORMAT_MOD {
        data.version_derived = if data.format == FORMAT_V206 {
            DTM_V21
        } else {
            DTM_V204
        };
    } else {
        /* DTM 2.015/2.02 have depth=0 and 31 instruments instead of 63.
         * There are also modules with depth!=0 but 31 instruments,
         * and it's not clear what the origin of those is. */
        if data.depth != 0 {
            data.version_derived = DTM_V203;
        } else {
            data.version_derived = DTM_V2015;
        }
    }

    if data.vers_flag && data.version != 0 {
        data.version_derived = data.version << 8;
        m.flow_mode = FLOW_MODE_DTM_19;
    } else {
        /* 6/8 channel mode's position jump bugs also affect its
         * interactions with pattern loop. (Very weakly) attempt
         * to support the bugs. */
        m.flow_mode = if data.version_derived <= DTM_V204 && m.mod_.chn > 4 {
            FLOW_MODE_DTM_2015_6CH
        } else {
            FLOW_MODE_DTM_2015
        };
    }

    /* Sanity check */
    if m.mod_.chn > XMP_MAX_CHANNELS as i32 {
        return -1;
    }

    if dtm_event_size(data.format) < 0 {
        return -1;
    }

    0
}

/// Translation of `get_inst()`.
fn get_inst(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mut buf = [0u8; 50];

    /* Sanity check */
    if m.mod_.ins != 0 {
        return -1;
    }

    m.mod_.ins = hio_read16b(f) as i32;
    m.mod_.smp = m.mod_.ins;

    /* Sanity check */
    if m.mod_.ins > MAX_INSTRUMENTS {
        return -1;
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

        if hio_read(&mut buf, 1, 50, f) < 50 {
            return -1;
        }

        /* hio_read32b(f); (reserved) */
        let mut len = readmem32b(&buf[4..]) as i32;
        if len < 0 {
            len = 0;
        }
        mod_.xxs[iu].len = len;
        mod_.xxi[iu].nsm = (mod_.xxs[iu].len > 0) as i32;
        let fine = (buf[8] << 4) as i8 as i32; /* finetune */
        mod_.xxi[iu].sub[0].vol = buf[9] as i32;
        mod_.xxi[iu].sub[0].pan = XMP_INST_NO_DEFAULT_PAN;
        let mut repstart = readmem32b(&buf[10..]) as i32;
        let mut replen = readmem32b(&buf[14..]) as i32;

        if repstart >= len || repstart < 0 || replen < 0 {
            repstart = 0;
            replen = 0;
        }
        if replen > len - repstart {
            replen = len - repstart;
        }
        mod_.xxs[iu].lps = repstart;
        mod_.xxs[iu].lpe = repstart + replen;

        libxmp_instrument_name(mod_, i, &buf[18..], 22);

        /* DT pre-2.04: global sample depth is used for playback; the
         * sample depth field seems to be for the resampler only.
         * See Lot/5th-2.dtm, which relies on this.
         */
        if data.version_derived <= DTM_V203 {
            buf[41] = if data.depth != 0 { data.depth as u8 } else { 8 };
        }

        let stereo = buf[40]; /* stereo */
        let xxs = &mut mod_.xxs[iu];
        if buf[41] > 8 {
            /* resolution (8, 16, or rarely 0) */
            xxs.flg |= XMP_SAMPLE_16BIT;
            xxs.len >>= 1;
            xxs.lps >>= 1;
            xxs.lpe >>= 1;
        }

        /* 2.04 through 1.1: panoramic mode forces stereo samples to be
         * interpreted as mono. This bug was fixed in 1.901 or another
         * commercial version, so this can't really be checked, and
         * there's no reason to do this anyway. */
        /*
        if (data->stereo == DTM_PANORAMIC_STEREO &&
            data->version_derived == DTM_V204) {
            stereo = 0;
        }
        */

        if stereo != 0 {
            /* TODO: in old stereo mode, stereo samples do something
             * unusual: the left channel is sent to the first
             * channel of the "pair" it is played in, and the
             * right channel is sent to the second channel of
             * the "pair" (a "pair" is channels 1&2, 3&4, etc.).
             * If the stereo sample is played in the first channel,
             * it overrides whatever is played by the second, but
             * if it is played in the second channel, the first
             * channel can override it (or more often is silent).
             * Note that for 1&2, 5&6, etc, the left sample will
             * pan right and the right sample will be pan left.
             *
             * The worst part of this is that each channel of the
             * stereo sample is subject to the pitch and volume OF
             * THE CHANNEL IT IS PLAYING IN. This probably needs a
             * custom read_event and some mixer hacks.
             */
            xxs.flg |= XMP_SAMPLE_STEREO;
            xxs.len >>= 1;
            xxs.lps >>= 1;
            xxs.lpe >>= 1;
        }
        if xxs.lpe - xxs.lps > 1 {
            xxs.flg |= XMP_SAMPLE_LOOP;
        }

        let midinote = readmem16b(&buf[42..]) as i32; /* midi note */
        /* hio_read16b(f); (unknown (0x0000)) */
        let mut c2spd = readmem32b(&buf[46..]) as i32; /* frequency */

        /* DT pre-2.04: the sample rate is used for resampling only(?)
         * and the global sample rate is used for playback instead.
         * This field was removed from 2.04. */
        if data.version_derived <= DTM_V203 {
            c2spd = data.c2spd;
        }
        let sub = &mut mod_.xxi[iu].sub[0];
        libxmp_c2spd_to_note(c2spd, &mut sub.xpo, &mut sub.fin);

        /* DT 2.03 through commercial 1.2: midi note resamples
         * the sample and changes the rate in some cases, but does
         * nothing during normal playback.
         * DT 1.901+: changing the midi note acts as a transpose
         * during playback and does not modify the sample at all.
         * Values under C-2 are clamped, over B-7 are ignored.
         */
        if data.version_derived >= DTM_V19 && midinote < 95 {
            sub.xpo += 48 - midinote.max(24);
        }

        /* It's strange that we have both c2spd and finetune */
        sub.fin += fine;

        sub.sid = i;
    }

    0
}

/// Translation of `get_dapt()`.
fn get_dapt(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;

    if !data.patt_flag {
        return -1;
    }

    if !data.pflag {
        data.pflag = true;
        data.last_pat = 0;

        if libxmp_init_pattern(mod_) < 0 {
            return -1;
        }
    }

    hio_read32b(f); /* 0xffffffff */
    let pat = hio_read16b(f) as i32;
    let rows = hio_read16b(f) as i32;

    /* Sanity check */
    /* DT 2.04, all known versions of 1.9x: maximum rows is 96.
     * These can load/resave larger patterns made with a hex editor,
     * but with a buggy UI. It isn't clear how stable these are. */
    /* DHS: maximum configurable by interface is 4*99 = 396.
     * TODO: it uses a different format for this entire chunk. */
    if pat < 0
        || pat >= mod_.pat
        || rows < 0
        || (data.version_derived <= DTM_V19 && rows > 96)
        || (data.version_derived > DTM_V19 && rows > 396)
    {
        return -1;
    }
    if pat < data.last_pat {
        return -1;
    }

    for i in data.last_pat..=pat {
        if libxmp_alloc_pattern_tracks(mod_, i, rows) < 0 {
            return -1;
        }
    }
    data.last_pat = pat + 1;

    /* (Re)allocate the pattern buffer. */
    let event_size = dtm_event_size(data.format) as usize;
    let total_size = event_size * rows as usize * mod_.chn as usize;
    if data.patbuf.len() < total_size {
        let add = total_size - data.patbuf.len();
        if data.patbuf.try_reserve_exact(add).is_err() {
            return -1;
        }
        data.patbuf.resize(total_size, 0);
    }
    if hio_read(&mut data.patbuf[..total_size], 1, total_size, f) < total_size {
        return -1;
    }

    let mut pos = 0;
    for j in 0..rows {
        for k in 0..mod_.chn {
            let event = mod_.event_mut(pat, k, j);
            dtm_translate_event(event, &data.patbuf[pos..pos + event_size], data);
            pos += event_size;
        }
    }

    0
}

/// Translation of `get_dait()`.
fn get_dait(m: &mut ModuleData, size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    if !data.sflag {
        data.sflag = true;
        data.insnum = 0;
    }

    if size > 2 {
        /* Sanity check */
        if data.insnum >= m.mod_.ins {
            return -1;
        }

        let sid = m.mod_.xxi[data.insnum as usize].sub[0].sid as usize;
        let smpctl = m.smpctl;
        let ret = libxmp_load_sample(
            smpctl,
            Some(f),
            SAMPLE_FLAG_BIGEND | SAMPLE_FLAG_INTERLEAVED,
            &mut m.mod_.xxs[sid],
            None,
        );

        if ret < 0 {
            return -1;
        }
    }

    data.insnum += 1;

    0
}

/// Translation of `get_text()`.
fn get_text(m: &mut ModuleData, size: u32, f: &mut HioHandle<'_>, _data: &mut LocalData) -> i32 {
    if size < 12 || (size & 1) != 0 || m.comment.is_some() {
        return 0;
    }
    /*type	= */
    hio_read16b(f);
    let len = hio_read32b(f);
    /*tab	= */
    hio_read16b(f);
    /*res	= */
    hio_read16b(f);

    /* "=$FFFF <=> length is odd" */
    let skip_byte = hio_read16b(f) == 0xffff;

    if len != size.wrapping_sub(12).wrapping_sub(skip_byte as u32) {
        return 0;
    }
    if len == 0 {
        return 0;
    }
    if len as i64 > hio_size(f) - hio_tell(f) {
        return 0;
    }

    if let Ok(mut comment) = try_vec(len as usize + 1, 0u8) {
        if skip_byte {
            hio_read8(f);
        }
        let len = hio_read(&mut comment, 1, len as usize, f);
        comment[len] = b'\0';
        m.comment = Some(comment);
    }
    0
}

/// Translation of `dt_load()`.
fn dt_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut data = LocalData::default();

    let mut handle = libxmp_iff_new::<LocalData>();

    m.c4rate = C4_NTSC_RATE;

    /* IFF chunk IDs */
    let mut ret = libxmp_iff_register(&mut handle, b"D.T.", get_d_t_);
    ret |= libxmp_iff_register(&mut handle, b"S.Q.", get_s_q_);
    ret |= libxmp_iff_register(&mut handle, b"VERS", get_vers);
    ret |= libxmp_iff_register(&mut handle, b"PATT", get_patt);
    ret |= libxmp_iff_register(&mut handle, b"INST", get_inst);
    ret |= libxmp_iff_register(&mut handle, b"SV19", get_sv19);
    ret |= libxmp_iff_register(&mut handle, b"DAPT", get_dapt);
    ret |= libxmp_iff_register(&mut handle, b"DAIT", get_dait);
    ret |= libxmp_iff_register(&mut handle, b"TEXT", get_text);

    if ret != 0 {
        return -1;
    }

    /* Load IFF chunks */
    let ret = libxmp_iff_load(&handle, m, f, &mut data);
    libxmp_iff_release(handle);
    data.patbuf = Vec::new();
    if ret < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    /* alloc remaining patterns */
    if mod_.xxp.is_some() {
        for i in data.last_pat..mod_.pat {
            if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
                return -1;
            }
        }
    }

    /* Correct the module type now that the version fields are known. */
    let s = if data.version >= 20 {
        format!(
            "Digital Home Studio DTM {}.{}",
            data.version / 10,
            data.version % 10
        )
    } else if data.version != 0 {
        format!(
            "Digital Tracker {}.{} DTM",
            data.version / 10,
            data.version % 10
        )
    } else if data.format == FORMAT_V204 {
        "Digital Tracker 2.04 DTM".into()
    } else if data.depth != 0 {
        "Digital Tracker 2.03 DTM".into()
    } else {
        "Digital Tracker 2.015 DTM".into()
    };
    libxmp_set_type(m, s.as_bytes());

    if data.version_derived >= DTM_V204 && data.stereo == DTM_PANORAMIC_STEREO {
        /* Panoramic stereo mode: all channels default to center.
         * In 1.9xx, the SV19 chunk is used to specify initial values,
         * so this should be skipped if it was loaded.
         */
        if !data.sv19_flag {
            let mod_ = &mut m.mod_;
            for i in 0..mod_.chn as usize {
                mod_.xxc[i].pan = 0x80;
            }
        }
    }

    0
}
