// Rust translation of src/loaders/common.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2025 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What the loaders share: allocating instruments, samples, patterns and
//! tracks, names, ProTracker events and instrument files.

use super::super::common::*;
#[cfg(windows)]
use super::super::filetype::libxmp_get_filetype;
use super::super::hio::*;
use super::super::path::*;
use super::super::period::libxmp_period_to_note;
use super::super::*;
use super::*;

/// Translation of `libxmp_init_instrument()`.
pub(crate) fn libxmp_init_instrument(m: &mut ModuleData) -> i32 {
    let mod_ = &mut m.mod_;

    if mod_.ins > 0 {
        let Ok(xxi) = try_vec(mod_.ins as usize, XmpInstrument::default()) else {
            return -1;
        };
        mod_.xxi = xxi;
    }

    if mod_.smp > 0 {
        /* Sanity check */
        if mod_.smp > MAX_SAMPLES {
            return -1;
        }

        let Ok(xxs) = try_vec(mod_.smp as usize, XmpSample::default()) else {
            return -1;
        };
        mod_.xxs = xxs;
        let Ok(xtra) = try_vec(mod_.smp as usize, ExtraSampleData::default()) else {
            return -1;
        };
        m.xtra = xtra;

        for x in &mut m.xtra {
            x.c5spd = m.c4rate as f64;
        }
    }

    0
}

/* Sample number adjustment (originally by Vitamin/CAIG).
 * Only use this AFTER a previous usage of libxmp_init_instrument,
 * and don't use this to free samples that have already been loaded. */
/// Translation of `libxmp_realloc_samples()`.
pub(crate) fn libxmp_realloc_samples(m: &mut ModuleData, new_size: i32) -> i32 {
    let mod_ = &mut m.mod_;

    /* Sanity check */
    if new_size < 0 {
        return -1;
    }

    if new_size == 0 {
        /* Don't rely on implementation-defined realloc(x,0) behavior. */
        mod_.smp = 0;
        mod_.xxs = Vec::new();
        m.xtra = Vec::new();
        return 0;
    }

    let new_size_u = new_size as usize;
    if new_size_u > mod_.xxs.len() && mod_.xxs.try_reserve(new_size_u - mod_.xxs.len()).is_err() {
        return -1;
    }
    if new_size_u > m.xtra.len() && m.xtra.try_reserve(new_size_u - m.xtra.len()).is_err() {
        return -1;
    }
    mod_.xxs.resize(new_size_u, XmpSample::default());
    m.xtra.resize(new_size_u, ExtraSampleData::default());

    if new_size > mod_.smp {
        for i in mod_.smp as usize..new_size_u {
            mod_.xxs[i] = XmpSample::default();
            m.xtra[i] = ExtraSampleData::default();
            m.xtra[i].c5spd = m.c4rate as f64;
        }
    }
    mod_.smp = new_size;
    0
}

/// Translation of `libxmp_alloc_subinstrument()`.
pub(crate) fn libxmp_alloc_subinstrument(mod_: &mut XmpModule, i: i32, num: i32) -> i32 {
    if num == 0 {
        return 0;
    }

    let Ok(sub) = try_vec(num.max(0) as usize, XmpSubinstrument::default()) else {
        return -1;
    };
    mod_.xxi[i as usize].sub = sub;

    0
}

/// Translation of `libxmp_init_pattern()`.
pub(crate) fn libxmp_init_pattern(mod_: &mut XmpModule) -> i32 {
    let Ok(xxt) = try_vec(mod_.trk.max(0) as usize, None) else {
        return -1;
    };
    mod_.xxt = Some(xxt);

    let Ok(xxp) = try_vec(mod_.pat.max(0) as usize, None) else {
        return -1;
    };
    mod_.xxp = Some(xxp);

    0
}

/// Translation of `libxmp_alloc_pattern()`.
pub(crate) fn libxmp_alloc_pattern(mod_: &mut XmpModule, num: i32) -> i32 {
    let chn = mod_.chn;
    let pat = mod_.pat;
    let Some(xxp) = mod_.xxp.as_mut() else {
        return -1;
    };

    /* Sanity check */
    if num < 0 || num >= pat || xxp[num as usize].is_some() {
        return -1;
    }

    /* (calloc'd room for at least one track index, as upstream's
     * sizeof(struct xmp_pattern) has) */
    let Ok(index) = try_vec(chn.max(1) as usize, 0) else {
        return -1;
    };
    xxp[num as usize] = Some(Box::new(XmpPattern { rows: 0, index }));

    0
}

/// Translation of `libxmp_alloc_track()`.
pub(crate) fn libxmp_alloc_track(mod_: &mut XmpModule, num: i32, rows: i32) -> i32 {
    let trk = mod_.trk;
    let Some(xxt) = mod_.xxt.as_mut() else {
        return -1;
    };

    /* Sanity check */
    if num < 0 || num >= trk || xxt[num as usize].is_some() || rows <= 0 {
        return -1;
    }

    let Ok(event) = try_vec(rows as usize, XmpEvent::default()) else {
        return -1;
    };
    xxt[num as usize] = Some(Box::new(XmpTrack { rows, event }));

    0
}

/// Translation of `libxmp_alloc_tracks_in_pattern()`.
pub(crate) fn libxmp_alloc_tracks_in_pattern(mod_: &mut XmpModule, num: i32) -> i32 {
    for i in 0..mod_.chn {
        let t = num * mod_.chn + i;
        let rows = mod_.pattern(num).rows;

        if libxmp_alloc_track(mod_, t, rows) < 0 {
            return -1;
        }

        mod_.pattern_mut(num).index[i as usize] = t;
    }

    0
}

/// Translation of `libxmp_alloc_pattern_tracks()`.
pub(crate) fn libxmp_alloc_pattern_tracks(mod_: &mut XmpModule, num: i32, rows: i32) -> i32 {
    /* Sanity check */
    if rows <= 0 || rows > 256 {
        return -1;
    }

    if libxmp_alloc_pattern(mod_, num) < 0 {
        return -1;
    }

    mod_.pattern_mut(num).rows = rows;

    if libxmp_alloc_tracks_in_pattern(mod_, num) < 0 {
        return -1;
    }

    0
}

/* Some formats explicitly allow more than 256 rows (e.g. OctaMED). This function
 * allows those formats to work without disrupting the sanity check for other formats.
 */
/// Translation of `libxmp_alloc_pattern_tracks_long()`.
pub(crate) fn libxmp_alloc_pattern_tracks_long(mod_: &mut XmpModule, num: i32, rows: i32) -> i32 {
    /* Sanity check */
    if rows <= 0 || rows > 32768 {
        return -1;
    }

    if libxmp_alloc_pattern(mod_, num) < 0 {
        return -1;
    }

    mod_.pattern_mut(num).rows = rows;

    if libxmp_alloc_tracks_in_pattern(mod_, num) < 0 {
        return -1;
    }

    0
}

/// Translation of `libxmp_instrument_name()`.
pub(crate) fn libxmp_instrument_name(mod_: &mut XmpModule, i: i32, r: &[u8], mut n: i32) {
    clamp(&mut n, 0, 31);

    libxmp_copy_adjust(&mut mod_.xxi[i as usize].name, r, n);
}

/// Translation of `libxmp_copy_adjust()`: `s` must hold `n + 1` bytes.
/// (Upstream's `strncpy()` reads `r` up to its NUL or `n` bytes, which
/// here is up to its end too.)
pub(crate) fn libxmp_copy_adjust(s: &mut [u8], r: &[u8], n: i32) {
    let n = n as usize;

    s[..n + 1].fill(0);
    strncpy(s, r, n.min(r.len()));

    let mut i = 0;
    while i < n && s[i] != 0 {
        if !isprint(s[i]) || s[i] > 127 {
            s[i] = b'.';
        }
        i += 1;
    }

    let mut len = strlen(s);
    while len > 0 && s[len - 1] == b' ' {
        s[len - 1] = 0;
        len -= 1;
    }
}

/// Translation of `libxmp_read_title()`.
pub(crate) fn libxmp_read_title(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, mut s: i32) {
    let mut buf = [0u8; XMP_NAME_SIZE];

    let Some(t) = t else {
        return;
    };
    if s < 0 {
        return;
    }

    if s >= XMP_NAME_SIZE as i32 {
        s = XMP_NAME_SIZE as i32 - 1;
    }

    t[..s as usize + 1].fill(0);

    let s = hio_read(&mut buf, 1, s as usize, f);
    buf[s] = 0;
    libxmp_copy_adjust(t, &buf, s as i32);
}

/// Translation of `libxmp_test_name()`.
pub(crate) fn libxmp_test_name(s: &[u8], n: i32, flags: i32) -> i32 {
    for i in 0..n.max(0) as usize {
        let c = s[i];
        if c == b'\0' && flags & TEST_NAME_IGNORE_AFTER_0 != 0 {
            break;
        }
        if c == b'\r' && flags & TEST_NAME_IGNORE_AFTER_CR != 0 {
            break;
        }
        if c > 0x7f {
            return -1;
        }
        /* ACS_Team2.mod has a backspace in instrument name */
        /* Numerous ST modules from Music Channel BBS have char 14. */
        if c > 0 && c < 32 && c != 0x08 && c != 0x0e {
            return -1;
        }
    }

    0
}

/// Translation of `libxmp_copy_name_for_fopen()`: `dest` must hold `n`
/// bytes.
pub(crate) fn libxmp_copy_name_for_fopen(dest: &mut [u8], name: &[u8], n: i32) -> i32 {
    let mut converted_colon = false;
    let name_at = |i: usize| name.get(i).copied().unwrap_or(0);
    let name_str = &name[..strlen(name)];

    /* libxmp_copy_adjust, but make sure the filename won't do anything
     * malicious when given to fopen. This should only be used on song files.
     */
    if name_str == b"."
        || name_str.windows(2).any(|w| w == b"..")
        || name_at(0) == b'\\'
        || name_at(0) == b'/'
        || name_at(0) == b':'
        || name_at(0) == b'\0'
    {
        return -1;
    }

    let mut i = 0;
    while (i as i32) < n - 1 {
        let t = name_at(i);
        if t == 0 {
            break;
        }

        /* Reject non-ASCII symbols as they have poorly defined behavior.
         */
        if !(32..0x7f).contains(&t) {
            return -1;
        }

        /* Reject anything resembling a Windows-style root path. Allow
         * converting a single : to / so things like ST-01:samplename
         * work. (Leave the : as-is on Amiga.)
         */
        if i > 0 && t == b':' && !converted_colon {
            let t2 = name_at(i + 1);
            if t2 == 0 || t2 == b'/' || t2 == b'\\' {
                return -1;
            }

            converted_colon = true;
            dest[i] = b'/';
            i += 1;
            continue;
        }

        if t == b'\\' {
            dest[i] = b'/';
            i += 1;
            continue;
        }

        dest[i] = t;
        i += 1;
    }
    dest[i] = b'\0';
    0
}

/*
 * Honor Noisetracker effects:
 *
 *  0 - arpeggio
 *  1 - portamento up
 *  2 - portamento down
 *  3 - Tone-portamento
 *  4 - Vibrato
 *  A - Slide volume
 *  B - Position jump
 *  C - Set volume
 *  D - Pattern break
 *  E - Set filter (keep the led off, please!)
 *  F - Set speed (now up to $1F)
 *
 * Pex Tufvesson's notes from http://www.livet.se/mahoney/:
 *
 * Note that some of the modules will have bugs in the playback with all
 * known PC module players. This is due to that in many demos where I synced
 * events in the demo with the music, I used commands that these newer PC
 * module players erroneously interpret as "newer-version-trackers commands".
 * Which they aren't.
 */
/// Translation of `libxmp_decode_noisetracker_event()`.
pub(crate) fn libxmp_decode_noisetracker_event(event: &mut XmpEvent, mod_event: &[u8]) {
    *event = XmpEvent::default();
    event.note = libxmp_period_to_note((lsn(mod_event[0]) << 8) + mod_event[1] as i32) as u8;
    event.ins = ((msn(mod_event[0]) << 4) | msn(mod_event[2])) as u8;
    let fxt = lsn(mod_event[2]);

    if fxt <= 0x06 || (fxt >= 0x0a && fxt != 0x0e) {
        event.fxt = fxt as u8;
        event.fxp = mod_event[3];
    }

    libxmp_disable_continue_fx(event);
}

/// Translation of `libxmp_decode_protracker_event()`.
pub(crate) fn libxmp_decode_protracker_event(event: &mut XmpEvent, mod_event: &[u8]) {
    let fxt = lsn(mod_event[2]);

    *event = XmpEvent::default();
    event.note = libxmp_period_to_note((lsn(mod_event[0]) << 8) + mod_event[1] as i32) as u8;
    event.ins = ((msn(mod_event[0]) << 4) | msn(mod_event[2])) as u8;

    if fxt != 0x08 {
        event.fxt = fxt as u8;
        event.fxp = mod_event[3];
    }

    libxmp_disable_continue_fx(event);
}

/// Translation of `libxmp_disable_continue_fx()`.
pub(crate) fn libxmp_disable_continue_fx(event: &mut XmpEvent) {
    if event.fxp == 0 {
        match event.fxt {
            0x05 => event.fxt = 0x03,
            0x06 => event.fxt = 0x04,
            0x01 | 0x02 | 0x0a => event.fxt = 0x00,
            _ => {}
        }
    } else if event.fxt == 0x0e && (event.fxp == 0xa0 || event.fxp == 0xb0) {
        event.fxt = 0;
        event.fxp = 0;
    }
}

/* libxmp_check_filename_case(): */
/* Given a directory, see if file exists there, ignoring case */

/// Translation of `libxmp_check_filename_case()` (the version for
/// case-insensitive file systems, which directly probes the file).
#[cfg(windows)]
pub(crate) fn libxmp_check_filename_case(sp: &mut LibxmpPath, dir: &[u8], name: &[u8]) -> i32 {
    let dir: &[u8] = if dir.first().copied().unwrap_or(0) == b'\0' {
        b"."
    } else {
        dir
    };

    let mut tmp = LibxmpPath::default();
    libxmp_path_init(&mut tmp);
    if libxmp_path_join(&mut tmp, Some(dir), Some(name)) < 0 {
        return -1;
    }

    if libxmp_get_filetype(tmp.path.as_deref().unwrap_or(b"")) & XMP_FILETYPE_FILE == 0 {
        libxmp_path_free(&mut tmp);
        return -1;
    }
    libxmp_path_move(sp, &mut tmp);
    0
}

/// Translation of `libxmp_check_filename_case()` (the version for targets
/// with `dirent`).
#[cfg(not(windows))]
pub(crate) fn libxmp_check_filename_case(sp: &mut LibxmpPath, dir: &[u8], name: &[u8]) -> i32 {
    let mut ret = -1;

    let dir: &[u8] = if dir.first().copied().unwrap_or(0) == b'\0' {
        b"."
    } else {
        dir
    };

    let Ok(dirp) = std::fs::read_dir(super::super::filetype::os_path(dir)) else {
        return -1;
    };

    let name_str = &name[..strlen(name)];
    for d in dirp {
        let Ok(d) = d else {
            break;
        };
        let d_name = d.file_name();
        #[cfg(unix)]
        let d_name = {
            use std::os::unix::ffi::OsStrExt;
            d_name.as_bytes().to_vec()
        };
        #[cfg(not(unix))]
        let d_name = d_name.to_string_lossy().into_owned().into_bytes();
        if d_name.eq_ignore_ascii_case(name_str) {
            // FIXME (upstream): the path is built from the name asked for,
            // not the one found, so a file whose name differs in case
            // isn't opened.
            if libxmp_path_join(sp, Some(dir), Some(name)) == 0 {
                ret = 0;
                break;
            }
        }
    }

    ret
}

/// Translation of `libxmp_get_instrument_path()`.
fn libxmp_get_instrument_path(m: &ModuleData) -> Option<Vec<u8>> {
    if let Some(path) = &m.instrument_path {
        return Some(path.clone());
    }
    let env = std::env::var_os("XMP_INSTRUMENT_PATH")?;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(env.into_vec())
    }
    #[cfg(not(unix))]
    {
        Some(env.to_string_lossy().into_owned().into_bytes())
    }
}

/// Translation of `libxmp_find_instrument_file()`.
pub(crate) fn libxmp_find_instrument_file(
    m: &ModuleData,
    sp: &mut LibxmpPath,
    ins_name: &[u8],
) -> i32 {
    let ins_path = libxmp_get_instrument_path(m);
    if let Some(ins_path) = &ins_path {
        if libxmp_check_filename_case(sp, ins_path, ins_name) == 0 {
            return 0;
        }
    }

    /* Try the module dir if the instrument path didn't work. */
    if let Some(dirname) = &m.dirname {
        if libxmp_check_filename_case(sp, dirname, ins_name) == 0 {
            return 0;
        }
    }

    -1
}

/// Translation of `libxmp_set_type()`: the caller formats the type
/// (upstream's `vsnprintf()`), which is cut to `XMP_NAME_SIZE - 1` bytes.
pub(crate) fn libxmp_set_type(m: &mut ModuleData, type_: &[u8]) {
    snprintf_bytes(&mut m.mod_.type_, XMP_NAME_SIZE, type_);
}

/// Translation of `schism_tracker_date()`.
fn schism_tracker_date(year: i32, month: i32, day: i32) -> i32 {
    let mut mm = (month + 9) % 12;
    let mut yy = year - mm / 10;

    yy = yy * 365 + (yy / 4) - (yy / 100) + (yy / 400);
    mm = (mm * 306 + 5) / 10;

    yy + mm + (day - 1)
}

/* Generate a Schism Tracker version string.
 * Schism Tracker versions are stored as follows:
 *
 * s_ver <= 0x50:		0.s_ver
 * s_ver > 0x50, < 0xfff:	days from epoch=(s_ver - 0x50)
 * s_ver = 0xfff:		days from epoch=l_ver
 */
/// Translation of `libxmp_schism_tracker_string()`.
pub(crate) fn libxmp_schism_tracker_string(buf: &mut [u8], size: usize, s_ver: i32, l_ver: i32) {
    if s_ver >= 0x50 {
        /* time_t epoch_sec = 1256947200; */
        let mut t: i64 = schism_tracker_date(2009, 10, 31) as i64;

        if s_ver == 0xfff {
            t += l_ver as i64;
        } else {
            t += (s_ver - 0x50) as i64;
        }

        /* Date algorithm reimplemented from OpenMPT.
         */
        let mut year = ((t * 10000 + 14780) / 3652425) as i32;
        let yl = year as i64;
        let mut dayofyear = (t - (365 * yl + (yl / 4) - (yl / 100) + (yl / 400))) as i32;
        if dayofyear < 0 {
            year -= 1;
            let yl = year as i64;
            dayofyear = (t - (365 * yl + (yl / 4) - (yl / 100) + (yl / 400))) as i32;
        }
        let mut month = (100 * dayofyear + 52) / 3060;
        let day = dayofyear - (month * 306 + 5) / 10 + 1;

        year += (month + 2) / 12;
        month = (month + 2) % 12 + 1;

        let s = format!("Schism Tracker {:04}-{:02}-{:02}", year, month, day);
        snprintf_bytes(buf, size, s.as_bytes());
    } else {
        let s = format!("Schism Tracker 0.{:x}", s_ver);
        snprintf_bytes(buf, size, s.as_bytes());
    }
}

/* Old MPT modules (from MPT <=1.16, older versions of OpenMPT) rely on a
 * pre-amp routine that scales mix volume down. This is based on the module's
 * channel count and a tracker pre-amp setting that isn't saved in the module.
 * This setting defaults to 128. When fixed to 128, it can be optimized out.
 *
 * In OpenMPT, this pre-amp routine is only available in the MPT and OpenMPT
 * 1.17 RC1 and RC2 mix modes. Changing a module to the compatible or 1.17 RC3
 * mix modes will permanently disable it for that module. OpenMPT applies the
 * old mix modes to MPT <=1.16 modules, "IT 8.88", and in old OpenMPT-made
 * modules that specify one of these mix modes in their extended properties.
 *
 * Set mod->chn and m->mvol first!
 */
/// Translation of `libxmp_apply_mpt_preamp()`.
pub(crate) fn libxmp_apply_mpt_preamp(m: &mut ModuleData) {
    /* OpenMPT uses a slightly different table. */
    #[rustfmt::skip]
    static PREAMP_TABLE: [u8; 16] = [
        0x60, 0x60, 0x60, 0x70, /* 0-7 */
        0x80, 0x88, 0x90, 0x98, /* 8-15 */
        0xA0, 0xA4, 0xA8, 0xB0, /* 16-23 */
        0xB4, 0xB8, 0xBC, 0xC0, /* 24-31 */
    ];

    let mut chn = m.mod_.chn;
    clamp(&mut chn, 1, 31);

    m.mvol = (m.mvol * 96) / PREAMP_TABLE[(chn >> 1) as usize] as i32;

    /* Pre-amp is applied like this in the mixers of libmodplug/libopenmpt
     * (still vastly simplified).

    int preamp = 128;

    if (preamp > 128) {
        preamp = 128 + ((preamp - 128) * (chn + 4)) / 16;
    }
    preamp = preamp * m->mvol / 64;
    preamp = (preamp << 7) / preamp_table[chn >> 1];

    ...

    channel_volume_16bit = (channel_volume_16bit * preamp) >> 7;
    */
}
