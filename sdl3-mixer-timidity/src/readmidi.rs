/*
    TiMidity -- Experimental MIDI to WAVE converter
    Copyright (C) 1995 Tuukka Toivonen <toivonen@clinet.fi>

    This program is free software; you can redistribute it and/or modify
    it under the terms of the Perl Artistic License, available in COPYING.
*/
// Modified 2026-10-07: translated into Rust from readmidi.c and readmidi.h in SDL_mixer 3.3.0's TiMidity (see NOTICE).

//! Reading Standard MIDI Files (and RIFF RMID files) into a list of
//! events. Translation of `readmidi.c` (and `readmidi.h`).

use std::sync::Mutex;

use sdl3::io::{IoStream, IoWhence};

use crate::common::c_f64_to_i32;
use crate::instrum::SPECIAL_PROGRAM;
use crate::playmidi::*;
use crate::{MidiEvent, MidiEventList, MidiSong, MAXCHAN};

/* Computes how many (fractional) samples one MIDI delta-time unit contains */
fn compute_sample_increment(song: &mut MidiSong, tempo: i32, divisions: i32) {
    let a = (tempo as f64) * (song.rate as f64) * (65536.0 / 1000000.0) / (divisions as f64);

    song.sample_correction = c_f64_to_i32(a) & 0xFFFF;
    song.sample_increment = c_f64_to_i32(a) >> 16;

    crate::snddbg!(
        "Samples per delta-t: {} (correction {})",
        song.sample_increment,
        song.sample_correction
    );
}

/// `SDL_ReadU8()`.
fn read_u8(io: &mut IoStream<'_>) -> Option<u8> {
    io.read_u8().ok()
}

/// `SDL_ReadIO()` into a buffer, true when it was filled.
fn read_full(io: &mut IoStream<'_>, buf: &mut [u8]) -> bool {
    io.read(buf) == buf.len()
}

/* Read variable-length number (7 bits per byte, MSB first) */
fn getvl(io: &mut IoStream<'_>) -> i32 {
    let mut l: i32 = 0;
    loop {
        let Some(c) = read_u8(io) else {
            return l;
        };
        l = l.wrapping_add((c & 0x7f) as i32);
        if c & 0x80 == 0 {
            return l;
        }
        l = l.wrapping_shl(7);
    }
}

// (`dumpstring()` is only built with DEBUG_CHATTER.)

/// What `read_midi_event()` returns: a new event, `MAGIC_EOT`, or `NULL`.
enum ReadEvent {
    Event(MidiEvent),
    Eot,
    Error,
}

/// `read_midi_event()`'s `static` variables, which upstream keeps from
/// one call to the next, and from one song to the next.
struct ReadMidiStatics {
    laststatus: u8,
    lastchan: u8,
    nrpn: u8,
    rpn_msb: [u8; 16],
    rpn_lsb: [u8; 16], /* one per channel */
}

// FIXME (upstream): the running status and the RPN state carry over from
// one file to the next (and are shared by threads loading at once).
static READ_MIDI_STATICS: Mutex<ReadMidiStatics> = Mutex::new(ReadMidiStatics {
    laststatus: 0,
    lastchan: 0,
    nrpn: 0,
    rpn_msb: [0; 16],
    rpn_lsb: [0; 16],
});

/// `MIDIEVENT(at,t,ch,pa,pb)`.
fn midievent(song: &mut MidiSong, at: i32, t: i32, ch: u8, pa: u8, pb: u8) -> ReadEvent {
    if song.evlist.try_reserve(1).is_err() {
        song.oom = 1;
        return ReadEvent::Error;
    }
    ReadEvent::Event(MidiEvent {
        time: at,
        type_: t as u8,
        channel: ch,
        a: pa,
        b: pb,
    })
}

/* Read a MIDI event, returning a freshly allocated element that can
be linked to the event list */
fn read_midi_event(song: &mut MidiSong, io: &mut IoStream<'_>) -> ReadEvent {
    let mut st = READ_MIDI_STATICS.lock().unwrap_or_else(|e| e.into_inner());
    let mut a: u8;
    let mut b: u8;

    loop {
        song.at = song.at.wrapping_add(getvl(io));
        let Some(me) = read_u8(io) else {
            crate::snddbg!("read_midi_event: SDL_IOread() failure\n");
            return ReadEvent::Error;
        };

        if me == 0xF0 || me == 0xF7 {
            /* SysEx event */
            let len = getvl(io);
            let _ = io.seek(len as i64, IoWhence::Cur);
        } else if me == 0xFF {
            /* Meta event */
            let Some(type_) = read_u8(io) else {
                crate::snddbg!("read_midi_event: SDL_IOread() failure\n");
                return ReadEvent::Error;
            };
            let len = getvl(io);
            if type_ > 0 && type_ < 16 {
                let _ = io.seek(len as i64, IoWhence::Cur);
            } else {
                match type_ {
                    0x2F => {
                        /* End of Track */
                        return ReadEvent::Eot;
                    }

                    0x51 => {
                        /* Tempo */
                        let (Some(ta), Some(tb), Some(tc)) =
                            (read_u8(io), read_u8(io), read_u8(io))
                        else {
                            crate::snddbg!("read_midi_event: SDL_IOread() failure\n");
                            return ReadEvent::Error;
                        };
                        return midievent(song, song.at, ME_TEMPO, tc, ta, tb);
                    }

                    _ => {
                        crate::snddbg!("(Meta event type 0x{:02x}, length {})\n", type_, len);
                        if io.seek(len as i64, IoWhence::Cur).is_err() {
                            crate::snddbg!("read_midi_event: SDL_IOseek() failure\n");
                            return ReadEvent::Error;
                        }
                    }
                }
            }
        } else {
            a = me;
            if a & 0x80 != 0 {
                /* status byte */
                st.lastchan = a & 0x0F;
                st.laststatus = (a >> 4) & 0x07;
                let Some(x) = read_u8(io) else {
                    crate::snddbg!("read_midi_event: SDL_IOread() failure\n");
                    return ReadEvent::Error;
                };
                a = x & 0x7F;
            }
            let lastchan = st.lastchan;
            match st.laststatus {
                0 => {
                    /* Note off */
                    let Some(x) = read_u8(io) else {
                        crate::snddbg!("read_midi_event: SDL_IOread() failure\n");
                        return ReadEvent::Error;
                    };
                    b = x & 0x7F;
                    return midievent(song, song.at, ME_NOTEOFF, lastchan, a, b);
                }

                1 => {
                    /* Note on */
                    let Some(x) = read_u8(io) else {
                        crate::snddbg!("read_midi_event: SDL_IOread() failure\n");
                        return ReadEvent::Error;
                    };
                    b = x & 0x7F;
                    return midievent(song, song.at, ME_NOTEON, lastchan, a, b);
                }

                2 => {
                    /* Key Pressure */
                    let Some(x) = read_u8(io) else {
                        crate::snddbg!("read_midi_event: SDL_IOread() failure\n");
                        return ReadEvent::Error;
                    };
                    b = x & 0x7F;
                    return midievent(song, song.at, ME_KEYPRESSURE, lastchan, a, b);
                }

                3 => {
                    /* Control change */
                    let Some(x) = read_u8(io) else {
                        crate::snddbg!("read_midi_event: SDL_IOread() failure\n");
                        return ReadEvent::Error;
                    };
                    b = x & 0x7F;
                    {
                        let mut control = 255;
                        match a {
                            7 => control = ME_MAINVOLUME,
                            10 => control = ME_PAN,
                            11 => control = ME_EXPRESSION,
                            64 => {
                                control = ME_SUSTAIN;
                                b = (b >= 64) as u8;
                            }
                            120 => control = ME_ALL_SOUNDS_OFF,
                            121 => control = ME_RESET_CONTROLLERS,
                            123 => control = ME_ALL_NOTES_OFF,

                            /* These should be the SCC-1 tone bank switch
                            commands. I don't know why there are two, or
                            why the latter only allows switching to bank 0.
                            Also, some MIDI files use 0 as some sort of
                            continuous controller. This will cause lots of
                            warnings about undefined tone banks. */
                            0 => control = ME_TONE_BANK,
                            32 => {
                                if b != 0 {
                                    crate::snddbg!("(Strange: tone bank change 0x{:02x})\n", b);
                                }
                                /* `Bank Select LSB' is not worked at GS. Please ignore it. */
                            }

                            100 => {
                                st.nrpn = 0;
                                st.rpn_msb[lastchan as usize] = b;
                            }
                            101 => {
                                st.nrpn = 0;
                                st.rpn_lsb[lastchan as usize] = b;
                            }
                            99 => {
                                st.nrpn = 1;
                                st.rpn_msb[lastchan as usize] = b;
                            }
                            98 => {
                                st.nrpn = 1;
                                st.rpn_lsb[lastchan as usize] = b;
                            }

                            6 => {
                                if st.nrpn != 0 {
                                    crate::snddbg!(
                                        "(Data entry (MSB) for NRPN {:02x},{:02x}: {})\n",
                                        st.rpn_msb[lastchan as usize],
                                        st.rpn_lsb[lastchan as usize],
                                        b
                                    );
                                } else {
                                    match ((st.rpn_msb[lastchan as usize] as i32) << 8)
                                        | st.rpn_lsb[lastchan as usize] as i32
                                    {
                                        0x0000 => {
                                            /* Pitch bend sensitivity */
                                            control = ME_PITCH_SENS;
                                        }

                                        0x7F7F => {
                                            /* RPN reset */
                                            /* reset pitch bend sensitivity to 2 */
                                            return midievent(
                                                song,
                                                song.at,
                                                ME_PITCH_SENS,
                                                lastchan,
                                                2,
                                                0,
                                            );
                                        }

                                        _ => {
                                            crate::snddbg!(
                                                "(Data entry (MSB) for RPN {:02x},{:02x}: {})\n",
                                                st.rpn_msb[lastchan as usize],
                                                st.rpn_lsb[lastchan as usize],
                                                b
                                            );
                                        }
                                    }
                                }
                            }

                            _ => {
                                crate::snddbg!("(Control {}: {})\n", a, b);
                            }
                        }
                        if control != 255 {
                            return midievent(song, song.at, control, lastchan, b, 0);
                        }
                    }
                }

                4 => {
                    /* Program change */
                    a &= 0x7f;
                    return midievent(song, song.at, ME_PROGRAM, lastchan, a, 0);
                }

                5 => { /* Channel pressure - NOT IMPLEMENTED */ }

                6 => {
                    /* Pitch wheel */
                    let Some(x) = read_u8(io) else {
                        crate::snddbg!("read_midi_event: SDL_IOread() failure\n");
                        return ReadEvent::Error;
                    };
                    b = x & 0x7F;
                    return midievent(song, song.at, ME_PITCHWHEEL, lastchan, a, b);
                }

                _ => {
                    crate::snddbg!(
                        "*** Can't happen: status 0x{:02X}, channel 0x{:02X}\n",
                        st.laststatus,
                        lastchan
                    );
                }
            }
        }
    }
}

/* Read a midi track into the linked list, either merging with any previous
tracks or appending to them. */
fn read_track(song: &mut MidiSong, io: &mut IoStream<'_>, append: bool) -> i32 {
    let mut meep: usize = 0;
    if append && !song.evlist.is_empty() {
        /* find the last event in the list */
        while let Some(next) = song.evlist[meep].next {
            meep = next as usize;
        }
        song.at = song.evlist[meep].event.time;
    } else {
        song.at = 0;
    }

    /* Check the formalities */
    let mut tmp = [0u8; 4];
    let mut lenbuf = [0u8; 4];
    if !read_full(io, &mut tmp) || !read_full(io, &mut lenbuf) {
        crate::snddbg!("Can't read track header.\n");
        return -1;
    }
    let len = i32::from_be_bytes(lenbuf);
    let next_pos = io.tell().unwrap_or(-1) + len as i64;
    if &tmp != b"MTrk" {
        crate::snddbg!("Corrupt MIDI file.\n");
        return -2;
    }

    loop {
        let newlist = match read_midi_event(song, io) {
            ReadEvent::Error => return -2, /* Some kind of error  */
            ReadEvent::Eot => {
                /* End-of-track Hack. */
                /* If the track ends before the size of the
                 * track data, skip any junk at the end.  */
                let pos = io.tell().unwrap_or(-1);
                if pos < next_pos {
                    let _ = io.seek(next_pos - pos, IoWhence::Cur);
                }
                return 0;
            }
            ReadEvent::Event(event) => event,
        };

        let mut next = song.evlist[meep].next;
        while let Some(n) = next {
            if song.evlist[n as usize].event.time >= newlist.time {
                break;
            }
            meep = n as usize;
            next = song.evlist[meep].next;
        }

        let new_index = song.evlist.len() as u32;
        song.evlist.push(MidiEventList {
            event: newlist,
            next,
        });
        song.evlist[meep].next = Some(new_index);

        song.event_count += 1; /* Count the event. (About one?) */
        meep = new_index as usize;
    }
}

/* Free the linked event list from memory. */
fn free_midi_list(song: &mut MidiSong) {
    song.evlist = Vec::new();
}

/* Allocate an array of MidiEvents and fill it from the linked list of
events, marking used instruments for loading. Convert event times to
samples: handle tempo changes. Strip unnecessary events from the list.
Free the linked list. */
fn groom_list(
    song: &mut MidiSong,
    divisions: i32,
    eventsp: &mut i32,
    samplesp: &mut i32,
) -> Option<Vec<MidiEvent>> {
    let mut current_bank = [0i32; MAXCHAN];
    let mut current_set = [0i32; MAXCHAN];
    let mut current_program = [0i32; MAXCHAN];
    /* Or should each bank have its own current program? */

    for i in 0..MAXCHAN {
        current_bank[i] = 0;
        current_set[i] = 0;
        current_program[i] = song.default_program;
    }

    let mut tempo = 500000;
    compute_sample_increment(song, tempo, divisions);

    /* This may allocate a bit more than we need */
    let mut groomed_list: Vec<MidiEvent> = Vec::new();
    if groomed_list
        .try_reserve_exact((song.event_count as usize).saturating_add(1))
        .is_err()
    {
        song.oom = 1;
        free_midi_list(song);
        return None;
    }
    let mut meep = Some(0usize);

    let mut our_event_count = 0;
    let mut st: i32 = 0;
    let mut at: i32 = 0;
    let mut sample_cum: i32 = 0;
    let mut counting_time = 2; /* We strip any silence before the first NOTE ON. */

    for _ in 0..song.event_count {
        let Some(m) = meep else {
            break;
        };
        let mut skip_this_event = 0;
        let mut ev = song.evlist[m].event;
        let ch = ev.channel as usize;

        if ev.type_ as i32 == ME_TEMPO {
            skip_this_event = 1;
        } else if ch >= MAXCHAN {
            skip_this_event = 1;
        } else {
            match ev.type_ as i32 {
                ME_PROGRAM => {
                    if isdrumchannel(song, ch) {
                        let new_value;
                        if song.drumset[ev.a as usize].is_some() {
                            /* Is this a defined drumset? */
                            new_value = ev.a as i32;
                        } else {
                            crate::snddbg!("Drum set {} is undefined\n", ev.a);
                            ev.a = 0;
                            song.evlist[m].event.a = 0;
                            new_value = 0;
                        }
                        if current_set[ch] != new_value {
                            current_set[ch] = new_value;
                        } else {
                            skip_this_event = 1;
                        }
                    } else {
                        let new_value = ev.a as i32;
                        if (current_program[ch] != SPECIAL_PROGRAM)
                            && (current_program[ch] != new_value)
                        {
                            current_program[ch] = new_value;
                        } else {
                            skip_this_event = 1;
                        }
                    }
                }

                ME_NOTEON => {
                    if counting_time != 0 {
                        counting_time = 1;
                    }
                    if isdrumchannel(song, ch) {
                        /* Mark this instrument to be loaded */
                        if let Some(bank) = song.drumset[current_set[ch] as usize].as_mut() {
                            let slot = &mut bank.instrument[ev.a as usize & 0x7F];
                            if !slot.is_some() {
                                *slot = crate::InstSlot::MagicLoad;
                            }
                        }
                    } else if current_program[ch] != SPECIAL_PROGRAM {
                        /* Mark this instrument to be loaded */
                        if let Some(bank) = song.tonebank[current_bank[ch] as usize].as_mut() {
                            let slot = &mut bank.instrument[current_program[ch] as usize & 0x7F];
                            if !slot.is_some() {
                                *slot = crate::InstSlot::MagicLoad;
                            }
                        }
                    }
                }

                ME_TONE_BANK => {
                    if isdrumchannel(song, ch) {
                        skip_this_event = 1;
                    } else {
                        let new_value;
                        if song.tonebank[ev.a as usize].is_some() {
                            /* Is this a defined tone bank? */
                            new_value = ev.a as i32;
                        } else {
                            crate::snddbg!("Tone bank {} is undefined\n", ev.a);
                            ev.a = 0;
                            song.evlist[m].event.a = 0;
                            new_value = 0;
                        }
                        if current_bank[ch] != new_value {
                            current_bank[ch] = new_value;
                        } else {
                            skip_this_event = 1;
                        }
                    }
                }

                _ => {}
            }
        }

        /* Recompute time in samples*/
        let dt = ev.time.wrapping_sub(at);
        if dt != 0 && counting_time == 0 {
            if song.sample_increment > 2147483647 / dt || song.sample_correction > 2147483647 / dt {
                crate::snddbg!("Overflow in sample counter\n");
                free_midi_list(song);
                return None;
            }
            let mut samples_to_do = song.sample_increment.wrapping_mul(dt);
            sample_cum = sample_cum.wrapping_add(song.sample_correction.wrapping_mul(dt));
            if (sample_cum as u32) & 0xFFFF0000 != 0 {
                samples_to_do = samples_to_do.wrapping_add((sample_cum >> 16) & 0xFFFF);
                sample_cum &= 0x0000FFFF;
            }
            if st >= 2147483647i32.wrapping_sub(samples_to_do) {
                crate::snddbg!("Overflow in sample counter\n");
                free_midi_list(song);
                return None;
            }
            st = st.wrapping_add(samples_to_do);
        } else if counting_time == 1 {
            counting_time = 0;
        }
        if ev.type_ as i32 == ME_TEMPO {
            tempo = ev.channel as i32 + ev.b as i32 * 256 + ev.a as i32 * 65536;
            compute_sample_increment(song, tempo, divisions);
        }
        if skip_this_event == 0 {
            /* Add the event to the list */
            let mut lp = ev;
            lp.time = st;
            groomed_list.push(lp);
            our_event_count += 1;
        }
        at = ev.time;
        meep = song.evlist[m].next.map(|n| n as usize);
    }
    /* Add an End-of-Track event */
    groomed_list.push(MidiEvent {
        time: st,
        type_: ME_EOT as u8,
        ..Default::default()
    });
    our_event_count += 1;
    free_midi_list(song);

    *eventsp = our_event_count;
    *samplesp = st;
    Some(groomed_list)
}

/// Translation of `read_midi_file()`.
pub(crate) fn read_midi_file(
    song: &mut MidiSong,
    io: &mut IoStream<'_>,
    count: &mut i32,
    sp: &mut i32,
) -> Option<Vec<MidiEvent>> {
    let divisions: i32;
    let mut tmp = [0u8; 4];
    let mut lenbuf = [0u8; 4];

    song.event_count = 0;
    song.at = 0;
    song.evlist = Vec::new();

    if !read_full(io, &mut tmp) || !read_full(io, &mut lenbuf) {
        crate::snddbg!("Not a MIDI file!\n");
        return None;
    }
    if &tmp == b"RIFF" {
        /* RMID ?? */
        if !read_full(io, &mut tmp)
            || &tmp != b"RMID"
            || !read_full(io, &mut tmp)
            || &tmp != b"data"
            || !read_full(io, &mut tmp)
            /* SMF must begin from here onwards: */
            || !read_full(io, &mut tmp)
            || !read_full(io, &mut lenbuf)
        {
            crate::snddbg!("Not an RMID file!\n");
            return None;
        }
    }
    let len = i32::from_be_bytes(lenbuf);
    if &tmp != b"MThd" || len < 6 {
        crate::snddbg!("Not a MIDI file!\n");
        return None;
    }

    let header = (|| {
        Some((
            io.read_s16_be().ok()?,
            io.read_s16_be().ok()?,
            io.read_s16_be().ok()?,
        ))
    })();
    let Some((format, tracks, divisions_tmp)) = header else {
        crate::snddbg!("Not a MIDI file!\n");
        return None;
    };

    if divisions_tmp < 0 {
        /* SMPTE time -- totally untested. Got a MIDI file that uses this? */
        divisions = (-(divisions_tmp as i32 / 256)) * (divisions_tmp as i32 & 0xFF);
    } else {
        divisions = divisions_tmp as i32;
    }

    if len > 6 {
        crate::snddbg!("MIDI file header size {} bytes", len);
        let _ = io.seek((len - 6) as i64, IoWhence::Cur); /* skip the excess */
    }
    if !(0..=2).contains(&format) {
        crate::snddbg!("Unknown MIDI file format {}\n", format);
        return None;
    }
    if tracks < 1 {
        crate::snddbg!("Bad number of tracks {}\n", tracks);
        return None;
    }
    if format == 0 && tracks != 1 {
        crate::snddbg!("{} tracks with Type-0 MIDI (must be 1.)\n", tracks);
        return None;
    }
    crate::snddbg!(
        "Format: {}  Tracks: {}  Divisions: {}\n",
        format,
        tracks,
        divisions
    );

    /* Put a do-nothing event first in the list for easier processing */
    if song.evlist.try_reserve(1).is_err() {
        song.oom = 1;
        return None;
    }
    song.evlist.push(MidiEventList {
        event: MidiEvent::default(),
        next: None,
    });
    song.event_count += 1;

    match format {
        0 => {
            if read_track(song, io, false) != 0 {
                free_midi_list(song);
                return None;
            }
        }

        1 => {
            for _ in 0..tracks {
                if read_track(song, io, false) != 0 {
                    free_midi_list(song);
                    return None;
                }
            }
        }

        _ => {
            /* We simply play the tracks sequentially */
            for _ in 0..tracks {
                if read_track(song, io, true) != 0 {
                    free_midi_list(song);
                    return None;
                }
            }
        }
    }

    groom_list(song, divisions, count, sp)
}
