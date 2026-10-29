/*
    libmpg123: MPEG Audio Decoder library (version 1.33.7)

    copyright 1995-2023 by the mpg123 project
    free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
*/
// Modified 2026-10-17: tests of the Rust translation of SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

//! `testdata/reference.txt` is the output of a C program linked with the
//! libmpg123 that SDL_mixer vendors, built as SDL_mixer's CMake builds it
//! (but with its generic decoder: the assembly ones left out), which this
//! repeats on the translation and compares line by line. For each file
//! (the ones in `testdata/`, made by `tools/gen_mpg123_testdata.py`, and
//! sdl3-mixer's MPEG audio test files, made by
//! `tools/gen_sdl_mixer_testdata.py`, where the repository has them), it
//! records the format, frame info, state, scanned length, decoded output
//! and ID3 data of a handle set up as SDL_mixer's decoder sets it up;
//! exact and fuzzy seeks; each output encoding; a number of flags and
//! parameters; reading without a seek callback and with ICY metadata; and
//! the same (without the seeks and options) for truncated and corrupted
//! copies.

use std::fmt::Write;
use std::path::PathBuf;
use std::sync::Arc;

use sdl3_mixer_mpg123::mpg123::*;
use sdl3_mixer_mpg123::*;

struct Mem {
    d: Arc<Vec<u8>>,
    n: usize,
    pos: usize,
}

impl Mpg123Io for Mem {
    fn read(&mut self, dst: &mut [u8], got: &mut usize) -> i32 {
        let left = self.n - self.pos;
        let n = dst.len().min(left);
        dst[..n].copy_from_slice(&self.d[self.pos..self.pos + n]);
        self.pos += n;
        *got = n;
        0
    }

    fn lseek(&mut self, off: i64, whence: i32) -> i64 {
        let base = match whence {
            SEEK_SET => 0,
            SEEK_CUR => self.pos as i64,
            _ => self.n as i64,
        };
        let p = (base + off).clamp(0, self.n as i64);
        self.pos = p as usize;
        p
    }
}

const FNV0: u64 = 14695981039346656037;

fn fnv(mut h: u64, data: &[u8]) -> u64 {
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

const ENCS: [i32; 11] = [
    MPG123_ENC_SIGNED_16,
    MPG123_ENC_UNSIGNED_16,
    MPG123_ENC_SIGNED_32,
    MPG123_ENC_UNSIGNED_32,
    MPG123_ENC_SIGNED_24,
    MPG123_ENC_UNSIGNED_24,
    MPG123_ENC_FLOAT_32,
    MPG123_ENC_SIGNED_8,
    MPG123_ENC_UNSIGNED_8,
    MPG123_ENC_ULAW_8,
    MPG123_ENC_ALAW_8,
];

/// A handle set up as SDL_mixer's decoder_mpg123.c does (or with only
/// `enc` enabled, and `flags` added).
fn open_mem(
    data: &Arc<Vec<u8>>,
    n: usize,
    enc: i32,
    flags: i64,
    ret: &mut i32,
) -> Option<Box<Mpg123Handle<'static>>> {
    open_mem_p(&mut String::new(), data, n, enc, flags, true, None, ret)
}

/// `open_mem()` with or without the seek callback, and with a parameter
/// (`(key, val, fval)`) set first.
#[allow(clippy::too_many_arguments)]
fn open_mem_p(
    out: &mut String,
    data: &Arc<Vec<u8>>,
    n: usize,
    enc: i32,
    flags: i64,
    seekable: bool,
    param: Option<(i32, i64, f64)>,
    ret: &mut i32,
) -> Option<Box<Mpg123Handle<'static>>> {
    let mut h = match Mpg123Handle::new(None) {
        Ok(h) => h,
        Err(r) => {
            *ret = r;
            return None;
        }
    };
    h.param2(MPG123_ADD_FLAGS, MPG123_QUIET | flags, 0.0);
    if let Some((key, val, fval)) = param {
        write!(out, " p={}", h.param2(key, val, fval)).unwrap();
    }
    h.reader64(seekable);
    h.format_none();
    for &rate in mpg123_rates() {
        let formats = if enc != 0 {
            enc
        } else {
            MPG123_ENC_SIGNED_8
                | MPG123_ENC_UNSIGNED_8
                | MPG123_ENC_SIGNED_16
                | MPG123_ENC_SIGNED_32
                | MPG123_ENC_FLOAT_32
        };
        h.format(rate, MPG123_MONO | MPG123_STEREO, formats);
    }
    *ret = h.open_handle64(Box::new(Mem {
        d: data.clone(),
        n,
        pos: 0,
    }));
    Some(h)
}

/// Decode everything (at most `limit` bytes), in 1024-byte reads.
fn decode_all(out: &mut String, h: &mut Mpg123Handle<'_>, limit: usize) {
    let mut buf = [0u8; 1024];
    let mut hash = FNV0;
    let mut total = 0usize;
    let mut ret;
    let mut newfmt = 0;
    loop {
        let mut done = 0;
        ret = h.read(&mut buf, &mut done);
        hash = fnv(hash, &buf[..done]);
        total += done;
        if ret == MPG123_NEW_FORMAT {
            let (mut rate, mut ch, mut enc) = (0, 0, 0);
            h.getformat(Some(&mut rate), Some(&mut ch), Some(&mut enc));
            write!(out, " [fmt {rate} {ch} {enc} @{total}]").unwrap();
            newfmt += 1;
            if newfmt > 20 {
                break;
            }
            continue;
        }
        if ret != MPG123_OK || total >= limit {
            break;
        }
    }
    let err = h.errcode();
    let clip = h.clip();
    writeln!(out, " {total} {hash:016x} end={ret} err={err} clip={clip}").unwrap();
}

fn hash_str(out: &mut String, label: &str, s: Option<&Mpg123String>) {
    match s {
        None => write!(out, " {label}=-").unwrap(),
        Some(s) => write!(out, " {label}={}:{:016x}", s.fill(), fnv(FNV0, s.bytes())).unwrap(),
    }
}

fn print_texts(out: &mut String, label: &str, t: &[Mpg123Text]) {
    write!(out, " {label}[{}]", t.len()).unwrap();
    for t in t {
        write!(
            out,
            " {:02x}{:02x}{:02x}{:02x}/{:02x}{:02x}{:02x}",
            t.id[0], t.id[1], t.id[2], t.id[3], t.lang[0], t.lang[1], t.lang[2]
        )
        .unwrap();
        hash_str(out, "d", Some(&t.description));
        hash_str(out, "t", Some(&t.text));
    }
}

fn test_meta(out: &mut String, h: &mut Mpg123Handle<'_>) {
    let mc = h.meta_check();
    write!(out, "meta: {mc}").unwrap();
    {
        let (v1, v2) = h.id3();
        match v1 {
            Some(v1) => write!(out, " v1={:016x}", fnv(FNV0, v1)).unwrap(),
            None => write!(out, " v1=-").unwrap(),
        }
        if let Some(v2) = v2 {
            write!(out, " v2={}", v2.version).unwrap();
            hash_str(out, "title", v2.get(v2.title));
            hash_str(out, "artist", v2.get(v2.artist));
            hash_str(out, "album", v2.get(v2.album));
            hash_str(out, "year", v2.get(v2.year));
            hash_str(out, "genre", v2.get(v2.genre));
            hash_str(out, "comment", v2.get(v2.comment));
            print_texts(out, "text", &v2.text);
            print_texts(out, "comm", &v2.comment_list);
            print_texts(out, "extra", &v2.extra);
            write!(out, " pic[{}]", v2.picture.len()).unwrap();
            for p in &v2.picture {
                write!(out, " {}", p.type_ as i8).unwrap();
                hash_str(out, "m", Some(&p.mime_type));
                hash_str(out, "d", Some(&p.description));
                let data = p.data.as_deref().unwrap_or(&[]);
                write!(out, " {}:{:016x}", p.size, fnv(FNV0, &data[..p.size])).unwrap();
            }
        }
    }
    let (r1, r2) = h.id3_raw();
    let s1 = r1.map_or(0, |r| r.len());
    let (s2, h2) = r2.map_or((0, 0), |r| (r.len(), fnv(FNV0, r)));
    writeln!(out, " raw={s1},{s2}:{h2:016x}").unwrap();
}

fn test_file(out: &mut String, data: &Arc<Vec<u8>>, len: usize, full: bool) {
    let mut ret = 0;
    let h = open_mem(data, len, 0, 0, &mut ret);
    let (mut rate, mut ch, mut enc) = (0i64, 0i32, 0i32);
    let Some(mut h) = h else {
        writeln!(out, "open: {ret} -99 0 0 0").unwrap();
        return;
    };
    let fr = h.getformat(Some(&mut rate), Some(&mut ch), Some(&mut enc));
    write!(out, "open: {ret} {fr} {rate} {ch} {enc}").unwrap();
    if fr != MPG123_OK {
        write!(out, " err={}", h.errcode()).unwrap();
    }
    writeln!(out).unwrap();
    if fr == MPG123_OK {
        let mut mi = Mpg123FrameInfo::default();
        h.info(&mut mi);
        writeln!(
            out,
            "info: {} {} {} {} {} {} {} {} {} {} {}",
            mi.version,
            mi.layer,
            mi.rate,
            mi.mode,
            mi.mode_ext,
            mi.framesize,
            mi.flags,
            mi.emphasis,
            mi.bitrate,
            mi.abr_rate,
            mi.vbr
        )
        .unwrap();
        let (mut d, mut p, mut a, mut f, mut dd) = (0, 0, 0, 0, 0);
        h.getstate(MPG123_ENC_DELAY, Some(&mut d), None);
        h.getstate(MPG123_ENC_PADDING, Some(&mut p), None);
        h.getstate(MPG123_ACCURATE, Some(&mut a), None);
        h.getstate(MPG123_FRANKENSTEIN, Some(&mut f), None);
        h.getstate(MPG123_DEC_DELAY, Some(&mut dd), None);
        let len = h.length64();
        writeln!(out, "state: {d} {p} {a} {f} {dd} len={len}").unwrap();
    }
    let sr = h.scan();
    let len64 = h.length64();
    writeln!(out, "scan: {sr} len={len64} err={}", h.errcode()).unwrap();
    write!(out, "decode:").unwrap();
    decode_all(out, &mut h, usize::MAX);
    test_meta(out, &mut h);
    drop(h);

    if !full {
        return;
    }

    /* seeks on a fresh handle, as SDL_mixer's tracks seek (no scan first) */
    let mut h = open_mem(data, len, 0, 0, &mut ret).unwrap();
    for off in [0i64, 1, 576, 1152, 1500, 4000, 100000, 2000, 0, 7] {
        let r = h.seek64(off, SEEK_SET);
        write!(out, "seek {off}: {r} tell={}", h.tell64()).unwrap();
        decode_all(out, &mut h, 2048);
    }
    let r = h.seek64(100, SEEK_END);
    write!(out, "seek end-100: {r} tell={}", h.tell64()).unwrap();
    decode_all(out, &mut h, usize::MAX);
    drop(h);

    /* fuzzy seeks */
    let mut h = open_mem(data, len, 0, MPG123_FUZZY, &mut ret).unwrap();
    for off in [3000i64, 500, 9000] {
        let r = h.seek64(off, SEEK_SET);
        write!(out, "fuzzy {off}: {r} tell={}", h.tell64()).unwrap();
        decode_all(out, &mut h, 2048);
    }
    drop(h);

    /* every output encoding */
    for e in ENCS {
        let mut h = open_mem(data, len, e, 0, &mut ret).unwrap();
        let (mut rate, mut ch, mut enc) = (0i64, 0i32, 0i32);
        let fr = h.getformat(Some(&mut rate), Some(&mut ch), Some(&mut enc));
        write!(out, "enc {e}: {ret} {fr} {rate} {ch} {enc}").unwrap();
        decode_all(out, &mut h, usize::MAX);
    }

    /* mono and stereo forced, the gapless and Xing parsing off, raw tags and pictures */
    const FLAGS: [i64; 15] = [
        MPG123_FORCE_MONO,
        MPG123_MONO_LEFT,
        MPG123_MONO_RIGHT,
        MPG123_FORCE_STEREO,
        MPG123_FORCE_8BIT,
        MPG123_FORCE_FLOAT,
        MPG123_IGNORE_INFOFRAME,
        MPG123_IGNORE_STREAMLENGTH,
        MPG123_PICTURE | MPG123_STORE_RAW_ID3,
        MPG123_SKIP_ID3V2,
        MPG123_PLAIN_ID3TEXT,
        MPG123_NO_PEEK_END,
        MPG123_NO_READAHEAD,
        MPG123_NO_RESYNC,
        MPG123_NO_FRANKENSTEIN,
    ];
    for flags in FLAGS {
        let mut h = open_mem(data, len, 0, flags, &mut ret).unwrap();
        let (mut rate, mut ch, mut enc) = (0i64, 0i32, 0i32);
        let fr = h.getformat(Some(&mut rate), Some(&mut ch), Some(&mut enc));
        write!(out, "flags {flags:x}: {ret} {fr} {rate} {ch} {enc}").unwrap();
        write!(out, " len={}", h.length64()).unwrap();
        decode_all(out, &mut h, usize::MAX);
        if flags & (MPG123_PICTURE | MPG123_SKIP_ID3V2 | MPG123_PLAIN_ID3TEXT) != 0 {
            test_meta(out, &mut h);
        }
    }
    /* without gapless decoding */
    let mut h = open_mem(data, len, 0, 0, &mut ret).unwrap();
    h.param2(MPG123_REMOVE_FLAGS, MPG123_GAPLESS, 0.0);
    write!(out, "nogapless: len={}", h.length64()).unwrap();
    decode_all(out, &mut h, usize::MAX);
    drop(h);

    /* parameters SDL_mixer leaves alone */
    const PARAMS: [(i32, i64, f64); 9] = [
        (MPG123_RVA, MPG123_RVA_MIX as i64, 0.0),
        (MPG123_RVA, MPG123_RVA_ALBUM as i64, 0.0),
        (MPG123_DOWNSPEED, 1, 0.0),
        (MPG123_UPSPEED, 2, 0.0),
        (MPG123_OUTSCALE, 0, 0.3),
        (MPG123_RESYNC_LIMIT, 3, 0.0),
        (MPG123_INDEX_SIZE, -50, 0.0),
        (MPG123_PREFRAMES, 0, 0.0),
        (MPG123_PREFRAMES, 5, 0.0),
    ];
    for (key, val, fval) in PARAMS {
        write!(out, "param {key} {val} {fval}:").unwrap();
        let param = Some((key, val, fval));
        let mut h = open_mem_p(out, data, len, 0, 0, true, param, &mut ret).unwrap();
        write!(out, " {ret} len={}", h.length64()).unwrap();
        let r = h.seek64(1500, SEEK_SET);
        write!(out, " seek={r}").unwrap();
        decode_all(out, &mut h, usize::MAX);
    }

    /* no seek callback: a seek forward, one back, and ICY metadata every 1000 bytes */
    write!(out, "noseek:").unwrap();
    let mut h = open_mem_p(out, data, len, 0, 0, false, None, &mut ret).unwrap();
    write!(out, " {ret} len={}", h.length64()).unwrap();
    let r1 = h.seek64(3000, SEEK_SET);
    let r2 = h.seek64(100, SEEK_SET);
    write!(out, " seek={r1},{r2}").unwrap();
    decode_all(out, &mut h, usize::MAX);
    drop(h);
    write!(out, "icy:").unwrap();
    let param = Some((MPG123_ICY_INTERVAL, 1000, 0.0));
    let mut h = open_mem_p(out, data, len, 0, 0, false, param, &mut ret).unwrap();
    write!(out, " {ret}").unwrap();
    decode_all(out, &mut h, usize::MAX);
    let mc = h.meta_check();
    let icy = h
        .icy()
        .map(|d| &d[..d.iter().position(|&b| b == 0).unwrap_or(d.len())]);
    let (n, hash) = icy.map_or((0, 0), |d| (d.len(), fnv(FNV0, d)));
    writeln!(out, "icymeta: {mc} {n}:{hash:016x}").unwrap();
}

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0
    }
}

/// Truncated and corrupted copies, as the mixer tests make them.
fn damage(out: &mut String, data: &[u8]) {
    let len = data.len();
    let whole = Arc::new(data.to_vec());
    for cut in [0usize, 1, 4, 10, 20, 40, 100, 300, 1000, 2000] {
        if cut >= len {
            break;
        }
        write!(out, "trunc {cut}: ").unwrap();
        test_file(out, &whole, cut, false);
    }
    for cut in [len / 3, len / 2, len - 1] {
        write!(out, "trunc {cut}: ").unwrap();
        test_file(out, &whole, cut, false);
    }
    for v in 0..8usize {
        let mut bad = data.to_vec();
        let mut lcg = Lcg((v * 7919 + len) as u32);
        let flips = 1 + v * 2;
        for _ in 0..flips {
            let r = lcg.next();
            let at = if v < 3 {
                (r >> 8) as usize % len.min(64)
            } else {
                (r >> 8) as usize % len
            };
            bad[at] ^= (1 + (lcg.next() >> 24) % 255) as u8;
        }
        write!(out, "corrupt {v}: ").unwrap();
        let bad = Arc::new(bad);
        test_file(out, &bad, len, false);
    }
}

/// The whole output for one file.
fn test_one(out: &mut String, name: &str, data: &[u8]) {
    writeln!(out, "== {name}").unwrap();
    let whole = Arc::new(data.to_vec());
    test_file(out, &whole, data.len(), true);
    damage(out, data);
}

/// `(name, bytes)` of each of the files in testdata/.
macro_rules! own_files {
    [$($name:literal),* $(,)?] => {
        &[$(($name, include_bytes!(concat!("../testdata/", $name)) as &[u8])),*]
    };
}

/// This crate's test files, in name order, embedded: the tests of the
/// Emscripten build see only their own (in-memory) file system.
const OWN_FILES: &[(&str, &[u8])] = own_files![
    "mp1_dual.mp1",
    "mp1_free.mp1",
    "mp1_joint_crc.mp1",
    "mp1_lsf.mp1",
    "mp1_mono.mp1",
    "mp2_32k_low.mp2",
    "mp2_48k_low.mp2",
    "mp2_dual.mp2",
    "mp2_joint_crc.mp2",
    "mp2_lsf_stereo.mp2",
    "mp3_abr.mp3",
    "mp3_id3_mid.mp3",
    "mp3_id3v1_end.mp3",
    "mp3_id3v22_frames.mp3",
    "mp3_id3v23_unsync.mp3",
    "mp3_id3v24_footer.mp3",
    "mp3_lsf_intensity.mp3",
    "mp3_mpeg25_stereo.mp3",
    "mp3_replaygain.mp3",
    "mp3_riff.wav",
    "mp3_shine.mp3",
    "mp3_short_blocks.mp3",
    "mp3_stereo_info.mp3",
];

/// The package's directory: where Cargo says it is when it runs the tests
/// (on Android, the runner's copy of the workspace's test data on the
/// device: see tools/android_run.py), else where it was built.
fn manifest_dir() -> PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), PathBuf::from)
}

/// [`OWN_FILES`] has every file of testdata/.
#[test]
#[cfg_attr(target_os = "emscripten", ignore = "host file system")]
fn own_files_are_the_testdata_files() {
    let dir = manifest_dir().join("testdata");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n != "reference.txt")
        .collect();
    names.sort();
    let embedded: Vec<&str> = OWN_FILES.iter().map(|&(name, _)| name).collect();
    assert_eq!(names, embedded);
}

/// The file of a reference section: this crate's test data, or sdl3-mixer's
/// (which a packaged crate doesn't have, nor the Emscripten build's tests).
fn find(name: &str) -> Option<Vec<u8>> {
    if let Some(&(_, data)) = OWN_FILES.iter().find(|&&(n, _)| n == name) {
        return Some(data.to_vec());
    }
    let mixer = manifest_dir()
        .join("../sdl3-mixer/src/testdata/audio")
        .join(name);
    std::fs::read(mixer).ok()
}

#[test]
fn matches_upstream() {
    let reference = include_str!("../testdata/reference.txt");
    let mut sections: Vec<(String, String)> = Vec::new();
    for line in reference.lines() {
        if let Some(name) = line.strip_prefix("== ") {
            sections.push((name.to_string(), String::new()));
        }
        let section = &mut sections.last_mut().expect("a section").1;
        section.push_str(line);
        section.push('\n');
    }
    assert!(sections.len() > 20);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failures = std::sync::Mutex::new(Vec::new());
    let skipped = std::sync::atomic::AtomicUsize::new(0);
    let work = || loop {
        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let Some((name, expected)) = sections.get(i) else {
            break;
        };
        let Some(data) = find(name) else {
            skipped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            continue;
        };
        let mut out = String::new();
        test_one(&mut out, name, &data);
        if out != *expected {
            let (n, (want, got)) = expected
                .lines()
                .zip(out.lines())
                .enumerate()
                .find(|(_, (a, b))| a != b)
                .unwrap_or((0, ("(line count)", "(line count)")));
            failures.lock().unwrap().push(format!(
                "{name}, line {n}:\n  upstream: {want}\n  ours:     {got}"
            ));
        }
    };
    if cfg!(target_os = "emscripten") {
        // (no threads: one file after another)
        work();
    } else {
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(work);
            }
        });
    }
    let failures = failures.into_inner().unwrap();
    assert!(
        failures.is_empty(),
        "{} file(s) differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
    let skipped = skipped.into_inner();
    if skipped > 0 {
        eprintln!("{skipped} reference file(s) of sdl3-mixer not found, skipped");
    }
    // This crate's own files are always there.
    assert!(sections.len() - skipped >= 23);
}

/// `MPG_DIR=dir MPG_FILES="a.mp3 b.mp2" MPG_OUT=out.txt cargo test --
/// --ignored dump` writes the output for other files (to compare with the
/// C program's).
#[test]
#[ignore]
fn dump() {
    let dir = std::env::var("MPG_DIR").unwrap();
    let files = std::env::var("MPG_FILES").unwrap();
    let mut out = String::new();
    for name in files.split_whitespace() {
        let data = std::fs::read(format!("{dir}/{name}")).unwrap();
        test_one(&mut out, name, &data);
    }
    std::fs::write(std::env::var("MPG_OUT").unwrap(), out).unwrap();
}
