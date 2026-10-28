// Tests for the CoreAudio driver: the hand-declared layouts and constants
// (checked against the SDK with a C harness), the format, channel layout
// and buffer count arithmetic of OpenDevice/PrepareAudioQueue, and (when
// the Mac has audio devices) real playback and recording. On the UIKit
// platforms, the default devices, the audio session's categories and what
// opening and closing devices does to the session (under `simctl spawn`
// on the iOS Simulator; tests/uikit_audio.rs runs as an app).

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
    #[cfg(target_os = "macos")]
    {
        assert_eq!(size_of::<AudioObjectPropertyAddress>(), 12);
        assert_eq!(size_of::<AudioBuffer>(), 16);
        assert_eq!(size_of::<AudioBufferList>(), 24);
        assert_eq!(offset_of!(AudioBufferList, mBuffers), 8);
    }
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
    assert_eq!(kAudioFormatLinearPCM, u32::from_be_bytes(*b"lpcm"));
    #[cfg(target_os = "macos")]
    {
        assert_eq!(kAudioHardwareBadDeviceError, 560227702);
        assert_eq!(kAudioHardwarePropertyDevices, u32::from_be_bytes(*b"dev#"));
        assert_eq!(kAudioDevicePropertyDeviceUID, u32::from_be_bytes(*b"uid "));
    }
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
    // Every system this builds for has the WAVE layouts.
    assert!(crate::core::apple::foundation::available!(
        macos(10, 15),
        ios(13, 0),
        tvos(13, 0)
    ));
}

#[test]
fn buffer_counts() {
    // Three buffers, unless one is shorter than 15ms: then enough for
    // twice that.
    assert_eq!(num_audio_buffers(1024, 48000, 15.0), 3);
    assert_eq!(num_audio_buffers(720, 48000, 15.0), 3); // exactly 15ms
    assert_eq!(num_audio_buffers(512, 48000, 15.0), 4); // 10.67ms: ceil(1.41) * 2
    assert_eq!(num_audio_buffers(64, 48000, 15.0), 24); // 1.33ms: ceil(11.25) * 2
    assert_eq!(num_audio_buffers(1, 44100, 15.0), 1324);
    // Older iOS hardware: 40ms.
    assert_eq!(num_audio_buffers(1024, 48000, 40.0), 4); // 21.33ms: ceil(1.88) * 2
    assert_eq!(num_audio_buffers(1920, 48000, 40.0), 3);
    // 15ms on every system this builds for (iOS 7.1's Foundation is
    // 1047.25).
    assert_eq!(minimum_audio_buffer_time_ms(), 15.0);
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

#[cfg(target_os = "macos")]
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
            // (all of it, through the resampler too, should the device's
            // rate have changed: the UIKit platforms' device takes on the
            // session's rate, the one asked for)
            stream.flush();
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

/// The categories and options `UpdateAudioSession()` picks (the
/// session's read-back ones are in tests/uikit_audio.rs, where setting
/// `DuckOthers` alone reads back with `MixWithOthers`; tvOS has no
/// `DefaultToSpeaker`).
#[cfg(not(target_os = "macos"))]
#[test]
fn session_categories() {
    use super::session::{session_category, Category::*};
    let c = session_category;
    // (DefaultToSpeaker with playing and recording, but on tvOS)
    const SPEAKER: usize = if cfg!(target_os = "tvos") { 0 } else { 0x8 };
    // Playback, mixing with others, unless the hint says otherwise
    assert_eq!(c(None, false, 1, 0, true), Some((Playback, 0x1)));
    assert_eq!(c(None, true, 1, 0, true), Some((Playback, 0x3)));
    assert_eq!(c(None, false, 0, 0, true), Some((Playback, 0x1)));
    for hint in [
        "ambient",
        "AVAudioSessionCategoryAmbient",
        "AMBIENT",
        "aMbIeNt",
    ] {
        assert_eq!(
            c(Some(hint), false, 1, 0, true),
            Some((Ambient, 0x1)),
            "{hint}"
        );
        assert_eq!(
            c(Some(hint), true, 1, 0, true),
            Some((Ambient, 0x1)),
            "{hint}"
        );
    }
    // (no short name for the solo ambient category)
    assert_eq!(
        c(Some("AVAudioSessionCategorySoloAmbient"), true, 1, 0, true),
        Some((SoloAmbient, 0))
    );
    assert_eq!(
        c(Some("soloambient"), false, 1, 0, true),
        Some((Playback, 0x1))
    );
    for hint in ["playback", "PlayBack", "AVAudioSessionCategoryPlayback"] {
        assert_eq!(
            c(Some(hint), false, 1, 0, true),
            Some((Playback, 0)),
            "{hint}"
        );
        assert_eq!(
            c(Some(hint), true, 0, 1, true),
            Some((Playback, 0x2)),
            "{hint}"
        );
    }
    for hint in ["playandrecord", "AVAudioSessionCategoryPlayAndRecord"] {
        assert_eq!(
            c(Some(hint), false, 1, 0, true),
            Some((PlayAndRecord, 0x65 | SPEAKER))
        );
        assert_eq!(
            c(Some(hint), true, 1, 1, true),
            Some((PlayAndRecord, 0x67 | SPEAKER))
        );
        // (the retry after ResourceNotAvailable)
        assert_eq!(c(Some(hint), false, 1, 1, false), Some((Playback, 0x1)));
    }
    // Values it doesn't know are the default.
    for hint in ["record", "bogus", "", "playback "] {
        assert_eq!(
            c(Some(hint), true, 1, 0, true),
            Some((Playback, 0x3)),
            "{hint:?}"
        );
        assert_eq!(
            c(Some(hint), false, 0, 1, true),
            Some((Playback, 0x1)),
            "{hint:?}"
        );
    }
    // Without the hint, what is open: recording, both.
    assert_eq!(c(None, true, 0, 1, true), Some((Record, 0x5)));
    assert_eq!(
        c(None, false, 2, 1, true),
        Some((PlayAndRecord, 0x65 | SPEAKER))
    );
    assert_eq!(
        c(None, true, 1, 2, true),
        Some((PlayAndRecord, 0x67 | SPEAKER))
    );
    assert_eq!(c(None, false, 1, 1, false), None);
    assert_eq!(c(None, false, 0, 1, false), Some((Record, 0x5)));
    // On every system this builds for, Foundation is newer than iOS 7.1's.
    assert!(!super::session::older_ios_hardware());
}

/// On the UIKit platforms the driver has the default devices only, at the
/// core's default formats until they open.
#[cfg(not(target_os = "macos"))]
#[test]
fn default_devices_only() {
    let _l = crate::test_support::test_lock();
    let _cleanup = crate::audio::drivers::tests::QuitAudioOnDrop;
    if !init_coreaudio() {
        return;
    }
    let playback = playback_devices().unwrap();
    let recording = recording_devices().unwrap();
    assert_eq!((playback.len(), recording.len()), (1, 1));
    assert_eq!(
        audio_device_name(playback[0]).unwrap(),
        "System audio playback device"
    );
    assert_eq!(
        audio_device_name(recording[0]).unwrap(),
        "System audio recording device"
    );
    assert_eq!(
        audio_device_name(AUDIO_DEVICE_DEFAULT_PLAYBACK).unwrap(),
        "System audio playback device"
    );
    assert_eq!(
        audio_device_name(AUDIO_DEVICE_DEFAULT_RECORDING).unwrap(),
        "System audio recording device"
    );
    assert_eq!(
        crate::audio::audio_device_format(playback[0]).unwrap(),
        (AudioSpec::new(AudioFormat::S16, 2, 44100), 1024)
    );
    assert_eq!(
        crate::audio::audio_device_format(recording[0]).unwrap(),
        (AudioSpec::new(AudioFormat::S16, 1, 44100), 1024)
    );
}

/// Opening and closing devices in a spawned program (not an app) on the
/// iOS Simulator: the audio session read back, and whether the devices'
/// queues run, against upstream's C in the same place. (Upstream's
/// `SDL_Init()` ends such a process, registering its notification
/// delegate with `UNUserNotificationCenter`, which throws outside an app:
/// its program stands in a nil notification center. What the session was
/// before depends on the tests that ran before in the process, so the
/// trace starts with the first device.)
#[cfg(not(target_os = "macos"))]
#[test]
fn spawned_session_matches_c() {
    use crate::core::apple::foundation::string_from_ns;
    use crate::core::apple::objc::{autoreleasepool, class, msg_send, Id, NSUInteger};
    use std::sync::atomic::AtomicI32;

    /// Upstream's trace, from the same steps in C spawned on the same
    /// simulator (an iPhone 16 with iOS 18.5), the runner's audio playing
    /// and recording.
    #[cfg(not(target_os = "tvos"))]
    const EXPECTED: &str = "\
init: ok driver=coreaudio
playback 0: System audio playback device
recording 0: System audio recording device
playback: open: ok format=0x8010 channels=2 freq=44100 frames=1024
playback: session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
playback: playback advanced=yes recording advanced=no
playback: closed
playback closed: session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
recording: open: ok format=0x8010 channels=1 freq=44100 frames=1024
recording: session: category=AVAudioSessionCategoryRecord options=0x5 mode=AVAudioSessionModeDefault
recording: playback advanced=no recording advanced=yes
recording: closed
recording closed: session: category=AVAudioSessionCategoryRecord options=0x5 mode=AVAudioSessionModeDefault
both: playback: open: ok format=0x8010 channels=2 freq=44100 frames=1024
both: recording: open: ok format=0x8010 channels=1 freq=44100 frames=1024
both: session: category=AVAudioSessionCategoryPlayAndRecord options=0x6d mode=AVAudioSessionModeDefault
both: playback advanced=yes recording advanced=yes
both: recording closed
both: recording closed: session: category=AVAudioSessionCategoryPlayAndRecord options=0x6d mode=AVAudioSessionModeDefault
both: playback closed
both: playback closed: session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
ambient: open: ok format=0x8010 channels=2 freq=44100 frames=1024
ambient: session: category=AVAudioSessionCategoryAmbient options=0x1 mode=AVAudioSessionModeDefault
ambient: closed
ducking: open: ok format=0x8010 channels=2 freq=44100 frames=1024
ducking: session: category=AVAudioSessionCategoryPlayback options=0x3 mode=AVAudioSessionModeDefault
ducking: closed
quit: session: category=AVAudioSessionCategoryPlayback options=0x3 mode=AVAudioSessionModeDefault
done
";

    /// The same on tvOS (an Apple TV 4K (3rd generation) with tvOS 18.5).
    #[cfg(target_os = "tvos")]
    const EXPECTED: &str = "\
init: ok driver=coreaudio
playback 0: System audio playback device
recording 0: System audio recording device
playback: open: ok format=0x8010 channels=2 freq=44100 frames=1024
playback: session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
playback: playback advanced=yes recording advanced=no
playback: closed
playback closed: session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
recording: open: ok format=0x8010 channels=1 freq=44100 frames=1024
recording: session: category=AVAudioSessionCategoryRecord options=0x5 mode=AVAudioSessionModeDefault
recording: playback advanced=no recording advanced=yes
recording: closed
recording closed: session: category=AVAudioSessionCategoryRecord options=0x5 mode=AVAudioSessionModeDefault
both: playback: open: ok format=0x8010 channels=2 freq=44100 frames=1024
both: recording: open: ok format=0x8010 channels=1 freq=44100 frames=1024
both: session: category=AVAudioSessionCategoryPlayAndRecord options=0x65 mode=AVAudioSessionModeDefault
both: playback advanced=yes recording advanced=yes
both: recording closed
both: recording closed: session: category=AVAudioSessionCategoryPlayAndRecord options=0x65 mode=AVAudioSessionModeDefault
both: playback closed
both: playback closed: session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
ambient: open: ok format=0x8010 channels=2 freq=44100 frames=1024
ambient: session: category=AVAudioSessionCategoryAmbient options=0x1 mode=AVAudioSessionModeDefault
ambient: closed
ducking: open: ok format=0x8010 channels=2 freq=44100 frames=1024
ducking: session: category=AVAudioSessionCategoryPlayback options=0x3 mode=AVAudioSessionModeDefault
ducking: closed
quit: session: category=AVAudioSessionCategoryPlayback options=0x3 mode=AVAudioSessionModeDefault
done
";

    static COUNTS: [AtomicI32; 2] = [AtomicI32::new(0), AtomicI32::new(0)];

    fn session(trace: &mut Vec<String>, label: &str) {
        autoreleasepool(|| {
            // SAFETY: AVAudioSession's shared instance and its properties.
            let (category, options, mode) = unsafe {
                let s: Id = msg_send![class!(AVAudioSession), sharedInstance];
                let category: Id = msg_send![s, category];
                let options: NSUInteger = msg_send![s, categoryOptions];
                let mode: Id = msg_send![s, mode];
                (string_from_ns(category), options, string_from_ns(mode))
            };
            trace.push(format!(
                "{label}: session: category={} options={options:#x} mode={}",
                category.as_deref().unwrap_or("nil"),
                mode.as_deref().unwrap_or("nil")
            ));
        })
    }

    fn open(trace: &mut Vec<String>, label: &str, which: u32, idx: usize) -> u32 {
        match crate::audio::open_audio_device(which, None) {
            Err(e) => {
                trace.push(format!("{label}: open: error: {}", e.message()));
                0
            }
            Ok(d) => {
                let (spec, frames) = crate::audio::audio_device_format(d).unwrap();
                trace.push(format!(
                    "{label}: open: ok format={:#x} channels={} freq={} frames={frames}",
                    spec.format.0, spec.channels, spec.freq
                ));
                crate::audio::set_audio_postmix_callback(
                    d,
                    Some(move |_: &AudioSpec, _: &mut [f32]| {
                        COUNTS[idx].fetch_add(1, Ordering::Relaxed);
                    }),
                )
                .unwrap();
                d
            }
        }
    }

    fn advancing(trace: &mut Vec<String>, label: &str) {
        let counts = || {
            (
                COUNTS[0].load(Ordering::Relaxed),
                COUNTS[1].load(Ordering::Relaxed),
            )
        };
        std::thread::sleep(Duration::from_millis(250));
        let (p0, r0) = counts();
        std::thread::sleep(Duration::from_millis(500));
        let (p1, r1) = counts();
        let yes_no = |b: bool| if b { "yes" } else { "no" };
        eprintln!("note: {label}: callbacks {} {}", p1 - p0, r1 - r0);
        trace.push(format!(
            "{label}: playback advanced={} recording advanced={}",
            yes_no(p1 > p0),
            yes_no(r1 > r0)
        ));
    }

    fn close(trace: &mut Vec<String>, line: &str, d: u32) {
        if d != 0 {
            crate::audio::close_audio_device(d);
            trace.push(line.into());
        }
    }

    let _l = crate::test_support::test_lock();
    let _cleanup = crate::audio::drivers::tests::QuitAudioOnDrop;
    let mut trace: Vec<String> = Vec::new();
    let t = &mut trace;

    crate::hints::set(crate::hints::AUDIO_DRIVER, "coreaudio").unwrap();
    let r = crate::init::init_subsystem(InitFlags::AUDIO);
    crate::hints::reset(crate::hints::AUDIO_DRIVER);
    t.push(format!(
        "init: {} driver={}",
        match &r {
            Ok(()) => "ok".to_string(),
            Err(e) => e.message().to_string(),
        },
        current_audio_driver().unwrap_or("(null)")
    ));
    for (i, id) in playback_devices()
        .unwrap_or_default()
        .into_iter()
        .enumerate()
    {
        t.push(format!("playback {i}: {}", audio_device_name(id).unwrap()));
    }
    for (i, id) in recording_devices()
        .unwrap_or_default()
        .into_iter()
        .enumerate()
    {
        t.push(format!("recording {i}: {}", audio_device_name(id).unwrap()));
    }

    let p = open(t, "playback", AUDIO_DEVICE_DEFAULT_PLAYBACK, 0);
    session(t, "playback");
    advancing(t, "playback");
    close(t, "playback: closed", p);
    session(t, "playback closed");

    let r = open(t, "recording", AUDIO_DEVICE_DEFAULT_RECORDING, 1);
    session(t, "recording");
    advancing(t, "recording");
    close(t, "recording: closed", r);
    session(t, "recording closed");

    let p = open(t, "both: playback", AUDIO_DEVICE_DEFAULT_PLAYBACK, 0);
    let r = open(t, "both: recording", AUDIO_DEVICE_DEFAULT_RECORDING, 1);
    session(t, "both");
    advancing(t, "both");
    close(t, "both: recording closed", r);
    session(t, "both: recording closed");
    close(t, "both: playback closed", p);
    session(t, "both: playback closed");

    crate::hints::set(crate::hints::AUDIO_CATEGORY, "ambient").unwrap();
    crate::hints::set(crate::hints::AUDIO_DUCK_OTHERS, "1").unwrap();
    let p = open(t, "ambient", AUDIO_DEVICE_DEFAULT_PLAYBACK, 0);
    session(t, "ambient");
    close(t, "ambient: closed", p);
    crate::hints::reset(crate::hints::AUDIO_CATEGORY);
    let p = open(t, "ducking", AUDIO_DEVICE_DEFAULT_PLAYBACK, 0);
    session(t, "ducking");
    close(t, "ducking: closed", p);
    crate::hints::reset(crate::hints::AUDIO_DUCK_OTHERS);

    crate::init::quit_subsystem(InitFlags::AUDIO);
    session(t, "quit");
    t.push("done".into());

    for line in &trace {
        eprintln!("trace: {line}");
    }
    let expected: Vec<&str> = EXPECTED.lines().collect();
    assert_eq!(trace, expected);
}
