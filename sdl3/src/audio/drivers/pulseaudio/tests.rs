// Tests for the PulseAudio driver: the ABI declarations and format and
// channel map tables, the failed connection without a server, and (when the
// `pulseaudio` binary is installed) playback and recording through a
// private server with a null sink.

use super::*;
use crate::audio::{
    audio_device_name, current_audio_driver, playback_devices, recording_devices, AudioDevice,
    AudioStream, AUDIO_DEVICE_DEFAULT_PLAYBACK,
};
use crate::init::InitFlags;
use crate::test_support::TempDir;
use std::mem::{offset_of, size_of};
use std::time::Duration;

#[test]
fn abi_layouts_match_libpulse() {
    // <pulse/sample.h>, <pulse/channelmap.h>, <pulse/volume.h>, <pulse/def.h>
    assert_eq!(size_of::<PaSampleSpec>(), 12);
    assert_eq!(size_of::<PaChannelMap>(), 4 + 4 * PA_CHANNELS_MAX);
    assert_eq!(size_of::<PaCvolume>(), 4 + 4 * PA_CHANNELS_MAX);
    assert_eq!(size_of::<PaBufferAttr>(), 20);
    // <pulse/introspect.h> (LP64)
    #[cfg(target_pointer_width = "64")]
    {
        assert_eq!(offset_of!(PaSinkInfo, sample_spec), 24);
        assert_eq!(offset_of!(PaSourceInfo, channel_map), 36);
        assert_eq!(offset_of!(PaSourceInfo, owner_module), 168);
        assert_eq!(offset_of!(PaSourceInfo, volume), 172);
        assert_eq!(offset_of!(PaSourceInfo, mute), 304);
        assert_eq!(offset_of!(PaSourceInfo, monitor_of_sink), 308);
        assert_eq!(offset_of!(PaServerInfo, sample_spec), 32);
        assert_eq!(offset_of!(PaServerInfo, default_sink_name), 48);
        assert_eq!(offset_of!(PaServerInfo, default_source_name), 56);
    }
}

#[test]
fn formats_round_trip() {
    for (sdl, pa) in [
        (AudioFormat::U8, 0),
        (AudioFormat::S16LE, 3),
        (AudioFormat::S16BE, 4),
        (AudioFormat::F32LE, 5),
        (AudioFormat::F32BE, 6),
        (AudioFormat::S32LE, 7),
        (AudioFormat::S32BE, 8),
    ] {
        assert_eq!(sdl_format_to_pulse_format(sdl), Some(pa));
        assert_eq!(pulse_format_to_sdl_format(pa), sdl);
    }
    // No signed 8-bit format in PulseAudio; A-law, µ-law and 24-bit aren't SDL's.
    assert_eq!(sdl_format_to_pulse_format(AudioFormat::S8), None);
    for pa in [1, 2, 9, 10, 11, 12, -1] {
        assert_eq!(pulse_format_to_sdl_format(pa), AudioFormat::UNKNOWN);
    }
}

#[test]
fn channel_maps_follow_sdl_order() {
    let map = |channels: u8| {
        let mut pacmap = PaChannelMap {
            channels: 0,
            map: [-1; PA_CHANNELS_MAX],
        };
        pulse_create_channel_map(&mut pacmap, channels);
        assert_eq!(pacmap.channels, channels);
        pacmap.map[..channels as usize].to_vec()
    };
    // PA_CHANNEL_POSITION_*: MONO 0, FL 1, FR 2, FC 3, RC 4, RL 5, RR 6, LFE 7, SL 10, SR 11.
    assert_eq!(map(1), [0]);
    assert_eq!(map(2), [1, 2]);
    assert_eq!(map(3), [1, 2, 7]);
    assert_eq!(map(4), [1, 2, 5, 6]);
    assert_eq!(map(5), [1, 2, 7, 5, 6]);
    assert_eq!(map(6), [1, 2, 3, 7, 5, 6]);
    assert_eq!(map(7), [1, 2, 3, 7, 4, 10, 11]);
    assert_eq!(map(8), [1, 2, 3, 7, 5, 6, 10, 11]);
}

#[test]
fn state_predicates() {
    // PA_CONTEXT_IS_GOOD: CONNECTING, AUTHORIZING, SETTING_NAME, READY.
    let good: Vec<bool> = (0..7).map(pa_context_is_good).collect();
    assert_eq!(good, [false, true, true, true, true, false, false]);
    // PA_STREAM_IS_GOOD: CREATING, READY.
    let good: Vec<bool> = (0..5).map(pa_stream_is_good).collect();
    assert_eq!(good, [false, true, true, false, false]);
}

/// Wait up to `timeout` for `what`, pumping events (disconnects are
/// handled on the main thread).
fn wait_for_up_to(timeout: Duration, what: &dyn Fn() -> bool) -> bool {
    let start = std::time::Instant::now();
    while !what() && start.elapsed() < timeout {
        crate::events::pump();
        std::thread::sleep(Duration::from_millis(10));
    }
    what()
}

/// A private PulseAudio server with a null sink, killed on drop.
struct PrivateServer {
    child: std::process::Child,
    _dir: TempDir,
}

impl Drop for PrivateServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Start `pulseaudio` in a temp dir, or `None` (with a note) if that isn't
/// possible here.
fn start_private_server() -> Option<(PrivateServer, String)> {
    let dir = TempDir::new("pulse");
    let socket = dir.path("native");
    let child = std::process::Command::new("pulseaudio")
        .args([
            "-n",
            "--daemonize=no",
            "--exit-idle-time=-1",
            "--disable-shm=yes",
            "--log-target=stderr",
            "-L",
            "module-null-sink sink_name=sdltest",
            "-L",
            &format!("module-native-protocol-unix socket={socket} auth-anonymous=1"),
        ])
        .env("HOME", &dir.0)
        .env("XDG_RUNTIME_DIR", &dir.0)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let child = match child {
        Ok(c) => c,
        Err(e) => {
            eprintln!("note: skipping the PulseAudio server test: can't run pulseaudio: {e}");
            return None;
        }
    };
    let server = PrivateServer { child, _dir: dir };
    let start = std::time::Instant::now();
    while !std::path::Path::new(&socket).exists() {
        if start.elapsed() > Duration::from_secs(10) {
            eprintln!("note: skipping the PulseAudio server test: the server didn't start");
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Some((server, format!("unix:{socket}")))
}

#[test]
fn no_server_means_no_driver() {
    let _l = crate::test_support::test_lock();
    if std::env::var_os("PULSE_SERVER").is_some() {
        eprintln!("note: PULSE_SERVER is set; not testing the failed connection");
        return;
    }
    // A server address nobody listens on: libpulse fails to connect and the
    // driver fails to initialize (falling through to the next one).
    let dir = TempDir::new("pulse-none");
    let lib = match load_pulseaudio_library() {
        Ok(lib) => lib,
        Err(e) => {
            eprintln!("note: libpulse isn't installed: {}", e.message());
            return;
        }
    };
    // SAFETY: pa_get_library_version() has no preconditions.
    let version = unsafe { opt_str((lib.pa_get_library_version)()) };
    assert!(version.is_some_and(|v| !v.is_empty()));
    drop(lib);
    std::env::set_var("PULSE_SERVER", format!("unix:{}", dir.path("nothing-here")));
    crate::hints::set(crate::hints::AUDIO_DRIVER, "pulseaudio").unwrap();
    let r = crate::init::init_subsystem(InitFlags::AUDIO);
    std::env::remove_var("PULSE_SERVER");
    crate::hints::reset(crate::hints::AUDIO_DRIVER);
    assert!(r.is_err());
    assert!(current_audio_driver().is_none());
}

#[test]
fn playback_and_recording_through_a_private_server() {
    let _l = crate::test_support::test_lock();
    if load_pulseaudio_library().is_err() {
        eprintln!("note: skipping the PulseAudio server test: libpulse isn't installed");
        return;
    }
    let Some((_server, address)) = start_private_server() else {
        return;
    };
    let _cleanup = crate::audio::drivers::tests::QuitAudioOnDrop;
    std::env::set_var("PULSE_SERVER", &address);
    crate::hints::set(crate::hints::AUDIO_DRIVER, "pulseaudio").unwrap();
    crate::hints::set(crate::hints::AUDIO_INCLUDE_MONITORS, "1").unwrap();
    let r = crate::init::init_subsystem(InitFlags::AUDIO);
    std::env::remove_var("PULSE_SERVER");
    let reset = || {
        crate::hints::reset(crate::hints::AUDIO_DRIVER);
        crate::hints::reset(crate::hints::AUDIO_INCLUDE_MONITORS);
    };
    if let Err(e) = r {
        reset();
        panic!(
            "PulseAudio should connect to the private server: {}",
            e.message()
        );
    }
    assert_eq!(current_audio_driver(), Some("pulseaudio"));
    assert!(!playback_devices().unwrap().is_empty(), "the null sink");
    let monitors = recording_devices().unwrap();
    assert!(!monitors.is_empty(), "the null sink's monitor");

    // Playback: the stream drains into the null sink.
    let dev = AudioDevice::open(AUDIO_DEVICE_DEFAULT_PLAYBACK, None).unwrap();
    let (spec, frames) = dev.format().unwrap();
    assert!(frames > 0);
    let stream = AudioStream::new(Some(&spec), None).unwrap();
    dev.bind(&stream).unwrap();
    stream
        .put_data(&vec![0x11u8; spec.frame_size() * frames as usize * 4])
        .unwrap();
    let start = std::time::Instant::now();
    while stream.queued() > 0 && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(stream.queued(), 0, "the device pulled the data");

    // Recording from the monitor while playing.
    let rec = AudioDevice::open(monitors[0], None).unwrap();
    let (rspec, _) = rec.format().unwrap();
    let rstream = AudioStream::new(None, Some(&rspec)).unwrap();
    rec.bind(&rstream).unwrap();
    let start = std::time::Instant::now();
    while rstream.available() == 0 && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(rstream.available() > 0, "the monitor recorded");
    drop(rec);
    drop(rstream);
    drop(dev);
    drop(stream);

    // Hotplug: a sink loaded at run time appears, becomes the default and
    // disappears again (when pactl is there to do it). This runs without
    // the monitors, since a removed sink is looked up by an index that a
    // source can share (see the FIXME in `hotplug_callback`).
    crate::init::quit_subsystem(InitFlags::AUDIO);
    crate::hints::reset(crate::hints::AUDIO_INCLUDE_MONITORS);
    std::env::set_var("PULSE_SERVER", &address);
    let r = crate::init::init_subsystem(InitFlags::AUDIO);
    std::env::remove_var("PULSE_SERVER");
    if let Err(e) = r {
        reset();
        panic!("PulseAudio should reconnect: {}", e.message());
    }
    let pactl = |args: &[&str]| {
        std::process::Command::new("pactl")
            .args(args)
            .env("PULSE_SERVER", &address)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    };
    let named = |name: &str| {
        playback_devices()
            .unwrap()
            .into_iter()
            .find(|&d| audio_device_name(d).is_ok_and(|n| n == name))
    };
    let wait_for = |what: &dyn Fn() -> bool| wait_for_up_to(Duration::from_secs(10), what);
    if let Some(module) = pactl(&[
        "load-module",
        "module-null-sink",
        "sink_name=hotplugged",
        "sink_properties=device.description=Hotplugged",
    ]) {
        assert!(wait_for(&|| named("Hotplugged").is_some()), "added");
        pactl(&["set-default-sink", "hotplugged"]).unwrap();
        // (a volume change wakes the hotplug thread again, in case the
        // default change's signal was missed; see the FIXME in `hotplug_thread`)
        let is_default =
            || audio_device_name(AUDIO_DEVICE_DEFAULT_PLAYBACK).is_ok_and(|n| n == "Hotplugged");
        let mut changed = false;
        for volume in ["90%", "100%"].iter().cycle().take(10) {
            changed = wait_for_up_to(Duration::from_millis(500), &is_default);
            if changed {
                break;
            }
            let _ = pactl(&["set-sink-volume", "hotplugged", volume]);
        }
        assert!(changed, "the new default");
        pactl(&["unload-module", &module]).unwrap();
        assert!(wait_for(&|| named("Hotplugged").is_none()), "removed");
    } else {
        eprintln!("note: skipping the PulseAudio hotplug check: no pactl");
    }

    crate::init::quit_subsystem(InitFlags::AUDIO);
    reset();
}
