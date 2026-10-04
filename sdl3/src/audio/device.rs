// Rust translation of src/audio/SDL_audio.c (and the device parts of
// src/audio/SDL_sysaudio.h) from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Audio devices: physical devices, the logical devices opened on them,
//! stream binding, the device threads and the driver interface.
//!
//! Lock order, as upstream: the subsystem lock is never held while taking a
//! device lock; then physical device → logical device → audio stream.
//! Device and logical device locks are recursive, so callbacks running on
//! a device thread may call back into the audio API.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak};
use std::time::Duration;

use super::convert::{convert_audio, ConvertSrc};
use super::format::{audio_channel_maps_equal, audio_specs_equal, AudioFormat, AudioSpec};
use super::queue::{chmap_ref, ChannelMap};
use super::stream::{set_audio_stream_channel_map, AudioStream, AudioStreamCallback, StreamInner};
use super::{drivers, resample};
use crate::error::{Error, Result};
use crate::events::{AudioDeviceEvent, AudioDeviceID, Event, EventType};
use crate::hints;
use crate::properties::Properties;
use crate::thread::{Condition, ReentrantMutex, ReentrantMutexGuard, Thread, ThreadPriority};

/// A value used to request a default playback audio device.
/// Translation of `SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK`.
///
/// Several functions that require an [`AudioDeviceID`] will accept this
/// value to signify the app just wants the system to choose a default
/// device instead of the app providing a specific one.
pub const AUDIO_DEVICE_DEFAULT_PLAYBACK: AudioDeviceID = 0xFFFF_FFFF;

/// A value used to request a default recording audio device.
/// Translation of `SDL_AUDIO_DEVICE_DEFAULT_RECORDING`.
pub const AUDIO_DEVICE_DEFAULT_RECORDING: AudioDeviceID = 0xFFFF_FFFE;

/// The device's unique, persistent, backend-specific id, if it has one.
/// Translation of `SDL_PROP_AUDIO_DEVICE_UNIQUE_ID_STRING`.
pub const PROP_AUDIO_DEVICE_UNIQUE_ID_STRING: &str = "SDL.audio.device.unique_id";

// !!! FIXME: These are wordy and unlocalized...
pub(crate) const DEFAULT_PLAYBACK_DEVNAME: &str = "System audio playback device";
pub(crate) const DEFAULT_RECORDING_DEVNAME: &str = "System audio recording device";

// these are used when no better specifics are known. We default to CD audio quality.
const DEFAULT_AUDIO_PLAYBACK_FORMAT: AudioFormat = AudioFormat::S16;
const DEFAULT_AUDIO_PLAYBACK_CHANNELS: i32 = 2;
#[cfg(target_os = "nto")]
const DEFAULT_AUDIO_PLAYBACK_FREQUENCY: i32 = 48000;
#[cfg(not(target_os = "nto"))]
const DEFAULT_AUDIO_PLAYBACK_FREQUENCY: i32 = 44100;

const DEFAULT_AUDIO_RECORDING_FORMAT: AudioFormat = AudioFormat::S16;
const DEFAULT_AUDIO_RECORDING_CHANNELS: i32 = 1;
const DEFAULT_AUDIO_RECORDING_FREQUENCY: i32 = 44100;

/// A callback that fires when data is about to be fed to an audio device.
/// Translation of `SDL_AudioPostmixCallback`.
///
/// This is useful for accessing the final mix, perhaps for writing a
/// visualizer or applying a final effect to the audio data before playback.
/// It receives the device's spec (always in `F32` format) and the samples,
/// which it may change. It runs on the device thread, with the device
/// locked.
pub type AudioPostmixCallback = Arc<dyn Fn(&AudioSpec, &mut [f32]) + Send + Sync>;

/// Flags that push duplicate code into the core and reduce `#ifdef`s
/// (the flag fields of `SDL_AudioDriverImpl`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DriverFlags {
    /// !!! FIXME: rename this, it's not a callback thread anymore.
    pub(crate) provides_own_callback_thread: bool,
    pub(crate) has_recording_support: bool,
    pub(crate) only_has_default_playback_device: bool,
    /// !!! FIXME: is there ever a time where you'd have a default playback and not a default recording (or vice versa)?
    pub(crate) only_has_default_recording_device: bool,
}

/// A backend's interface. Translation of the driver-wide entry points of
/// `SDL_AudioDriverImpl`; every method has the default stub of
/// `CompleteAudioEntryPoints()`.
pub(crate) trait AudioDriverImpl: Send + Sync {
    fn flags(&self) -> DriverFlags;

    /// Translation of `DetectDevices`; the default adds one default device
    /// of each kind (`SDL_AudioDetectDevices_Default()`).
    fn detect_devices(&self) -> (Option<Arc<PhysicalDevice>>, Option<Arc<PhysicalDevice>>) {
        let flags = self.flags();
        // you have to write your own implementation if these assertions fail.
        crate::sdl_assert!(flags.only_has_default_playback_device);
        crate::sdl_assert!(flags.only_has_default_recording_device || !flags.has_recording_support);

        let default_playback = add_audio_device(false, DEFAULT_PLAYBACK_DEVNAME, None, None, 0x1);
        let default_recording = if flags.has_recording_support {
            add_audio_device(true, DEFAULT_RECORDING_DEVNAME, None, None, 0x2)
        } else {
            None
        };
        (default_playback, default_recording)
    }

    /// Translation of `OpenDevice`: open the device at the format in
    /// `state` (which the backend may change) and return its per-device
    /// interface.
    fn open_device(
        &self,
        _device: &PhysicalDevice,
        _state: &mut PhysState,
    ) -> Result<Arc<dyn DeviceBackend>> {
        Err(Error::unsupported())
    }

    /// Translation of `ThreadInit`: called by audio thread at start.
    fn thread_init(&self, device: &PhysicalDevice) {
        let _ = crate::thread::set_current_thread_priority(if device.recording {
            ThreadPriority::High
        } else {
            ThreadPriority::TimeCritical
        });
    }

    /// Translation of `ThreadDeinit`: called by audio thread at end.
    fn thread_deinit(&self, _device: &PhysicalDevice) {}

    /// Translation of `FreeDeviceHandle`: SDL is done with this device;
    /// free the handle from `add_audio_device`.
    fn free_device_handle(&self, _device: &PhysicalDevice) {}

    /// Translation of `DeinitializeStart`: SDL calls this, then starts
    /// destroying objects, then calls `deinitialize`. This is a good place
    /// to stop hotplug detection.
    fn deinitialize_start(&self) {}

    /// Translation of `Deinitialize`.
    fn deinitialize(&self) {}
}

/// The per-device entry points of `SDL_AudioDriverImpl` (and the
/// backend's `SDL_PrivateAudioData`), for an opened device. Every method
/// has the default stub of `CompleteAudioEntryPoints()`.
///
/// `wait_*` run on the device thread without the device lock; everything
/// else with it held. The core owns the buffer passed to `play_device`.
pub(crate) trait DeviceBackend: Send + Sync {
    /// Translation of `WaitDevice`.
    fn wait_device(&self, _device: &PhysicalDevice) -> bool {
        true
    }

    /// Translation of `GetDeviceBuf`: how many bytes (at most
    /// `buffer_size`) to produce this iteration; 0 abandons the iteration,
    /// `None` is a failure.
    fn get_device_buf(&self, _device: &PhysicalDevice, _buffer_size: usize) -> Option<usize> {
        Some(0)
    }

    /// Translation of `PlayDevice`. SHOULD NOT BLOCK, as the device is
    /// locked. Block in `wait_device` instead!
    fn play_device(&self, _device: &PhysicalDevice, _buffer: &[u8]) -> bool {
        true
    }

    /// Translation of `WaitRecordingDevice`.
    fn wait_recording_device(&self, _device: &PhysicalDevice) -> bool {
        true
    }

    /// Translation of `RecordDevice`: fill `buffer`, returning the bytes
    /// recorded.
    fn record_device(&self, _device: &PhysicalDevice, _buffer: &mut [u8]) -> Result<usize> {
        Err(Error::unsupported())
    }

    /// Translation of `FlushRecording`.
    fn flush_recording(&self, _device: &PhysicalDevice) {}

    /// Translation of `CloseDevice`.
    fn close_device(&self, _device: &PhysicalDevice) {}
}

/// Translation of `AudioBootStrap`.
pub(crate) struct AudioBootStrap {
    pub(crate) name: &'static str,
    pub(crate) desc: &'static str,
    pub(crate) init: fn() -> Option<Arc<dyn AudioDriverImpl>>,
    /// if true: request explicitly, or it won't be available.
    pub(crate) demand_only: bool,
    pub(crate) is_preferred: bool,
}

/// Available audio drivers. Translation of `bootstrap`.
///
/// (The platform drivers arrive with the platform layer; disk and dummy
/// are both demand-only, so until then audio initializes only when one of
/// them is requested with [`hints::AUDIO_DRIVER`].)
static BOOTSTRAP: &[&AudioBootStrap] = &[
    &drivers::disk::DISKAUDIO_BOOTSTRAP,
    &drivers::dummy::DUMMYAUDIO_BOOTSTRAP,
];

/// The fields of `SDL_AudioDevice` behind its `lock`.
pub(crate) struct PhysState {
    /// The device's current audio specification
    pub(crate) spec: AudioSpec,
    /// The size, in bytes, of the device's playback/recording buffer.
    pub(crate) buffer_size: usize,
    /// The device's channel map, or `None` for SDL default layout.
    pub(crate) chmap: ChannelMap,
    /// The device's default audio specification
    pub(crate) default_spec: AudioSpec,
    /// Number of sample frames the devices wants per-buffer.
    pub(crate) sample_frames: i32,
    /// Value to use for memset to silence a buffer in this device's format
    pub(crate) silence_value: u8,
    /// true if audio thread can skip silence/mix/convert stages and just do a basic memcpy.
    simple_copy: bool,
    /// The zombie implementations replace the backend's (`SetAudioDeviceZombieFunctions()`).
    zombie_ops: bool,

    /// The buffer handed to `play_device` (what `GetDeviceBuf` returns upstream).
    device_buffer: Vec<u8>,
    // Scratch buffers used for mixing.
    work_buffer: Vec<u8>,
    mix_buffer: Vec<f32>,
    postmix_buffer: Vec<f32>,
    /// Size of work_buffer (and mix_buffer) in bytes.
    work_buffer_size: usize,

    /// A thread to feed the audio device
    thread: Option<Thread>,
    /// true if this physical device is currently opened by the backend.
    currently_opened: bool,
    /// Properties!
    props: Option<Properties>,
    /// The backend's data for an opened device (`hidden` plus its entry points).
    backend: Option<Arc<dyn DeviceBackend>>,
    /// All logical devices associated with this physical device (newest first).
    logical_devices: Vec<Arc<LogicalDevice>>,
}

type DeviceGuard<'a> = ReentrantMutexGuard<'a, RefCell<PhysState>>;

/// A piece of physical hardware, whether it is in use or not.
/// Translation of `SDL_AudioDevice`.
///
/// These objects exist as long as the system-level device is available.
/// Physical devices get destroyed for three reasons:
///  - They were lost to the system (a USB cable is kicked out, etc).
///  - They failed for some other unlikely reason at the API level (which is _also_ probably a USB cable being kicked out).
///  - We are shutting down, so all allocated resources are being freed.
///
/// They are _not_ destroyed because we are done using them (when we "close" a playing device).
pub(crate) struct PhysicalDevice {
    /// A mutex for locking access to this struct
    lock: ReentrantMutex<RefCell<PhysState>>,
    /// A condition variable to protect device close, where we can't hold the device lock forever.
    close_cond: Condition,
    /// Reference count of the device; logical devices, device threads, etc, add to this.
    refcount: AtomicI32,
    /// human-readable name of the device. ("SoundBlaster Pro 16")
    pub(crate) name: String,
    /// unique, platform-specific, backend-specific string to identify this specific device.
    unique_id: Option<String>,
    /// the unique instance ID of this device.
    pub(crate) instance_id: AudioDeviceID,
    /// a way for the backend to identify this device _when not opened_
    pub(crate) handle: usize,
    /// true if this is a recording device instead of an playback device
    pub(crate) recording: bool,
    /// non-zero if we are signaling the audio thread to end.
    shutdown: AtomicI32,
    /// non-zero if this was a disconnected device and we're waiting for it to be decommissioned.
    zombie: AtomicI32,
}

/// The device's `silence_value`, for backends (called with the device
/// locked and its state not borrowed).
pub(crate) fn silence_value_of(device: &PhysicalDevice) -> u8 {
    device.lock().borrow().silence_value
}

impl PhysicalDevice {
    fn lock(&self) -> DeviceGuard<'_> {
        self.lock.lock()
    }

    /// Whether the audio thread is being asked to end.
    pub(crate) fn shutting_down(&self) -> bool {
        self.shutdown.load(Ordering::Acquire) != 0
    }
}

/// The fields of `SDL_LogicalAudioDevice` that upstream guards with the
/// physical device's lock.
pub(crate) struct LogicalState {
    /// Volume of the device output.
    gain: f32,
    /// all audio streams currently bound to this opened device (newest first).
    pub(crate) bound_streams: Vec<Arc<StreamInner>>,
    /// true if this was opened as a default device.
    opened_as_default: bool,
    /// true if device was opened with `open_audio_device_stream` (so it forbids binding changes, etc).
    simplified: bool,
    /// If set, callback into the app that lets them access the final postmix buffer.
    postmix: Option<AudioPostmixCallback>,
    /// Properties, maybe copied from physical device.
    props: Option<Properties>,
}

/// An opened device. Translation of `SDL_LogicalAudioDevice`.
///
/// Logical devices are an abstraction in SDL3; you can open the same
/// physical device multiple times, and each will result in an object with
/// its own set of bound audio streams, etc, even though internally these
/// are all processed as a group when mixing the final output for the
/// physical device.
pub(crate) struct LogicalDevice {
    /// the unique instance ID of this device.
    pub(crate) instance_id: AudioDeviceID,
    recording: bool,
    /// If whole logical device is paused (process no streams bound to this device).
    paused: AtomicBool,
    /// The physical device associated with this opened device.
    physical_device: Mutex<Arc<PhysicalDevice>>,
    state: ReentrantMutex<RefCell<LogicalState>>,
}

impl LogicalDevice {
    /// Whether this is a recording device.
    pub(crate) fn recording(&self) -> bool {
        self.recording
    }

    /// Whether it was opened with `open_audio_device_stream`.
    pub(crate) fn simplified(&self) -> bool {
        self.state.lock().borrow().simplified
    }

    fn physical(&self) -> Arc<PhysicalDevice> {
        self.physical_device
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// Translation of `SDL_AudioDriver` (`current_audio`), behind its `subsystem_rwlock`.
#[derive(Default)]
struct CurrentAudio {
    /// The name of this audio driver
    name: Option<&'static str>,
    /// The description of this audio driver
    #[allow(dead_code)]
    desc: Option<&'static str>,
    /// the backend's interface
    driver: Option<Arc<dyn AudioDriverImpl>>,
    flags: DriverFlags,
    /// the collection of currently-available audio devices (recording and playback), for mapping AudioDeviceID to a physical device.
    device_hash_physical: Option<BTreeMap<AudioDeviceID, Arc<PhysicalDevice>>>,
    /// the collection of currently-opened logical devices.
    device_hash_logical: Option<BTreeMap<AudioDeviceID, Arc<LogicalDevice>>>,
    /// a list of all existing audio streams.
    existing_streams: Vec<Weak<StreamInner>>,
    default_playback_device_id: AudioDeviceID,
    default_recording_device_id: AudioDeviceID,
    pending_events: Vec<(EventType, AudioDeviceID)>,

    // !!! FIXME: most (all?) of these don't have to be atomic.
    playback_device_count: i32,
    recording_device_count: i32,
    /// non-zero during SDL_Quit, so we known not to accept any last-minute device hotplugs.
    shutting_down: bool,
}

static CURRENT_AUDIO: RwLock<Option<CurrentAudio>> = RwLock::new(None);

fn audio_read() -> RwLockReadGuard<'static, Option<CurrentAudio>> {
    CURRENT_AUDIO.read().unwrap_or_else(|e| e.into_inner())
}

fn audio_write() -> RwLockWriteGuard<'static, Option<CurrentAudio>> {
    CURRENT_AUDIO.write().unwrap_or_else(|e| e.into_inner())
}

/// The current driver's interface (cloned out so it's called without the
/// subsystem lock).
fn current_driver() -> Option<Arc<dyn AudioDriverImpl>> {
    audio_read().as_ref().and_then(|a| a.driver.clone())
}

fn driver_flags() -> DriverFlags {
    audio_read().as_ref().map(|a| a.flags).unwrap_or_default()
}

/// Deduplicated list of audio bootstrap drivers. Translation of `deduped_bootstrap`.
fn deduped_bootstrap() -> Vec<&'static AudioBootStrap> {
    let mut out: Vec<&'static AudioBootStrap> = Vec::new();
    // Build a list of unique audio drivers.
    for (i, b) in BOOTSTRAP.iter().enumerate() {
        if !BOOTSTRAP[..i].iter().any(|o| o.name == b.name) {
            out.push(b);
        }
    }
    out
}

/// The number of built-in audio drivers. Translation of `SDL_GetNumAudioDrivers()`.
///
/// This function returns a hardcoded number. This never returns a negative
/// value; if there are no drivers compiled into this build of SDL, this
/// function returns zero. The presence of a driver in this list does not
/// mean it will function, it just means SDL is capable of interacting with
/// that interface. For example, a build of SDL might have esound support,
/// but if there's no esound server available, SDL's esound driver would fail
/// if used.
pub fn num_audio_drivers() -> usize {
    deduped_bootstrap().len()
}

/// The name of a built in audio driver, like "alsa", "coreaudio" or
/// "wasapi". Translation of `SDL_GetAudioDriver()`.
///
/// The list of audio drivers is given in the order that they are normally
/// initialized by default; the drivers that seem more reasonable to choose
/// first (as far as the SDL developers believe) are earlier in the list.
pub fn audio_driver(index: usize) -> Result<&'static str> {
    deduped_bootstrap()
        .get(index)
        .map(|b| b.name)
        .ok_or_else(|| Error::invalid_param("index"))
}

/// The name of the current audio driver, or `None` if no driver has been
/// initialized. Translation of `SDL_GetCurrentAudioDriver()`.
pub fn current_audio_driver() -> Option<&'static str> {
    audio_read().as_ref().and_then(|a| a.name)
}

/// Backends can call this to get a reasonable default sample frame count
/// for a device's sample rate. Translation of `SDL_GetDefaultSampleFramesFromFreq()`.
pub(crate) fn get_default_sample_frames_from_freq(freq: i32) -> i32 {
    if let Some(hint) = hints::get(hints::AUDIO_DEVICE_SAMPLE_FRAMES) {
        let val = crate::stdlib::atoi(&hint);
        if val > 0 {
            return val;
        }
    }

    if freq <= 22050 {
        512
    } else if freq <= 48000 {
        1024
    } else if freq <= 96000 {
        2048
    } else {
        4096
    }
}

/// Translation of `OnAudioStreamCreated()`.
///
/// NOTE that you can create an audio stream without initializing the audio
/// subsystem, but it will not be automatically destroyed during a later
/// call to `init::quit`! You must explicitly destroy it yourself!
pub(crate) fn on_audio_stream_created(stream: &Arc<StreamInner>) {
    if let Some(audio) = audio_write().as_mut() {
        audio.existing_streams.retain(|w| w.strong_count() > 0);
        audio.existing_streams.insert(0, Arc::downgrade(stream));
    }
}

/// Translation of `OnAudioStreamDestroy()`.
pub(crate) fn on_audio_stream_destroy(stream: &Arc<StreamInner>) {
    if let Some(audio) = audio_write().as_mut() {
        audio
            .existing_streams
            .retain(|w| w.strong_count() > 0 && !std::ptr::eq(w.as_ptr(), Arc::as_ptr(stream)));
    }
}

/// Translation of `AudioDeviceCanUseSimpleCopy()`. The device should be locked.
fn audio_device_can_use_simple_copy(st: &PhysState) -> bool {
    match st.logical_devices.as_slice() {
        [logdev] => {
            // there's only _ONE_ logical device
            let guard = logdev.state.lock();
            let ls = guard.borrow();
            ls.postmix.is_none() && // there isn't a postmix callback
                ls.bound_streams.len() == 1 // there's only _ONE_ bound stream.
        }
        _ => false,
    }
}

/// Translation of `UpdateAudioStreamFormatsPhysical()`. Should hold the device lock.
fn update_audio_stream_formats_physical(device: &PhysicalDevice, guard: &DeviceGuard<'_>) {
    let recording = device.recording;
    let (mut spec, devformat, chmap, channels, logical_devices) = {
        let mut st = guard.borrow_mut();
        let spec = st.spec;
        if !recording {
            let simple_copy = audio_device_can_use_simple_copy(&st);
            st.simple_copy = simple_copy;
        }
        (
            spec,
            spec.format,
            st.chmap,
            st.spec.channels,
            st.logical_devices.clone(),
        )
    };
    if !recording && !guard.borrow().simple_copy {
        spec.format = AudioFormat::F32; // mixing and postbuf operates in float32 format.
    }

    for logdev in &logical_devices {
        let lguard = logdev.state.lock();
        let streams = {
            let ls = lguard.borrow();
            if recording {
                let need_float32 = ls.postmix.is_some() || ls.gain != 1.0;
                spec.format = if need_float32 {
                    AudioFormat::F32
                } else {
                    devformat
                };
            }
            ls.bound_streams.clone()
        };

        for stream in &streams {
            // set the proper end of the stream to the device's format.
            // SDL_SetAudioStreamFormat does a ton of validation just to memcpy an audiospec.
            let sguard = stream.lock();
            let mut s = sguard.borrow_mut();
            if recording {
                s.src_spec = spec;
            } else {
                s.dst_spec = spec;
            }
            let _ = set_audio_stream_channel_map(&mut s, recording, chmap_ref(&chmap), channels);
            // this should be fast for normal cases, though!
        }
    }
}

// Zombie device implementation...

/// These get used when a device is disconnected or fails, so audiostreams
/// don't overflow with data that isn't being consumed and apps relying on
/// audio callbacks don't stop making progress. Translation of the
/// `Zombie*Device` functions.
struct Zombie;

impl Zombie {
    /// Translation of `ZombieWaitDevice()`.
    fn wait_device(device: &PhysicalDevice) -> bool {
        if !device.shutting_down() {
            let (frames, freq) = {
                let guard = device.lock();
                let st = guard.borrow();
                (
                    st.buffer_size / st.spec.frame_size().max(1),
                    st.spec.freq.max(1) as usize,
                )
            };
            crate::timer::delay(Duration::from_millis(((frames * 1000) / freq) as u64));
        }
        true
    }
}

// device management and hotplug...

/// increments on each device add to provide unique instance IDs
static LAST_DEVICE_INSTANCE_ID: AtomicU32 = AtomicU32::new(0);

/// Translation of `AssignAudioDeviceInstanceId()`.
fn assign_audio_device_instance_id(recording: bool, islogical: bool) -> AudioDeviceID {
    /* Assign an instance id! Start at 2, in case there are things from the SDL2 era that still think 1 is a special value.
    Also, make sure we don't assign SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK, etc. */

    // The bottom two bits of the instance id tells you if it's an playback device (1<<0), and if it's a physical device (1<<1).
    let flags: AudioDeviceID =
        (if recording { 0 } else { 1 << 0 }) | (if islogical { 0 } else { 1 << 1 });

    let instance_id = ((LAST_DEVICE_INSTANCE_ID.fetch_add(1, Ordering::AcqRel) + 1) << 2) | flags;
    crate::sdl_assert!((2..AUDIO_DEVICE_DEFAULT_RECORDING).contains(&instance_id));
    instance_id
}

/// Whether an audio device is physical (instead of logical).
/// Translation of `SDL_IsAudioDevicePhysical()`.
///
/// An [`AudioDeviceID`] that represents physical hardware is a physical
/// device; there is one for each piece of hardware that SDL can see. Logical
/// devices are created by calling [`open_audio_device`] or
/// [`open_audio_device_stream`], and while each is associated with a
/// physical device, there can be any number of logical devices on one
/// physical device.
///
/// For the most part, logical and physical IDs are interchangeable--if you
/// try to open a logical device, SDL understands to assign that effort to
/// the underlying physical device, etc. However, it might be useful to know
/// if an arbitrary device ID is physical or logical.
pub fn is_audio_device_physical(devid: AudioDeviceID) -> bool {
    // bit #1 of devid is set for physical devices and unset for logical.
    (devid & (1 << 1)) != 0
}

/// Translation of `SDL_IsAudioDeviceLogical()`.
fn is_audio_device_logical(devid: AudioDeviceID) -> bool {
    // bit #1 of devid is set for physical devices and unset for logical.
    (devid & (1 << 1)) == 0
}

/// Whether an audio device is a playback device (instead of recording).
/// Translation of `SDL_IsAudioDevicePlayback()`.
pub fn is_audio_device_playback(devid: AudioDeviceID) -> bool {
    // bit #0 of devid is set for playback devices and unset for recording.
    (devid & (1 << 0)) != 0
}

/// Translation of `SDL_IsAudioDeviceRecording()`.
fn is_audio_device_recording(devid: AudioDeviceID) -> bool {
    // bit #0 of devid is set for playback devices and unset for recording.
    (devid & (1 << 0)) == 0
}

/// Backends can call these to change a device's refcount.
/// Translation of `RefPhysicalAudioDevice()`.
pub(crate) fn ref_physical_audio_device(device: &PhysicalDevice) {
    device.refcount.fetch_add(1, Ordering::AcqRel);
}

/// Don't hold the device lock when calling this, as we may destroy the
/// device! Translation of `UnrefPhysicalAudioDevice()`.
pub(crate) fn unref_physical_audio_device(device: &Arc<PhysicalDevice>) {
    if device.refcount.fetch_sub(1, Ordering::AcqRel) == 1 {
        // take it out of the device list.
        {
            let mut audio = audio_write();
            if let Some(audio) = audio.as_mut() {
                if let Some(hash) = audio.device_hash_physical.as_mut() {
                    if hash.remove(&device.instance_id).is_some() {
                        if device.recording {
                            audio.recording_device_count -= 1;
                        } else {
                            audio.playback_device_count -= 1;
                        }
                    }
                }
            }
        }
        destroy_physical_audio_device(device); // ...and nuke it.
    }
}

/// Run `f` with a physical device referenced and locked (`ObtainPhysicalAudioDeviceObj()`
/// followed by `ReleaseAudioDevice()`).
fn with_device_obj<R>(device: &Arc<PhysicalDevice>, f: impl FnOnce(&DeviceGuard<'_>) -> R) -> R {
    ref_physical_audio_device(device);
    let guard = device.lock();
    let r = f(&guard);
    drop(guard);
    unref_physical_audio_device(device);
    r
}

/// Run `f` with a logical device and its physical device locked.
/// Translation of `ObtainLogicalAudioDevice()` followed by `ReleaseAudioDevice()`.
fn with_logical_device<R>(
    devid: AudioDeviceID,
    f: impl FnOnce(&Arc<LogicalDevice>, &Arc<PhysicalDevice>, &DeviceGuard<'_>) -> R,
) -> Result<R> {
    if current_audio_driver().is_none() {
        return Err(Error::new("Audio subsystem is not initialized"));
    }

    let mut found = None;
    if is_audio_device_logical(devid) {
        // don't bother looking if it's not a logical device id value.
        let audio = audio_read();
        if let Some(logdev) = audio
            .as_ref()
            .and_then(|a| a.device_hash_logical.as_ref())
            .and_then(|h| h.get(&devid))
        {
            crate::sdl_assert!(logdev.instance_id == devid);
            let device = logdev.physical();
            ref_physical_audio_device(&device); // reference it, in case the logical device migrates to a new default.
            found = Some((logdev.clone(), device));
        }
    }

    let Some((logdev, mut device)) = found else {
        return Err(Error::new("Invalid audio device instance ID"));
    };

    // we have to release the subsystem_rwlock before we take the device lock, to avoid deadlocks, so do a loop
    //  to make sure the correct physical device gets locked, in case we're in a race with the default changing.
    loop {
        let guard = device.lock();
        let recheck_device = logdev.physical();
        if Arc::ptr_eq(&device, &recheck_device) {
            let r = f(&logdev, &device, &guard);
            drop(guard);
            unref_physical_audio_device(&device);
            return Ok(r);
        }

        // default changed from under us! Try again!
        ref_physical_audio_device(&recheck_device);
        drop(guard);
        unref_physical_audio_device(&device);
        device = recheck_device;
    }
}

/// Find the physical device associated with `devid`, referenced (not
/// locked). Note that a logical device instance id will return its
/// associated physical device! Part of `ObtainPhysicalAudioDevice()`.
fn find_physical_audio_device(devid: AudioDeviceID) -> Result<Arc<PhysicalDevice>> {
    if is_audio_device_logical(devid) {
        return with_logical_device(devid, |_, device, _| {
            ref_physical_audio_device(device);
            device.clone()
        });
    }
    if current_audio_driver().is_none() {
        // (the `islogical` path, above, checks this in ObtainLogicalAudioDevice.)
        return Err(Error::new("Audio subsystem is not initialized"));
    }

    let audio = audio_read();
    let device = audio
        .as_ref()
        .and_then(|a| a.device_hash_physical.as_ref())
        .and_then(|h| h.get(&devid))
        .cloned();
    crate::sdl_assert!(device.as_ref().is_none_or(|d| d.instance_id == devid));
    match device {
        Some(device) => {
            ref_physical_audio_device(&device);
            Ok(device)
        }
        None => Err(Error::new("Invalid audio device instance ID")),
    }
}

/// Run `f` with the physical device for `devid` locked.
/// Translation of `ObtainPhysicalAudioDevice()` + `ReleaseAudioDevice()`.
fn with_physical_device<R>(
    devid: AudioDeviceID,
    f: impl FnOnce(&Arc<PhysicalDevice>, &DeviceGuard<'_>) -> R,
) -> Result<R> {
    let device = find_physical_audio_device(devid)?;
    let guard = device.lock();
    let r = f(&device, &guard);
    drop(guard);
    unref_physical_audio_device(&device);
    Ok(r)
}

/// Translation of `ObtainPhysicalAudioDeviceDefaultAllowed()` + `ReleaseAudioDevice()`.
fn with_physical_device_default_allowed<R>(
    devid: AudioDeviceID,
    f: impl FnOnce(&Arc<PhysicalDevice>, &DeviceGuard<'_>) -> R,
) -> Result<R> {
    let wants_default =
        devid == AUDIO_DEVICE_DEFAULT_PLAYBACK || devid == AUDIO_DEVICE_DEFAULT_RECORDING;
    if !wants_default {
        return with_physical_device(devid, f);
    }

    let orig_devid = devid;
    let current_default = || {
        let audio = audio_read();
        let audio = audio.as_ref();
        if orig_devid == AUDIO_DEVICE_DEFAULT_PLAYBACK {
            audio.map_or(0, |a| a.default_playback_device_id)
        } else {
            audio.map_or(0, |a| a.default_recording_device_id)
        }
    };

    loop {
        let devid = current_default();

        if devid == 0 {
            return Err(Error::new("No default audio device available"));
        }

        let device = find_physical_audio_device(devid)?;
        let guard = device.lock();

        // make sure the default didn't change while we were waiting for the lock...
        if current_default() == devid {
            let r = f(&device, &guard);
            drop(guard);
            unref_physical_audio_device(&device);
            return Ok(r);
        }

        // let it go and try again.
        drop(guard);
        unref_physical_audio_device(&device);
    }
}

/// Translation of `DestroyLogicalAudioDevice()`.
///
/// this assumes you hold the _physical_ device lock for this logical
/// device! This will not unlock the lock or close the physical device! It
/// also will not unref the physical device, since we might be shutting
/// down; `close_audio_device` handles the unref.
fn destroy_logical_audio_device(
    logdev: &Arc<LogicalDevice>,
    device: &PhysicalDevice,
    guard: &DeviceGuard<'_>,
) {
    // Remove ourselves from the device_hash hashtable.
    if let Some(hash) = audio_write()
        .as_mut()
        .and_then(|a| a.device_hash_logical.as_mut())
    {
        // will be None while shutting down.
        hash.remove(&logdev.instance_id);
    }

    // remove ourselves from the physical device's list of logical devices.
    guard
        .borrow_mut()
        .logical_devices
        .retain(|l| !Arc::ptr_eq(l, logdev));

    // unbind any still-bound streams...
    let streams = {
        let lguard = logdev.state.lock();
        let mut ls = lguard.borrow_mut();
        ls.props = None;
        std::mem::take(&mut ls.bound_streams)
    };
    for stream in &streams {
        let sguard = stream.lock();
        sguard.borrow_mut().bound_device = None;
    }

    update_audio_stream_formats_physical(device, guard);
}

/// Translation of `DestroyPhysicalAudioDevice()`.
///
/// this must not be called while `device` is still in a device list, or
/// while a device's audio thread is still running.
fn destroy_physical_audio_device(device: &Arc<PhysicalDevice>) {
    // Destroy any logical devices that still exist...
    let guard = device.lock(); // don't use ObtainPhysicalAudioDeviceObj because we don't want to change refcounts while destroying.
    loop {
        let first = guard.borrow().logical_devices.first().cloned();
        let Some(logdev) = first else { break };
        destroy_logical_audio_device(&logdev, device, &guard);
    }

    close_physical_audio_device(device, &guard);

    if let Some(driver) = current_driver() {
        driver.free_device_handle(device);
    }

    let mut st = guard.borrow_mut();
    st.props = None;
    st.work_buffer = Vec::new();
    st.chmap = None;
}

/// Translation of `CreatePhysicalAudioDevice()`.
fn create_physical_audio_device(
    name: &str,
    unique_id: Option<&str>,
    recording: bool,
    spec: &AudioSpec,
    handle: usize,
) -> Option<Arc<PhysicalDevice>> {
    if audio_read().as_ref().is_none_or(|a| a.shutting_down) {
        return None; // we're shutting down, don't add any devices that are hotplugged at the last possible moment.
    }

    let device = Arc::new(PhysicalDevice {
        lock: ReentrantMutex::new(RefCell::new(PhysState {
            spec: *spec,
            buffer_size: 0,
            chmap: None,
            default_spec: *spec,
            sample_frames: get_default_sample_frames_from_freq(spec.freq),
            silence_value: spec.format.silence_value(),
            simple_copy: false,
            zombie_ops: false,
            device_buffer: Vec::new(),
            work_buffer: Vec::new(),
            mix_buffer: Vec::new(),
            postmix_buffer: Vec::new(),
            work_buffer_size: 0,
            thread: None,
            currently_opened: false,
            props: None,
            backend: None,
            logical_devices: Vec::new(),
        })),
        close_cond: Condition::new(),
        refcount: AtomicI32::new(0),
        name: name.to_owned(),
        unique_id: unique_id.map(str::to_owned),
        instance_id: assign_audio_device_instance_id(recording, false),
        handle,
        recording,
        shutdown: AtomicI32::new(0),
        zombie: AtomicI32::new(0),
    });

    {
        let mut audio = audio_write();
        let audio = audio.as_mut()?;
        let hash = audio.device_hash_physical.as_mut()?;
        hash.insert(device.instance_id, device.clone());
        if recording {
            audio.recording_device_count += 1;
        } else {
            audio.playback_device_count += 1;
        }
    }

    ref_physical_audio_device(&device); // unref'd on device disconnect.
    Some(device)
}

/// Queue device events to be pushed when the event queue is pumped (away
/// from any of our internal threads).
fn queue_pending_events(events: Vec<(EventType, AudioDeviceID)>) {
    if events.is_empty() {
        return;
    }
    if let Some(audio) = audio_write().as_mut() {
        audio.pending_events.extend(events);
    }
}

/// The audio backends call this when a new device is plugged in, and for
/// every device found during `detect_devices`. Translation of `SDL_AddAudioDevice()`.
pub(crate) fn add_audio_device(
    recording: bool,
    name: &str,
    unique_id: Option<&str>,
    inspec: Option<&AudioSpec>,
    handle: usize,
) -> Option<Arc<PhysicalDevice>> {
    // device handles MUST be unique! If the target reuses the same handle for hardware with both recording and playback interfaces, wrap it in a pointer you SDL_malloc'd!
    crate::sdl_assert!(find_physical_audio_device_by_handle(handle).is_none());

    let default_format = if recording {
        DEFAULT_AUDIO_RECORDING_FORMAT
    } else {
        DEFAULT_AUDIO_PLAYBACK_FORMAT
    };
    let default_channels = if recording {
        DEFAULT_AUDIO_RECORDING_CHANNELS
    } else {
        DEFAULT_AUDIO_PLAYBACK_CHANNELS
    };
    let default_freq = if recording {
        DEFAULT_AUDIO_RECORDING_FREQUENCY
    } else {
        DEFAULT_AUDIO_PLAYBACK_FREQUENCY
    };

    let spec = match inspec {
        None => AudioSpec::new(default_format, default_channels, default_freq),
        Some(s) => AudioSpec::new(
            if s.format.0 != 0 {
                s.format
            } else {
                default_format
            },
            if s.channels != 0 {
                s.channels
            } else {
                default_channels
            },
            if s.freq != 0 { s.freq } else { default_freq },
        ),
    };

    if recording {
        crate::sdl_assert!(driver_flags().has_recording_support);
    }
    let device = create_physical_audio_device(name, unique_id, recording, &spec, handle);

    // Add a device add event to the pending list, to be pushed when the event queue is pumped (away from any of our internal threads).
    if let Some(device) = &device {
        queue_pending_events(vec![(EventType::AUDIO_DEVICE_ADDED, device.instance_id)]);
    }

    device
}

/// Translation of `SetAudioDeviceZombieFunctions()`. You must hold the device lock.
///
/// Swap in "Zombie" versions of the usual platform interfaces, so the
/// device will keep making progress until the app closes it. Otherwise,
/// streams might continue to accumulate waste data that never drains, apps
/// that depend on audio callbacks to progress will freeze, etc.
fn set_audio_device_zombie_functions(guard: &DeviceGuard<'_>) {
    guard.borrow_mut().zombie_ops = true;
}

/// Translation of `SDL_AudioDeviceDisconnected_OnMainThread()`: called when
/// a device is removed from the system, or it fails unexpectedly.
fn audio_device_disconnected_on_main_thread(devid: AudioDeviceID) {
    let Ok(device) = find_physical_audio_device(devid) else {
        return; // apparently it went away already.
    };
    let guard = device.lock();

    // Save off removal info in a list so we can send events for each, next
    //  time the event queue pumps, in case something tries to close a device
    //  from an event filter, as this would risk deadlocks and other disasters
    //  if done from the device thread.
    let mut pending = Vec::new();

    let is_default_device = audio_read().as_ref().is_some_and(|a| {
        devid == a.default_playback_device_id || devid == a.default_recording_device_id
    });

    // zombie==2 means "we've handled the disconnect events". 1=="we marked this as dead from a random thread but haven't done anything else"  0==we think we're still alive.
    let first_disconnect = device
        .zombie
        .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
        || device
            .zombie
            .compare_exchange(1, 2, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
    if first_disconnect {
        // if already disconnected this device, don't do it twice.
        set_audio_device_zombie_functions(&guard); // in case we beat the device thread to this.

        // on default devices, dump any logical devices that explicitly opened this device. Things that opened the system default can stay.
        // on non-default devices, dump everything.
        // (by "dump" we mean send a REMOVED event; the zombie will keep consuming audio data for these logical devices until explicitly closed.)
        let logical_devices = guard.borrow().logical_devices.clone();
        for logdev in &logical_devices {
            let opened_as_default = logdev.state.lock().borrow().opened_as_default;
            if !is_default_device || !opened_as_default {
                // if opened as a default, leave it on the zombie device for later migration.
                pending.push((EventType::AUDIO_DEVICE_REMOVED, logdev.instance_id));
            }
        }

        pending.push((EventType::AUDIO_DEVICE_REMOVED, device.instance_id));
    }

    drop(guard);
    unref_physical_audio_device(&device); // (ReleaseAudioDevice)

    if first_disconnect {
        queue_pending_events(pending);
        unref_physical_audio_device(&device);
    }

    // We always ref this in SDL_AudioDeviceDisconnected(), so if multiple attempts
    // to disconnect are queued, the pointer stays valid until the last one comes
    // through.
    unref_physical_audio_device(&device);
}

/// Backends should call this if an opened audio device is lost. This can
/// happen due to i/o errors, or a device being unplugged, etc.
/// Translation of `SDL_AudioDeviceDisconnected()`.
pub(crate) fn audio_device_disconnected(device: &Arc<PhysicalDevice>) {
    // lots of risk of various audio backends deadlocking because they're calling
    // this while holding a backend-specific lock, which causes problems when we
    // want to obtain the device lock while its audio thread is also waiting for
    // that lock to be released. So just queue the work on the main thread.
    ref_physical_audio_device(device);
    let _ = device
        .zombie
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire); // note that we're (un)dead right now, if we haven't already, but leave the event notifications for the main thread.
    let devid = device.instance_id;
    let _ = crate::events::run_on_main_thread(
        move || audio_device_disconnected_on_main_thread(devid),
        false,
    );
}

/// Translation of `GetFirstAddedAudioDevice()`.
fn get_first_added_audio_device(recording: bool) -> Option<Arc<PhysicalDevice>> {
    // (Device IDs increase as new devices are added, so the first device added has the lowest AudioDeviceID value.)
    let audio = audio_read();
    let hash = audio.as_ref()?.device_hash_physical.as_ref()?;
    hash.iter()
        .find(|(&devid, _)| {
            crate::sdl_assert!(is_audio_device_physical(devid)); // should only be iterating device_hash_physical.
            is_audio_device_recording(devid) == recording
        })
        .map(|(_, d)| d.clone())
}

/// Used by `init` to initialize a particular audio driver.
/// Translation of `SDL_InitAudio()`.
pub(crate) fn init_audio(driver_name: Option<&str>) -> Result<()> {
    if current_audio_driver().is_some() {
        quit_audio(); // shutdown driver if already running.
    }

    // make sure device IDs start at 2 (because of SDL2 legacy interface), but don't reset the counter on each init, in case the app is holding an old device ID somewhere.
    let _ = LAST_DEVICE_INSTANCE_ID.compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire);

    resample::setup_audio_resampler();

    // Select the proper audio driver
    let hint = hints::get(hints::AUDIO_DRIVER);
    let driver_name = driver_name.map(str::to_owned).or(hint);

    let mut initialized = None;
    let mut tried_to_init = false;

    let try_init = |b: &'static AudioBootStrap| -> Option<(&'static AudioBootStrap, Arc<dyn AudioDriverImpl>)> {
        // (SDL_zero(current_audio) and the fresh hashtables)
        *audio_write() = Some(CurrentAudio {
            device_hash_physical: Some(BTreeMap::new()),
            device_hash_logical: Some(BTreeMap::new()),
            ..CurrentAudio::default()
        });
        let driver = (b.init)()?;
        Some((b, driver))
    };

    match driver_name.as_deref() {
        Some(name) if !name.is_empty() => {
            for driver_attempt in name.split(',') {
                if initialized.is_some() || driver_attempt.is_empty() {
                    break;
                }

                // SDL 1.2 uses the name "dsound", so we'll support both.
                let driver_attempt = match driver_attempt {
                    "dsound" => "directsound",
                    "pulse" => "pulseaudio", // likewise, "pulse" was renamed to "pulseaudio"
                    other => other,
                };

                for b in BOOTSTRAP {
                    if !b.is_preferred && b.name.eq_ignore_ascii_case(driver_attempt) {
                        tried_to_init = true;
                        if let Some(r) = try_init(b) {
                            initialized = Some(r);
                            break;
                        }
                    }
                }
            }
        }
        _ => {
            for b in BOOTSTRAP {
                if initialized.is_some() {
                    break;
                }
                if b.demand_only {
                    continue;
                }

                tried_to_init = true;
                initialized = try_init(b);
            }
        }
    }

    let Some((bootstrap, driver)) = initialized else {
        *audio_write() = None;
        // specific drivers will set the error message if they fail, but otherwise we do it here.
        return Err(if !tried_to_init {
            match driver_name {
                Some(name) => Error::new(format!("Audio target '{name}' not available")),
                None => Error::new("No available audio device"),
            }
        } else {
            Error::new(format!(
                "Audio target '{}' failed to initialize",
                driver_name.unwrap_or_default()
            ))
        });
    };

    {
        let mut audio = audio_write();
        if let Some(audio) = audio.as_mut() {
            audio.name = Some(bootstrap.name);
            audio.desc = Some(bootstrap.desc);
            audio.flags = driver.flags();
            audio.driver = Some(driver.clone());
        }
    }
    crate::debug!(
        crate::log::Category::System,
        "SDL chose audio backend '{}'",
        bootstrap.name
    );

    // Make sure we have a list of devices available at startup...
    let (mut default_playback, mut default_recording) = driver.detect_devices();

    // If no default was _ever_ specified, just take the first device we see, if any.
    if default_playback.is_none() {
        default_playback = get_first_added_audio_device(false);
    }

    if default_recording.is_none() {
        default_recording = get_first_added_audio_device(true);
    }

    let mut audio = audio_write();
    if let Some(audio) = audio.as_mut() {
        if let Some(d) = &default_playback {
            audio.default_playback_device_id = d.instance_id;
            ref_physical_audio_device(d); // extra ref on default devices.
        }

        if let Some(d) = &default_recording {
            audio.default_recording_device_id = d.instance_id;
            ref_physical_audio_device(d); // extra ref on default devices.
        }
    }

    Ok(())
}

/// Used by `init` to shut down previously-initialized audio.
/// Translation of `SDL_QuitAudio()`.
pub(crate) fn quit_audio() {
    let Some(driver) = current_driver() else {
        return; // not initialized?!
    };

    driver.deinitialize_start();

    // Destroy any audio streams that still exist...unless app asked to keep it.
    let streams: Vec<Arc<StreamInner>> = audio_read()
        .as_ref()
        .map(|a| {
            a.existing_streams
                .iter()
                .filter_map(Weak::upgrade)
                .collect()
        })
        .unwrap_or_default();
    for stream in &streams {
        if AudioStream::wants_auto_cleanup(stream) {
            AudioStream::destroy_for_quit(stream);
        }
    }

    let device_hash_physical = {
        let mut audio = audio_write();
        let Some(audio) = audio.as_mut() else { return };
        audio.existing_streams.clear();
        audio.shutting_down = true;
        let hash = audio.device_hash_physical.take();
        audio.device_hash_logical = None;
        audio.pending_events.clear();
        audio.playback_device_count = 0;
        audio.recording_device_count = 0;
        hash
    };

    for device in device_hash_physical.into_iter().flatten().map(|(_, d)| d) {
        destroy_physical_audio_device(&device);
    }

    // Free the driver data
    driver.deinitialize();

    *audio_write() = None;
}

/// Translation of `SDL_AudioThreadFinalize()`.
fn audio_thread_finalize(_device: &PhysicalDevice) {}

/// Translation of `MixFloat32Audio()`: `SDL_MixAudio()` of float32 data at
/// full volume (`dst = clamp(src * 1.0 + dst, -1, 1)`).
#[allow(clippy::manual_clamp)] // (the comparisons of SDL_MixAudio, NaN behaviour included)
fn mix_float32_audio(dst: &mut [f32], src: impl Iterator<Item = f32>) {
    for (d, s) in dst.iter_mut().zip(src) {
        let mut v = s * 1.0 + *d;
        if v > 1.0 {
            v = 1.0;
        } else if v < -1.0 {
            v = -1.0;
        }
        *d = v;
    }
}

fn f32_samples(bytes: &[u8]) -> impl Iterator<Item = f32> + '_ {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
}

/// Swap a scratch buffer out of the device state while it's in use without
/// the state borrowed; [`put_back`] returns it.
fn take<T: Default>(slot: &mut T) -> T {
    std::mem::take(slot)
}

/// Return a taken buffer, unless something reallocated it meanwhile.
fn put_back<T>(slot: &mut Vec<T>, buf: Vec<T>) {
    if slot.is_empty() {
        *slot = buf;
    }
}

// Playback device thread. This is split into chunks, so backends that need to control this directly can use the pieces they need without duplicating effort.

/// Translation of `SDL_PlaybackAudioThreadSetup()`.
pub(crate) fn playback_audio_thread_setup(device: &PhysicalDevice) {
    crate::sdl_assert!(!device.recording);
    if let Some(driver) = current_driver() {
        driver.thread_init(device);
    }
}

/// The device's backend for this iteration, or `None` when the zombie
/// functions are in use.
fn active_backend(st: &PhysState) -> Option<Arc<dyn DeviceBackend>> {
    if st.zombie_ops {
        None
    } else {
        st.backend.clone()
    }
}

/// Translation of `SDL_PlaybackAudioThreadIterate()`.
pub(crate) fn playback_audio_thread_iterate(device: &Arc<PhysicalDevice>) -> bool {
    crate::sdl_assert!(!device.recording);

    let guard = device.lock();

    if device.shutting_down() {
        return false; // we're done, shut it down.
    }

    if device.zombie.load(Ordering::Acquire) == 1 {
        // we've been marked as (un)dead but not fully processed. Set up the zombie functions so we stop talking to the real backend.
        set_audio_device_zombie_functions(&guard);
    }

    let mut failed = false;
    let (backend, mut buffer_size, spec, chmap, silence_value, simple_copy, logical_devices) = {
        let st = guard.borrow();
        (
            active_backend(&st),
            st.buffer_size,
            st.spec,
            st.chmap,
            st.silence_value,
            st.simple_copy,
            st.logical_devices.clone(),
        )
    };

    let got = match &backend {
        Some(b) => b.get_device_buf(device, buffer_size),
        None => Some(buffer_size), // ZombieGetDeviceBuf()
    };
    match got {
        Some(0) => {
            // WASAPI (maybe others, later) does this to say "just abandon this iteration and try again next time."
        }
        None => failed = true,
        Some(n) => {
            crate::sdl_assert!(n <= buffer_size); // you can ask for less, but not more.
            buffer_size = n.min(buffer_size);

            let (mut device_buffer, mut work_buffer, mut mix_buffer, mut postmix_buffer) = {
                let mut st = guard.borrow_mut();
                crate::sdl_assert!(audio_device_can_use_simple_copy(&st) == st.simple_copy); // make sure this hasn't gotten out of sync.
                (
                    take(&mut st.device_buffer),
                    take(&mut st.work_buffer),
                    take(&mut st.mix_buffer),
                    take(&mut st.postmix_buffer),
                )
            };
            if device_buffer.len() < buffer_size {
                device_buffer.resize(buffer_size, silence_value);
            }

            // can we do a basic copy without silencing/mixing the buffer? This is an extremely likely scenario, so we special-case it.
            if simple_copy {
                let logdev = &logical_devices[0];
                let (stream, gain) = {
                    let lguard = logdev.state.lock();
                    let ls = lguard.borrow();
                    (ls.bound_streams[0].clone(), ls.gain)
                };

                let dst_chmap = {
                    // We should have updated this elsewhere if the format changed!
                    let sguard = stream.lock();
                    let s = sguard.borrow();
                    crate::sdl_assert!(audio_specs_equal(&s.dst_spec, &spec, None, None));
                    crate::sdl_assert!(s.src_spec.format != AudioFormat::UNKNOWN);
                    s.dst_chmap
                };

                let out = &mut device_buffer[..buffer_size];
                let br = if logdev.paused.load(Ordering::Acquire) {
                    Ok(0)
                } else {
                    AudioStream::borrowed(&stream).get_data_adjust_gain(out, gain)
                };
                match br {
                    Err(_) => {
                        // Probably OOM. Kill the audio device; the whole thing is likely dying soon anyhow.
                        failed = true;
                        out.fill(silence_value); // just supply silence to the device before we die.
                    }
                    Ok(br) => {
                        if br < buffer_size {
                            out[br..].fill(silence_value); // silence whatever we didn't write to.
                        }

                        // generally channel maps will line up, but if the audio stream's chmap has been explicitly changed, do a final swizzle to device layout.
                        if br > 0
                            && !audio_channel_maps_equal(
                                spec.channels,
                                chmap_ref(&dst_chmap),
                                chmap_ref(&chmap),
                            )
                        {
                            convert_audio(
                                br / spec.frame_size(),
                                ConvertSrc::InDst,
                                spec.format,
                                spec.channels,
                                None,
                                out,
                                spec.format,
                                spec.channels,
                                chmap_ref(&chmap),
                                None,
                                1.0,
                            );
                        }
                    }
                }
            } else {
                // need to actually mix (or silence the buffer)
                let needed_samples = buffer_size / spec.format.bytesize() as usize;
                let work_buffer_size = needed_samples * std::mem::size_of::<f32>();
                let outspec = AudioSpec {
                    format: AudioFormat::F32,
                    ..spec
                };

                if work_buffer.len() < work_buffer_size {
                    work_buffer.resize(work_buffer_size, 0);
                }
                mix_buffer.clear();
                mix_buffer.resize(needed_samples, 0.0); // start with silence.

                'logical: for logdev in &logical_devices {
                    if logdev.paused.load(Ordering::Acquire) {
                        continue; // paused? Skip this logical device.
                    }

                    let lguard = logdev.state.lock();
                    let (postmix, gain, streams) = {
                        let ls = lguard.borrow();
                        (ls.postmix.clone(), ls.gain, ls.bound_streams.clone())
                    };

                    if postmix.is_some() {
                        postmix_buffer.clear();
                        postmix_buffer.resize(needed_samples, 0.0); // start with silence.
                    }

                    for stream in &streams {
                        let dst_chmap = {
                            // We should have updated this elsewhere if the format changed!
                            let sguard = stream.lock();
                            let s = sguard.borrow();
                            crate::sdl_assert!(audio_specs_equal(
                                &s.dst_spec,
                                &outspec,
                                None,
                                None
                            ));
                            crate::sdl_assert!(s.src_spec.format != AudioFormat::UNKNOWN);
                            s.dst_chmap
                        };

                        /* this will hold a lock on `stream` while getting. We don't explicitly lock the streams
                        for iterating here because the binding linked list can only change while the device lock is held.
                        (we _do_ lock the stream during binding/unbinding to make sure that two threads can't try to bind
                        the same stream to different devices at the same time, though.) */
                        match AudioStream::borrowed(stream)
                            .get_data_adjust_gain(&mut work_buffer[..work_buffer_size], gain)
                        {
                            Err(_) => {
                                // Probably OOM. Kill the audio device; the whole thing is likely dying soon anyhow.
                                failed = true;
                                break 'logical;
                            }
                            Ok(0) => {}
                            Ok(br) => {
                                // it's okay if we get less than requested, we mix what we have.
                                // generally channel maps will line up, but if the audio stream's chmap has been explicitly changed, do a final swizzle to device layout.
                                if !audio_channel_maps_equal(
                                    spec.channels,
                                    chmap_ref(&dst_chmap),
                                    chmap_ref(&chmap),
                                ) {
                                    // FIXME (upstream): this swizzles F32 data with the device's (possibly non-F32) format and frame size.
                                    convert_audio(
                                        br / spec.frame_size(),
                                        ConvertSrc::InDst,
                                        spec.format,
                                        spec.channels,
                                        None,
                                        &mut work_buffer[..],
                                        spec.format,
                                        spec.channels,
                                        chmap_ref(&chmap),
                                        None,
                                        1.0,
                                    );
                                }
                                let target = if postmix.is_some() {
                                    &mut postmix_buffer
                                } else {
                                    &mut mix_buffer
                                };
                                mix_float32_audio(target, f32_samples(&work_buffer[..br]));
                            }
                        }
                    }

                    if let Some(postmix) = &postmix {
                        postmix(&outspec, &mut postmix_buffer[..needed_samples]);
                        mix_float32_audio(&mut mix_buffer, postmix_buffer.iter().copied());
                    }
                }

                // (the mix is in float32; move it to the device's format)
                let out = &mut device_buffer[..buffer_size];
                if spec.format == AudioFormat::F32 {
                    for (o, s) in out.chunks_exact_mut(4).zip(&mix_buffer) {
                        o.copy_from_slice(&s.to_ne_bytes());
                    }
                } else {
                    // !!! FIXME: we can't promise the device buf is aligned/padded for SIMD.
                    let mut mixed = Vec::with_capacity(needed_samples * 4);
                    for s in &mix_buffer {
                        mixed.extend_from_slice(&s.to_ne_bytes());
                    }
                    convert_audio(
                        needed_samples / spec.channels as usize,
                        ConvertSrc::Slice(&mixed),
                        AudioFormat::F32,
                        spec.channels,
                        None,
                        &mut work_buffer[..],
                        spec.format,
                        spec.channels,
                        None,
                        None,
                        1.0,
                    );
                    out.copy_from_slice(&work_buffer[..buffer_size]);
                }
            }

            // PlayDevice SHOULD NOT BLOCK, as we are holding a lock right now. Block in WaitDevice instead!
            let played = match &backend {
                Some(b) => b.play_device(device, &device_buffer[..buffer_size]),
                None => true, // ZombiePlayDevice(): no-op, just throw the audio away.
            };
            if !played {
                failed = true;
            }

            let mut st = guard.borrow_mut();
            put_back(&mut st.device_buffer, device_buffer);
            put_back(&mut st.work_buffer, work_buffer);
            put_back(&mut st.mix_buffer, mix_buffer);
            put_back(&mut st.postmix_buffer, postmix_buffer);
        }
    }

    drop(guard);

    if failed {
        audio_device_disconnected(device); // doh.
    }

    true // always go on if not shutting down, even if device failed.
}

/// Translation of `SDL_PlaybackAudioThreadShutdown()`.
pub(crate) fn playback_audio_thread_shutdown(device: &PhysicalDevice) {
    crate::sdl_assert!(!device.recording);
    let (frames, freq) = {
        let guard = device.lock();
        let st = guard.borrow();
        (
            st.buffer_size / st.spec.frame_size().max(1),
            st.spec.freq.max(1) as usize,
        )
    };
    // Wait for the audio to drain if device didn't die.
    if device.zombie.load(Ordering::Acquire) == 0 {
        let delay = (((frames * 1000) / freq) * 2).min(100);
        crate::timer::delay(Duration::from_millis(delay as u64));
    }
    if let Some(driver) = current_driver() {
        driver.thread_deinit(device);
    }
    audio_thread_finalize(device);
}

/// Translation of `PlaybackAudioThread()` (the thread entry point).
fn playback_audio_thread(device: Arc<PhysicalDevice>) -> i32 {
    crate::sdl_assert!(!device.recording);
    playback_audio_thread_setup(&device);

    while playback_audio_thread_iterate(&device) {
        let backend = active_backend(&device.lock().borrow());
        let ok = match backend {
            Some(b) => b.wait_device(&device),
            None => Zombie::wait_device(&device),
        };
        if !ok {
            audio_device_disconnected(&device); // doh. (but don't break out of the loop, just be a zombie for now!)
        }
    }

    playback_audio_thread_shutdown(&device);
    0
}

// Recording device thread. This is split into chunks, so backends that need to control this directly can use the pieces they need without duplicating effort.

/// Translation of `SDL_RecordingAudioThreadSetup()`.
pub(crate) fn recording_audio_thread_setup(device: &PhysicalDevice) {
    crate::sdl_assert!(device.recording);
    if let Some(driver) = current_driver() {
        driver.thread_init(device);
    }
}

/// Translation of `SDL_RecordingAudioThreadIterate()`.
pub(crate) fn recording_audio_thread_iterate(device: &Arc<PhysicalDevice>) -> bool {
    crate::sdl_assert!(device.recording);

    let guard = device.lock();

    if device.shutting_down() {
        return false; // we're done, shut it down.
    }

    if device.zombie.load(Ordering::Acquire) == 1 {
        // we've been marked as (un)dead but not fully processed. Set up the zombie functions so we stop talking to the real backend.
        set_audio_device_zombie_functions(&guard);
    }

    let mut failed = false;
    let (backend, buffer_size, spec, chmap, silence_value, logical_devices) = {
        let st = guard.borrow();
        (
            active_backend(&st),
            st.buffer_size,
            st.spec,
            st.chmap,
            st.silence_value,
            st.logical_devices.clone(),
        )
    };

    if logical_devices.is_empty() {
        // nothing wants data, dump anything pending.
        if let Some(b) = &backend {
            b.flush_recording(device);
        } // (ZombieFlushRecording(): no-op, this is all imaginary.)
    } else {
        let (mut work_buffer, mut mix_buffer_bytes, mut postmix_buffer) = {
            let mut st = guard.borrow_mut();
            (
                take(&mut st.work_buffer),
                Vec::<u8>::new(),
                take(&mut st.postmix_buffer),
            )
        };
        if work_buffer.len() < buffer_size {
            work_buffer.resize(buffer_size, 0);
        }

        // this SHOULD NOT BLOCK, as we are holding a lock right now. Block in WaitRecordingDevice!
        let br = match &backend {
            Some(b) => b.record_device(device, &mut work_buffer[..buffer_size]),
            None => {
                // ZombieRecordDevice(): return a full buffer of silence every time.
                work_buffer[..buffer_size].fill(silence_value);
                Ok(buffer_size)
            }
        };
        match br {
            Err(_) => failed = true, // uhoh, device failed for some reason!
            Ok(0) => {}
            Ok(br0) => {
                // queue the new data to each bound stream.
                'logical: for logdev in &logical_devices {
                    if logdev.paused.load(Ordering::Acquire) {
                        continue; // paused? Skip this logical device.
                    }

                    let lguard = logdev.state.lock();
                    let (postmix, gain, streams) = {
                        let ls = lguard.borrow();
                        (ls.postmix.clone(), ls.gain, ls.bound_streams.clone())
                    };

                    let mut br = br0;
                    let mut out_format = spec.format;
                    let mut floats: Vec<u8> = Vec::new();

                    // I don't know why someone would want a postmix on a recording device, but we offer it for API consistency.
                    if postmix.is_some() || gain != 1.0 {
                        // move to float format.
                        let outspec = AudioSpec {
                            format: AudioFormat::F32,
                            ..spec
                        };
                        let frames = br / spec.frame_size();
                        br = frames * outspec.frame_size();
                        floats = vec![0u8; br.max(work_buffer.len())];
                        convert_audio(
                            frames,
                            ConvertSrc::Slice(&work_buffer[..br0]),
                            spec.format,
                            outspec.channels,
                            None,
                            &mut floats,
                            AudioFormat::F32,
                            outspec.channels,
                            None,
                            None,
                            gain,
                        );
                        out_format = AudioFormat::F32;
                        if let Some(postmix) = &postmix {
                            postmix_buffer.clear();
                            postmix_buffer.extend(f32_samples(&floats[..br]));
                            postmix(&outspec, &mut postmix_buffer);
                            for (o, s) in floats.chunks_exact_mut(4).zip(&postmix_buffer) {
                                o.copy_from_slice(&s.to_ne_bytes());
                            }
                        }
                    }
                    let output_buffer: &[u8] =
                        if out_format == AudioFormat::F32 && !floats.is_empty() {
                            &floats[..br]
                        } else {
                            &work_buffer[..br]
                        };

                    for stream in &streams {
                        let src_chmap = {
                            // We should have updated this elsewhere if the format changed!
                            let sguard = stream.lock();
                            let s = sguard.borrow();
                            crate::sdl_assert!(s.src_spec.format == out_format);
                            crate::sdl_assert!(s.src_spec.channels == spec.channels);
                            crate::sdl_assert!(s.src_spec.freq == spec.freq);
                            crate::sdl_assert!(s.dst_spec.format != AudioFormat::UNKNOWN);
                            s.src_chmap
                        };

                        let mut final_buf = output_buffer;

                        // generally channel maps will line up, but if the audio stream's chmap has been explicitly changed, do a final swizzle to stream layout.
                        if !audio_channel_maps_equal(
                            spec.channels,
                            chmap_ref(&src_chmap),
                            chmap_ref(&chmap),
                        ) {
                            // (the mix buffer is otherwise unused on recording devices, so it makes convenient scratch space here.)
                            mix_buffer_bytes.clear();
                            mix_buffer_bytes.resize(br.max(buffer_size), 0);
                            convert_audio(
                                br / spec.frame_size(),
                                ConvertSrc::Slice(output_buffer),
                                spec.format,
                                spec.channels,
                                None,
                                &mut mix_buffer_bytes,
                                spec.format,
                                spec.channels,
                                chmap_ref(&src_chmap),
                                None,
                                1.0,
                            );
                            final_buf = &mix_buffer_bytes[..br];
                        }

                        /* this will hold a lock on `stream` while putting. We don't explicitly lock the streams
                        for iterating here because the binding linked list can only change while the device lock is held.
                        (we _do_ lock the stream during binding/unbinding to make sure that two threads can't try to bind
                        the same stream to different devices at the same time, though.) */
                        if AudioStream::borrowed(stream).put_data(final_buf).is_err() {
                            // oh crud, we probably ran out of memory. This is possibly an overreaction to kill the audio device, but it's likely the whole thing is going down in a moment anyhow.
                            failed = true;
                            break 'logical;
                        }
                    }
                }
            }
        }

        let mut st = guard.borrow_mut();
        put_back(&mut st.work_buffer, work_buffer);
        put_back(&mut st.postmix_buffer, postmix_buffer);
    }

    drop(guard);

    if failed {
        audio_device_disconnected(device); // doh.
    }

    true // always go on if not shutting down, even if device failed.
}

/// Translation of `SDL_RecordingAudioThreadShutdown()`.
pub(crate) fn recording_audio_thread_shutdown(device: &PhysicalDevice) {
    crate::sdl_assert!(device.recording);
    let backend = active_backend(&device.lock().borrow());
    if let Some(b) = backend {
        b.flush_recording(device);
    }
    if let Some(driver) = current_driver() {
        driver.thread_deinit(device);
    }
    audio_thread_finalize(device);
}

/// Translation of `RecordingAudioThread()` (the thread entry point).
fn recording_audio_thread(device: Arc<PhysicalDevice>) -> i32 {
    crate::sdl_assert!(device.recording);
    recording_audio_thread_setup(&device);

    loop {
        let backend = active_backend(&device.lock().borrow());
        let ok = match backend {
            Some(b) => b.wait_recording_device(&device),
            None => Zombie::wait_device(&device),
        };
        if !ok {
            audio_device_disconnected(&device); // doh. (but don't break out of the loop, just be a zombie for now!)
        }
        if !recording_audio_thread_iterate(&device) {
            break;
        }
    }

    recording_audio_thread_shutdown(&device);
    0
}

/// Translation of `GetAudioDevices()`.
fn get_audio_devices(recording: bool) -> Result<Vec<AudioDeviceID>> {
    let audio = audio_read();
    let Some(audio) = audio.as_ref().filter(|a| a.name.is_some()) else {
        return Err(Error::new("Audio subsystem is not initialized"));
    };
    let mut result = Vec::new();
    let mut skipped = 0;
    for (&devid, device) in audio.device_hash_physical.iter().flatten() {
        crate::sdl_assert!(is_audio_device_physical(devid)); // should only be iterating device_hash_physical.
        if is_audio_device_recording(devid) == recording {
            let zombie = device.zombie.load(Ordering::Acquire) != 0;
            if zombie {
                skipped += 1;
            } else {
                result.push(devid);
            }
        }
    }
    let num_devices = if recording {
        audio.recording_device_count
    } else {
        audio.playback_device_count
    };
    crate::sdl_assert!((result.len() + skipped) as i32 == num_devices);
    Ok(result)
}

/// Get a list of currently-connected audio playback devices.
/// Translation of `SDL_GetAudioPlaybackDevices()`.
///
/// This returns of list of available devices that play sound, perhaps to
/// speakers or headphones ("playback" devices). If you want devices that
/// record audio, like a microphone ("recording" devices), use
/// [`recording_devices`] instead.
///
/// This only returns a list of physical devices; it will not have any
/// device IDs returned by [`open_audio_device`].
///
/// If this function returns an empty list, it's not an error; it just means
/// there are no devices available.
pub fn playback_devices() -> Result<Vec<AudioDeviceID>> {
    get_audio_devices(false)
}

/// Get a list of currently-connected audio recording devices.
/// Translation of `SDL_GetAudioRecordingDevices()`.
///
/// This returns of list of available devices that record audio, like a
/// microphone ("recording" devices). If you want devices that play sound,
/// perhaps to speakers or headphones ("playback" devices), use
/// [`playback_devices`] instead.
pub fn recording_devices() -> Result<Vec<AudioDeviceID>> {
    get_audio_devices(true)
}

/// Find a physical device, selected by a callback. DOES NOT LOCK THE DEVICE.
/// Translation of `SDL_FindPhysicalAudioDeviceByCallback()`.
#[allow(dead_code)] // for hotplug-capable backends
pub(crate) fn find_physical_audio_device_by_callback(
    callback: impl Fn(&PhysicalDevice) -> bool,
) -> Result<Arc<PhysicalDevice>> {
    let audio = audio_read();
    let Some(audio) = audio.as_ref().filter(|a| a.name.is_some()) else {
        return Err(Error::new("Audio subsystem is not initialized"));
    };

    audio
        .device_hash_physical
        .iter()
        .flatten()
        .map(|(_, d)| d)
        .find(|d| callback(d)) // found it?
        .cloned()
        .ok_or_else(|| Error::new("Device not found"))
}

/// Find the device associated with the handle supplied to
/// `add_audio_device`. DOES NOT LOCK THE DEVICE.
/// Translation of `SDL_FindPhysicalAudioDeviceByHandle()`.
pub(crate) fn find_physical_audio_device_by_handle(handle: usize) -> Option<Arc<PhysicalDevice>> {
    // (TestDeviceHandleCallback)
    let audio = audio_read();
    audio
        .as_ref()?
        .device_hash_physical
        .as_ref()?
        .values()
        .find(|d| d.handle == handle)
        .cloned()
}

/// Get the human-readable name of a specific audio device.
/// Translation of `SDL_GetAudioDeviceName()`.
///
/// This works with the default device IDs too, returning the current
/// default physical device's name.
pub fn audio_device_name(devid: AudioDeviceID) -> Result<String> {
    // This does not lock the device because the device's name never changes, so
    // it doesn't have to lock the whole device, as this can causes a deadlock in sdl2-compat in
    // certain corner cases.
    // However, just to make sure the device pointer itself remains valid (in case the device is
    // unplugged at the wrong moment), we hold the subsystem_rwlock while we copy the string.
    let audio = audio_read();
    let Some(audio) = audio.as_ref().filter(|a| a.name.is_some()) else {
        return Err(Error::new("Audio subsystem is not initialized"));
    };

    let islogical = is_audio_device_logical(devid);

    // Allow default device IDs to be used, just return the current default physical device's name.
    let devid = match devid {
        AUDIO_DEVICE_DEFAULT_PLAYBACK => audio.default_playback_device_id,
        AUDIO_DEVICE_DEFAULT_RECORDING => audio.default_recording_device_id,
        other => other,
    };

    let name = if islogical {
        audio
            .device_hash_logical
            .as_ref()
            .and_then(|h| h.get(&devid))
            .map(|logdev| logdev.physical().name.clone())
    } else {
        audio
            .device_hash_physical
            .as_ref()
            .and_then(|h| h.get(&devid))
            .map(|d| d.name.clone())
    };
    name.ok_or_else(|| Error::new("Invalid audio device instance ID"))
}

/// Get the current audio format of a specific audio device, and its buffer
/// size in sample frames. Translation of `SDL_GetAudioDeviceFormat()`.
///
/// For an opened device, this will report the format the device is
/// currently using. If the device isn't yet opened, this will report the
/// device's preferred format (or a reasonable default if this can't be
/// determined).
///
/// You may also specify [`AUDIO_DEVICE_DEFAULT_PLAYBACK`] or
/// [`AUDIO_DEVICE_DEFAULT_RECORDING`] here, which is useful for getting a
/// reasonable recommendation before opening the system-recommended default
/// device.
///
/// You can also use this to request the current device buffer size. This
/// is specified in sample frames and represents the amount of data SDL will
/// feed to the physical hardware in each chunk. This can be converted to
/// milliseconds of audio with the following equation:
///
/// `ms = (int) ((((Sint64) frames) * 1000) / spec.freq);`
///
/// Buffer size is only important if you need low-level control over the
/// audio playback timing. Most apps do not need this.
pub fn audio_device_format(devid: AudioDeviceID) -> Result<(AudioSpec, i32)> {
    with_physical_device_default_allowed(devid, |_, guard| {
        let st = guard.borrow();
        (st.spec, st.sample_frames)
    })
}

/// Get the current channel map of an audio device (`None` for the default
/// layout, or if the device can't be found). Translation of `SDL_GetAudioDeviceChannelMap()`.
///
/// Channel maps are optional; most things do not need them, instead passing
/// data in the [order that SDL expects](https://wiki.libsdl.org/SDL3/CategoryAudio#channel-layouts).
///
/// Audio devices usually have no remapping applied. This is represented by
/// returning `None`, and does not signify an error.
pub fn audio_device_channel_map(devid: AudioDeviceID) -> Option<Vec<i32>> {
    with_physical_device_default_allowed(devid, |_, guard| {
        let st = guard.borrow();
        st.chmap.map(|m| m[..st.spec.channels as usize].to_vec())
    })
    .ok()
    .flatten()
}

/// Get the properties associated with an audio device.
/// Translation of `SDL_GetAudioDeviceProperties()`.
///
/// The following read-only properties are provided by SDL:
///
/// - [`PROP_AUDIO_DEVICE_UNIQUE_ID_STRING`]: a unique, persistent,
///   backend-specific string to identify this specific device, if the
///   backend can provide one.
///
/// A logical device gets its own set of properties.
pub fn audio_device_properties(devid: AudioDeviceID) -> Result<Properties> {
    let fill = |props: &mut Option<Properties>, device: &PhysicalDevice| {
        props
            .get_or_insert_with(|| {
                let p = Properties::new();
                // fill in some basic properties.
                if let Some(id) = &device.unique_id {
                    let _ = p.set(PROP_AUDIO_DEVICE_UNIQUE_ID_STRING, id.as_str());
                }
                p
            })
            .clone()
    };
    if is_audio_device_logical(devid) {
        with_logical_device(devid, |logdev, device, _| {
            fill(&mut logdev.state.lock().borrow_mut().props, device)
        })
    } else {
        with_physical_device(devid, |device, guard| {
            fill(&mut guard.borrow_mut().props, device)
        })
    }
}

/// Translation of `SerializePhysicalDeviceClose()`.
///
/// this is awkward, but this makes sure we can release the device lock so
/// the device thread can terminate but also not have two things race to
/// close or open the device while the lock is unprotected. you hold the
/// lock when calling this, it will release the lock and wait while the
/// shutdown flag is set. BE CAREFUL WITH THIS.
fn serialize_physical_device_close(device: &PhysicalDevice, guard: &DeviceGuard<'_>) {
    while device.shutting_down() {
        device.close_cond.wait(guard);
    }
}

/// Translation of `ClosePhysicalAudioDevice()`. This expects the device lock to be held.
fn close_physical_audio_device(device: &PhysicalDevice, guard: &DeviceGuard<'_>) {
    serialize_physical_device_close(device, guard);

    let currently_opened = guard.borrow().currently_opened;
    if currently_opened {
        // there might be other cleanup even when closed, but only log this if we were fully up and running.
        let devtypestr = if device.recording {
            "recording"
        } else {
            "playback"
        };
        crate::debug!(
            crate::log::Category::Audio,
            "AUDIO: closing {devtypestr} device '{}'",
            device.name
        );
    }

    device.shutdown.store(1, Ordering::Release);

    // YOU MUST PROTECT KEY POINTS WITH SerializePhysicalDeviceClose() WHILE THE THREAD JOINS
    let thread = guard.borrow_mut().thread.take();
    guard.raw().unlock();

    if let Some(thread) = thread {
        thread.wait();
    }

    let backend = {
        guard.raw().lock();
        let mut st = guard.borrow_mut();
        let backend = if st.currently_opened {
            st.backend.take()
        } else {
            None
        };
        st.currently_opened = false;
        st.backend = None; // just in case.
        guard.raw().unlock();
        backend
    };
    if let Some(backend) = backend {
        backend.close_device(device); // if ProvidesOwnCallbackThread, this must join on any existing device thread before returning!
    }

    guard.raw().lock();
    device.shutdown.store(0, Ordering::Release); // ready to go again.
    device.close_cond.broadcast(); // release anyone waiting in SerializePhysicalDeviceClose; they'll still block until we release device->lock, though.

    let mut st = guard.borrow_mut();
    st.device_buffer = Vec::new();
    st.work_buffer = Vec::new();
    st.mix_buffer = Vec::new();
    st.postmix_buffer = Vec::new();

    st.spec = st.default_spec;
    st.sample_frames = 0;
    st.silence_value = st.spec.format.silence_value();
}

/// Close a previously-opened audio device. Translation of `SDL_CloseAudioDevice()`.
///
/// The application should close open audio devices once they are no longer
/// needed. This only closes the logical device; when the last logical
/// device on a physical device closes, the hardware is closed too.
pub fn close_audio_device(devid: AudioDeviceID) {
    let _ = with_logical_device(devid, |logdev, device, guard| {
        destroy_logical_audio_device(logdev, device, guard);

        if guard.borrow().logical_devices.is_empty() {
            // no more logical devices? Close the physical device, too.
            close_physical_audio_device(device, guard);
        }
        unref_physical_audio_device(device); // one reference for each logical device.
    });
}

/// Translation of `PrepareAudioFormat()`.
fn prepare_audio_format(recording: bool, spec: &mut AudioSpec) {
    if spec.freq == 0 {
        spec.freq = if recording {
            DEFAULT_AUDIO_RECORDING_FREQUENCY
        } else {
            DEFAULT_AUDIO_PLAYBACK_FREQUENCY
        };

        if let Some(hint) = hints::get(hints::AUDIO_FREQUENCY) {
            let val = crate::stdlib::atoi(&hint);
            if val > 0 {
                spec.freq = val;
            }
        }
    }

    if spec.channels == 0 {
        spec.channels = if recording {
            DEFAULT_AUDIO_RECORDING_CHANNELS
        } else {
            DEFAULT_AUDIO_PLAYBACK_CHANNELS
        };

        if let Some(hint) = hints::get(hints::AUDIO_CHANNELS) {
            let val = crate::stdlib::atoi(&hint);
            if val > 0 {
                spec.channels = val;
            }
        }
    }

    if spec.format.0 == 0 {
        let val = AudioFormat::parse(hints::get(hints::AUDIO_FORMAT).as_deref());
        spec.format = if val != AudioFormat::UNKNOWN {
            val
        } else if recording {
            DEFAULT_AUDIO_RECORDING_FORMAT
        } else {
            DEFAULT_AUDIO_PLAYBACK_FORMAT
        };
    }
}

/// Backends should call this if they change the device format, channels,
/// freq, or sample_frames to keep other state correct.
/// Translation of `SDL_UpdatedAudioDeviceFormat()`.
pub(crate) fn updated_audio_device_format(st: &mut PhysState) {
    st.silence_value = st.spec.format.silence_value();
    st.buffer_size = st.sample_frames.max(0) as usize * st.spec.frame_size();
    st.work_buffer_size = st.sample_frames.max(0) as usize
        * std::mem::size_of::<f32>()
        * st.spec.channels.max(0) as usize;
    st.work_buffer_size = st.buffer_size.max(st.work_buffer_size); // just in case we end up with a 64-bit audio format at some point.
}

/// Backends can call this to get a standardized name for a thread to power
/// a specific audio device. Translation of `SDL_GetAudioThreadName()`.
pub(crate) fn get_audio_thread_name(device: &PhysicalDevice) -> String {
    format!(
        "SDLAudio{}{}",
        if device.recording { 'C' } else { 'P' },
        device.instance_id as i32
    )
}

/// Translation of `OpenPhysicalAudioDevice()`. This expects the device lock to be held.
fn open_physical_audio_device(
    device: &Arc<PhysicalDevice>,
    guard: &DeviceGuard<'_>,
    inspec: Option<&AudioSpec>,
) -> Result<()> {
    serialize_physical_device_close(device, guard); // make sure another thread that's closing didn't release the lock to let the device thread join...

    // Just pretend to open a zombie device. It can still collect logical devices on a default device under the assumption they will all migrate when the default device is officially changed.
    if device.zombie.load(Ordering::Acquire) != 0 {
        return Ok(()); // Braaaaaaaaains.
    }

    let devtypestr = if device.recording {
        "recording"
    } else {
        "playback"
    };

    let mut spec = inspec
        .copied()
        .unwrap_or_else(|| guard.borrow().default_spec);
    prepare_audio_format(device.recording, &mut spec);

    let (currently_opened, current) = {
        let st = guard.borrow();
        (st.currently_opened, st.spec)
    };
    if currently_opened {
        // if something has already opened the device at a lower quality, attempt to reopen it with the new request.
        // This prevents something intentionally low quality, like VoIP playback, from making the system sound bad
        // because it opened the hardware before the CD-quality background music arrived. In theory this could cause
        // an audio hitch, but it would be a one-time thing, and as we've learned from default device migration, not
        // actually that painful in practice.
        if spec.format.bitsize() <= current.format.bitsize()
            && spec.channels <= current.channels
            && spec.freq <= current.freq
        {
            return Ok(()); // we're already good.
        }

        // uhoh, have to reopen the device...choose the "better" values from each spec.
        spec.format = if spec.format.bitsize() > current.format.bitsize() {
            spec.format
        } else {
            current.format
        };
        spec.channels = spec.channels.max(current.channels);
        spec.freq = spec.freq.max(current.freq);

        crate::debug!(
            crate::log::Category::Audio,
            "AUDIO: attempt to reopen {devtypestr} device '{}' at higher spec! ({},{},{} => {},{},{})",
            device.name,
            current.format.short_name(),
            current.channels,
            current.freq,
            spec.format.short_name(),
            spec.channels,
            spec.freq
        );
        close_physical_audio_device(device, guard);
        if open_physical_audio_device(device, guard, Some(&spec)).is_err() {
            // no good, try to go back to our original spec...
            if let Err(e) = open_physical_audio_device(device, guard, Some(&current)) {
                // okay, _now_ we're in trouble. Report the device as disconnected, since we've just broken all the existing logical devices. :(
                // !!! FIXME: the logical devices need a thread to run the zombie implementation.
                audio_device_disconnected(device);
                return Err(e);
            }
        }

        // adjust all the attached audio streams to the new format, send format change events, etc.
        let (spec, sample_frames) = {
            let mut st = guard.borrow_mut();
            let spec = st.spec; // save off whatever we ended up with.
            st.spec = current; // put it back to what it was so the next function call doesn't return immediately. The next call will reset it properly.
            (spec, st.sample_frames)
        };
        let _ = audio_device_format_changed_already_locked(device, guard, &spec, sample_frames); // if this fails, it's probably because we're out of memory and didn't send the events, but the device is _probably_ functional!

        let now = guard.borrow().spec;
        crate::debug!(
            crate::log::Category::Audio,
            "AUDIO: {devtypestr} device '{}' is now at spec ({},{},{})",
            device.name,
            now.format.short_name(),
            now.channels,
            now.freq
        );

        return Ok(()); // carry on with the reconfigured device!
    }

    // FIXME (upstream): the requested `spec` is only used for logging and
    // for the reopen comparison above; the device opens at its current
    // (default) spec, as in this version of SDL_audio.c.

    // These start with the backend's implementation, but we might swap them out with zombie versions later.
    {
        let mut st = guard.borrow_mut();
        st.zombie_ops = false;
        st.sample_frames = get_default_sample_frames_from_freq(st.spec.freq);
        updated_audio_device_format(&mut st); // start this off sane.
    }

    crate::debug!(
        crate::log::Category::Audio,
        "AUDIO: attempt to open {devtypestr} device '{}' at spec ({},{},{})",
        device.name,
        spec.format.short_name(),
        spec.channels,
        spec.freq
    );

    guard.borrow_mut().currently_opened = true; // mark this true even if impl.OpenDevice fails, so we know to clean up.
    let Some(driver) = current_driver() else {
        close_physical_audio_device(device, guard);
        return Err(Error::new("Audio subsystem is not initialized"));
    };
    let opened = {
        let mut st = guard.borrow_mut();
        driver.open_device(device, &mut st)
    };
    let backend = match opened {
        Ok(b) => b,
        Err(e) => {
            crate::debug!(
                crate::log::Category::Audio,
                "AUDIO: open of {devtypestr} device '{}' failed: {}",
                device.name,
                e.message()
            );
            close_physical_audio_device(device, guard); // clean up anything the backend left half-initialized.
            return Err(e);
        }
    };

    {
        let mut st = guard.borrow_mut();
        st.backend = Some(backend);
        updated_audio_device_format(&mut st); // in case the backend changed things and forgot to call this.

        // Allocate a scratch audio buffer
        st.work_buffer = vec![0u8; st.work_buffer_size];
        st.device_buffer = vec![st.silence_value; st.buffer_size];

        if st.spec.format != AudioFormat::F32 {
            st.mix_buffer = vec![0.0f32; st.work_buffer_size / 4];
        }
    }

    // Start the audio thread if necessary
    if !driver_flags().provides_own_callback_thread {
        let threadname = get_audio_thread_name(device);
        let thread_device = device.clone();
        let spawned = if device.recording {
            Thread::spawn(threadname, move || recording_audio_thread(thread_device))
        } else {
            Thread::spawn(threadname, move || playback_audio_thread(thread_device))
        };

        match spawned {
            Ok(thread) => guard.borrow_mut().thread = Some(thread),
            Err(e) => {
                crate::debug!(
                    crate::log::Category::Audio,
                    "AUDIO: open of {devtypestr} device '{}' failed: {}",
                    device.name,
                    e.message()
                );
                close_physical_audio_device(device, guard);
                return Err(Error::new("Couldn't create audio thread"));
            }
        }
    }

    let now = guard.borrow().spec;
    crate::debug!(
        crate::log::Category::Audio,
        "AUDIO: attempt to open {devtypestr} device '{}' succeeded! Opened at spec ({},{},{})",
        device.name,
        now.format.short_name(),
        now.channels,
        now.freq
    );

    Ok(())
}

/// Open a specific audio device. Translation of `SDL_OpenAudioDevice()`.
///
/// You can open both playback and recording devices through this function.
/// Playback devices will take data from bound audio streams, mix it, and
/// send it to the hardware. Recording devices will feed any bound audio
/// streams with a copy of any incoming data.
///
/// An opened audio device starts out with no audio streams bound. To start
/// audio playing, bind a stream and supply audio data to it. Unlike SDL2,
/// there is no audio callback; you only bind audio streams and make sure
/// they have data flowing into them (however, you can simulate SDL2's
/// semantics fairly closely by using [`open_audio_device_stream`] instead
/// of this function).
///
/// If you don't care about opening a specific device, pass a `devid` of
/// either [`AUDIO_DEVICE_DEFAULT_PLAYBACK`] or
/// [`AUDIO_DEVICE_DEFAULT_RECORDING`]. In this case, SDL will try to pick
/// the most reasonable default, and may also switch between physical
/// devices seamlessly later, if the most reasonable default changes during
/// the lifetime of this opened device (user changed the default in the OS's
/// system preferences, the default got unplugged so the system jumped to a
/// new one, the user plugged in headphones on a mobile device, etc). Unless
/// you have a good reason to choose a specific device, this is probably
/// what you want.
///
/// You may request a specific format for the audio device, but there is no
/// promise the device will honor that request for several reasons. As such,
/// it's only meant to be a hint as to what data your app will provide.
/// Audio streams will accept data in whatever format you specify and
/// manage conversion for you as appropriate. [`audio_device_format`] can
/// tell you the preferred format for the device before opening and the
/// actual format the device is using after opening.
///
/// It's legal to open the same device ID more than once; each successful
/// open will generate a new logical [`AudioDeviceID`] that is managed
/// separately from others on the same physical device. This allows
/// libraries to open a device separately from the main app and bind its
/// own streams without conflicting.
///
/// It is also legal to open a device ID returned by a previous call to
/// this function; doing so just creates another logical device on the same
/// physical device. This may be useful for making logical groupings of
/// audio streams.
///
/// This function returns the opened device ID on success. This is a new,
/// unique AudioDeviceID that represents a logical device. Close it with
/// [`close_audio_device`] (or use [`AudioDevice`], which closes on drop).
pub fn open_audio_device(devid: AudioDeviceID, spec: Option<&AudioSpec>) -> Result<AudioDeviceID> {
    if current_audio_driver().is_none() {
        return Err(Error::new("Audio subsystem is not initialized"));
    }

    let mut wants_default =
        devid == AUDIO_DEVICE_DEFAULT_PLAYBACK || devid == AUDIO_DEVICE_DEFAULT_RECORDING;

    // this will let you use a logical device to make a new logical device on the parent physical device. Could be useful?
    let device = if wants_default || is_audio_device_physical(devid) {
        with_physical_device_default_allowed(devid, |device, _| {
            ref_physical_audio_device(device);
            device.clone()
        })?
    } else {
        with_logical_device(devid, |logdev, device, _| {
            wants_default = logdev.state.lock().borrow().opened_as_default; // was the original logical device meant to be a default? Make this one, too.
            ref_physical_audio_device(device);
            device.clone()
        })?
    };

    let result = {
        let guard = device.lock();
        let r = if !wants_default && device.zombie.load(Ordering::Acquire) != 0 {
            // uhoh, this device is undead, and just waiting to be cleaned up. Refuse explicit opens.
            Err(Error::new(
                "Device was already lost and can't accept new opens",
            ))
        } else {
            // if this is the first thing using this physical device, open at the OS level if necessary...
            open_physical_audio_device(&device, &guard, spec).map(|()| {
                ref_physical_audio_device(&device); // unref'd on successful SDL_CloseAudioDevice
                let logdev = Arc::new(LogicalDevice {
                    instance_id: assign_audio_device_instance_id(device.recording, true),
                    recording: device.recording,
                    paused: AtomicBool::new(false),
                    physical_device: Mutex::new(device.clone()),
                    state: ReentrantMutex::new(RefCell::new(LogicalState {
                        gain: 1.0,
                        bound_streams: Vec::new(),
                        opened_as_default: wants_default,
                        simplified: false,
                        postmix: None,
                        props: None,
                    })),
                });
                guard.borrow_mut().logical_devices.insert(0, logdev.clone());
                update_audio_stream_formats_physical(&device, &guard);
                logdev
            })
        };
        drop(guard);
        unref_physical_audio_device(&device); // (ReleaseAudioDevice)
        r
    };

    let logdev = result?;
    let id = logdev.instance_id;
    let inserted = match audio_write()
        .as_mut()
        .and_then(|a| a.device_hash_logical.as_mut())
    {
        Some(hash) => {
            hash.insert(id, logdev);
            true
        }
        None => false,
    };
    if !inserted {
        close_audio_device(id);
        return Err(Error::new("Audio subsystem is shutting down"));
    }

    Ok(id)
}

/// Translation of `SetLogicalAudioDevicePauseState()`.
fn set_logical_audio_device_pause_state(devid: AudioDeviceID, value: bool) -> Result<()> {
    with_logical_device(devid, |logdev, _, _| {
        logdev.paused.store(value, Ordering::Release)
    })
}

/// Use this function to pause audio playback on a specified device.
/// Translation of `SDL_PauseAudioDevice()`.
///
/// This function pauses audio processing for a given device. Any bound
/// audio streams will not progress, and no audio will be generated. Pausing
/// one device does not prevent other unpaused devices from running.
///
/// Unlike in SDL2, audio devices start in an _unpaused_ state, since an
/// app has to bind a stream before any audio will flow. Pausing a paused
/// device is a legal no-op.
///
/// Pausing a device can be useful to halt all audio without unbinding all
/// the audio streams. This might be useful while a game is paused, or a
/// level is loading, etc.
///
/// Physical devices can not be paused or unpaused, only logical devices
/// created through [`open_audio_device`] can be.
pub fn pause_audio_device(devid: AudioDeviceID) -> Result<()> {
    set_logical_audio_device_pause_state(devid, true)
}

/// Use this function to unpause audio playback on a specified device.
/// Translation of `SDL_ResumeAudioDevice()`.
///
/// This function unpauses audio processing for a given device that has
/// previously been paused with [`pause_audio_device`]. Once unpaused, any
/// bound audio streams will begin to progress again, and audio can be
/// generated.
///
/// Physical devices can not be paused or unpaused, only logical devices
/// created through [`open_audio_device`] can be.
pub fn resume_audio_device(devid: AudioDeviceID) -> Result<()> {
    set_logical_audio_device_pause_state(devid, false)
}

/// Use this function to query if an audio device is paused (`false` for
/// invalid ids and physical devices). Translation of `SDL_AudioDevicePaused()`.
pub fn audio_device_paused(devid: AudioDeviceID) -> bool {
    with_logical_device(devid, |logdev, _, _| logdev.paused.load(Ordering::Acquire))
        .unwrap_or(false)
}

/// Get the gain of an audio device. Translation of `SDL_GetAudioDeviceGain()`.
///
/// The gain of a device is its volume; a larger gain means a louder output,
/// with a gain of zero being silence.
///
/// Audio devices default to a gain of 1.0 (no change in output).
///
/// Physical devices may not have their gain changed, only logical devices,
/// and this function will always fail on physical devices.
pub fn audio_device_gain(devid: AudioDeviceID) -> Result<f32> {
    with_logical_device(devid, |logdev, _, _| logdev.state.lock().borrow().gain)
}

/// Change the gain of an audio device. Translation of `SDL_SetAudioDeviceGain()`.
///
/// The gain of a device is its volume; a larger gain means a louder output,
/// with a gain of zero being silence.
///
/// Audio devices default to a gain of 1.0 (no change in output).
///
/// Physical devices may not have their gain changed, only logical devices,
/// and this function will always fail on physical devices.
///
/// This is applied, along with any per-audiostream gain, during playback to
/// the hardware, and can be continuously changed to create various effects.
/// On recording devices, this will adjust the gain before passing the data
/// into an audiostream; that recording audiostream can then adjust its gain
/// further when outputting the data elsewhere, if it likes, but that second
/// gain is not applied until the data leaves the audiostream again.
pub fn set_audio_device_gain(devid: AudioDeviceID, gain: f32) -> Result<()> {
    if gain < 0.0 {
        return Err(Error::invalid_param("gain"));
    }

    with_logical_device(devid, |logdev, device, guard| {
        logdev.state.lock().borrow_mut().gain = gain;
        update_audio_stream_formats_physical(device, guard);
    })
}

/// Set a callback that fires when data is about to be fed to an audio
/// device. Translation of `SDL_SetAudioPostmixCallback()`.
///
/// This is useful for accessing the final mix, perhaps for writing a
/// visualizer or applying a final effect to the audio data before playback.
///
/// The buffer is the final mix of all bound audio streams on an opened
/// device; this callback will fire regularly for any device that is both
/// opened and unpaused. If there is no new data to mix, either because no
/// streams are bound to the device or all the streams are empty, this
/// callback will still fire with the entire buffer set to silence.
///
/// This callback is allowed to make changes to the data; the contents of
/// the buffer after this call is what is ultimately passed along to the
/// hardware.
///
/// The callback is always provided the data in float format (values from
/// -1.0f to 1.0f), but the number of channels or sample rate may be
/// different than the format the app requested when opening the device;
/// SDL might have had to manage a conversion behind the scenes, or the
/// playback might have jumped to new physical hardware when a system
/// default changed, etc. These details may change between calls.
///
/// This callback runs in the device's thread, so any thing that interacts
/// with it should be thread-safe; the device is locked while it runs.
///
/// Physical devices may not have postmix callbacks, only logical devices.
/// `None` removes the callback.
pub fn set_audio_postmix_callback(
    devid: AudioDeviceID,
    callback: Option<impl Fn(&AudioSpec, &mut [f32]) + Send + Sync + 'static>,
) -> Result<()> {
    let cb: Option<AudioPostmixCallback> = callback.map(|c| Arc::new(c) as AudioPostmixCallback);
    set_audio_postmix_callback_arc(devid, cb)
}

fn set_audio_postmix_callback_arc(
    devid: AudioDeviceID,
    callback: Option<AudioPostmixCallback>,
) -> Result<()> {
    with_logical_device(devid, |logdev, device, guard| {
        {
            let mut st = guard.borrow_mut();
            if callback.is_some() && st.postmix_buffer.is_empty() {
                st.postmix_buffer = vec![0.0f32; st.work_buffer_size / 4];
            }
        }

        logdev.state.lock().borrow_mut().postmix = callback;

        update_audio_stream_formats_physical(device, guard);
    })
}

/// Bind a list of audio streams to an audio device.
/// Translation of `SDL_BindAudioStreams()`.
///
/// Audio data will flow through any bound streams. For a playback device,
/// data for all bound streams will be mixed together and fed to the device.
/// For a recording device, a copy of recorded data will be provided to each
/// bound stream.
///
/// Audio streams can only be bound to an open device. This operation is
/// atomic--all streams bound in the same call will start processing at the
/// same time, so they can stay in sync. Also: either all streams will be
/// bound or none of them will be.
///
/// It is an error to bind an already-bound stream; it must be explicitly
/// unbound first.
///
/// Binding a stream to a device will set its output format for playback
/// devices, and its input format for recording devices, so they match the
/// device's settings. The caller is welcome to change the other end of the
/// stream's format at any time with [`AudioStream::set_format`]. If the
/// other end of the stream's format has never been set (the audio stream
/// was created with a `None` spec), this function will set it to match the
/// device end's format.
pub fn bind_audio_streams(devid: AudioDeviceID, streams: &[&AudioStream]) -> Result<()> {
    if streams.is_empty() {
        return Ok(()); // nothing to do
    }

    if is_audio_device_physical(devid) {
        return Err(Error::new("Audio streams are bound to device ids from SDL_OpenAudioDevice, not raw physical devices"));
    }

    with_logical_device(devid, |logdev, device, guard| {
        let lguard = logdev.state.lock();
        let result = if lguard.borrow().simplified {
            Err(Error::new(
                "Cannot change stream bindings on device opened with SDL_OpenAudioDeviceStream",
            ))
        } else {
            // lock all the streams upfront, so we can verify they aren't bound elsewhere and add them all in one block, as this is intended to add everything or nothing.
            let sguards: Vec<_> = streams.iter().map(|s| s.inner().lock()).collect();
            let mut result = Ok(());
            for (i, sg) in sguards.iter().enumerate() {
                let s = sg.borrow();
                if s.bound_device.is_some() {
                    result = Err(Error::new(format!(
                        "Stream #{i} is already bound to a device"
                    )));
                    break;
                } else if s.simplified {
                    // You can get here if you closed the device instead of destroying the stream.
                    result = Err(Error::new(
                        "Cannot change binding on a stream created with SDL_OpenAudioDeviceStream",
                    ));
                    break;
                }
            }

            if result.is_ok() {
                // Now that everything is verified, chain everything together.
                let recording = device.recording;
                let devspec = guard.borrow().spec;
                for (stream, sg) in streams.iter().zip(&sguards) {
                    let mut s = sg.borrow_mut();
                    // if the stream never had its non-device-end format set, just set it to the device end's format.
                    if recording && s.dst_spec.format == AudioFormat::UNKNOWN {
                        s.dst_spec = devspec;
                    } else if !recording && s.src_spec.format == AudioFormat::UNKNOWN {
                        s.src_spec = devspec;
                    }

                    s.bound_device = Some(logdev.clone());
                    lguard
                        .borrow_mut()
                        .bound_streams
                        .insert(0, stream.inner().clone());
                }
            }
            result
        };
        drop(lguard);

        update_audio_stream_formats_physical(device, guard);
        result
    })?
}

/// Bind a single audio stream to an audio device.
/// Translation of `SDL_BindAudioStream()`.
///
/// This is a convenience function, equivalent to calling
/// `bind_audio_streams(devid, &[stream])`.
pub fn bind_audio_stream(devid: AudioDeviceID, stream: &AudioStream) -> Result<()> {
    bind_audio_streams(devid, &[stream])
}

/// Unbind a list of audio streams from their audio devices.
/// Translation of `SDL_UnbindAudioStreams()`.
///
/// The streams being unbound do not all have to be on the same device. All
/// streams on the same device will be unbound atomically (data will stop
/// flowing through all unbound streams on the same device at the same
/// time).
///
/// Unbinding a stream that isn't bound to a device is a legal no-op.
///
/// !!! FIXME: this and BindAudioStreams are mutex nightmares.  :/
pub fn unbind_audio_streams(streams: &[&AudioStream]) {
    if streams.is_empty() {
        return; // nothing to do
    }

    /* to prevent deadlock when holding both locks, we _must_ lock the device first, and the stream second, as that is the order the audio thread will do it.
    But this means we have an unlikely, pathological case where a stream could change its binding between when we lookup its bound device and when we lock everything,
    so we double-check here. */
    loop {
        // lock to check this and then release it, in case the device isn't locked yet.
        let bindings: Vec<Option<(Arc<LogicalDevice>, Arc<PhysicalDevice>)>> = streams
            .iter()
            .map(|s| {
                s.inner().lock().borrow().bound_device.clone().map(|l| {
                    let d = l.physical();
                    (l, d)
                })
            })
            .collect();

        // lock in correct order: every device, then every stream.
        // (this requires recursive mutexes, since we're likely locking the same device multiple times.)
        let device_guards: Vec<_> = bindings.iter().flatten().map(|(_, d)| d.lock()).collect();
        let stream_guards: Vec<_> = streams.iter().map(|s| s.inner().lock()).collect();

        let unchanged = bindings.iter().zip(&stream_guards).all(|(b, sg)| {
            let now = sg.borrow().bound_device.clone();
            match (b, now) {
                (None, None) => true,
                (Some((l, d)), Some(n)) => Arc::ptr_eq(l, &n) && Arc::ptr_eq(d, &l.physical()),
                _ => false,
            }
        });
        if !unchanged {
            continue; // it changed bindings! Try again.
        }

        // everything is locked, start unbinding streams.
        let mut on_simplified = Vec::with_capacity(streams.len());
        for ((b, stream), _sg) in bindings.iter().zip(streams).zip(&stream_guards) {
            // don't allow unbinding from "simplified" devices (opened with SDL_OpenAudioDeviceStream). Just ignore them.
            let mut simplified = false;
            if let Some((logdev, _)) = b {
                let lguard = logdev.state.lock();
                simplified = lguard.borrow().simplified;
                if !simplified {
                    lguard
                        .borrow_mut()
                        .bound_streams
                        .retain(|s| !Arc::ptr_eq(s, stream.inner()));
                }
            }
            on_simplified.push(simplified);
        }

        // Finalize and unlock everything.
        for (sg, &simplified) in stream_guards.iter().zip(&on_simplified) {
            // (upstream clears the binding of a stream on a "simplified"
            // device too, though it stays in that device's list, so the
            // stream looks unbound and destroying it no longer closes the
            // device; fixed here by leaving those streams bound.)
            if !simplified {
                sg.borrow_mut().bound_device = None;
            }
        }
        drop(stream_guards);
        let mut guards = device_guards.iter();
        for (_, device) in bindings.iter().flatten() {
            let guard = guards.next().expect("one guard per bound stream");
            update_audio_stream_formats_physical(device, guard);
        }
        drop(device_guards);
        break;
    }
}

/// Unbind a single audio stream from its audio device.
/// Translation of `SDL_UnbindAudioStream()`.
pub fn unbind_audio_stream(stream: &AudioStream) {
    unbind_audio_streams(&[stream]);
}

/// Convenience function for straightforward audio init for the common case.
/// Translation of `SDL_OpenAudioDeviceStream()`.
///
/// If all your app intends to do is provide a single source of PCM audio,
/// this function allows you to do all your audio setup in a single call.
///
/// This is also intended to be a clean means to migrate apps from SDL2.
///
/// This function will open an audio device, create a stream and bind it.
/// Unlike other methods of setup, the audio device will be closed when this
/// stream is destroyed, so the app can treat the returned [`AudioStream`]
/// as the only object needed to manage audio playback.
///
/// Also unlike other functions, the audio device begins paused. This is to
/// map more closely to SDL2-style behavior, since there is no extra step
/// here to bind a stream to begin audio flowing. The audio device should be
/// resumed with [`AudioStream::resume_device`].
///
/// This function works with both playback and recording devices.
///
/// The `spec` parameter represents the app's side of the audio stream. That
/// is, for recording audio, this will be the output format, and for playing
/// audio, this will be the input format. If `spec` is `None`, the system
/// will choose the format, and the app can use [`AudioStream::format`] to
/// obtain this information later.
///
/// If you don't care about opening a specific audio device, you can (and
/// probably _should_), use [`AUDIO_DEVICE_DEFAULT_PLAYBACK`] for playback
/// and [`AUDIO_DEVICE_DEFAULT_RECORDING`] for recording.
///
/// One can optionally provide a callback function; if `None`, the app is
/// expected to queue audio data for playback (or unqueue audio data if
/// capturing). Otherwise, the callback will begin to fire once the device
/// is unpaused: as the stream's get callback for playback, its put callback
/// for recording.
///
/// Destroying the returned stream with drop will also close the audio
/// device associated with this stream.
pub fn open_audio_device_stream(
    devid: AudioDeviceID,
    spec: Option<&AudioSpec>,
    callback: Option<impl Fn(&AudioStream, i32, i32) + Send + Sync + 'static>,
) -> Result<AudioStream> {
    let logdevid = open_audio_device(devid, spec)?;
    let callback: Option<AudioStreamCallback> =
        callback.map(|c| Arc::new(c) as AudioStreamCallback);

    let made = with_logical_device(logdevid, |logdev, device, guard| -> Result<AudioStream> {
        logdev.paused.store(true, Ordering::Release); // start the device paused, to match SDL2.

        let recording = device.recording;
        let devspec = guard.borrow().spec;

        // if the app didn't request a format _at all_, just make a stream that does no conversion; they can query for it later.
        let spec = spec.copied().unwrap_or(devspec);

        let stream = if recording {
            AudioStream::new(Some(&devspec), Some(&spec))?
        } else {
            AudioStream::new(Some(&spec), Some(&devspec))?
        };

        // don't do all the complicated validation and locking of SDL_BindAudioStream just to set a few fields here.
        {
            let lguard = logdev.state.lock();
            let mut ls = lguard.borrow_mut();
            ls.bound_streams = vec![stream.inner().clone()];
            ls.simplified = true; // forbid further binding changes on this logical device.
        }
        {
            let sguard = stream.inner().lock();
            let mut s = sguard.borrow_mut();
            s.bound_device = Some(logdev.clone());
            s.simplified = true; // so we know to close the audio device when this is destroyed.
        }

        update_audio_stream_formats_physical(device, guard);

        if callback.is_some() {
            stream.set_callback_arc(!recording, callback);
        }
        Ok(stream)
    });

    match made {
        Ok(Ok(stream)) => Ok(stream),
        Ok(Err(e)) | Err(e) => {
            close_audio_device(logdevid);
            Err(e)
        }
    }
}

/// Called internally by backends when the system default device changes.
/// Translation of `SDL_DefaultAudioDeviceChanged()`.
#[allow(dead_code)] // for hotplug-capable backends
pub(crate) fn default_audio_device_changed(new_default_device: &Arc<PhysicalDevice>) {
    let recording = new_default_device.recording;

    // change the official default over right away, so new opens will go to the new device.
    let current_devid = {
        let mut audio = audio_write();
        let Some(audio) = audio.as_mut() else { return };
        let current_devid = if recording {
            audio.default_recording_device_id
        } else {
            audio.default_playback_device_id
        };
        let is_already_default = new_default_device.instance_id == current_devid;
        if is_already_default {
            return; // this is already the default.
        }
        if recording {
            audio.default_recording_device_id = new_default_device.instance_id;
        } else {
            audio.default_playback_device_id = new_default_device.instance_id;
        }
        current_devid
    };

    // Queue up events to push to the queue next time it pumps (presumably
    //  in a safer thread).
    // !!! FIXME: this duplicates some code we could probably refactor.
    let mut pending = Vec::new();

    // Default device gets an extra ref, so it lives until a new default replaces it, even if disconnected.
    ref_physical_audio_device(new_default_device);

    let mut old_default: Option<Arc<PhysicalDevice>> = None;
    with_device_obj(new_default_device, |new_guard| {
        let Ok(current_default_device) = find_physical_audio_device(current_devid) else {
            return;
        };
        old_default = Some(current_default_device.clone());
        let cur_guard = current_default_device.lock();

        // migrate any logical devices that were opened as a default to the new physical device...

        crate::sdl_assert!(current_default_device.recording == recording);

        // See if we have to open the new physical device, and if so, find the best audiospec for it.
        let mut spec = AudioSpec::default();
        let mut needs_migration = false;

        let logical_devices = cur_guard.borrow().logical_devices.clone();
        for logdev in &logical_devices {
            let lguard = logdev.state.lock();
            let ls = lguard.borrow();
            if ls.opened_as_default {
                needs_migration = true;
                for stream in &ls.bound_streams {
                    let sguard = stream.lock();
                    let s = sguard.borrow();
                    let streamspec = if recording { &s.dst_spec } else { &s.src_spec };
                    if streamspec.format.bitsize() > spec.format.bitsize() {
                        spec.format = streamspec.format;
                    }
                    if streamspec.channels > spec.channels {
                        spec.channels = streamspec.channels;
                    }
                    if streamspec.freq > spec.freq {
                        spec.freq = streamspec.freq;
                    }
                }
            }
        }

        if needs_migration {
            // New default physical device not been opened yet? Open at the OS level...
            if open_physical_audio_device(new_default_device, new_guard, Some(&spec)).is_err() {
                needs_migration = false; // uhoh, just leave everything on the old default, nothing to be done.
            }
        }

        if needs_migration {
            // we don't currently report channel map changes, so we'll leave them as NULL for now.
            let spec_changed = !audio_specs_equal(
                &cur_guard.borrow().spec,
                &new_guard.borrow().spec,
                None,
                None,
            );
            for logdev in &logical_devices {
                if !logdev.state.lock().borrow().opened_as_default {
                    continue; // not opened as a default, leave it on the current physical device.
                }

                // now migrate the logical device. Hold subsystem_rwlock so ObtainLogicalAudioDevice doesn't get a device in the middle of transition.
                {
                    let _audio = audio_write();
                    cur_guard
                        .borrow_mut()
                        .logical_devices
                        .retain(|l| !Arc::ptr_eq(l, logdev));
                    *logdev
                        .physical_device
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) = new_default_device.clone();
                    new_guard
                        .borrow_mut()
                        .logical_devices
                        .insert(0, logdev.clone());
                }

                crate::sdl_assert!(current_default_device.refcount.load(Ordering::Acquire) > 1); // we should hold at least one extra reference to this device, beyond logical devices, during this phase...
                ref_physical_audio_device(new_default_device);
                unref_physical_audio_device(&current_default_device);

                let postmix = logdev.state.lock().borrow().postmix.clone();
                let _ = set_audio_postmix_callback_arc(logdev.instance_id, postmix);

                // Queue an event for each logical device we moved.
                if spec_changed {
                    pending.push((EventType::AUDIO_DEVICE_FORMAT_CHANGED, logdev.instance_id));
                }
            }

            update_audio_stream_formats_physical(&current_default_device, &cur_guard);
            update_audio_stream_formats_physical(new_default_device, new_guard);

            if cur_guard.borrow().logical_devices.is_empty() {
                // nothing left on the current physical device, close it.
                close_physical_audio_device(&current_default_device, &cur_guard);
            }
        }

        drop(cur_guard);
        unref_physical_audio_device(&current_default_device); // (ReleaseAudioDevice)
    });

    // Default device gets an extra ref, so it lives until a new default replaces it, even if disconnected.
    if let Some(old) = old_default {
        // (despite the name, it's no longer current at this point)
        unref_physical_audio_device(&old);
    }

    queue_pending_events(pending);
}

/// Translation of `SDL_AudioDeviceFormatChangedAlreadyLocked()`: backends
/// call this if a device's format is changing (opened or not); SDL will
/// update state and carry on with the new format.
fn audio_device_format_changed_already_locked(
    device: &PhysicalDevice,
    guard: &DeviceGuard<'_>,
    newspec: &AudioSpec,
    new_sample_frames: i32,
) -> Result<()> {
    {
        let st = guard.borrow();
        // we don't currently have any place where channel maps change from under you, but we can check that if necessary later.
        if audio_specs_equal(&st.spec, newspec, None, None) && new_sample_frames == st.sample_frames
        {
            return Ok(()); // we're already in that format.
        }
    }

    guard.borrow_mut().spec = *newspec;
    update_audio_stream_formats_physical(device, guard);

    {
        let mut st = guard.borrow_mut();
        let orig_work_buffer_size = st.work_buffer_size;
        st.sample_frames = new_sample_frames;
        updated_audio_device_format(&mut st);
        if !st.work_buffer.is_empty() && st.work_buffer_size > orig_work_buffer_size {
            st.work_buffer = vec![0u8; st.work_buffer_size];

            if !st.postmix_buffer.is_empty() {
                st.postmix_buffer = vec![0.0f32; st.work_buffer_size / 4];
            }

            st.mix_buffer = Vec::new();
            if st.spec.format != AudioFormat::F32 {
                st.mix_buffer = vec![0.0f32; st.work_buffer_size / 4];
            }
        }
        st.device_buffer = vec![st.silence_value; st.buffer_size];
    }

    // Post an event for the physical device, and each logical device on this physical device.
    // Queue up events to push to the queue next time it pumps (presumably
    //  in a safer thread).
    // !!! FIXME: this duplicates some code we could probably refactor.
    let mut pending = vec![(EventType::AUDIO_DEVICE_FORMAT_CHANGED, device.instance_id)];
    for logdev in guard.borrow().logical_devices.iter() {
        pending.push((EventType::AUDIO_DEVICE_FORMAT_CHANGED, logdev.instance_id));
    }
    queue_pending_events(pending);

    Ok(())
}

/// Backends should call this if a device's format is changing (opened or
/// not). Translation of `SDL_AudioDeviceFormatChanged()`.
#[allow(dead_code)] // for backends that renegotiate formats
pub(crate) fn audio_device_format_changed(
    device: &Arc<PhysicalDevice>,
    newspec: &AudioSpec,
    new_sample_frames: i32,
) -> Result<()> {
    with_device_obj(device, |guard| {
        audio_device_format_changed_already_locked(device, guard, newspec, new_sample_frames)
    })
}

/// Push pending audio device events. Translation of `SDL_UpdateAudio()`,
/// called from the event pump.
pub(crate) fn update_audio() {
    let pending_events = {
        let audio = audio_read();
        match audio.as_ref() {
            Some(a) if !a.pending_events.is_empty() => {}
            _ => return, // nothing to do, check next time.
        }
        drop(audio);
        // okay, let's take this whole list of events so we can dump the lock, and new ones can queue up for a later update.
        let mut audio = audio_write();
        match audio.as_mut() {
            Some(a) => std::mem::take(&mut a.pending_events), // in case this changed...
            None => return,
        }
    };

    for (event_type, devid) in pending_events {
        if crate::events::event_enabled(event_type) {
            let _ = crate::events::push(Event::AudioDevice(AudioDeviceEvent {
                event_type,
                timestamp: Duration::ZERO,
                which: devid,
                recording: is_audio_device_recording(devid), // bit #0 of devid is set for playback devices and unset for recording.
            }));
        }
    }
}

/// An opened (logical) audio device that closes when dropped.
///
/// A convenience wrapper over [`open_audio_device`] and the functions that
/// take a logical device id.
#[derive(Debug)]
pub struct AudioDevice {
    id: AudioDeviceID,
}

impl AudioDevice {
    /// Open a device; see [`open_audio_device`].
    pub fn open(devid: AudioDeviceID, spec: Option<&AudioSpec>) -> Result<AudioDevice> {
        open_audio_device(devid, spec).map(|id| AudioDevice { id })
    }

    /// The logical device's id.
    pub fn id(&self) -> AudioDeviceID {
        self.id
    }

    /// See [`audio_device_name`].
    pub fn name(&self) -> Result<String> {
        audio_device_name(self.id)
    }

    /// See [`audio_device_format`].
    pub fn format(&self) -> Result<(AudioSpec, i32)> {
        audio_device_format(self.id)
    }

    /// See [`audio_device_channel_map`].
    pub fn channel_map(&self) -> Option<Vec<i32>> {
        audio_device_channel_map(self.id)
    }

    /// See [`audio_device_properties`].
    pub fn properties(&self) -> Result<Properties> {
        audio_device_properties(self.id)
    }

    /// See [`pause_audio_device`].
    pub fn pause(&self) -> Result<()> {
        pause_audio_device(self.id)
    }

    /// See [`resume_audio_device`].
    pub fn resume(&self) -> Result<()> {
        resume_audio_device(self.id)
    }

    /// See [`audio_device_paused`].
    pub fn paused(&self) -> bool {
        audio_device_paused(self.id)
    }

    /// See [`audio_device_gain`].
    pub fn gain(&self) -> Result<f32> {
        audio_device_gain(self.id)
    }

    /// See [`set_audio_device_gain`].
    pub fn set_gain(&self, gain: f32) -> Result<()> {
        set_audio_device_gain(self.id, gain)
    }

    /// See [`set_audio_postmix_callback`].
    pub fn set_postmix_callback(
        &self,
        callback: Option<impl Fn(&AudioSpec, &mut [f32]) + Send + Sync + 'static>,
    ) -> Result<()> {
        set_audio_postmix_callback(self.id, callback)
    }

    /// Bind one stream; see [`bind_audio_stream`].
    pub fn bind(&self, stream: &AudioStream) -> Result<()> {
        bind_audio_stream(self.id, stream)
    }

    /// Bind several streams at once; see [`bind_audio_streams`].
    pub fn bind_all(&self, streams: &[&AudioStream]) -> Result<()> {
        bind_audio_streams(self.id, streams)
    }

    /// Close the device now; see [`close_audio_device`].
    pub fn close(self) {
        // (Drop does the work)
    }
}

impl Drop for AudioDevice {
    fn drop(&mut self) {
        close_audio_device(self.id);
    }
}
