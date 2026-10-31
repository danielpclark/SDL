// Tests for the driver list and the bootstrap's fall-through, which have to
// pass whether or not this machine has a sound server, a sound card or the
// system libraries the platform drivers load.

use crate::audio::{audio_driver, current_audio_driver, num_audio_drivers};
use crate::init::InitFlags;

/// The deduplicated driver list, in upstream's `bootstrap[]` order.
pub(crate) const EXPECTED_DRIVERS: &[&str] = &[
    #[cfg(target_os = "linux")]
    "pipewire",
    #[cfg(target_os = "linux")]
    "pulseaudio",
    #[cfg(target_os = "linux")]
    "alsa",
    #[cfg(windows)]
    "wasapi",
    #[cfg(target_vendor = "apple")]
    "coreaudio",
    #[cfg(target_os = "android")]
    "AAudio",
    #[cfg(target_os = "android")]
    "openslES",
    #[cfg(target_os = "emscripten")]
    "emscripten",
    "disk",
    "dummy",
];

/// The drivers that initialize without being asked for by name.
const NON_DEMAND_ONLY: &[&str] = &[
    #[cfg(target_os = "linux")]
    "pipewire",
    #[cfg(target_os = "linux")]
    "pulseaudio",
    #[cfg(target_os = "linux")]
    "alsa",
    #[cfg(windows)]
    "wasapi",
    #[cfg(target_vendor = "apple")]
    "coreaudio",
    #[cfg(target_os = "android")]
    "AAudio",
    #[cfg(target_os = "android")]
    "openslES",
    #[cfg(target_os = "emscripten")]
    "emscripten",
];

/// Shuts audio down (and resets the driver hints) when dropped, so a
/// failing driver test doesn't leave the subsystem up for the next one.
#[cfg_attr(
    not(any(target_os = "linux", target_vendor = "apple", target_os = "android")),
    allow(dead_code)
)] // (used by the platform drivers' tests)
pub(crate) struct QuitAudioOnDrop;

impl Drop for QuitAudioOnDrop {
    fn drop(&mut self) {
        for _ in 0..8 {
            if current_audio_driver().is_none() {
                break;
            }
            crate::init::quit_subsystem(InitFlags::AUDIO);
        }
        for h in [
            crate::hints::AUDIO_DRIVER,
            crate::hints::AUDIO_INCLUDE_MONITORS,
            crate::hints::AUDIO_ALSA_DEFAULT_PLAYBACK_DEVICE,
            crate::hints::AUDIO_ALSA_DEFAULT_RECORDING_DEVICE,
        ] {
            crate::hints::reset(h);
        }
    }
}

/// Initialize audio with `hint` as the driver hint, returning the driver
/// that came up (and shutting it down again).
fn init_with(hint: Option<&str>) -> Option<&'static str> {
    crate::hints::reset(crate::hints::AUDIO_DRIVER);
    if let Some(h) = hint {
        crate::hints::set(crate::hints::AUDIO_DRIVER, h).unwrap();
    }
    let r = crate::init::init_subsystem(InitFlags::AUDIO);
    let name = current_audio_driver();
    assert_eq!(r.is_ok(), name.is_some());
    if r.is_ok() {
        crate::init::quit_subsystem(InitFlags::AUDIO);
    }
    crate::hints::reset(crate::hints::AUDIO_DRIVER);
    assert!(current_audio_driver().is_none());
    name
}

#[test]
fn driver_list_is_in_upstream_order() {
    let drivers: Vec<&str> = (0..num_audio_drivers())
        .map(|i| audio_driver(i).unwrap())
        .collect();
    assert_eq!(drivers, EXPECTED_DRIVERS);
}

#[test]
fn bootstrap_falls_through_to_the_next_driver() {
    let _l = crate::test_support::test_lock();

    // Which drivers work here, each asked for by name.
    let works: Vec<&str> = EXPECTED_DRIVERS
        .iter()
        .copied()
        .filter(|&d| init_with(Some(d)) == Some(d))
        .collect();
    eprintln!("note: audio drivers that initialize here: {works:?}");
    assert!(works.contains(&"dummy") && works.contains(&"disk"));

    // A list in the hint: the first one that initializes wins, so failing
    // drivers fall through to the next, and finally to dummy.
    let mut list: Vec<&str> = NON_DEMAND_ONLY.to_vec();
    list.push("dummy");
    let expected = list.iter().copied().find(|d| works.contains(d));
    assert_eq!(init_with(Some(&list.join(","))), expected);
    assert_eq!(init_with(Some("nosuch,dummy")), Some("dummy"));
    assert_eq!(init_with(Some("nosuch")), None);

    // No hint: the first non-demand-only driver that initializes, in
    // bootstrap order, or nothing. (PipeWire comes first in its "preferred"
    // form, which also gives up when the server has no session manager
    // with audio, or no devices; then it's tried again after PulseAudio.)
    let mut working = NON_DEMAND_ONLY
        .iter()
        .copied()
        .filter(|d| works.contains(d));
    let expected = working.next();
    let got = init_with(None);
    if got != expected {
        assert_eq!(
            expected,
            Some("pipewire"),
            "only PipeWire's preferred form can step aside"
        );
        assert_eq!(got, working.next());
    }
}

/// The Android drivers (AAudio, OpenSL ES) in a program the runner spawns:
/// no Java VM, but AAudio and OpenSL ES work for the shell's processes too.
/// Their default devices, a playback device's format and its iterations (a
/// postmix callback's), a stream's tone played through it, and recording,
/// which needs the app's `RECORD_AUDIO` permission, so a Java VM. Note
/// (upstream): there upstream dereferences a null `JNIEnv` (the device
/// threads' `Android_AudioThreadInit()`, AAudio's `Deinitialize`'s
/// `Android_StopAudioHotplug()`, the permission's request); here those do
/// nothing and the request fails. (`android_audio` and
/// `android_audio_opensles` trace the drivers in an app, against
/// upstream's C.)
#[cfg(target_os = "android")]
fn android_driver_without_a_java_vm(hint: &str, name: &str) {
    use crate::audio::{
        audio_device_format, audio_device_name, close_audio_device, open_audio_device,
        open_audio_device_stream, playback_devices, recording_devices, set_audio_postmix_callback,
        AudioFormat, AudioSpec, AUDIO_DEVICE_DEFAULT_PLAYBACK, AUDIO_DEVICE_DEFAULT_RECORDING,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let _l = crate::test_support::test_lock();
    let _quit = QuitAudioOnDrop;
    assert!(crate::core::android::get_android_jni_env().is_none());

    crate::hints::set(crate::hints::AUDIO_DRIVER, hint).unwrap();
    crate::init::init_subsystem(InitFlags::AUDIO).unwrap();
    assert_eq!(current_audio_driver(), Some(name));

    // The default devices only
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

    // Playback: the device's format, and its iterations
    let dev = open_audio_device(AUDIO_DEVICE_DEFAULT_PLAYBACK, None).unwrap();
    let (spec, frames) = audio_device_format(dev).unwrap();
    eprintln!("note: {name} playback: {spec:?}, {frames} frames");
    assert!([
        AudioFormat::S16,
        AudioFormat::S32,
        AudioFormat::F32,
        AudioFormat::U8
    ]
    .contains(&spec.format));
    assert!(spec.channels >= 1 && spec.freq > 0 && frames > 0);
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    set_audio_postmix_callback(
        dev,
        Some(move |_: &AudioSpec, _: &mut [f32]| {
            c.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while count.load(Ordering::Relaxed) < 5 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        count.load(Ordering::Relaxed) >= 5,
        "{name}'s device didn't iterate"
    );
    close_audio_device(dev);

    // A stream's tone, played through the default device
    let tone = AudioSpec {
        format: AudioFormat::S16,
        channels: 1,
        freq: 22050,
    };
    let stream = open_audio_device_stream(
        AUDIO_DEVICE_DEFAULT_PLAYBACK,
        Some(&tone),
        None::<fn(&crate::audio::AudioStream, i32, i32)>,
    )
    .unwrap();
    let samples: Vec<u8> = (0..tone.freq / 4)
        .flat_map(|i| {
            let t = i as f32 / tone.freq as f32;
            (((t * 440.0 * std::f32::consts::TAU).sin() * 8000.0) as i16).to_ne_bytes()
        })
        .collect();
    stream.put_data(&samples).unwrap();
    stream.resume_device().unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while stream.queued() > 0 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(stream.queued(), 0, "{name} didn't play the tone");
    drop(stream);

    // Recording asks for the permission, which needs a Java VM
    assert_eq!(
        open_audio_device(AUDIO_DEVICE_DEFAULT_RECORDING, None)
            .unwrap_err()
            .message(),
        "Failed, there is no JavaVM"
    );
}

#[cfg(target_os = "android")]
#[test]
fn aaudio_without_a_java_vm() {
    android_driver_without_a_java_vm("aaudio", "AAudio");
}

#[cfg(target_os = "android")]
#[test]
fn opensles_without_a_java_vm() {
    android_driver_without_a_java_vm("opensles", "openslES");
}
