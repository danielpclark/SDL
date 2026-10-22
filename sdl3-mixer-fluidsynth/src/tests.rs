/* FluidSynth - A Software Synthesizer
 *
 * Copyright (C) 2003  Peter Hanappe and others.
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public License
 * as published by the Free Software Foundation; either version 2.1 of
 * the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful, but
 * WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see
 * <https://www.gnu.org/licenses/>.
 */
// Modified 2026-10-17: tests of the Rust translation of SDL_mixer 3.3.0's
// FluidSynth 2.5.7 (see NOTICE).

//! The files in `testdata/` are made by `tools/gen_fluidsynth_testdata.py`
//! (a synthetic SoundFont, MIDI files, broken copies of both, and
//! `cases.txt`). `testdata/reference.txt` is the output of a C program
//! built from upstream SDL_mixer's FluidSynth (configured as SDL_mixer's
//! CMake configures it) and SDL3, which this test repeats and compares
//! line by line: each case of `cases.txt` sets up the settings, the synth,
//! a SoundFont loader reading from memory and the player as SDL_mixer's
//! FluidSynth decoder does, renders 128 frames at a time while the player
//! plays (up to a limit), and prints the results of the calls, an FNV-1a
//! hash of the samples' bytes, and the messages logged at every level but
//! `FLUID_DBG` on the way.
//!
//! Upstream's C was built to use SDL's bundled math functions, as sdl3's
//! are, rather than the C library's; with its IIR filter compiled without
//! the `-ffast-math` upstream's CMake gives it (whose results depend on
//! the compiler and the CPU); and patched where the translation works
//! around upstream's memory errors and hangs (the `Note (upstream)` notes
//! of fluid_voice.c, fluid_midi.c and fluid_rev.c, whose reverb renders
//! its first blocks with uninitialized parameters). The files here don't
//! hit the other places where the translation does something defined where
//! upstream reads or writes out of bounds.
//!
//! `FSREF_CASES`, `FSREF_DIR` and `FSREF_DUMP` (a cases file, its data
//! directory and where to write the output instead of comparing it) and
//! `FSREF_TRACE` (a hash every 4096 frames on stderr) are for debugging.

use std::cell::RefCell;
use std::fmt::Write;
use std::sync::{Arc, Once};

use crate::*;

static REFERENCE: &str = include_str!("../testdata/reference.txt");
static CASES: &str = include_str!("../testdata/cases.txt");

const FNV0: u64 = 14695981039346656037;

fn fnv(mut h: u64, data: &[u8]) -> u64 {
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

thread_local! {
    /// The messages logged on this thread while it captures them.
    static CAPTURE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Sends the messages of every level but FLUID_DBG to the capturing
/// thread's buffer (the log functions are global; the tests' threads
/// capture their own messages).
fn install_log() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        for level in 0..LAST_LOG_LEVEL {
            let fun: Option<FluidLogFunction> = if level == FLUID_DBG {
                None
            } else {
                Some(Arc::new(|level: i32, message: &str| {
                    CAPTURE.with(|c| {
                        if let Some(out) = c.borrow_mut().as_mut() {
                            // (each run of non-ASCII characters prints as
                            // one '?', as the reference program prints each
                            // run of non-ASCII bytes)
                            let mut line = String::new();
                            let mut ascii = true;
                            for ch in message.chars() {
                                if ch.is_ascii() {
                                    line.push(ch);
                                    ascii = true;
                                } else if ascii {
                                    line.push('?');
                                    ascii = false;
                                }
                            }
                            let _ = writeln!(out, "  log{} {}", level, line);
                        }
                    });
                }))
            };
            set_log_function(level, fun);
        }
    });
}

fn capture_start() {
    install_log();
    CAPTURE.with(|c| *c.borrow_mut() = Some(String::new()));
}

fn capture_take() -> String {
    CAPTURE.with(|c| c.borrow_mut().replace(String::new()).unwrap_or_default())
}

fn capture_stop() -> String {
    CAPTURE.with(|c| c.borrow_mut().take().unwrap_or_default())
}

/// The reference program's memory file: SDL_ReadIO() and SDL_SeekIO()
/// semantics over a buffer.
struct MemFile {
    data: Arc<[u8]>,
    pos: i64,
}

impl SfFile for MemFile {
    fn read(&mut self, buf: &mut [u8]) -> i32 {
        let count = buf.len() as i64;
        if count > 0xFFFF_FFFF {
            return FLUID_FAILED;
        }
        let avail = self.data.len() as i64 - self.pos;
        let n = count.min(avail).max(0);
        buf[..n as usize].copy_from_slice(&self.data[self.pos as usize..(self.pos + n) as usize]);
        self.pos += n;
        if n == count {
            FLUID_OK
        } else {
            FLUID_FAILED
        }
    }

    fn seek(&mut self, offset: i64, origin: i32) -> i32 {
        let base = match origin {
            SEEK_SET => 0,
            SEEK_CUR => self.pos,
            SEEK_END => self.data.len() as i64,
            _ => return FLUID_FAILED,
        };
        let pos = base.wrapping_add(offset);
        if pos < 0 {
            return FLUID_FAILED;
        }
        self.pos = pos.min(self.data.len() as i64);
        FLUID_OK
    }

    fn tell(&mut self) -> i64 {
        self.pos
    }

    fn close(self: Box<Self>) -> i32 {
        FLUID_OK
    }
}

/// `(name, bytes)` of each of the files in testdata/.
macro_rules! testdata {
    [$($name:literal),* $(,)?] => {
        &[$(($name, include_bytes!(concat!("../testdata/", $name)) as &[u8])),*]
    };
}

/// The SoundFonts and MIDI files of testdata/, in name order, embedded:
/// the tests of the Emscripten build see only their own (in-memory) file
/// system.
const TESTDATA: &[(&str, &[u8])] = testdata![
    "midi/bad_eot.mid",
    "midi/bad_keysig.mid",
    "midi/bad_mthd_len.mid",
    "midi/bad_smpte_ofs.mid",
    "midi/bad_tempo.mid",
    "midi/bad_timesig.mid",
    "midi/bad_varlen.mid",
    "midi/basic.mid",
    "midi/big_param.mid",
    "midi/empty.mid",
    "midi/fast.mid",
    "midi/format1.mid",
    "midi/format2.mid",
    "midi/hanging.mid",
    "midi/header_only.mid",
    "midi/huge_meta.mid",
    "midi/huge_sysex.mid",
    "midi/junk_chunk.mid",
    "midi/long_track.mid",
    "midi/many.mid",
    "midi/mono.mid",
    "midi/more_tracks.mid",
    "midi/neg_tracks.mid",
    "midi/negative_chunk.mid",
    "midi/no_eot.mid",
    "midi/no_status.mid",
    "midi/nonascii_chunk.mid",
    "midi/not_midi.mid",
    "midi/notracks.mid",
    "midi/nrpn.mid",
    "midi/short_header.mid",
    "midi/short_track.mid",
    "midi/slow.mid",
    "midi/smpte.mid",
    "midi/sys_common.mid",
    "midi/sysex.mid",
    "midi/truncated.mid",
    "sf2_badinfo.sf2",
    "sf2_badinst.sf2",
    "sf2_badsamples.sf2",
    "sf2_badsize.sf2",
    "sf2_badsm24.sf2",
    "sf2_extra_pdta.sf2",
    "sf2_header.sf2",
    "sf2_noimod.sf2",
    "sf2_nopdta.sf2",
    "sf2_noshdr.sf2",
    "sf2_oddinfo.sf2",
    "sf2_truncated.sf2",
    "sf2_version1.sf2",
    "sf2_version3.sf2",
    "sf2_version9.sf2",
    "test.sf2",
    "test_v201.sf2",
];

/// A file of testdata/ ([`TESTDATA`]), or of `FSREF_DIR`.
fn testdata(name: &str) -> Option<Vec<u8>> {
    if let Ok(dir) = std::env::var("FSREF_DIR") {
        return std::fs::read(dir + "/" + name).ok();
    }
    TESTDATA
        .iter()
        .find(|&&(n, _)| n == name)
        .map(|&(_, data)| data.to_vec())
}

/// [`TESTDATA`] has every SoundFont and MIDI file of testdata/.
#[test]
#[cfg_attr(target_os = "emscripten", ignore = "host file system")]
fn testdata_is_every_file() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata");
    let mut names = Vec::new();
    for sub in ["", "midi/"] {
        for e in std::fs::read_dir(dir.join(sub)).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_file() {
                let name = format!("{sub}{}", e.file_name().into_string().unwrap());
                if !name.ends_with(".txt") {
                    names.push(name);
                }
            }
        }
    }
    names.sort();
    let embedded: Vec<&str> = TESTDATA.iter().map(|&(name, _)| name).collect();
    assert_eq!(names, embedded);
}

/// C's atof() of the simple numbers in cases.txt.
fn atof(s: &str) -> f64 {
    s.parse().unwrap_or(0.0)
}

/// C's atoi() of the simple numbers in cases.txt.
fn atoi(s: &str) -> i32 {
    s.parse().unwrap_or(0)
}

fn apply_setting(settings: &Settings, kv: &str) {
    let Some((k, v)) = kv.split_once('=') else {
        return;
    };
    match settings.get_type(k) {
        FluidType::Num => {
            settings.setnum(k, atof(v));
        }
        FluidType::Int => {
            settings.setint(k, atoi(v));
        }
        FluidType::Str => {
            settings.setstr(k, Some(v));
        }
        _ => {}
    }
}

/// One line of cases.txt, rendered as the reference program does.
fn run_case(line: &str, out: &mut String) {
    let tok: Vec<&str> = line.split_whitespace().collect();
    if tok.len() < 5 {
        return;
    }
    let rate = atoi(tok[3]);
    let maxframes: i64 = tok[4].parse().unwrap();
    let sf = if tok[1] != "-" {
        testdata(tok[1])
    } else {
        None
    };
    let mid = testdata(tok[2]);

    let settings = Settings::new();
    settings.setnum("synth.gain", 1.0);
    settings.setnum("synth.sample-rate", rate as f64);
    let mut sr = 0.0;
    settings.getnum("synth.sample-rate", &mut sr);
    settings.setnum("synth.chorus.depth", 5.0);
    settings.setnum("synth.chorus.level", 0.35);
    settings.setnum("synth.reverb.damp", 0.4);
    settings.setnum("synth.reverb.level", 0.15);
    settings.setnum("synth.reverb.width", 4.0);
    settings.setnum("synth.reverb.room-size", 0.6);
    for kv in &tok[5..] {
        apply_setting(&settings, kv);
    }
    let Some(synth) = Synth::new(&settings) else {
        out.push_str(&capture_take());
        let _ = writeln!(out, "{} synth=fail", tok[0]);
        return;
    };
    let mut sfid = -2;
    if let Some(sf) = sf {
        let mut loader = SfLoader::new_defsfloader(&settings);
        let data: Arc<[u8]> = sf.into();
        loader.set_callbacks(Arc::new(move |name: &str| -> Option<Box<dyn SfFile>> {
            if name != "&FAKESFNAME&" {
                return None;
            }
            Some(Box::new(MemFile {
                data: data.clone(),
                pos: 0,
            }))
        }));
        synth.add_sfloader(loader);
        sfid = synth.sfload("&FAKESFNAME&", true);
    }
    let Some(player) = Player::new(&synth) else {
        out.push_str(&capture_take());
        let _ = writeln!(out, "{} sf={} player=fail", tok[0], sfid);
        return;
    };
    let add = player.add_mem(mid.as_deref().unwrap_or(&[]));
    let play = player.play();
    let mut frames: i64 = 0;
    let mut h = FNV0;
    let mut samples = [0f32; 256];
    let trace = std::env::var_os("FSREF_TRACE").is_some();
    while frames < maxframes && player.get_status() == FLUID_PLAYER_PLAYING {
        let wr = synth.write_float(128, &mut samples, 0, 2, None, 1, 2);
        if wr != FLUID_OK {
            break;
        }
        for s in &samples {
            h = fnv(h, &s.to_le_bytes());
        }
        frames += 128;
        if trace && frames % 4096 == 0 {
            eprintln!("{} {} {:016x}", tok[0], frames, h);
        }
    }
    out.push_str(&capture_take());
    let _ = writeln!(
        out,
        "{} sf={} add={} play={} frames={} status={} ticks={} total={} voices={} hash={:016x}",
        tok[0],
        sfid,
        add,
        play,
        frames,
        player.get_status(),
        player.get_current_tick(),
        player.get_total_ticks(),
        synth.get_active_voice_count(),
        h
    );
    player.stop();
    drop(player);
    drop(synth);
    out.push_str(&capture_take());
}

/// The tests that count the global sample cache's entries run one at a
/// time with the ones that load SoundFonts.
static SYNTH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn reference() {
    let _lock = SYNTH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    capture_start();
    let mut out = String::new();
    // (FSREF_CASES, FSREF_DIR and FSREF_DUMP run other cases, for debugging)
    let cases = match std::env::var("FSREF_CASES") {
        Ok(path) => std::fs::read_to_string(path).unwrap(),
        Err(_) => CASES.to_owned(),
    };
    for line in cases.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        run_case(line, &mut out);
    }
    capture_stop();
    if let Some(path) = std::env::var_os("FSREF_DUMP") {
        std::fs::write(path, &out).unwrap();
        if std::env::var_os("FSREF_CASES").is_some() {
            return;
        }
    }
    let mut failures = 0;
    let got: Vec<&str> = out.lines().collect();
    let want: Vec<&str> = REFERENCE.lines().collect();
    for i in 0..got.len().max(want.len()) {
        let g = got.get(i).copied().unwrap_or("<missing>");
        let w = want.get(i).copied().unwrap_or("<missing>");
        if g != w {
            failures += 1;
            if failures <= 20 {
                eprintln!("line {}:\n  got  {}\n  want {}", i + 1, g, w);
            }
        }
    }
    assert_eq!(failures, 0, "{} lines differ from the reference", failures);
}

/// The gentables computed at run time give the tables upstream's C++
/// compiler computes at build time with gcem (FNV-1a hashes of the C
/// tables' bytes).
#[test]
fn gentables() {
    use crate::gentables::*;
    let hash = |t: &[f64]| t.iter().fold(FNV0, |h, v| fnv(h, &v.to_le_bytes()));
    assert_eq!(
        (FLUID_CT2HZ_TAB.len(), hash(&FLUID_CT2HZ_TAB)),
        (1200, 0x69590de37e334d63)
    );
    assert_eq!(
        (FLUID_CB2AMP_TAB.len(), hash(&FLUID_CB2AMP_TAB)),
        (1441, 0x6596a1fc3b193880)
    );
    assert_eq!(
        (FLUID_CONCAVE_TAB.len(), hash(&FLUID_CONCAVE_TAB)),
        (128, 0xac88ceb12a752c14)
    );
    assert_eq!(
        (FLUID_CONVEX_TAB.len(), hash(&FLUID_CONVEX_TAB)),
        (128, 0xd6555a4fdd181994)
    );
    assert_eq!(
        (FLUID_PAN_TAB.len(), hash(&FLUID_PAN_TAB)),
        (1002, 0xb6256f864e030aee)
    );
    assert_eq!(
        (INTERP_COEFF.len(), hash(&INTERP_COEFF)),
        (1024, 0x53547415310c50e4)
    );
    assert_eq!(
        (INTERP_COEFF_LINEAR.len(), hash(&INTERP_COEFF_LINEAR)),
        (512, 0xa4a7c903fdfb1448)
    );
    assert_eq!(
        (SINC_TABLE7.len(), hash(&SINC_TABLE7)),
        (256 * 7, 0xba23a7372e1bd0c4)
    );
}

fn synth_with(sf: &Arc<[u8]>) -> Synth {
    let settings = Settings::new();
    let synth = Synth::new(&settings).unwrap();
    let mut loader = SfLoader::new_defsfloader(&settings);
    let data = sf.clone();
    loader.set_callbacks(Arc::new(move |_: &str| -> Option<Box<dyn SfFile>> {
        Some(Box::new(MemFile {
            data: data.clone(),
            pos: 0,
        }))
    }));
    synth.add_sfloader(loader);
    assert_eq!(synth.sfload("&FAKESFNAME&", true), 1);
    synth
}

/// The sample cache keeps a SoundFont's sample data once for the synths
/// that load it (by name), and frees it with the last one.
#[test]
fn samplecache() {
    use crate::sfloader::samplecache::fluid_samplecache_count_entries;
    let _lock = SYNTH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let sf: Arc<[u8]> = testdata("test.sf2").unwrap().into();
    assert_eq!(fluid_samplecache_count_entries(), 0);
    let a = synth_with(&sf);
    let n = fluid_samplecache_count_entries();
    assert!(n > 0);
    let b = synth_with(&sf);
    assert_eq!(fluid_samplecache_count_entries(), n);
    drop(a);
    assert_eq!(fluid_samplecache_count_entries(), n);
    drop(b);
    assert_eq!(fluid_samplecache_count_entries(), 0);
}

/// The public API's channel messages reach the synth, and a SYSEX
/// message reports whether it was handled.
#[test]
fn api() {
    let _lock = SYNTH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let sf: Arc<[u8]> = testdata("test.sf2").unwrap().into();
    let synth = synth_with(&sf);
    assert_eq!(synth.noteon(0, 60, 100), FLUID_OK);
    assert_eq!(synth.noteon(0, 128, 100), FLUID_FAILED);
    assert_eq!(synth.noteon(16, 60, 100), FLUID_FAILED);
    let mut buf = [0f32; 512];
    assert_eq!(synth.write_float(256, &mut buf, 0, 2, None, 1, 2), FLUID_OK);
    assert!(buf.iter().any(|&v| v != 0.0));
    assert!(synth.get_active_voice_count() > 0);
    assert_eq!(
        synth.write_float(257, &mut buf, 0, 2, None, 1, 2),
        FLUID_FAILED
    );
    let (mut l, mut r) = ([0f32; 64], [0f32; 64]);
    assert_eq!(
        synth.write_float(64, &mut l, 0, 1, Some(&mut r), 0, 1),
        FLUID_OK
    );
    assert_eq!(synth.cc(0, 7, 20), FLUID_OK);
    assert_eq!(synth.pitch_bend(0, 0x3000), FLUID_OK);
    assert_eq!(synth.program_change(1, 2), FLUID_OK);
    assert_eq!(synth.channel_pressure(0, 30), FLUID_OK);
    assert_eq!(synth.key_pressure(0, 60, 30), FLUID_OK);
    assert_eq!(
        synth.sysex(&[0x7E, 0x7F, 0x09, 0x01], true),
        (FLUID_OK, true)
    );
    assert_eq!(synth.sysex(&[0x7D, 0x01], false), (FLUID_OK, false));
    assert_eq!(synth.noteoff(0, 60), FLUID_OK);
    assert_eq!(synth.all_notes_off(-1), FLUID_OK);
    assert_eq!(synth.all_sounds_off(-1), FLUID_OK);
    assert_eq!(synth.system_reset(), FLUID_OK);
    assert_eq!(synth.program_reset(), FLUID_OK);
    let mut ev = FluidMidiEvent::new();
    ev.set_type(0x90);
    ev.set_channel(2);
    ev.set_key(64);
    ev.set_velocity(90);
    assert_eq!(synth.handle_midi_event(&ev), FLUID_OK);
    ev.set_sysex(&[0x7E, 0x7F, 0x09, 0x01]);
    assert_eq!(synth.handle_midi_event(&ev), FLUID_OK);
    assert_eq!(audio_driver_register(None), FLUID_OK);
    assert_eq!(audio_driver_register(Some(&[])), FLUID_OK);
    assert_eq!(audio_driver_register(Some(&["alsa"])), FLUID_FAILED);
    assert_eq!(fluid_version(), (2, 5, 7));
    assert_eq!(fluid_version_str(), "2.5.7");
}

/// The player's seek, stop and loop: seeking while it plays takes effect
/// at the next block, and a stopped player plays on from where it was.
#[test]
fn player() {
    let _lock = SYNTH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let sf: Arc<[u8]> = testdata("test.sf2").unwrap().into();
    let synth = synth_with(&sf);
    let player = Player::new(&synth).unwrap();
    assert_eq!(player.get_status(), FLUID_PLAYER_READY);
    assert_eq!(player.play(), FLUID_OK); // nothing to play: still ready
    assert_eq!(player.get_status(), FLUID_PLAYER_READY);
    assert_eq!(
        player.add_mem(&testdata("midi/basic.mid").unwrap()),
        FLUID_OK
    );
    assert_eq!(player.seek(100000), FLUID_OK); // not playing yet: any position
    assert_eq!(player.play(), FLUID_OK);
    assert_eq!(player.get_status(), FLUID_PLAYER_PLAYING);
    let mut buf = [0f32; 256];
    for _ in 0..20 {
        synth.write_float(128, &mut buf, 0, 2, None, 1, 2);
    }
    assert_eq!(player.get_total_ticks(), 228);
    assert_eq!(player.seek(-1), FLUID_FAILED);
    assert_eq!(player.seek(229), FLUID_FAILED);
    assert_eq!(player.seek(10), FLUID_OK);
    assert_eq!(player.seek(20), FLUID_FAILED); // the first is still pending
    synth.write_float(128, &mut buf, 0, 2, None, 1, 2);
    assert_eq!(player.get_current_tick(), 10);
    assert_eq!(player.stop(), FLUID_OK);
    assert_eq!(player.get_status(), FLUID_PLAYER_DONE);
    synth.write_float(128, &mut buf, 0, 2, None, 1, 2);
    assert_eq!(player.set_loop(2), FLUID_OK);
    assert_eq!(player.play(), FLUID_OK);
    let mut blocks = 0;
    while player.get_status() == FLUID_PLAYER_PLAYING && blocks < 100000 {
        synth.write_float(128, &mut buf, 0, 2, None, 1, 2);
        blocks += 1;
    }
    assert_eq!(player.get_status(), FLUID_PLAYER_DONE);
    // two more times through the song (and its 2 s grace periods)
    assert!(blocks > 1300, "{}", blocks);
    drop(player);
    drop(synth);
}
