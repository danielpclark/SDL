//! Tests that pin the audio core to upstream's C behaviour: each one feeds
//! deterministic inputs through the translation and compares a hash of the
//! outputs with the hash printed by upstream's own code (SDL_audiotypecvt.c,
//! SDL_audiocvt.c, SDL_audioqueue.c, SDL_audioresample.c, SDL_mixer.c and
//! SDL_wave.c compiled for x86-64 with SSE, against stubbed SDL internals)
//! over the same inputs.

use super::convert::{convert_audio, ConvertSrc};
use super::format::{AudioFormat, AudioSpec};
use super::resample;
use super::*;
use crate::io::IoStream;

/// The harness's LCG (`rnd()`, `rbyte()`, `rfloat()`).
struct Rng(u64);

impl Rng {
    fn rnd(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    fn byte(&mut self) -> u8 {
        (self.rnd() >> 7) as u8
    }
    fn float(&mut self) -> f32 {
        let v = (self.rnd() % 24001) as i32 - 12000;
        v as f32 / 10000.0
    }
}

/// FNV-1a, 64 bits.
struct Hash(u64);

impl Hash {
    fn new() -> Hash {
        Hash(0xcbf29ce484222325)
    }
    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= x as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_ne_bytes());
    }
}

const FORMATS: [AudioFormat; 8] = [
    AudioFormat::U8,
    AudioFormat::S8,
    AudioFormat::S16LE,
    AudioFormat::S16BE,
    AudioFormat::S32LE,
    AudioFormat::S32BE,
    AudioFormat::F32LE,
    AudioFormat::F32BE,
];

/// Random samples of `fmt` (`fill()`).
fn fill(rng: &mut Rng, buf: &mut [u8], fmt: AudioFormat, samples: usize) {
    if fmt.is_float() {
        for i in 0..samples {
            let f = rng.float();
            let b = if fmt.is_big_endian() {
                f.to_be_bytes()
            } else {
                f.to_le_bytes()
            };
            buf[i * 4..i * 4 + 4].copy_from_slice(&b);
        }
    } else {
        for b in &mut buf[..samples * fmt.bytesize() as usize] {
            *b = rng.byte();
        }
    }
}

/// A stream that `quit_audio()` leaves alone. Tests that init and quit the
/// audio subsystem run in parallel with the others, and quitting destroys
/// every auto-cleanup stream (like upstream's `SDL_QuitAudio()`).
fn stream_surviving_quit(src: Option<&AudioSpec>, dst: Option<&AudioSpec>) -> AudioStream {
    let s = AudioStream::new(src, dst).unwrap();
    s.properties()
        .set(PROP_AUDIOSTREAM_AUTO_CLEANUP_BOOLEAN, false)
        .unwrap();
    s
}

#[test]
fn convert_audio_matches_upstream() {
    const EXPECTED: [u64; 8] = [
        0x322b21297b629d81,
        0x1f845b30eddf3568,
        0x110d0b94943adca3,
        0x4e89b7a49c5bddcc,
        0xeb972b481159ad45,
        0x59ccb7d959758134,
        0x4789f22169ddfce7,
        0xac602e60162f5066,
    ];
    let mut src = [0u8; 8 * 4 * 64];
    let mut dst = [0u8; 8 * 4 * 64];
    let mut scratch = [0u8; 8 * 4 * 64];
    let mut rng = Rng(1);
    let mut combo = 0;
    for (sf, expected) in EXPECTED.iter().enumerate() {
        let mut h = Hash::new();
        for sc in 1..=8i32 {
            for &df in &FORMATS {
                for dc in 1..=8i32 {
                    let frames = 13;
                    let smap: Vec<i32> = (0..sc).map(|i| sc - 1 - i).collect();
                    let dmap: Vec<i32> = (0..dc)
                        .map(|i| if i == 0 { -1 } else { (dc - i) % dc })
                        .collect();
                    let gain = if combo % 3 == 0 { 0.7 } else { 1.0 };
                    let sm = (combo % 5 == 0).then_some(&smap[..]);
                    let dm = (combo % 7 == 0).then_some(&dmap[..]);
                    fill(&mut rng, &mut src, FORMATS[sf], frames * sc as usize);
                    dst.fill(0xAA);
                    convert_audio(
                        frames,
                        ConvertSrc::Slice(&src),
                        FORMATS[sf],
                        sc,
                        sm,
                        &mut dst,
                        df,
                        dc,
                        dm,
                        Some(&mut scratch),
                        gain,
                    );
                    h.bytes(&dst[..frames * dc as usize * df.bytesize() as usize]);
                    combo += 1;
                }
            }
        }
        assert_eq!(h.0, *expected, "source format {}", FORMATS[sf]);
    }
}

#[test]
fn resampler_matches_upstream() {
    let rates = [
        (44100, 48000),
        (48000, 44100),
        (22050, 96000),
        (96000, 8000),
        (8000, 11025),
        (44100, 44099),
    ];
    let mut input = vec![0u8; (100 + 14) * 8 * 4];
    let mut out = vec![0u8; 2000 * 8 * 4];
    let mut h = Hash::new();
    let mut rng = Rng(2);
    for (src_rate, dst_rate) in rates {
        for chans in 1..=8usize {
            let inframes = 100;
            for i in 0..(inframes + 14) * chans {
                input[i * 4..i * 4 + 4].copy_from_slice(&rng.float().to_ne_bytes());
            }
            let rate = resample::get_resample_rate(src_rate, dst_rate);
            let mut off = ((rng.rnd() % 1000) as i64) << 20;
            let mut off2 = off;
            let outframes = resample::get_resampler_output_frames(inframes as i64, rate, &mut off2);
            resample::resample_audio(
                chans,
                &input,
                7 * chans * 4,
                inframes as i32,
                &mut out,
                outframes as usize,
                rate,
                &mut off,
            );
            h.bytes(&out[..outframes as usize * chans * 4]);
            h.bytes(&off.to_ne_bytes());
            h.bytes(&outframes.to_ne_bytes());
        }
    }
    assert_eq!(h.0, 0xd37b688289c0990b);
}

#[test]
fn streams_match_upstream() {
    struct Case {
        src: AudioSpec,
        dst: AudioSpec,
        gain: f32,
        ratio: f32,
        remap: bool,
    }
    let spec = AudioSpec::new;
    let cases = [
        Case {
            src: spec(AudioFormat::S16LE, 2, 44100),
            dst: spec(AudioFormat::F32LE, 2, 48000),
            gain: 1.0,
            ratio: 1.0,
            remap: false,
        },
        Case {
            src: spec(AudioFormat::U8, 1, 22050),
            dst: spec(AudioFormat::S16BE, 2, 44100),
            gain: 1.0,
            ratio: 1.0,
            remap: false,
        },
        Case {
            src: spec(AudioFormat::F32LE, 6, 48000),
            dst: spec(AudioFormat::S16LE, 2, 48000),
            gain: 0.5,
            ratio: 1.0,
            remap: false,
        },
        Case {
            src: spec(AudioFormat::S32BE, 2, 96000),
            dst: spec(AudioFormat::U8, 1, 8000),
            gain: 1.0,
            ratio: 1.0,
            remap: false,
        },
        Case {
            src: spec(AudioFormat::S16LE, 2, 44100),
            dst: spec(AudioFormat::S16LE, 2, 44100),
            gain: 1.0,
            ratio: 1.3,
            remap: false,
        },
        Case {
            src: spec(AudioFormat::S8, 4, 32000),
            dst: spec(AudioFormat::F32BE, 8, 44100),
            gain: 0.8,
            ratio: 0.7,
            remap: true,
        },
        Case {
            src: spec(AudioFormat::F32LE, 2, 48000),
            dst: spec(AudioFormat::F32LE, 2, 48000),
            gain: 1.0,
            ratio: 1.0,
            remap: true,
        },
        Case {
            src: spec(AudioFormat::S16LE, 1, 11025),
            dst: spec(AudioFormat::S32LE, 3, 44100),
            gain: 1.0,
            ratio: 1.0,
            remap: false,
        },
    ];
    let mut buf = vec![0u8; 1 << 16];
    let mut h = Hash::new();
    let mut rng = Rng(3);
    let got_u32 = |r: crate::error::Result<usize>| r.map_or(u32::MAX, |n| n as u32);
    for c in &cases {
        let s = stream_surviving_quit(Some(&c.src), Some(&c.dst));
        s.set_gain(c.gain).unwrap();
        s.set_frequency_ratio(c.ratio).unwrap();
        if c.remap {
            let map: Vec<i32> = (0..c.src.channels)
                .map(|i| c.src.channels - 1 - i)
                .collect();
            s.set_input_channel_map(Some(&map)).unwrap();
            let map: Vec<i32> = (0..c.dst.channels)
                .map(|i| (i + 1) % c.dst.channels)
                .collect();
            s.set_output_channel_map(Some(&map)).unwrap();
        }
        let fs = c.src.frame_size();
        for _ in 0..9 {
            let frames = 1 + (rng.rnd() % 700) as usize;
            fill(
                &mut rng,
                &mut buf,
                c.src.format,
                frames * c.src.channels as usize,
            );
            s.put_data(&buf[..frames * fs]).unwrap();
            let want = 1 + (rng.rnd() % 3000) as usize;
            let got = s.get_data(&mut buf[..want]);
            h.u32(got_u32(got.clone()));
            if let Ok(n) = got {
                h.bytes(&buf[..n]);
            }
        }
        s.flush();
        h.u32(s.available() as u32);
        loop {
            let got = s.get_data(&mut buf[..999]);
            h.u32(got_u32(got.clone()));
            match got {
                Ok(n) if n > 0 => h.bytes(&buf[..n]),
                _ => break,
            }
        }
    }
    assert_eq!(h.0, 0xa2eefa2c32f2de5c);
}

#[test]
fn mix_audio_matches_upstream() {
    let vols = [0.0f32, 0.3, 1.0, 1.5, 3.0];
    let mut dst = [0u8; 4096];
    let mut src = [0u8; 4096];
    let mut h = Hash::new();
    let mut rng = Rng(4);
    for &f in &FORMATS {
        for &v in &vols {
            let samples = 301;
            fill(&mut rng, &mut dst, f, samples);
            fill(&mut rng, &mut src, f, samples);
            let n = samples * f.bytesize() as usize;
            mix_audio(&mut dst[..n], &src[..n], f, v).unwrap();
            h.bytes(&dst[..n]);
        }
    }
    assert_eq!(h.0, 0x4e81dfc168d61856);
    assert_eq!(
        mix_audio(&mut dst, &src, AudioFormat(0x1234), 1.0)
            .unwrap_err()
            .message(),
        "SDL_MixAudio(): unknown audio format"
    );
}

/// The harness's `build_wav()`.
#[allow(clippy::too_many_arguments)]
fn build_wav(
    rng: &mut Rng,
    tag: u16,
    channels: u16,
    freq: u32,
    blockalign: u16,
    bits: u16,
    ext: &[u8],
    fact: Option<u32>,
    datalen: u32,
    adpcm: u8,
    datatype: u8,
) -> Vec<u8> {
    let mut w = Vec::new();
    let w16 = |w: &mut Vec<u8>, v: u16| w.extend_from_slice(&v.to_le_bytes());
    let w32 = |w: &mut Vec<u8>, v: u32| w.extend_from_slice(&v.to_le_bytes());
    w32(&mut w, 0x46464952);
    w32(&mut w, 0);
    w32(&mut w, 0x45564157);
    w32(&mut w, 0x20746D66);
    w32(
        &mut w,
        16 + if ext.is_empty() {
            0
        } else {
            2 + ext.len() as u32
        },
    );
    w16(&mut w, tag);
    w16(&mut w, channels);
    w32(&mut w, freq);
    w32(&mut w, freq * blockalign as u32);
    w16(&mut w, blockalign);
    w16(&mut w, bits);
    if !ext.is_empty() {
        w16(&mut w, ext.len() as u16);
        w.extend_from_slice(ext);
    }
    if let Some(factlen) = fact {
        w32(&mut w, 0x74636166);
        w32(&mut w, 4);
        w32(&mut w, factlen);
    }
    w32(&mut w, 0x5453494c);
    w32(&mut w, 3);
    w.extend_from_slice(b"abc\0"); // LIST with odd length + pad
    w32(&mut w, 0x61746164);
    w32(&mut w, datalen);
    for i in 0..datalen {
        let mut b = rng.byte();
        let o = i % blockalign as u32;
        if adpcm == 1 {
            // MS ADPCM: predictor bytes at the start of each block
            if o < channels as u32 {
                b = (rng.rnd() % 7) as u8; // (index 7 is out of range; upstream's off-by-one reads past the coefficients there)
            }
        } else if adpcm == 2 {
            // IMA: step index and reserved byte
            if o < channels as u32 * 4 {
                if o % 4 == 2 {
                    b = (rng.rnd() % 89) as u8;
                } else if o % 4 == 3 {
                    b = 0;
                }
            }
        }
        if datatype == 1 && i % 4 == 3 {
            b &= 0xBF; // keep floats finite
        }
        w.push(b);
    }
    let riff = (w.len() - 8) as u32;
    w[4..8].copy_from_slice(&riff.to_le_bytes());
    w
}

fn hash_wav(h: &mut Hash, wav: &[u8]) {
    let mut io = IoStream::from_const_mem(wav);
    match load_wav_io(&mut io) {
        Ok((spec, buf)) => {
            h.u32(1);
            h.u32(spec.format.0);
            h.u32(spec.channels as u32);
            h.u32(spec.freq as u32);
            h.u32(buf.len() as u32);
            h.bytes(&buf);
        }
        Err(e) => {
            h.u32(0);
            h.bytes(e.message().as_bytes());
        }
    }
}

#[test]
fn wave_loader_matches_upstream() {
    let _l = crate::test_support::test_lock();
    let coeffs: [i16; 14] = [
        256, 0, 512, -256, 0, 0, 192, 64, 240, 0, 460, -208, 392, -232,
    ];
    let mut msext = [0u8; 32];
    msext[2] = 7;
    for (i, c) in coeffs.iter().enumerate() {
        msext[4 + i * 2..6 + i * 2].copy_from_slice(&c.to_le_bytes());
    }
    let ext = [
        16u8, 0, 3, 0, 0, 0, 1, 0, 0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113,
    ];

    let mut h = Hash::new();
    let mut rng = Rng(5);
    let trunc_modes = [
        None,
        Some("verystrict"),
        Some("strict"),
        Some("dropframe"),
        Some("dropblock"),
    ];
    let fact_modes = [
        None,
        Some("truncate"),
        Some("strict"),
        Some("ignorezero"),
        Some("ignore"),
    ];
    let set = |name: &str, v: Option<&str>| match v {
        Some(v) => {
            crate::hints::set(name, v).unwrap();
        }
        None => {
            crate::hints::reset(name);
        }
    };
    for t in trunc_modes {
        for fm in fact_modes {
            set(crate::hints::WAVE_TRUNCATION, t);
            set(crate::hints::WAVE_FACT_CHUNK, fm);
            let r = &mut rng;
            let files = [
                build_wav(r, 1, 1, 22050, 1, 8, &[], None, 1001, 0, 0),
                build_wav(r, 1, 2, 44100, 4, 16, &[], None, 2003, 0, 0),
                build_wav(r, 1, 2, 48000, 6, 24, &[], None, 1800, 0, 0),
                build_wav(r, 1, 1, 8000, 4, 32, &[], Some(50), 1000, 0, 0),
                build_wav(r, 3, 2, 44100, 8, 32, &[], Some(0), 1600, 0, 1),
                build_wav(r, 6, 2, 8000, 2, 8, &[], Some(100), 999, 0, 0),
                build_wav(r, 7, 1, 8000, 1, 8, &[], Some(700), 777, 0, 0),
                build_wav(
                    r,
                    2,
                    2,
                    22050,
                    512,
                    4,
                    &msext,
                    Some(1500),
                    512 * 3 + 100,
                    1,
                    0,
                ),
                build_wav(r, 2, 1, 22050, 256, 4, &msext, None, 256 * 4 + 7, 1, 0),
                build_wav(
                    r,
                    0x11,
                    2,
                    44100,
                    1024,
                    4,
                    &[0, 0],
                    Some(2000),
                    1024 * 2 + 37,
                    2,
                    0,
                ),
                build_wav(
                    r,
                    0x11,
                    1,
                    11025,
                    256,
                    4,
                    &[0xF9, 0x01],
                    None,
                    256 * 3 + 130,
                    2,
                    0,
                ),
                build_wav(r, 0xFFFE, 2, 44100, 4, 16, &ext, None, 1000, 0, 0),
                build_wav(r, 0x55, 2, 44100, 4, 16, &[], None, 100, 0, 0),
                build_wav(r, 1, 0, 44100, 4, 16, &[], None, 100, 0, 0),
            ];
            // (the C harness builds and loads one file at a time; the
            // generator draws the same numbers either way)
            for f in &files {
                hash_wav(&mut h, f);
            }
        }
    }
    crate::hints::reset(crate::hints::WAVE_TRUNCATION);
    crate::hints::reset(crate::hints::WAVE_FACT_CHUNK);
    assert_eq!(h.0, 0x19b9203dc4f4c004);
}

#[test]
fn ms_adpcm_rejects_out_of_range_coefficient_index() {
    let _l = crate::test_support::test_lock();
    crate::hints::reset(crate::hints::WAVE_TRUNCATION);
    crate::hints::reset(crate::hints::WAVE_FACT_CHUNK);
    let coeffs: [i16; 14] = [
        256, 0, 512, -256, 0, 0, 192, 64, 240, 0, 460, -208, 392, -232,
    ];
    let mut msext = [0u8; 32];
    msext[2] = 7; // seven coefficient pairs: valid indices are 0..=6
    for (i, c) in coeffs.iter().enumerate() {
        msext[4 + i * 2..6 + i * 2].copy_from_slice(&c.to_le_bytes());
    }
    let mut rng = Rng(9);
    let mut wav = build_wav(&mut rng, 2, 1, 22050, 256, 4, &msext, None, 256, 1, 0);
    let data = wav.len() - 256;

    // The last valid index decodes...
    wav[data] = 6;
    assert!(load_wav_io(&mut IoStream::from_const_mem(&wav)).is_ok());

    // ...and an index equal to the coefficient count is rejected (upstream
    // lets it through and reads past the coefficient array).
    wav[data] = 7;
    assert_eq!(
        load_wav_io(&mut IoStream::from_const_mem(&wav))
            .unwrap_err()
            .message(),
        "Invalid MS ADPCM coefficient index in block header"
    );
}

#[test]
fn stream_api_behaviour() {
    let s16 = AudioSpec::new(AudioFormat::S16, 2, 48000);
    let s = stream_surviving_quit(Some(&s16), None);
    assert_eq!(
        s.format().unwrap_err().message(),
        "Stream has no destination format"
    );
    assert_eq!(
        s.put_data(&[0; 4]).unwrap_err().message(),
        "Stream has no destination format"
    );
    s.set_format(None, Some(&AudioSpec::new(AudioFormat::F32, 1, 48000)))
        .unwrap();
    assert_eq!(
        s.put_data(&[0; 3]).unwrap_err().message(),
        "Can't add partial sample frames"
    );
    assert!(s
        .set_format(Some(&AudioSpec::new(AudioFormat(7), 2, 1)), None)
        .is_err());
    assert_eq!(
        s.set_frequency_ratio(0.001).unwrap_err().message(),
        "Frequency ratio is too low"
    );
    assert!(s.set_gain(-1.0).is_err());
    assert_eq!(
        s.set_input_channel_map(Some(&[0])).unwrap_err().message(),
        "Wrong number of channels"
    );
    assert_eq!(
        s.set_input_channel_map(Some(&[0, 2]))
            .unwrap_err()
            .message(),
        "Invalid channel mapping"
    );
    s.set_input_channel_map(Some(&[1, 0])).unwrap();
    assert_eq!(s.input_channel_map(), Some(vec![1, 0]));
    s.set_input_channel_map(Some(&[0, 1])).unwrap(); // identity resets
    assert_eq!(s.input_channel_map(), None);
    assert_eq!(
        s.device().unwrap_err().message(),
        "Audio stream not bound to an audio device"
    );

    // Callbacks see the stream and can feed it.
    let fed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let f2 = fed.clone();
    s.set_get_callback(Some(
        move |stream: &AudioStream, additional: i32, total: i32| {
            assert!(total >= additional);
            if additional > 0 {
                stream.put_data(&vec![0x10u8; additional as usize]).unwrap();
                f2.fetch_add(additional as usize, std::sync::atomic::Ordering::SeqCst);
            }
        },
    ));
    let mut out = [0u8; 400];
    assert_eq!(s.get_data(&mut out).unwrap(), 400);
    assert_eq!(
        fed.load(std::sync::atomic::Ordering::SeqCst),
        400,
        "100 mono f32 frames = 100 stereo s16 frames"
    );

    // No-copy data is dropped once consumed (or when cleared).
    struct Tracked(Vec<u8>, std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl AsRef<[u8]> for Tracked {
        fn as_ref(&self) -> &[u8] {
            &self.0
        }
    }
    impl Drop for Tracked {
        fn drop(&mut self) {
            self.1.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    s.set_get_callback(None::<fn(&AudioStream, i32, i32)>);
    let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    s.put_data_no_copy(Tracked(vec![0; 64], dropped.clone()))
        .unwrap();
    assert_eq!(s.queued(), 64);
    s.clear();
    assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(s.queued(), 0);

    // Planar data: missing channels are silence.
    let left = [1u8, 0, 2, 0];
    s.put_planar_data(&[Some(&left)], 2).unwrap();
    assert_eq!(s.queued(), 8);

    // convert_audio_samples
    let out = convert_audio_samples(
        &AudioSpec::new(AudioFormat::U8, 1, 8000),
        &[0x80, 0xFF, 0x00],
        &AudioSpec::new(AudioFormat::S16LE, 1, 8000),
    )
    .unwrap();
    assert_eq!(out, [0x00, 0x00, 0x00, 0x7F, 0x00, 0x80]);
}

#[test]
fn dummy_driver_playback_and_recording() {
    let _l = crate::test_support::test_lock();
    assert!(current_audio_driver().is_none());
    assert!(playback_devices().is_err());
    let drivers: Vec<&str> = (0..num_audio_drivers())
        .map(|i| audio_driver(i).unwrap())
        .collect();
    assert_eq!(drivers, crate::audio::drivers::tests::EXPECTED_DRIVERS);
    assert!(audio_driver(drivers.len()).is_err());

    // Disk and dummy are demand-only: without a platform driver, nothing
    // initializes without the hint.
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        crate::hints::reset(crate::hints::AUDIO_DRIVER);
        assert_eq!(
            crate::init::init_subsystem(crate::init::InitFlags::AUDIO)
                .unwrap_err()
                .message(),
            "No available audio device"
        );
    }
    crate::hints::set(crate::hints::AUDIO_DRIVER, "nope").unwrap();
    assert_eq!(
        crate::init::init_subsystem(crate::init::InitFlags::AUDIO)
            .unwrap_err()
            .message(),
        "Audio target 'nope' not available"
    );
    crate::hints::set(crate::hints::AUDIO_DRIVER, "dummy").unwrap();
    crate::hints::set(crate::hints::AUDIO_DUMMY_TIMESCALE, "0.05").unwrap();
    crate::init::init_subsystem(crate::init::InitFlags::AUDIO).unwrap();
    assert_eq!(current_audio_driver(), Some("dummy"));

    let playback = playback_devices().unwrap();
    let recording = recording_devices().unwrap();
    assert_eq!((playback.len(), recording.len()), (1, 1));
    assert!(is_audio_device_physical(playback[0]) && is_audio_device_playback(playback[0]));
    assert!(!is_audio_device_playback(recording[0]));
    assert_eq!(
        audio_device_name(playback[0]).unwrap(),
        "System audio playback device"
    );
    assert_eq!(
        audio_device_name(AUDIO_DEVICE_DEFAULT_RECORDING).unwrap(),
        "System audio recording device"
    );
    let (spec, frames) = audio_device_format(AUDIO_DEVICE_DEFAULT_PLAYBACK).unwrap();
    assert_eq!(spec, AudioSpec::new(AudioFormat::S16, 2, 44100));
    assert_eq!(frames, 1024);

    // Playback: a callback-driven stream gets pulled by the device thread.
    let pulled = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let p2 = pulled.clone();
    let stream = open_audio_device_stream(
        AUDIO_DEVICE_DEFAULT_PLAYBACK,
        Some(&AudioSpec::new(AudioFormat::F32, 1, 22050)),
        Some(move |s: &AudioStream, additional: i32, _total: i32| {
            if additional > 0 {
                s.put_data(&vec![0u8; additional as usize]).unwrap();
                p2.fetch_add(additional as usize, std::sync::atomic::Ordering::SeqCst);
            }
        }),
    )
    .unwrap();
    assert!(stream.device_paused(), "simplified streams start paused");
    stream.resume_device().unwrap();
    let start = std::time::Instant::now();
    while pulled.load(std::sync::atomic::Ordering::SeqCst) == 0
        && start.elapsed() < std::time::Duration::from_secs(5)
    {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(pulled.load(std::sync::atomic::Ordering::SeqCst) > 0);
    let devid = stream.device().unwrap();
    assert!(!is_audio_device_physical(devid));
    assert_eq!(
        bind_audio_stream(devid, &stream).unwrap_err().message(),
        "Cannot change stream bindings on device opened with SDL_OpenAudioDeviceStream"
    );
    drop(stream); // closes the device

    // Recording: the dummy device records silence into bound streams.
    let dev = AudioDevice::open(AUDIO_DEVICE_DEFAULT_RECORDING, None).unwrap();
    assert_eq!(dev.gain().unwrap(), 1.0);
    dev.set_gain(0.5).unwrap();
    let rec = AudioStream::new(None, Some(&AudioSpec::new(AudioFormat::S16, 1, 44100))).unwrap();
    dev.bind(&rec).unwrap();
    assert_eq!(rec.device().unwrap(), dev.id());
    assert_eq!(
        rec.format().unwrap().0.format,
        AudioFormat::F32,
        "gain makes the device side float"
    );
    let start = std::time::Instant::now();
    while rec.available() == 0 && start.elapsed() < std::time::Duration::from_secs(5) {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let mut buf = [1u8; 64];
    let n = rec.get_data(&mut buf).unwrap();
    assert!(n > 0 && buf[..n].iter().all(|&b| b == 0), "silence");
    unbind_audio_stream(&rec);
    assert!(rec.device().is_err());
    drop(dev);

    // Postmix sees the final float mix.
    let dev = AudioDevice::open(AUDIO_DEVICE_DEFAULT_PLAYBACK, None).unwrap();
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let s2 = seen.clone();
    dev.set_postmix_callback(Some(move |spec: &AudioSpec, buf: &mut [f32]| {
        assert_eq!(spec.format, AudioFormat::F32);
        assert!(!buf.is_empty());
        s2.store(true, std::sync::atomic::Ordering::SeqCst);
    }))
    .unwrap();
    let start = std::time::Instant::now();
    while !seen.load(std::sync::atomic::Ordering::SeqCst)
        && start.elapsed() < std::time::Duration::from_secs(5)
    {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(seen.load(std::sync::atomic::Ordering::SeqCst));
    dev.pause().unwrap();
    assert!(dev.paused());
    drop(dev);

    // Device-added events were queued and are delivered by the pump.
    crate::events::pump();
    let mut added = 0;
    while let Some(e) = crate::events::poll() {
        if let crate::events::Event::AudioDevice(e) = e {
            if e.event_type == crate::events::EventType::AUDIO_DEVICE_ADDED {
                added += 1;
            }
        }
    }
    assert_eq!(added, 2);

    crate::init::quit_subsystem(crate::init::InitFlags::AUDIO);
    assert!(current_audio_driver().is_none());
    crate::hints::reset(crate::hints::AUDIO_DRIVER);
    crate::hints::reset(crate::hints::AUDIO_DUMMY_TIMESCALE);
}

#[test]
fn mix_path_swizzles_float_data_to_device_layout() {
    let _l = crate::test_support::test_lock();
    crate::hints::set(crate::hints::AUDIO_DRIVER, "dummy").unwrap();
    crate::hints::set(crate::hints::AUDIO_DUMMY_TIMESCALE, "0.05").unwrap();
    crate::init::init_subsystem(crate::init::InitFlags::AUDIO).unwrap();

    // The dummy device is S16 stereo, so mixing happens in a separate F32 buffer.
    let dev = AudioDevice::open(AUDIO_DEVICE_DEFAULT_PLAYBACK, None).unwrap();
    assert_eq!(dev.format().unwrap().0.format, AudioFormat::S16);
    super::device::set_device_chmap_for_test(dev.id(), Some(&[1, 0]));

    // A postmix callback forces the mixing path and shows the final F32 mix.
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<f32>::new()));
    let s2 = seen.clone();
    dev.set_postmix_callback(Some(move |_spec: &AudioSpec, buf: &mut [f32]| {
        let mut seen = s2.lock().unwrap();
        if seen.is_empty() && buf.iter().any(|&x| x != 0.0) {
            seen.extend_from_slice(buf);
        }
    }))
    .unwrap();

    let stream = AudioStream::new(Some(&AudioSpec::new(AudioFormat::F32, 2, 44100)), None).unwrap();
    let frame: Vec<u8> = [0.25f32, -0.5]
        .iter()
        .flat_map(|x| x.to_ne_bytes())
        .collect();
    dev.bind(&stream).unwrap();
    // Leave the stream's output in default order, so the device has to
    // swizzle it into its own layout.
    stream.set_output_channel_map(None).unwrap();
    stream.put_data(&frame.repeat(44100)).unwrap();

    let start = std::time::Instant::now();
    while seen.lock().unwrap().is_empty() && start.elapsed() < std::time::Duration::from_secs(5) {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let mix = std::mem::take(&mut *seen.lock().unwrap());
    assert!(!mix.is_empty(), "the stream was mixed");
    for f in mix.chunks_exact(2) {
        assert!(
            f == [0.0, 0.0] || f == [-0.5, 0.25],
            "frame {f:?} is not the swapped stereo pair"
        );
    }

    drop(dev);
    drop(stream);
    crate::init::quit_subsystem(crate::init::InitFlags::AUDIO);
    crate::hints::reset(crate::hints::AUDIO_DRIVER);
    crate::hints::reset(crate::hints::AUDIO_DUMMY_TIMESCALE);
}

#[test]
fn recording_swizzle_handles_float_data() {
    let _l = crate::test_support::test_lock();
    let tmp = crate::test_support::TempDir::new("diskswizzle");
    let input = tmp.path("in.raw");
    // S16 stereo frames, left 1000 and right -2000.
    let frame: Vec<u8> = [1000i16, -2000]
        .iter()
        .flat_map(|x| x.to_le_bytes())
        .collect();
    std::fs::write(&input, frame.repeat(64)).unwrap();

    crate::hints::set(crate::hints::AUDIO_DRIVER, "disk").unwrap();
    crate::hints::set(crate::hints::AUDIO_DISK_TIMESCALE, "0.05").unwrap();
    crate::hints::set(crate::hints::AUDIO_DISK_INPUT_FILE, &input).unwrap();
    crate::init::init_subsystem(crate::init::InitFlags::AUDIO).unwrap();

    let phys = recording_devices().unwrap()[0];
    super::device::set_device_default_channels_for_test(phys, 2);

    // Record four frames into a stream whose input channel map differs from
    // the device's, so the device swizzles each buffer into the stream's
    // layout before handing it over.
    let record = |gain: f32| -> Vec<(i16, i16)> {
        let dev = AudioDevice::open(phys, None).unwrap();
        let (spec, _) = dev.format().unwrap();
        assert_eq!((spec.format, spec.channels), (AudioFormat::S16LE, 2));
        dev.set_gain(gain).unwrap(); // below 1, the device side of the stream is F32
        let rec = AudioStream::new(None, Some(&spec)).unwrap();
        dev.bind(&rec).unwrap();
        rec.set_input_channel_map(Some(&[1, 0])).unwrap();
        let start = std::time::Instant::now();
        while rec.available() < 16 && start.elapsed() < std::time::Duration::from_secs(5) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let mut buf = [0u8; 16];
        assert_eq!(rec.get_data(&mut buf).unwrap(), 16);
        buf.chunks_exact(4)
            .map(|f| {
                (
                    i16::from_le_bytes([f[0], f[1]]),
                    i16::from_le_bytes([f[2], f[3]]),
                )
            })
            .collect()
    };

    // At unit gain the swizzle runs on the device's own S16 data.
    let full = record(1.0);
    assert!(full.iter().all(|&f| f == full[0]), "{full:?}");
    // At half gain it runs on F32 data, and must give the same layout.
    let half = record(0.5);
    for &(l, r) in &half {
        assert!(
            (l - full[0].0 / 2).abs() <= 1 && (r - full[0].1 / 2).abs() <= 1,
            "half-gain frame ({l}, {r}) does not match unit-gain {:?}",
            full[0]
        );
    }

    crate::init::quit_subsystem(crate::init::InitFlags::AUDIO);
    for h in [
        crate::hints::AUDIO_DRIVER,
        crate::hints::AUDIO_DISK_TIMESCALE,
        crate::hints::AUDIO_DISK_INPUT_FILE,
    ] {
        crate::hints::reset(h);
    }
}

#[test]
fn unbinding_a_simplified_stream_is_ignored() {
    let _l = crate::test_support::test_lock();
    crate::hints::set(crate::hints::AUDIO_DRIVER, "dummy").unwrap();
    crate::hints::set(crate::hints::AUDIO_DUMMY_TIMESCALE, "0.05").unwrap();
    crate::init::init_subsystem(crate::init::InitFlags::AUDIO).unwrap();

    let stream = open_audio_device_stream(
        AUDIO_DEVICE_DEFAULT_PLAYBACK,
        Some(&AudioSpec::new(AudioFormat::F32, 2, 44100)),
        None::<fn(&AudioStream, i32, i32)>,
    )
    .unwrap();
    let devid = stream.device().unwrap();

    // The stream stays bound to the device it was opened with...
    unbind_audio_stream(&stream);
    assert_eq!(stream.device().unwrap(), devid);
    unbind_audio_streams(&[&stream]);
    assert_eq!(stream.device().unwrap(), devid);
    assert!(audio_device_format(devid).is_ok());

    // ...so destroying it still closes that device.
    drop(stream);
    assert!(audio_device_format(devid).is_err());

    crate::init::quit_subsystem(crate::init::InitFlags::AUDIO);
    crate::hints::reset(crate::hints::AUDIO_DRIVER);
    crate::hints::reset(crate::hints::AUDIO_DUMMY_TIMESCALE);
}

#[test]
fn disk_driver_writes_and_reads_files() {
    let _l = crate::test_support::test_lock();
    let tmp = crate::test_support::TempDir::new("diskaudio");
    let out = tmp.path("out.raw");
    let input = tmp.path("in.raw");
    std::fs::write(&input, [7u8, 0, 9, 0]).unwrap(); // two S16 mono frames

    crate::hints::set(crate::hints::AUDIO_DRIVER, "disk").unwrap();
    crate::hints::set(crate::hints::AUDIO_DISK_TIMESCALE, "0.05").unwrap();
    crate::hints::set(crate::hints::AUDIO_DISK_OUTPUT_FILE, &out).unwrap();
    crate::hints::set(crate::hints::AUDIO_DISK_INPUT_FILE, &input).unwrap();
    crate::init::init_subsystem(crate::init::InitFlags::AUDIO).unwrap();
    assert_eq!(current_audio_driver(), Some("disk"));

    // Playback: the device writes whole buffers of its format to the file.
    let dev = AudioDevice::open(AUDIO_DEVICE_DEFAULT_PLAYBACK, None).unwrap();
    let (spec, frames) = dev.format().unwrap();
    let stream = AudioStream::new(Some(&spec), None).unwrap();
    dev.bind(&stream).unwrap();
    stream
        .put_data(&vec![0x22u8; spec.frame_size() * 64])
        .unwrap();
    let buffer_bytes = spec.frame_size() * frames as usize;
    let data_bytes = spec.frame_size() * 64;
    // (The device thread starts with the device and may write buffers of
    // silence before the stream is bound: wait for the buffer holding the
    // data.)
    let has_data = |w: &[u8]| {
        w.iter()
            .position(|&b| b != 0)
            .is_some_and(|at| w.len() >= at - at % buffer_bytes + buffer_bytes)
    };
    let start = std::time::Instant::now();
    while !has_data(&std::fs::read(&out).unwrap_or_default())
        && start.elapsed() < std::time::Duration::from_secs(5)
    {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    drop(dev);
    let written = std::fs::read(&out).unwrap();
    assert!(written.len() >= buffer_bytes && written.len().is_multiple_of(buffer_bytes));
    let at = written
        .iter()
        .position(|&b| b != 0)
        .expect("the queued data was written");
    assert!(
        at.is_multiple_of(buffer_bytes),
        "silence, in whole buffers, before the data"
    );
    assert!(
        written[at..at + data_bytes].iter().all(|&b| b == 0x22),
        "the queued data, in one piece"
    );
    assert!(
        written[at + data_bytes..].iter().all(|&b| b == 0),
        "then silence"
    );
    drop(stream);

    // Recording: the file's data, then silence once it runs out.
    let rec = open_audio_device_stream(
        AUDIO_DEVICE_DEFAULT_RECORDING,
        None,
        None::<fn(&AudioStream, i32, i32)>,
    )
    .unwrap();
    rec.resume_device().unwrap();
    let start = std::time::Instant::now();
    while rec.available() < 8 && start.elapsed() < std::time::Duration::from_secs(5) {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let mut buf = [0xFFu8; 8];
    assert_eq!(rec.get_data(&mut buf).unwrap(), 8);
    assert_eq!(buf, [7, 0, 9, 0, 0, 0, 0, 0]);
    drop(rec);

    crate::init::quit_subsystem(crate::init::InitFlags::AUDIO);
    for h in [
        crate::hints::AUDIO_DRIVER,
        crate::hints::AUDIO_DISK_TIMESCALE,
        crate::hints::AUDIO_DISK_OUTPUT_FILE,
        crate::hints::AUDIO_DISK_INPUT_FILE,
    ] {
        crate::hints::reset(h);
    }
}
