// Rust translation of src/loaders/abk_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// AMOS/STOS Music Bank Loader
// Copyright (C) 2014-2026 Stephen J Leary and Claudio Matsuoka
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The AMOS Music Bank loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::hio::*;
use super::super::period::libxmp_period_to_note;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

const AMOS_BANK: u32 = 0x416d426b;
const AMOS_MUSIC_TYPE: u16 = 0x0003;
const AMOS_MAIN_HEADER: u32 = 0x14;
const AMOS_STRING_LEN: usize = 0x10;
#[allow(dead_code)]
const AMOS_BASE_FREQ: i32 = 8192;
const AMOS_ABK_CHANNELS: usize = 4;
#[allow(dead_code)]
const ABK_HEADER_SECTION_COUNT: i32 = 3;

pub(crate) const LIBXMP_LOADER_ABK: FormatLoader = FormatLoader {
    name: "AMOS Music Bank",
    test: abk_test,
    loader: abk_load,
};

/// Translation of `struct abk_header`: represents the main ABK header.
#[derive(Default)]
struct AbkHeader {
    instruments_offset: u32,
    songs_offset: u32,
    patterns_offset: u32,
}

/// Translation of `struct abk_song`: represents a song in an ABK module.
#[derive(Default)]
struct AbkSong {
    playlist_offset: [u32; AMOS_ABK_CHANNELS],
    tempo: u16,
    song_name: [u8; AMOS_STRING_LEN],
}

/// Translation of `struct abk_playlist`: represents an ABK playlist.
#[derive(Default)]
struct AbkPlaylist {
    length: u16,
    pattern: Vec<u16>,
}

/// Translation of `struct abk_instrument`: represents an ABK instrument.
#[derive(Clone, Copy, Default)]
struct AbkInstrument {
    sample_offset: u32,
    sample_length: u32,

    repeat_offset: u32,
    repeat_end: u16,

    sample_volume: u16,

    sample_name: [u8; AMOS_STRING_LEN],
}

/// Translation of `read_abk_playlist()`: read the ABK playlist out from
/// the file stream. This method malloc's some memory for the playlist
/// and can realloc if the playlist is very long.
///
/// `f` is the stream to read from, `playlist_offset` the offset to the
/// playlist sections, and `playlist` is populated with the result.
fn read_abk_playlist(f: &mut HioHandle<'_>, playlist_offset: u32, playlist: &mut AbkPlaylist) {
    let mut arraysize = 64;

    /* clear the length */
    playlist.length = 0;

    /* move to the start of the songs data section. */
    hio_seek(f, playlist_offset as i64, SEEK_SET);

    playlist.pattern = vec![0; arraysize];

    let mut playdata = hio_read16b(f);

    while playdata != 0xFFFF && playdata != 0xFFFE {
        /* i hate doing a realloc in a loop
        but i'd rather not traverse the list twice.*/

        if playlist.length as usize >= arraysize {
            arraysize *= 2;
            playlist.pattern.resize(arraysize, 0);
        }

        playlist.pattern[playlist.length as usize] = playdata;
        playlist.length = playlist.length.wrapping_add(1);
        playdata = hio_read16b(f);
    }
}

/// Translation of `read_abk_song()`.
fn read_abk_song(f: &mut HioHandle<'_>, song: &mut AbkSong, songs_section_offset: u32) -> i32 {
    /* move to the start of the songs data section */
    hio_seek(f, songs_section_offset as i64, SEEK_SET);

    if hio_read16b(f) != 1 {
        /* we only support a single song.
         * in a an abk file for now */
        return -1;
    }

    let song_section = hio_read32b(f);

    if hio_seek(
        f,
        songs_section_offset.wrapping_add(song_section) as i64,
        SEEK_SET,
    ) < 0
    {
        return -1;
    }

    for i in 0..AMOS_ABK_CHANNELS {
        song.playlist_offset[i] = (hio_read16b(f) as u32)
            .wrapping_add(songs_section_offset)
            .wrapping_add(song_section);
    }

    song.tempo = hio_read16b(f);

    /* unused. just progress the file pointer forward */
    hio_read16b(f);

    if hio_read(&mut song.song_name, 1, AMOS_STRING_LEN, f) != AMOS_STRING_LEN {
        return -1;
    }

    0
}

/// Translation of `read_abk_pattern()`: reads an ABK pattern into a
/// `xmp_event` structure.
///
/// `f` is the stream to read data from, `events` the events object to
/// populate, and `pattern_offset_abs` the absolute file offset to the
/// start of the pattern to read. Returns the size of the pattern.
fn read_abk_pattern(
    f: &mut HioHandle<'_>,
    events: &mut [XmpEvent],
    pattern_offset_abs: u32,
) -> i32 {
    let mut inst: u8 = 0;
    let mut jumped = false;
    let mut per_command: u8 = 0;
    let mut per_param: u8 = 0;

    let storepos = hio_tell(f) as i32;
    if storepos < 0 {
        return -1;
    }

    /* count how many abk positions are used in this pattern */
    let mut position: u8 = 0;

    hio_seek(f, pattern_offset_abs as i64, SEEK_SET);

    /* read the first bit of pattern data */
    let mut patdata = hio_read16b(f);

    while patdata != 0x8000 && !jumped {
        if patdata == 0x9100 {
            break;
        }

        if patdata & 0x8000 != 0 {
            let command = ((patdata >> 8) & 0x7F) as u8;
            let param = (patdata & 0x007F) as u8;

            if command != 0x03
                && command != 0x09
                && command != 0x0b
                && command != 0x0c
                && command != 0x0d
                && command < 0x10
            {
                per_command = 0;
                per_param = 0;
            }

            let p = position as usize;
            match command {
                0x01 | 0x0e => {
                    /* portamento up */
                    events[p].fxt = FX_PORTA_UP;
                    events[p].fxp = param;
                }
                0x02 | 0x0f => {
                    /* portamento down */
                    events[p].fxt = FX_PORTA_DN;
                    events[p].fxp = param;
                }
                0x03 => {
                    /* set volume */
                    events[p].fxt = FX_VOLSET;
                    events[p].fxp = param;
                }
                0x04 => { /* stop effect */ }
                0x05 => {
                    /* repeat */
                    events[p].fxt = FX_EXTENDED;
                    if param == 0 {
                        events[p].fxp = 0x50;
                    } else {
                        events[p].fxp = 0x60 | (param & 0x0f);
                    }
                }
                0x06 => {
                    /* lowpass filter off */
                    events[p].fxt = FX_EXTENDED;
                    events[p].fxp = 0x00;
                }
                0x07 => {
                    /* lowpass filter on */
                    events[p].fxt = FX_EXTENDED;
                    events[p].fxp = 0x01;
                }
                0x08 => {
                    /* set tempo */
                    if let Some(speed) = 100u8.checked_div(param) {
                        events[p].fxt = FX_SPEED;
                        events[p].fxp = speed;
                    }
                }
                0x09 => {
                    /* set instrument */
                    inst = param + 1;
                }
                0x0a => {
                    /* arpeggio */
                    per_command = FX_ARPEGGIO;
                    per_param = param;
                }
                0x0b => {
                    /* tone portamento */
                    per_command = FX_TONEPORTA;
                    per_param = param;
                }
                0x0c => {
                    /* vibrato */
                    per_command = FX_VIBRATO;
                    per_param = param;
                }
                0x0d => {
                    /* volume slide */
                    if param != 0 {
                        per_command = FX_VOLSLIDE;
                        per_param = param;
                    } else {
                        per_command = 0;
                        per_param = 0;
                    }
                }
                0x10 => {
                    /* delay */
                    if per_command != 0 || per_param != 0 {
                        let mut i = 0;
                        while i < param && position < 64 {
                            events[position as usize].fxt = per_command;
                            events[position as usize].fxp = per_param;
                            position += 1;
                            i += 1;
                        }
                    } else {
                        position = position.wrapping_add(param);
                    }
                    if position >= 64 {
                        jumped = true;
                    }
                }
                0x11 => {
                    /* position jump */
                    events[p].fxt = FX_JUMP;
                    events[p].fxp = param;
                    /* break out of the loop because we've jumped.*/
                    jumped = true;
                }
                _ => {
                    /* (upstream's disabled branch writes out an error for
                     * any unprocessed commands) */
                    return -1;
                }
            }
        } else if patdata & 0x4000 != 0 {
            /* old note format.*/
            /* old note format is 2 x 2 byte words with bit 14 set on the first word */
            /* WORD 1: 0x4XDD | X = don't care, D = delay to apply after note. (Usually in 7FDD form).
             * WORD 2: 0xXPPP | PPP = Amiga Period */

            let delay = patdata & 0xff;
            patdata = hio_read16b(f);

            if patdata == 0 && delay == 0 {
                /* a zero note, with zero delay ends the pattern */
                break;
            }

            if patdata != 0 {
                /* convert the note from amiga period format to xmp's internal format.*/
                let p = position as usize;
                events[p].note = libxmp_period_to_note((patdata & 0x0fff) as i32) as u8;
                events[p].ins = inst;
            }

            /* now add on the delay */
            position = position.wrapping_add(delay as u8);
            if position >= 64 {
                break;
            }
        } else {
            /* new note format */
            /* convert the note from amiga period format to xmp's internal format.*/
            let p = position as usize;
            events[p].note = libxmp_period_to_note((patdata & 0x0fff) as i32) as u8;
            events[p].ins = inst;
        }

        /* read the data for the next pass round the loop */
        patdata = hio_read16b(f);

        /* check for an EOF while reading */
        if hio_eof(f) != 0 {
            break;
        }
    }
    if (1..64).contains(&position) {
        events[position as usize - 1].f2t = FX_BREAK;
    }

    hio_seek(f, storepos as i64, SEEK_SET);

    position as i32
}

/// Translation of `read_abk_insts()`.
fn read_abk_insts(
    f: &mut HioHandle<'_>,
    _inst_section_size: u32,
    count: i32,
) -> Option<Vec<AbkInstrument>> {
    if count < 1 {
        return None;
    }

    let mut inst = try_vec(count as usize, AbkInstrument::default()).ok()?;

    for i in 0..count as usize {
        inst[i].sample_offset = hio_read32b(f);
        inst[i].repeat_offset = hio_read32b(f);
        inst[i].sample_length = hio_read16b(f) as u32;
        inst[i].repeat_end = hio_read16b(f);
        inst[i].sample_volume = hio_read16b(f);
        let sample_length = hio_read16b(f) as u32;

        /* detect a potential bug where the sample length is not specified (and we might already know the length) */
        if sample_length > 4 {
            inst[i].sample_length = sample_length;
        }

        if hio_read(&mut inst[i].sample_name, 1, 16, f) != 16 {
            return None;
        }
    }

    Some(inst)
}

/// Translation of `abk_test()`.
fn abk_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    let mut song = AbkSong::default();
    let mut music = [0u8; 8];

    if hio_read32b(f) != AMOS_BANK {
        return -1;
    }

    if hio_read16b(f) != AMOS_MUSIC_TYPE {
        return -1;
    }

    /* skip over length and chip/fastmem.*/
    hio_seek(f, 6, SEEK_CUR);

    if hio_read(&mut music, 1, 8, f) != 8 {
        /* get the "Music   " */
        return -1;
    }

    if &music != b"Music   " {
        return -1;
    }

    /* Attempt to read title. */
    hio_read32b(f); /* instruments_offset */
    let song_section_offset = hio_read32b(f);

    if let Some(t) = t {
        if read_abk_song(
            f,
            &mut song,
            AMOS_MAIN_HEADER.wrapping_add(song_section_offset),
        ) == 0
        {
            libxmp_copy_adjust(t, &song.song_name, AMOS_STRING_LEN as i32);
        }
    }

    0
}

/// Translation of `abk_load()`.
fn abk_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut main_header = AbkHeader::default();
    let mut song = AbkSong::default();
    let mut playlist = AbkPlaylist::default();

    hio_seek(f, AMOS_MAIN_HEADER as i64, SEEK_SET);

    main_header.instruments_offset = hio_read32b(f);
    main_header.songs_offset = hio_read32b(f);
    main_header.patterns_offset = hio_read32b(f);

    /* Sanity check */
    if main_header.instruments_offset > 0x00100000
        || main_header.songs_offset > 0x00100000
        || main_header.patterns_offset > 0x00100000
    {
        return -1;
    }

    let inst_section_size = main_header.instruments_offset;

    libxmp_set_type(m, b"AMOS Music Bank");

    if read_abk_song(f, &mut song, AMOS_MAIN_HEADER + main_header.songs_offset) < 0 {
        return -1;
    }
    let _ = song.tempo;

    libxmp_copy_adjust(&mut m.mod_.name, &song.song_name, AMOS_STRING_LEN as i32);

    hio_seek(
        f,
        (AMOS_MAIN_HEADER + main_header.patterns_offset) as i64,
        SEEK_SET,
    );

    let mod_ = &mut m.mod_;
    mod_.chn = AMOS_ABK_CHANNELS as i32;
    mod_.pat = hio_read16b(f) as i32;

    /* Sanity check */
    if mod_.pat > 256 {
        return -1;
    }

    mod_.trk = mod_.chn * mod_.pat;

    /* move to the start of the instruments section. */
    hio_seek(
        f,
        (AMOS_MAIN_HEADER + main_header.instruments_offset) as i64,
        SEEK_SET,
    );
    mod_.ins = hio_read16b(f) as i32;

    /* Sanity check */
    if mod_.ins > 255 {
        return -1;
    }

    mod_.smp = mod_.ins;

    /* Read and convert instruments and samples */

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    /* read all the instruments in */
    let Some(ci) = read_abk_insts(f, inst_section_size, m.mod_.ins) else {
        return -1;
    };

    /* store the location of the first sample so we can read them later. */
    let first_sample_offset =
        (AMOS_MAIN_HEADER + main_header.instruments_offset).wrapping_add(ci[0].sample_offset);

    let mod_ = &mut m.mod_;
    for i in 0..mod_.ins {
        let iu = i as usize;
        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        let xxs = &mut mod_.xxs[iu];
        xxs.len = (ci[iu].sample_length << 1) as i32;

        if xxs.len > 0 {
            mod_.xxi[iu].nsm = 1;
        }

        /* the repeating stuff. */
        if ci[iu].repeat_offset > ci[iu].sample_offset {
            xxs.lps = ((ci[iu].repeat_offset - ci[iu].sample_offset) << 1) as i32;
        } else {
            xxs.lps = 0;
        }
        xxs.lpe = ci[iu].repeat_end as i32;
        if xxs.lpe > 2 {
            xxs.lpe <<= 1;
            xxs.flg = XMP_SAMPLE_LOOP;
        }
        /*printf("%02x lps=%04x lpe=%04x\n", i,  mod->xxs[i].lps, mod->xxs[i].lpe);*/

        let sub = &mut mod_.xxi[iu].sub[0];
        sub.vol = ci[iu].sample_volume as i32;
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.sid = i;

        libxmp_instrument_name(mod_, i, &ci[iu].sample_name, 16);
    }

    drop(ci);

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* figure out the playlist order.
     * TODO: if the 4 channels arent in the same order then
     * we need to fail here. */

    read_abk_playlist(f, song.playlist_offset[0], &mut playlist);

    /* move to the start of the instruments section */
    /* then convert the patterns one at a time. there is a pattern for each channel.*/
    hio_seek(
        f,
        (AMOS_MAIN_HEADER + main_header.patterns_offset + 2) as i64,
        SEEK_SET,
    );

    mod_.len = 0;

    let mut i = 0;
    for _ in 0..mod_.pat {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        for k in 0..mod_.chn {
            let pattern = hio_read16b(f);
            let t = i * mod_.chn + k;
            if read_abk_pattern(
                f,
                &mut mod_.track_mut(t).event,
                AMOS_MAIN_HEADER + main_header.patterns_offset + pattern as u32,
            ) < 0
            {
                return -1;
            }
        }

        i += 1;
    }

    /* Sanity check */
    if playlist.length > 256 {
        return -1;
    }

    mod_.len = playlist.length as i32;

    /* now push all the patterns into the module and set the length */
    for i in 0..playlist.length as usize {
        mod_.xxo[i] = playlist.pattern[i] as u8;
    }

    /* free up some memory here */
    drop(playlist);

    /* Read samples */
    hio_seek(f, first_sample_offset as i64, SEEK_SET);

    for i in 0..m.mod_.ins as usize {
        if m.mod_.xxs[i].len <= 2 {
            continue;
        }

        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[i], None) < 0 {
            return -1;
        }
    }

    0
}
