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

//! Corrupt SoundFonts, DLS files and MIDI files must load or fail, and
//! play, without panicking (where upstream's C would read or write out of
//! bounds or spin, the translation does something defined; see the
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
    ("test.dls", include_bytes!("../testdata/test.dls")),
    ("dls_art.dls", include_bytes!("../testdata/dls_art.dls")),
    ("dls_pgal.dls", include_bytes!("../testdata/dls_pgal.dls")),
    (
        "dls_pgal_nover.dls",
        include_bytes!("../testdata/dls_pgal_nover.dls"),
    ),
    (
        "dls_samples.dls",
        include_bytes!("../testdata/dls_samples.dls"),
    ),
    ("dls_sort.dls", include_bytes!("../testdata/dls_sort.dls")),
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
    (
        "midi/dls_basic.mid",
        include_bytes!("../testdata/midi/dls_basic.mid"),
    ),
    (
        "midi/dls_drums.mid",
        include_bytes!("../testdata/midi/dls_drums.mid"),
    ),
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
    play_song(&synth, mid, frames);
}

/// Load the DLS file by its path (a copy in the temporary directory),
/// through the synth's own loaders, the SoundFont 2 loader and then the
/// DLS loader (the public API has no DLS loader over memory), and play
/// the song as [`play()`] does.
fn play_dls(dls: &[u8], mid: &[u8], frames: usize) {
    let path = std::env::temp_dir().join(format!(
        "sdl3-mixer-fluidsynth-robustness-{}.dls",
        std::process::id()
    ));
    std::fs::write(&path, dls).unwrap();
    let settings = Settings::new();
    settings.setnum("synth.sample-rate", 22050.0);
    let synth = Synth::new(&settings).unwrap();
    synth.sfload(path.to_str().unwrap(), true);
    play_song(&synth, mid, frames);
    // (a DLS font keeps its file open until it goes)
    drop(synth);
    let _ = std::fs::remove_file(&path);
}

/// Play the song for up to `frames` frames, seeking back once.
fn play_song(synth: &Synth, mid: &[u8], frames: usize) {
    let player = Player::new(synth).unwrap();
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

/// The chunks of a RIFF form: the offset of each header, its size, and
/// whether it's a LIST (or the form itself).
fn riff_chunks(data: &[u8]) -> Vec<(usize, u32, bool)> {
    fn walk(data: &[u8], mut pos: usize, end: usize, out: &mut Vec<(usize, u32, bool)>) {
        while pos + 8 <= end.min(data.len()) {
            let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap());
            let list = &data[pos..pos + 4] == b"LIST";
            out.push((pos, size, list));
            if list {
                walk(data, pos + 12, pos + 8 + size as usize, out);
            }
            pos += 8 + size as usize + (size & 1) as usize;
        }
    }
    let mut out = Vec::new();
    if data.len() >= 12 {
        let size = u32::from_le_bytes(data[4..8].try_into().unwrap());
        out.push((0, size, true));
        walk(data, 12, data.len(), &mut out);
    }
    out
}

/// A corrupted copy of a DLS file: bytes flipped (anywhere, or from the
/// instruments on), a chunk's size changed, or the file cut short.
///
/// FluidSynth allocates and zero-fills what a chunk's size (or the pool
/// table's count of cues) claims before it reads the chunk, up to 4 GiB
/// (see the `FIXME (upstream)` notes in sfloader/dls.rs), so whether a
/// large claim fails, and how long it takes, depends on the machine. Here
/// those fields only get sizes near the file's; the fuzz pass compared
/// larger ones with upstream's C under a limit on the address space.
fn corrupt_dls(data: &[u8], lcg: &mut Lcg, kind: usize) -> Vec<u8> {
    let chunks = riff_chunks(data);
    let mut fixed = Vec::new();
    for &(pos, _, _) in &chunks {
        fixed.extend(pos + 4..pos + 8);
        if &data[pos..pos + 4] == b"ptbl" {
            fixed.extend(pos + 12..pos + 16);
        }
    }
    let mut bad = data.to_vec();
    match kind {
        0 | 1 => {
            let start = match kind {
                0 => 0,
                _ => data.windows(4).position(|w| w == b"lins").unwrap(),
            };
            for _ in 0..1 + lcg.below(6) {
                let at = start + lcg.below(bad.len() - start);
                if !fixed.contains(&at) {
                    bad[at] ^= (1 + lcg.next() % 255) as u8;
                }
            }
        }
        2 => {
            let (pos, size, _) = chunks[lcg.below(chunks.len())];
            let size = match lcg.below(6) {
                0 => lcg.below(13) as u32,
                1 => size.saturating_sub(1 + lcg.below(4) as u32),
                2 => size + 1 + lcg.below(4) as u32,
                3 => size * 2,
                4 => size + lcg.below(4096) as u32,
                _ => data.len() as u32,
            };
            bad[pos + 4..pos + 8].copy_from_slice(&size.to_le_bytes());
        }
        _ => bad.truncate(lcg.below(data.len())),
    }
    bad
}

#[test]
fn corrupt_dls_files() {
    quiet();
    let mut lcg = Lcg(41);
    for (name, mid) in [
        ("test.dls", "dls_basic"),
        ("dls_art.dls", "dls_basic"),
        ("dls_pgal.dls", "dls_drums"),
        ("dls_pgal_nover.dls", "dls_drums"),
        ("dls_samples.dls", "dls_basic"),
        ("dls_sort.dls", "basic"),
    ] {
        let dls = testdata(name);
        let mid = testdata(&format!("midi/{}.mid", mid));
        for i in 0..16 {
            let bad = corrupt_dls(&dls, &mut lcg, i % 4);
            play_dls(&bad, &mid, 12000);
        }
    }
}
