// Rust translation of src/loaders/masi_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/*
 * Originally based on the PSM loader from Modplug by Olivier Lapicque and
 * fixed comparing the One Must Fall! PSMs with Kenny Chou's MTM files.
 */

/*
 * From EPICTEST Readme.1st:
 *
 * The Music And Sound Interface, MASI, is the basis behind all new Epic
 * games. MASI uses its own proprietary file format, PSM, for storing
 * its music.
 */

/*
 * kode54's comment on Sinaria PSMs in the foo_dumb hydrogenaudio forum:
 *
 * "The Sinaria variant uses eight character pattern and instrument IDs,
 * the sample headers are laid out slightly different, and the patterns
 * use a different format for the note values, and also different effect
 * scales for certain commands.
 *
 * [Epic] PSM uses high nibble for octave and low nibble for note, for
 * a valid range up to 0x7F, for a range of D-1 through D#9 compared to
 * IT. (...) Sinaria PSM uses plain note values, from 1 - 83, for a
 * range of C-3 through B-9.
 *
 * [Epic] PSM also uses an effect scale for portamento, volume slides,
 * and vibrato that is about four times as sensitive as the IT equivalents.
 * Sinaria does not. This seems to coincide with the MOD/S3M to PSM
 * converter that Joshua Jensen released in the EPICTEST.ZIP file which
 * can still be found on a few FTP sites. It converted effects literally,
 * even though the bundled players behaved as the libraries used with
 * Epic's games did and made the effects sound too strong."
 */

/*
 * Claudio's note: Sinaria seems to have a finetune byte just before
 * volume and some kind of (stereo?) interleaved sample, with 16-byte
 * frames (see Sinaria songs 5 and 8). Sinaria song 10 still sounds
 * ugly, maybe caused by finetune issues?
 */

//! The Epic MegaGames MASI (PSM) loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_c2spd_to_note;
use super::super::*;
use super::common::*;
use super::iff::*;
use super::sample::libxmp_load_sample;
use super::*;

const MAGIC_PSM_: u32 = magic4(b'P', b'S', b'M', b' ');
const MAGIC_FILE: u32 = magic4(b'F', b'I', b'L', b'E');
const MAGIC_TITL: u32 = magic4(b'T', b'I', b'T', b'L');
const MAGIC_OPLH: u32 = magic4(b'O', b'P', b'L', b'H');
const MAGIC_PPAN: u32 = magic4(b'P', b'P', b'A', b'N');

pub(crate) const LIBXMP_LOADER_MASI: FormatLoader = FormatLoader {
    name: "Epic MegaGames MASI",
    test: masi_test,
    loader: masi_load,
};

/// Translation of `masi_test()`.
fn masi_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    if hio_read32b(f) != MAGIC_PSM_ {
        return -1;
    }

    hio_read8(f);
    hio_read8(f);
    hio_read8(f);
    if hio_read8(f) != 0 {
        return -1;
    }

    if hio_read32b(f) != MAGIC_FILE {
        return -1;
    }

    hio_read32b(f);
    let val = hio_read32l(f) as i32;
    hio_seek(f, val as i64, SEEK_CUR);

    if hio_read32b(f) == MAGIC_TITL {
        let val = hio_read32l(f) as i32;
        libxmp_read_title(f, t, val);
    } else {
        libxmp_read_title(f, t, 0);
    }

    0
}

/// Translation of `struct local_data`.
#[derive(Default)]
struct LocalData {
    sinaria: bool,
    cur_pat: i32,
    cur_ins: i32,
    pnam: Vec<u8>,
    pord: Vec<u8>,
}

/// Translation of `get_sdft()`.
fn get_sdft(_m: &mut ModuleData, _size: u32, _f: &mut HioHandle<'_>, _data: &mut LocalData) -> i32 {
    0
}

/// Translation of `get_titl()`.
fn get_titl(
    m: &mut ModuleData,
    mut size: u32,
    f: &mut HioHandle<'_>,
    _data: &mut LocalData,
) -> i32 {
    let mod_ = &mut m.mod_;
    let mut buf = [0u8; XMP_NAME_SIZE];

    size = if size > XMP_NAME_SIZE as u32 - 1 {
        XMP_NAME_SIZE as u32 - 1
    } else {
        size
    };
    let size = hio_read(&mut buf, 1, size as usize, f);
    strncpy(&mut mod_.name, &buf[..size], size);
    mod_.name[size] = b'\0';

    0
}

/// Translation of `get_dsmp_cnt()`.
fn get_dsmp_cnt(
    m: &mut ModuleData,
    _size: u32,
    _f: &mut HioHandle<'_>,
    _data: &mut LocalData,
) -> i32 {
    let mod_ = &mut m.mod_;

    mod_.ins += 1;
    mod_.smp = mod_.ins;

    0
}

/// Translation of `get_pbod_cnt()`.
fn get_pbod_cnt(
    m: &mut ModuleData,
    _size: u32,
    f: &mut HioHandle<'_>,
    data: &mut LocalData,
) -> i32 {
    let mod_ = &mut m.mod_;
    let mut buf = [0u8; 20];

    mod_.pat += 1;
    if hio_read(&mut buf, 1, 20, f) < 20 {
        return -1;
    }
    if buf[9] != 0 && buf[13] == 0 {
        data.sinaria = true;
    }

    0
}

/// Translation of `get_dsmp()`.
fn get_dsmp(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let flags = hio_read8(f); /* flags */
    hio_seek(f, 8, SEEK_CUR); /* songname */
    hio_seek(f, if data.sinaria { 8 } else { 4 }, SEEK_CUR); /* smpid */

    let i = data.cur_ins;
    /* (both passes see the same chunks, so this is one of the counted) */
    if i >= m.mod_.ins {
        return -1;
    }
    let iu = i as usize;
    let mod_ = &mut m.mod_;
    if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
        return -1;
    }

    hio_read(&mut mod_.xxi[iu].name, 1, 31, f);
    hio_seek(f, 8, SEEK_CUR);
    hio_read8(f); /* insno */
    hio_read8(f);
    let xxs = &mut mod_.xxs[iu];
    xxs.len = hio_read32l(f) as i32;
    xxs.lps = hio_read32l(f) as i32;
    xxs.lpe = hio_read32l(f) as i32;
    xxs.flg = if flags & 0x80 != 0 {
        XMP_SAMPLE_LOOP
    } else {
        0
    };
    hio_read16l(f);

    if xxs.lpe < 0 {
        xxs.lpe = 0;
    }

    if xxs.len > 0 {
        mod_.xxi[iu].nsm = 1;
    }

    let mut finetune = 0;
    if data.sinaria {
        finetune = (hio_read8(f) << 4) as i8 as i32;
    }

    let sub = &mut mod_.xxi[iu].sub[0];
    sub.vol = hio_read8(f) as i32 / 2 + 1;
    hio_read32l(f);
    sub.pan = XMP_INST_NO_DEFAULT_PAN;
    sub.sid = i;
    let srate = hio_read16l(f) as i32;

    libxmp_c2spd_to_note(srate, &mut sub.xpo, &mut sub.fin);
    sub.fin += finetune;

    hio_seek(f, 16, SEEK_CUR);
    let smpctl = m.smpctl;
    if libxmp_load_sample(
        smpctl,
        Some(f),
        SAMPLE_FLAG_8BDIFF,
        &mut m.mod_.xxs[iu],
        None,
    ) < 0
    {
        return -1;
    }

    data.cur_ins += 1;

    0
}

/// Translation of `convert_porta()`.
fn convert_porta(param: u8, sinaria: bool) -> u8 {
    if sinaria {
        return param;
    }

    if param < 4 {
        param | 0xf0
    } else {
        param >> 2
    }
}

/// Translation of `get_pbod()`.
fn get_pbod(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;
    let mut dummy = XmpEvent::default();

    let i = data.cur_pat;
    let n = if data.sinaria { 8 } else { 4 };

    /*len =*/
    hio_read32l(f);
    /* (upstream writes the name before checking the pattern number, which
     * the pattern allocation below does) */
    if i < 0 || i >= mod_.pat {
        return -1;
    }
    let p = i as usize * 8;
    hio_read(&mut data.pnam[p..p + n], 1, n, f);

    let rows = hio_read16l(f) as i32;
    if hio_error(f) != 0 {
        return -1;
    }

    if libxmp_alloc_pattern_tracks(mod_, i, rows) < 0 {
        return -1;
    }

    let mut r = 0;

    loop {
        let mut rowlen = hio_read16l(f) as i32 - 2;
        if hio_error(f) != 0 {
            return -1;
        }
        while rowlen > 0 {
            let flag = hio_read8(f);

            if rowlen == 1 {
                break;
            }

            let chan = hio_read8(f);
            rowlen -= 2;

            let event = if (chan as i32) < mod_.chn {
                mod_.event_mut(i, chan as i32, r)
            } else {
                &mut dummy
            };

            if flag & 0x80 != 0 {
                let mut note = hio_read8(f);
                rowlen -= 1;
                if data.sinaria {
                    note = note.wrapping_add(36);
                } else {
                    note = ((note as i32 >> 4) * 12 + (note as i32 & 0x0f) + 1 + 12) as u8;
                }
                event.note = note;
            }

            if flag & 0x40 != 0 {
                event.ins = hio_read8(f).wrapping_add(1);
                rowlen -= 1;
            }

            if flag & 0x20 != 0 {
                event.vol = hio_read8(f) / 2 + 1;
                rowlen -= 1;
            }

            if flag & 0x10 != 0 {
                let mut fxt = hio_read8(f);
                let mut fxp = hio_read8(f);
                rowlen -= 2;

                /* (upstream keeps a conversion of compressed events here
                 * under #if 0) */

                match fxt {
                    /* Volume slide */
                    0x01 => {
                        /* fine volslide up */
                        fxt = FX_EXTENDED;
                        fxp = ((EX_F_VSLIDE_UP << 4) as u8) | ((fxp / 2) & 0x0f);
                    }
                    0x02 => {
                        /* volslide up */
                        fxt = FX_VOLSLIDE;
                        fxp = (fxp / 2) << 4;
                    }
                    0x03 => {
                        /* fine volslide down */
                        fxt = FX_EXTENDED;
                        fxp = ((EX_F_VSLIDE_DN << 4) as u8) | ((fxp / 2) & 0x0f);
                    }
                    0x04 => {
                        /* volslide down */
                        fxt = FX_VOLSLIDE;
                        fxp /= 2;
                    }

                    /* Portamento */
                    0x0b => {
                        /* fine portamento up */
                        fxt = FX_PORTA_UP;
                        fxp = ((EX_F_PORTA_UP << 4) as u8) | convert_porta(fxp, data.sinaria);
                    }
                    0x0c => {
                        /* portamento up */
                        fxt = FX_PORTA_UP;
                        fxp = convert_porta(fxp, data.sinaria);
                    }
                    0x0d => {
                        /* fine portamento up */
                        fxt = FX_PORTA_DN;
                        fxp = ((EX_F_PORTA_DN << 4) as u8) | convert_porta(fxp, data.sinaria);
                    }
                    0x0e => {
                        /* portamento down */
                        fxt = FX_PORTA_DN;
                        fxp = convert_porta(fxp, data.sinaria);
                    }
                    0x0f => {
                        /* tone portamento */
                        fxt = FX_TONEPORTA;
                        fxp >>= 2;
                    }
                    0x10 => {
                        /* toneporta + vslide up */
                        fxt = FX_TONE_VSLIDE;
                        // FIXME (upstream): the parameter is taken from the
                        // new effect type instead of the parameter.
                        fxp = fxt & 0xf0;
                    }
                    0x11 => {
                        /* glissando */
                        fxt = FX_EXTENDED;
                        fxp = ((EX_GLISS << 4) as u8) | (fxp & 0x0f);
                    }
                    0x12 => {
                        /* toneporta + vslide down */
                        fxt = FX_TONE_VSLIDE;
                        fxp >>= 4;
                    }

                    /* 0x13: S3M S: crashes MASI */

                    /* Vibrato */
                    0x15 => {
                        /* vibrato */
                        fxt = if data.sinaria {
                            FX_VIBRATO
                        } else {
                            FX_FINE_VIBRATO
                        };
                        /* fxp remains the same */
                    }
                    0x16 => {
                        /* vibrato waveform */
                        fxt = FX_EXTENDED;
                        fxp = ((EX_VIBRATO_WF << 4) as u8) | (fxp & 0x0f);
                    }
                    0x17 => {
                        /* vibrato + vslide up */
                        fxt = FX_VIBRA_VSLIDE;
                        fxp >>= 4;
                    }
                    0x18 => {
                        /* vibrato + vslide down */
                        fxt = FX_VIBRA_VSLIDE;
                        fxp &= 0x0f;
                    }

                    /* Tremolo */
                    0x1f => {
                        /* tremolo */
                        fxt = FX_TREMOLO;
                        /* fxp remains the same */
                    }
                    0x20 => {
                        /* tremolo waveform */
                        fxt = FX_EXTENDED;
                        fxp = ((EX_TREMOLO_WF << 4) as u8) | (fxp & 0x0f);
                    }

                    /* Sample commands */
                    0x29 => {
                        /* 3-byte offset */
                        fxt = FX_OFFSET;
                        /* use only the middle byte */
                        fxp = hio_read8(f);
                        hio_read8(f);
                        rowlen -= 2;
                    }
                    0x2a => {
                        /* retrig note */
                        fxt = FX_EXTENDED;
                        fxp = ((EX_RETRIG << 4) as u8) | (fxp & 0x0f);
                    }
                    0x2b => {
                        /* note cut */
                        fxt = FX_EXTENDED;
                        fxp = ((EX_CUT << 4) as u8) | (fxp & 0x0f);
                    }
                    0x2c => {
                        /* note delay */
                        fxt = FX_EXTENDED;
                        fxp = ((EX_DELAY << 4) as u8) | (fxp & 0x0f);
                    }

                    /* Position change */
                    0x33 => {
                        /* position jump */
                        /* not used in MASI */
                        fxt = FX_JUMP;
                        fxp >>= 1;
                        hio_read8(f);
                        rowlen -= 1;
                    }
                    0x34 => {
                        /* pattern break */
                        /* not used in MASI */
                        fxt = FX_BREAK;
                    }
                    0x35 => {
                        /* pattern loop */
                        fxt = FX_EXTENDED;
                        fxp = ((EX_PATTERN_LOOP << 4) as u8) | (fxp & 0x0f);
                    }
                    0x36 => {
                        /* pattern delay */
                        fxt = FX_EXTENDED;
                        fxp = ((EX_PATT_DELAY << 4) as u8) | (fxp & 0x0f);
                    }

                    /* Speed change */
                    0x3d => {
                        /* speed */
                        fxt = FX_SPEED;
                    }
                    0x3e => {
                        /* tempo */
                        fxt = FX_SPEED;
                    }

                    /* Other */
                    0x47 => {
                        /* arpeggio */
                        fxt = FX_S3M_ARPEGGIO;
                    }
                    0x48 => {
                        /* set finetune */
                        fxt = FX_EXTENDED;
                        fxp = ((EX_FINETUNE << 4) as u8) | (fxp & 0x0f);
                    }
                    0x49 => {
                        /* set pan */
                        fxt = FX_SETPAN;
                        fxp <<= 4;
                    }

                    _ => {
                        fxt = 0;
                        fxp = 0;
                    }
                }

                event.fxt = fxt;
                event.fxp = fxp;
            }
        }
        r += 1;
        if r >= rows {
            break;
        }
    }

    data.cur_pat += 1;

    0
}

/// Translation of `get_song()`.
fn get_song(m: &mut ModuleData, _size: u32, f: &mut HioHandle<'_>, _data: &mut LocalData) -> i32 {
    let mod_ = &mut m.mod_;

    hio_seek(f, 10, SEEK_CUR);
    mod_.chn = hio_read8(f) as i32;

    0
}

/// Translation of `subchunk_oplh()`.
fn subchunk_oplh(
    m: &mut ModuleData,
    size: u32,
    f: &mut HioHandle<'_>,
    data: &mut LocalData,
) -> i32 {
    let mod_ = &mut m.mod_;
    let mut first_order_chunk = i32::MAX;
    let mut size = size as i32;

    /* First two bytes = Number of chunks that follow */
    let num_chunk = hio_read16l(f) as i32;

    /* Sub sub chunks */
    let mut i = 0;
    while i < num_chunk && size > 0 {
        let opcode = hio_read8(f);

        size -= 1;

        if opcode == 0 {
            /* last sub sub chunk */
            break;
        }

        /* Saga Musix's note in OpenMPT:
         *
         * "This is more like a playlist than a collection of global
         *  values. In theory, a tempo item inbetween two order items
         *  should modify the tempo when switching patterns. No module
         *  uses this feature in practice though, so we can keep our
         *  loader simple. Unimplemented opcodes do nothing or freeze
         *  MASI."
         */
        match opcode {
            0x01 => {
                /* Play order list item */
                if mod_.len >= XMP_MAX_MOD_LENGTH as i32 {
                    return -1;
                }
                let n = if data.sinaria { 8 } else { 4 };
                let p = mod_.len as usize * 8;
                hio_read(&mut data.pord[p..p + n], 1, n, f);
                size -= n as i32;
                mod_.len += 1;
                if first_order_chunk == i32::MAX {
                    first_order_chunk = i;
                }
            }

            /* 0x02: Play range */
            /* 0x03: Jump loop */
            0x04 => {
                /* Jump line (restart position) */
                let restart_chunk = hio_read16l(f) as i32;
                size -= 2;

                /* This jumps to the command line, but since we're converting
                 * play order list items to our order list, only change the
                 * restart position if it's after the first order chunk.
                 */

                if restart_chunk >= first_order_chunk {
                    mod_.rst = restart_chunk - first_order_chunk;
                }
            }

            /* 0x05: Channel flip */
            /* 0x06: Transpose */
            0x07 => {
                /* Default speed */
                mod_.spd = hio_read8(f) as i32;
                size -= 1;
            }
            0x08 => {
                /* Default tempo */
                mod_.bpm = hio_read8(f) as i32;
                size -= 1;
            }
            0x0c => {
                /* Sample map table */
                hio_read16l(f);
                hio_read16l(f);
                hio_read16l(f);
                size -= 6;
            }
            0x0d => {
                /* Channel panning table */
                let chn = hio_read8(f) as usize;
                let pan = hio_read8(f) as i32;
                let type_ = hio_read8(f);

                if chn < XMP_MAX_CHANNELS {
                    let xxc = &mut mod_.xxc[chn];

                    size -= 3;

                    match type_ {
                        0 => {
                            /* use panning */
                            xxc.pan = pan ^ 0x80;
                        }
                        2 => {
                            /* surround */
                            xxc.pan = 0x80;
                            xxc.flg |= XMP_CHANNEL_SURROUND;
                        }
                        4 => {
                            /* center */
                            xxc.pan = 0x80;
                        }
                        _ => {}
                    }
                }
            }
            0x0e => {
                /* Channel volume table */
                let chn = hio_read8(f) as usize;
                let vol = hio_read8(f) as i32;

                if chn < XMP_MAX_CHANNELS {
                    let xxc = &mut mod_.xxc[chn];

                    size -= 2;

                    xxc.vol = (vol >> 2) + 1;
                }
            }
            _ => {
                /*printf("channel %d: %02x %02x\n", i, c, hio_read8(f));*/
                return -1;
            }
        }
        i += 1;
    }

    0
}

/* Sinaria channel panning table */
/// Translation of `subchunk_ppan()`.
fn subchunk_ppan(
    m: &mut ModuleData,
    size: u32,
    f: &mut HioHandle<'_>,
    _data: &mut LocalData,
) -> i32 {
    let mod_ = &mut m.mod_;
    let mut size = size as i32;

    let mut i = 0;
    while i < XMP_MAX_CHANNELS && size > 0 {
        let xxc = &mut mod_.xxc[i];
        let type_ = hio_read8(f);
        let pan = hio_read8(f) as i32;

        size -= 2;

        match type_ {
            0 => {
                /* use panning */
                xxc.pan = pan ^ 0x80;
            }
            2 => {
                /* surround */
                xxc.pan = 0x80;
                xxc.flg |= XMP_CHANNEL_SURROUND;
            }
            4 => {
                /* center */
                xxc.pan = 0x80;
            }
            _ => {}
        }
        i += 1;
    }

    0
}

/* Subchunk loader based on OpenMPT LoadPSM.cpp */
/// Translation of `get_song_2()`.
fn get_song_2(m: &mut ModuleData, size: u32, f: &mut HioHandle<'_>, data: &mut LocalData) -> i32 {
    let mut buf = [0u8; 20];
    let mut size = size as i32;

    hio_read(&mut buf, 1, 9, f);
    hio_read16l(f);
    size -= 11;

    /* Iterate over subchunks. We want OPLH and PPAN */
    while size > 0 {
        let magic = hio_read32b(f);
        let subchunk_size = hio_read32l(f) as i32;
        size -= 8;

        if subchunk_size <= 0 || hio_error(f) != 0 {
            return -1;
        }

        size = size.wrapping_sub(subchunk_size);

        match magic {
            MAGIC_OPLH => {
                if subchunk_oplh(m, subchunk_size as u32, f, data) < 0 {
                    return -1;
                }
            }

            MAGIC_PPAN => {
                if subchunk_ppan(m, subchunk_size as u32, f, data) < 0 {
                    return -1;
                }
            }

            _ => {
                hio_seek(f, subchunk_size as i64, SEEK_CUR);
            }
        }
    }

    0
}

/// Translation of `masi_load()`.
fn masi_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut data = LocalData::default();

    hio_read32b(f);

    data.sinaria = false;
    m.mod_.name[0] = 0;

    hio_seek(f, 8, SEEK_CUR); /* skip file size and FILE */
    m.mod_.ins = 0;
    m.mod_.smp = 0;
    data.cur_pat = 0;
    data.cur_ins = 0;
    let offset = hio_tell(f) as i32;

    let mut handle = libxmp_iff_new::<LocalData>();

    /* IFF chunk IDs */
    let mut ret = libxmp_iff_register(&mut handle, b"TITL", get_titl);
    ret |= libxmp_iff_register(&mut handle, b"SDFT", get_sdft);
    ret |= libxmp_iff_register(&mut handle, b"SONG", get_song);
    ret |= libxmp_iff_register(&mut handle, b"DSMP", get_dsmp_cnt);
    ret |= libxmp_iff_register(&mut handle, b"PBOD", get_pbod_cnt);

    if ret != 0 {
        return -1;
    }

    libxmp_iff_set_quirk(&mut handle, IFF_LITTLE_ENDIAN);

    /* Load IFF chunks */
    if libxmp_iff_load(&handle, m, f, &mut data) < 0 {
        libxmp_iff_release(handle);
        return -1;
    }

    libxmp_iff_release(handle);

    let mod_ = &mut m.mod_;
    mod_.trk = mod_.pat * mod_.chn;
    let Ok(pnam) = try_vec(mod_.pat as usize * 8, 0u8) else {
        /* pattern names */
        return -1;
    };
    data.pnam = pnam;

    let Ok(pord) = try_vec(XMP_MAX_MOD_LENGTH * 8, 0u8) else {
        /* pattern orders */
        return -1;
    };
    data.pord = pord;

    libxmp_set_type(
        m,
        if data.sinaria {
            b"Sinaria PSM"
        } else {
            b"Epic MegaGames MASI PSM"
        },
    );

    m.c4rate = C4_NTSC_RATE;

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    if libxmp_init_pattern(&mut m.mod_) < 0 {
        return -1;
    }

    hio_seek(f, start as i64 + offset as i64, SEEK_SET);

    m.mod_.len = 0;

    let mut handle = libxmp_iff_new::<LocalData>();

    /* IFF chunk IDs */
    let mut ret = libxmp_iff_register(&mut handle, b"SONG", get_song_2);
    ret |= libxmp_iff_register(&mut handle, b"DSMP", get_dsmp);
    ret |= libxmp_iff_register(&mut handle, b"PBOD", get_pbod);

    if ret != 0 {
        return -1;
    }

    libxmp_iff_set_quirk(&mut handle, IFF_LITTLE_ENDIAN);

    /* Load IFF chunks */
    if libxmp_iff_load(&handle, m, f, &mut data) < 0 {
        libxmp_iff_release(handle);
        return -1;
    }

    libxmp_iff_release(handle);

    let mod_ = &mut m.mod_;
    let n = if data.sinaria { 8 } else { 4 };
    for i in 0..mod_.len as usize {
        let mut j = 0;
        while j < mod_.pat {
            let p = j as usize * 8;
            if data.pord[i * 8..i * 8 + n] == data.pnam[p..p + n] {
                mod_.xxo[i] = j as u8;
                break;
            }
            j += 1;
        }

        if j == mod_.pat {
            break;
        }
    }

    0
}
