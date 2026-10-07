/*

    TiMidity -- Experimental MIDI to WAVE converter
    Copyright (C) 1995 Tuukka Toivonen <toivonen@clinet.fi>

    This program is free software; you can redistribute it and/or modify
    it under the terms of the Perl Artistic License, available in COPYING.

   instrum.c

   Code to load and unload GUS-compatible instrument patches.

*/
// Modified 2026-10-07: translated into Rust from instrum.c and instrum.h in SDL_mixer 3.3.0's TiMidity (see NOTICE).

//! Loading GUS patches. Translation of `instrum.c` (and `instrum.h`).

use std::sync::Arc;

use sdl3::io::{IoStream, IoWhence};

use crate::common::timi_openfile;
use crate::options::*;
use crate::resample::pre_resample;
use crate::sndfont::load_soundfont;
use crate::tables::SINE_CYCLE_LENGTH;
use crate::timidity::{tones, Globals};
use crate::{InstSlot, Instrument, MidiSong, Sample, INST_GUS, MAXBANK, VIBRATO_SAMPLE_INCREMENTS};

/* Bits in modes: */
pub(crate) const MODES_16BIT: u8 = 1 << 0;
pub(crate) const MODES_UNSIGNED: u8 = 1 << 1;
pub(crate) const MODES_LOOPING: u8 = 1 << 2;
pub(crate) const MODES_PINGPONG: u8 = 1 << 3;
pub(crate) const MODES_REVERSE: u8 = 1 << 4;
pub(crate) const MODES_SUSTAIN: u8 = 1 << 5;
pub(crate) const MODES_ENVELOPE: u8 = 1 << 6;

// (MAGIC_LOAD_INSTRUMENT is InstSlot::MagicLoad.)

pub(crate) const SPECIAL_PROGRAM: i32 = -1;

// (free_instrument() is Instrument's Drop.)

fn free_bank(song: &mut MidiSong, dr: bool, b: usize) {
    let bank = if dr {
        &mut song.drumset[b]
    } else {
        &mut song.tonebank[b]
    };
    if let Some(bank) = bank {
        for i in 0..MAXBANK {
            bank.instrument[i] = InstSlot::None;
        }
    }
}

fn convert_envelope_rate(song: &MidiSong, rate: u8) -> i32 {
    let mut r: i32;

    r = 3 - ((rate as i32 >> 6) & 0x3);
    r *= 3;
    r = ((rate & 0x3f) as i32) << r; /* 6.9 fixed point */

    /* 15.15 fixed point. */
    r = ((r * 44100) / song.rate).wrapping_mul(song.control_ratio);

    if FAST_DECAY {
        r.wrapping_shl(10)
    } else {
        r.wrapping_shl(9)
    }
}

fn convert_envelope_offset(offset: u8) -> i32 {
    /* This is not too good... Can anyone tell me what these values mean?
    Are they GUS-style "exponential" volumes? And what does that mean? */

    /* 15.15 fixed point */
    (offset as i32) << (7 + 15)
}

fn convert_tremolo_sweep(song: &MidiSong, sweep: u8) -> i32 {
    if sweep == 0 {
        return 0;
    }

    ((song.control_ratio.wrapping_mul(SWEEP_TUNING)).wrapping_shl(SWEEP_SHIFT as u32))
        .wrapping_div(song.rate.wrapping_mul(sweep as i32))
}

fn convert_vibrato_sweep(song: &MidiSong, sweep: u8, vib_control_ratio: i32) -> i32 {
    if sweep == 0 {
        return 0;
    }

    crate::common::c_f64_to_i32(
        tim_fscale((vib_control_ratio as f64) * SWEEP_TUNING as f64, SWEEP_SHIFT) as f64
            / (song.rate.wrapping_mul(sweep as i32)) as f64,
    )

    /* this was overflowing with seashore.pat

    ((vib_control_ratio * SWEEP_TUNING) << SWEEP_SHIFT) /
    (song->rate * sweep); */
}

fn convert_tremolo_rate(song: &MidiSong, rate: u8) -> i32 {
    ((SINE_CYCLE_LENGTH
        .wrapping_mul(song.control_ratio)
        .wrapping_mul(rate as i32))
    .wrapping_shl(RATE_SHIFT as u32))
    .wrapping_div(TREMOLO_RATE_TUNING.wrapping_mul(song.rate))
}

fn convert_vibrato_rate(song: &MidiSong, rate: u8) -> i32 {
    /* Return a suitable vibrato_control_ratio value */
    (VIBRATO_RATE_TUNING.wrapping_mul(song.rate))
        / (rate as i32 * 2 * VIBRATO_SAMPLE_INCREMENTS as i32)
}

fn reverse_data(sp: &mut [i16], ls: i32, le: i32) {
    // Sint16 s, *ep=sp+le; sp+=ls; le-=ls; le/=2; while (le--) { swap }
    let mut a = ls as usize;
    let mut e = le as usize;
    let mut n = (le - ls) / 2;
    while n > 0 {
        n -= 1;
        if a < sp.len() && e < sp.len() {
            sp.swap(a, e);
        }
        a += 1;
        e = e.wrapping_sub(1);
    }
}

/// `SDL_ReadIO()`, true when it filled `buf`.
fn read_full(io: &mut IoStream<'_>, buf: &mut [u8]) -> bool {
    io.read(buf) == buf.len()
}

/// What failed in `load_instrument()` (its `goto` labels).
enum Fail {
    Nomem,
    Badread,
    Fail,
}

/*
If panning or note_to_use != -1, it will be used for all samples,
instead of the sample-specific values in the instrument file.

For note_to_use, any value <0 or >127 will be forced to 0.

For other parameters, 1 means yes, 0 means no, other values are
undefined.

TODO: do reverse loops right */
fn load_instrument(
    song: &mut MidiSong,
    name: Option<&[u8]>,
    percussion: i32,
    panning: i32,
    amp: i32,
    note_to_use: i32,
    strip_loop: i32,
    strip_envelope: i32,
    strip_tail: i32,
) -> Option<Box<Instrument>> {
    let _ = percussion; /* unused */
    let name = name?;

    /* Open patch file */
    let mut io = timi_openfile(name);
    if io.is_none() {
        /* Try with various extensions */
        for ext in PATCH_EXT_LIST {
            // SDL_snprintf(tmp, sizeof(tmp), "%s%s", name, patch_ext[i])
            let mut tmp = name.to_vec();
            tmp.extend_from_slice(ext.as_bytes());
            tmp.truncate(1023);
            io = timi_openfile(&tmp);
            if io.is_some() {
                break;
            }
        }
    }

    let Some(mut io) = io else {
        crate::snddbg!("Instrument `{}' can't be found.\n", String::from_utf8_lossy(name));
        return None;
    };

    crate::snddbg!("Loading instrument {}\n", String::from_utf8_lossy(name));

    /* Read some headers and do cursory sanity checks. There are loads
    of magic offsets. This could be rewritten... */

    let mut tmp = [0u8; 1024];
    if !read_full(&mut io, &mut tmp[..239])
        || (&tmp[..22] != b"GF1PATCH110\0ID#000002\0" && &tmp[..22] != b"GF1PATCH100\0ID#000002\0")
    {
        /* don't know what the
        differences are */
        crate::snddbg!("{}: not an instrument\n", String::from_utf8_lossy(name));
        return None; /* badpat */
    }

    if tmp[82] != 1 && tmp[82] != 0 {
        /* instruments. To some patch makers,
        0 means 1 */
        crate::snddbg!("Can't handle patches with {} instruments\n", tmp[82]);
        return None; /* badpat */
    }

    if tmp[151] != 1 && tmp[151] != 0 {
        /* layers. What's a layer? */
        crate::snddbg!("Can't handle instruments with {} layers\n", tmp[151]);
        return None; /* badpat */
    }

    let mut ip = Box::new(Instrument {
        type_: INST_GUS,
        samples: tmp[198] as i8 as i32,
        sample: Vec::new(),
    });
    // (a negative count is a huge size_t for SDL_malloc(), which fails.)
    if ip.samples < 0 || ip.sample.try_reserve_exact(ip.samples as usize).is_err() {
        song.oom = 1;
        return None; /* nomem */
    }

    let mut i = 0;
    let result = (|| -> Result<(), Fail> {
        while i < ip.samples {
            let mut sp = Sample::default();

            macro_rules! read_char {
                () => {{
                    let mut b = [0u8; 1];
                    if !read_full(&mut io, &mut b) {
                        return Err(Fail::Badread);
                    }
                    b[0]
                }};
            }
            macro_rules! read_short {
                () => {{
                    let mut b = [0u8; 2];
                    if !read_full(&mut io, &mut b) {
                        return Err(Fail::Badread);
                    }
                    u16::from_le_bytes(b)
                }};
            }
            macro_rules! read_long {
                () => {{
                    let mut b = [0u8; 4];
                    if !read_full(&mut io, &mut b) {
                        return Err(Fail::Badread);
                    }
                    i32::from_le_bytes(b)
                }};
            }

            let _ = io.seek(7, IoWhence::Cur); /* Skip the wave name */

            let fractions = read_char!();

            sp.data_length = read_long!();
            sp.loop_start = read_long!();
            sp.loop_end = read_long!();
            sp.sample_rate = read_short!() as i32;
            sp.low_freq = read_long!();
            sp.high_freq = read_long!();
            sp.root_freq = read_long!();
            let _ = io.seek(2, IoWhence::Cur); /* Why have a "root frequency" and then
                                               * "tuning"?? */

            tmp[0] = read_char!();

            if panning == -1 {
                sp.panning = (((tmp[0] as i8 as i32) * 8 + 4) & 0x7f) as i8;
            } else {
                sp.panning = (panning & 0x7F) as u8 as i8;
            }

            /* envelope, tremolo, and vibrato */
            if !read_full(&mut io, &mut tmp[..18]) {
                return Err(Fail::Badread);
            }

            if tmp[13] == 0 || tmp[14] == 0 {
                sp.tremolo_sweep_increment = 0;
                sp.tremolo_phase_increment = 0;
                sp.tremolo_depth = 0;
                crate::snddbg!(" * no tremolo\n");
            } else {
                sp.tremolo_sweep_increment = convert_tremolo_sweep(song, tmp[12]);
                sp.tremolo_phase_increment = convert_tremolo_rate(song, tmp[13]);
                sp.tremolo_depth = tmp[14];
                crate::snddbg!(
                    " * tremolo: sweep {}, phase {}, depth {}\n",
                    sp.tremolo_sweep_increment,
                    sp.tremolo_phase_increment,
                    sp.tremolo_depth
                );
            }

            if tmp[16] == 0 || tmp[17] == 0 {
                sp.vibrato_sweep_increment = 0;
                sp.vibrato_control_ratio = 0;
                sp.vibrato_depth = 0;
                crate::snddbg!(" * no vibrato\n");
            } else {
                sp.vibrato_control_ratio = convert_vibrato_rate(song, tmp[16]);
                sp.vibrato_sweep_increment =
                    convert_vibrato_sweep(song, tmp[15], sp.vibrato_control_ratio);
                sp.vibrato_depth = tmp[17];
                crate::snddbg!(
                    " * vibrato: sweep {}, ctl {}, depth {}\n",
                    sp.vibrato_sweep_increment,
                    sp.vibrato_control_ratio,
                    sp.vibrato_depth
                );
            }

            sp.modes = read_char!();

            let _ = io.seek(40, IoWhence::Cur); /* skip the useless scale frequency, scale
                                                factor (what's it mean?), and reserved
                                                space */

            /* Mark this as a fixed-pitch instrument if such a deed is desired. */
            if note_to_use != -1 {
                sp.note_to_use = note_to_use as u8 as i8;
            } else {
                sp.note_to_use = 0;
            }

            /* seashore.pat in the Midia patch set has no Sustain. I don't
            understand why, and fixing it by adding the Sustain flag to
            all looped patches probably breaks something else. We do it
            anyway. */
            if sp.modes & MODES_LOOPING != 0 {
                sp.modes |= MODES_SUSTAIN;
            }

            /* Strip any loops and envelopes we're permitted to */
            if (strip_loop == 1)
                && (sp.modes & (MODES_SUSTAIN | MODES_LOOPING | MODES_PINGPONG | MODES_REVERSE)
                    != 0)
            {
                crate::snddbg!(" - Removing loop and/or sustain\n");
                sp.modes &= !(MODES_SUSTAIN | MODES_LOOPING | MODES_PINGPONG | MODES_REVERSE);
            }

            if strip_envelope == 1 {
                if sp.modes & MODES_ENVELOPE != 0 {
                    crate::snddbg!(" - Removing envelope\n");
                }
                sp.modes &= !MODES_ENVELOPE;
            } else if strip_envelope != 0 {
                /* Have to make a guess. */
                if sp.modes & (MODES_LOOPING | MODES_PINGPONG | MODES_REVERSE) == 0 {
                    /* No loop? Then what's there to sustain? No envelope needed
                    either... */
                    sp.modes &= !(MODES_SUSTAIN | MODES_ENVELOPE);
                    crate::snddbg!(" - No loop, removing sustain and envelope\n");
                } else if &tmp[..6] == b"??????" || (tmp[11] as i8) >= 100 {
                    /* Envelope rates all maxed out? Envelope end at a high "offset"?
                    That's a weird envelope. Take it out. */
                    sp.modes &= !MODES_ENVELOPE;
                    crate::snddbg!(" - Weirdness, removing envelope\n");
                } else if sp.modes & MODES_SUSTAIN == 0 {
                    /* No sustain? Then no envelope.  I don't know if this is
                    justified, but patches without sustain usually don't need the
                    envelope either... at least the Gravis ones. They're mostly
                    drums.  I think. */
                    sp.modes &= !MODES_ENVELOPE;
                    crate::snddbg!(" - No sustain, removing envelope\n");
                }
            }

            for j in 0..6 {
                sp.envelope_rate[j] = convert_envelope_rate(song, tmp[j]);
                sp.envelope_offset[j] = convert_envelope_offset(tmp[6 + j]);
            }

            /* Then read the sample data */
            // SDL_malloc(sp->data_length+4): a negative size_t fails.
            let data_length = sp.data_length;
            if data_length.wrapping_add(4) < 0 {
                return Err(Fail::Nomem);
            }
            if data_length < 0 {
                // FIXME (upstream): a length of -4 to -1 allocates 0 to 3
                // bytes, then reads the rest of the file into them (it
                // can't read -1 bytes, which fails the read).
                return Err(Fail::Badread);
            }
            let mut bytes: Vec<u8> = Vec::new();
            {
                // (read in pieces, so a length the file doesn't have fails
                // the read, as it would upstream, without first allocating
                // it all.)
                let want = data_length as usize;
                let mut chunk = [0u8; 65536];
                while bytes.len() < want {
                    let n = (want - bytes.len()).min(chunk.len());
                    let got = io.read(&mut chunk[..n]);
                    if bytes.try_reserve(got).is_err() {
                        return Err(Fail::Nomem);
                    }
                    bytes.extend_from_slice(&chunk[..got]);
                    if got < n {
                        return Err(Fail::Badread);
                    }
                }
            }

            if sp.modes & MODES_16BIT == 0 {
                /* convert to 16-bit data */
                // (data_length bytes, then the 2 extra samples upstream's
                // +4 bytes hold.)
                let mut new16: Vec<i16> = Vec::new();
                if new16.try_reserve_exact(bytes.len() + 2).is_err() {
                    return Err(Fail::Nomem);
                }
                sp.data_length = sp.data_length.wrapping_mul(2);
                sp.loop_start = sp.loop_start.wrapping_mul(2);
                sp.loop_end = sp.loop_end.wrapping_mul(2);
                for &c in &bytes {
                    new16.push(((c as u16) << 8) as i16);
                }
                new16.resize(bytes.len() + 2, 0);
                sp.data = new16;
            } else {
                /* convert to machine byte order */
                // (the data_length+4 bytes upstream allocates, the ones
                // past the data zeroed: upstream leaves them uninitialized.)
                let n = (bytes.len() + 4) / 2;
                let mut data: Vec<i16> = Vec::new();
                if data.try_reserve_exact(n).is_err() {
                    return Err(Fail::Nomem);
                }
                for k in 0..n {
                    let lo = bytes.get(2 * k).copied().unwrap_or(0);
                    let hi = bytes.get(2 * k + 1).copied().unwrap_or(0);
                    data.push(i16::from_le_bytes([lo, hi]));
                }
                sp.data = data;
            }
            drop(bytes);

            if sp.modes & MODES_UNSIGNED != 0 {
                /* convert to signed data */
                let k = (sp.data_length / 2) as usize;
                for s in sp.data.iter_mut().take(k) {
                    *s ^= 0x8000u16 as i16;
                }
            }

            /* Reverse reverse loops and pass them off as normal loops */
            if sp.modes & MODES_REVERSE != 0 {
                /* The GUS apparently plays reverse loops by reversing the
                whole sample. We do the same because the GUS does not SUCK. */

                crate::snddbg!("Reverse loop in {}\n", String::from_utf8_lossy(name));
                // FIXME (upstream): this reverses one sample past the end
                // (the first of the extra samples, not yet set upstream: 0
                // here), and the last real sample is then overwritten with
                // 0 below.
                reverse_data(&mut sp.data, 0, sp.data_length / 2);

                let t = sp.loop_start;
                sp.loop_start = sp.data_length.wrapping_sub(sp.loop_end);
                sp.loop_end = sp.data_length.wrapping_sub(t);

                sp.modes &= !MODES_REVERSE;
                sp.modes |= MODES_LOOPING; /* just in case */
            }

            if ADJUST_SAMPLE_VOLUMES {
                if amp != -1 {
                    sp.volume = ((amp as f64) / 100.0) as f32;
                } else {
                    /* Try to determine a volume scaling factor for the sample.
                    This is a very crude adjustment, but things sound more
                    balanced with it. Still, this should be a runtime option. */
                    let k = (sp.data_length / 2).max(0) as usize;
                    let mut maxamp: i16 = 0;
                    for &s in sp.data.iter().take(k) {
                        let mut a = s;
                        if a < 0 {
                            a = a.wrapping_neg();
                        }
                        if a > maxamp {
                            maxamp = a;
                        }
                    }
                    sp.volume = (32768.0 / maxamp as f64) as f32;
                    crate::snddbg!(" * volume comp: {}\n", sp.volume);
                }
            } else if amp != -1 {
                sp.volume = ((amp as f64) / 100.0) as f32;
            } else {
                sp.volume = 1.0;
            }

            sp.data_length /= 2; /* These are in bytes. Convert into samples. */
            sp.loop_start /= 2;
            sp.loop_end /= 2;

            /* initialize the added extra sample space (see the +4 bytes) */
            let dl = sp.data_length as usize;
            if let Some(s) = sp.data.get_mut(dl) {
                *s = 0;
            }
            if let Some(s) = sp.data.get_mut(dl + 1) {
                *s = 0;
            }

            /* Then fractional samples */
            sp.data_length = sp.data_length.wrapping_shl(FRACTION_BITS as u32);
            sp.loop_start = sp.loop_start.wrapping_shl(FRACTION_BITS as u32);
            sp.loop_end = sp.loop_end.wrapping_shl(FRACTION_BITS as u32);

            /* Adjust for fractional loop points. This is a guess. Does anyone
            know what "fractions" really stands for? */
            sp.loop_start |= ((fractions & 0x0F) as i32) << (FRACTION_BITS - 4);
            sp.loop_end |= (((fractions >> 4) & 0x0F) as i32) << (FRACTION_BITS - 4);

            /* If this instrument will always be played on the same note,
            and it's not looped, we can resample it now. */
            if sp.note_to_use != 0 && sp.modes & MODES_LOOPING == 0 {
                pre_resample(song, &mut sp);
                if song.oom != 0 {
                    return Err(Fail::Fail);
                }
            }

            if strip_tail == 1 {
                /* Let's not really, just say we did. */
                crate::snddbg!(" - Stripping tail\n");
                sp.data_length = sp.loop_end;
            }

            ip.sample.push(Arc::new(sp));
            i += 1;
        }
        Ok(())
    })();

    match result {
        Ok(()) => Some(ip),
        Err(Fail::Nomem) => {
            song.oom = 1;
            None
        }
        Err(Fail::Badread) => {
            crate::snddbg!("Error reading sample {}\n", i);
            None
        }
        Err(Fail::Fail) => None,
    }
}

fn fill_bank(song: &mut MidiSong, g: &mut Globals, dr: bool, b: usize) -> i32 {
    let mut errors = 0;
    let exists = if dr {
        song.drumset[b].is_some()
    } else {
        song.tonebank[b].is_some()
    };
    if !exists {
        crate::snddbg!(
            "Huh. Tried to load instruments in non-existent {} {}\n",
            if dr { "drumset" } else { "tone bank" },
            b
        );
        return 0;
    }
    for i in 0..MAXBANK {
        let bank = if dr {
            song.drumset[b].as_mut()
        } else {
            song.tonebank[b].as_mut()
        };
        let Some(bank) = bank else {
            break;
        };
        if !matches!(bank.instrument[i], InstSlot::MagicLoad) {
            continue;
        }
        let master = if dr {
            &mut g.master_drumset[b]
        } else {
            &mut g.master_tonebank[b]
        };
        let tone = tones(bank, master).and_then(|t| t.get(i).cloned()).unwrap_or_default();
        if tone.name.is_none() {
            crate::snddbg!(
                "No instrument mapped to {} {}, program {}{}\n",
                if dr { "drum set" } else { "tone bank" },
                b,
                i,
                if b != 0 {
                    ""
                } else {
                    " - this instrument will not be heard"
                }
            );
            if b != 0 {
                /* Mark the corresponding instrument in the default
                bank / drumset for loading (if it isn't already) */
                let bank0 = if !dr {
                    song.tonebank[0].as_mut()
                } else {
                    song.drumset[0].as_mut()
                };
                if let Some(bank0) = bank0 {
                    if !bank0.instrument[i].is_some() {
                        bank0.instrument[i] = InstSlot::MagicLoad;
                    }
                }
            }
            set_slot(song, dr, b, i, None);
            errors += 1;
        } else {
            /* preload soundfont */
            let ip = load_soundfont(
                song,
                g,
                0,
                if dr { 128 } else { b as i32 },
                if dr { b as i32 } else { i as i32 },
                if dr { i as i32 } else { -1 },
            );
            let loaded = ip.is_some();
            set_slot(song, dr, b, i, ip);
            if loaded {
                continue;
            }
            /* try gus patch */
            let ip = load_instrument(
                song,
                tone.name.as_deref(),
                if dr { 1 } else { 0 },
                tone.pan,
                tone.amp,
                if tone.note != -1 {
                    tone.note
                } else if dr {
                    i as i32
                } else {
                    -1
                },
                if tone.strip_loop != -1 {
                    tone.strip_loop
                } else if dr {
                    1
                } else {
                    -1
                },
                if tone.strip_envelope != -1 {
                    tone.strip_envelope
                } else if dr {
                    1
                } else {
                    -1
                },
                tone.strip_tail,
            );
            let loaded = ip.is_some();
            set_slot(song, dr, b, i, ip);
            if loaded {
                continue;
            }
            /* no patch; search soundfont again. */
            let ip = load_soundfont(
                song,
                g,
                1,
                if dr { 128 } else { b as i32 },
                if dr { b as i32 } else { i as i32 },
                if dr { i as i32 } else { -1 },
            );
            if ip.is_none() {
                crate::snddbg!(
                    "Couldn't load instrument {} ({} {}, program {})\n",
                    String::from_utf8_lossy(tone.name.as_deref().unwrap_or_default()),
                    if dr { "drum set" } else { "tone bank" },
                    b,
                    i
                );
                errors += 1;
            }
            set_slot(song, dr, b, i, ip);
        }
    }
    errors
}

/// `bank->instrument[i] = ip`.
fn set_slot(song: &mut MidiSong, dr: bool, b: usize, i: usize, ip: Option<Box<Instrument>>) {
    let bank = if dr {
        song.drumset[b].as_mut()
    } else {
        song.tonebank[b].as_mut()
    };
    if let Some(bank) = bank {
        bank.instrument[i] = match ip {
            Some(ip) => InstSlot::Loaded(ip),
            None => InstSlot::None,
        };
    }
}

/// Translation of `load_missing_instruments()`.
pub(crate) fn load_missing_instruments(song: &mut MidiSong, g: &mut Globals) -> i32 {
    let mut errors = 0;
    for i in (0..MAXBANK).rev() {
        if song.tonebank[i].is_some() {
            errors += fill_bank(song, g, false, i);
        }
        if song.drumset[i].is_some() {
            errors += fill_bank(song, g, true, i);
        }
    }
    errors
}

/// Translation of `free_instruments()`.
pub(crate) fn free_instruments(song: &mut MidiSong) {
    for i in (0..MAXBANK).rev() {
        if song.tonebank[i].is_some() {
            free_bank(song, false, i);
        }
        if song.drumset[i].is_some() {
            free_bank(song, true, i);
        }
    }
}

/// Translation of `set_default_instrument()`.
pub(crate) fn set_default_instrument(song: &mut MidiSong, name: &[u8]) -> i32 {
    song.default_instrument = load_instrument(song, Some(name), 0, -1, -1, -1, 0, 0, 0);
    if song.default_instrument.is_none() {
        return -1;
    }
    song.default_program = SPECIAL_PROGRAM;
    0
}
