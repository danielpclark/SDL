// Rust translation of src/loaders/amf_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* AMF loader written based on the format specs by Miodrag Vallat with
 * fixes by Andre Timmermans.
 *
 * The AMF format is the internal format used by DSMI, the DOS Sound and Music
 * Interface, which is the engine of DMP. As DMP was able to play more and more
 * module formats, the format evolved to support more features. There were 5
 * official formats, numbered from 10 (AMF 1.0) to 14 (AMF 1.4).
 */

//! The DSMI Advanced Module Format (AMF) loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_c2spd_to_note;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

pub(crate) const LIBXMP_LOADER_AMF: FormatLoader = FormatLoader {
    name: "DSMI Advanced Module Format",
    test: amf_test,
    loader: amf_load,
};

/// Translation of `amf_test()`.
fn amf_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    let mut buf = [0u8; 4];

    if hio_read(&mut buf, 1, 3, f) < 3 {
        return -1;
    }

    if buf[0] != b'A' || buf[1] != b'M' || buf[2] != b'F' {
        return -1;
    }

    let ver = hio_read8(f);
    if (ver != 0x01 && ver < 0x08) || ver > 0x0e {
        return -1;
    }

    libxmp_read_title(f, t, 32);

    0
}

/// Translation of `amf_load()`.
fn amf_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut buf = [0u8; 1024];
    let mut no_loopend = false;

    hio_read(&mut buf, 1, 3, f);
    let ver = hio_read8(f) as i32;

    if hio_read(&mut buf, 1, 32, f) != 32 {
        return -1;
    }

    m.mod_.name[..32].copy_from_slice(&buf[..32]);
    m.mod_.name[32] = b'\0';
    let s = format!("DSMI {}.{} AMF", ver / 10, ver % 10);
    libxmp_set_type(m, s.as_bytes());

    let mod_ = &mut m.mod_;
    mod_.ins = hio_read8(f) as i32;
    mod_.len = hio_read8(f) as i32;
    mod_.trk = hio_read16l(f) as i32;
    mod_.chn = 4;

    if ver >= 0x09 {
        mod_.chn = hio_read8(f) as i32;
    }

    /* Sanity check */
    if mod_.ins == 0
        || mod_.len == 0
        || mod_.trk == 0
        || mod_.chn == 0
        || mod_.chn > XMP_MAX_CHANNELS as i32
    {
        return -1;
    }

    mod_.smp = mod_.ins;
    mod_.pat = mod_.len;

    if ver == 0x09 || ver == 0x0a {
        hio_read(&mut buf, 1, 16, f); /* channel remap table */
    }

    if ver >= 0x0b {
        let pan_len = if ver >= 0x0c { 32 } else { 16 };

        if hio_read(&mut buf, 1, pan_len, f) != pan_len {
            /* panning table */
            return -1;
        }

        for i in 0..pan_len {
            mod_.xxc[i].pan = 0x80 + 2 * buf[i] as i8 as i32;
        }
    }

    if ver >= 0x0d {
        mod_.bpm = hio_read8(f) as i32;
        mod_.spd = hio_read8(f) as i32;
    }

    m.c4rate = C4_NTSC_RATE;

    /* Orders */

    /*
     * Andre Timmermans <andre.timmermans@atos.net> says:
     *
     * Order table: track numbers in this table are not explained,
     * but as you noticed you have to perform -1 to obtain the index
     * in the track table. For value 0, found in some files, I think
     * it means an empty track.
     *
     * 2021 note: this is misleading. Do not subtract 1 from the logical
     * track values found in the order table; load the mapping table to
     * index 1 instead.
     */

    let mod_ = &mut m.mod_;
    for i in 0..mod_.len as usize {
        mod_.xxo[i] = i as u8;
    }

    let Ok(xxp) = try_vec(mod_.pat as usize, None) else {
        return -1;
    };
    mod_.xxp = Some(xxp);

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern(mod_, i) < 0 {
            return -1;
        }

        mod_.pattern_mut(i).rows = if ver >= 0x0e {
            hio_read16l(f) as i32
        } else {
            64
        };

        if mod_.pattern(i).rows > 256 {
            return -1;
        }

        for j in 0..mod_.chn as usize {
            let t = hio_read16l(f);
            mod_.pattern_mut(i).index[j] = t as i32;
        }
    }

    /* Instruments */

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    /* Probe for 2-byte loop start 1.0 format
     * in facing_n.amf and sweetdrm.amf have only the sample
     * loop start specified in 2 bytes
     *
     * These modules are an early variant of the AMF 1.0 format. Since
     * normal AMF 1.0 files have 32-bit lengths/loop start/loop end,
     * this is possibly caused by these fields having been expanded for
     * the 1.0 format, but M2AMF 1.3 writing instrument structs with
     * the old length (which would explain the missing 6 bytes).
     */
    if ver == 0x0a {
        let pos = hio_tell(f);
        if pos < 0 {
            return -1;
        }
        for _ in 0..m.mod_.ins {
            let b = hio_read8(f);
            if b != 0 && b != 1 {
                no_loopend = true;
                break;
            }
            hio_seek(f, 32 + 13, SEEK_CUR);
            if hio_read32l(f) > m.mod_.ins as u32 {
                /* check index */
                no_loopend = true;
                break;
            }
            let len = hio_read32l(f);
            if len > 0x100000 {
                /* check len */
                no_loopend = true;
                break;
            }
            if hio_read16l(f) == 0x0000 {
                /* check c2spd */
                no_loopend = true;
                break;
            }
            if hio_read8(f) > 0x40 {
                /* check volume */
                no_loopend = true;
                break;
            }
            let val = hio_read32l(f); /* check loop start */
            if val > len {
                no_loopend = true;
                break;
            }
            let val = hio_read32l(f); /* check loop end */
            if val > len {
                no_loopend = true;
                break;
            }
        }
        hio_seek(f, pos, SEEK_SET);
    }

    let mod_ = &mut m.mod_;
    for i in 0..mod_.ins {
        let iu = i as usize;

        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        hio_read8(f);

        hio_read(&mut buf, 1, 32, f);
        libxmp_instrument_name(mod_, i, &buf, 32);

        hio_read(&mut buf, 1, 13, f); /* sample name */
        hio_read32l(f); /* sample index */

        let xxi = &mut mod_.xxi[iu];
        xxi.nsm = 1;
        xxi.sub[0].sid = i;
        xxi.sub[0].pan = XMP_INST_NO_DEFAULT_PAN;

        let xxs = &mut mod_.xxs[iu];
        if ver >= 0x0a {
            xxs.len = hio_read32l(f) as i32;
        } else {
            xxs.len = hio_read16l(f) as i32;
        }
        let c2spd = hio_read16l(f) as i32;
        let sub = &mut xxi.sub[0];
        libxmp_c2spd_to_note(c2spd, &mut sub.xpo, &mut sub.fin);
        sub.vol = hio_read8(f) as i32;

        /*
         * Andre Timmermans <andre.timmermans@atos.net> says:
         *
         * [Miodrag Vallat's] doc tells that in version 1.0 only
         * sample loop start is present (2 bytes) but the files I
         * have tells both start and end are present (2*4 bytes).
         * Maybe it should be read as version < 1.0.
         *
         * CM: confirmed with Maelcum's "The tribal zone"
         */

        if no_loopend {
            xxs.lps = hio_read16l(f) as i32;
            xxs.lpe = xxs.len;
        } else if ver >= 0x0a {
            xxs.lps = hio_read32l(f) as i32;
            xxs.lpe = hio_read32l(f) as i32;
        } else {
            /* Non-looping samples are stored with lpe=-1, not 0. */
            xxs.lps = hio_read16l(f) as i32;
            xxs.lpe = hio_read16l(f) as i32;

            if xxs.lpe == 0xffff {
                xxs.lpe = 0;
            }
        }

        if no_loopend {
            xxs.flg = if xxs.lps > 0 { XMP_SAMPLE_LOOP } else { 0 };
        } else {
            xxs.flg = if xxs.lpe > xxs.lps {
                XMP_SAMPLE_LOOP
            } else {
                0
            };
        }
    }

    if hio_error(f) != 0 {
        return -1;
    }

    /* Tracks */

    /* Index 0 is a blank track that isn't stored in the file. To keep
     * things simple, load the mapping table to index 1 so the table
     * index is the same as the logical track value. Older versions
     * attempted to remap it to index 0 and subtract 1 from the index,
     * breaking modules that directly reference the empty track in the
     * order table (see "cosmos st.amf").
     */
    let Ok(mut trkmap) = try_vec(mod_.trk as usize + 1, 0i32) else {
        return -1;
    };
    let mut newtrk = 0;

    for i in 1..=mod_.trk as usize {
        /* read track table */
        let t = hio_read16l(f) as i32;
        trkmap[i] = t;
        if t > newtrk {
            newtrk = t;
        }
    }

    for i in 0..mod_.pat {
        /* read track table */
        for j in 0..mod_.chn as usize {
            let mut k = mod_.pattern(i).index[j] as u16;

            /* Use empty track if an invalid track is requested
             * (such as in Lasse Makkonen "faster and louder")
             */
            if k as i32 > mod_.trk {
                k = 0;
            }
            mod_.pattern_mut(i).index[j] = trkmap[k as usize];
        }
    }

    mod_.trk = newtrk + 1; /* + empty track */
    drop(trkmap);

    if hio_error(f) != 0 {
        return -1;
    }

    let Ok(xxt) = try_vec(mod_.trk as usize, None) else {
        return -1;
    };
    mod_.xxt = Some(xxt);

    /* Alloc track 0 as empty track */
    if libxmp_alloc_track(mod_, 0, 64) < 0 {
        return -1;
    }

    /* Alloc rest of the tracks */
    for i in 1..mod_.trk {
        if libxmp_alloc_track(mod_, i, 64) < 0 {
            /* FIXME! */
            return -1;
        }

        /* Previous versions loaded this as a 24-bit value, but it's
         * just a word. The purpose of the third byte is unknown, and
         * DSMI just ignores it.
         */
        let mut size = hio_read16l(f) as i32;
        hio_read8(f);

        if hio_error(f) != 0 {
            return -1;
        }

        /* Version 0.1 AMFs do not count the end-of-track marker in
         * the event count, so add 1. This hasn't been verified yet. */
        if ver == 0x01 && size != 0 {
            size += 1;
        }

        for j in 0..size {
            let t1 = hio_read8(f); /* row */
            let t2 = hio_read8(f); /* type */
            let mut t3 = hio_read8(f); /* parameter */

            if t1 == 0xff && t2 == 0xff && t3 == 0xff {
                break;
            }

            /* If an event is encountered past the end of the
             * track, treat it the same as the track end. This is
             * encountered in "Avoid.amf".
             */
            if t1 as i32 >= mod_.track(i).rows {
                if hio_seek(f, ((size - j - 1) * 3) as i64, SEEK_CUR) != 0 {
                    return -1;
                }

                break;
            }

            let event = &mut mod_.track_mut(i).event[t1 as usize];

            if t2 < 0x7f {
                /* note */
                if t2 > 0 {
                    event.note = t2 + 1;
                }
                /* A volume value of 0xff indicates that
                 * the old volume should be reused. Prior
                 * libxmp versions also forgot to add 1 here.
                 */
                event.vol = if t3 != 0xff { t3 + 1 } else { 0 };
            } else if t2 == 0x7f {
                /* note retrigger */

                /* AMF.TXT claims that this duplicates the
                 * previous event, which is a lie. M2AMF emits
                 * this for MODs when an instrument change
                 * occurs with no note, indicating it should
                 * retrigger (like in PT 2.3). Ignore this.
                 *
                 * See: "aladdin - aladd.pc.amf", "eye.amf".
                 */
            } else if t2 == 0x80 {
                /* instrument */
                event.ins = t3.wrapping_add(1);
            } else {
                /* effects */
                let mut fxp: u8 = 0;
                let mut fxt: u8 = 0;
                let s3 = t3 as i8;

                match t2 {
                    0x81 => {
                        fxt = FX_S3M_SPEED;
                        fxp = t3;
                    }
                    0x82 => {
                        if s3 > 0 {
                            fxt = FX_VOLSLIDE;
                            fxp = t3 << 4;
                        } else {
                            fxt = FX_VOLSLIDE;
                            fxp = (-(s3 as i32) & 0x0f) as u8;
                        }
                    }
                    0x83 => {
                        /* See volume notes above. Previous
                         * releases forgot to add 1 here. */
                        event.vol = t3.wrapping_add(1);
                    }
                    0x84 => {
                        /* AT: Not explained for 0x84, pitch
                         * slide, value 0x00 corresponds to
                         * S3M E00 and 0x80 stands for S3M F00
                         * (I checked with M2AMF)
                         */
                        if s3 >= 0 {
                            fxt = FX_PORTA_DN;
                            fxp = t3;
                        } else if t3 == 0x80 {
                            fxt = FX_PORTA_UP;
                            fxp = 0;
                        } else {
                            fxt = FX_PORTA_UP;
                            fxp = (-(s3 as i32)) as u8;
                        }
                    }
                    0x85 => { /* porta abs -- unknown */ }
                    0x86 => {
                        fxt = FX_TONEPORTA;
                        fxp = t3;
                    }

                    /* AT: M2AMF maps both tremolo and tremor to
                     * 0x87. Since tremor is only found in certain
                     * formats, maybe it would be better to
                     * consider it is a tremolo.
                     */
                    0x87 => {
                        fxt = FX_TREMOLO;
                        fxp = t3;
                    }
                    0x88 => {
                        fxt = FX_ARPEGGIO;
                        fxp = t3;
                    }
                    0x89 => {
                        fxt = FX_VIBRATO;
                        fxp = t3;
                    }
                    0x8a => {
                        if s3 > 0 {
                            fxt = FX_TONE_VSLIDE;
                            fxp = t3 << 4;
                        } else {
                            fxt = FX_TONE_VSLIDE;
                            fxp = (-(s3 as i32) & 0x0f) as u8;
                        }
                    }
                    0x8b => {
                        if s3 > 0 {
                            fxt = FX_VIBRA_VSLIDE;
                            fxp = t3 << 4;
                        } else {
                            fxt = FX_VIBRA_VSLIDE;
                            fxp = (-(s3 as i32) & 0x0f) as u8;
                        }
                    }
                    0x8c => {
                        fxt = FX_BREAK;
                        fxp = t3;
                    }
                    0x8d => {
                        fxt = FX_JUMP;
                        fxp = t3;
                    }
                    0x8e => { /* sync -- unknown */ }
                    0x8f => {
                        fxt = FX_EXTENDED;
                        fxp = ((EX_RETRIG << 4) as u8) | (t3 & 0x0f);
                    }
                    0x90 => {
                        fxt = FX_OFFSET;
                        fxp = t3;
                    }
                    0x91 => {
                        if s3 > 0 {
                            fxt = FX_EXTENDED;
                            fxp = ((EX_F_VSLIDE_UP << 4) as u8) | (t3 & 0x0f);
                        } else {
                            fxt = FX_EXTENDED;
                            fxp = ((EX_F_VSLIDE_DN << 4) as u8) | (t3 & 0x0f);
                        }
                    }
                    0x92 => {
                        // FIXME (upstream): the fine slide's parameter is
                        // taken from fxp (0) instead of t3.
                        if s3 > 0 {
                            fxt = FX_PORTA_DN;
                            fxp = 0xf0 | (fxp & 0x0f);
                        } else {
                            fxt = FX_PORTA_UP;
                            fxp = 0xf0 | (fxp & 0x0f);
                        }
                    }
                    0x93 => {
                        fxt = FX_EXTENDED;
                        fxp = ((EX_DELAY << 4) as u8) | (t3 & 0x0f);
                    }
                    0x94 => {
                        fxt = FX_EXTENDED;
                        fxp = ((EX_CUT << 4) as u8) | (t3 & 0x0f);
                    }
                    0x95 => {
                        fxt = FX_SPEED;
                        if t3 < 0x21 {
                            t3 = 0x21;
                        }
                        fxp = t3;
                    }
                    0x96 => {
                        // FIXME (upstream): as 0x92, the extra fine slide's
                        // parameter is taken from fxp (0).
                        if s3 > 0 {
                            fxt = FX_PORTA_DN;
                            fxp = 0xe0 | (fxp & 0x0f);
                        } else {
                            fxt = FX_PORTA_UP;
                            fxp = 0xe0 | (fxp & 0x0f);
                        }
                    }
                    0x97 => {
                        /* Same as S3M pan, but param is offset by -0x40. */
                        if t3 == 0x64 {
                            /* 0xA4 - 0x40 */
                            fxt = FX_SURROUND;
                            fxp = 1;
                        } else if t3 >= 0xC0 || t3 <= 0x40 {
                            let pan = (s3 as i32 * 2) + 0x80;
                            fxt = FX_SETPAN;
                            fxp = 0xff.min(pan) as u8;
                        }
                    }
                    _ => {}
                }

                event.fxt = fxt;
                event.fxp = fxp;
            }
        }
    }

    /* Samples */

    for i in 0..m.mod_.ins as usize {
        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), SAMPLE_FLAG_UNS, &mut m.mod_.xxs[i], None) < 0 {
            return -1;
        }
    }

    m.quirk |= QUIRK_FINEFX;

    0
}
