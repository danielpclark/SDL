// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/
// Copyright (C) 2003-2006 Shay Green, and its other authors; see NOTICE.
// Under the GNU Lesser General Public License, version 2.1 or later (see COPYING).
// Modified 2026-10-17: tests of the Rust translation of SDL_mixer 3.3.0's Game_Music_Emu 0.6.5 (see NOTICE).

//! `testdata/reference.txt` is the output of a C++ program linked with the
//! libgme that SDL_mixer vendors, built as SDL_mixer's CMake builds it
//! (patched where the translation works around its out-of-bounds reads and
//! uninitialized memory: the `FIXME (upstream)` notes; and with its math
//! routed through SDL's bundled fdlibm, as sdl3's is), which this repeats
//! on the translation and compares line by line. For each file in
//! `testdata/` (made by `tools/gen_gme_testdata.py`) it records the
//! identified type, the track count and every track's information as an
//! info-only reader and as an emulator, voices and equalizer, the first
//! two seconds of the first tracks (hashes of the samples, with the
//! position and whether the track ended), track switching, seeks forward
//! and back, a fade to the end, and another rate with tempo, stereo
//! depth, equalizer and muting changes; then the same, briefly, for
//! truncated and corrupted copies.

use std::fmt::Write;
use std::path::{Path, PathBuf};

use sdl3_mixer_gme::*;

const FNV0: u64 = 14695981039346656037;

fn fnv(mut h: u64, data: &[i16]) -> u64 {
    for s in data {
        for b in s.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(1099511628211);
        }
    }
    h
}

fn esc(s: &[u8]) -> String {
    let mut o = String::new();
    for &c in s {
        if (0x20..0x7F).contains(&c) && c != b'\\' && c != b'|' {
            o.push(c as char);
        } else {
            write!(o, "\\x{c:02X}").unwrap();
        }
    }
    o
}

fn errs(e: GmeErr) -> String {
    match e {
        Ok(()) => "ok".into(),
        Err(e) => e.into(),
    }
}

fn print_info(out: &mut String, emu: &dyn MusicEmu, t: i32) {
    match gme_track_info(emu, t) {
        Err(e) => writeln!(out, "info {t}: err={e}").unwrap(),
        Ok(i) => writeln!(
            out,
            "info {t}: {} {} {} {} {}|{}|{}|{}|{}|{}|{}|{}",
            i.length,
            i.intro_length,
            i.loop_length,
            i.fade_length,
            i.play_length,
            esc(&i.system),
            esc(&i.game),
            esc(&i.song),
            esc(&i.author),
            esc(&i.copyright),
            esc(&i.comment),
            esc(&i.dumper)
        )
        .unwrap(),
    }
}

fn print_warning(out: &mut String, emu: &mut dyn MusicEmu) {
    if let Some(w) = gme_warning(emu) {
        writeln!(out, "warning: {w}").unwrap();
    }
}

/// play n samples in blocks of `block`, printing a hash every `every` samples
fn play(out: &mut String, emu: &mut dyn MusicEmu, n: i64, block: i32, every: i64, label: &str) {
    let mut buf = vec![0i16; block as usize];
    let mut h = FNV0;
    let mut done = 0i64;
    let mut since = 0i64;
    while done < n {
        let mut c = block as i64;
        if c > n - done {
            c = n - done;
        }
        let err = gme_play(emu, &mut buf[..c as usize]);
        h = fnv(h, &buf[..c as usize]);
        done += c;
        since += c;
        if let Err(e) = err {
            writeln!(out, "{label}: play err={e}").unwrap();
        }
        if since >= every || done >= n {
            writeln!(
                out,
                "{label} {done}: {h:016x} tell={} samples={} ended={}",
                gme_tell(emu),
                gme_tell_samples(emu),
                gme_track_ended(emu)
            )
            .unwrap();
            since = 0;
        }
    }
    print_warning(out, emu);
}

fn open_print(out: &mut String, data: &[u8], rate: i32, label: &str) -> Option<Box<dyn MusicEmu>> {
    match gme_open_data(data, rate) {
        Err(e) => {
            writeln!(out, "{label}: err={e}").unwrap();
            None
        }
        Ok(mut emu) => {
            writeln!(
                out,
                "{label}: tracks={} type={} voices={}",
                gme_track_count(emu.as_ref()),
                gme_type_extension(Some(gme_type(emu.as_ref()))),
                if rate == GME_INFO_ONLY {
                    -1
                } else {
                    gme_voice_count(emu.as_ref())
                }
            )
            .unwrap();
            print_warning(out, emu.as_mut());
            Some(emu)
        }
    }
}

fn info_test(out: &mut String, data: &[u8], max_tracks: i32) {
    let Some(mut emu) = open_print(out, data, GME_INFO_ONLY, "info") else {
        return;
    };
    let mut n = gme_track_count(emu.as_ref());
    if n > max_tracks {
        n = max_tracks;
    }
    for t in 0..n {
        print_info(out, emu.as_ref(), t);
    }
    print_info(out, emu.as_ref(), gme_track_count(emu.as_ref())); // past the end
    print_warning(out, emu.as_mut());
}

fn full_test(out: &mut String, data: &[u8], secs_samples: i64) {
    info_test(out, data, 8);

    let Some(mut emu) = open_print(out, data, 44100, "open 44100") else {
        return;
    };
    let emu = emu.as_mut();
    {
        let mut names = String::new();
        for i in 0..gme_voice_count(emu) {
            names += " ";
            names += &esc(gme_voice_name(emu, i).as_bytes());
        }
        writeln!(out, "voices:{names}").unwrap();
        let eq = gme_equalizer(emu);
        writeln!(
            out,
            "eq: {:.3} {:.3} multi={}",
            eq.treble,
            eq.bass,
            gme_multi_channel(emu)
        )
        .unwrap();
    }
    let tracks = gme_track_count(emu);
    let nt = tracks.min(3);
    for t in 0..nt {
        let err = gme_start_track(emu, t);
        writeln!(out, "start {t}: {}", errs(err)).unwrap();
        print_warning(out, emu);
        print_info(out, emu, t);
        play(out, emu, secs_samples, 1024, 16384, &format!("t{t}"));
    }
    // track switching past the end
    writeln!(
        out,
        "start {tracks}: {}",
        errs(gme_start_track(emu, tracks))
    )
    .unwrap();
    writeln!(out, "start -1: {}", errs(gme_start_track(emu, -1))).unwrap();

    // seeks
    if tracks > 0 {
        writeln!(out, "start 0: {}", errs(gme_start_track(emu, 0))).unwrap();
        play(out, emu, 4096, 512, 4096, "pre");
        let e = errs(gme_seek_samples(emu, 30000));
        writeln!(
            out,
            "seek_samples 30000: {e} tell={}",
            gme_tell_samples(emu)
        )
        .unwrap();
        play(out, emu, 4096, 512, 4096, "s1");
        let e = errs(gme_seek_samples(emu, 10000));
        writeln!(
            out,
            "seek_samples 10000: {e} tell={}",
            gme_tell_samples(emu)
        )
        .unwrap();
        play(out, emu, 4096, 512, 4096, "s2");
        let e = errs(gme_seek(emu, 700));
        writeln!(out, "seek 700: {e} tell={}", gme_tell(emu)).unwrap();
        play(out, emu, 2048, 2048, 2048, "s3");
        let e = errs(gme_seek_samples(emu, 200000));
        writeln!(
            out,
            "seek_samples 200000: {e} tell={} ended={}",
            gme_tell_samples(emu),
            gme_track_ended(emu)
        )
        .unwrap();
        play(out, emu, 2048, 2048, 2048, "s4");

        // fade
        writeln!(out, "start 0: {}", errs(gme_start_track(emu, 0))).unwrap();
        gme_set_fade_msecs(emu, 150, 250);
        let mut n = 0i64;
        let mut h = FNV0;
        let mut buf = vec![0i16; 1024];
        while gme_track_ended(emu) == 0 && n < 44100 * 2 * 2 {
            let _ = gme_play(emu, &mut buf);
            h = fnv(h, &buf);
            n += 1024;
        }
        writeln!(out, "fade: {n} {h:016x} tell={}", gme_tell(emu)).unwrap();
        let _ = gme_play(emu, &mut buf);
        writeln!(out, "after: {:016x}", fnv(FNV0, &buf)).unwrap();
        gme_set_fade(emu, 100000);
        writeln!(out, "start 0: {}", errs(gme_start_track(emu, 0))).unwrap();
        play(out, emu, 8192, 1024, 8192, "nofade");
    }

    // other settings, at another rate
    let Some(mut emu) = open_print(out, data, 32000, "open 32000") else {
        return;
    };
    let emu = emu.as_mut();
    if tracks > 0 {
        gme_set_autoload_playback_limit(emu, 0);
        gme_ignore_silence(emu, 1);
        gme_set_tempo(emu, 1.7);
        gme_set_stereo_depth(emu, 0.8);
        let mut eq = gme_equalizer(emu);
        eq.treble = -20.0;
        eq.bass = 200.0;
        gme_set_equalizer(emu, &eq);
        gme_mute_voices(emu, 0x5);
        let last = tracks - 1;
        writeln!(out, "start {last}: {}", errs(gme_start_track(emu, last))).unwrap();
        play(out, emu, 32000, 1000, 16000, "tempo");
        writeln!(out, "tell_scaled={}", gme_tell_scaled(emu)).unwrap();
        let e = errs(gme_seek_scaled(emu, 300));
        writeln!(
            out,
            "seek_scaled 300: {e} tell_scaled={}",
            gme_tell_scaled(emu)
        )
        .unwrap();
        play(out, emu, 4000, 1000, 4000, "sc");
        gme_mute_voice(emu, 0, 0);
        gme_mute_voice(emu, 1, 1);
        gme_set_tempo(emu, 0.6);
        gme_set_stereo_depth(emu, 0.0);
        play(out, emu, 8000, 1000, 8000, "slow");
        gme_enable_accuracy(emu, 1);
        gme_disable_echo(emu, 1);
        gme_clear_playlist(emu);
        writeln!(out, "tracks={}", gme_track_count(emu)).unwrap();
        writeln!(out, "start 0: {}", errs(gme_start_track(emu, 0))).unwrap();
        play(out, emu, 8000, 1000, 8000, "acc");
    }
}

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0
    }
}

fn quick_test(out: &mut String, data: &[u8]) {
    info_test(out, data, 2);
    let Some(mut emu) = open_print(out, data, 44100, "open") else {
        return;
    };
    let emu = emu.as_mut();
    if gme_track_count(emu) > 0 {
        writeln!(out, "start 0: {}", errs(gme_start_track(emu, 0))).unwrap();
        print_warning(out, emu);
        play(out, emu, 8192, 1024, 8192, "q");
    }
}

fn misc(out: &mut String) {
    writeln!(out, "== misc").unwrap();
    write!(out, "types:").unwrap();
    for t in gme_type_list() {
        write!(
            out,
            " {}/{}/{}/{}",
            gme_type_extension(Some(t)),
            gme_type_system(t),
            gme_type_multitrack(t),
            gme_fixed_track_count(t)
        )
        .unwrap();
    }
    writeln!(out).unwrap();
    let exts = [
        "nsf", "x.NSFE", "a.b.spc", "vgz", "VGM", "gym", "kss", "hes", "sap", "ay", "gbs", "nsfee",
        "toolong", "", ".", "song.mp3",
    ];
    for e in exts {
        writeln!(
            out,
            "ext {e}: {}",
            gme_type_extension(gme_identify_extension(e.as_bytes()))
        )
        .unwrap();
    }
    let heads: [&[u8]; 14] = [
        b"ZXAY",
        b"GBS\x01",
        b"GYMX",
        b"HESM",
        b"KSCC",
        b"KSSX",
        b"NESM",
        b"NSFE",
        b"SAP\x0D",
        b"SNES",
        b"Vgm ",
        b"\x1F\x8B\x08\x00",
        b"RIFF",
        b"GBS\x02",
    ];
    for (i, h) in heads.iter().enumerate() {
        writeln!(out, "header {i}: {}", gme_identify_header(h)).unwrap();
    }
    writeln!(
        out,
        "open short: {}",
        errs(gme_open_data(b"NES", 44100).map(|_| ()))
    )
    .unwrap();
    writeln!(
        out,
        "open junk: {}",
        errs(gme_open_data(b"JUNKJUNK", 44100).map(|_| ()))
    )
    .unwrap();
}

fn file_test(out: &mut String, name: &str, d: &[u8]) {
    writeln!(out, "== {name}").unwrap();
    writeln!(out, "identify: {}", gme_identify_header(d)).unwrap();
    full_test(out, d, 44100 * 2);

    // truncated copies
    let cuts = [
        4,
        16,
        d.len() as i64 / 3,
        d.len() as i64 / 2,
        d.len() as i64 - 1,
    ];
    for n in cuts {
        if n < 1 || n >= d.len() as i64 {
            continue;
        }
        writeln!(out, "-- cut {n}").unwrap();
        quick_test(out, &d[..n as usize]);
    }
    // corrupted copies
    let mut lcg = Lcg(1234);
    for k in 0..6 {
        let mut t = d.to_vec();
        let flips = 1 + k * 3;
        for _ in 0..flips {
            let mut pos = (lcg.next() % t.len() as u32) as usize;
            if pos < 4 {
                pos += 4; // keep the magic
            }
            if pos >= t.len() {
                continue;
            }
            t[pos] ^= ((lcg.next() >> 24) as u8) | 1;
        }
        writeln!(out, "-- flip {k}").unwrap();
        quick_test(out, &t);
    }
}

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

/// The reference's lines for a section (`== name` up to the next one).
fn reference_section(name: &str) -> String {
    let reference = std::fs::read(testdata().join("reference.txt")).unwrap();
    let reference = String::from_utf8_lossy(&reference);
    let mut want = String::new();
    let mut keep = false;
    for line in reference.lines() {
        if let Some(n) = line.strip_prefix("== ") {
            keep = n == name;
        }
        if keep {
            want.push_str(line);
            want.push('\n');
        }
    }
    assert!(!want.is_empty(), "no reference for {name}");
    want
}

fn compare(out: &str, name: &str) {
    let want = reference_section(name);
    if out == want {
        return;
    }
    let mut report = String::new();
    let mut shown = 0;
    for (i, (a, b)) in out.lines().zip(want.lines()).enumerate() {
        if a != b {
            writeln!(report, "line {i}:\n  got:  {a}\n  want: {b}").unwrap();
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
    panic!("{name}: output differs from upstream's:\n{report}");
}

fn check_file(name: &str) {
    let d = std::fs::read(testdata().join(name)).unwrap();
    let mut out = String::new();
    file_test(&mut out, name, &d);
    compare(&out, name);
}

#[test]
fn misc_matches_upstream() {
    let mut out = String::new();
    misc(&mut out);
    compare(&out, "misc");
}

macro_rules! file_tests {
    ($($name:ident: $file:literal,)*) => {
        $(
            #[test]
            fn $name() {
                check_file($file);
            }
        )*
    };
}

file_tests! {
    ay_badtracks_matches_upstream: "ay_badtracks.ay",
    ay_cpc_matches_upstream: "ay_cpc.ay",
    ay_odd_matches_upstream: "ay_odd.ay",
    ay_spectrum_matches_upstream: "ay_spectrum.ay",
    gbs_apu_matches_upstream: "gbs_apu.gbs",
    gbs_odd_matches_upstream: "gbs_odd.gbs",
    gbs_timer_matches_upstream: "gbs_timer.gbs",
    gbs_timer2_matches_upstream: "gbs_timer2.gbs",
    hes_extra_matches_upstream: "hes_extra.hes",
    hes_multi_matches_upstream: "hes_multi.hes",
    hes_odd_matches_upstream: "hes_odd.hes",
    hes_psg_matches_upstream: "hes_psg.hes",
    kss_ext_matches_upstream: "kss_ext.kss",
    kss_gg_matches_upstream: "kss_gg.kss",
    kss_odd_matches_upstream: "kss_odd.kss",
    kss_scc_matches_upstream: "kss_scc.kss",
    kss_sms_matches_upstream: "kss_sms.kss",
    nsf_apu_matches_upstream: "nsf_apu.nsf",
    nsf_badaddr_matches_upstream: "nsf_badaddr.nsf",
    nsf_banks_matches_upstream: "nsf_banks.nsf",
    nsf_exp_matches_upstream: "nsf_exp.nsf",
    nsf_odd_matches_upstream: "nsf_odd.nsf",
    nsf_vrc7_matches_upstream: "nsf_vrc7.nsf",
    nsfe_full_matches_upstream: "nsfe_full.nsfe",
    nsfe_min_matches_upstream: "nsfe_min.nsfe",
    nsfe_order_matches_upstream: "nsfe_order.nsfe",
}

/// Every file in `testdata/` has its test above.
#[test]
fn every_file_is_tested() {
    let mut names: Vec<String> = std::fs::read_dir(testdata())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != "reference.txt")
        .collect();
    names.sort();
    let reference = std::fs::read(testdata().join("reference.txt")).unwrap();
    let reference = String::from_utf8_lossy(&reference);
    let sections: Vec<&str> = reference
        .lines()
        .filter_map(|l| l.strip_prefix("== "))
        .filter(|n| *n != "misc")
        .collect();
    assert_eq!(names, sections);
}
