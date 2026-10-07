/*
    TiMidity -- Experimental MIDI to WAVE converter
    Copyright (C) 1995 Tuukka Toivonen <toivonen@clinet.fi>

    This program is free software; you can redistribute it and/or modify
    it under the terms of the Perl Artistic License, available in COPYING.
*/
// Modified 2026-10-07: tests of the Rust translation of SDL_mixer 3.3.0's TiMidity (see NOTICE).

//! Corrupt MIDI files, patches and SoundFonts must load or fail, and play,
//! without panicking (where upstream's C would read or write out of
//! bounds, divide by zero or spin, the translation does something defined;
//! see the `FIXME (upstream)` notes). These run in their own process, as
//! TiMidity's configuration is global, and its MIDI reader keeps state
//! from one file to the next (which `matches_upstream` must start without).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sdl3::audio::{AudioFormat, AudioSpec};
use sdl3::io::IoStream;
use sdl3_mixer_timidity::{exit, init, set_soundfont, MidiSong};

/// TiMidity's configuration is global: one test at a time.
static LOCK: Mutex<()> = Mutex::new(());

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() >> 8) as usize % n
    }
}

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

/// Flip `flips` random bytes of `data` in `range`.
fn corrupt(data: &[u8], seed: u32, flips: usize, range: std::ops::Range<usize>) -> Vec<u8> {
    let mut bad = data.to_vec();
    let mut lcg = Lcg(seed);
    let end = range.end.min(bad.len());
    if range.start >= end {
        return bad;
    }
    for _ in 0..flips {
        let at = range.start + lcg.below(end - range.start);
        bad[at] ^= (1 + lcg.next() % 255) as u8;
    }
    bad
}

/// Load and play up to `seconds` (at 22050 Hz), seeking halfway.
fn play(data: &[u8], seconds: i64) -> Option<i64> {
    let spec = AudioSpec {
        format: AudioFormat::S16LE,
        channels: 2,
        freq: 22050,
    };
    let mut song = MidiSong::load(&mut IoStream::from_const_mem(data), &spec, 256).ok()?;
    song.set_volume(800);
    song.start();
    let mut buf = [0u8; 1024];
    let mut total = 0i64;
    while total < seconds * 22050 * 4 {
        let n = song.play_some(&mut buf);
        if n <= 0 {
            break;
        }
        total += n as i64;
        if total == 40 * 1024 {
            song.seek(song.song_length() / 2);
        }
    }
    Some(total)
}

#[test]
fn corrupt_midi_files() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = testdata();
    init(Some(&dir.join("timidity.cfg").to_string_lossy())).unwrap();
    for name in [
        "basic.mid",
        "format1.mid",
        "format2.mid",
        "rmid.rmi",
        "banks.mid",
        "smpte.mid",
    ] {
        let data = std::fs::read(dir.join("midi").join(name)).unwrap();
        for seed in 0..40u32 {
            let flips = 1 + (seed as usize % 8);
            let bad = corrupt(&data, seed * 31 + 7, flips, 0..data.len());
            let _ = play(&bad, 2);
        }
        for cut in (0..data.len()).step_by(7) {
            let _ = play(&data[..cut], 1);
        }
    }
    exit();
}

/// Write copies of the test patches, one of them corrupted, and a
/// configuration naming them, to a directory of their own. (A patch that
/// makes an allocation fail fails the whole song.)
fn corrupt_patches(out: &Path, seed: u32) -> PathBuf {
    let dir = testdata().join("patches");
    let _ = std::fs::create_dir_all(out);
    let mut cfg = String::from("bank 0\n");
    let mut drums = String::from("drumset 0\n");
    let mut lcg = Lcg(seed);
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != "negsamples.pat")
        .collect();
    names.sort();
    let victim = seed as usize % names.len();
    for (i, name) in names.iter().enumerate() {
        let data = std::fs::read(dir.join(name)).unwrap();
        // the headers (the first sample's included), then anywhere
        let flips = 1 + lcg.below(6);
        let bad = if i != victim {
            data
        } else if lcg.below(3) != 0 {
            corrupt(&data, lcg.next(), flips, 0..239 + 96)
        } else {
            corrupt(&data, lcg.next(), flips, 0..data.len())
        };
        let file = format!("p{i}.pat");
        std::fs::write(out.join(&file), bad).unwrap();
        let opts = [
            "",
            " strip=tail",
            " note=60",
            " keep=loop keep=env",
            " strip=env",
        ];
        cfg.push_str(&format!("{i} {file}{}\n", opts[lcg.below(opts.len())]));
        drums.push_str(&format!(
            "{} {file}{}\n",
            35 + i,
            opts[lcg.below(opts.len())]
        ));
    }
    cfg.push_str(&drums);
    let path = out.join("corrupt.cfg");
    std::fs::write(&path, cfg).unwrap();
    path
}

/// A MIDI file playing every program and drum the corrupted patches are
/// mapped to, at several pitches, with pitch bends.
fn every_program() -> Vec<u8> {
    let mut ev: Vec<u8> = Vec::new();
    let mut push = |delta: u8, bytes: &[u8]| {
        ev.push(delta);
        ev.extend_from_slice(bytes);
    };
    for p in 0..24u8 {
        let ch = p % 9;
        push(0, &[0xC0 | ch, p]);
        push(0, &[0xE0 | ch, 0, (p * 5) & 0x7F]);
        for (k, note) in [24u8, 60, 100, 127].iter().enumerate() {
            push(0, &[0x90 | ch, *note, 100]);
            push(0, &[0x99, 35 + p, 100]);
            push(if k == 3 { 30 } else { 0 }, &[0x80 | ch, *note, 0]);
        }
        push(10, &[0x89, 35 + p, 0]);
    }
    push(0, &[0xFF, 0x2F, 0x00]);
    let mut midi = b"MThd\0\0\0\x06\0\0\0\x01\0\x60MTrk".to_vec();
    midi.extend_from_slice(&(ev.len() as u32).to_be_bytes());
    midi.extend_from_slice(&ev);
    midi
}

#[test]
fn corrupt_patches_play() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = std::env::temp_dir().join(format!("sdl3-mixer-timidity-{}", std::process::id()));
    let midi = every_program();
    let mut played = 0;
    for seed in 0..150 {
        let cfg = corrupt_patches(&out, seed);
        if init(Some(&cfg.to_string_lossy())).is_ok() {
            if play(&midi, 4).is_some_and(|n| n > 0) {
                played += 1;
            }
            let _ = play(
                &std::fs::read(testdata().join("midi/sustained.mid")).unwrap(),
                2,
            );
        }
        exit();
    }
    let _ = std::fs::remove_dir_all(&out);
    // (most of them still play: this tests something.)
    assert!(played > 100, "only {played} of 150 played");
}

#[test]
fn corrupt_soundfonts() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = testdata();
    let sf2 = std::fs::read(dir.join("test.sf2")).unwrap();
    let midi = std::fs::read(dir.join("midi/sf2.mid")).unwrap();
    let out = std::env::temp_dir().join(format!("sdl3-mixer-timidity-sf2-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&out);
    // the presets, instruments, zones and generators are near the end
    let pdta = sf2.windows(4).position(|w| w == b"pdta").unwrap();
    let mut played = 0;
    for seed in 0..60u32 {
        let flips = 1 + seed as usize % 6;
        let bad = if seed % 4 == 0 {
            corrupt(&sf2, seed, flips, 0..sf2.len())
        } else {
            corrupt(&sf2, seed, flips, pdta..sf2.len())
        };
        let path = out.join(format!("c{seed}.sf2"));
        std::fs::write(&path, &bad).unwrap();
        if set_soundfont(Some(&path.to_string_lossy())).is_ok() && init(None).is_ok() {
            if play(&midi, 2).is_some_and(|n| n > 0) {
                played += 1;
            }
            let _ = play(&midi, 2);
        }
        exit();
    }
    let _ = std::fs::remove_dir_all(&out);
    // (most of them still play: this tests something.)
    assert!(played > 30, "only {played} of 60 played");
}
