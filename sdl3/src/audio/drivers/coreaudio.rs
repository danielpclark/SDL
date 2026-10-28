// Rust translation of src/audio/coreaudio/SDL_coreaudio.m and
// src/audio/coreaudio/SDL_coreaudio.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The CoreAudio driver, on every Apple platform: an AudioQueue per
//! opened device, run on a thread of the driver's own with its CFRunLoop
//! (`ProvidesOwnCallbackThread`).
//!
//! On macOS (upstream's `MACOSX_COREAUDIO`) the devices come from the
//! HAL's AudioObject properties, with listeners for hotplug, default
//! device changes and lost devices, and each queue is assigned its device.
//! On the UIKit platforms (iOS, tvOS, visionOS) there is the default
//! playback and the default recording device only, and opening and
//! closing devices manages the app's `AVAudioSession` (its category from
//! [`hints::AUDIO_CATEGORY`](crate::hints::AUDIO_CATEGORY) and
//! [`hints::AUDIO_DUCK_OTHERS`](crate::hints::AUDIO_DUCK_OTHERS), its
//! activation, the preferred sample rate) and listens for interruptions:
//! see the `session` module.
//!
//! Two things move because the core holds the device lock while it opens a
//! device (as upstream does), and the Rust core's thread helpers take that
//! lock where upstream's don't:
//!
//! * the queue thread runs the default `ThreadInit` itself (what
//!   `SDL_PlaybackAudioThreadSetup()` and `SDL_RecordingAudioThreadSetup()`
//!   do) instead of calling those helpers, which would wait for the lock the
//!   opening thread holds while it waits for the queue thread;
//! * `SDL_UpdatedAudioDeviceFormat()` runs in `OpenDevice` before the thread
//!   starts (nothing changes the format in between), and the channel map
//!   `PrepareAudioQueue()` picks for 7.1/6.1 on old systems is handed back
//!   to `OpenDevice`, which stores it once the thread is ready.

#[cfg(not(target_os = "macos"))]
mod session;
mod sys;

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use self::sys::*;
#[cfg(target_os = "macos")]
use crate::audio::device::{
    audio_device_disconnected, default_audio_device_changed, find_physical_audio_device_by_callback,
};
use crate::audio::device::{
    default_thread_init, find_physical_audio_device_by_handle, get_audio_thread_name,
    playback_audio_thread_iterate, playback_audio_thread_shutdown, recording_audio_thread_iterate,
    recording_audio_thread_shutdown, updated_audio_device_format, AudioBootStrap, AudioDriverImpl,
    DeviceBackend, DriverFlags, PhysState, PhysicalDevice,
};
use crate::audio::queue::store_chmap;
use crate::audio::{AudioFormat, AudioSpec};
use crate::core::apple::foundation::{
    available, kCFRunLoopDefaultMode, CFRunLoopGetCurrent, CFRunLoopRunInMode, CFTimeInterval,
};
#[cfg(target_os = "macos")]
use crate::core::apple::foundation::{string_from_cf, CFRelease, CFStringRef};
#[cfg(not(target_os = "macos"))]
use crate::core::apple::objc::StrongPtr;

use crate::error::{Error, Result};
use crate::thread::Thread;

const DEBUG_COREAUDIO: bool = false;

/// Translation of `CHECK_RESULT()`: an error unless `result` is `noErr`.
fn check_result(result: OSStatus, msg: &str) -> Result<()> {
    if result != noErr {
        if DEBUG_COREAUDIO {
            crate::log!("COREAUDIO: Got error {} from '{}'!", result, msg);
        }
        return Err(Error::new(format!("CoreAudio error ({msg}): {result}")));
    }
    Ok(())
}

// Apparently AudioDeviceID values might not be unique, so we wrap it in an SDL_malloc()'d pointer
//  to make it so. Use FindCoreAudioDeviceByHandle to deal with this redirection, if you need to
//  map from an AudioDeviceID to a SDL handle.
/// Translation of `SDLCoreAudioHandle`. The device's handle is the address
/// of a boxed one; it is also the client data of the device's
/// `kAudioDevicePropertyDeviceIsAlive` listener (upstream passes the
/// `SDL_AudioDevice`, which the handle finds here).
#[cfg(target_os = "macos")]
struct CoreAudioHandle {
    devid: AudioDeviceID,
    recording: bool,
}

/// The `SDLCoreAudioHandle` behind a device's handle.
///
/// # Safety
///
/// While this driver is the current one, every physical device's handle
/// is a `CoreAudioHandle` it boxed, freed only by `FreeDeviceHandle`.
#[cfg(target_os = "macos")]
unsafe fn core_audio_handle<'a>(handle: usize) -> &'a CoreAudioHandle {
    // SAFETY: the caller's contract.
    unsafe { &*(handle as *const CoreAudioHandle) }
}

/// Translation of `TestCoreAudioDeviceHandleCallback()`.
#[cfg(target_os = "macos")]
fn test_core_audio_device_handle_callback(
    device: &PhysicalDevice,
    handle: &CoreAudioHandle,
) -> bool {
    // SAFETY: every device of this driver has a boxed handle.
    let a = unsafe { core_audio_handle(device.handle) };
    let b = handle;
    (a.devid == b.devid) && (a.recording == b.recording)
}

/// Translation of `FindCoreAudioDeviceByHandle()`.
#[cfg(target_os = "macos")]
fn find_core_audio_device_by_handle(
    devid: AudioDeviceID,
    recording: bool,
) -> Option<Arc<PhysicalDevice>> {
    let handle = CoreAudioHandle { devid, recording };
    find_physical_audio_device_by_callback(|d| test_core_audio_device_handle_callback(d, &handle))
        .ok()
}

/// Translation of `devlist_address`.
#[cfg(target_os = "macos")]
static DEVLIST_ADDRESS: AudioObjectPropertyAddress = AudioObjectPropertyAddress {
    mSelector: kAudioHardwarePropertyDevices,
    mScope: kAudioObjectPropertyScopeGlobal,
    mElement: kAudioObjectPropertyElementMain,
};

/// Translation of `default_playback_device_address`.
#[cfg(target_os = "macos")]
static DEFAULT_PLAYBACK_DEVICE_ADDRESS: AudioObjectPropertyAddress = AudioObjectPropertyAddress {
    mSelector: kAudioHardwarePropertyDefaultOutputDevice,
    mScope: kAudioObjectPropertyScopeGlobal,
    mElement: kAudioObjectPropertyElementMain,
};

/// Translation of `default_recording_device_address`.
#[cfg(target_os = "macos")]
static DEFAULT_RECORDING_DEVICE_ADDRESS: AudioObjectPropertyAddress = AudioObjectPropertyAddress {
    mSelector: kAudioHardwarePropertyDefaultInputDevice,
    mScope: kAudioObjectPropertyScopeGlobal,
    mElement: kAudioObjectPropertyElementMain,
};

/// Translation of `alive_address`.
#[cfg(target_os = "macos")]
static ALIVE_ADDRESS: AudioObjectPropertyAddress = AudioObjectPropertyAddress {
    mSelector: kAudioDevicePropertyDeviceIsAlive,
    mScope: kAudioObjectPropertyScopeGlobal,
    mElement: kAudioObjectPropertyElementMain,
};

/// Translation of `DeviceAliveNotification()`.
#[cfg(target_os = "macos")]
unsafe extern "C" fn device_alive_notification(
    devid: AudioObjectID,
    _num_addr: u32,
    addrs: *const AudioObjectPropertyAddress,
    data: *mut c_void,
) -> OSStatus {
    let Some(device) = find_physical_audio_device_by_handle(data as usize) else {
        return noErr; // (already gone)
    };
    // SAFETY: `data` is the device's handle (see CoreAudioHandle).
    crate::sdl_assert!(unsafe { core_audio_handle(data as usize) }.devid == devid);

    let mut alive: u32 = 1;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: an address CoreAudio passed us and an out value of its size.
    let error = unsafe {
        AudioObjectGetPropertyData(
            devid,
            addrs,
            0,
            ptr::null(),
            &mut size,
            &mut alive as *mut u32 as *mut c_void,
        )
    };

    let mut dead = false;
    if error == kAudioHardwareBadDeviceError {
        dead = true; // device was unplugged.
    } else if (error == kAudioHardwareNoError) && (alive == 0) {
        dead = true; // device died in some other way.
    }

    if dead {
        if DEBUG_COREAUDIO {
            crate::log!("COREAUDIO: device '{}' is lost!", device.name);
        }
        audio_device_disconnected(&device);
    }

    noErr
}

/// Translation of `COREAUDIO_FreeDeviceHandle()`.
#[cfg(target_os = "macos")]
fn coreaudio_free_device_handle(device: &PhysicalDevice) {
    let handle = device.handle as *mut CoreAudioHandle;
    // SAFETY: the box add_audio_device was given; the listener goes first,
    // then the box, once.
    unsafe {
        AudioObjectRemovePropertyListener(
            (*handle).devid,
            &ALIVE_ADDRESS,
            device_alive_notification,
            handle as *mut c_void,
        );
        drop(Box::from_raw(handle));
    }
}

/// Translation of `GetAudioDeviceStringProperty()`.
#[cfg(target_os = "macos")]
fn get_audio_device_string_property(
    dev: AudioDeviceID,
    addr: &AudioObjectPropertyAddress,
) -> Option<String> {
    let mut cfstr: CFStringRef = ptr::null();
    let mut size = size_of::<CFStringRef>() as u32;
    // SAFETY: an out value of the size given.
    let r = unsafe {
        AudioObjectGetPropertyData(
            dev,
            addr,
            0,
            ptr::null(),
            &mut size,
            &mut cfstr as *mut CFStringRef as *mut c_void,
        )
    };
    if r != kAudioHardwareNoError {
        return None;
    }

    // SAFETY: the HAL hands back a CFString we own.
    unsafe {
        let retval = string_from_cf(cfstr);
        if !cfstr.is_null() {
            CFRelease(cfstr);
        }
        retval
    }
}

/// The size of a property, `AudioObjectGetPropertyDataSize()`.
#[cfg(target_os = "macos")]
fn property_data_size(dev: AudioObjectID, addr: &AudioObjectPropertyAddress) -> Option<u32> {
    let mut size = 0u32;
    // SAFETY: a valid address and out value.
    let r = unsafe { AudioObjectGetPropertyDataSize(dev, addr, 0, ptr::null(), &mut size) };
    (r == kAudioHardwareNoError).then_some(size)
}

/// The total channel count of an `AudioBufferList` read into `buf`.
#[cfg(target_os = "macos")]
fn stream_configuration_channels(buf: &[u64]) -> i32 {
    let list = buf.as_ptr() as *const AudioBufferList;
    let mut channels = 0;
    // SAFETY: the HAL filled `buf` with an AudioBufferList whose
    // mNumberBuffers buffers fit in it (it told us the size).
    unsafe {
        let n = (*list).mNumberBuffers as usize;
        let first = ptr::addr_of!((*list).mBuffers) as *const AudioBuffer;
        let room = (buf.len() * 8 - size_of::<u64>()) / size_of::<AudioBuffer>();
        for j in 0..n.min(room) {
            channels += (*first.add(j)).mNumberChannels as i32;
        }
    }
    channels
}

/// Translation of `RefreshPhysicalDevices()`.
///
/// This only _adds_ new devices. Removal is handled by devices triggering kAudioDevicePropertyDeviceIsAlive property changes.
#[cfg(target_os = "macos")]
fn refresh_physical_devices() {
    let Some(size) = property_data_size(kAudioObjectSystemObject, &DEVLIST_ADDRESS) else {
        return;
    };
    let mut devs = vec![0 as AudioDeviceID; size as usize / size_of::<AudioDeviceID>()];
    let mut size = (devs.len() * size_of::<AudioDeviceID>()) as u32;
    // SAFETY: an out array of `size` bytes.
    if unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject,
            &DEVLIST_ADDRESS,
            0,
            ptr::null(),
            &mut size,
            devs.as_mut_ptr() as *mut c_void,
        )
    } != kAudioHardwareNoError
    {
        return;
    }

    let total_devices = size as usize / size_of::<AudioDeviceID>();
    devs.truncate(total_devices);
    for dev in devs.iter_mut() {
        if find_core_audio_device_by_handle(*dev, true).is_some()
            || find_core_audio_device_by_handle(*dev, false).is_some()
        {
            *dev = 0; // The system and SDL both agree it's already here, don't check it again.
        }
    }

    // any non-zero items remaining in `devs` are new devices to be added.
    for recording in [false, true] {
        let mut addr = AudioObjectPropertyAddress {
            mSelector: 0,
            mScope: if recording {
                kAudioDevicePropertyScopeInput
            } else {
                kAudioDevicePropertyScopeOutput
            },
            mElement: kAudioObjectPropertyElementMain,
        };

        for (i, &dev) in devs.iter().enumerate() {
            if dev == 0 {
                continue; // already added.
            }

            addr.mSelector = kAudioDevicePropertyStreamConfiguration;
            let Some(size) = property_data_size(dev, &addr) else {
                continue;
            };
            let mut buflist = vec![0u64; (size as usize).div_ceil(8).max(1)];
            let mut size = size;

            // SAFETY: an out buffer of `size` bytes, aligned for the list.
            let result = unsafe {
                AudioObjectGetPropertyData(
                    dev,
                    &addr,
                    0,
                    ptr::null(),
                    &mut size,
                    buflist.as_mut_ptr() as *mut c_void,
                )
            };

            let mut spec = AudioSpec::new(AudioFormat::UNKNOWN, 0, 0);
            if result == noErr {
                spec.channels = stream_configuration_channels(&buflist);
            }

            if spec.channels == 0 {
                continue;
            }

            let mut sample_rate: f64 = 0.0;
            let mut size = size_of::<f64>() as u32;
            addr.mSelector = kAudioDevicePropertyNominalSampleRate;
            // SAFETY: an out value of the size given.
            if unsafe {
                AudioObjectGetPropertyData(
                    dev,
                    &addr,
                    0,
                    ptr::null(),
                    &mut size,
                    &mut sample_rate as *mut f64 as *mut c_void,
                )
            } == noErr
            {
                spec.freq = sample_rate as i32;
            }

            addr.mSelector = kAudioObjectPropertyName;
            let Some(name) = get_audio_device_string_property(dev, &addr) else {
                continue;
            };

            addr.mSelector = kAudioDevicePropertyDeviceUID;
            let unique_id = get_audio_device_string_property(dev, &addr); // it's okay if this one fails.

            // Some devices have whitespace at the end...trim it.
            let mut name = Some(name.trim_end_matches(' ').to_owned());

            if name.as_deref() == Some("") {
                // name is blank?!
                name = unique_id.clone().filter(|u| !u.is_empty());
            }

            if DEBUG_COREAUDIO {
                crate::log!(
                    "COREAUDIO: Found {} device #{}: '{}' (devid {}, unique_id='{}')",
                    if recording { "recording" } else { "playback" },
                    i,
                    name.as_deref().unwrap_or("(null)"),
                    dev,
                    unique_id.as_deref().unwrap_or("(null)")
                );
            }
            let newhandle = Box::into_raw(Box::new(CoreAudioHandle {
                devid: dev,
                recording,
            }));
            let device = crate::audio::device::add_audio_device(
                recording,
                name.as_deref().unwrap_or("Unnamed audio device"),
                unique_id.as_deref(),
                Some(&spec),
                newhandle as usize,
            );
            if device.is_some() {
                // SAFETY: a static address and our listener, with the
                // handle (which outlives it) as its data.
                unsafe {
                    AudioObjectAddPropertyListener(
                        dev,
                        &ALIVE_ADDRESS,
                        device_alive_notification,
                        newhandle as *mut c_void,
                    )
                };
            } else {
                // SAFETY: the box just made; nothing else has it.
                drop(unsafe { Box::from_raw(newhandle) });
            }
        }
    }
}

/// Translation of `DeviceListChangedNotification()`: this is called when the system's list of available audio devices changes.
#[cfg(target_os = "macos")]
unsafe extern "C" fn device_list_changed_notification(
    _system_obj: AudioObjectID,
    _num_addr: u32,
    _addrs: *const AudioObjectPropertyAddress,
    _data: *mut c_void,
) -> OSStatus {
    refresh_physical_devices();
    noErr
}

/// Translation of `DefaultAudioDeviceChangedNotification()`.
#[cfg(target_os = "macos")]
fn default_audio_device_changed_notification(
    recording: bool,
    in_object_id: AudioObjectID,
    addr: *const AudioObjectPropertyAddress,
) -> OSStatus {
    let mut devid: AudioDeviceID = 0;
    let mut size = size_of::<AudioDeviceID>() as u32;
    // SAFETY: the address CoreAudio passed and an out value of its size.
    if unsafe {
        AudioObjectGetPropertyData(
            in_object_id,
            addr,
            0,
            ptr::null(),
            &mut size,
            &mut devid as *mut AudioDeviceID as *mut c_void,
        )
    } == noErr
    {
        // (SDL_DefaultAudioDeviceChanged(NULL) does nothing)
        if let Some(device) = find_core_audio_device_by_handle(devid, recording) {
            default_audio_device_changed(&device);
        }
    }
    noErr
}

/// Translation of `DefaultPlaybackDeviceChangedNotification()`.
#[cfg(target_os = "macos")]
unsafe extern "C" fn default_playback_device_changed_notification(
    in_object_id: AudioObjectID,
    in_number_addresses: u32,
    in_addresses: *const AudioObjectPropertyAddress,
    _in_user_data: *mut c_void,
) -> OSStatus {
    if DEBUG_COREAUDIO {
        crate::log!("COREAUDIO: default playback device changed!");
    }
    crate::sdl_assert!(in_number_addresses == 1);
    default_audio_device_changed_notification(false, in_object_id, in_addresses)
}

/// Translation of `DefaultRecordingDeviceChangedNotification()`.
#[cfg(target_os = "macos")]
unsafe extern "C" fn default_recording_device_changed_notification(
    in_object_id: AudioObjectID,
    in_number_addresses: u32,
    in_addresses: *const AudioObjectPropertyAddress,
    _in_user_data: *mut c_void,
) -> OSStatus {
    if DEBUG_COREAUDIO {
        crate::log!("COREAUDIO: default recording device changed!");
    }
    crate::sdl_assert!(in_number_addresses == 1);
    default_audio_device_changed_notification(true, in_object_id, in_addresses)
}

/// The default device of a kind (`kAudioHardwarePropertyDefault*Device`).
#[cfg(target_os = "macos")]
fn system_default_device(addr: &AudioObjectPropertyAddress) -> Option<AudioDeviceID> {
    let mut devid: AudioDeviceID = 0;
    let mut size = size_of::<AudioDeviceID>() as u32;
    // SAFETY: a static address and an out value of its size.
    let r = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject,
            addr,
            0,
            ptr::null(),
            &mut size,
            &mut devid as *mut AudioDeviceID as *mut c_void,
        )
    };
    (r == noErr).then_some(devid)
}

/// Translation of `COREAUDIO_DetectDevices()`.
#[cfg(target_os = "macos")]
fn coreaudio_detect_devices() -> (Option<Arc<PhysicalDevice>>, Option<Arc<PhysicalDevice>>) {
    let mut default_playback = None;
    let mut default_recording = None;

    refresh_physical_devices();

    // SAFETY: a static address and our listener.
    unsafe {
        AudioObjectAddPropertyListener(
            kAudioObjectSystemObject,
            &DEVLIST_ADDRESS,
            device_list_changed_notification,
            ptr::null_mut(),
        )
    };

    // Get the Device ID
    if let Some(devid) = system_default_device(&DEFAULT_PLAYBACK_DEVICE_ADDRESS) {
        if let Some(device) = find_core_audio_device_by_handle(devid, false) {
            default_playback = Some(device);
        }
    }
    // SAFETY: as above.
    unsafe {
        AudioObjectAddPropertyListener(
            kAudioObjectSystemObject,
            &DEFAULT_PLAYBACK_DEVICE_ADDRESS,
            default_playback_device_changed_notification,
            ptr::null_mut(),
        )
    };

    if let Some(devid) = system_default_device(&DEFAULT_RECORDING_DEVICE_ADDRESS) {
        if let Some(device) = find_core_audio_device_by_handle(devid, true) {
            default_recording = Some(device);
        }
    }
    // SAFETY: as above.
    unsafe {
        AudioObjectAddPropertyListener(
            kAudioObjectSystemObject,
            &DEFAULT_RECORDING_DEVICE_ADDRESS,
            default_recording_device_changed_notification,
            ptr::null_mut(),
        )
    };

    (default_playback, default_recording)
}

/// Translation of `struct SDL_PrivateAudioData`: what the queue thread and
/// the queue's callbacks share with the device's entry points.
struct Shared {
    /// The device (for the `SDL_*AudioThreadIterate()` calls).
    device: Weak<PhysicalDevice>,
    recording: bool,
    audio_queue: AtomicPtr<c_void>,
    /// `audioBuffer` (`numAudioBuffers` of them; AudioQueueDispose() frees the actual buffer objects).
    audio_buffer: Mutex<Vec<usize>>,
    current_buffer: AtomicPtr<AudioQueueBuffer>,
    strdesc: AudioStreamBasicDescription,
    #[cfg(target_os = "macos")]
    device_id: AudioDeviceID,
    /// Whether an interruption paused the queue (`interrupted`).
    #[cfg(not(target_os = "macos"))]
    interrupted: std::sync::atomic::AtomicBool,
    /// The device's format, as `OpenDevice` left it (`device->spec`,
    /// `sample_frames`, `buffer_size`, `silence_value`).
    spec: AudioSpec,
    sample_frames: i32,
    buffer_size: usize,
    silence_value: u8,
}

impl Shared {
    fn queue(&self) -> AudioQueueRef {
        self.audio_queue.load(Ordering::Acquire)
    }

    /// Hand the current buffer back to the queue as it is.
    fn requeue(&self, buffer: AudioQueueBufferRef) {
        // SAFETY: a buffer of our live queue.
        unsafe { AudioQueueEnqueueBuffer(self.queue(), buffer, 0, ptr::null()) };
    }
}

/// An opened device: `SDL_PrivateAudioData` and the entry points. It is
/// made first thing in `OpenDevice` (`device->hidden`), the queue thread's
/// data once the device's format is settled.
struct CoreAudioDevice {
    /// (what `CountOpenAudioDevices()` reads of the device)
    #[cfg(not(target_os = "macos"))]
    recording: bool,
    shared: OnceLock<Arc<Shared>>,
    thread: Mutex<Option<Thread>>,
    /// The `SDLInterruptionListener` (`interruption_listener`).
    #[cfg(not(target_os = "macos"))]
    interruption_listener: Mutex<StrongPtr>,
}

impl DeviceBackend for CoreAudioDevice {
    /// The queue thread runs the default `ThreadInit` itself.
    fn thread_init(&self, _device: &PhysicalDevice) {}

    /// Translation of `COREAUDIO_GetDeviceBuf()`.
    fn get_device_buf(&self, _device: &PhysicalDevice, buffer_size: usize) -> Option<usize> {
        let current_buffer = self.shared().current_buffer.load(Ordering::Acquire);
        crate::sdl_assert!(!current_buffer.is_null()); // should have been called from PlaybackBufferReadyCallback
        if current_buffer.is_null() {
            return Some(0);
        }
        // SAFETY: the buffer the queue handed to our callback.
        unsafe {
            crate::sdl_assert!(!(*current_buffer).mAudioData.is_null());
            // (the core mixes into a buffer of its own, which PlayDevice copies)
            Some(((*current_buffer).mAudioDataBytesCapacity as usize).min(buffer_size))
        }
    }

    /// Translation of `COREAUDIO_PlayDevice()`.
    fn play_device(&self, _device: &PhysicalDevice, buffer: &[u8]) -> bool {
        let shared = self.shared();
        let current_buffer = shared.current_buffer.load(Ordering::Acquire);
        crate::sdl_assert!(!current_buffer.is_null()); // should have been called from PlaybackBufferReadyCallback
        if current_buffer.is_null() {
            return true;
        }
        // SAFETY: the queue's buffer has mAudioDataBytesCapacity bytes.
        unsafe {
            let cap = (*current_buffer).mAudioDataBytesCapacity as usize;
            let n = buffer.len().min(cap);
            ptr::copy_nonoverlapping(buffer.as_ptr(), (*current_buffer).mAudioData as *mut u8, n);
            (*current_buffer).mAudioDataByteSize = (*current_buffer).mAudioDataBytesCapacity;
        }
        shared
            .current_buffer
            .store(ptr::null_mut(), Ordering::Release);
        shared.requeue(current_buffer);
        true
    }

    /// Translation of `COREAUDIO_RecordDevice()`.
    fn record_device(&self, _device: &PhysicalDevice, buffer: &mut [u8]) -> Result<usize> {
        let shared = self.shared();
        let current_buffer = shared.current_buffer.load(Ordering::Acquire);
        crate::sdl_assert!(!current_buffer.is_null()); // should have been called from RecordingBufferReadyCallback
        if current_buffer.is_null() {
            return Ok(0);
        }
        // SAFETY: the queue's buffer holds mAudioDataByteSize recorded bytes.
        let cpy = unsafe {
            crate::sdl_assert!(!(*current_buffer).mAudioData.is_null());
            let size = (*current_buffer).mAudioDataByteSize as usize;
            crate::sdl_assert!(buffer.len() >= size); // `cpy` makes sure this won't overflow a buffer, but we _will_ drop samples if this assertion fails!
            let cpy = buffer.len().min(size);
            ptr::copy_nonoverlapping(
                (*current_buffer).mAudioData as *const u8,
                buffer.as_mut_ptr(),
                cpy,
            );
            cpy
        };
        shared
            .current_buffer
            .store(ptr::null_mut(), Ordering::Release);
        shared.requeue(current_buffer); // requeue for capturing more data later.
        Ok(cpy)
    }

    /// Translation of `COREAUDIO_FlushRecording()`.
    fn flush_recording(&self, _device: &PhysicalDevice) {
        flush_recording(self.shared());
    }

    /// Translation of `COREAUDIO_CloseDevice()`.
    fn close_device(&self, device: &PhysicalDevice) {
        crate::sdl_assert!(device.shutting_down()); // should have been set by SDL_audio.c
        let _ = self.close();
    }
}

/// `COREAUDIO_FlushRecording()` (also run from the recording callback).
fn flush_recording(shared: &Shared) {
    let current_buffer = shared
        .current_buffer
        .swap(ptr::null_mut(), Ordering::AcqRel);
    if !current_buffer.is_null() {
        // also gets called at shutdown, when no buffer is available.
        // just requeue the current buffer without reading from it, so it can be refilled with new data later.
        shared.requeue(current_buffer);
    }
}

impl CoreAudioDevice {
    /// The queue thread's data, made by `OpenDevice` (which hands the
    /// device to the core only once it is).
    fn shared(&self) -> &Arc<Shared> {
        self.shared.get().expect("an opened CoreAudio device")
    }

    /// What `COREAUDIO_CloseDevice()` does, which upstream's core also
    /// calls when `OpenDevice` fails. On UIKit, the error of updating the
    /// audio session, if that fails (upstream's `SDL_SetError()` there
    /// replaces a failed open's error).
    fn close(&self) -> Result<()> {
        #[cfg(not(target_os = "macos"))]
        session::remove_interruption_listener(
            &mut self
                .interruption_listener
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );

        // dispose of the audio queue before waiting on the thread, or it might stall for a long time!
        if let Some(shared) = self.shared.get() {
            let queue = shared.audio_queue.swap(ptr::null_mut(), Ordering::AcqRel);
            if !queue.is_null() {
                // SAFETY: our queue, disposed once.
                unsafe {
                    AudioQueueFlush(queue);
                    AudioQueueStop(queue, 0);
                    AudioQueueDispose(queue, 0);
                }
            }
        }

        let thread = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(thread) = thread {
            crate::sdl_assert!(self
                .shared
                .get()
                .and_then(|s| s.device.upgrade())
                .is_none_or(|d| d.shutting_down())); // should have been set by SDL_audio.c
            thread.wait();
        }

        #[cfg(not(target_os = "macos"))]
        let result = session::update_audio_session(true);
        #[cfg(target_os = "macos")]
        let result = Ok(());

        // AudioQueueDispose() frees the actual buffer objects.
        if let Some(shared) = self.shared.get() {
            shared
                .audio_buffer
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
        }
        #[cfg(not(target_os = "macos"))]
        session::free_hidden(self);
        result
    }
}

/// Translation of `PlaybackBufferReadyCallback()`.
unsafe extern "C" fn playback_buffer_ready_callback(
    in_user_data: *mut c_void,
    _in_aq: AudioQueueRef,
    in_buffer: AudioQueueBufferRef,
) {
    // SAFETY: the queue's user data is our Shared, alive while the queue is.
    let shared = unsafe { &*(in_user_data as *const Shared) };
    crate::sdl_assert!(!in_buffer.is_null()); // ...right?
    crate::sdl_assert!(shared.current_buffer.load(Ordering::Acquire).is_null()); // shouldn't have anything pending
    shared.current_buffer.store(in_buffer, Ordering::Release);
    let okay = match shared.device.upgrade() {
        Some(device) => playback_audio_thread_iterate(&device),
        None => false,
    };
    crate::sdl_assert!(shared.current_buffer.load(Ordering::Acquire).is_null() || !okay); // PlayDevice should have enqueued and cleaned it out, unless we failed or shutdown.

    // buffer is unexpectedly here? We're probably dying, but try to requeue this buffer with silence.
    let current_buffer = shared
        .current_buffer
        .swap(ptr::null_mut(), Ordering::AcqRel);
    if !current_buffer.is_null() {
        // SAFETY: the queue's buffer has mAudioDataBytesCapacity bytes.
        unsafe {
            ptr::write_bytes(
                (*current_buffer).mAudioData as *mut u8,
                shared.silence_value,
                (*current_buffer).mAudioDataBytesCapacity as usize,
            );
        }
        shared.requeue(current_buffer);
    }
}

/// Translation of `RecordingBufferReadyCallback()`.
unsafe extern "C" fn recording_buffer_ready_callback(
    in_user_data: *mut c_void,
    in_aq: AudioQueueRef,
    in_buffer: AudioQueueBufferRef,
    _in_start_time: *const AudioTimeStamp,
    _in_number_packet_descriptions: u32,
    _in_packet_descs: *const AudioStreamPacketDescription,
) {
    // SAFETY: as in playback_buffer_ready_callback.
    let shared = unsafe { &*(in_user_data as *const Shared) };
    crate::sdl_assert!(in_aq == shared.queue() || shared.queue().is_null());
    crate::sdl_assert!(!in_buffer.is_null()); // ...right?
    crate::sdl_assert!(shared.current_buffer.load(Ordering::Acquire).is_null()); // shouldn't have anything pending
    shared.current_buffer.store(in_buffer, Ordering::Release);
    let device = shared.device.upgrade();
    if let Some(device) = &device {
        recording_audio_thread_iterate(device);
    }

    // buffer is unexpectedly here? We're probably dying, but try to requeue this buffer anyhow.
    if !shared.current_buffer.load(Ordering::Acquire).is_null() {
        crate::sdl_assert!(device.as_ref().is_none_or(|d| d.shutting_down()));
        flush_recording(shared); // just flush it manually, which will requeue it.
    }
}

/// Translation of `PrepareDevice()`.
#[cfg(target_os = "macos")]
fn prepare_device(device: &PhysicalDevice) -> Result<AudioDeviceID> {
    crate::sdl_assert!(device.handle != 0); // this meant "system default" in SDL2, but doesn't anymore

    // SAFETY: a handle of this driver.
    let handle = unsafe { core_audio_handle(device.handle) };
    let devid = handle.devid;

    let mut addr = AudioObjectPropertyAddress {
        mSelector: 0,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };

    let mut alive: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    addr.mSelector = kAudioDevicePropertyDeviceIsAlive;
    addr.mScope = if device.recording {
        kAudioDevicePropertyScopeInput
    } else {
        kAudioDevicePropertyScopeOutput
    };
    // SAFETY: an out value of the size given.
    let result = unsafe {
        AudioObjectGetPropertyData(
            devid,
            &addr,
            0,
            ptr::null(),
            &mut size,
            &mut alive as *mut u32 as *mut c_void,
        )
    };
    check_result(
        result,
        "AudioDeviceGetProperty (kAudioDevicePropertyDeviceIsAlive)",
    )?;
    if alive == 0 {
        return Err(Error::new(
            "CoreAudio: requested device exists, but isn't alive.",
        ));
    }

    // some devices don't support this property, so errors are fine here.
    let mut pid: libc::pid_t = 0;
    let mut size = size_of::<libc::pid_t>() as u32;
    addr.mSelector = kAudioDevicePropertyHogMode;
    // SAFETY: as above.
    let result = unsafe {
        AudioObjectGetPropertyData(
            devid,
            &addr,
            0,
            ptr::null(),
            &mut size,
            &mut pid as *mut libc::pid_t as *mut c_void,
        )
    };
    if (result == noErr) && (pid != -1) {
        return Err(Error::new("CoreAudio: requested device is being hogged."));
    }

    Ok(devid)
}

/// Translation of `AssignDeviceToAudioQueue()`.
#[cfg(target_os = "macos")]
fn assign_device_to_audio_queue(shared: &Shared) -> Result<()> {
    let prop = AudioObjectPropertyAddress {
        mSelector: kAudioDevicePropertyDeviceUID,
        mScope: if shared.recording {
            kAudioDevicePropertyScopeInput
        } else {
            kAudioDevicePropertyScopeOutput
        },
        mElement: kAudioObjectPropertyElementMain,
    };

    let mut devuid: CFStringRef = ptr::null();
    let mut devuidsize = size_of::<CFStringRef>() as u32;
    // SAFETY: an out value of the size given.
    let result = unsafe {
        AudioObjectGetPropertyData(
            shared.device_id,
            &prop,
            0,
            ptr::null(),
            &mut devuidsize,
            &mut devuid as *mut CFStringRef as *mut c_void,
        )
    };
    check_result(
        result,
        "AudioObjectGetPropertyData (kAudioDevicePropertyDeviceUID)",
    )?;
    // SAFETY: our live queue, and the CFString the HAL gave us.
    let result = unsafe {
        let result = AudioQueueSetProperty(
            shared.queue(),
            kAudioQueueProperty_CurrentDevice,
            &devuid as *const CFStringRef as *const c_void,
            devuidsize,
        );
        CFRelease(devuid); // Release devuid; we're done with it and AudioQueueSetProperty should have retained if it wants to keep it.
        result
    };
    check_result(
        result,
        "AudioQueueSetProperty (kAudioQueueProperty_CurrentDevice)",
    )
}

/// The channel layout `PrepareAudioQueue()` sets for a channel count, and
/// the channel map converting SDL's layout to it, if one is needed
/// (`modern`: `@available(macOS 10.15, iOS 13.0, tvOS 13.0, *)`).
fn channel_layout(
    channels: i32,
    modern: bool,
) -> Result<(AudioChannelLayoutTag, Option<&'static [i32]>)> {
    Ok(match channels {
        // a standard mono stream
        1 => (kAudioChannelLayoutTag_Mono, None),
        // a standard stereo stream (L R) - implied playback
        2 => (kAudioChannelLayoutTag_Stereo, None),
        // L R LFE
        3 => (kAudioChannelLayoutTag_DVD_4, None),
        // front left, front right, back left, back right
        4 => (kAudioChannelLayoutTag_Quadraphonic, None),
        // L R LFE Ls Rs
        5 => (kAudioChannelLayoutTag_DVD_6, None),
        //  L R C LFE Ls Rs
        6 => (kAudioChannelLayoutTag_DVD_12, None),
        7 => {
            if modern {
                // L R C LFE Cs Ls Rs
                (kAudioChannelLayoutTag_WAVE_6_1, None)
            } else {
                // L R C LFE Ls Rs Cs
                // Convert from SDL channel layout to kAudioChannelLayoutTag_MPEG_6_1_A
                static SWIZZLE_MAP: [i32; 7] = [0, 1, 2, 3, 6, 4, 5];
                (kAudioChannelLayoutTag_MPEG_6_1_A, Some(&SWIZZLE_MAP[..]))
            }
        }
        8 => {
            if modern {
                // L R C LFE Rls Rrs Ls Rs
                (kAudioChannelLayoutTag_WAVE_7_1, None)
            } else {
                // L R C LFE Ls Rs Rls Rrs
                // Convert from SDL channel layout to kAudioChannelLayoutTag_MPEG_7_1_C
                static SWIZZLE_MAP: [i32; 8] = [0, 1, 2, 3, 6, 7, 4, 5];
                (kAudioChannelLayoutTag_MPEG_7_1_C, Some(&SWIZZLE_MAP[..]))
            }
        }
        _ => return Err(Error::new("Unsupported audio channels")),
    })
}

/// The shortest time the queued buffers hold, in milliseconds.
fn minimum_audio_buffer_time_ms() -> f64 {
    // Make sure we can feed the device a minimum amount of time
    #[allow(unused_mut)] // (changed on the UIKit platforms only)
    let mut minimum_audio_buffer_time_ms = 15.0;
    #[cfg(not(target_os = "macos"))]
    if session::older_ios_hardware() {
        // Older iOS hardware, use 40 ms as a minimum time
        minimum_audio_buffer_time_ms = 40.0;
    }
    minimum_audio_buffer_time_ms
}

/// How many buffers `PrepareAudioQueue()` queues for `sample_frames` frames
/// at `freq`, holding at least `minimum_audio_buffer_time_ms`.
fn num_audio_buffers(sample_frames: i32, freq: i32, minimum_audio_buffer_time_ms: f64) -> i32 {
    // we use THREE audio buffers by default, unlike most things that would
    // choose two alternating buffers, because it helps with issues on
    // Bluetooth headsets when recording and playing at the same time.
    // See conversation in #8192 for details.
    let mut num_audio_buffers = 3;
    let msecs = (sample_frames as f64 / freq as f64) * 1000.0;
    if msecs < minimum_audio_buffer_time_ms {
        // use more buffers if we have a VERY small sample set.
        num_audio_buffers =
            crate::stdlib::math::ceil(minimum_audio_buffer_time_ms / msecs) as i32 * 2;
    }
    num_audio_buffers
}

/// Translation of `PrepareAudioQueue()`; returns the channel map the
/// device needs, if any.
fn prepare_audio_queue(shared: &Arc<Shared>) -> Result<Option<&'static [i32]>> {
    let strdesc = &shared.strdesc;
    let recording = shared.recording;
    let user_data = Arc::as_ptr(shared) as *mut c_void;
    let mut queue: AudioQueueRef = ptr::null_mut();

    // SAFETY: CFRunLoopGetCurrent() has no preconditions.
    crate::sdl_assert!(!unsafe { CFRunLoopGetCurrent() }.is_null());

    // SAFETY: a valid description, our callbacks with the Shared (which
    // outlives the queue: close disposes of it first) as their data, this
    // thread's run loop, and an out pointer.
    let result = unsafe {
        if recording {
            AudioQueueNewInput(
                strdesc,
                recording_buffer_ready_callback,
                user_data,
                CFRunLoopGetCurrent(),
                kCFRunLoopDefaultMode,
                0,
                &mut queue,
            )
        } else {
            AudioQueueNewOutput(
                strdesc,
                playback_buffer_ready_callback,
                user_data,
                CFRunLoopGetCurrent(),
                kCFRunLoopDefaultMode,
                0,
                &mut queue,
            )
        }
    };
    check_result(
        result,
        if recording {
            "AudioQueueNewInput"
        } else {
            "AudioQueueNewOutput"
        },
    )?;
    shared.audio_queue.store(queue, Ordering::Release);

    #[cfg(target_os = "macos")]
    assign_device_to_audio_queue(shared)?;

    // (SDL_UpdatedAudioDeviceFormat(): OpenDevice did it before starting this thread.)

    // Set the channel layout for the audio queue
    let mut layout = AudioChannelLayout::default();
    let channels = shared.spec.channels;
    let modern = if channels >= 7 {
        available!(macos(10, 15), ios(13, 0), tvos(13, 0))
    } else {
        true
    };
    let (tag, chmap) = channel_layout(channels, modern)?;
    layout.mChannelLayoutTag = tag;
    if layout.mChannelLayoutTag != 0 {
        // SAFETY: our queue and a layout of the size given.
        let result = unsafe {
            AudioQueueSetProperty(
                queue,
                kAudioQueueProperty_ChannelLayout,
                &layout as *const AudioChannelLayout as *const c_void,
                size_of::<AudioChannelLayout>() as u32,
            )
        };
        check_result(
            result,
            "AudioQueueSetProperty(kAudioQueueProperty_ChannelLayout)",
        )?;
    }

    let num_audio_buffers = num_audio_buffers(
        shared.sample_frames,
        shared.spec.freq,
        minimum_audio_buffer_time_ms(),
    );

    if DEBUG_COREAUDIO {
        crate::log!("COREAUDIO: numAudioBuffers == {}", num_audio_buffers);
    }

    for _ in 0..num_audio_buffers {
        let mut buffer: AudioQueueBufferRef = ptr::null_mut();
        // SAFETY: our queue and an out pointer.
        let result =
            unsafe { AudioQueueAllocateBuffer(queue, shared.buffer_size as u32, &mut buffer) };
        check_result(result, "AudioQueueAllocateBuffer")?;
        shared
            .audio_buffer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(buffer as usize);
        // SAFETY: the buffer just allocated, mAudioDataBytesCapacity bytes.
        let result = unsafe {
            ptr::write_bytes(
                (*buffer).mAudioData as *mut u8,
                shared.silence_value,
                (*buffer).mAudioDataBytesCapacity as usize,
            );
            (*buffer).mAudioDataByteSize = (*buffer).mAudioDataBytesCapacity;
            // !!! FIXME: should we use AudioQueueEnqueueBufferWithParameters and specify all frames be "trimmed" so these are immediately ready to refill with SDL callback data?
            AudioQueueEnqueueBuffer(queue, buffer, 0, ptr::null())
        };
        check_result(result, "AudioQueueEnqueueBuffer")?;
    }

    // SAFETY: our queue.
    let result = unsafe { AudioQueueStart(queue, ptr::null()) };
    check_result(result, "AudioQueueStart")?;

    Ok(chmap) // We're running!
}

/// Translation of `AudioQueueThreadEntry()`.
fn audio_queue_thread_entry(
    shared: Arc<Shared>,
    ready: mpsc::Sender<std::result::Result<Option<&'static [i32]>, Error>>,
) -> i32 {
    let Some(device) = shared.device.upgrade() else {
        let _ = ready.send(Err(Error::new("CoreAudio: the device went away")));
        return 0;
    };

    // (SDL_RecordingAudioThreadSetup()/SDL_PlaybackAudioThreadSetup(): the
    // default ThreadInit, called directly; see the module notes.)
    default_thread_init(&device);

    match prepare_audio_queue(&shared) {
        Err(e) => {
            let _ = ready.send(Err(e));
            return 0;
        }
        Ok(chmap) => {
            // init was successful, alert parent thread and start running...
            let _ = ready.send(Ok(chmap));
        }
    }

    // This would be WaitDevice/WaitRecordingDevice in the normal SDL audio thread, but we get *BufferReadyCallback calls here to know when to iterate.
    while !device.shutting_down() {
        // SAFETY: runs this thread's run loop.
        unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.10, 1) };
    }

    if shared.recording {
        recording_audio_thread_shutdown(&device);
    } else {
        // Drain off any pending playback.
        let secs =
            (shared.sample_frames as CFTimeInterval / shared.spec.freq as CFTimeInterval) * 2.0;
        // SAFETY: as above.
        unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, secs, 0) };
        playback_audio_thread_shutdown(&device);
    }

    0
}

/// The `AudioStreamBasicDescription` `OpenDevice` sets up for `spec`, and
/// the format it settled on (`None` if CoreAudio takes none of the
/// requested format's relatives).
fn stream_description(spec: &AudioSpec) -> Option<(AudioFormat, AudioStreamBasicDescription)> {
    // Setup a AudioStreamBasicDescription with the requested format
    let mut strdesc = AudioStreamBasicDescription {
        mFormatID: kAudioFormatLinearPCM,
        mFormatFlags: kLinearPCMFormatFlagIsPacked,
        mChannelsPerFrame: spec.channels as u32,
        mSampleRate: spec.freq as f64,
        mFramesPerPacket: 1,
        ..AudioStreamBasicDescription::default()
    };

    let test_format = spec.format.closest_formats().iter().copied().find(|&f| {
        // CoreAudio handles most of SDL's formats natively.
        matches!(
            f,
            AudioFormat::U8
                | AudioFormat::S8
                | AudioFormat::S16LE
                | AudioFormat::S16BE
                | AudioFormat::S32LE
                | AudioFormat::S32BE
                | AudioFormat::F32LE
                | AudioFormat::F32BE
        )
    })?;

    strdesc.mBitsPerChannel = test_format.bitsize();
    if test_format.is_big_endian() {
        strdesc.mFormatFlags |= kLinearPCMFormatFlagIsBigEndian;
    }

    if test_format.is_float() {
        strdesc.mFormatFlags |= kLinearPCMFormatFlagIsFloat;
    } else if test_format.is_signed() {
        strdesc.mFormatFlags |= kLinearPCMFormatFlagIsSignedInteger;
    }

    strdesc.mBytesPerFrame = strdesc.mChannelsPerFrame * strdesc.mBitsPerChannel / 8;
    strdesc.mBytesPerPacket = strdesc.mBytesPerFrame * strdesc.mFramesPerPacket;
    Some((test_format, strdesc))
}

/// Translation of `COREAUDIO_OpenDevice()`.
fn coreaudio_open_device(
    device: &PhysicalDevice,
    state: &mut PhysState,
) -> Result<Arc<dyn DeviceBackend>> {
    // Initialize all variables that we clean on shutdown
    let hidden = Arc::new(CoreAudioDevice {
        #[cfg(not(target_os = "macos"))]
        recording: device.recording,
        shared: OnceLock::new(),
        thread: Mutex::new(None),
        #[cfg(not(target_os = "macos"))]
        interruption_listener: Mutex::new(StrongPtr::nil()),
    });
    #[cfg(not(target_os = "macos"))]
    session::hidden_allocated(&hidden);

    match open_hidden(&hidden, device, state) {
        Ok(()) => Ok(hidden),
        Err(e) => {
            // (upstream's core closes a device whose OpenDevice failed,
            // CloseDevice() included; this core has no backend to close
            // then, so it happens here)
            let closed = hidden.close();
            #[cfg(target_os = "macos")]
            return closed.and(Err(e));
            // FIXME (upstream): CloseDevice()'s UpdateAudioSession() counts
            // the open devices with SDL_FindPhysicalAudioDeviceByCallback(),
            // whose callback never ends the search, so it sets "Device not
            // found": that replaces the error OpenDevice failed with (unless
            // updating the session fails, with an error of its own).
            #[cfg(not(target_os = "macos"))]
            {
                let _ = e;
                Err(closed
                    .err()
                    .unwrap_or_else(|| Error::new("Device not found")))
            }
        }
    }
}

/// The steps of `COREAUDIO_OpenDevice()` after `device->hidden` is made.
fn open_hidden(
    hidden: &Arc<CoreAudioDevice>,
    device: &PhysicalDevice,
    state: &mut PhysState,
) -> Result<()> {
    #[cfg(not(target_os = "macos"))]
    session::open_session(device.recording, state)?;

    let Some((test_format, strdesc)) = stream_description(&state.spec) else {
        // shouldn't happen, but just in case...
        return Err(Error::new("coreaudio: Unsupported audio format"));
    };
    state.spec.format = test_format;

    #[cfg(target_os = "macos")]
    let device_id = prepare_device(device)?;

    // (SDL_UpdatedAudioDeviceFormat(), upstream's first step on the queue
    // thread; the thread needs the results.)
    updated_audio_device_format(state);

    let shared = Arc::new(Shared {
        device: find_physical_audio_device_by_handle(device.handle)
            .as_ref()
            .map_or_else(Weak::new, Arc::downgrade),
        recording: device.recording,
        audio_queue: AtomicPtr::new(ptr::null_mut()),
        audio_buffer: Mutex::new(Vec::new()),
        current_buffer: AtomicPtr::new(ptr::null_mut()),
        strdesc,
        #[cfg(target_os = "macos")]
        device_id,
        #[cfg(not(target_os = "macos"))]
        interrupted: std::sync::atomic::AtomicBool::new(false),
        spec: state.spec,
        sample_frames: state.sample_frames,
        buffer_size: state.buffer_size,
        silence_value: state.silence_value,
    });
    let _ = hidden.shared.set(shared.clone());

    // This has to init in a new thread so it can get its own CFRunLoop. :/
    let (ready_tx, ready_rx) = mpsc::channel();
    let threadname = get_audio_thread_name(device);
    let thread_shared = shared.clone();
    let thread = Thread::spawn(threadname, move || {
        audio_queue_thread_entry(thread_shared, ready_tx)
    })?;
    *hidden.thread.lock().unwrap_or_else(|e| e.into_inner()) = Some(thread);

    let ready = ready_rx
        .recv()
        .unwrap_or_else(|_| Err(Error::new("CoreAudio: the audio queue thread ended")));
    match ready {
        Err(thread_error) => {
            let thread = hidden
                .thread
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            if let Some(thread) = thread {
                thread.wait();
            }
            Err(thread_error)
        }
        Ok(chmap) => {
            if let Some(chmap) = chmap {
                state.chmap = store_chmap(Some(chmap), state.spec.channels);
            }
            #[cfg(not(target_os = "macos"))]
            session::add_interruption_listener(hidden, &shared);
            Ok(())
        }
    }
}

/// Translation of `COREAUDIO_DeinitializeStart()` (which has nothing to
/// do on the UIKit platforms).
#[cfg(target_os = "macos")]
fn coreaudio_deinitialize_start() {
    // SAFETY: the listeners DetectDevices added, with the same data.
    unsafe {
        AudioObjectRemovePropertyListener(
            kAudioObjectSystemObject,
            &DEVLIST_ADDRESS,
            device_list_changed_notification,
            ptr::null_mut(),
        );
        AudioObjectRemovePropertyListener(
            kAudioObjectSystemObject,
            &DEFAULT_PLAYBACK_DEVICE_ADDRESS,
            default_playback_device_changed_notification,
            ptr::null_mut(),
        );
        AudioObjectRemovePropertyListener(
            kAudioObjectSystemObject,
            &DEFAULT_RECORDING_DEVICE_ADDRESS,
            default_recording_device_changed_notification,
            ptr::null_mut(),
        );
    }
}

/// The CoreAudio driver (`COREAUDIO_Init()`'s entry points).
struct CoreAudio;

impl AudioDriverImpl for CoreAudio {
    fn flags(&self) -> DriverFlags {
        DriverFlags {
            provides_own_callback_thread: true,
            has_recording_support: true,
            // (the UIKit platforms get the core's default devices)
            only_has_default_playback_device: cfg!(not(target_os = "macos")),
            only_has_default_recording_device: cfg!(not(target_os = "macos")),
        }
    }

    #[cfg(target_os = "macos")]
    fn detect_devices(&self) -> (Option<Arc<PhysicalDevice>>, Option<Arc<PhysicalDevice>>) {
        coreaudio_detect_devices()
    }

    fn open_device(
        &self,
        device: &PhysicalDevice,
        state: &mut PhysState,
    ) -> Result<Arc<dyn DeviceBackend>> {
        coreaudio_open_device(device, state)
    }

    #[cfg(target_os = "macos")]
    fn free_device_handle(&self, device: &PhysicalDevice) {
        coreaudio_free_device_handle(device);
    }

    #[cfg(target_os = "macos")]
    fn deinitialize_start(&self) {
        coreaudio_deinitialize_start();
    }
}

/// Translation of `COREAUDIO_Init()`.
fn coreaudio_init() -> Option<Arc<dyn AudioDriverImpl>> {
    Some(Arc::new(CoreAudio))
}

/// Translation of `COREAUDIO_bootstrap`.
pub(crate) static COREAUDIO_BOOTSTRAP: AudioBootStrap = AudioBootStrap {
    name: "coreaudio",
    desc: "CoreAudio",
    init: coreaudio_init,
    demand_only: false,
    is_preferred: false,
};

#[cfg(test)]
mod tests;
