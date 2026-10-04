// Tests for the WASAPI driver: the hand-declared COM layouts, the format
// and period arithmetic of PrepDevice, and (when the system, or Wine, has
// an audio endpoint) a real playback device.

use super::*;
use crate::audio::{
    current_audio_driver, playback_devices, recording_devices, AudioDevice, AudioStream,
    AUDIO_DEVICE_DEFAULT_PLAYBACK, AUDIO_DEVICE_DEFAULT_RECORDING,
};
use crate::init::InitFlags;

use std::mem::size_of;
use std::time::{Duration, Instant};

/// The number of function pointers in a vtable.
fn methods<V>() -> usize {
    size_of::<V>() / size_of::<usize>()
}

#[test]
fn com_layouts_match_the_sdk() {
    // IUnknown (3) + the methods each interface adds, as in audioclient.h.
    assert_eq!(methods::<IUnknownVtbl>(), 3);
    assert_eq!(methods::<IAudioClientVtbl>(), 3 + 12);
    assert_eq!(methods::<IAudioClient2Vtbl>(), 3 + 12 + 3);
    assert_eq!(methods::<IAudioClient3Vtbl>(), 3 + 12 + 3 + 3);
    assert_eq!(methods::<IAudioRenderClientVtbl>(), 3 + 2);
    assert_eq!(methods::<IAudioCaptureClientVtbl>(), 3 + 3);
    assert_eq!(
        size_of::<SdlAudioClientProperties>(),
        SIZEOF_AUDIOCLIENTPROPERTIES as usize
    );
    assert_eq!(size_of::<WAVEFORMATEX>(), 18);
}

/// `GUID` as the registry spells it.
fn guid_string(g: &GUID) -> String {
    format!(
        "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        g.data1,
        g.data2,
        g.data3,
        g.data4[0],
        g.data4[1],
        g.data4[2],
        g.data4[3],
        g.data4[4],
        g.data4[5],
        g.data4[6],
        g.data4[7]
    )
}

#[test]
fn interface_ids_match_the_sdk() {
    // The `MIDL_INTERFACE()` strings in audioclient.h.
    assert_eq!(
        guid_string(&SDL_IID_IAUDIOCLIENT),
        "1CB9AD4C-DBFA-4C32-B178-C2F568A703B2"
    );
    assert_eq!(
        guid_string(&SDL_IID_IAUDIOCLIENT2),
        "726778CD-F60A-4EDA-82DE-E47610CD78AA"
    );
    assert_eq!(
        guid_string(&SDL_IID_IAUDIOCLIENT3),
        "7ED4EE07-8E67-4CD4-8C1A-2B7A5987AD42"
    );
    assert_eq!(
        guid_string(&SDL_IID_IAUDIORENDERCLIENT),
        "F294ACFC-3146-4483-A7BF-ADDCA7C260E2"
    );
    assert_eq!(
        guid_string(&SDL_IID_IAUDIOCAPTURECLIENT),
        "C8ADBD64-E71E-48A0-A4DE-185C395CD317"
    );
}

#[test]
fn format_negotiation_picks_the_mix_format() {
    // Shared mode converts nothing for us (without AUTOCONVERTPCM), so
    // the device's format is whichever of the requested format's closest
    // relatives the mix format is.
    assert_eq!(
        choose_format(AudioFormat::S16, Some(AudioFormat::F32)),
        Some(AudioFormat::F32)
    );
    assert_eq!(
        choose_format(AudioFormat::F32, Some(AudioFormat::F32)),
        Some(AudioFormat::F32)
    );
    assert_eq!(
        choose_format(AudioFormat::U8, Some(AudioFormat::S32)),
        Some(AudioFormat::S32)
    );
    assert_eq!(choose_format(AudioFormat::S16, None), None);
    // (A big-endian mix format is never produced, but it's still a relative.)
    assert_eq!(
        choose_format(AudioFormat::S16LE, Some(AudioFormat::S16BE)),
        Some(AudioFormat::S16BE)
    );
}

#[test]
fn period_arithmetic() {
    // 10ms (in 100ns units) at 48kHz is 480 frames; partial frames round up.
    assert_eq!(period_sample_frames(100_000, 48000), 480);
    assert_eq!(period_sample_frames(100_000, 44100), 441);
    assert_eq!(period_sample_frames(30_000, 44100), 133);
    assert_eq!(period_sample_frames(0, 48000), 0);

    // IAudioClient3 periods are whole multiples of the fundamental period,
    // clamped to what the engine allows.
    assert_eq!(shared_mode_period_in_frames(1024, 480, 480, 4800), 960);
    assert_eq!(shared_mode_period_in_frames(1200, 480, 480, 4800), 1440);
    assert_eq!(shared_mode_period_in_frames(100, 480, 480, 4800), 480);
    assert_eq!(shared_mode_period_in_frames(100_000, 480, 480, 4800), 4800);
    assert_eq!(shared_mode_period_in_frames(1024, 128, 256, 512), 512);
    // SDL_clamp() with an empty range gives the low bound below it, and
    // the high bound otherwise.
    assert_eq!(shared_mode_period_in_frames(64, 64, 512, 256), 512);
    assert_eq!(shared_mode_period_in_frames(1024, 64, 512, 256), 256);
}

#[test]
fn wave_format_rate() {
    let mut wf = WAVEFORMATEX {
        wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
        nChannels: 2,
        nSamplesPerSec: 48000,
        nAvgBytesPerSec: 48000 * 8,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    // SAFETY: `wf` is a live, writable WAVEFORMATEX.
    unsafe { set_wave_format_rate(&mut wf, 22050) };
    let (rate, bytes, align) = (wf.nSamplesPerSec, wf.nAvgBytesPerSec, wf.nBlockAlign);
    assert_eq!((rate, bytes, align), (22050, 22050 * 8, 8));
}

#[test]
fn management_thread_runs_tasks_in_order() {
    let _l = crate::test_support::test_lock();
    if init_management_thread().is_err() {
        // (COM or IMMDevice unavailable: WASAPI_Init fails the same way.)
        crate::test_support::skip("wasapi", "the WASAPI management thread didn't start");
        return;
    }
    let order = Arc::new(Mutex::new(Vec::new()));
    for i in 0..3 {
        let order = order.clone();
        proxy_no_wait(move || {
            order.lock().unwrap().push(i);
            Ok(())
        });
    }
    let o = order.clone();
    assert!(proxy_and_wait(move || {
        o.lock().unwrap().push(3);
        Ok(())
    })
    .is_ok());
    assert!(proxy_and_wait(|| Err(Error::new("task failed"))).is_err());
    assert_eq!(*order.lock().unwrap(), [0, 1, 2, 3]);
    deinit_management_thread();
    assert!(pending_tasks().is_empty());
    assert!(MANAGEMENT.thread.lock().unwrap().is_none());
}

#[test]
fn playback_and_recording_when_there_is_an_endpoint() {
    let _l = crate::test_support::test_lock();
    let _cleanup = crate::audio::drivers::tests::QuitAudioOnDrop;
    crate::hints::set(crate::hints::AUDIO_DRIVER, "wasapi").unwrap();
    if let Err(e) = crate::init::init_subsystem(InitFlags::AUDIO) {
        // (Wine without an audio driver, or a server without one.)
        assert!(current_audio_driver().is_none());
        crate::test_support::skip(
            "wasapi",
            format_args!("WASAPI didn't initialize: {}", e.message()),
        );
        return;
    }
    assert_eq!(current_audio_driver(), Some("wasapi"));
    let playback = playback_devices().unwrap();
    let recording = recording_devices().unwrap();
    eprintln!(
        "note: WASAPI endpoints: {} playback, {} recording",
        playback.len(),
        recording.len()
    );

    match AudioDevice::open(AUDIO_DEVICE_DEFAULT_PLAYBACK, None) {
        Err(e) => crate::test_support::skip(
            "wasapi",
            format_args!("no WASAPI playback: {}", e.message()),
        ),
        Ok(dev) => {
            let (spec, frames) = dev.format().unwrap();
            assert!(spec.channels > 0 && spec.freq > 0 && frames > 0);
            let stream = AudioStream::new(Some(&spec), None).unwrap();
            dev.bind(&stream).unwrap();
            stream
                .put_data(&vec![0u8; spec.frame_size() * frames as usize * 4])
                .unwrap();
            // The device thread drains the stream as the endpoint plays.
            let start = Instant::now();
            while stream.queued() > 0 && start.elapsed() < Duration::from_secs(10) {
                std::thread::sleep(Duration::from_millis(5));
            }
            eprintln!("note: WASAPI left {} bytes queued", stream.queued());
            drop(dev);
        }
    }

    match AudioDevice::open(AUDIO_DEVICE_DEFAULT_RECORDING, None) {
        Err(e) => crate::test_support::skip(
            "wasapi",
            format_args!("no WASAPI recording: {}", e.message()),
        ),
        Ok(dev) => {
            let (spec, _) = dev.format().unwrap();
            let stream = AudioStream::new(None, Some(&spec)).unwrap();
            dev.bind(&stream).unwrap();
            let start = Instant::now();
            while stream.available() == 0 && start.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(5));
            }
            eprintln!("note: WASAPI recorded {} bytes", stream.available());
            drop(dev);
        }
    }
}

/// Hardware (docs/HARDWARE_TESTING.md): the default playback endpoint
/// plays half a second of a quiet 440 Hz tone, then the default recording
/// endpoint records half a second; both must work.
#[test]
#[ignore = "hardware: plays a quiet tone and records from the default microphone"]
fn hardware_default_playback_and_recording() {
    use crate::audio::{audio_device_name, AudioSpec};
    let _l = crate::test_support::test_lock();
    let _cleanup = crate::audio::drivers::tests::QuitAudioOnDrop;
    crate::hints::set(crate::hints::AUDIO_DRIVER, "wasapi").unwrap();
    let r = crate::init::init_subsystem(InitFlags::AUDIO);
    crate::hints::reset(crate::hints::AUDIO_DRIVER);
    if let Err(e) = r {
        crate::test_support::skip(
            "wasapi",
            format_args!("WASAPI didn't initialize: {}", e.message()),
        );
        return;
    }
    for (what, ids) in [
        ("playback", playback_devices().unwrap()),
        ("recording", recording_devices().unwrap()),
    ] {
        println!("WASAPI {what} endpoints: {}", ids.len());
        for id in ids {
            println!("  {id}: {:?}", audio_device_name(id).unwrap());
        }
    }

    // Playback: the device thread pulls the tone through the stream.
    let dev = match AudioDevice::open(AUDIO_DEVICE_DEFAULT_PLAYBACK, None) {
        Ok(dev) => dev,
        Err(e) => {
            crate::test_support::skip(
                "wasapi",
                format_args!("no default playback endpoint: {}", e.message()),
            );
            return;
        }
    };
    let (spec, frames) = dev.format().unwrap();
    println!(
        "playback {:?}: {:?}, {} channels, {} Hz, {frames} sample frames per period",
        audio_device_name(AUDIO_DEVICE_DEFAULT_PLAYBACK).unwrap(),
        spec.format,
        spec.channels,
        spec.freq
    );
    let tone_spec = AudioSpec::new(crate::audio::AudioFormat::F32, 1, 48000);
    let tone: Vec<u8> = (0..24000)
        .flat_map(|i| {
            let t = i as f32 / 48000.0;
            (0.05 * (t * 440.0 * std::f32::consts::TAU).sin()).to_ne_bytes()
        })
        .collect();
    let stream = AudioStream::new(Some(&tone_spec), None).unwrap();
    dev.bind(&stream).unwrap();
    stream.put_data(&tone).unwrap();
    stream.flush();
    let start = Instant::now();
    while stream.queued() > 0 && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(5));
    }
    println!("played {} bytes in {:?}", tone.len(), start.elapsed());
    assert_eq!(stream.queued(), 0, "the endpoint played the tone");
    drop(dev);
    drop(stream);

    // Recording: half a second arrives in about that time.
    let dev = match AudioDevice::open(AUDIO_DEVICE_DEFAULT_RECORDING, None) {
        Ok(dev) => dev,
        Err(e) => {
            crate::test_support::skip(
                "wasapi",
                format_args!("no default recording endpoint: {}", e.message()),
            );
            return;
        }
    };
    let (spec, frames) = dev.format().unwrap();
    println!(
        "recording {:?}: {:?}, {} channels, {} Hz, {frames} sample frames per period",
        audio_device_name(AUDIO_DEVICE_DEFAULT_RECORDING).unwrap(),
        spec.format,
        spec.channels,
        spec.freq
    );
    let stream = AudioStream::new(None, Some(&tone_spec)).unwrap();
    dev.bind(&stream).unwrap();
    let want = 24000 * 4;
    let start = Instant::now();
    while (stream.available() as usize) < want && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut buf = vec![0u8; want];
    let got = stream.get_data(&mut buf).unwrap();
    let peak = buf[..got]
        .chunks_exact(4)
        .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]).abs())
        .fold(0.0f32, f32::max);
    println!(
        "recorded {got} bytes in {:?}, peak level {peak:.4}",
        start.elapsed()
    );
    assert_eq!(got, want, "the endpoint recorded half a second");
    drop(dev);
}
