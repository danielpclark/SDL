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

//! Corrupt SoundFonts and MIDI files must load or fail, and play, without
//! panicking (where upstream's C would read or write out of bounds or
//! spin, the translation does something defined; see the
//! `Note (upstream)` notes). The same kinds of corruption, many more of
//! them, rendered bit for bit as upstream's C renders them when it was
//! checked.

use std::sync::Arc;

use sdl3_mixer_fluidsynth::*;

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() >> 8) as usize % n.max(1)
    }
}

/// The files of testdata/ these tests read, embedded: the tests of the
/// Emscripten build see only their own (in-memory) file system.
const TESTDATA: &[(&str, &[u8])] = &[
    ("test.sf2", include_bytes!("../testdata/test.sf2")),
    (
        "midi/basic.mid",
        include_bytes!("../testdata/midi/basic.mid"),
    ),
    (
        "midi/format1.mid",
        include_bytes!("../testdata/midi/format1.mid"),
    ),
    (
        "midi/sysex.mid",
        include_bytes!("../testdata/midi/sysex.mid"),
    ),
    ("midi/mono.mid", include_bytes!("../testdata/midi/mono.mid")),
    ("midi/nrpn.mid", include_bytes!("../testdata/midi/nrpn.mid")),
    ("midi/many.mid", include_bytes!("../testdata/midi/many.mid")),
];

fn testdata(name: &str) -> Vec<u8> {
    let (_, data) = TESTDATA
        .iter()
        .find(|&&(n, _)| n == name)
        .unwrap_or_else(|| panic!("{name} isn't in TESTDATA"));
    data.to_vec()
}

/// Flip `flips` random bytes of `data` from `start` on.
fn corrupt(data: &[u8], lcg: &mut Lcg, start: usize, flips: usize) -> Vec<u8> {
    let mut bad = data.to_vec();
    for _ in 0..flips {
        let at = start + lcg.below(bad.len() - start);
        bad[at] ^= (1 + lcg.next() % 255) as u8;
    }
    bad
}

/// A SoundFont in memory.
struct MemFile {
    data: Arc<[u8]>,
    pos: i64,
}

impl SfFile for MemFile {
    fn read(&mut self, buf: &mut [u8]) -> i32 {
        let n = (buf.len() as i64)
            .min(self.data.len() as i64 - self.pos)
            .max(0);
        buf[..n as usize].copy_from_slice(&self.data[self.pos as usize..(self.pos + n) as usize]);
        self.pos += n;
        if n == buf.len() as i64 {
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

/// Load the SoundFont and play the song for up to `frames` frames, as
/// SDL_mixer's decoder does, seeking back once.
fn play(sf: &[u8], mid: &[u8], frames: usize) {
    let settings = Settings::new();
    settings.setnum("synth.sample-rate", 22050.0);
    let synth = Synth::new(&settings).unwrap();
    let mut loader = SfLoader::new_defsfloader(&settings);
    let data: Arc<[u8]> = sf.into();
    loader.set_callbacks(Arc::new(move |_: &str| -> Option<Box<dyn SfFile>> {
        Some(Box::new(MemFile {
            data: data.clone(),
            pos: 0,
        }))
    }));
    synth.add_sfloader(loader);
    synth.sfload("&FAKESFNAME&", true);
    let player = Player::new(&synth).unwrap();
    player.add_mem(mid);
    player.play();
    let mut buf = [0f32; 256];
    let mut done = 0;
    while done < frames && player.get_status() == FLUID_PLAYER_PLAYING {
        assert_eq!(synth.write_float(128, &mut buf, 0, 2, None, 1, 2), FLUID_OK);
        assert!(buf.iter().all(|v| v.is_finite()));
        done += 128;
        if done == 128 * 40 {
            player.seek(0);
        }
    }
    player.stop();
}

/// Silence the log: corrupt files log a lot.
fn quiet() {
    for level in 0..LAST_LOG_LEVEL {
        set_log_function(level, None);
    }
}

#[test]
fn corrupt_soundfonts() {
    quiet();
    let sf = testdata("test.sf2");
    let mid = testdata("midi/basic.mid");
    let pdta = sf.windows(4).position(|w| w == b"pdta").unwrap();
    let mut lcg = Lcg(17);
    for i in 0..60 {
        let n = 1 + lcg.below(6);
        let bad = match i % 3 {
            0 => corrupt(&sf, &mut lcg, pdta, n),
            1 => corrupt(&sf, &mut lcg, 0, n),
            _ => sf[..lcg.below(sf.len())].to_vec(),
        };
        play(&bad, &mid, 12000);
    }
}

#[test]
fn corrupt_midi_files() {
    quiet();
    let sf = testdata("test.sf2");
    let mut lcg = Lcg(29);
    for name in ["basic", "format1", "sysex", "mono", "nrpn", "many"] {
        let mid = testdata(&format!("midi/{}.mid", name));
        for i in 0..12 {
            let n = 1 + lcg.below(4);
            let bad = match i % 3 {
                0 => corrupt(&mid, &mut lcg, 8, n),
                1 => corrupt(&mid, &mut lcg, 0, n.min(2)),
                _ => mid[..lcg.below(mid.len())].to_vec(),
            };
            play(&sf, &bad, 12000);
        }
    }
}
