/*
    TiMidity -- Experimental MIDI to WAVE converter
    Copyright (C) 1995 Tuukka Toivonen <toivonen@clinet.fi>

    This program is free software; you can redistribute it and/or modify
    it under the terms of the Perl Artistic License, available in COPYING.
*/
// Modified 2026-10-07: tests of the Rust translation of SDL_mixer 3.3.0's TiMidity (see NOTICE).

//! The files in `testdata/` are made by `tools/gen_timidity_testdata.py`
//! (synthetic GUS patches, a SoundFont, configuration files and MIDI
//! files, some deliberately broken). `testdata/reference.txt` is the
//! output of a C program built from upstream SDL_mixer's TiMidity and
//! SDL3, which this test repeats and compares line by line: every
//! configuration file loaded (or failing to); every MIDI file rendered in
//! eight output formats, rates, buffer sizes and volumes (FNV-1a hashes of
//! the bytes), then seeked and rendered some more; truncated copies and
//! copies with corrupted bytes of a few, which must render, or fail, as
//! upstream does; and songs played with a SoundFont.
//!
//! Upstream's C was patched where the translation works around its memory
//! errors (the `FIXME (upstream)` notes: buffers it leaves uninitialized
//! are zeroed, as the translation's are), and built to use SDL's bundled
//! math functions, as sdl3's are, rather than the C library's. The files
//! here don't hit the places where the translation does something defined
//! where upstream reads or writes out of bounds.
//!
//! TiMidity's configuration is global, and `read_midi_event()` keeps the
//! running status from one file to the next, so everything runs in one
//! test, in upstream's order. It runs in `sdl3-mixer-timidity/`, as the C
//! program does (`default.cfg` names a directory relative to it).

use std::fmt::Write;

use sdl3::audio::{AudioFormat, AudioSpec};
use sdl3::io::IoStream;

use crate::*;

static REFERENCE: &[u8] = include_bytes!("../testdata/reference.txt");

const FNV0: u64 = 14695981039346656037;

fn fnv(mut h: u64, data: &[u8]) -> u64 {
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

struct Variant {
    format: AudioFormat,
    channels: i32,
    freq: i32,
    samples: i32,
    volume: i32,
}

const VARIANTS: [Variant; 8] = [
    Variant {
        format: AudioFormat::S16LE,
        channels: 2,
        freq: 44100,
        samples: 256,
        volume: 100,
    },
    Variant {
        format: AudioFormat::S32LE,
        channels: 2,
        freq: 48000,
        samples: 256,
        volume: 800,
    },
    Variant {
        format: AudioFormat::F32LE,
        channels: 1,
        freq: 22050,
        samples: 1000,
        volume: 70,
    },
    Variant {
        format: AudioFormat::U8,
        channels: 2,
        freq: 11025,
        samples: 100,
        volume: 300,
    },
    Variant {
        format: AudioFormat::S8,
        channels: 1,
        freq: 8000,
        samples: 64,
        volume: 100,
    },
    Variant {
        format: AudioFormat::S16BE,
        channels: 1,
        freq: 32000,
        samples: 333,
        volume: 50,
    },
    Variant {
        format: AudioFormat::F32BE,
        channels: 2,
        freq: 96000,
        samples: 512,
        volume: 100,
    },
    Variant {
        format: AudioFormat::S32BE,
        channels: 1,
        freq: 4000,
        samples: 50,
        volume: 900,
    },
];

const CAP_FRAMES: i64 = 5 * 48000;

fn bytes_per_frame(v: &Variant) -> i32 {
    v.format.bytesize() as i32 * v.channels
}

fn rc(r: sdl3::Result<()>) -> i32 {
    if r.is_ok() {
        0
    } else {
        -1
    }
}

/// Load, play to the end, then seek and play a little.
fn render(out: &mut String, data: &[u8], v: &Variant, seek: u32) {
    let spec = AudioSpec {
        format: v.format,
        channels: v.channels,
        freq: v.freq,
    };
    let Ok(mut song) = MidiSong::load(&mut IoStream::from_const_mem(data), &spec, v.samples) else {
        writeln!(out, "load failed").unwrap();
        return;
    };
    write!(out, "len={} ", song.song_length()).unwrap();
    song.set_volume(v.volume);
    song.start();
    let chunk = (v.samples * bytes_per_frame(v)) as usize;
    let mut buf = vec![0u8; chunk * 3];
    let mut h = FNV0;
    let mut total: i64 = 0;
    let mut calls = 0;
    let mut r;
    loop {
        buf[..chunk].fill(0xAA);
        r = song.play_some(&mut buf[..chunk]);
        calls += 1;
        if r <= 0 {
            break;
        }
        h = fnv(h, &buf[..r as usize]);
        total += r as i64;
        if total >= CAP_FRAMES * bytes_per_frame(v) as i64 {
            break;
        }
    }
    write!(
        out,
        "bytes={total} calls={calls} rc={r} time={} active={} {h:016x}",
        song.song_time(),
        song.is_active() as i32
    )
    .unwrap();
    if seek != 0 {
        song.seek(500);
        buf[..chunk].fill(0xAA);
        r = song.play_some(&mut buf[..chunk]);
        write!(out, " afterend={r}").unwrap();
        song.start();
        song.seek(seek);
        write!(out, " seektime={}", song.song_time()).unwrap();
        h = FNV0;
        for _ in 0..6 {
            buf[..chunk].fill(0xAA);
            r = song.play_some(&mut buf[..chunk]);
            if r <= 0 {
                break;
            }
            h = fnv(h, &buf[..r as usize]);
        }
        write!(out, " rc={r} {h:016x}").unwrap();
        /* three buffers at once: each goes to the start */
        song.start();
        buf.fill(0xAA);
        r = song.play_some(&mut buf);
        write!(out, " triple={r} {:016x}", fnv(FNV0, &buf)).unwrap();
        song.stop();
        write!(out, " stopped={}", song.play_some(&mut buf[..chunk])).unwrap();
    }
    writeln!(out).unwrap();
}

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0
    }
}

const DIR: &str = "testdata";

fn read(name: &str) -> Vec<u8> {
    std::fs::read(format!("{DIR}/{name}")).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn play_file(out: &mut String, name: &str, nvariants: usize, damage: bool) {
    let data = read(&format!("midi/{name}"));
    let len = data.len();
    writeln!(out, "== {name}").unwrap();
    for (i, v) in VARIANTS.iter().enumerate().take(nvariants) {
        write!(out, "v{i}: ").unwrap();
        render(out, &data, v, 300 + i as u32 * 250);
    }
    if damage {
        const CUTS: [usize; 16] = [0, 3, 4, 8, 12, 13, 14, 18, 21, 22, 23, 26, 30, 40, 60, 100];
        for cut in CUTS {
            if cut >= len {
                break;
            }
            write!(out, "trunc {cut}: ").unwrap();
            render(out, &data[..cut], &VARIANTS[0], 0);
        }
        for cut in [len / 4, len / 2, len * 3 / 4, len - 1] {
            write!(out, "trunc {cut}: ").unwrap();
            render(out, &data[..cut], &VARIANTS[0], 0);
        }
        for v in 0..8u32 {
            let mut bad = data.clone();
            let mut lcg = Lcg(v.wrapping_mul(7919).wrapping_add(len as u32));
            let flips = 1 + v % 4;
            for _ in 0..flips {
                let r = lcg.next();
                let at = if v < 4 {
                    (r >> 8) as usize % len.min(64)
                } else {
                    14 + (r >> 8) as usize % (len - 14)
                };
                bad[at] ^= (1 + (lcg.next() >> 24) % 255) as u8;
            }
            write!(out, "corrupt {v}: ").unwrap();
            render(out, &bad, &VARIANTS[1], 0);
        }
    }
}

fn init_cfg(out: &mut String, name: &str) {
    writeln!(
        out,
        "init {name}: {}",
        rc(init(Some(&format!("{DIR}/{name}"))))
    )
    .unwrap();
}

fn run(out: &mut String) {
    writeln!(out, "== configs").unwrap();
    const BAD: [&str; 30] = [
        "bad_syntax.cfg",
        "bad_nobank.cfg",
        "bad_program.cfg",
        "bad_banknum.cfg",
        "bad_drumnum.cfg",
        "bad_option.cfg",
        "bad_noequals.cfg",
        "bad_amp.cfg",
        "bad_note.cfg",
        "pan_word.cfg",
        "bad_panleft.cfg",
        "bad_keep.cfg",
        "bad_strip.cfg",
        "bad_quote.cfg",
        "bad_quote2.cfg",
        "bad_quote3.cfg",
        "bad_quote4.cfg",
        "bad_dir.cfg",
        "bad_source.cfg",
        "bad_source_missing.cfg",
        "bad_source_loop.cfg",
        "bad_default.cfg",
        "bad_soundfont.cfg",
        "bad_soundfont_missing.cfg",
        "bad_font.cfg",
        "bad_font_exclude.cfg",
        "bad_font_order.cfg",
        "sf2_badorder.cfg",
        "sf2_badoption.cfg",
        "nowhere.cfg",
    ];
    for name in BAD {
        init_cfg(out, name);
        exit();
    }
    writeln!(out, "init NULL: {}", rc(init(None))).unwrap();
    exit();
    writeln!(out, "noconfig: {}", rc(init_no_config())).unwrap();
    play_file(out, "basic.mid", 1, false);
    exit();

    init_cfg(out, "timidity.cfg");
    {
        let data = read("midi/basic.mid");
        let mut spec = AudioSpec {
            format: AudioFormat::S16LE,
            channels: 6,
            freq: 44100,
        };
        let r = MidiSong::load(&mut IoStream::from_const_mem(&data), &spec, 256);
        writeln!(
            out,
            "6ch: {} {}",
            r.is_ok() as i32,
            r.err().map(|e| e.to_string()).unwrap_or_default()
        )
        .unwrap();
        spec.channels = 2;
        spec.format = AudioFormat::UNKNOWN;
        let r = MidiSong::load(&mut IoStream::from_const_mem(&data), &spec, 256);
        writeln!(
            out,
            "unknown: {} {}",
            r.is_ok() as i32,
            r.err().map(|e| e.to_string()).unwrap_or_default()
        )
        .unwrap();
    }
    const FILES: [&str; 21] = [
        "bad_format.mid",
        "bad_format0_2tracks.mid",
        "bad_header_len.mid",
        "bad_missing_track.mid",
        "bad_no_eot.mid",
        "bad_rmid.rmi",
        "bad_track_magic.mid",
        "bad_tracks0.mid",
        "banks.mid",
        "basic.mid",
        "empty.mid",
        "format1.mid",
        "format2.mid",
        "lead_in.mid",
        "oom.mid",
        "polyphony.mid",
        "rmid.rmi",
        "smpte.mid",
        "sustained.mid",
        "track_len_long.mid",
        "track_len_short.mid",
    ];
    for name in FILES {
        let full = !name.starts_with("bad_") && name != "oom.mid";
        let damage = full && (name == "basic.mid" || name == "format1.mid" || name == "rmid.rmi");
        play_file(out, name, if full { 8 } else { 1 }, damage);
    }
    exit();

    init_cfg(out, "sf2.cfg");
    play_file(out, "sf2.mid", 3, false);
    play_file(out, "sf2.mid", 1, false);
    play_file(out, "basic.mid", 1, false);
    exit();

    writeln!(out, "== soundfonts").unwrap();
    for name in [
        "sf2_truncated.sf2",
        "sf2_badsize.sf2",
        "sf2_version3.sf2",
        "nowhere.sf2",
        "test.sf2",
    ] {
        writeln!(
            out,
            "soundfont {name}: {}",
            rc(set_soundfont(Some(&format!("{DIR}/{name}"))))
        )
        .unwrap();
    }
    writeln!(out, "init: {}", rc(init(Some("unused.cfg")))).unwrap();
    play_file(out, "sf2.mid", 2, false);
    play_file(out, "basic.mid", 1, false);
    exit();

    init_cfg(out, "sbk.cfg");
    play_file(out, "sf2.mid", 2, false);
    play_file(out, "basic.mid", 1, false);
    exit();

    init_cfg(out, "default.cfg");
    play_file(out, "default.mid", 2, false);
    play_file(out, "basic.mid", 1, false);
    exit();
}

#[test]
fn matches_upstream() {
    let mut out = String::new();
    run(&mut out);
    let want = String::from_utf8_lossy(REFERENCE);
    let same = out.lines().count() == want.lines().count()
        && out.lines().zip(want.lines()).all(|(a, b)| a == b);
    if !same {
        let mut report = String::new();
        let mut shown = 0;
        for (i, (a, b)) in out.lines().zip(want.lines()).enumerate() {
            if a != b {
                writeln!(report, "line {}:\n  got:  {a}\n  want: {b}", i + 1).unwrap();
                shown += 1;
                if shown == 30 {
                    break;
                }
            }
        }
        writeln!(
            report,
            "({} lines, reference has {})",
            out.lines().count(),
            want.lines().count()
        )
        .unwrap();
        panic!("output differs from upstream's:\n{report}");
    }
}
