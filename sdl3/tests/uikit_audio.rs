// Tests of the CoreAudio driver on UIKit: the app's audio session, its
// interruptions and the default devices.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The audio session belongs to an app, so this is a program of its own
//! (`harness = false`), run as an app on the iOS Simulator by
//! `tools/ios_run.py` (the `__DATA,__sdl3_runner` section below, which
//! also has the runner grant the app the microphone), as `uikit_main` is.
//! Its main function initializes audio only, and traces (with the main run
//! loop running between the steps):
//!
//! * `[AVAudioSession sharedInstance]`'s category, options and mode
//!   before audio starts and at each step after (read back from the
//!   session);
//! * the driver, its devices (the default playback and recording ones
//!   only) and their formats, and the events that add them;
//! * the default playback device opened and closed for each value of
//!   `SDL_AUDIO_CATEGORY` (upstream's names and short names, in other
//!   cases, and values it doesn't know) with and without
//!   `SDL_AUDIO_DUCK_OTHERS`;
//! * the default recording device alone, then with the playback device
//!   (play and record), then in the playback category (where it can't
//!   open);
//! * interruptions, posted in the program
//!   (`AVAudioSessionInterruptionNotification`: began, ended, without a
//!   type, without the session) and ended by the app becoming active or
//!   coming to the foreground (the notifications posted), with a paused
//!   device, and around closing and reopening the device: whether the
//!   devices' postmix callbacks run (the queue's buffers reach the core)
//!   in half a second after each;
//! * a stream's half second of a tone played through the device, then
//!   audio quit.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! run the same way on the same simulator (an iPhone 16 with iOS 18.5),
//! whose audio goes to the runner's (a "Speaker" route, and a built-in
//! microphone). Elsewhere this program does nothing.

#[cfg(not(target_os = "ios"))]
fn main() {}

/// The runner's words: run as an app, granted the microphone.
#[cfg(target_os = "ios")]
#[used]
#[link_section = "__DATA,__sdl3_runner"]
static RUNNER: [u8; 23] = *b"app privacy=microphone\0";

#[cfg(target_os = "ios")]
mod uikit_support;

#[cfg(target_os = "ios")]
fn main() {
    ios::main()
}

#[cfg(target_os = "ios")]
mod ios {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::audio::{self, AudioDeviceID, AudioFormat, AudioSpec, AudioStream};
    use sdl3::events::{self, Event};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};

    use super::uikit_support::objc::{self, Id};
    use super::uikit_support::{check_trace, checked, elapsed, event_name, exit};

    /// Upstream's trace (the C program, on an iPhone 16 simulator with
    /// iOS 18.5).
    const EXPECTED: &str = "\
app state: active
start: session: category=AVAudioSessionCategorySoloAmbient options=0x0 mode=AVAudioSessionModeDefault
init audio: ok
audio driver: coreaudio
playback device 0: System audio playback device
playback devices: 1
recording device 0: System audio recording device
recording devices: 1
default playback: System audio playback device
default recording: System audio recording device
default playback: format=0x8010 channels=2 freq=44100 frames=1024
default recording: format=0x8010 channels=1 freq=44100 frames=1024
init: events: AUDIO_DEVICE_ADDED(System audio playback device, recording=0), AUDIO_DEVICE_ADDED(System audio recording device, recording=1)
after init: session: category=AVAudioSessionCategorySoloAmbient options=0x0 mode=AVAudioSessionModeDefault
case 0 (category=unset duck=unset): open: ok
case 0 (category=unset duck=unset): format=0x8010 channels=2 freq=44100 frames=1024
case 0 (category=unset duck=unset): session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
case 0 (category=unset duck=unset): closed
case 0 (category=unset duck=unset): session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
case 1 (category=unset duck=1): open: ok
case 1 (category=unset duck=1): format=0x8010 channels=2 freq=44100 frames=1024
case 1 (category=unset duck=1): session: category=AVAudioSessionCategoryPlayback options=0x3 mode=AVAudioSessionModeDefault
case 1 (category=unset duck=1): closed
case 1 (category=unset duck=1): session: category=AVAudioSessionCategoryPlayback options=0x3 mode=AVAudioSessionModeDefault
case 2 (category=ambient duck=unset): open: ok
case 2 (category=ambient duck=unset): format=0x8010 channels=2 freq=44100 frames=1024
case 2 (category=ambient duck=unset): session: category=AVAudioSessionCategoryAmbient options=0x1 mode=AVAudioSessionModeDefault
case 2 (category=ambient duck=unset): closed
case 2 (category=ambient duck=unset): session: category=AVAudioSessionCategoryAmbient options=0x1 mode=AVAudioSessionModeDefault
case 3 (category=ambient duck=1): open: ok
case 3 (category=ambient duck=1): format=0x8010 channels=2 freq=44100 frames=1024
case 3 (category=ambient duck=1): session: category=AVAudioSessionCategoryAmbient options=0x1 mode=AVAudioSessionModeDefault
case 3 (category=ambient duck=1): closed
case 3 (category=ambient duck=1): session: category=AVAudioSessionCategoryAmbient options=0x1 mode=AVAudioSessionModeDefault
case 4 (category=AVAudioSessionCategoryAmbient duck=unset): open: ok
case 4 (category=AVAudioSessionCategoryAmbient duck=unset): format=0x8010 channels=2 freq=44100 frames=1024
case 4 (category=AVAudioSessionCategoryAmbient duck=unset): session: category=AVAudioSessionCategoryAmbient options=0x1 mode=AVAudioSessionModeDefault
case 4 (category=AVAudioSessionCategoryAmbient duck=unset): closed
case 4 (category=AVAudioSessionCategoryAmbient duck=unset): session: category=AVAudioSessionCategoryAmbient options=0x1 mode=AVAudioSessionModeDefault
case 5 (category=AMBIENT duck=unset): open: ok
case 5 (category=AMBIENT duck=unset): format=0x8010 channels=2 freq=44100 frames=1024
case 5 (category=AMBIENT duck=unset): session: category=AVAudioSessionCategoryAmbient options=0x1 mode=AVAudioSessionModeDefault
case 5 (category=AMBIENT duck=unset): closed
case 5 (category=AMBIENT duck=unset): session: category=AVAudioSessionCategoryAmbient options=0x1 mode=AVAudioSessionModeDefault
case 6 (category=AVAudioSessionCategorySoloAmbient duck=unset): open: ok
case 6 (category=AVAudioSessionCategorySoloAmbient duck=unset): format=0x8010 channels=2 freq=44100 frames=1024
case 6 (category=AVAudioSessionCategorySoloAmbient duck=unset): session: category=AVAudioSessionCategorySoloAmbient options=0x0 mode=AVAudioSessionModeDefault
case 6 (category=AVAudioSessionCategorySoloAmbient duck=unset): closed
case 6 (category=AVAudioSessionCategorySoloAmbient duck=unset): session: category=AVAudioSessionCategorySoloAmbient options=0x0 mode=AVAudioSessionModeDefault
case 7 (category=AVAudioSessionCategorySoloAmbient duck=1): open: ok
case 7 (category=AVAudioSessionCategorySoloAmbient duck=1): format=0x8010 channels=2 freq=44100 frames=1024
case 7 (category=AVAudioSessionCategorySoloAmbient duck=1): session: category=AVAudioSessionCategorySoloAmbient options=0x0 mode=AVAudioSessionModeDefault
case 7 (category=AVAudioSessionCategorySoloAmbient duck=1): closed
case 7 (category=AVAudioSessionCategorySoloAmbient duck=1): session: category=AVAudioSessionCategorySoloAmbient options=0x0 mode=AVAudioSessionModeDefault
case 8 (category=playback duck=unset): open: ok
case 8 (category=playback duck=unset): format=0x8010 channels=2 freq=44100 frames=1024
case 8 (category=playback duck=unset): session: category=AVAudioSessionCategoryPlayback options=0x0 mode=AVAudioSessionModeDefault
case 8 (category=playback duck=unset): closed
case 8 (category=playback duck=unset): session: category=AVAudioSessionCategoryPlayback options=0x0 mode=AVAudioSessionModeDefault
case 9 (category=PlayBack duck=1): open: ok
case 9 (category=PlayBack duck=1): format=0x8010 channels=2 freq=44100 frames=1024
case 9 (category=PlayBack duck=1): session: category=AVAudioSessionCategoryPlayback options=0x3 mode=AVAudioSessionModeDefault
case 9 (category=PlayBack duck=1): closed
case 9 (category=PlayBack duck=1): session: category=AVAudioSessionCategoryPlayback options=0x3 mode=AVAudioSessionModeDefault
case 10 (category=AVAudioSessionCategoryPlayback duck=0): open: ok
case 10 (category=AVAudioSessionCategoryPlayback duck=0): format=0x8010 channels=2 freq=44100 frames=1024
case 10 (category=AVAudioSessionCategoryPlayback duck=0): session: category=AVAudioSessionCategoryPlayback options=0x0 mode=AVAudioSessionModeDefault
case 10 (category=AVAudioSessionCategoryPlayback duck=0): closed
case 10 (category=AVAudioSessionCategoryPlayback duck=0): session: category=AVAudioSessionCategoryPlayback options=0x0 mode=AVAudioSessionModeDefault
case 11 (category=playandrecord duck=unset): open: ok
case 11 (category=playandrecord duck=unset): format=0x8010 channels=2 freq=44100 frames=1024
case 11 (category=playandrecord duck=unset): session: category=AVAudioSessionCategoryPlayAndRecord options=0x6d mode=AVAudioSessionModeDefault
case 11 (category=playandrecord duck=unset): closed
case 11 (category=playandrecord duck=unset): session: category=AVAudioSessionCategoryPlayAndRecord options=0x6d mode=AVAudioSessionModeDefault
case 12 (category=playandrecord duck=1): open: ok
case 12 (category=playandrecord duck=1): format=0x8010 channels=2 freq=44100 frames=1024
case 12 (category=playandrecord duck=1): session: category=AVAudioSessionCategoryPlayAndRecord options=0x6f mode=AVAudioSessionModeDefault
case 12 (category=playandrecord duck=1): closed
case 12 (category=playandrecord duck=1): session: category=AVAudioSessionCategoryPlayAndRecord options=0x6f mode=AVAudioSessionModeDefault
case 13 (category=AVAudioSessionCategoryPlayAndRecord duck=unset): open: ok
case 13 (category=AVAudioSessionCategoryPlayAndRecord duck=unset): format=0x8010 channels=2 freq=44100 frames=1024
case 13 (category=AVAudioSessionCategoryPlayAndRecord duck=unset): session: category=AVAudioSessionCategoryPlayAndRecord options=0x6d mode=AVAudioSessionModeDefault
case 13 (category=AVAudioSessionCategoryPlayAndRecord duck=unset): closed
case 13 (category=AVAudioSessionCategoryPlayAndRecord duck=unset): session: category=AVAudioSessionCategoryPlayAndRecord options=0x6d mode=AVAudioSessionModeDefault
case 14 (category=record duck=unset): open: ok
case 14 (category=record duck=unset): format=0x8010 channels=2 freq=44100 frames=1024
case 14 (category=record duck=unset): session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
case 14 (category=record duck=unset): closed
case 14 (category=record duck=unset): session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
case 15 (category=bogus duck=1): open: ok
case 15 (category=bogus duck=1): format=0x8010 channels=2 freq=44100 frames=1024
case 15 (category=bogus duck=1): session: category=AVAudioSessionCategoryPlayback options=0x3 mode=AVAudioSessionModeDefault
case 15 (category=bogus duck=1): closed
case 15 (category=bogus duck=1): session: category=AVAudioSessionCategoryPlayback options=0x3 mode=AVAudioSessionModeDefault
case 16 (category= duck=unset): open: ok
case 16 (category= duck=unset): format=0x8010 channels=2 freq=44100 frames=1024
case 16 (category= duck=unset): session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
case 16 (category= duck=unset): closed
case 16 (category= duck=unset): session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
case 17 (category=unset duck=unset): open: ok
case 17 (category=unset duck=unset): format=0x8010 channels=2 freq=44100 frames=1024
case 17 (category=unset duck=unset): session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
case 17 (category=unset duck=unset): closed
case 17 (category=unset duck=unset): session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
categories: events: (none)
recording: open: ok
recording: format=0x8010 channels=1 freq=44100 frames=1024
recording: session: category=AVAudioSessionCategoryRecord options=0x5 mode=AVAudioSessionModeDefault
recording: recording advanced=yes
recording: closed
recording closed: session: category=AVAudioSessionCategoryRecord options=0x5 mode=AVAudioSessionModeDefault
both: playback: open: ok
both: playback: format=0x8010 channels=2 freq=44100 frames=1024
both: playback: session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
both: recording: open: ok
both: recording: format=0x8010 channels=1 freq=44100 frames=1024
both: session: category=AVAudioSessionCategoryPlayAndRecord options=0x6d mode=AVAudioSessionModeDefault
both: playback advanced=yes recording advanced=yes
both: posted AVAudioSessionInterruptionNotification type=began
both: began: playback advanced=no recording advanced=no
both: posted AVAudioSessionInterruptionNotification type=ended
both: ended: playback advanced=yes recording advanced=yes
both: recording closed
both: recording closed: session: category=AVAudioSessionCategoryPlayAndRecord options=0x6d mode=AVAudioSessionModeDefault
both: playback closed
both: playback closed: session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
recording in playback: open: error: Device not found
recording in playback: session: category=AVAudioSessionCategoryPlayback options=0x0 mode=AVAudioSessionModeDefault
recording: events: (none)
interruptions: open: ok
interruptions: format=0x8010 channels=2 freq=44100 frames=1024
interruptions: session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
playing: playback advanced=yes
began: posted AVAudioSessionInterruptionNotification type=began
began: playback advanced=no
began: device paused=no
began twice: posted AVAudioSessionInterruptionNotification type=began
began twice: playback advanced=no
ended: posted AVAudioSessionInterruptionNotification type=ended
ended: playback advanced=yes
ended twice: posted AVAudioSessionInterruptionNotification type=ended
ended twice: playback advanced=yes
before active: posted AVAudioSessionInterruptionNotification type=began
before active: playback advanced=no
posted UIApplicationDidBecomeActiveNotification
became active: playback advanced=yes
before foreground: posted AVAudioSessionInterruptionNotification type=began
before foreground: playback advanced=no
posted UIApplicationWillEnterForegroundNotification (no object)
will enter foreground: playback advanced=yes
before no type: posted AVAudioSessionInterruptionNotification type=began
before no type: playback advanced=no
posted AVAudioSessionInterruptionNotification without a type
no type: playback advanced=yes
posted AVAudioSessionInterruptionNotification type=began without the session
began without the session: playback advanced=yes
paused: yes
paused: playback advanced=no
paused: posted AVAudioSessionInterruptionNotification type=began
paused: posted AVAudioSessionInterruptionNotification type=ended
paused, interrupted and ended: playback advanced=no
resumed: paused=no
resumed: playback advanced=yes
before close: posted AVAudioSessionInterruptionNotification type=began
before close: playback advanced=no
closed while interrupted
closed while interrupted: session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
after close: posted AVAudioSessionInterruptionNotification type=ended
reopened: open: ok
reopened: format=0x8010 channels=2 freq=44100 frames=1024
reopened: playback advanced=yes
reopened: posted AVAudioSessionInterruptionNotification type=began
reopened: began: playback advanced=no
reopened: posted AVAudioSessionInterruptionNotification type=ended
reopened: ended: playback advanced=yes
reopened: closed
interruptions: events: (none)
stream: open: ok
stream: format=0x8010 channels=2 freq=44100 frames=1024
stream: played: yes
stream: closed
before quit: session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
quit audio
after quit: session: category=AVAudioSessionCategoryPlayback options=0x1 mode=AVAudioSessionModeDefault
quit: events: (none)
done
";

    static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn t(line: impl Into<String>) {
        let line = line.into();
        println!("trace: {line}");
        TRACE.lock().unwrap().push(line);
    }

    fn yes_no(b: bool) -> &'static str {
        if b {
            "yes"
        } else {
            "no"
        }
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFRunLoopDefaultMode: *const c_void;
        fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after: u8) -> i32;
    }

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {
        static AVAudioSessionInterruptionNotification: Id;
        static AVAudioSessionInterruptionTypeKey: Id;
    }

    /// `AVAudioSessionInterruptionTypeBegan` and `...Ended`.
    const BEGAN: usize = 1;
    const ENDED: usize = 0;

    /// Run the main run loop for `ms` milliseconds.
    fn run_loop(ms: u64) {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            // SAFETY: the main thread's run loop, in its default mode.
            unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, 0) };
        }
    }

    /// Run the run loop for `ms`, then the events (pumped).
    fn events(label: &str, ms: u64) {
        run_loop(ms);
        events::pump();
        let mut seen = Vec::new();
        while let Some(event) = events::poll() {
            match event {
                Event::AudioDevice(e) => {
                    let name = audio::audio_device_name(e.which).unwrap_or_else(|_| "?".into());
                    seen.push(format!(
                        "{}({name}, recording={})",
                        event_name(e.event_type),
                        e.recording as i32
                    ));
                }
                other => seen.push(event_name(other.event_type())),
            }
        }
        if seen.is_empty() {
            t(format!("{label}: events: (none)"));
        } else {
            t(format!("{label}: events: {}", seen.join(", ")));
        }
    }

    fn shared_session() -> Id {
        // SAFETY: AVAudioSession's class method.
        unsafe { objc::send0(objc::class("AVAudioSession"), "sharedInstance") }
    }

    fn session(label: &str) {
        let s = shared_session();
        // SAFETY: AVAudioSession's properties.
        let (category, options, mode): (Id, usize, Id) = unsafe {
            (
                objc::send0(s, "category"),
                objc::send0(s, "categoryOptions"),
                objc::send0(s, "mode"),
            )
        };
        t(format!(
            "{label}: session: category={} options={options:#x} mode={}",
            objc::string(category).unwrap_or_else(|| "nil".into()),
            objc::string(mode).unwrap_or_else(|| "nil".into())
        ));
        // SAFETY: as above.
        let (rate, preferred, io): (f64, f64, f64) = unsafe {
            (
                objc::send0(s, "sampleRate"),
                objc::send0(s, "preferredSampleRate"),
                objc::send0(s, "IOBufferDuration"),
            )
        };
        println!("uikit_audio: info: {label}: rate={rate} preferred={preferred} io={io}");
    }

    fn format_of(label: &str, id: AudioDeviceID) {
        match audio::audio_device_format(id) {
            Ok((spec, frames)) => t(format!(
                "{label}: format={:#x} channels={} freq={} frames={frames}",
                spec.format.0, spec.channels, spec.freq
            )),
            Err(e) => t(format!("{label}: format: error: {}", e.message())),
        }
    }

    fn open_dev(label: &str, which: AudioDeviceID) -> AudioDeviceID {
        match audio::open_audio_device(which, None) {
            Err(e) => {
                t(format!("{label}: open: error: {}", e.message()));
                0
            }
            Ok(d) => {
                t(format!("{label}: open: ok"));
                format_of(label, d);
                d
            }
        }
    }

    /// The postmix callbacks' counts (playback, recording).
    static COUNTS: [AtomicI32; 2] = [AtomicI32::new(0), AtomicI32::new(0)];

    fn count_callbacks(dev: AudioDeviceID, which: usize) {
        audio::set_audio_postmix_callback(
            dev,
            Some(move |_: &AudioSpec, _: &mut [f32]| {
                COUNTS[which].fetch_add(1, Ordering::Relaxed);
            }),
        )
        .unwrap();
    }

    /// Whether each device's postmix callback ran in half a second (after a
    /// quarter of a second to settle).
    fn advancing(label: &str, play: AudioDeviceID, rec: AudioDeviceID) {
        run_loop(250);
        let p0 = COUNTS[0].load(Ordering::Relaxed);
        let r0 = COUNTS[1].load(Ordering::Relaxed);
        run_loop(500);
        let p1 = COUNTS[0].load(Ordering::Relaxed);
        let r1 = COUNTS[1].load(Ordering::Relaxed);
        println!(
            "uikit_audio: info: {label}: playback callbacks {}, recording callbacks {}",
            p1 - p0,
            r1 - r0
        );
        if play != 0 && rec != 0 {
            t(format!(
                "{label}: playback advanced={} recording advanced={}",
                yes_no(p1 > p0),
                yes_no(r1 > r0)
            ));
        } else if rec != 0 {
            t(format!("{label}: recording advanced={}", yes_no(r1 > r0)));
        } else {
            t(format!("{label}: playback advanced={}", yes_no(p1 > p0)));
        }
    }

    /// `[[NSNotificationCenter defaultCenter] postNotificationName:name
    /// object:object userInfo:info]`.
    fn post(name: Id, object: Id, info: Id) {
        // SAFETY: NSNotificationCenter's methods.
        unsafe {
            let center: Id = objc::send0(objc::class("NSNotificationCenter"), "defaultCenter");
            let () = objc::send3(
                center,
                "postNotificationName:object:userInfo:",
                name,
                object,
                info,
            );
        }
    }

    /// `@{ AVAudioSessionInterruptionTypeKey: @(type) }`.
    fn interruption_info(ty: usize) -> Id {
        // SAFETY: NSNumber's and NSDictionary's class methods.
        unsafe {
            let number: Id = objc::send1(objc::class("NSNumber"), "numberWithUnsignedInteger:", ty);
            objc::send2(
                objc::class("NSDictionary"),
                "dictionaryWithObject:forKey:",
                number,
                AVAudioSessionInterruptionTypeKey,
            )
        }
    }

    fn interruption(label: &str, ty: usize) {
        // SAFETY: AVFoundation's constant.
        let name = unsafe { AVAudioSessionInterruptionNotification };
        post(name, shared_session(), interruption_info(ty));
        t(format!(
            "{label}: posted AVAudioSessionInterruptionNotification type={}",
            if ty == BEGAN { "began" } else { "ended" }
        ));
    }

    fn set_hint(name: &str, value: Option<&str>) {
        match value {
            Some(v) => {
                hints::set(name, v).unwrap();
            }
            None => {
                hints::reset(name);
            }
        }
    }

    fn application_state() -> isize {
        // SAFETY: a UIApplication property.
        unsafe { objc::send0(objc::shared_application(), "applicationState") }
    }

    pub(super) fn main() {
        elapsed();
        let status = sdl3::app::run_app(|_args| {
            let status = checked(main_function);
            exit(status);
        });
        println!("uikit_audio: run_app returned {status}, which it never does");
        exit(1);
    }

    fn main_function() {
        println!("uikit_audio: {:.3} s: main", elapsed());
        let end = Instant::now() + Duration::from_secs(30);
        while application_state() != 0 && Instant::now() < end {
            run_loop(10);
        }
        t(format!(
            "app state: {}",
            if application_state() == 0 {
                "active"
            } else {
                "not active"
            }
        ));
        session("start");

        let r = init::init(InitFlags::AUDIO);
        t(format!(
            "init audio: {}",
            match &r {
                Ok(()) => "ok".to_string(),
                Err(e) => e.message().to_string(),
            }
        ));
        t(format!(
            "audio driver: {}",
            audio::current_audio_driver().unwrap_or("(none)")
        ));
        let ids = audio::playback_devices().unwrap_or_default();
        for (i, &id) in ids.iter().enumerate() {
            t(format!(
                "playback device {i}: {}",
                audio::audio_device_name(id).unwrap()
            ));
        }
        t(format!("playback devices: {}", ids.len()));
        let ids = audio::recording_devices().unwrap_or_default();
        for (i, &id) in ids.iter().enumerate() {
            t(format!(
                "recording device {i}: {}",
                audio::audio_device_name(id).unwrap()
            ));
        }
        t(format!("recording devices: {}", ids.len()));
        t(format!(
            "default playback: {}",
            audio::audio_device_name(audio::AUDIO_DEVICE_DEFAULT_PLAYBACK).unwrap()
        ));
        t(format!(
            "default recording: {}",
            audio::audio_device_name(audio::AUDIO_DEVICE_DEFAULT_RECORDING).unwrap()
        ));
        format_of("default playback", audio::AUDIO_DEVICE_DEFAULT_PLAYBACK);
        format_of("default recording", audio::AUDIO_DEVICE_DEFAULT_RECORDING);
        events("init", 300);
        session("after init");

        // The category for each hint value and duck setting, opening the
        // default playback device
        let cases: &[(Option<&str>, Option<&str>)] = &[
            (None, None),
            (None, Some("1")),
            (Some("ambient"), None),
            (Some("ambient"), Some("1")),
            (Some("AVAudioSessionCategoryAmbient"), None),
            (Some("AMBIENT"), None),
            (Some("AVAudioSessionCategorySoloAmbient"), None),
            (Some("AVAudioSessionCategorySoloAmbient"), Some("1")),
            (Some("playback"), None),
            (Some("PlayBack"), Some("1")),
            (Some("AVAudioSessionCategoryPlayback"), Some("0")),
            (Some("playandrecord"), None),
            (Some("playandrecord"), Some("1")),
            (Some("AVAudioSessionCategoryPlayAndRecord"), None),
            (Some("record"), None),
            (Some("bogus"), Some("1")),
            (Some(""), None),
            (None, None),
        ];
        for (i, &(category, duck)) in cases.iter().enumerate() {
            let label = format!(
                "case {i} (category={} duck={})",
                category.unwrap_or("unset"),
                duck.unwrap_or("unset")
            );
            set_hint(hints::AUDIO_CATEGORY, category);
            set_hint(hints::AUDIO_DUCK_OTHERS, duck);
            let d = open_dev(&label, audio::AUDIO_DEVICE_DEFAULT_PLAYBACK);
            session(&label);
            if d != 0 {
                audio::close_audio_device(d);
                t(format!("{label}: closed"));
                session(&label);
            }
        }
        set_hint(hints::AUDIO_CATEGORY, None);
        set_hint(hints::AUDIO_DUCK_OTHERS, None);
        events("categories", 100);

        // Recording
        let rec = open_dev("recording", audio::AUDIO_DEVICE_DEFAULT_RECORDING);
        session("recording");
        if rec != 0 {
            count_callbacks(rec, 1);
            advancing("recording", 0, rec);
            audio::close_audio_device(rec);
            t("recording: closed");
            session("recording closed");
        }
        let play = open_dev("both: playback", audio::AUDIO_DEVICE_DEFAULT_PLAYBACK);
        session("both: playback");
        let rec = open_dev("both: recording", audio::AUDIO_DEVICE_DEFAULT_RECORDING);
        session("both");
        if play != 0 && rec != 0 {
            count_callbacks(play, 0);
            count_callbacks(rec, 1);
            advancing("both", play, rec);
            interruption("both", BEGAN);
            advancing("both: began", play, rec);
            interruption("both", ENDED);
            advancing("both: ended", play, rec);
        }
        if rec != 0 {
            audio::close_audio_device(rec);
            t("both: recording closed");
            session("both: recording closed");
        }
        if play != 0 {
            audio::close_audio_device(play);
            t("both: playback closed");
            session("both: playback closed");
        }
        set_hint(hints::AUDIO_CATEGORY, Some("playback"));
        let rec = open_dev(
            "recording in playback",
            audio::AUDIO_DEVICE_DEFAULT_RECORDING,
        );
        session("recording in playback");
        if rec != 0 {
            count_callbacks(rec, 1);
            advancing("recording in playback", 0, rec);
            audio::close_audio_device(rec);
            t("recording in playback: closed");
        }
        set_hint(hints::AUDIO_CATEGORY, None);
        events("recording", 100);

        // Interruptions
        let play = open_dev("interruptions", audio::AUDIO_DEVICE_DEFAULT_PLAYBACK);
        session("interruptions");
        if play != 0 {
            count_callbacks(play, 0);
            advancing("playing", play, 0);
            interruption("began", BEGAN);
            advancing("began", play, 0);
            t(format!(
                "began: device paused={}",
                yes_no(audio::audio_device_paused(play))
            ));
            interruption("began twice", BEGAN);
            advancing("began twice", play, 0);
            interruption("ended", ENDED);
            advancing("ended", play, 0);
            interruption("ended twice", ENDED);
            advancing("ended twice", play, 0);

            interruption("before active", BEGAN);
            advancing("before active", play, 0);
            // SAFETY: UIKit's constant.
            post(
                unsafe { objc::UIApplicationDidBecomeActiveNotification },
                objc::shared_application(),
                std::ptr::null_mut(),
            );
            t("posted UIApplicationDidBecomeActiveNotification");
            advancing("became active", play, 0);

            interruption("before foreground", BEGAN);
            advancing("before foreground", play, 0);
            // SAFETY: as above.
            post(
                unsafe { objc::UIApplicationWillEnterForegroundNotification },
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            t("posted UIApplicationWillEnterForegroundNotification (no object)");
            advancing("will enter foreground", play, 0);

            interruption("before no type", BEGAN);
            advancing("before no type", play, 0);
            // SAFETY: AVFoundation's constant.
            post(
                unsafe { AVAudioSessionInterruptionNotification },
                shared_session(),
                std::ptr::null_mut(),
            );
            t("posted AVAudioSessionInterruptionNotification without a type");
            advancing("no type", play, 0);

            // SAFETY: as above.
            post(
                unsafe { AVAudioSessionInterruptionNotification },
                std::ptr::null_mut(),
                interruption_info(BEGAN),
            );
            t("posted AVAudioSessionInterruptionNotification type=began without the session");
            advancing("began without the session", play, 0);

            audio::pause_audio_device(play).unwrap();
            t(format!(
                "paused: {}",
                yes_no(audio::audio_device_paused(play))
            ));
            advancing("paused", play, 0);
            interruption("paused", BEGAN);
            interruption("paused", ENDED);
            advancing("paused, interrupted and ended", play, 0);
            audio::resume_audio_device(play).unwrap();
            t(format!(
                "resumed: paused={}",
                yes_no(audio::audio_device_paused(play))
            ));
            advancing("resumed", play, 0);

            interruption("before close", BEGAN);
            advancing("before close", play, 0);
            audio::close_audio_device(play);
            t("closed while interrupted");
            session("closed while interrupted");
            interruption("after close", ENDED);
            let play = open_dev("reopened", audio::AUDIO_DEVICE_DEFAULT_PLAYBACK);
            if play != 0 {
                count_callbacks(play, 0);
                advancing("reopened", play, 0);
                interruption("reopened", BEGAN);
                advancing("reopened: began", play, 0);
                interruption("reopened", ENDED);
                advancing("reopened: ended", play, 0);
                audio::close_audio_device(play);
                t("reopened: closed");
            }
        }
        events("interruptions", 100);

        // Samples reaching the device: a stream's half second of a tone
        let play = open_dev("stream", audio::AUDIO_DEVICE_DEFAULT_PLAYBACK);
        if play != 0 {
            let (spec, _) = audio::audio_device_format(play).unwrap();
            let src = AudioSpec::new(AudioFormat::F32, 1, 48000);
            let stream = AudioStream::new(Some(&src), Some(&spec)).unwrap();
            let frames = 48000 / 2;
            let tone: Vec<u8> = (0..frames)
                .flat_map(|i| {
                    (0.05f32 * (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / 48000.0).sin())
                        .to_ne_bytes()
                })
                .collect();
            stream.put_data(&tone).unwrap();
            stream.flush();
            audio::bind_audio_stream(play, &stream).unwrap();
            let start = Instant::now();
            while stream.queued() > 0 && start.elapsed() < Duration::from_secs(5) {
                run_loop(10);
            }
            println!(
                "uikit_audio: info: half a second played in {} ms",
                start.elapsed().as_millis()
            );
            t(format!("stream: played: {}", yes_no(stream.queued() == 0)));
            drop(stream);
            audio::close_audio_device(play);
            t("stream: closed");
        }
        session("before quit");
        init::quit_subsystem(InitFlags::AUDIO);
        t("quit audio");
        session("after quit");
        events("quit", 100);
        init::quit();
        t("done");

        check_trace(&TRACE.lock().unwrap(), EXPECTED);
    }
}
