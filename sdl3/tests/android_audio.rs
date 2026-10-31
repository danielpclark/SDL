// Tests of the Android audio drivers in an app: AAudio (the default).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (`harness = false`), run by `tools/android_run.py`, whose
//! manifest asks for `RECORD_AUDIO` (not granted at first), and whose main
//! function traces the AAudio driver (see `android_audio_app`: the
//! devices, the audio manager's device callback, playback, a stream's
//! tone, recording denied and granted, the trips to the background).
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! (built with upstream's android-project, Gradle and CMake, run by the
//! runner's `--run-apk`) on the same emulator (API 35, Google APIs,
//! x86-64). Elsewhere this program does nothing.

#[cfg(not(target_os = "android"))]
fn main() {}

/// The runner's words: run as an app, which asks for the microphone.
#[cfg(target_os = "android")]
#[used]
#[link_section = ".sdl3_runner"]
static RUNNER: [u8; 29] = *b"app permissions=RECORD_AUDIO\0";

#[cfg(target_os = "android")]
mod android_support;

#[cfg(target_os = "android")]
mod android_audio_app;

/// Upstream's trace (the C program, on the emulator: API 35, Google APIs,
/// x86-64).
#[cfg(target_os = "android")]
const EXPECTED: &str = "\
drivers: AAudio, openslES, disk, dummy
init: ok
init: audio driver: AAudio
init: playback device 0: System audio playback device
init: playback devices: 1
init: recording device 0: System audio recording device
init: recording devices: 1
default playback: format=0x8010 channels=2 freq=44100 frames=1024
default recording: format=0x8010 channels=1 freq=44100 frames=1024
init: events: AUDIO_DEVICE_ADDED(System audio playback device, recording=0), AUDIO_DEVICE_ADDED(System audio recording device, recording=1)
hotplug: registerAudioDeviceCallback(): ok
hotplug: playback device 0: System audio playback device
hotplug: playback devices: 1
hotplug: recording device 0: System audio recording device
hotplug: recording devices: 1
hotplug: events: (none)
hotplug: unregisterAudioDeviceCallback(): ok
playback: open: ok
playback: format=0x8010 channels=2 freq=44100 frames=882
playback: playback advanced=yes
playback: thread: name=SDLAudioP(playback device) nice=-16
playback: closed
stream: open: ok
stream: format=0x8010 channels=1 freq=22050 frames=441
stream: played: yes
stream: closed
denied: open: error: This app doesn't have RECORD_AUDIO permission
denied: dialog: tapped permission_deny_button
recording: open: ok
recording: format=0x8010 channels=1 freq=44100 frames=882
recording: recording advanced=yes
recording: thread: name=SDLAudioC(recording device) nice=-16
both: open: ok
both: format=0x8010 channels=2 freq=44100 frames=882
blocking trip: before: playback advanced=yes recording advanced=yes
blocking trip: in the background: yes
blocking trip: in the background: playback advanced=no recording advanced=no
blocking trip: back: yes
blocking trip: back: playback advanced=yes recording advanced=yes
blocking trip: app events: WILL_ENTER_BACKGROUND(0,0), DID_ENTER_BACKGROUND(0,0), WILL_ENTER_FOREGROUND(0,0), DID_ENTER_FOREGROUND(0,0)
both: closed
both: events: (none)
quit
init again: ok
init again: audio driver: AAudio
not blocking: open: ok
not blocking: format=0x8010 channels=2 freq=44100 frames=882
non-blocking trip: before: playback advanced=yes
non-blocking trip: in the background: yes
non-blocking trip: in the background: playback advanced=yes
non-blocking trip: back: yes
non-blocking trip: back: playback advanced=yes
non-blocking trip: app events: WILL_ENTER_BACKGROUND(0,0), DID_ENTER_BACKGROUND(0,0), WILL_ENTER_FOREGROUND(0,0), DID_ENTER_FOREGROUND(0,0)
not blocking: closed
main returns
";

#[cfg(target_os = "android")]
fn main() -> std::process::ExitCode {
    android_support::start();
    let status = android_support::checked(|| android_audio_app::main(None, EXPECTED));
    android_support::finish(status)
}
