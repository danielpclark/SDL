// The Android audio drivers in an app: android_audio.rs's and
// android_audio_opensles.rs's program, against upstream's C.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The program traces, with the audio subsystem alone (whose event pump
//! handles the activity's lifecycle on Android whatever the video
//! subsystem):
//!
//! * the drivers, the one that came up, its devices (the default ones
//!   only: upstream's `ALLOW_MULTIPLE_ANDROID_AUDIO_DEVICES` is 0) and
//!   their formats, and the events that add them;
//! * the audio manager's device callback registered and unregistered from
//!   Java (`SDLAudioManager.registerAudioDeviceCallback()`, through the
//!   activity's class loader), which passes each of the system's devices to
//!   `nativeAddAudioDevice()`: those natives do nothing then (upstream
//!   never registers it), so the devices and events don't change (the
//!   emulator has no way to plug a device in from outside);
//! * the default playback device opened: its format, its thread's name
//!   and priority (`audioSetThreadPriority()`'s, read in its first postmix
//!   callback: `/proc`'s `comm` and `getpriority()`), whether it advances
//!   (its postmix callbacks in half a second);
//! * a stream's tone (S16, mono, 22050 Hz) played through it;
//! * the default recording device without the `RECORD_AUDIO` permission
//!   (the system's dialog, answered "deny" by the runner from a thread of
//!   the program's while SDL's waits in `SDL_OpenAudioDevice()`), then
//!   with it (`pm grant`): its format, its thread, whether it advances;
//! * both devices open through a trip to the background (the HOME key,
//!   then the activity launched again), made by a thread of the program's
//!   while SDL's main thread pumps the events, which blocks while the app
//!   is paused (`SDL_HINT_ANDROID_BLOCK_ON_PAUSE`): whether each device
//!   advances before, a second after the app entered the background (the
//!   pump paused AAudio's streams, OpenSL ES's player but not its
//!   recorder), and back; and the application's events of the trip;
//! * the same with the pump not blocking (audio initialized again with the
//!   hint off), the playback device only, which isn't paused.
//!
//! What depends on timing isn't traced: the counts of callbacks (printed
//! as `android_audio: info:` lines), the application's events around the
//! permission's dialog (the activity's pause and resume cancel out in
//! the lifecycle queue unless a pump comes between), and `LOW_MEMORY`
//! (the memory trimmed once the app is hidden).

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sdl3::audio::{self, AudioDeviceID, AudioFormat, AudioSpec};
use sdl3::events::{self, Event, EventType};
use sdl3::hints;
use sdl3::init::{self, InitFlags};

use super::android_support::events::{ordered, pump_for, saw, take, unwatch, watch};
use super::android_support::jni::{self, Jvalue};
use super::android_support::{check_trace, event_name, os_thread_name, request};

static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn t(line: impl Into<String>) {
    let line = line.into();
    println!("trace: {line}");
    TRACE.lock().unwrap().push(line);
}

fn yes(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

fn result<T>(r: &sdl3::Result<T>) -> String {
    match r {
        Ok(_) => "ok".to_string(),
        Err(e) => format!("error: {}", e.message()),
    }
}

/// Each line of a request's answer, traced as `STEP: LINE`.
fn trace_answer(step: &str, answer: &[String]) {
    for line in answer {
        t(format!("{step}: {line}"));
    }
}

/// The physical devices' IDs, for the trace's names.
static PLAYBACK_ID: AtomicU32 = AtomicU32::new(0);
static RECORDING_ID: AtomicU32 = AtomicU32::new(0);

/// A device ID as the trace writes it.
fn id_name(id: AudioDeviceID) -> &'static str {
    if id == PLAYBACK_ID.load(Ordering::SeqCst) {
        "(playback device)"
    } else if id == RECORDING_ID.load(Ordering::SeqCst) {
        "(recording device)"
    } else {
        "(other device)"
    }
}

/// The audio device events since the last call (pumped).
fn trace_device_events(step: &str) {
    let mut seen = Vec::new();
    events::pump();
    while let Some(event) = events::poll() {
        if let Event::AudioDevice(e) = event {
            let name = audio::audio_device_name(e.which).unwrap_or_else(|_| "?".into());
            seen.push(format!(
                "{}({name}, recording={})",
                event_name(e.event_type),
                e.recording as i32
            ));
        }
    }
    t(format!(
        "{step}: events: {}",
        if seen.is_empty() {
            "(none)".to_string()
        } else {
            seen.join(", ")
        }
    ));
}

fn trace_devices(step: &str) {
    let ids = audio::playback_devices().unwrap_or_default();
    for (i, id) in ids.iter().enumerate() {
        t(format!(
            "{step}: playback device {i}: {}",
            audio::audio_device_name(*id).unwrap_or_default()
        ));
    }
    t(format!("{step}: playback devices: {}", ids.len()));
    let ids = audio::recording_devices().unwrap_or_default();
    for (i, id) in ids.iter().enumerate() {
        t(format!(
            "{step}: recording device {i}: {}",
            audio::audio_device_name(*id).unwrap_or_default()
        ));
    }
    t(format!("{step}: recording devices: {}", ids.len()));
}

fn trace_format(step: &str, id: AudioDeviceID) {
    match audio::audio_device_format(id) {
        Ok((spec, frames)) => t(format!(
            "{step}: format={:#x} channels={} freq={} frames={frames}",
            spec.format.0, spec.channels, spec.freq
        )),
        Err(e) => t(format!("{step}: format: error: {}", e.message())),
    }
}

/// The postmix callbacks' counts (playback, recording), and each device
/// thread's name and priority (its first callback's).
static COUNTS: [AtomicI32; 2] = [AtomicI32::new(0), AtomicI32::new(0)];
static THREAD_INFO: [Mutex<String>; 2] = [Mutex::new(String::new()), Mutex::new(String::new())];
static THREAD_SEEN: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];

fn postmix(which: usize) {
    if !THREAD_SEEN[which].load(Ordering::SeqCst) {
        let mut comm = os_thread_name();
        let (letter, id) = if which == 1 {
            ('C', RECORDING_ID.load(Ordering::SeqCst))
        } else {
            ('P', PLAYBACK_ID.load(Ordering::SeqCst))
        };
        if comm == format!("SDLAudio{letter}{}", id as i32) {
            comm = format!("SDLAudio{letter}{}", id_name(id));
        }
        // SAFETY: no arguments.
        let tid = unsafe { libc::gettid() };
        // SAFETY: this thread's ID.
        let nice = unsafe { libc::getpriority(libc::PRIO_PROCESS, tid as libc::id_t) };
        *THREAD_INFO[which].lock().unwrap() = format!("name={comm} nice={nice}");
        THREAD_SEEN[which].store(true, Ordering::SeqCst);
    }
    COUNTS[which].fetch_add(1, Ordering::SeqCst);
}

fn count_callbacks(dev: AudioDeviceID, which: usize) {
    THREAD_SEEN[which].store(false, Ordering::SeqCst);
    audio::set_audio_postmix_callback(
        dev,
        Some(move |_: &AudioSpec, _: &mut [f32]| postmix(which)),
    )
    .unwrap();
}

/// A wait of `ms`, pumping on SDL's main thread, sleeping on another.
static ON_MAIN: AtomicBool = AtomicBool::new(true);

fn wait_ms(ms: u64) {
    if ON_MAIN.load(Ordering::SeqCst) {
        pump_for(ms);
    } else {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

/// Whether each device's postmix callback ran in half a second (after a
/// quarter of a second to settle).
fn advancing(step: &str, play: bool, rec: bool) {
    wait_ms(250);
    let p0 = COUNTS[0].load(Ordering::SeqCst);
    let r0 = COUNTS[1].load(Ordering::SeqCst);
    wait_ms(500);
    let p1 = COUNTS[0].load(Ordering::SeqCst);
    let r1 = COUNTS[1].load(Ordering::SeqCst);
    println!(
        "android_audio: info: {step}: playback callbacks {}, recording callbacks {}",
        p1 - p0,
        r1 - r0
    );
    if play && rec {
        t(format!(
            "{step}: playback advanced={} recording advanced={}",
            yes(p1 > p0),
            yes(r1 > r0)
        ));
    } else if rec {
        t(format!("{step}: recording advanced={}", yes(r1 > r0)));
    } else {
        t(format!("{step}: playback advanced={}", yes(p1 > p0)));
    }
}

fn trace_thread(step: &str, which: usize) {
    let info = if THREAD_SEEN[which].load(Ordering::SeqCst) {
        THREAD_INFO[which].lock().unwrap().clone()
    } else {
        "(no callback)".to_string()
    };
    t(format!("{step}: thread: {info}"));
}

fn open_dev(step: &str, which: AudioDeviceID) -> AudioDeviceID {
    match audio::open_audio_device(which, None) {
        Err(e) => {
            t(format!("{step}: open: error: {}", e.message()));
            0
        }
        Ok(dev) => {
            t(format!("{step}: open: ok"));
            trace_format(step, dev);
            dev
        }
    }
}

/// SDLAudioManager's static method `name` ()V, called through JNI (its
/// class from the activity's class loader).
fn call_audio_manager(step: &str, name: &str) {
    let env = sdl3::android::jni_env()
        .expect("a JNI environment")
        .as_ptr();
    let activity = sdl3::android::activity().expect("the activity").as_ptr();
    let activity_class = jni::object_class(env, activity);
    let loader = jni::call_object(
        env,
        activity_class,
        "getClassLoader",
        "()Ljava/lang/ClassLoader;",
        &[],
    );
    let cname = jni::new_string(env, "org.libsdl.app.SDLAudioManager");
    let manager = jni::call_object(
        env,
        loader,
        "loadClass",
        "(Ljava/lang/String;)Ljava/lang/Class;",
        &[Jvalue { l: cname }],
    );
    jni::call_static_void(env, manager, name, "()V", &[]);
    t(format!("{step}: {name}(): ok"));
    for r in [manager, cname, loader, activity_class, activity] {
        jni::delete_local_ref(env, r);
    }
}

/// A trip to the background and back, made by a thread of its own while
/// SDL's main thread pumps (blocking while the app is paused, with the
/// block on pause): how the devices advance before, a second after the
/// app entered the background, and back.
fn trip_thread(name: &str, play: bool, rec: bool) {
    advancing(&format!("{name}: before"), play, rec);
    request("background");
    let end = Instant::now() + Duration::from_secs(10);
    while !saw(EventType::DID_ENTER_BACKGROUND) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    t(format!(
        "{name}: in the background: {}",
        yes(saw(EventType::DID_ENTER_BACKGROUND))
    ));
    std::thread::sleep(Duration::from_millis(1000));
    advancing(&format!("{name}: in the background"), play, rec);
    request("foreground");
    let end = Instant::now() + Duration::from_secs(30);
    while !saw(EventType::DID_ENTER_FOREGROUND) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    t(format!(
        "{name}: back: {}",
        yes(saw(EventType::DID_ENTER_FOREGROUND))
    ));
    advancing(&format!("{name}: back"), play, rec);
}

fn is_app_event_but_low_memory(ty: EventType) -> bool {
    (0x100..0x150).contains(&ty.0) && ty != EventType::LOW_MEMORY
}

fn audio_trip(name: &'static str, play: bool, rec: bool) {
    take();
    ON_MAIN.store(false, Ordering::SeqCst);
    let done = Arc::new(AtomicBool::new(false));
    let d = done.clone();
    let thread = std::thread::Builder::new()
        .name("trip".into())
        .spawn(move || {
            trip_thread(name, play, rec);
            d.store(true, Ordering::SeqCst);
        })
        .unwrap();
    while !done.load(Ordering::SeqCst) {
        events::pump();
        events::flush_events(EventType::FIRST, EventType::LAST);
        std::thread::sleep(Duration::from_millis(10));
    }
    thread.join().expect("the trip's thread");
    ON_MAIN.store(true, Ordering::SeqCst);
    pump_for(500);
    // (the memory trimmed once the app is hidden: LOW_MEMORY, or not, by timing)
    t(format!(
        "{name}: app events: {}",
        ordered(&take(), is_app_event_but_low_memory)
    ));
}

fn phys_ids() {
    let ids = audio::playback_devices().unwrap_or_default();
    PLAYBACK_ID.store(ids.first().copied().unwrap_or(0), Ordering::SeqCst);
    let ids = audio::recording_devices().unwrap_or_default();
    RECORDING_ID.store(ids.first().copied().unwrap_or(0), Ordering::SeqCst);
}

fn init_audio(step: &str) {
    t(format!("{step}: {}", result(&init::init(InitFlags::AUDIO))));
    t(format!(
        "{step}: audio driver: {}",
        audio::current_audio_driver().unwrap_or("(none)")
    ));
    phys_ids();
}

/// The program, with `SDL_HINT_AUDIO_DRIVER` set to `driver_hint` when
/// there's one; its trace compared with `expected`, upstream's.
pub(super) fn main(driver_hint: Option<&str>, expected: &str) {
    ON_MAIN.store(true, Ordering::SeqCst);
    if let Some(hint) = driver_hint {
        hints::set(hints::AUDIO_DRIVER, hint).unwrap();
    }
    let drivers: Vec<&str> = (0..audio::num_audio_drivers())
        .map(|i| audio::audio_driver(i).unwrap_or_default())
        .collect();
    t(format!("drivers: {}", drivers.join(", ")));
    init_audio("init");
    watch();
    trace_devices("init");
    trace_format("default playback", audio::AUDIO_DEVICE_DEFAULT_PLAYBACK);
    trace_format("default recording", audio::AUDIO_DEVICE_DEFAULT_RECORDING);
    trace_device_events("init");

    // The audio manager's device callback (SDL doesn't register it with
    // ALLOW_MULTIPLE_ANDROID_AUDIO_DEVICES 0: its natives do nothing)
    call_audio_manager("hotplug", "registerAudioDeviceCallback");
    pump_for(1000);
    trace_devices("hotplug");
    trace_device_events("hotplug");
    call_audio_manager("hotplug", "unregisterAudioDeviceCallback");

    // Playback
    let play = open_dev("playback", audio::AUDIO_DEVICE_DEFAULT_PLAYBACK);
    if play != 0 {
        count_callbacks(play, 0);
        advancing("playback", true, false);
        trace_thread("playback", 0);
        audio::close_audio_device(play);
        t("playback: closed");
    }

    // A stream's tone
    {
        let tone = AudioSpec {
            format: AudioFormat::S16,
            channels: 1,
            freq: 22050,
        };
        match audio::open_audio_device_stream(
            audio::AUDIO_DEVICE_DEFAULT_PLAYBACK,
            Some(&tone),
            None::<fn(&audio::AudioStream, i32, i32)>,
        ) {
            Err(e) => t(format!("stream: open: error: {}", e.message())),
            Ok(stream) => {
                t("stream: open: ok");
                trace_format("stream", stream.device().unwrap_or(0));
                let n = tone.freq / 4;
                let samples: Vec<u8> = (0..n)
                    .flat_map(|i| {
                        let s = (i as f32 / tone.freq as f32 * 440.0 * 2.0 * std::f32::consts::PI)
                            .sin();
                        ((s * 8000.0) as i16).to_ne_bytes()
                    })
                    .collect();
                let _ = stream.put_data(&samples);
                let _ = stream.resume_device();
                let end = Instant::now() + Duration::from_secs(5);
                while stream.queued() > 0 && Instant::now() < end {
                    pump_for(10);
                }
                t(format!("stream: played: {}", yes(stream.queued() == 0)));
                drop(stream);
                t("stream: closed");
            }
        }
    }

    // Recording: the permission denied (the system's dialog)
    {
        let thread = std::thread::Builder::new()
            .name("ask".into())
            .spawn(|| request("permission-dialog deny"))
            .unwrap();
        let rec = open_dev("denied", audio::AUDIO_DEVICE_DEFAULT_RECORDING);
        let answer = thread.join().expect("the request's thread");
        trace_answer("denied", &answer);
        if rec != 0 {
            audio::close_audio_device(rec);
        }
        // (the activity's pause and resume around the dialog cancel out
        // in the lifecycle queue unless a pump came between: not traced)
        pump_for(1500);
        take();
    }

    // Recording: granted
    trace_answer("granted", &request("permission grant RECORD_AUDIO"));
    let rec = open_dev("recording", audio::AUDIO_DEVICE_DEFAULT_RECORDING);
    if rec != 0 {
        count_callbacks(rec, 1);
        advancing("recording", false, true);
        trace_thread("recording", 1);
    }

    // Both, and a trip to the background with the pump blocking on pause
    let play = open_dev("both", audio::AUDIO_DEVICE_DEFAULT_PLAYBACK);
    if play != 0 {
        count_callbacks(play, 0);
    }
    audio_trip("blocking trip", play != 0, rec != 0);
    if play != 0 {
        audio::close_audio_device(play);
    }
    if rec != 0 {
        audio::close_audio_device(rec);
    }
    t("both: closed");
    trace_device_events("both");

    // Again with the pump not blocking: nothing paused
    unwatch();
    init::quit();
    t("quit");
    hints::set(hints::ANDROID_BLOCK_ON_PAUSE, "0").unwrap();
    init_audio("init again");
    watch();
    let play = open_dev("not blocking", audio::AUDIO_DEVICE_DEFAULT_PLAYBACK);
    if play != 0 {
        count_callbacks(play, 0);
    }
    audio_trip("non-blocking trip", true, false);
    if play != 0 {
        audio::close_audio_device(play);
    }
    t("not blocking: closed");
    unwatch();
    init::quit();
    t("main returns");
    let trace = TRACE.lock().unwrap().clone();
    check_trace(&trace, expected);
}
