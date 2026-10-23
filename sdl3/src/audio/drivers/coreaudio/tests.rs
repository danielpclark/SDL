// Tests for the CoreAudio driver: the hand-declared layouts and constants
// (checked against the SDK with a C harness), the format, channel layout
// and buffer count arithmetic of OpenDevice/PrepareAudioQueue, and (when
// the Mac has audio devices) real playback and recording.

use super::*;
use crate::audio::{
    audio_device_name, current_audio_driver, playback_devices, recording_devices, AudioDevice,
    AudioStream, AUDIO_DEVICE_DEFAULT_PLAYBACK, AUDIO_DEVICE_DEFAULT_RECORDING,
};
use crate::init::InitFlags;

use std::mem::{offset_of, size_of};
use std::time::{Duration, Instant};

#[test]
fn layouts_match_the_sdk() {
    // sizeof/offsetof printed by a clang harness against the macOS SDK
    // (the same on arm64 and x86_64).
    assert_eq!(size_of::<AudioStreamBasicDescription>(), 40);
    assert_eq!(size_of::<AudioChannelLayout>(), 32);
    assert_eq!(size_of::<AudioObjectPropertyAddress>(), 12);
    assert_eq!(size_of::<AudioBuffer>(), 16);
    assert_eq!(size_of::<AudioBufferList>(), 24);
    assert_eq!(offset_of!(AudioBufferList, mBuffers), 8);
    assert_eq!(size_of::<AudioQueueBuffer>(), 56);
    assert_eq!(offset_of!(AudioQueueBuffer, mAudioData), 8);
    assert_eq!(offset_of!(AudioQueueBuffer, mAudioDataByteSize), 16);
    assert_eq!(offset_of!(AudioQueueBuffer, mUserData), 24);
    assert_eq!(offset_of!(AudioQueueBuffer, mPacketDescriptionCapacity), 32);
    assert_eq!(offset_of!(AudioQueueBuffer, mPacketDescriptions), 40);
    assert_eq!(offset_of!(AudioQueueBuffer, mPacketDescriptionCount), 48);
}

#[test]
fn constants_match_the_sdk() {
    // The tags in the order the harness printed them.
    assert_eq!(
        [
            kAudioChannelLayoutTag_Mono,
            kAudioChannelLayoutTag_Stereo,
            kAudioChannelLayoutTag_DVD_4,
            kAudioChannelLayoutTag_Quadraphonic,
            kAudioChannelLayoutTag_DVD_6,
            kAudioChannelLayoutTag_DVD_12,
            kAudioChannelLayoutTag_WAVE_6_1,
            kAudioChannelLayoutTag_MPEG_6_1_A,
            kAudioChannelLayoutTag_WAVE_7_1,
            kAudioChannelLayoutTag_MPEG_7_1_C,
        ],
        [
            6553601, 6619138, 8716291, 7077892, 8847365, 7929862, 12320775, 8192007, 12386312,
            8388616
        ]
    );
    assert_eq!(kAudioHardwareBadDeviceError, 560227702);
    assert_eq!(kAudioHardwarePropertyDevices, u32::from_be_bytes(*b"dev#"));
    assert_eq!(kAudioDevicePropertyDeviceUID, u32::from_be_bytes(*b"uid "));
}

#[test]
fn stream_descriptions() {
    let d = |f, ch, freq| stream_description(&AudioSpec::new(f, ch, freq)).unwrap();

    // CoreAudio takes every format SDL has, as it is.
    let (f, s) = d(AudioFormat::S16LE, 2, 48000);
    assert_eq!(f, AudioFormat::S16LE);
    assert_eq!(s.mFormatID, kAudioFormatLinearPCM);
    assert_eq!(
        s.mFormatFlags,
        kLinearPCMFormatFlagIsPacked | kLinearPCMFormatFlagIsSignedInteger
    );
    assert_eq!(
        (s.mSampleRate, s.mChannelsPerFrame, s.mBitsPerChannel),
        (48000.0, 2, 16)
    );
    assert_eq!(
        (s.mBytesPerFrame, s.mFramesPerPacket, s.mBytesPerPacket),
        (4, 1, 4)
    );

    let (f, s) = d(AudioFormat::F32BE, 6, 44100);
    assert_eq!(f, AudioFormat::F32BE);
    assert_eq!(
        s.mFormatFlags,
        kLinearPCMFormatFlagIsPacked
            | kLinearPCMFormatFlagIsBigEndian
            | kLinearPCMFormatFlagIsFloat
    );
    assert_eq!((s.mBitsPerChannel, s.mBytesPerFrame), (32, 24));

    // U8 is unsigned: neither float nor signed.
    let (f, s) = d(AudioFormat::U8, 1, 8000);
    assert_eq!(f, AudioFormat::U8);
    assert_eq!(s.mFormatFlags, kLinearPCMFormatFlagIsPacked);
    assert_eq!((s.mBitsPerChannel, s.mBytesPerFrame), (8, 1));

    let (_, s) = d(AudioFormat::S32BE, 8, 96000);
    assert_eq!(
        s.mFormatFlags,
        kLinearPCMFormatFlagIsPacked
            | kLinearPCMFormatFlagIsBigEndian
            | kLinearPCMFormatFlagIsSignedInteger
    );

    // Not a format at all: nothing to pick from.
    assert!(stream_description(&AudioSpec::new(AudioFormat::UNKNOWN, 2, 48000)).is_none());
}

#[test]
fn channel_layouts() {
    let tag = |ch, modern| channel_layout(ch, modern).unwrap();
    assert_eq!(tag(1, true), (kAudioChannelLayoutTag_Mono, None));
    assert_eq!(tag(2, true), (kAudioChannelLayoutTag_Stereo, None));
    assert_eq!(tag(3, true), (kAudioChannelLayoutTag_DVD_4, None));
    assert_eq!(tag(4, true), (kAudioChannelLayoutTag_Quadraphonic, None));
    assert_eq!(tag(5, true), (kAudioChannelLayoutTag_DVD_6, None));
    assert_eq!(tag(6, true), (kAudioChannelLayoutTag_DVD_12, None));
    assert_eq!(tag(7, true), (kAudioChannelLayoutTag_WAVE_6_1, None));
    assert_eq!(tag(8, true), (kAudioChannelLayoutTag_WAVE_7_1, None));
    // Before macOS 10.15: MPEG layouts, with SDL's channels swizzled.
    assert_eq!(
        tag(7, false),
        (
            kAudioChannelLayoutTag_MPEG_6_1_A,
            Some(&[0, 1, 2, 3, 6, 4, 5][..])
        )
    );
    assert_eq!(
        tag(8, false),
        (
            kAudioChannelLayoutTag_MPEG_7_1_C,
            Some(&[0, 1, 2, 3, 6, 7, 4, 5][..])
        )
    );
    assert_eq!(
        channel_layout(9, true).unwrap_err().message(),
        "Unsupported audio channels"
    );
    assert!(channel_layout(0, true).is_err());
    // Every macOS this builds for has the WAVE layouts.
    assert!(crate::core::apple::foundation::available!(macos(10, 15)));
}

#[test]
fn buffer_counts() {
    // Three buffers, unless one is shorter than 15ms: then enough for
    // twice that.
    assert_eq!(num_audio_buffers(1024, 48000), 3);
    assert_eq!(num_audio_buffers(720, 48000), 3); // exactly 15ms
    assert_eq!(num_audio_buffers(512, 48000), 4); // 10.67ms: ceil(1.41) * 2
    assert_eq!(num_audio_buffers(64, 48000), 24); // 1.33ms: ceil(11.25) * 2
    assert_eq!(num_audio_buffers(1, 44100), 1324);
}

#[test]
fn check_result_messages() {
    assert!(check_result(noErr, "x").is_ok());
    assert_eq!(
        check_result(-50, "AudioQueueStart").unwrap_err().message(),
        "CoreAudio error (AudioQueueStart): -50"
    );
}

/// Initialize audio with the CoreAudio driver, or skip.
fn init_coreaudio() -> bool {
    crate::hints::set(crate::hints::AUDIO_DRIVER, "coreaudio").unwrap();
    let r = crate::init::init_subsystem(InitFlags::AUDIO);
    crate::hints::reset(crate::hints::AUDIO_DRIVER);
    if let Err(e) = r {
        assert!(current_audio_driver().is_none());
        crate::test_support::skip(
            "coreaudio",
            format_args!("CoreAudio didn't initialize: {}", e.message()),
        );
        return false;
    }
    assert_eq!(current_audio_driver(), Some("coreaudio"));
    true
}

#[test]
fn devices_match_the_hal() {
    let _l = crate::test_support::test_lock();
    let _cleanup = crate::audio::drivers::tests::QuitAudioOnDrop;
    if !init_coreaudio() {
        return;
    }
    let playback = playback_devices().unwrap();
    let recording = recording_devices().unwrap();
    eprintln!(
        "note: CoreAudio devices: {} playback, {} recording",
        playback.len(),
        recording.len()
    );

    // Every HAL device with output (input) streams is a playback
    // (recording) device, once, by its trimmed name.
    let devs = {
        let size = property_data_size(kAudioObjectSystemObject, &DEVLIST_ADDRESS).unwrap();
        let mut devs = vec![0 as AudioDeviceID; size as usize / 4];
        let mut size = size;
        // SAFETY: an out array of `size` bytes.
        let r = unsafe {
            AudioObjectGetPropertyData(
                kAudioObjectSystemObject,
                &DEVLIST_ADDRESS,
                0,
                ptr::null(),
                &mut size,
                devs.as_mut_ptr() as *mut c_void,
            )
        };
        assert_eq!(r, noErr);
        devs.truncate(size as usize / 4);
        devs
    };
    for (recording, ids) in [(false, &playback), (true, &recording)] {
        let mut expected = 0;
        for &dev in &devs {
            if find_core_audio_device_by_handle(dev, recording).is_some() {
                expected += 1;
            }
        }
        assert_eq!(ids.len(), expected, "recording={recording}");
        for &id in ids.iter() {
            let name = audio_device_name(id).unwrap();
            assert!(!name.is_empty() && !name.ends_with(' '), "{name:?}");
            let (spec, frames) = crate::audio::audio_device_format(id).unwrap();
            assert!(
                spec.channels > 0 && spec.freq > 0 && frames > 0,
                "{name}: {spec:?}"
            );
            eprintln!(
                "note:   {name} | fmt={} ch={} freq={} frames={frames}",
                spec.format.0, spec.channels, spec.freq
            );
        }
    }

    // The defaults are the HAL's defaults.
    for (recording, addr, default_id) in [
        (
            false,
            &DEFAULT_PLAYBACK_DEVICE_ADDRESS,
            AUDIO_DEVICE_DEFAULT_PLAYBACK,
        ),
        (
            true,
            &DEFAULT_RECORDING_DEVICE_ADDRESS,
            AUDIO_DEVICE_DEFAULT_RECORDING,
        ),
    ] {
        if let Some(devid) = system_default_device(addr) {
            if let Some(dev) = find_core_audio_device_by_handle(devid, recording) {
                assert_eq!(audio_device_name(default_id).unwrap(), dev.name);
            }
        }
    }
}

#[test]
fn playback_and_recording_when_there_are_devices() {
    let _l = crate::test_support::test_lock();
    let _cleanup = crate::audio::drivers::tests::QuitAudioOnDrop;
    if !init_coreaudio() {
        return;
    }

    match AudioDevice::open(AUDIO_DEVICE_DEFAULT_PLAYBACK, None) {
        Err(e) => crate::test_support::skip(
            "coreaudio",
            format_args!("no CoreAudio playback: {}", e.message()),
        ),
        Ok(dev) => {
            let (spec, frames) = dev.format().unwrap();
            assert!(spec.channels > 0 && spec.freq > 0 && frames > 0);
            let stream = AudioStream::new(Some(&spec), None).unwrap();
            dev.bind(&stream).unwrap();
            stream
                .put_data(&vec![0u8; spec.frame_size() * frames as usize * 4])
                .unwrap();
            // The queue's callbacks drain the stream as the device plays.
            let start = Instant::now();
            while stream.queued() > 0 && start.elapsed() < Duration::from_secs(10) {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(stream.queued(), 0, "the device played the silence");
            // Opening at a higher spec reopens the device (a new queue and
            // thread) and keeps playing.
            let better =
                AudioSpec::new(AudioFormat::F32, spec.channels.max(2), spec.freq.max(48000));
            let dev2 = AudioDevice::open(AUDIO_DEVICE_DEFAULT_PLAYBACK, Some(&better)).unwrap();
            stream
                .put_data(&vec![0u8; spec.frame_size() * frames as usize * 2])
                .unwrap();
            let start = Instant::now();
            while stream.queued() > 0 && start.elapsed() < Duration::from_secs(10) {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(stream.queued(), 0, "the reopened device played");
            drop(dev2);
            drop(dev);
        }
    }

    match AudioDevice::open(AUDIO_DEVICE_DEFAULT_RECORDING, None) {
        Err(e) => crate::test_support::skip(
            "coreaudio",
            format_args!("no CoreAudio recording: {}", e.message()),
        ),
        Ok(dev) => {
            let (spec, _) = dev.format().unwrap();
            let stream = AudioStream::new(None, Some(&spec)).unwrap();
            dev.bind(&stream).unwrap();
            let start = Instant::now();
            while stream.available() == 0 && start.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(5));
            }
            // (Without microphone permission the queue records silence, but
            // it still records.)
            eprintln!("note: CoreAudio recorded {} bytes", stream.available());
            assert!(stream.available() > 0, "the recording queue delivered data");
            drop(dev);
        }
    }

    // Open and close in quick succession: the queue thread is joined each
    // time, and nothing is left behind.
    for _ in 0..5 {
        if let Ok(dev) = AudioDevice::open(AUDIO_DEVICE_DEFAULT_PLAYBACK, None) {
            drop(dev);
        }
    }
}

/// Hardware (docs/HARDWARE_TESTING.md): the default playback device plays
/// half a second of a quiet 440 Hz tone, then the default recording device
/// records half a second; both must work. Without microphone permission
/// for the terminal, macOS hands the recording queue silence: that skips.
#[test]
#[ignore = "hardware: plays a quiet tone and records from the default microphone"]
fn hardware_default_playback_and_recording() {
    let _l = crate::test_support::test_lock();
    let _cleanup = crate::audio::drivers::tests::QuitAudioOnDrop;
    if !init_coreaudio() {
        return;
    }
    for (what, ids) in [
        ("playback", playback_devices().unwrap()),
        ("recording", recording_devices().unwrap()),
    ] {
        println!("CoreAudio {what} devices: {}", ids.len());
        for id in ids {
            println!("  {id}: {:?}", audio_device_name(id).unwrap());
        }
    }

    // Playback: the queue's callbacks pull the tone through the stream.
    let dev = match AudioDevice::open(AUDIO_DEVICE_DEFAULT_PLAYBACK, None) {
        Ok(dev) => dev,
        Err(e) => {
            crate::test_support::skip(
                "coreaudio",
                format_args!("no default playback device: {}", e.message()),
            );
            return;
        }
    };
    let (spec, frames) = dev.format().unwrap();
    println!(
        "playback {:?}: {:?}, {} channels, {} Hz, {frames} sample frames per buffer",
        audio_device_name(AUDIO_DEVICE_DEFAULT_PLAYBACK).unwrap(),
        spec.format,
        spec.channels,
        spec.freq
    );
    let tone_spec = AudioSpec::new(AudioFormat::F32, 1, 48000);
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
    assert_eq!(stream.queued(), 0, "the device played the tone");
    drop(dev);
    drop(stream);

    // Recording: half a second arrives in about that time.
    let dev = match AudioDevice::open(AUDIO_DEVICE_DEFAULT_RECORDING, None) {
        Ok(dev) => dev,
        Err(e) => {
            crate::test_support::skip(
                "coreaudio",
                format_args!("no default recording device: {}", e.message()),
            );
            return;
        }
    };
    let (spec, frames) = dev.format().unwrap();
    println!(
        "recording {:?}: {:?}, {} channels, {} Hz, {frames} sample frames per buffer",
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
    assert_eq!(got, want, "the device recorded half a second");
    if peak == 0.0 {
        crate::test_support::skip(
            "coreaudio",
            "the recording is digital silence: no microphone permission for this terminal (System Settings > Privacy & Security > Microphone)?",
        );
    }
    drop(dev);
}
