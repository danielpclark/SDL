// Rust translation of src/audio/aaudio/SDL_aaudio.c and
// src/audio/aaudio/SDL_aaudio.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The AAudio driver (Android 8.1, API 27, and later): libaaudio loaded at
//! run time, an AAudio stream per opened device, whose data callback (on
//! AAudio's thread) moves the audio through a triple-buffered ring that the
//! device's thread fills (playback) or empties (recording), a semaphore
//! telling that thread when a buffer is free (or full). The error callback
//! (a lost device: the route changed, ...) only flags the device; its next
//! `PlayDevice` builds a new stream (a new default device) and tells the
//! core of the new format, and its next `RecordDevice` fails (the device is
//! lost). While the app is in the background the event pump pauses the
//! streams ([`pause_devices`], [`resume_devices`]: playback paused,
//! recording stopped). Recording asks for the `RECORD_AUDIO` permission
//! first, blocking until it's answered.
//!
//! Upstream's `ALLOW_MULTIPLE_ANDROID_AUDIO_DEVICES` is 0 ("this appears to
//! be broken right now"), so there are only the default devices, and no
//! hotplug: `Android_StartAudioHotplug()` (the audio manager's device
//! callback) isn't used, though `Deinitialize` unregisters it.
//!
//! The core owns the buffer handed to `PlayDevice` here, so it is copied
//! into the ring there (upstream's `GetDeviceBuf` hands out the ring's
//! buffer itself). The devices whose `hidden` exists are kept in a list of
//! the driver's (for the pause and resume, which upstream finds with
//! `SDL_FindPhysicalAudioDeviceByCallback()`), and each one's stream is
//! behind a mutex, which the pause and resume (on SDL's main thread) and
//! the rebuild after an error (on the device's thread) take.

mod sys;

use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicI32, AtomicPtr, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::Duration;

use self::sys::*;
use crate::audio::device::{
    audio_device_format_changed_while_opening, updated_audio_device_format, with_device_state,
    AudioBootStrap, AudioDriverImpl, DeviceBackend, DriverFlags, PhysState, PhysicalDevice,
};
use crate::audio::AudioFormat;
use crate::error::{Error, Result};
use crate::hints;
use crate::loadso::SharedObject;
use crate::log::Category;

/// `LIB_AAUDIO_SO`.
const LIB_AAUDIO_SO: &str = "libaaudio.so";

/// `SDL_FUNCTION` in `BuildAAudioStream()`'s errors.
const BUILD_AAUDIO_STREAM: &str = "BuildAAudioStream";

/// `ctx`: the library, while the driver is initialized (`ctx.handle`).
static CTX: RwLock<Option<Arc<AaudioLib>>> = RwLock::new(None);

fn ctx() -> Option<Arc<AaudioLib>> {
    CTX.read().unwrap_or_else(|e| e.into_inner()).clone()
}

fn set_ctx(lib: Option<Arc<AaudioLib>>) {
    *CTX.write().unwrap_or_else(|e| e.into_inner()) = lib;
}

/// The opened devices (those whose `device->hidden` exists).
static HIDDEN: Mutex<Vec<Arc<AaudioDevice>>> = Mutex::new(Vec::new());

fn hidden_devices() -> MutexGuard<'static, Vec<Arc<AaudioDevice>>> {
    HIDDEN.lock().unwrap_or_else(|e| e.into_inner())
}

// Debug (upstream's `LOGI()`, compiled out with `#if 0`)
macro_rules! logi {
    ($($arg:tt)*) => {};
}

/// A stream (`hidden->stream`), owned by its device.
struct StreamPtr(*mut AAudioStream);

// SAFETY: an AAudio stream may be used from any thread; the mutex around
// it orders the driver's calls.
unsafe impl Send for StreamPtr {}

/// The mixing buffer and its counters (the parts of `SDL_PrivateAudioData`
/// that `BuildAAudioStream()` makes anew for each stream), shared with the
/// stream's data callback.
struct Ring {
    /// Raw mixing buffer (`mixbuf`).
    mixbuf: UnsafeCell<Box<[u8]>>,
    /// `num_buffers * device->buffer_size`.
    mixbuf_bytes: usize,
    /// The device's `buffer_size`, `SDL_AUDIO_FRAMESIZE(device->spec)` and
    /// `silence_value` for this stream (what the callback reads of the
    /// device).
    buffer_size: usize,
    framesize: usize,
    silence_value: u8,
    callback_bytes: AtomicUsize,
    processed_bytes: AtomicUsize,
    semaphore: crate::thread::Semaphore,
}

// SAFETY: the data callback and the device's thread use parts of `mixbuf`
// the semaphore keeps apart (as upstream's), with the counters' release and
// acquire orderings for barriers; everything else is immutable or atomic.
unsafe impl Sync for Ring {}

impl Ring {
    fn buf(&self) -> *mut u8 {
        // SAFETY: the box is never replaced; only its bytes are written,
        // through this pointer.
        unsafe { (*self.mixbuf.get()).as_mut_ptr() }
    }
}

/// Translation of `struct SDL_PrivateAudioData`.
struct AaudioDevice {
    lib: Arc<AaudioLib>,
    recording: bool,
    stream: Mutex<StreamPtr>,
    /// The current stream's ring (null before the stream's first build, and
    /// between a lost stream and its new one); owned (`Box::into_raw`).
    ring: AtomicPtr<Ring>,
    error_callback_triggered: AtomicI32,
}

impl AaudioDevice {
    fn stream(&self) -> MutexGuard<'_, StreamPtr> {
        self.stream.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn ring(&self) -> Option<&Ring> {
        // SAFETY: the ring lives until it is replaced or freed, which only
        // happens with no stream running (see free_ring).
        unsafe { self.ring.load(Ordering::Acquire).as_ref() }
    }

    /// `SDL_aligned_free(hidden->mixbuf)` and
    /// `SDL_DestroySemaphore(hidden->semaphore)`, once the stream is closed.
    fn free_ring(&self) {
        let ring = self.ring.swap(ptr::null_mut(), Ordering::AcqRel);
        if !ring.is_null() {
            // SAFETY: Box::into_raw()'s, taken once (swapped out).
            drop(unsafe { Box::from_raw(ring) });
        }
    }

    /// `hidden->stream` stopped and closed, and set to NULL.
    fn close_stream(&self) {
        let mut stream = self.stream();
        if !stream.0.is_null() {
            // SAFETY: an open stream.
            unsafe {
                (self.lib.AAudioStream_requestStop)(stream.0);
                // !!! FIXME: do we have to wait for the state to change to make sure all buffered audio has played, or will close do this (or will the system do this after the close)?
                // !!! FIXME: also, will this definitely wait for a running data callback to finish, and then stop the callback from firing again?
                (self.lib.AAudioStream_close)(stream.0);
            }
            stream.0 = ptr::null_mut();
        }
    }
}

/// Translation of `AAUDIO_errorCallback()`.
unsafe extern "C" fn aaudio_error_callback(
    _stream: *mut AAudioStream,
    user_data: *mut c_void,
    error: aaudio_result_t,
) {
    // SAFETY: the device's `hidden`, which outlives its streams.
    let hidden = unsafe { &*(user_data as *const AaudioDevice) };
    logi!(
        "SDL AAUDIO_errorCallback: {} - {}",
        error,
        hidden.lib.result_text(error)
    );

    // You MUST NOT close the audio stream from this callback, so we cannot call SDL_AudioDeviceDisconnected here.
    // Just flag the device so we can kill it in PlayDevice instead.
    hidden
        .error_callback_triggered
        .store(error, Ordering::Release); // AAUDIO_OK is zero, so !triggered means no error.
    if let Some(ring) = hidden.ring() {
        ring.semaphore.signal(); // in case we're blocking in WaitDevice.
    }
}

/// Translation of `AAUDIO_dataCallback()`.
unsafe extern "C" fn aaudio_data_callback(
    _stream: *mut AAudioStream,
    user_data: *mut c_void,
    audio_data: *mut c_void,
    num_frames: i32,
) -> aaudio_data_callback_result_t {
    // SAFETY: the device's `hidden`, which outlives its streams.
    let hidden = unsafe { &*(user_data as *const AaudioDevice) };
    let Some(ring) = hidden.ring() else {
        return AAUDIO_CALLBACK_RESULT_CONTINUE;
    };
    let framesize = ring.framesize;
    let callback_bytes = num_frames.max(0) as usize * framesize;
    let cb = ring.callback_bytes.load(Ordering::Relaxed);
    let old_buffer_index = cb / ring.buffer_size;
    let mixbuf = ring.buf();

    if hidden.recording {
        let input = audio_data as *const u8;
        let processed = ring.processed_bytes.load(Ordering::Acquire);
        let available_bytes = ring.mixbuf_bytes - (cb - processed);
        let size = available_bytes.min(callback_bytes);
        let offset = cb % ring.mixbuf_bytes;
        let end = (offset + size) % ring.mixbuf_bytes;
        crate::sdl_assert!(size <= ring.mixbuf_bytes);

        //LOGI("Recorded %zu frames, %zu available, %zu max (%zu written, %zu read)", callback_bytes / framesize, available_bytes / framesize, hidden->mixbuf_bytes / framesize, hidden->callback_bytes / framesize, hidden->processed_bytes / framesize);

        // SAFETY: AAudio's `num_frames` frames of input, and the ring's
        // free part (the device's thread reads only the filled part).
        unsafe {
            if offset <= end {
                ptr::copy_nonoverlapping(input, mixbuf.add(offset), size);
            } else {
                let partial = ring.mixbuf_bytes - offset;
                ptr::copy_nonoverlapping(input, mixbuf.add(offset), partial);
                ptr::copy_nonoverlapping(input.add(partial), mixbuf, end);
            }
        }

        // (SDL_MemoryBarrierRelease())
        ring.callback_bytes.store(cb + size, Ordering::Release);

        if size < callback_bytes {
            logi!(
                "Audio recording overflow, dropped {} frames",
                (callback_bytes - size) / framesize
            );
        }
    } else {
        let output = audio_data as *mut u8;
        // (SDL_MemoryBarrierAcquire())
        let processed = ring.processed_bytes.load(Ordering::Acquire);
        let available_bytes = processed - cb;
        let size = available_bytes.min(callback_bytes);
        let offset = cb % ring.mixbuf_bytes;
        let end = (offset + size) % ring.mixbuf_bytes;
        crate::sdl_assert!(size <= ring.mixbuf_bytes);

        //LOGI("Playing %zu frames, %zu available, %zu max (%zu written, %zu read)", callback_bytes / framesize, available_bytes / framesize, hidden->mixbuf_bytes / framesize, hidden->processed_bytes / framesize, hidden->callback_bytes / framesize);

        // SAFETY: AAudio's buffer of `num_frames` frames, and the ring's
        // filled part (the device's thread writes only the free part).
        unsafe {
            if offset <= end {
                ptr::copy_nonoverlapping(mixbuf.add(offset), output, size);
            } else {
                let partial = ring.mixbuf_bytes - offset;
                ptr::copy_nonoverlapping(mixbuf.add(offset), output, partial);
                ptr::copy_nonoverlapping(mixbuf, output.add(partial), end);
            }
        }
        ring.callback_bytes.store(cb + size, Ordering::Release);

        if size < callback_bytes {
            logi!(
                "Audio playback underflow, missed {} frames",
                (callback_bytes - size) / framesize
            );
            // SAFETY: the rest of AAudio's buffer.
            unsafe {
                ptr::write_bytes(output.add(size), ring.silence_value, callback_bytes - size)
            };
        }
    }

    let new_buffer_index = ring.callback_bytes.load(Ordering::Relaxed) / ring.buffer_size;
    for _ in old_buffer_index..new_buffer_index {
        // Trigger audio processing
        ring.semaphore.signal();
    }

    AAUDIO_CALLBACK_RESULT_CONTINUE
}

/// The AAudio driver (`AAUDIO_Init()`).
struct Aaudio {
    lib: Arc<AaudioLib>,
}

/// An opened device's interface.
struct AaudioBackend {
    hidden: Arc<AaudioDevice>,
}

impl AaudioBackend {
    /// Translation of `RecoverAAudioDevice()`.
    fn recover_aaudio_device(&self, device: &PhysicalDevice) -> Result<()> {
        let hidden = &self.hidden;

        // attempt to build a new stream, in case there's a new default device.
        hidden.close_stream();

        hidden.free_ring();

        with_device_state(device, |st| {
            let prev_sample_frames = st.sample_frames;
            let prevspec = st.spec;

            build_aaudio_stream(hidden, device, st)?; // oh well, we tried.

            // we don't know the new device spec until we open the new device, so we saved off the old one and force it back
            // so SDL_AudioDeviceFormatChanged can set up all the important state if necessary and then set it back to the new spec.
            let new_sample_frames = st.sample_frames;
            let newspec = st.spec;

            st.sample_frames = prev_sample_frames;
            st.spec = prevspec;
            audio_device_format_changed_while_opening(device, st, &newspec, new_sample_frames)
            // (ugh, when it fails)
        })
    }
}

impl DeviceBackend for AaudioBackend {
    /// `impl->ThreadInit = Android_AudioThreadInit`.
    fn thread_init(&self, device: &PhysicalDevice) {
        crate::core::android::audio_thread_init(device);
    }

    /// Translation of `AAUDIO_WaitDevice()`.
    fn wait_device(&self, device: &PhysicalDevice) -> bool {
        while !device.shutting_down() {
            // this semaphore won't fire when the app is in the background (AAUDIO_PauseDevices was called).
            match self.hidden.ring() {
                Some(ring) => {
                    if ring
                        .semaphore
                        .wait_timeout(Some(Duration::from_millis(100)))
                    {
                        return true; // semaphore was signaled, let's go!
                    }
                }
                // (no stream: a failed rebuild)
                None => crate::timer::delay(Duration::from_millis(100)),
            }
            // Still waiting on the semaphore (or the system), check other things then wait again.
        }
        true
    }

    /// Translation of `AAUDIO_GetDeviceBuf()` (the core's buffer, copied
    /// into the ring by `play_device`).
    fn get_device_buf(&self, _device: &PhysicalDevice, buffer_size: usize) -> Option<usize> {
        Some(buffer_size)
    }

    /// Translation of `AAUDIO_PlayDevice()`.
    fn play_device(&self, device: &PhysicalDevice, buffer: &[u8]) -> bool {
        let hidden = &self.hidden;

        // AAUDIO_dataCallback picks up our work and unblocks AAUDIO_WaitDevice. But make sure we didn't fail here.
        let err = hidden.error_callback_triggered.load(Ordering::Acquire);
        if err != 0 {
            crate::error!(
                Category::Audio,
                "aaudio: Audio device triggered error {} ({})",
                err,
                hidden.lib.result_text(err)
            );

            if self.recover_aaudio_device(device).is_err() {
                return false; // oh well, we went down hard.
            }
        } else if let Some(ring) = hidden.ring() {
            // (the buffer GetDeviceBuf() gave: the ring's next one)
            let processed = ring.processed_bytes.load(Ordering::Relaxed);
            let offset = processed % ring.mixbuf_bytes;
            let len = buffer.len().min(ring.mixbuf_bytes - offset);
            // SAFETY: the ring's free part, which the semaphore keeps the
            // callback out of.
            unsafe { ptr::copy_nonoverlapping(buffer.as_ptr(), ring.buf().add(offset), len) };
            // (SDL_MemoryBarrierRelease())
            ring.processed_bytes
                .store(processed + buffer.len(), Ordering::Release);
        }
        true
    }

    /// `impl->WaitRecordingDevice = AAUDIO_WaitDevice`.
    fn wait_recording_device(&self, device: &PhysicalDevice) -> bool {
        self.wait_device(device)
    }

    /// Translation of `AAUDIO_RecordDevice()`.
    fn record_device(&self, _device: &PhysicalDevice, buffer: &mut [u8]) -> Result<usize> {
        let hidden = &self.hidden;

        // AAUDIO_dataCallback picks up our work and unblocks AAUDIO_WaitDevice. But make sure we didn't fail here.
        if hidden.error_callback_triggered.load(Ordering::Acquire) != 0 {
            hidden.error_callback_triggered.store(0, Ordering::Release);
            return Err(Error::new("AAudio device triggered an error"));
        }

        let Some(ring) = hidden.ring() else {
            return Err(Error::new("AAudio device has no stream"));
        };
        let buflen = buffer.len();
        crate::sdl_assert!(buflen == ring.buffer_size); // If this isn't true, we need to change semaphore trigger logic and account for wrapping copies here
        let processed = ring.processed_bytes.load(Ordering::Relaxed);
        let offset = processed % ring.mixbuf_bytes;
        // (SDL_MemoryBarrierAcquire())
        let _ = ring.callback_bytes.load(Ordering::Acquire);
        let len = buflen.min(ring.mixbuf_bytes - offset);
        // SAFETY: the ring's filled part, which the semaphore keeps the
        // callback out of.
        unsafe { ptr::copy_nonoverlapping(ring.buf().add(offset), buffer.as_mut_ptr(), len) };
        ring.processed_bytes
            .store(processed + buflen, Ordering::Release);
        Ok(buflen)
    }

    /// Translation of `AAUDIO_CloseDevice()`.
    fn close_device(&self, _device: &PhysicalDevice) {
        logi!("AAUDIO_CloseDevice");
        free_hidden(&self.hidden);
    }
}

/// The end of `AAUDIO_CloseDevice()` (and of a failed `OpenDevice`): the
/// stream closed, the ring freed, `device->hidden = NULL`.
fn free_hidden(hidden: &Arc<AaudioDevice>) {
    hidden.close_stream();
    hidden.free_ring();
    hidden_devices().retain(|d| !Arc::ptr_eq(d, hidden));
}

/// Translation of `SetOptionalStreamUsage()`.
fn set_optional_stream_usage(lib: &AaudioLib, builder: *mut AAudioStreamBuilder) {
    if let Some(set_usage) = lib.AAudioStreamBuilder_setUsage {
        // optional API: requires Android 28
        if let Some(hint) = hints::get(hints::AUDIO_DEVICE_STREAM_ROLE) {
            let mut usage = AAUDIO_USAGE_MEDIA; // covers most things, and is the system default.
            if hint.eq_ignore_ascii_case("Communications") || hint.eq_ignore_ascii_case("GameChat")
            {
                usage = AAUDIO_USAGE_VOICE_COMMUNICATION;
            } else if hint.eq_ignore_ascii_case("Game") {
                usage = AAUDIO_USAGE_GAME;
            }
            // SAFETY: a builder of AAudio_createStreamBuilder().
            unsafe { set_usage(builder, usage) };

            // !!! FIXME: I _think_ this is okay with the current set of usages we support, but the docs
            // !!! FIXME:  say you need to dip down into Java to call android.app.Activity.setVolumeControlStream(usage)
            // !!! FIXME:  so the physical volume buttons control this stream, but that might be more for special cases
            // !!! FIXME:  like notification sounds, etc, and it's possible you _don't_ want to override this for those
            // !!! FIXME:  special cases, too! We'll revisit if there are bug reports.
        }
    }
}

/// Translation of `BuildAAudioStream()`.
fn build_aaudio_stream(
    hidden: &Arc<AaudioDevice>,
    device: &PhysicalDevice,
    st: &mut PhysState,
) -> Result<()> {
    let lib = &hidden.lib;
    let recording = device.recording;

    hidden.error_callback_triggered.store(0, Ordering::Release);

    let mut builder: *mut AAudioStreamBuilder = ptr::null_mut();
    // SAFETY: an out-pointer for the builder.
    let res = unsafe { (lib.AAudio_createStreamBuilder)(&mut builder) };
    if res != AAUDIO_OK {
        logi!("SDL Failed AAudio_createStreamBuilder {}", res);
        return Err(Error::new(format!(
            "SDL Failed AAudio_createStreamBuilder {res}"
        )));
    } else if builder.is_null() {
        logi!("SDL Failed AAudio_createStreamBuilder - builder NULL");
        return Err(Error::new(
            "SDL Failed AAudio_createStreamBuilder - builder NULL",
        ));
    }

    if crate::core::android::ALLOW_MULTIPLE_ANDROID_AUDIO_DEVICES {
        let aaudio_device_id = device.handle as i32;
        logi!("Opening device id {}", aaudio_device_id);
        // SAFETY: the builder.
        unsafe { (lib.AAudioStreamBuilder_setDeviceId)(builder, aaudio_device_id) };
    }

    let mut format = if st.spec.format == AudioFormat::S32
        && crate::core::android::get_android_sdk_version() >= 31
    {
        AAUDIO_FORMAT_PCM_I32
    } else if st.spec.format == AudioFormat::F32 {
        AAUDIO_FORMAT_PCM_FLOAT
    } else {
        AAUDIO_FORMAT_PCM_I16 // sint16 is a safe bet for everything else.
    };
    let sample_frames = if hints::get(hints::AUDIO_DEVICE_SAMPLE_FRAMES).is_some() {
        st.sample_frames
    } else {
        // Use 20 ms for the default audio buffer size
        st.spec.freq / 50
    };
    let direction = if recording {
        AAUDIO_DIRECTION_INPUT
    } else {
        AAUDIO_DIRECTION_OUTPUT
    };
    let user_data = Arc::as_ptr(hidden) as *mut c_void;
    // SAFETY: the builder; the callbacks' user data is the device's
    // `hidden`, which outlives the stream (closed before it's freed).
    unsafe {
        (lib.AAudioStreamBuilder_setFormat)(builder, format);
        (lib.AAudioStreamBuilder_setSampleRate)(builder, st.spec.freq);
        (lib.AAudioStreamBuilder_setChannelCount)(builder, st.spec.channels);

        (lib.AAudioStreamBuilder_setBufferCapacityInFrames)(builder, 2 * sample_frames); // AAudio requires that the buffer capacity is at least
        (lib.AAudioStreamBuilder_setFramesPerDataCallback)(builder, sample_frames); //      twice the size of the data callback buffer size

        (lib.AAudioStreamBuilder_setDirection)(builder, direction);
        (lib.AAudioStreamBuilder_setErrorCallback)(builder, aaudio_error_callback, user_data);
        (lib.AAudioStreamBuilder_setDataCallback)(builder, aaudio_data_callback, user_data);
    }
    // Some devices have flat sounding audio when low latency mode is enabled, but this is a better experience for most people
    if hints::get_bool(hints::ANDROID_LOW_LATENCY_AUDIO, true) {
        crate::log!("Low latency audio enabled");
        // SAFETY: the builder.
        unsafe {
            (lib.AAudioStreamBuilder_setPerformanceMode)(
                builder,
                AAUDIO_PERFORMANCE_MODE_LOW_LATENCY,
            )
        };
    } else {
        crate::log!("Low latency audio disabled");
    }

    set_optional_stream_usage(lib, builder);

    if let (true, Some(set_input_preset)) = (recording, lib.AAudioStreamBuilder_setInputPreset) {
        // optional API: requires Android 28
        if let Some(hint) = hints::get(hints::ANDROID_AAUDIO_INPUT_PRESET) {
            let preset: aaudio_input_preset_t = crate::stdlib::atoi(&hint);
            if preset != 0 {
                // SAFETY: the builder.
                unsafe { set_input_preset(builder, preset) };
            }
        }
    }

    logi!(
        "AAudio Try to open {} hz {} {} channels samples {}",
        st.spec.freq,
        st.spec.format.name(),
        st.spec.channels,
        st.sample_frames
    );

    let mut stream = hidden.stream();
    // SAFETY: the builder, and an out-pointer for the stream.
    let res = unsafe { (lib.AAudioStreamBuilder_openStream)(builder, &mut stream.0) };
    if res != AAUDIO_OK {
        logi!("SDL Failed AAudioStreamBuilder_openStream {}", res);
        // SAFETY: the builder, deleted once.
        unsafe { (lib.AAudioStreamBuilder_delete)(builder) };
        return Err(Error::new(format!(
            "{BUILD_AAUDIO_STREAM} : {}",
            lib.result_text(res)
        )));
    }
    // SAFETY: the builder, deleted once.
    unsafe { (lib.AAudioStreamBuilder_delete)(builder) };
    let s = stream.0;

    // SAFETY: the open stream (here and below).
    st.sample_frames = unsafe { (lib.AAudioStream_getFramesPerDataCallback)(s) };
    if st.sample_frames == AAUDIO_UNSPECIFIED {
        // We'll get variable frames in the callback, make sure we have at least half a buffer available
        st.sample_frames = unsafe { (lib.AAudioStream_getBufferCapacityInFrames)(s) } / 2;
    }

    st.spec.freq = unsafe { (lib.AAudioStream_getSampleRate)(s) };
    st.spec.channels = unsafe { (lib.AAudioStream_getChannelCount)(s) };

    format = unsafe { (lib.AAudioStream_getFormat)(s) };
    st.spec.format = if format == AAUDIO_FORMAT_PCM_I16 {
        AudioFormat::S16
    } else if format == AAUDIO_FORMAT_PCM_I32 {
        AudioFormat::S32
    } else if format == AAUDIO_FORMAT_PCM_FLOAT {
        AudioFormat::F32
    } else {
        return Err(Error::new(format!(
            "Got unexpected audio format {format} from AAudioStream_getFormat"
        )));
    };

    updated_audio_device_format(st);

    // Allocate a triple buffered mixing buffer
    // Two buffers can be in the process of being filled while the third is being read
    let num_buffers = 3;
    let mixbuf_bytes = num_buffers * st.buffer_size;
    let ring = Box::new(Ring {
        mixbuf: UnsafeCell::new(vec![0u8; mixbuf_bytes].into_boxed_slice()),
        mixbuf_bytes,
        buffer_size: st.buffer_size.max(1),
        framesize: st.spec.frame_size(),
        silence_value: st.silence_value,
        // (processed_bytes = 0; callback_bytes = 0)
        callback_bytes: AtomicUsize::new(0),
        processed_bytes: AtomicUsize::new(0),
        semaphore: crate::thread::Semaphore::new(if recording {
            0
        } else {
            num_buffers as u32 - 1
        }),
    });
    hidden.ring.store(Box::into_raw(ring), Ordering::Release);

    logi!(
        "AAudio Actually opened {} hz {} {} channels samples {}, buffers {}",
        st.spec.freq,
        st.spec.format.name(),
        st.spec.channels,
        st.sample_frames,
        num_buffers
    );

    let res = unsafe { (lib.AAudioStream_requestStart)(s) };
    if res != AAUDIO_OK {
        logi!(
            "SDL Failed AAudioStream_requestStart {} recording:{}",
            res,
            recording as i32
        );
        return Err(Error::new(format!(
            "{BUILD_AAUDIO_STREAM} : {}",
            lib.result_text(res)
        )));
    }

    logi!("SDL AAudioStream_requestStart OK");

    Ok(())
}

/// Translation of `RequestAndroidPermissionBlockingCallback()` and its
/// wait (`AAUDIO_OpenDevice()`'s and the OpenSL ES recorder's): ask for
/// `RECORD_AUDIO`, and wait for the answer. !!! FIXME: make this
/// non-blocking!
pub(super) fn request_record_audio_permission_blocking() -> Result<()> {
    let permission_response = Arc::new(AtomicI32::new(0));
    let response = permission_response.clone();
    crate::core::android::request_android_permission(
        "android.permission.RECORD_AUDIO",
        Box::new(move |_permission: &str, granted: bool| {
            response.store(if granted { 1 } else { -1 }, Ordering::Release);
        }),
    )?;

    while permission_response.load(Ordering::Acquire) == 0 {
        crate::timer::delay(Duration::from_millis(10));
    }

    if permission_response.load(Ordering::Acquire) < 0 {
        logi!("This app doesn't have RECORD_AUDIO permission");
        return Err(Error::new("This app doesn't have RECORD_AUDIO permission"));
    }
    Ok(())
}

impl AudioDriverImpl for Aaudio {
    fn flags(&self) -> DriverFlags {
        let allow_multiple = crate::core::android::ALLOW_MULTIPLE_ANDROID_AUDIO_DEVICES;
        DriverFlags {
            has_recording_support: true,
            only_has_default_playback_device: !allow_multiple,
            only_has_default_recording_device: !allow_multiple,
            ..DriverFlags::default()
        }
    }

    /// `impl->DetectDevices = Android_StartAudioHotplug` with multiple
    /// devices; else the default devices.
    fn detect_devices(&self) -> (Option<Arc<PhysicalDevice>>, Option<Arc<PhysicalDevice>>) {
        if crate::core::android::ALLOW_MULTIPLE_ANDROID_AUDIO_DEVICES {
            crate::core::android::start_audio_hotplug()
        } else {
            crate::audio::device::default_detect_devices(&self.flags())
        }
    }

    /// Translation of `AAUDIO_OpenDevice()`.
    fn open_device(
        &self,
        device: &PhysicalDevice,
        state: &mut PhysState,
    ) -> Result<Arc<dyn DeviceBackend>> {
        if crate::core::android::ALLOW_MULTIPLE_ANDROID_AUDIO_DEVICES {
            crate::sdl_assert!(device.handle != 0); // AAUDIO_UNSPECIFIED is zero, so legit devices should all be non-zero.
        }

        logi!("AAUDIO_OpenDevice");

        if device.recording {
            request_record_audio_permission_blocking()?;
        }

        let hidden = Arc::new(AaudioDevice {
            lib: self.lib.clone(),
            recording: device.recording,
            stream: Mutex::new(StreamPtr(ptr::null_mut())),
            ring: AtomicPtr::new(ptr::null_mut()),
            error_callback_triggered: AtomicI32::new(0),
        });
        hidden_devices().push(hidden.clone());

        match build_aaudio_stream(&hidden, device, state) {
            Ok(()) => Ok(Arc::new(AaudioBackend { hidden })),
            Err(e) => {
                // (the core's ClosePhysicalAudioDevice() then calls
                // CloseDevice, which frees what was made)
                free_hidden(&hidden);
                Err(e)
            }
        }
    }

    /// Translation of `AAUDIO_Deinitialize()`.
    fn deinitialize(&self) {
        crate::core::android::stop_audio_hotplug();

        logi!("AAUDIO_Deinitialize");
        set_ctx(None); // (SDL_UnloadObject(ctx.handle), once the devices are gone; SDL_zero(ctx))
        logi!("End AAUDIO");
    }
}

/// Translation of `PauseOneDevice()`.
fn pause_one_device(hidden: &AaudioDevice) {
    let stream = hidden.stream();
    if !stream.0.is_null() {
        // SAFETY: an open stream.
        let res = if hidden.recording {
            // Pause() isn't implemented for recording, use Stop()
            unsafe { (hidden.lib.AAudioStream_requestStop)(stream.0) }
        } else {
            unsafe { (hidden.lib.AAudioStream_requestPause)(stream.0) }
        };

        if res != AAUDIO_OK {
            logi!("SDL Failed AAudioStream_requestPause {}", res);
            // (SDL_SetError("%s : %s", SDL_FUNCTION, ...): errors are values here)
        }
    }
}

/// Pause (block) all non already paused audio devices by taking their
/// mixer lock. Translation of `AAUDIO_PauseDevices()` (keep enumerating).
pub(crate) fn pause_devices() {
    if ctx().is_some() {
        // AAUDIO driver is used?
        let devices = hidden_devices().clone();
        for hidden in &devices {
            pause_one_device(hidden);
        }
    }
}

/// Translation of `ResumeOneDevice()`.
fn resume_one_device(hidden: &AaudioDevice) {
    let stream = hidden.stream();
    if !stream.0.is_null() {
        // SAFETY: an open stream.
        let res = unsafe { (hidden.lib.AAudioStream_requestStart)(stream.0) };
        if res != AAUDIO_OK {
            logi!("SDL Failed AAudioStream_requestStart {}", res);
            // (SDL_SetError("%s : %s", SDL_FUNCTION, ...): errors are values here)
        }
    }
}

/// Resume (unblock) all non already paused audio devices by releasing
/// their mixer lock. Translation of `AAUDIO_ResumeDevices()` (keep
/// enumerating).
pub(crate) fn resume_devices() {
    if ctx().is_some() {
        // AAUDIO driver is used?
        let devices = hidden_devices().clone();
        for hidden in &devices {
            resume_one_device(hidden);
        }
    }
}

/// Translation of `AAUDIO_Init()`.
fn aaudio_init() -> Option<Arc<dyn AudioDriverImpl>> {
    logi!("AAUDIO_Init");

    /* AAudio was introduced in Android 8.0, but has reference counting crash issues in that release,
     * so don't use it until 8.1.
     *
     * See https://github.com/google/oboe/issues/40 for more information.
     */
    if crate::core::android::get_android_sdk_version() < 27 {
        return None;
    }

    set_ctx(None);

    let Ok(handle) = SharedObject::load(LIB_AAUDIO_SO) else {
        logi!("SDL couldn't find {}", LIB_AAUDIO_SO);
        return None;
    };

    let lib = match AaudioLib::load_functions(handle) {
        Ok(lib) => Arc::new(lib),
        // (SDL_UnloadObject(ctx.handle): dropped; SDL_zero(ctx))
        Err(_) => return None,
    };
    set_ctx(Some(lib.clone()));

    logi!("SDL AAUDIO_Init OK");
    Some(Arc::new(Aaudio { lib }))
}

/// Translation of `AAUDIO_bootstrap`.
pub(crate) static AAUDIO_BOOTSTRAP: AudioBootStrap = AudioBootStrap {
    name: "AAudio",
    desc: "AAudio audio driver",
    init: aaudio_init,
    demand_only: false,
    is_preferred: false,
};
