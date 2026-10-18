// Tests of sdl3-mixer against upstream SDL_mixer's C.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The files in `testdata/audio/` are made by
//! `tools/gen_sdl_mixer_testdata.py` (a synthetic signal, encoded by ffmpeg,
//! or by WavPack's encoder for the WavPack files, or put together by the
//! script), the tracker modules in
//! `testdata/modules/` by `tools/gen_xmp_testdata.py`.
//! `testdata/reference.txt` is the output
//! of a C program built from upstream SDL_mixer (with its dr_mp3, dr_flac
//! and stb_vorbis decoders, its Opus decoder over the libopus 1.4,
//! opusfile 0.12 and libogg 1.3.5 it vendors, built as its CMake builds
//! them but without SIMD, its XMP decoder over the libxmp 4.7.3 it
//! vendors, built as its CMake builds it, and its WAVPACK decoder over the
//! WavPack 5.9.0 it vendors, built as its CMake builds it but with the C
//! decorrelation passes rather than the assembly ones, and its MPG123
//! decoder over the libmpg123 1.33.7 it vendors, built as its CMake builds
//! it but with only its generic C decoder) and SDL3, which these tests
//! repeat and compare line by line (without the `mpg123` feature, the
//! sections that changes come from `testdata/reference_nompg123.txt`, the
//! output of the same program built without MPG123):
//!
//! - for every file, the decoder that opens it, its format, metadata and
//!   duration, and the audio decoded with an `MIX_AudioDecoder` in its own
//!   format and converted to two others; the same for truncated copies and
//!   for copies with corrupted bytes (which must decode, or fail, exactly as
//!   upstream does), and for Ogg files, copies with corrupted packets in
//!   intact pages (their checksums fixed), for WavPack files, copies with
//!   corrupted block contents (their block checksums fixed);
//! - the output of mixers (`MIX_Generate()`) playing each file, loaded on
//!   demand and predecoded, with loops and seeks (and, for the Opus and
//!   WavPack files, seeks all around them; for WavPack files with a
//!   correction file, all of that with it, given as a stream, as a path,
//!   found next to the file, truncated, corrupted or wrong; for the modules,
//!   starting at orders and seeks
//!   around them, and libxmp's own API, loading from memory and playing in
//!   the output formats, interpolations and player flags SDL_mixer doesn't
//!   use);
//! - mixer features (gains, fades, stereo and 3D positioning, frequency
//!   ratios, tags, groups and callbacks, channel maps, other output formats)
//!   on a few files, the sine wave and raw audio decoders;
//! - the same for a few of sdl3-mixer-timidity's MIDI files: with the
//!   `fluidsynth` feature, as the FLUIDSYNTH decoder plays them without a
//!   SoundFont (and, with the `timidity` feature, the RMID file); with the
//!   `timidity` feature, as the TIMIDITY decoder plays them, with its test
//!   patches (upstream's C was built with its FLUIDSYNTH decoder, over the
//!   FluidSynth it vendors, built as its CMake builds it but with the IIR
//!   filter without `-ffast-math` (see sdl3-mixer-fluidsynth), and its
//!   TIMIDITY decoder, and run with `TIMIDITY_CFG` naming the same
//!   configuration);
//! - with the `fluidsynth` feature, sdl3-mixer-fluidsynth's MIDI files
//!   played with its SoundFont, given as a path or a stream, with
//!   FluidSynth settings, broken or missing;
//! - with the `gme` feature, sdl3-mixer-gme's rips through the GME decoder
//!   (`testdata/reference_gme.txt`; upstream's C was built with its GME
//!   decoder over the libgme it vendors, built as its CMake builds it,
//!   with only the emulators sdl3-mixer-gme translates so far): decoded,
//!   truncated and corrupted, mixed, started at tracks (orders) and seeked.
//!
//! Hashes are FNV-1a over the bytes. Upstream's C was patched where
//! sdl3-mixer works around its crashes and memory errors (the
//! `FIXME (upstream)` notes), and built to use SDL's bundled math functions,
//! as sdl3's are, rather than the C library's (libopus's `exp()`, `log()`
//! and `cos()` too, and libxmp's math, with its `powf()` as `pow()` on
//! doubles), and libxmp's random seed (the time) fixed at 0, as it is in
//! the tests here.
//!
//! One thing can't be pinned: corrupt input can decode to NaNs, and when
//! both operands of an x86 float operation are NaNs, the result is the
//! first one, whose sign can differ as the C and Rust compilers order the
//! operands differently. The files here don't hit that.

use std::fmt::Write;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::Arc;

use sdl3::audio::{AudioFormat, AudioSpec};
use sdl3::io::IoStream;
use sdl3::properties::Properties;

use crate::internal::{
    PrecacheIo, PROP_DECODER_WAVPACK_WVC_IOSTREAM_POINTER, PROP_DECODER_WAVPACK_WVC_PATH_STRING,
};
use crate::*;

macro_rules! files {
    ($($name:literal,)*) => {
        &[$(($name, include_bytes!(concat!("testdata/audio/", $name)) as &[u8]),)*]
    };
}

static FILES: &[(&str, &[u8])] = files![
    "aifc_alaw.aiff",
    "aifc_f32_mono.aiff",
    "aifc_f64_mono.aiff",
    "aifc_ulaw.aiff",
    "aiff_s16_stereo.aiff",
    "aiff_s24_mono.aiff",
    "aiff_s32_stereo.aiff",
    "aiff_s8_mono.aiff",
    "au_headerless.au",
    "au_s16_mono.au",
    "au_s24_unsupported.au",
    "au_s8_stereo.au",
    "au_ulaw.au",
    "flac_6ch.flac",
    "flac_c_escape0.flac",
    "flac_c_s12_8ch.flac",
    "flac_c_s16_lpc.flac",
    "flac_c_s20_stereo.flac",
    "flac_c_s24_3ch.flac",
    "flac_c_s32_mono.flac",
    "flac_c_s8_mono.flac",
    "flac_fixed_mono.flac",
    "flac_ogg.oga",
    "flac_s16_stereo.flac",
    "flac_s24_mono.flac",
    "mp1_stereo.mp1",
    "mp2_mono_lsf.mp2",
    "mp2_stereo.mp2",
    "mp3_ape_id3v1.mp3",
    "mp3_cbr_mono.mp3",
    "mp3_frankenstein.mp3",
    "mp3_free_format.mp3",
    "mp3_id3v22.mp3",
    "mp3_id3v2_notitle.mp3",
    "mp3_intensity.mp3",
    "mp3_joint_stereo.mp3",
    "mp3_junk.mp3",
    "mp3_mpeg25.mp3",
    "mp3_mpeg25_mixed.mp3",
    "mp3_mpeg2_noxing.mp3",
    "mp3_vbr_stereo.mp3",
    "ogg_3ch.ogg",
    "ogg_6ch.ogg",
    "ogg_8ch.ogg",
    "ogg_8k.ogg",
    "ogg_abr_mono.ogg",
    "ogg_chained.ogg",
    "ogg_chained_junk.ogg",
    "ogg_floor0.ogg",
    "ogg_hole.ogg",
    "ogg_mono.ogg",
    "ogg_mux.ogg",
    "ogg_q10_48k.ogg",
    "ogg_short.ogg",
    "ogg_stereo.ogg",
    "opus_6ch.opus",
    "opus_celt_2_5ms.opus",
    "opus_celt_44k.opus",
    "opus_celt_nb_5ms.opus",
    "opus_chained.opus",
    "opus_family255.opus",
    "opus_hole.opus",
    "opus_hybrid_10ms.opus",
    "opus_hybrid_stereo.opus",
    "opus_quad.opus",
    "opus_silk_mb_stereo.opus",
    "opus_silk_nb_60ms.opus",
    "opus_silk_wb.opus",
    "opus_spliced_mono.opus",
    "opus_spliced_stereo.opus",
    "opus_switch_hybrid_celt.opus",
    "opus_switch_silk_celt.opus",
    "voc_blocks.voc",
    "voc_loop_infinite.voc",
    "voc_loop_nosize.voc",
    "voc_s16_stereo.voc",
    "voc_type9.voc",
    "voc_u8_mono.voc",
    "wav_alaw.wav",
    "wav_f32_mono.wav",
    "wav_f64_stereo.wav",
    "wav_ima_mono.wav",
    "wav_ima_stereo.wav",
    "wav_loop.wav",
    "wav_msadpcm_mono.wav",
    "wav_msadpcm_stereo.wav",
    "wav_quad.wav",
    "wav_s16_stereo.wav",
    "wav_s24_mono.wav",
    "wav_s32_stereo.wav",
    "wav_u8_mono.wav",
    "wav_ulaw.wav",
    "wv_10ch.wv",
    "wv_37800.wv",
    "wv_3ch.wv",
    "wv_6ch.wv",
    "wv_6ch_offset.wv",
    "wv_apetag.wv",
    "wv_correlated.wv",
    "wv_dsd.wv",
    "wv_dsd_3ch.wv",
    "wv_dsd_spliced.wv",
    "wv_f32_inf.wv",
    "wv_f32_mono.wv",
    "wv_f32_ones.wv",
    "wv_f32_same.wv",
    "wv_f32_stereo.wv",
    "wv_false_stereo.wv",
    "wv_gap.wv",
    "wv_hybrid.wv",
    "wv_hybrid.wvc",
    "wv_hybrid_cc_s24.wv",
    "wv_hybrid_cc_s24.wvc",
    "wv_hybrid_f32.wv",
    "wv_hybrid_f32.wvc",
    "wv_hybrid_kbps.wv",
    "wv_hybrid_lossy.wv",
    "wv_hybrid_s32.wv",
    "wv_hybrid_s32.wvc",
    "wv_hybrid_s32_ones.wv",
    "wv_hybrid_s8.wv",
    "wv_hybrid_s8.wvc",
    "wv_hybrid_silence.wv",
    "wv_hybrid_silence.wvc",
    "wv_hybrid_xdecorr.wv",
    "wv_hybrid_xdecorr.wvc",
    "wv_id3v1.wv",
    "wv_id3v2.wv",
    "wv_offset.wv",
    "wv_quad_hybrid.wv",
    "wv_quad_hybrid.wvc",
    "wv_reorder.wv",
    "wv_s12_stereo.wv",
    "wv_s16_long.wv",
    "wv_s16_raw.wv",
    "wv_s16_stereo.wv",
    "wv_s24_mono.wv",
    "wv_s32_dups.wv",
    "wv_s32_int32.wv",
    "wv_s32_ones.wv",
    "wv_s32_stereo.wv",
    "wv_s32_zeros.wv",
    "wv_silence.wv",
    "wv_tags.wv",
    "wv_u8_mono.wv",
];

macro_rules! modules {
    ($($name:literal,)*) => {
        &[$(($name, include_bytes!(concat!("testdata/modules/", $name)) as &[u8]),)*]
    };
}

/// The tracker modules (made by `tools/gen_xmp_testdata.py`), in the
/// reference's order.
static MODULES: &[(&str, &[u8])] = modules![
    "669_composer.669",
    "abk_amos.abk",
    "amf_dsmi.amf",
    "arch_musx.mod",
    "asylum_amf.amf",
    "chip_kris.mod",
    "coco_coconizer.coco",
    "dbm_digibooster.dbm",
    "digi_booster.digi",
    "dt_dtm19.dtm",
    "emod_quadra.emod",
    "far_farandole.far",
    "flt_flt8.mod",
    "fnk_funktracker.fnk",
    "gal4_j2b.j2b",
    "gal5_j2b.j2b",
    "gdm_2gdm.gdm",
    "hmn_fest.mod",
    "ice_ice.mod",
    "imf_orpheus.imf",
    "ims_images.ims",
    "it_new.it",
    "it_old.it",
    "it_smp.it",
    "liq_liquid.liq",
    "masi16_psm16.psm",
    "masi_psm.psm",
    "mdl_v11.mdl",
    "med2_med112.med",
    "med3_med200.med",
    "med4_med210.med",
    "mgt_megatracker.mgt",
    "mmd0_med.med",
    "mmd1_octamed.med",
    "mmd2_octamed.med",
    "mmd3_octamed.med",
    "mod_6chn.mod",
    "mod_cd81.mod",
    "mod_clone.mod",
    "mod_fa04.mod",
    "mod_nt.mod",
    "mod_pt.mod",
    "mod_vblank.mod",
    "mtm_mtm.mtm",
    "muse_gal4.j2b",
    "no_liquid.no",
    "okt_oktalyzer.okt",
    "pt3_iffmodl.pt36",
    "ptm_ptm.ptm",
    "rtm_rtm110.rtm",
    "s3m_orpheus.s3m",
    "s3m_schism.s3m",
    "s3m_st3.s3m",
    "s3m_st301.s3m",
    "sfx_soundfx13.sfx",
    "st_soundtracker.mod",
    "stim_slamtilt.stim",
    "stm_st221.stm",
    "stx_stm2stx.stx",
    "sym_symphony.dsym",
    "ult_v004.ult",
    "umx_unreal.umx",
    "xm_amiga.xm",
    "xm_ft2.xm",
    "xm_old.xm",
    "xm_oxm.xm",
    "xmf_astroidea.xmf",
];

/// Files tested again with another decoder forced (the reference's
/// `name DECODER` sections): the Vorbis files with stb_vorbis, the FLAC
/// ones with dr_flac and the MPEG audio ones with dr_mp3, which upstream's
/// default build tries after libvorbisfile, libFLAC and libmpg123.
static FORCED: &[(&str, &str)] = &[
    ("ogg_6ch.ogg", "STBVORBIS"),
    ("ogg_8k.ogg", "STBVORBIS"),
    ("ogg_mono.ogg", "STBVORBIS"),
    ("ogg_stereo.ogg", "STBVORBIS"),
    ("flac_6ch.flac", "DRFLAC"),
    ("flac_fixed_mono.flac", "DRFLAC"),
    ("flac_ogg.oga", "DRFLAC"),
    ("flac_s16_stereo.flac", "DRFLAC"),
    ("flac_s24_mono.flac", "DRFLAC"),
    ("mp1_stereo.mp1", "DRMP3"),
    ("mp2_mono_lsf.mp2", "DRMP3"),
    ("mp2_stereo.mp2", "DRMP3"),
    ("mp3_ape_id3v1.mp3", "DRMP3"),
    ("mp3_cbr_mono.mp3", "DRMP3"),
    ("mp3_frankenstein.mp3", "DRMP3"),
    ("mp3_free_format.mp3", "DRMP3"),
    ("mp3_id3v22.mp3", "DRMP3"),
    ("mp3_id3v2_notitle.mp3", "DRMP3"),
    ("mp3_intensity.mp3", "DRMP3"),
    ("mp3_joint_stereo.mp3", "DRMP3"),
    ("mp3_junk.mp3", "DRMP3"),
    ("mp3_mpeg25.mp3", "DRMP3"),
    ("mp3_mpeg25_mixed.mp3", "DRMP3"),
    ("mp3_mpeg2_noxing.mp3", "DRMP3"),
    ("mp3_vbr_stereo.mp3", "DRMP3"),
];

thread_local! {
    /// The decoder every load of the file being tested asks for, if any
    /// (the C program's `g_force`).
    static FORCE: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

/// The files the mixer feature scenarios play.
static SCENARIO_FILES: &[&str] = &[
    "wav_s16_stereo.wav",
    "ogg_mono.ogg",
    "mp3_vbr_stereo.mp3",
    "flac_s16_stereo.flac",
    "wv_s16_long.wv",
];

static REFERENCE: &[u8] = include_bytes!("testdata/reference.txt");

/// The sections that differ without the `mpg123` feature (the MPEG audio
/// files' own sections, the MP3 scenarios and the decoder list), from the
/// same C program built without the MPG123 decoder.
#[cfg(not(feature = "mpg123"))]
static REFERENCE_NO_MPG123: &[u8] = include_bytes!("testdata/reference_nompg123.txt");

/// The reference's sections (`== name` and its lines), by name.
fn reference_sections(reference: &[u8]) -> std::collections::HashMap<String, String> {
    let mut sections = std::collections::HashMap::new();
    let mut name = None;
    for line in String::from_utf8_lossy(reference).lines() {
        if let Some(n) = line.strip_prefix("== ") {
            name = Some(n.to_string());
        }
        if let Some(name) = &name {
            let section: &mut String = sections.entry(name.clone()).or_default();
            section.push_str(line);
            section.push('\n');
        }
    }
    sections
}

const FNV0: u64 = 14695981039346656037;

fn fnv(mut h: u64, data: &[u8]) -> u64 {
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

fn fnv_f32(mut h: u64, data: &[f32]) -> u64 {
    for v in data {
        h = fnv(h, &v.to_ne_bytes());
    }
    h
}

fn file(name: &str) -> &'static [u8] {
    FILES.iter().find(|(n, _)| *n == name).unwrap().1
}

/// A stream over a copy of `data`.
fn mem_io(data: &[u8]) -> IoStream<'static> {
    PrecacheIo::open(Arc::new(data.to_vec()))
}

fn print_props(out: &mut String, p: &Properties) {
    if let Some(s) = p.get_string(PROP_METADATA_TITLE_STRING) {
        write!(out, " title=\"{s}\"").unwrap();
    }
    if let Some(s) = p.get_string(PROP_METADATA_ARTIST_STRING) {
        write!(out, " artist=\"{s}\"").unwrap();
    }
    if let Some(s) = p.get_string(PROP_METADATA_ALBUM_STRING) {
        write!(out, " album=\"{s}\"").unwrap();
    }
    if let Some(s) = p.get_string(PROP_METADATA_COPYRIGHT_STRING) {
        write!(out, " copyright=\"{s}\"").unwrap();
    }
    if p.contains(PROP_METADATA_TRACK_NUMBER) {
        write!(
            out,
            " track={}",
            p.get_number(PROP_METADATA_TRACK_NUMBER).unwrap_or(0)
        )
        .unwrap();
    }
    if p.contains(PROP_METADATA_TOTAL_TRACKS_NUMBER) {
        write!(
            out,
            " total={}",
            p.get_number(PROP_METADATA_TOTAL_TRACKS_NUMBER).unwrap_or(0)
        )
        .unwrap();
    }
    if p.contains(PROP_METADATA_YEAR_NUMBER) {
        write!(
            out,
            " year={}",
            p.get_number(PROP_METADATA_YEAR_NUMBER).unwrap_or(0)
        )
        .unwrap();
    }
    if p.contains(PROP_METADATA_DURATION_FRAMES_NUMBER) {
        write!(
            out,
            " frames={}",
            p.get_number(PROP_METADATA_DURATION_FRAMES_NUMBER)
                .unwrap_or(0)
        )
        .unwrap();
    }
    if p.contains(PROP_METADATA_DURATION_INFINITE_BOOLEAN) {
        write!(
            out,
            " infinite={}",
            p.get_bool(PROP_METADATA_DURATION_INFINITE_BOOLEAN)
                .unwrap_or(false) as i32
        )
        .unwrap();
    }
    for key in ["name", "type", "comment"] {
        if let Some(s) = p.get_string(&format!("SDL_mixer.metadata.xmp.{key}")) {
            write!(out, " xmp.{key}=\"{s}\"").unwrap();
        }
    }
}

const DECODE_CAP: usize = 8 * 1024 * 1024;

/// Open a decoder over the data and decode it all into `want` (or the
/// native format).
fn decode(
    out: &mut String,
    data: &[u8],
    want: Option<&AudioSpec>,
    describe: bool,
    force: Option<&str>,
) {
    decode_props(out, data, want, describe, force, None);
}

/// [`decode()`], with the caller's properties.
fn decode_props(
    out: &mut String,
    data: &[u8],
    want: Option<&AudioSpec>,
    describe: bool,
    force: Option<&str>,
    extra: Option<&Properties>,
) {
    let force = force.or(FORCE.get());
    let props = (force.is_some() || extra.is_some()).then(|| {
        let p = Properties::new();
        if let Some(extra) = extra {
            p.copy_from(extra).unwrap();
        }
        if let Some(name) = force {
            p.set(PROP_AUDIO_DECODER_STRING, name).unwrap();
        }
        p
    });
    let d = match AudioDecoder::new_io(mem_io(data), props.as_ref()) {
        Ok(d) => d,
        Err(e) => {
            writeln!(out, "err: {e}").unwrap();
            return;
        }
    };
    let mut spec = d.format().unwrap();
    if describe {
        let p = d.properties().unwrap();
        write!(
            out,
            "{} 0x{:04x} {} {}",
            p.get_string(PROP_AUDIO_DECODER_STRING)
                .unwrap_or_else(|| "?".into()),
            spec.format.0,
            spec.channels,
            spec.freq
        )
        .unwrap();
        print_props(out, &p);
        write!(out, " | ").unwrap();
    }
    if let Some(want) = want {
        spec = *want;
    }
    let mut buf = [0u8; 4096];
    let mut h = FNV0;
    let mut total = 0usize;
    let mut error = None;
    let mut rc: i32;
    loop {
        rc = match d.decode(&mut buf, &spec) {
            Ok(n) => n as i32,
            Err(e) => {
                error = Some(e);
                -1
            }
        };
        if rc <= 0 {
            break;
        }
        h = fnv(h, &buf[..rc as usize]);
        total += rc as usize;
        if total >= DECODE_CAP {
            rc = 1;
            break;
        }
    }
    write!(out, "{total} {h:016x} end={rc}").unwrap();
    if let Some(e) = error {
        write!(out, " err: {e}").unwrap();
    }
    writeln!(out).unwrap();
}

const CHUNK_FRAMES: usize = 2048;

/// Generate `chunks` chunks, hashing them and the return values.
fn generate(m: &Mixer, chunks: usize, h: &mut u64) {
    let spec = m.format().unwrap();
    let framesize = spec.frame_size();
    let mut buf = vec![0u8; CHUNK_FRAMES * 8 * 4];
    for _ in 0..chunks {
        buf.fill(0xAA);
        let rc = match m.generate(&mut buf[..CHUNK_FRAMES * framesize]) {
            Ok(n) => n as i32,
            Err(_) => -1,
        };
        *h = fnv(*h, &rc.to_ne_bytes());
        if rc > 0 {
            // F32 output can carry NaNs (raw audio of random bytes taken as
            // floats, in misc()): which NaN comes out of an operation on two
            // depends on the operand order, which the optimizer and the CPU
            // decide (a release build on an AMD CPU differs from a debug
            // one), so NaNs are hashed as one canonical NaN. The reference's
            // "raw 2" line was made the same way from upstream's bytes.
            if spec.format == AudioFormat::F32 {
                for sample in buf[..rc as usize].chunks_exact_mut(4) {
                    if f32::from_ne_bytes([sample[0], sample[1], sample[2], sample[3]]).is_nan() {
                        sample.copy_from_slice(&f32::NAN.to_ne_bytes());
                    }
                }
            }
            *h = fnv(*h, &buf[..rc as usize]);
        }
    }
}

/// Load audio from memory, with the caller's properties.
fn load_audio(
    m: &Mixer,
    data: &[u8],
    predecode: bool,
    extra: Option<&Properties>,
) -> sdl3::Result<Audio> {
    let force = FORCE.get();
    if extra.is_none() && force.is_none() {
        return Audio::load_io(Some(m), &mut mem_io(data), predecode);
    }
    let p = Properties::new();
    if let Some(extra) = extra {
        p.copy_from(extra).unwrap();
    }
    if let Some(name) = force {
        p.set(PROP_AUDIO_DECODER_STRING, name).unwrap();
    }
    p.set(PROP_AUDIO_LOAD_PREDECODE_BOOLEAN, predecode).unwrap();
    Audio::load_with_properties(Some(&mut mem_io(data)), Some(m), &p)
}

fn mixtest(out: &mut String, label: &str, data: &[u8], predecode: bool) {
    mixtest_props(out, label, data, predecode, None);
}

fn mixtest_props(
    out: &mut String,
    label: &str,
    data: &[u8],
    predecode: bool,
    extra: Option<&Properties>,
) {
    let spec = AudioSpec {
        format: AudioFormat::F32,
        channels: 2,
        freq: 48000,
    };
    let m = Mixer::new(&spec).unwrap();
    let a = match load_audio(&m, data, predecode, extra) {
        Ok(a) => a,
        Err(e) => {
            writeln!(out, "{label}: err: {e}").unwrap();
            return;
        }
    };
    let dur = a.duration().unwrap_or(-1);
    let t = Track::new(&m).unwrap();
    let _ = t.set_audio(Some(&a));
    let p = Properties::new();
    p.set(PROP_PLAY_LOOPS_NUMBER, 1i64).unwrap();
    let ok = t.play(Some(&p)).is_ok();
    let mut h = FNV0;
    generate(&m, 6, &mut h);
    let pos1 = t.playback_position().unwrap_or(-1);
    let ok2 = t.play(None).is_ok();
    let seeked = t
        .set_playback_position(if dur > 0 { dur / 3 } else { 100 })
        .is_ok();
    let pos2 = t.playback_position().unwrap_or(-1);
    generate(&m, 2, &mut h);
    writeln!(
        out,
        "{label}: dur={dur} play={} pos={pos1} replay={} seek={} pos={pos2} playing={} {h:016x}",
        ok as i32,
        ok2 as i32,
        seeked as i32,
        t.playing() as i32
    )
    .unwrap();
}

/// Seek a track around an Opus (or WavPack) file.
fn seektest(out: &mut String, data: &[u8]) {
    seektest_props(out, "seek", data, None);
}

fn seektest_props(out: &mut String, label: &str, data: &[u8], extra: Option<&Properties>) {
    let spec = AudioSpec {
        format: AudioFormat::F32,
        channels: 2,
        freq: 48000,
    };
    let m = Mixer::new(&spec).unwrap();
    let a = match load_audio(&m, data, false, extra) {
        Ok(a) => a,
        Err(e) => {
            writeln!(out, "{label}: err: {e}").unwrap();
            return;
        }
    };
    let dur = a.duration().unwrap_or(-1);
    let t = Track::new(&m).unwrap();
    let _ = t.set_audio(Some(&a));
    let ok = t.play(None).is_ok();
    let targets = [
        0,
        1,
        311,
        960,
        3840,
        4801,
        12345,
        dur / 2,
        dur - 500,
        dur - 1,
        0,
        dur + 1000,
        100,
    ];
    for target in targets {
        let seeked = t.set_playback_position(target).is_ok();
        let pos = t.playback_position().unwrap_or(-1);
        let mut h = FNV0;
        generate(&m, 1, &mut h);
        writeln!(
            out,
            "{label} {target}: play={} seek={} pos={pos} playing={} {h:016x}",
            ok as i32,
            seeked as i32,
            t.playing() as i32
        )
        .unwrap();
    }
}

/// Play a file loaded from its path (where a .wvc file next to a .wv one
/// is found).
fn pathtest(out: &mut String, path: &std::path::Path) {
    let spec = AudioSpec {
        format: AudioFormat::F32,
        channels: 2,
        freq: 48000,
    };
    let m = Mixer::new(&spec).unwrap();
    for predecode in [false, true] {
        let a = match Audio::load(Some(&m), path, predecode) {
            Ok(a) => a,
            Err(e) => {
                writeln!(out, "path {}: err: {e}", predecode as i32).unwrap();
                continue;
            }
        };
        let t = Track::new(&m).unwrap();
        let _ = t.set_audio(Some(&a));
        let ok = t.play(None).is_ok();
        let mut h = FNV0;
        generate(&m, 3, &mut h);
        writeln!(
            out,
            "path {}: dur={} play={} playing={} {h:016x}",
            predecode as i32,
            a.duration().unwrap_or(-1),
            ok as i32,
            t.playing() as i32
        )
        .unwrap();
    }
}

/// Play a module from orders and around it.
fn modtest(out: &mut String, data: &[u8]) {
    let spec = AudioSpec {
        format: AudioFormat::F32,
        channels: 2,
        freq: 48000,
    };
    let m = Mixer::new(&spec).unwrap();
    let a = match Audio::load_io(Some(&m), &mut mem_io(data), false) {
        Ok(a) => a,
        Err(e) => {
            writeln!(out, "modtest: err: {e}").unwrap();
            return;
        }
    };
    let dur = a.duration().unwrap_or(-1);
    let t = Track::new(&m).unwrap();
    let _ = t.set_audio(Some(&a));
    for order in [1i64, 2, 0, 200, -5, 3] {
        let p = Properties::new();
        p.set(PROP_PLAY_START_ORDER_NUMBER, order).unwrap();
        let played = t.play(Some(&p));
        let mut h = FNV0;
        generate(&m, 2, &mut h);
        write!(out, "order {order}: play={}", played.is_ok() as i32).unwrap();
        if let Err(e) = &played {
            write!(out, " err: {e}").unwrap();
        }
        writeln!(
            out,
            " pos={} playing={} {h:016x}",
            t.playback_position().unwrap_or(-1),
            t.playing() as i32
        )
        .unwrap();
    }
    let _ = t.play(None);
    for target in [0, 2205, dur / 2, dur - 1000, dur + 5000, 100] {
        let seeked = t.set_playback_position(target).is_ok();
        let pos = t.playback_position().unwrap_or(-1);
        let mut h = FNV0;
        generate(&m, 1, &mut h);
        writeln!(
            out,
            "modseek {target}: seek={} pos={pos} playing={} {h:016x}",
            seeked as i32,
            t.playing() as i32
        )
        .unwrap();
    }
}

fn fnv_int(h: u64, v: i32) -> u64 {
    fnv(h, &v.to_ne_bytes())
}

/// libxmp itself, in the modes SDL_mixer doesn't use.
fn xmptest(out: &mut String, data: &[u8]) {
    use crate::xmp::*;

    let mut c = xmp_create_context();
    let r = load::xmp_load_module_from_memory(&mut c, data);
    if r < 0 {
        writeln!(out, "xmp: load {r}").unwrap();
        xmp_free_context(c);
        return;
    }
    let chn;
    {
        let mi = xmp_get_module_info(&c).unwrap();
        let m = mi.mod_;
        chn = m.chn;
        write!(
            out,
            "xmp: pat={} trk={} chn={} ins={} smp={} spd={} bpm={} len={} rst={} gvl={} seqs={} vol={} md5=",
            m.pat,
            m.trk,
            m.chn,
            m.ins,
            m.smp,
            m.spd,
            m.bpm,
            m.len,
            m.rst,
            m.gvl,
            mi.num_sequences,
            mi.vol_base
        )
        .unwrap();
        for b in mi.md5 {
            write!(out, "{b:02x}").unwrap();
        }
        let mut hm = FNV0;
        for xxi in &m.xxi[..m.ins as usize] {
            hm = fnv(hm, cstr(&xxi.name));
            hm = fnv_int(fnv_int(fnv_int(hm, xxi.vol), xxi.nsm), xxi.rls);
            for sub in &xxi.sub[..xxi.nsm as usize] {
                for v in [
                    sub.vol, sub.gvl, sub.pan, sub.xpo, sub.fin, sub.vwf, sub.vde, sub.vra,
                    sub.vsw, sub.rvv, sub.sid, sub.nna, sub.dct, sub.dca, sub.ifc, sub.ifr,
                ] {
                    hm = fnv_int(hm, v);
                }
            }
        }
        for xxs in &m.xxs[..m.smp as usize] {
            hm = fnv(hm, cstr(&xxs.name));
            hm = fnv_int(
                fnv_int(fnv_int(fnv_int(hm, xxs.len), xxs.lps), xxs.lpe),
                xxs.flg,
            );
            if let Some(d) = &xxs.data {
                let bytes = xxs.len as usize
                    * if xxs.flg & XMP_SAMPLE_16BIT != 0 {
                        2
                    } else {
                        1
                    }
                    * if xxs.flg & XMP_SAMPLE_STEREO != 0 {
                        2
                    } else {
                        1
                    };
                hm = fnv(hm, &d[SAMPLE_GUARD..SAMPLE_GUARD + bytes]);
            }
        }
        writeln!(out, " {hm:016x}").unwrap();
    }
    let modes = [
        (
            22050,
            XMP_FORMAT_8BIT | XMP_FORMAT_UNSIGNED | XMP_FORMAT_MONO,
            XMP_INTERP_NEAREST,
            70,
            1,
            XMP_DSP_LOWPASS,
            0,
        ),
        (32000, 0, XMP_INTERP_SPLINE, 30, 2, 0, 0),
        (
            44100,
            XMP_FORMAT_MONO,
            XMP_INTERP_LINEAR,
            100,
            0,
            XMP_DSP_LOWPASS,
            XMP_FLAGS_A500,
        ),
        (
            11025,
            0,
            XMP_INTERP_SPLINE,
            -100,
            3,
            0,
            XMP_FLAGS_VBLANK | XMP_FLAGS_FX9BUG | XMP_FLAGS_FIXLOOP | XMP_FLAGS_A500,
        ),
    ];
    for (i, (rate, fmt, interp, mix, amp, dsp, flags)) in modes.into_iter().enumerate() {
        let r = xmp_start_player(&mut c, rate, fmt);
        if r < 0 {
            writeln!(out, "xmp {i}: start {r}").unwrap();
            continue;
        }
        let mut sr = control::xmp_set_player(&mut c, XMP_PLAYER_INTERP, interp);
        sr += control::xmp_set_player(&mut c, XMP_PLAYER_MIX, mix);
        sr += control::xmp_set_player(&mut c, XMP_PLAYER_AMP, amp);
        sr += control::xmp_set_player(&mut c, XMP_PLAYER_DSP, dsp);
        sr += control::xmp_set_player(&mut c, XMP_PLAYER_CFLAGS, flags);
        let mut h = FNV0;
        let mut hf = FNV0;
        let mut frames = 0;
        let mut fi = XmpFrameInfo::default();
        while frames < 300 {
            if xmp_play_frame(&mut c) < 0 {
                break;
            }
            xmp_get_frame_info(&c, &mut fi);
            if fi.loop_count > 0 {
                break;
            }
            h = fnv(h, control::frame_buffer(&c, &fi));
            for v in [
                fi.pos,
                fi.pattern,
                fi.row,
                fi.num_rows,
                fi.frame,
                fi.speed,
                fi.bpm,
                fi.time,
                fi.total_time,
                fi.frame_time,
                fi.buffer_size,
                fi.total_size,
                fi.volume,
                fi.virt_channels,
                fi.virt_used,
                fi.sequence,
            ] {
                hf = fnv_int(hf, v);
            }
            for ci in &fi.channel_info[..chn as usize] {
                for v in [
                    ci.period as i32,
                    ci.position as i32,
                    ci.pitchbend as i32,
                    ci.note as i32,
                    ci.instrument as i32,
                    ci.sample as i32,
                    ci.volume as i32,
                    ci.pan as i32,
                ] {
                    hf = fnv_int(hf, v);
                }
            }
            frames += 1;
        }
        let s1 = xmp_seek_time(&mut c, 700);
        let s2 = xmp_set_position(&mut c, 1);
        xmp_play_buffer(&mut c, None, 0, 0);
        let mut buf = [0u8; 2000];
        let mut h2 = FNV0;
        for _ in 0..5 {
            let pr = xmp_play_buffer(&mut c, Some(&mut buf), 2000, 1);
            h2 = fnv_int(h2, pr);
            h2 = fnv(h2, &buf);
        }
        writeln!(
            out,
            "xmp {i}: set={sr} mixer={} frames={frames} {h:016x} {hf:016x} seek={s1} pos={s2} {h2:016x}",
            control::xmp_get_player(&c, XMP_PLAYER_MIXER_TYPE)
        )
        .unwrap();
        xmp_end_player(&mut c);
    }
    xmp_release_module(&mut c);
    xmp_free_context(c);
}

/// Whether a test file is a tracker module (anything but the audio files'
/// formats).
fn is_module(name: &str) -> bool {
    let exts = [
        ".wav", ".aiff", ".au", ".flac", ".oga", ".mp2", ".mp3", ".ogg", ".opus", ".voc", ".mid",
        ".rmi", ".wv", ".wvc",
    ];
    match name.rfind('.') {
        None => true,
        Some(dot) => !exts.contains(&&name[dot..]),
    }
}

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0
    }
}

/// The WavPack blocks of the data: their offsets and lengths.
fn wv_blocks(d: &[u8]) -> Vec<(usize, usize)> {
    let mut blocks = Vec::new();
    let mut p = 0;
    while p + 32 <= d.len() && blocks.len() < 512 && &d[p..p + 4] == b"wvpk" {
        let size = u32::from_le_bytes(d[p + 4..p + 8].try_into().unwrap()) as usize + 8;
        if size < 32 || p + size > d.len() {
            break;
        }
        blocks.push((p, size));
        p += size;
    }
    blocks
}

/// Redo the block checksums (`ID_BLOCK_CHECKSUM`), as
/// `WavpackVerifySingleBlock()` checks them.
fn wv_fix_checksums(d: &mut [u8], blocks: &[(usize, usize)]) {
    for &(at, size) in blocks {
        let b = &mut d[at..at + size];
        let mut p = 32;
        while p + 2 <= size {
            let id = b[p];
            let mut bc = (b[p + 1] as usize) << 1;
            let mut h = 2;
            if id & 0x80 != 0 {
                if p + 4 > size {
                    break;
                }
                bc += ((b[p + 2] as usize) << 9) + ((b[p + 3] as usize) << 17);
                h = 4;
            }
            if (id & 0x3f) == 0x2f && p + h + bc <= size && (bc == 2 || bc == 4) {
                let mut csum = 0xFFFF_FFFFu32;
                let mut k = 0;
                while k + 1 < p {
                    csum = csum
                        .wrapping_mul(3)
                        .wrapping_add(b[k] as u32 + ((b[k + 1] as u32) << 8));
                    k += 2;
                }
                if bc == 4 {
                    b[p + h..p + h + 4].copy_from_slice(&csum.to_le_bytes());
                } else {
                    csum ^= csum >> 16;
                    b[p + h] = csum as u8;
                    b[p + h + 1] = (csum >> 8) as u8;
                }
            }
            p += h + bc;
        }
    }
}

/// WavPack: corrupt the blocks' contents (and, for the later variants,
/// their headers, but not their sizes), with the block checksums fixed
/// after.
fn wv_corrupt(data: &[u8], v: usize) -> Vec<u8> {
    let blocks = wv_blocks(data);
    let mut bad = data.to_vec();
    if blocks.is_empty() {
        return bad;
    }
    let mut lcg = Lcg((v * 104729 + data.len()) as u32);
    let flips = 1 + v % 4;
    for _ in 0..flips {
        let r = lcg.next();
        let (at, size) = blocks[(r >> 8) as usize % blocks.len()];
        let from = if v >= 12 { 8 } else { 32 }; // the later variants hit the block headers too
        let body = size - from;
        let r2 = lcg.next();
        let r3 = lcg.next();
        if body == 0 {
            continue;
        }
        bad[at + from + (r2 >> 8) as usize % body] ^= (1 + (r3 >> 24) % 255) as u8;
    }
    wv_fix_checksums(&mut bad, &blocks);
    bad
}

/// A WavPack file with its correction file, given as a stream property, as
/// a path property, and found next to it.
fn wvtest(out: &mut String, name: &str, data: &[u8]) {
    let wvcname = format!("{name}c");
    let Some(&(_, wvc)) = FILES.iter().find(|(n, _)| *n == wvcname) else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("sdl3-mixer-wv-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let wvcpath = dir.join(&wvcname);
    std::fs::write(&path, data).unwrap();
    std::fs::write(&wvcpath, wvc).unwrap();

    let variants: [(&str, &[u8]); 5] = [
        ("wvc", wvc),
        ("wvc half", &wvc[..wvc.len() / 2]),
        ("wvc short", &wvc[..wvc.len().min(40)]),
        ("wvc empty", &wvc[..0]),
        ("wvc self", data),
    ];
    let f32s48 = AudioSpec {
        format: AudioFormat::F32,
        channels: 2,
        freq: 48000,
    };
    for i in 0..variants.len() + 6 {
        let (label, wd) = if i < variants.len() {
            (variants[i].0.to_string(), variants[i].1.to_vec())
        } else {
            let v = i - variants.len();
            (format!("wvc corrupt {v}"), wv_corrupt(wvc, v))
        };
        let p = Properties::new();
        p.set_any(
            PROP_DECODER_WAVPACK_WVC_IOSTREAM_POINTER,
            std::sync::Mutex::new(mem_io(&wd)),
        )
        .unwrap();
        let rewind = || {
            let wio = p
                .get_any::<std::sync::Mutex<IoStream<'static>>>(
                    PROP_DECODER_WAVPACK_WVC_IOSTREAM_POINTER,
                )
                .unwrap();
            wio.lock()
                .unwrap()
                .seek(0, sdl3::io::IoWhence::Set)
                .unwrap();
        };
        write!(out, "{label}: ").unwrap();
        rewind();
        decode_props(out, data, None, true, None, Some(&p));
        if i == 0 {
            write!(out, "{label} f32s48: ").unwrap();
            rewind();
            decode_props(out, data, Some(&f32s48), false, None, Some(&p));
        }
        rewind();
        mixtest_props(out, &format!("{label} mix"), data, false, Some(&p));
        if i == 0 || i == 1 || i == variants.len() {
            rewind();
            seektest_props(out, &format!("{label} seek"), data, Some(&p));
        }
    }
    // the correction file by path, and next to the file
    let p = Properties::new();
    p.set(
        PROP_DECODER_WAVPACK_WVC_PATH_STRING,
        wvcpath.to_string_lossy().into_owned(),
    )
    .unwrap();
    write!(out, "wvc path: ").unwrap();
    decode_props(out, data, None, true, None, Some(&p));
    p.set(PROP_DECODER_WAVPACK_WVC_PATH_STRING, "/nonexistent/x.wvc")
        .unwrap();
    write!(out, "wvc no path: ").unwrap();
    decode_props(out, data, None, true, None, Some(&p));
    pathtest(out, &path);
    let _ = std::fs::remove_dir_all(&dir);
}

fn test_file(out: &mut String, name: &str, data: &[u8]) {
    let len = data.len();
    match FORCE.get() {
        Some(force) => writeln!(out, "== {name} {force}").unwrap(),
        None => writeln!(out, "== {name}").unwrap(),
    }

    write!(out, "open: ").unwrap();
    decode(out, data, None, true, None);
    let f32s48 = AudioSpec {
        format: AudioFormat::F32,
        channels: 2,
        freq: 48000,
    };
    write!(out, "f32s48: ").unwrap();
    decode(out, data, Some(&f32s48), false, None);
    let s16m22 = AudioSpec {
        format: AudioFormat::S16,
        channels: 1,
        freq: 22050,
    };
    write!(out, "s16m22: ").unwrap();
    decode(out, data, Some(&s16m22), false, None);
    if name.contains("headerless") {
        write!(out, "forced AU: ").unwrap();
        decode(out, data, None, true, Some("AU"));
        write!(out, "forced RAW: ").unwrap();
        decode(out, data, None, true, Some("RAW"));
    }

    const CUTS: &[usize] = &[
        0, 1, 2, 4, 8, 11, 12, 16, 20, 24, 32, 36, 40, 44, 48, 64, 100, 128, 200, 256, 300, 512,
        1000, 2000, 4096,
    ];
    for &cut in CUTS {
        if cut >= len {
            break;
        }
        write!(out, "trunc {cut}: ").unwrap();
        decode(out, &data[..cut], None, true, None);
    }
    for cut in [len / 4, len / 2, len * 3 / 4, len - 1] {
        write!(out, "trunc {cut}: ").unwrap();
        decode(out, &data[..cut], None, true, None);
    }

    for v in 0..6usize {
        let mut bad = data.to_vec();
        let mut lcg = Lcg((v * 7919 + len) as u32);
        let flips = 1 + v * 3;
        for _ in 0..flips {
            let r = lcg.next();
            // the first variants hit the headers
            let at = if v < 3 {
                (r >> 8) as usize % len.min(128)
            } else {
                (r >> 8) as usize % len
            };
            bad[at] ^= (1 + (lcg.next() >> 24) % 255) as u8;
        }
        write!(out, "corrupt {v}: ").unwrap();
        decode(out, &bad, None, true, None);
        mixtest(out, &format!("corrupt {v} mix"), &bad, false);
    }

    // Ogg: corrupt the packets, not the pages: fix the page checksums after.
    let mut pages = Vec::new(); // (offset, length)
    let mut audio_start = len;
    let mut p = 0;
    while p + 27 <= len && pages.len() < 256 && &data[p..p + 4] == b"OggS" {
        let h = 27 + data[p + 26] as usize;
        if p + h > len {
            break;
        }
        let body: usize = data[p + 27..p + h].iter().map(|&s| s as usize).sum();
        if p + h + body > len {
            break;
        }
        let granule = u64::from_le_bytes(data[p + 6..p + 14].try_into().unwrap());
        if granule != 0 && audio_start == len {
            audio_start = p;
        }
        pages.push((p, h + body));
        p += h + body;
    }
    for v in 0..12usize {
        if pages.is_empty() {
            break;
        }
        let start = if v < 8 { audio_start } else { pages[0].1 };
        if start >= len {
            continue;
        }
        let mut bad = data.to_vec();
        let mut lcg = Lcg((v * 104729 + len) as u32);
        let flips = 1 + v % 4;
        for _ in 0..flips {
            let r = lcg.next();
            let at = start + (r >> 8) as usize % (len - start);
            bad[at] ^= (1 + (lcg.next() >> 24) % 255) as u8;
        }
        for &(p, n) in &pages {
            let pg = &mut bad[p..p + n];
            pg[22..26].fill(0);
            let mut crc = 0u32;
            for &b in pg.iter() {
                crc ^= (b as u32) << 24;
                for _ in 0..8 {
                    crc = if crc & 0x8000_0000 != 0 {
                        (crc << 1) ^ 0x04c1_1db7
                    } else {
                        crc << 1
                    };
                }
            }
            pg[22..26].copy_from_slice(&crc.to_le_bytes());
        }
        write!(out, "ogg corrupt {v}: ").unwrap();
        decode(out, &bad, None, true, None);
        mixtest(out, &format!("ogg corrupt {v} mix"), &bad, false);
    }

    // WavPack: corrupt the blocks (their contents, then their headers too),
    // with the block checksums fixed after.
    if name.ends_with(".wv") || name.ends_with(".wvc") {
        for v in 0..18usize {
            let bad = wv_corrupt(data, v);
            write!(out, "wv corrupt {v}: ").unwrap();
            decode(out, &bad, None, true, None);
            mixtest(out, &format!("wv corrupt {v} mix"), &bad, false);
        }
    }

    mixtest(out, "mix", data, false);
    mixtest(out, "mixpre", data, true);
    if name.starts_with("opus_")
        || name.ends_with(".wv")
        || (FORCE.get().is_none() && (name.starts_with("ogg_") || name.starts_with("flac_")))
    {
        seektest(out, data);
    }
    if name.ends_with(".wv") {
        wvtest(out, name, data);
    }
    if is_module(name) {
        modtest(out, data);
        xmptest(out, data);
    }
}

/// Mixer features on one file.
fn scenarios(out: &mut String, name: &str, data: &[u8]) {
    writeln!(out, "== scenarios {name}").unwrap();

    for s in 0..13 {
        let mut spec = AudioSpec {
            format: AudioFormat::F32,
            channels: 2,
            freq: 48000,
        };
        if s == 9 {
            spec.channels = 6;
        }
        if s == 11 {
            spec = AudioSpec {
                format: AudioFormat::S16,
                channels: 1,
                freq: 22050,
            };
        }
        if s == 12 {
            spec = AudioSpec {
                format: AudioFormat::U8,
                channels: 4,
                freq: 8000,
            };
        }
        let m = Mixer::new(&spec).unwrap();
        let a = match Audio::load_io(Some(&m), &mut mem_io(data), false) {
            Ok(a) => a,
            Err(e) => {
                writeln!(out, "{s}: err: {e}").unwrap();
                continue;
            }
        };
        let t = Track::new(&m).unwrap();
        let mut t2 = None;
        let mut g = None;
        let _ = t.set_audio(Some(&a));
        let p = Properties::new();
        let mut h = FNV0;
        let stopped = Arc::new(AtomicI32::new(0));
        let raw_hash = Arc::new(AtomicU64::new(FNV0));
        let post_hash = Arc::new(AtomicU64::new(FNV0));
        {
            let stopped = stopped.clone();
            t.set_stopped_callback(Some(move |_: &Track| {
                stopped.fetch_add(1, Ordering::SeqCst);
            }))
            .unwrap();
        }
        match s {
            0 => {
                t.set_gain(0.5).unwrap();
                m.set_gain(0.8).unwrap();
            }
            1 => {
                p.set(PROP_PLAY_FADE_IN_FRAMES_NUMBER, 1000i64).unwrap();
                p.set(PROP_PLAY_MAX_FRAME_NUMBER, 6000i64).unwrap();
                p.set(PROP_PLAY_APPEND_SILENCE_FRAMES_NUMBER, 500i64)
                    .unwrap();
            }
            2 => t
                .set_stereo(Some(&StereoGains {
                    left: 0.3,
                    right: 0.9,
                }))
                .unwrap(),
            3 => t
                .set_3d_position(Some(&Point3D {
                    x: 1.0,
                    y: 0.0,
                    z: -1.0,
                }))
                .unwrap(),
            4 => {
                t.set_frequency_ratio(1.5).unwrap();
                m.set_frequency_ratio(0.75).unwrap();
            }
            5 => {
                let t2v = Track::new(&m).unwrap();
                let _ = t2v.set_audio(Some(&a));
                t.tag("a").unwrap();
                t2v.tag("a").unwrap();
                t2v.tag("b").unwrap();
                m.set_tag_gain("a", 0.5).unwrap();
                t2 = Some(t2v);
            }
            6 => {
                p.set(PROP_PLAY_LOOPS_NUMBER, 2i64).unwrap();
                p.set(PROP_PLAY_LOOP_START_FRAME_NUMBER, 1000i64).unwrap();
                p.set(PROP_PLAY_START_MILLISECOND_NUMBER, 5i64).unwrap();
            }
            7 => t.set_output_channel_map(Some(&[1, 0])).unwrap(),
            8 => {}
            9 => t
                .set_3d_position(Some(&Point3D {
                    x: -1.0,
                    y: 0.5,
                    z: 1.0,
                }))
                .unwrap(),
            10 => {
                let gv = Group::new(&m).unwrap();
                t.set_group(Some(&gv)).unwrap();
                t.set_cooked_callback(Some(|_: &Track, _: &AudioSpec, pcm: &mut [f32]| {
                    for v in pcm.iter_mut() {
                        *v *= 0.5;
                    }
                }))
                .unwrap();
                {
                    let raw_hash = raw_hash.clone();
                    t.set_raw_callback(Some(move |_: &Track, _: &AudioSpec, pcm: &mut [f32]| {
                        let h = raw_hash.load(Ordering::SeqCst);
                        raw_hash.store(fnv_f32(h, pcm), Ordering::SeqCst);
                    }))
                    .unwrap();
                }
                gv.set_postmix_callback(Some(|_: &Group, _: &AudioSpec, pcm: &mut [f32]| {
                    for v in pcm.iter_mut() {
                        *v = -*v;
                    }
                }))
                .unwrap();
                {
                    let post_hash = post_hash.clone();
                    m.set_postmix_callback(Some(
                        move |_: &Mixer, spec: &AudioSpec, pcm: &mut [f32]| {
                            let h = fnv_f32(post_hash.load(Ordering::SeqCst), pcm);
                            post_hash.store(fnv(h, &spec.channels.to_ne_bytes()), Ordering::SeqCst);
                        },
                    ))
                    .unwrap();
                }
                g = Some(gv);
            }
            _ => p.set(PROP_PLAY_LOOPS_NUMBER, -1i64).unwrap(),
        }
        let ok = if s == 5 {
            p.set(PROP_PLAY_START_FRAME_NUMBER, 100i64).unwrap();
            m.play_tag("a", Some(&p)).is_ok()
        } else {
            t.play(Some(&p)).is_ok()
        };
        generate(&m, 1, &mut h);
        if s == 5 {
            let _ = m.stop_tag("b", 10);
        }
        if s == 8 {
            let _ = t.pause();
        }
        generate(&m, 1, &mut h);
        if s == 8 {
            let _ = t.resume();
        }
        if s == 1 {
            let _ = t.stop(700);
        }
        generate(&m, 4, &mut h);
        writeln!(
            out,
            "{s}: play={} playing={} paused={} pos={} remaining={} stopped={} raw={:016x} post={:016x} {h:016x}",
            ok as i32,
            t.playing() as i32,
            t.paused() as i32,
            t.playback_position().unwrap_or(-1),
            t.remaining().unwrap_or(-1),
            stopped.load(Ordering::SeqCst),
            raw_hash.load(Ordering::SeqCst),
            post_hash.load(Ordering::SeqCst),
        )
        .unwrap();
        drop(t2);
        drop(t);
        drop(g);
    }
}

fn misc(out: &mut String) {
    writeln!(out, "== misc").unwrap();
    write!(out, "decoders:").unwrap();
    for i in 0..num_audio_decoders().unwrap() {
        write!(out, " {}", audio_decoder(i).unwrap()).unwrap();
    }
    writeln!(out).unwrap();

    let spec = AudioSpec {
        format: AudioFormat::F32,
        channels: 2,
        freq: 48000,
    };
    let m = Mixer::new(&spec).unwrap();
    for (hz, amp, ms) in [(440, 0.25f32, 100i64), (1000, 1.0, -1), (30, 0.5, 7)] {
        let a = Audio::sine_wave(Some(&m), hz, amp, ms).unwrap();
        let t = Track::new(&m).unwrap();
        let _ = t.set_audio(Some(&a));
        let _ = t.play(None);
        let mut h = FNV0;
        generate(&m, 4, &mut h);
        writeln!(
            out,
            "sine {hz}: dur={} playing={} {h:016x}",
            a.duration().unwrap_or(-1),
            t.playing() as i32
        )
        .unwrap();
    }

    let mut raw = [0u8; 6000];
    let mut lcg = Lcg(99);
    for b in raw.iter_mut() {
        *b = (lcg.next() >> 24) as u8;
    }
    let rawspecs = [
        (AudioFormat::S16, 1, 11025),
        (AudioFormat::U8, 2, 8000),
        (AudioFormat::F32, 1, 44100),
        (AudioFormat::S32BE, 3, 22050),
    ];
    for (i, (format, channels, freq)) in rawspecs.into_iter().enumerate() {
        let rawspec = AudioSpec {
            format,
            channels,
            freq,
        };
        let a = match Audio::load_raw(Some(&m), &raw, &rawspec) {
            Ok(a) => a,
            Err(e) => {
                writeln!(out, "raw {i}: err: {e}").unwrap();
                continue;
            }
        };
        let t = Track::new(&m).unwrap();
        let _ = t.set_audio(Some(&a));
        let _ = t.play(None);
        let mut h = FNV0;
        generate(&m, 3, &mut h);
        writeln!(out, "raw {i}: dur={} {h:016x}", a.duration().unwrap_or(-1)).unwrap();
    }
}

/// Whether an output line matches the reference's: the same, or (where
/// upstream fails without setting an error message, which prints as an
/// empty one) failing the same way with some message.
fn line_matches(got: &str, want: &str) -> bool {
    got == want || (want.ends_with("err: ") && got.starts_with(want))
}

/// Compare `out` with the reference's lines for `sections` (the lines from
/// `== section` up to the next section's).
fn compare(out: &str, sections: &[&str]) {
    compare_with(REFERENCE, out, sections);
}

/// [`compare()`], with another reference.
fn compare_with(reference: &[u8], out: &str, sections: &[&str]) {
    #[allow(unused_mut)]
    let mut reference = reference_sections(reference);
    // (upstream was built with its MPG123 decoder; without the `mpg123`
    // feature, the sections it changes are those of a build without it.)
    #[cfg(not(feature = "mpg123"))]
    reference.extend(reference_sections(REFERENCE_NO_MPG123));
    let mut want = String::new();
    for name in sections {
        let section = reference
            .get(*name)
            .unwrap_or_else(|| panic!("no reference for {name}"));
        for line in section.lines() {
            // (upstream was built with its FLUIDSYNTH, TIMIDITY and GME
            // decoders; without the `fluidsynth`, `timidity` and `gme`
            // features, there are none.)
            if line.starts_with("decoders:") {
                let mut line = line.to_string();
                if !cfg!(feature = "timidity") {
                    line = line.replace(" TIMIDITY", "");
                }
                if !cfg!(feature = "fluidsynth") {
                    line = line.replace(" FLUIDSYNTH", "");
                }
                if !cfg!(feature = "gme") {
                    line = line.replace(" GME", "");
                }
                want.push_str(&line);
            } else {
                want.push_str(line);
            }
            want.push('\n');
        }
    }
    assert!(!want.is_empty(), "no reference for {sections:?}");
    let same = out.lines().count() == want.lines().count()
        && out
            .lines()
            .zip(want.lines())
            .all(|(a, b)| line_matches(a, b));
    if !same {
        let mut report = String::new();
        let mut shown = 0;
        for (i, (a, b)) in out.lines().zip(want.lines()).enumerate() {
            if !line_matches(a, b) {
                writeln!(report, "line {i}:\n  got:  {a}\n  want: {b}").unwrap();
                shown += 1;
                if shown == 40 {
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

/// sdl3-mixer-timidity's test data: its configuration (which upstream's
/// TIMIDITY decoder reads from `TIMIDITY_CFG`) and MIDI files.
#[cfg(any(feature = "timidity", feature = "fluidsynth"))]
fn timidity_testdata() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../sdl3-mixer-timidity/testdata")
}

fn setup() {
    // The TIMIDITY decoder reads its configuration when the library is
    // initialized; the C program that made the reference ran with
    // TIMIDITY_CFG set to the same file (and TIMIDITY_SOUNDFONT unset).
    #[cfg(feature = "timidity")]
    {
        let cfg = timidity_testdata().join("timidity.cfg");
        sdl3::stdlib::setenv_unsafe("TIMIDITY_CFG", &cfg.to_string_lossy(), true).unwrap();
        sdl3::stdlib::unsetenv_unsafe("TIMIDITY_SOUNDFONT").unwrap();
    }
    crate::init().unwrap();
}

#[test]
fn misc_matches_upstream() {
    setup();
    let mut out = String::new();
    misc(&mut out);
    compare(&out, &["misc"]);
}

fn files_with_prefix(prefixes: &[&str]) {
    setup();
    // (the files, and those of them tested again with a forced decoder,
    // in the reference's order)
    let mut tests: Vec<(String, &str, &[u8], Option<&'static str>)> = Vec::new();
    for (name, data) in FILES.iter().chain(MODULES) {
        if prefixes.iter().any(|p| name.starts_with(p)) {
            tests.push((name.to_string(), name, data, None));
            for &(_, force) in FORCED.iter().filter(|(n, _)| n == name) {
                tests.push((format!("{name} {force}"), name, data, Some(force)));
            }
        }
    }
    tests.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = String::new();
    for (_, name, data, force) in &tests {
        FORCE.set(*force);
        test_file(&mut out, name, data);
        FORCE.set(None);
    }
    let names: Vec<&str> = tests.iter().map(|t| t.0.as_str()).collect();
    compare(&out, &names);
}

#[test]
fn wav_matches_upstream() {
    files_with_prefix(&["wav_"]);
}

#[test]
fn aiff_matches_upstream() {
    files_with_prefix(&["aiff_", "aifc_"]);
}

#[test]
fn au_matches_upstream() {
    files_with_prefix(&["au_"]);
}

#[test]
fn voc_matches_upstream() {
    files_with_prefix(&["voc_"]);
}

#[test]
fn mp3_matches_upstream() {
    files_with_prefix(&["mp3_", "mp2_"]);
}

// (the Ogg Vorbis files in a few tests, so they run in parallel)
#[test]
fn vorbis_channels_matches_upstream() {
    files_with_prefix(&["ogg_3", "ogg_6", "ogg_8"]);
}

#[test]
fn vorbis_abr_chained_matches_upstream() {
    files_with_prefix(&["ogg_a", "ogg_c"]);
}

#[test]
fn vorbis_floor0_holes_mux_matches_upstream() {
    files_with_prefix(&["ogg_f", "ogg_h", "ogg_m"]);
}

#[test]
fn vorbis_quality_short_matches_upstream() {
    files_with_prefix(&["ogg_q", "ogg_s"]);
}

// (the FLAC files in a few tests, so they run in parallel)
#[test]
fn flac_matches_upstream() {
    files_with_prefix(&["flac_6", "flac_f", "flac_o", "flac_s"]);
}

#[test]
fn flac_crafted_matches_upstream() {
    files_with_prefix(&["flac_c_e", "flac_c_s1", "flac_c_s8"]);
}

#[test]
fn flac_crafted_wide_matches_upstream() {
    files_with_prefix(&["flac_c_s2", "flac_c_s3"]);
}

#[test]
fn mod_matches_upstream() {
    files_with_prefix(&["mod_"]);
}

#[test]
fn xm_matches_upstream() {
    files_with_prefix(&["xm_"]);
}

#[test]
fn s3m_matches_upstream() {
    files_with_prefix(&["s3m_"]);
}

#[test]
fn it_matches_upstream() {
    files_with_prefix(&["it_"]);
}

/// One reference test per module format prefix.
macro_rules! module_tests {
    ($($name:ident: $prefix:literal,)*) => {
        $(
            #[test]
            fn $name() {
                files_with_prefix(&[$prefix]);
            }
        )*
    };
}

// The other formats libxmp loads (one test each, so they run in parallel).
module_tests! {
    c669_matches_upstream: "669_",
    abk_matches_upstream: "abk_",
    amf_matches_upstream: "amf_",
    arch_matches_upstream: "arch_",
    asylum_matches_upstream: "asylum_",
    chip_matches_upstream: "chip_",
    coco_matches_upstream: "coco_",
    dbm_matches_upstream: "dbm_",
    digi_matches_upstream: "digi_",
    dt_matches_upstream: "dt_",
    emod_matches_upstream: "emod_",
    far_matches_upstream: "far_",
    flt_matches_upstream: "flt_",
    fnk_matches_upstream: "fnk_",
    gal4_matches_upstream: "gal4_",
    gal5_matches_upstream: "gal5_",
    gdm_matches_upstream: "gdm_",
    hmn_matches_upstream: "hmn_",
    ice_matches_upstream: "ice_",
    imf_matches_upstream: "imf_",
    ims_matches_upstream: "ims_",
    liq_matches_upstream: "liq_",
    masi16_matches_upstream: "masi16_",
    masi_matches_upstream: "masi_",
    mdl_matches_upstream: "mdl_",
    med2_matches_upstream: "med2_",
    med3_matches_upstream: "med3_",
    med4_matches_upstream: "med4_",
    mgt_matches_upstream: "mgt_",
    mmd0_matches_upstream: "mmd0_",
    mmd1_matches_upstream: "mmd1_",
    mmd2_matches_upstream: "mmd2_",
    mmd3_matches_upstream: "mmd3_",
    mtm_matches_upstream: "mtm_",
    muse_matches_upstream: "muse_",
    no_matches_upstream: "no_",
    okt_matches_upstream: "okt_",
    pt3_matches_upstream: "pt3_",
    ptm_matches_upstream: "ptm_",
    rtm_matches_upstream: "rtm_",
    sfx_matches_upstream: "sfx_",
    st_matches_upstream: "st_",
    stim_matches_upstream: "stim_",
    stm_matches_upstream: "stm_",
    stx_matches_upstream: "stx_",
    sym_matches_upstream: "sym_",
    ult_matches_upstream: "ult_",
    umx_matches_upstream: "umx_",
    xmf_matches_upstream: "xmf_",
}
// (end of the other formats)

#[test]
fn opus_matches_upstream() {
    files_with_prefix(&["opus_"]);
}

#[test]
fn wavpack_matches_upstream() {
    files_with_prefix(&["wv_"]);
}

/// The MIDI files, in the reference's order (sdl3-mixer-timidity's, read
/// at run time, as the decoder reads its patches).
#[cfg(any(feature = "timidity", feature = "fluidsynth"))]
static MIDI_FILES: &[&str] = &[
    "bad_no_eot.mid",
    "basic.mid",
    "empty.mid",
    "format1.mid",
    "oom.mid",
    "rmid.rmi",
];

/// Whether there is a SoundFont where the FLUIDSYNTH decoder looks for one
/// by itself (then it doesn't play the MIDI files silently without one, as
/// the reference's C did).
#[cfg(feature = "fluidsynth")]
fn system_soundfont() -> bool {
    cfg!(all(unix, not(target_vendor = "apple")))
        && [
            "/usr/share/sounds/sf2/default-GM.sf2",
            "/usr/share/soundfonts/default.sf2",
            "/usr/share/sounds/sf2/default.sf2",
        ]
        .iter()
        .any(|p| std::path::Path::new(p).exists())
}

/// MIDI through the FLUIDSYNTH decoder without a SoundFont (the files
/// starting with "MThd") and the TIMIDITY decoder (the RMID file, and the
/// others with it forced), and the mixer's features on a MIDI file.
/// TiMidity's MIDI reader keeps state from one file to the next (see
/// sdl3-mixer-timidity), so this is the only test that plays MIDI with it,
/// in the reference's order.
#[cfg(any(feature = "timidity", feature = "fluidsynth"))]
#[test]
fn midi_matches_upstream() {
    setup();
    let dir = timidity_testdata().join("midi");
    if !dir.is_dir() {
        // (a packaged crate doesn't have sdl3-mixer-timidity's test data.)
        eprintln!("skipped: no {}", dir.display());
        return;
    }
    #[cfg(feature = "fluidsynth")]
    let fluidsynth = !system_soundfont();
    #[cfg(not(feature = "fluidsynth"))]
    let fluidsynth = false;
    if cfg!(feature = "fluidsynth") && !fluidsynth {
        eprintln!("skipped FLUIDSYNTH without a SoundFont: one is installed");
    }
    // (the reference's sections: each file, then with TIMIDITY forced)
    let mut tests: Vec<(String, &str, Option<&'static str>)> = Vec::new();
    for name in MIDI_FILES {
        if name.ends_with(".mid") {
            if fluidsynth {
                tests.push((name.to_string(), name, None));
            }
            if cfg!(feature = "timidity") {
                tests.push((format!("{name} TIMIDITY"), name, Some("TIMIDITY")));
            }
        } else if cfg!(feature = "timidity") {
            tests.push((name.to_string(), name, None));
        }
    }
    let mut out = String::new();
    for (_, name, force) in &tests {
        let data = std::fs::read(dir.join(name)).unwrap();
        FORCE.set(*force);
        test_file(&mut out, name, &data);
        FORCE.set(None);
    }
    let names: Vec<&str> = tests.iter().map(|t| t.0.as_str()).collect();
    compare(&out, &names);
    if fluidsynth {
        let data = std::fs::read(dir.join("basic.mid")).unwrap();
        let mut out = String::new();
        scenarios(&mut out, "basic.mid", &data);
        compare(&out, &["scenarios basic.mid"]);
    }
}

/// sdl3-mixer-fluidsynth's test data: its SoundFont and MIDI files.
#[cfg(feature = "fluidsynth")]
fn fluidsynth_testdata() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../sdl3-mixer-fluidsynth/testdata")
}

/// The MIDI files the FLUIDSYNTH sections play, in the reference's order.
#[cfg(feature = "fluidsynth")]
static FLUIDSYNTH_FILES: &[&str] = &[
    "basic.mid",
    "format1.mid",
    "mono.mid",
    "nrpn.mid",
    "sysex.mid",
];

/// The FLUIDSYNTH decoder with a SoundFont: by path, from a stream, with
/// settings, and with broken or missing SoundFonts.
#[cfg(feature = "fluidsynth")]
fn fstest(out: &mut String, sf2path: &std::path::Path, name: &str, data: &[u8]) {
    use crate::internal::{
        PROP_DECODER_FLUIDSYNTH_PROPS_NUMBER, PROP_DECODER_FLUIDSYNTH_SOUNDFONT_IOSTREAM_POINTER,
        PROP_DECODER_FLUIDSYNTH_SOUNDFONT_PATH_STRING,
    };
    let sf = std::fs::read(sf2path).unwrap();
    writeln!(out, "== fluidsynth {name}").unwrap();
    let extra = Properties::new();
    extra
        .set(
            PROP_DECODER_FLUIDSYNTH_SOUNDFONT_PATH_STRING,
            sf2path.to_str().unwrap(),
        )
        .unwrap();
    write!(out, "open: ").unwrap();
    decode_props(out, data, None, true, None, Some(&extra));
    let s16 = AudioSpec {
        format: AudioFormat::S16,
        channels: 1,
        freq: 22050,
    };
    write!(out, "s16m22: ").unwrap();
    decode_props(out, data, Some(&s16), false, None, Some(&extra));
    mixtest_props(out, "mix", data, false, Some(&extra));
    mixtest_props(out, "mixpre", data, true, Some(&extra));
    seektest_props(out, "seek", data, Some(&extra));

    let p2 = Properties::new();
    p2.set_any(
        PROP_DECODER_FLUIDSYNTH_SOUNDFONT_IOSTREAM_POINTER,
        std::sync::Mutex::new(mem_io(&sf)),
    )
    .unwrap();
    write!(out, "stream: ").unwrap();
    decode_props(out, data, None, true, None, Some(&p2));
    // (read to its end: the second time, the SoundFont is empty)
    write!(out, "stream again: ").unwrap();
    decode_props(out, data, None, true, None, Some(&p2));

    let fsp = Properties::new();
    fsp.set("synth.polyphony", 6i64).unwrap();
    fsp.set("synth.reverb.room-size", 0.9f32).unwrap();
    fsp.set("synth.midi-bank-select", "xg").unwrap();
    fsp.set("synth.reverb.active", false).unwrap(); // (not a number, float or string: ignored)
    fsp.set("synth.gain", 3i64).unwrap(); // (an int for a number setting: fails)
    fsp.set("no.such.setting", "x").unwrap();
    extra
        .set_any(PROP_DECODER_FLUIDSYNTH_PROPS_NUMBER, fsp)
        .unwrap();
    write!(out, "settings: ").unwrap();
    decode_props(out, data, None, true, None, Some(&extra));
    mixtest_props(out, "settings mix", data, false, Some(&extra));

    let bad = std::env::temp_dir().join(format!(
        "sdl3-mixer-fluidsynth-test-{}.sf2",
        std::process::id()
    ));
    std::fs::write(&bad, &sf[..sf.len() / 2]).unwrap();
    let p3 = Properties::new();
    p3.set(
        PROP_DECODER_FLUIDSYNTH_SOUNDFONT_PATH_STRING,
        bad.to_str().unwrap(),
    )
    .unwrap();
    write!(out, "badsf: ").unwrap();
    decode_props(out, data, None, true, None, Some(&p3));
    mixtest_props(out, "badsf mix", data, false, Some(&p3));
    let _ = std::fs::remove_file(&bad);
    write!(out, "nosf: ").unwrap();
    decode_props(out, data, None, true, None, Some(&p3));
}

/// MIDI through the FLUIDSYNTH decoder with sdl3-mixer-fluidsynth's
/// SoundFont.
#[cfg(feature = "fluidsynth")]
#[test]
fn fluidsynth_matches_upstream() {
    setup();
    let dir = fluidsynth_testdata();
    if !dir.is_dir() {
        // (a packaged crate doesn't have sdl3-mixer-fluidsynth's test data.)
        eprintln!("skipped: no {}", dir.display());
        return;
    }
    if system_soundfont() {
        eprintln!("skipped: a SoundFont is installed where FLUIDSYNTH looks for one");
        return;
    }
    let sf2 = dir.join("test.sf2");
    let mut out = String::new();
    let mut names = Vec::new();
    for name in FLUIDSYNTH_FILES {
        let data = std::fs::read(dir.join("midi").join(name)).unwrap();
        fstest(&mut out, &sf2, name, &data);
        names.push(format!("fluidsynth {name}"));
    }
    let names: Vec<&str> = names.iter().map(|n| n.as_str()).collect();
    compare(&out, &names);
}

/// The reference for the GME decoder (sdl3-mixer-gme's rips, read at run
/// time).
#[cfg(feature = "gme")]
static REFERENCE_GME: &[u8] = include_bytes!("testdata/reference_gme.txt");

/// sdl3-mixer-gme's test data: rips of tiny sound programs.
#[cfg(feature = "gme")]
fn gme_testdata() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../sdl3-mixer-gme/testdata")
}

/// A GME decoder over the data (a bounded amount of it: libgme plays on
/// and on), with its track information.
#[cfg(feature = "gme")]
fn gme_decode(
    out: &mut String,
    data: &[u8],
    want: Option<&AudioSpec>,
    force: Option<&str>,
    cap: usize,
) {
    let props = force.map(|name| {
        let p = Properties::new();
        p.set(PROP_AUDIO_DECODER_STRING, name).unwrap();
        p
    });
    let d = match AudioDecoder::new_io(mem_io(data), props.as_ref()) {
        Ok(d) => d,
        Err(e) => {
            writeln!(out, "err: {e}").unwrap();
            return;
        }
    };
    let mut spec = d.format().unwrap();
    let p = d.properties().unwrap();
    write!(
        out,
        "{} 0x{:04x} {} {}",
        p.get_string(PROP_AUDIO_DECODER_STRING)
            .unwrap_or_else(|| "?".into()),
        spec.format.0,
        spec.channels,
        spec.freq
    )
    .unwrap();
    print_props(out, &p);
    for key in [
        "song",
        "author",
        "game",
        "copyright",
        "system",
        "comment",
        "dumper",
    ] {
        if let Some(s) = p.get_string(&format!("SDL_mixer.metadata.gme.{key}")) {
            write!(out, " gme.{key}=\"{s}\"").unwrap();
        }
    }
    write!(out, " | ").unwrap();
    if let Some(want) = want {
        spec = *want;
    }
    let mut buf = [0u8; 4096];
    let mut h = FNV0;
    let mut total = 0usize;
    let mut error = None;
    let mut rc: i32;
    loop {
        rc = match d.decode(&mut buf, &spec) {
            Ok(n) => n as i32,
            Err(e) => {
                error = Some(e);
                -1
            }
        };
        if rc <= 0 {
            break;
        }
        h = fnv(h, &buf[..rc as usize]);
        total += rc as usize;
        if total >= cap {
            rc = 1;
            break;
        }
    }
    write!(out, "{total} {h:016x} end={rc}").unwrap();
    if let Some(e) = error {
        write!(out, " err: {e}").unwrap();
    }
    writeln!(out).unwrap();
}

/// Seek a track around a rip, printing what happens.
#[cfg(feature = "gme")]
fn gme_seeks(out: &mut String, label: &str, m: &Mixer, t: &Track) {
    for target in [0i64, 2205, 48000, 20000, 100] {
        let seeked = t.set_playback_position(target).is_ok();
        let pos = t.playback_position().unwrap_or(-1);
        let mut h = FNV0;
        generate(m, 1, &mut h);
        writeln!(
            out,
            "{label} {target}: seek={} pos={pos} playing={} {h:016x}",
            seeked as i32,
            t.playing() as i32
        )
        .unwrap();
    }
}

/// One of sdl3-mixer-gme's rips through the GME decoder: decoded, mixed,
/// started at tracks (orders) and seeked, truncated and corrupted.
#[cfg(feature = "gme")]
fn gmetest(out: &mut String, name: &str, data: &[u8]) {
    let len = data.len();
    writeln!(out, "== gme {name}").unwrap();
    write!(out, "open: ").unwrap();
    gme_decode(out, data, None, None, 96 * 1024);
    let f32s48 = AudioSpec {
        format: AudioFormat::F32,
        channels: 2,
        freq: 48000,
    };
    write!(out, "f32s48: ").unwrap();
    gme_decode(out, data, Some(&f32s48), None, 96 * 1024);
    let s16m22 = AudioSpec {
        format: AudioFormat::S16,
        channels: 1,
        freq: 22050,
    };
    write!(out, "s16m22: ").unwrap();
    gme_decode(out, data, Some(&s16m22), None, 32 * 1024);
    write!(out, "forced: ").unwrap();
    gme_decode(out, data, None, Some("GME"), 16 * 1024);
    for cut in [0, 3, 4, 16, len / 2, len - 1] {
        write!(out, "trunc {cut}: ").unwrap();
        gme_decode(out, &data[..cut], None, None, 16 * 1024);
    }
    for v in 0..4usize {
        let mut bad = data.to_vec();
        let mut lcg = Lcg((v * 7919 + len) as u32);
        let flips = 1 + v * 3;
        for _ in 0..flips {
            let r = lcg.next();
            let at = 4 + (r >> 8) as usize % (len - 4);
            bad[at] ^= (1 + (lcg.next() >> 24) % 255) as u8;
        }
        write!(out, "corrupt {v}: ").unwrap();
        gme_decode(out, &bad, None, None, 16 * 1024);
        mixtest(out, &format!("corrupt {v} mix"), &bad, false);
    }
    mixtest(out, "mix", data, false);

    let spec = AudioSpec {
        format: AudioFormat::F32,
        channels: 2,
        freq: 48000,
    };
    let m = Mixer::new(&spec).unwrap();
    let a = match Audio::load_io(Some(&m), &mut mem_io(data), false) {
        Ok(a) => a,
        Err(e) => {
            writeln!(out, "gmetracks: err: {e}").unwrap();
            return;
        }
    };
    let t = Track::new(&m).unwrap();
    let _ = t.set_audio(Some(&a));
    for order in [1i64, 2, 0, 200, -5, 3] {
        let p = Properties::new();
        p.set(PROP_PLAY_START_ORDER_NUMBER, order).unwrap();
        let played = t.play(Some(&p));
        let mut h = FNV0;
        generate(&m, 2, &mut h);
        write!(out, "order {order}: play={}", played.is_ok() as i32).unwrap();
        if let Err(e) = &played {
            write!(out, " err: {e}").unwrap();
        }
        writeln!(
            out,
            " pos={} playing={} {h:016x}",
            t.playback_position().unwrap_or(-1),
            t.playing() as i32
        )
        .unwrap();
    }
    let replay = t.play(None);
    write!(out, "replay: {}", replay.is_ok() as i32).unwrap();
    if let Err(e) = &replay {
        write!(out, " err: {e}").unwrap();
    }
    writeln!(out, " playing={}", t.playing() as i32).unwrap();
    gme_seeks(out, "gmeseek", &m, &t);
    // the same on a fresh track (the one above may have stopped)
    drop(t);
    let t = Track::new(&m).unwrap();
    let _ = t.set_audio(Some(&a));
    let replay = t.play(None);
    write!(out, "fresh: {}", replay.is_ok() as i32).unwrap();
    if let Err(e) = &replay {
        write!(out, " err: {e}").unwrap();
    }
    writeln!(out, " playing={}", t.playing() as i32).unwrap();
    gme_seeks(out, "freshseek", &m, &t);
}

/// The GME decoder on sdl3-mixer-gme's rips whose names start with
/// `prefix`, in the reference's order.
#[cfg(feature = "gme")]
fn gme_files(prefix: &str) {
    setup();
    let dir = gme_testdata();
    if !dir.is_dir() {
        // (a packaged crate doesn't have sdl3-mixer-gme's test data.)
        eprintln!("skipped: no {}", dir.display());
        return;
    }
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(prefix) && !n.starts_with("reference"))
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no {prefix}* rips");
    let mut out = String::new();
    let mut sections = Vec::new();
    for name in &names {
        let data = std::fs::read(dir.join(name)).unwrap();
        gmetest(&mut out, name, &data);
        sections.push(format!("gme {name}"));
    }
    let sections: Vec<&str> = sections.iter().map(|s| s.as_str()).collect();
    compare_with(REFERENCE_GME, &out, &sections);
}

#[cfg(feature = "gme")]
#[test]
fn gme_ay_matches_upstream() {
    gme_files("ay_");
}

#[cfg(feature = "gme")]
#[test]
fn gme_gbs_matches_upstream() {
    gme_files("gbs_");
}

#[cfg(feature = "gme")]
#[test]
fn gme_hes_matches_upstream() {
    gme_files("hes_");
}

#[cfg(feature = "gme")]
#[test]
fn gme_kss_matches_upstream() {
    gme_files("kss_");
}

#[cfg(feature = "gme")]
#[test]
fn gme_nsf_matches_upstream() {
    gme_files("nsf_");
}

#[cfg(feature = "gme")]
#[test]
fn gme_nsfe_matches_upstream() {
    gme_files("nsfe_");
}

#[cfg(feature = "gme")]
#[test]
fn gme_sap_matches_upstream() {
    gme_files("sap_");
}

#[cfg(feature = "gme")]
#[test]
fn gme_spc_matches_upstream() {
    gme_files("spc_");
}

#[cfg(feature = "gme")]
#[test]
fn gme_vgm_matches_upstream() {
    gme_files("vgm_");
}

#[test]
fn scenarios_match_upstream() {
    setup();
    for name in SCENARIO_FILES {
        let mut out = String::new();
        scenarios(&mut out, name, file(name));
        compare(&out, &[&format!("scenarios {name}")]);
    }
}
