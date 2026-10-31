// Rust translation of src/audio/openslES/SDL_openslES.c and
// src/audio/openslES/SDL_openslES.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The OpenSL ES driver (deprecated by Android, still supported): the
//! engine and its output mix, made by `Init`; one buffer-queue audio
//! player for playback and one audio recorder for recording (the globals
//! upstream keeps, so one of each at a time), with two buffers each, whose
//! callbacks (on OpenSL ES's thread) signal the device's thread. The
//! default devices only. While the app is in the background the event pump
//! pauses the player ([`pause_devices`], [`resume_devices`]; the recorder
//! isn't paused). Recording asks for the `RECORD_AUDIO` permission first,
//! blocking until it's answered.
//!
//! For more discussion of low latency audio on Android, see this:
//!   <https://googlesamples.github.io/android-audio-high-performance/guides/opensl_es.html>
//!
//! The core owns the buffer handed to `PlayDevice` here, so it is copied
//! into the player's next buffer there (upstream's `GetDeviceBuf` hands
//! out that buffer itself), which is then queued as upstream queues it.

mod sys;

use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use self::sys::*;
use crate::audio::device::{
    updated_audio_device_format, AudioBootStrap, AudioDriverImpl, DeviceBackend, DriverFlags,
    PhysState, PhysicalDevice,
};
use crate::audio::AudioFormat;
use crate::error::{Error, Result};
use crate::thread::Semaphore;

/// `NUM_BUFFERS` -- Don't lower this!
const NUM_BUFFERS: usize = 2;

// (LOGE(), LOGI() and LOGV(), compiled out with `#if 0`)
macro_rules! logx {
    ($($arg:tt)*) => {};
}

const SL_ANDROID_SPEAKER_STEREO: SLuint32 = SL_SPEAKER_FRONT_LEFT | SL_SPEAKER_FRONT_RIGHT;
const SL_ANDROID_SPEAKER_QUAD: SLuint32 =
    SL_ANDROID_SPEAKER_STEREO | SL_SPEAKER_BACK_LEFT | SL_SPEAKER_BACK_RIGHT;
const SL_ANDROID_SPEAKER_5DOT1: SLuint32 =
    SL_ANDROID_SPEAKER_QUAD | SL_SPEAKER_FRONT_CENTER | SL_SPEAKER_LOW_FREQUENCY;
const SL_ANDROID_SPEAKER_7DOT1: SLuint32 =
    SL_ANDROID_SPEAKER_5DOT1 | SL_SPEAKER_SIDE_LEFT | SL_SPEAKER_SIDE_RIGHT;

/// The globals: the engine, the output mix, the buffer queue player and
/// the recorder, with their interfaces.
struct Globals {
    // engine interfaces
    engine_object: SLObjectItf,
    engine_engine: SLEngineItf,

    // output mix interfaces
    output_mix_object: SLObjectItf,

    // buffer queue player interfaces
    bq_player_object: SLObjectItf,
    bq_player_play: SLPlayItf,
    bq_player_buffer_queue: SLAndroidSimpleBufferQueueItf,

    // recorder interfaces
    recorder_object: SLObjectItf,
    recorder_record: SLRecordItf,
    recorder_buffer_queue: SLAndroidSimpleBufferQueueItf,
}

// SAFETY: OpenSL ES objects and interfaces may be used from any thread
// (the engine is thread safe); the mutex orders SDL's calls.
unsafe impl Send for Globals {}

static GLOBALS: Mutex<Globals> = Mutex::new(Globals {
    engine_object: ptr::null(),
    engine_engine: ptr::null(),
    output_mix_object: ptr::null(),
    bq_player_object: ptr::null(),
    bq_player_play: ptr::null(),
    bq_player_buffer_queue: ptr::null(),
    recorder_object: ptr::null(),
    recorder_record: ptr::null(),
    recorder_buffer_queue: ptr::null(),
});

fn globals() -> MutexGuard<'static, Globals> {
    GLOBALS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Call an interface's function: `(*itf)->f(itf, args...)`.
macro_rules! sl {
    ($itf:expr, $f:ident($($arg:expr),*)) => {{
        let itf = $itf;
        // SAFETY: a live interface of an OpenSL ES object (the callers'
        // checks), whose table has the function.
        unsafe { ((**itf).$f)(itf $(, $arg)*) }
    }};
}

/// Translation of `OPENSLES_DestroyEngine()`.
fn opensles_destroy_engine(g: &mut Globals) {
    logx!("OPENSLES_DestroyEngine()");

    // destroy output mix object, and invalidate all associated interfaces
    if !g.output_mix_object.is_null() {
        sl!(g.output_mix_object, Destroy());
        g.output_mix_object = ptr::null();
    }

    // destroy engine object, and invalidate all associated interfaces
    if !g.engine_object.is_null() {
        sl!(g.engine_object, Destroy());
        g.engine_object = ptr::null();
        g.engine_engine = ptr::null();
    }
}

/// Translation of `OPENSLES_CreateEngine()`.
fn opensles_create_engine(g: &mut Globals) -> bool {
    // SAFETY: libOpenSLES's interface IDs.
    let ids: [SLInterfaceID; 1] = [unsafe { SL_IID_VOLUME }];
    let req: [SLboolean; 1] = [SL_BOOLEAN_FALSE];

    logx!("openSLES_CreateEngine()");

    let ok = (|| {
        // create engine
        // SAFETY: an out-pointer for the engine, and no options.
        let result = unsafe {
            slCreateEngine(
                &mut g.engine_object,
                0,
                ptr::null(),
                0,
                ptr::null(),
                ptr::null(),
            )
        };
        if SL_RESULT_SUCCESS != result {
            logx!("slCreateEngine failed: {}", result);
            return false;
        }
        logx!("slCreateEngine OK");

        // realize the engine
        let result = sl!(g.engine_object, Realize(SL_BOOLEAN_FALSE));
        if SL_RESULT_SUCCESS != result {
            logx!("RealizeEngine failed: {}", result);
            return false;
        }
        logx!("RealizeEngine OK");

        // get the engine interface, which is needed in order to create other objects
        // SAFETY: libOpenSLES's interface ID.
        let iid = unsafe { SL_IID_ENGINE };
        let out = (&mut g.engine_engine as *mut SLEngineItf).cast::<c_void>();
        let result = sl!(g.engine_object, GetInterface(iid, out));
        if SL_RESULT_SUCCESS != result {
            logx!("EngineGetInterface failed: {}", result);
            return false;
        }
        logx!("EngineGetInterface OK");

        // create output mix
        let result = sl!(
            g.engine_engine,
            CreateOutputMix(&mut g.output_mix_object, 1, ids.as_ptr(), req.as_ptr())
        );
        if SL_RESULT_SUCCESS != result {
            logx!("CreateOutputMix failed: {}", result);
            return false;
        }
        logx!("CreateOutputMix OK");

        // realize the output mix
        let result = sl!(g.output_mix_object, Realize(SL_BOOLEAN_FALSE));
        if SL_RESULT_SUCCESS != result {
            logx!("RealizeOutputMix failed: {}", result);
            return false;
        }
        true
    })();

    if !ok {
        opensles_destroy_engine(g);
    }
    ok
}

/// Translation of `struct SDL_PrivateAudioData`.
struct OpenslesDevice {
    /// `mixbuff`, `NUM_BUFFERS` buffers (`pmixbuff[i]` is `i *
    /// buffer_size` into it).
    mixbuff: UnsafeCell<Box<[u8]>>,
    buffer_size: usize,
    next_buffer: AtomicUsize,
    playsem: Semaphore,
}

// SAFETY: the buffer queue reads (or fills) the queued buffers, and the
// device's thread the one the semaphore gives it, as upstream's.
unsafe impl Sync for OpenslesDevice {}
// SAFETY: as above (the cell's box moves with the device).
unsafe impl Send for OpenslesDevice {}

impl OpenslesDevice {
    fn new(buffer_size: usize, initial: u32) -> Arc<OpenslesDevice> {
        Arc::new(OpenslesDevice {
            mixbuff: UnsafeCell::new(vec![0u8; NUM_BUFFERS * buffer_size].into_boxed_slice()),
            buffer_size,
            next_buffer: AtomicUsize::new(0),
            playsem: Semaphore::new(initial),
        })
    }

    /// `pmixbuff[i]`.
    fn pmixbuff(&self, i: usize) -> *mut u8 {
        // SAFETY: the box is never replaced; `i < NUM_BUFFERS`.
        unsafe { (*self.mixbuff.get()).as_mut_ptr().add(i * self.buffer_size) }
    }

    fn context(self: &Arc<Self>) -> *mut c_void {
        Arc::as_ptr(self) as *mut c_void
    }

    /// `next_buffer` moved on.
    fn advance(&self) -> usize {
        let next_buffer = self.next_buffer.load(Ordering::Relaxed);
        let mut n = next_buffer + 1;
        if n >= NUM_BUFFERS {
            n = 0;
        }
        self.next_buffer.store(n, Ordering::Relaxed);
        next_buffer
    }
}

/// this callback handler is called every time a buffer finishes recording.
/// Translation of `bqRecorderCallback()`.
unsafe extern "C" fn bq_recorder_callback(
    _bq: SLAndroidSimpleBufferQueueItf,
    context: *mut c_void,
) {
    // SAFETY: the device's `hidden`, which outlives the recorder.
    let audiodata = unsafe { &*(context as *const OpenslesDevice) };

    logx!("SLES: Recording Callback");
    audiodata.playsem.signal();
}

/// Translation of `OPENSLES_DestroyPCMRecorder()` (the semaphore and the
/// buffers go with the device's data).
fn opensles_destroy_pcm_recorder(g: &mut Globals) {
    // stop recording
    if !g.recorder_record.is_null() {
        let result = sl!(g.recorder_record, SetRecordState(SL_RECORDSTATE_STOPPED));
        if SL_RESULT_SUCCESS != result {
            logx!("SetRecordState stopped: {}", result);
        }
    }

    // destroy audio recorder object, and invalidate all associated interfaces
    if !g.recorder_object.is_null() {
        sl!(g.recorder_object, Destroy());
        g.recorder_object = ptr::null();
        g.recorder_record = ptr::null();
        g.recorder_buffer_queue = ptr::null();
    }
}

/// Translation of `OPENSLES_CreatePCMRecorder()`: the device's data in
/// `hidden` once it's made (for the callback; freed after the recorder
/// is destroyed).
fn opensles_create_pcm_recorder(
    st: &mut PhysState,
    hidden: &mut Option<Arc<OpenslesDevice>>,
) -> Result<()> {
    // !!! FIXME: make this non-blocking!
    super::aaudio::request_record_audio_permission_blocking()?;

    // Just go with signed 16-bit audio as it's the most compatible
    st.spec.format = AudioFormat::S16;
    st.spec.channels = 1;
    //device->spec.freq = SL_SAMPLINGRATE_16 / 1000;*/
    // Update the fragment size as size in bytes
    updated_audio_device_format(st);

    logx!(
        "Try to open {} hz {} bit {} channels {} samples {}",
        st.spec.freq,
        st.spec.format.bitsize(),
        st.spec.channels,
        if st.spec.format.0 & 0x1000 != 0 {
            "BE"
        } else {
            "LE"
        },
        st.sample_frames
    );

    let mut g = globals();

    // configure audio source
    let mut loc_dev = SLDataLocator_IODevice {
        locatorType: SL_DATALOCATOR_IODEVICE,
        deviceType: SL_IODEVICE_AUDIOINPUT,
        deviceID: SL_DEFAULTDEVICEID_AUDIOINPUT,
        device: ptr::null(),
    };
    let mut audio_src = SLDataSource {
        pLocator: (&mut loc_dev as *mut SLDataLocator_IODevice).cast(),
        pFormat: ptr::null_mut(),
    };

    // configure audio sink
    let mut loc_bufq = SLDataLocator_AndroidSimpleBufferQueue {
        locatorType: SL_DATALOCATOR_ANDROIDSIMPLEBUFFERQUEUE,
        numBuffers: NUM_BUFFERS as SLuint32,
    };

    let bits = st.spec.format.bitsize();
    let mut format_pcm = SLDataFormat_PCM {
        formatType: SL_DATAFORMAT_PCM,
        numChannels: st.spec.channels as SLuint32,
        samplesPerSec: (st.spec.freq as SLuint32).wrapping_mul(1000), // / kilo Hz to milli Hz
        bitsPerSample: bits,
        containerSize: bits,
        endianness: SL_BYTEORDER_LITTLEENDIAN,
        channelMask: SL_SPEAKER_FRONT_CENTER,
    };

    let mut audio_snk = SLDataSink {
        pLocator: (&mut loc_bufq as *mut SLDataLocator_AndroidSimpleBufferQueue).cast(),
        pFormat: (&mut format_pcm as *mut SLDataFormat_PCM).cast(),
    };

    // SAFETY: libOpenSLES's interface IDs.
    let ids: [SLInterfaceID; 1] = [unsafe { SL_IID_ANDROIDSIMPLEBUFFERQUEUE }];
    let req: [SLboolean; 1] = [SL_BOOLEAN_TRUE];

    let failed = || Error::new("Open device failed!");

    // create audio recorder
    // (requires the RECORD_AUDIO permission)
    let result = sl!(
        g.engine_engine,
        CreateAudioRecorder(
            &mut g.recorder_object,
            &mut audio_src,
            &mut audio_snk,
            1,
            ids.as_ptr(),
            req.as_ptr()
        )
    );
    if SL_RESULT_SUCCESS != result {
        logx!("CreateAudioRecorder failed: {}", result);
        return Err(failed());
    }

    // realize the recorder
    let result = sl!(g.recorder_object, Realize(SL_BOOLEAN_FALSE));
    if SL_RESULT_SUCCESS != result {
        logx!("RealizeAudioPlayer failed: {}", result);
        return Err(failed());
    }

    // get the record interface
    // SAFETY: libOpenSLES's interface ID.
    let iid = unsafe { SL_IID_RECORD };
    let out = (&mut g.recorder_record as *mut SLRecordItf).cast::<c_void>();
    let result = sl!(g.recorder_object, GetInterface(iid, out));
    if SL_RESULT_SUCCESS != result {
        logx!("SL_IID_RECORD interface get failed: {}", result);
        return Err(failed());
    }

    // get the buffer queue interface
    let out = (&mut g.recorder_buffer_queue as *mut SLAndroidSimpleBufferQueueItf).cast::<c_void>();
    let result = sl!(g.recorder_object, GetInterface(ids[0], out));
    if SL_RESULT_SUCCESS != result {
        logx!("SL_IID_BUFFERQUEUE interface get failed: {}", result);
        return Err(failed());
    }

    // Create the audio buffer semaphore, and the sound buffers
    let audiodata = hidden
        .insert(OpenslesDevice::new(st.buffer_size, 0))
        .clone();

    // register callback on the buffer queue
    // context is '(SDL_PrivateAudioData *)device->hidden'
    let result = sl!(
        g.recorder_buffer_queue,
        RegisterCallback(bq_recorder_callback, audiodata.context())
    );
    if SL_RESULT_SUCCESS != result {
        logx!("RegisterCallback failed: {}", result);
        return Err(failed());
    }

    // in case already recording, stop recording and clear buffer queue
    let result = sl!(g.recorder_record, SetRecordState(SL_RECORDSTATE_STOPPED));
    if SL_RESULT_SUCCESS != result {
        logx!("Record set state failed: {}", result);
        return Err(failed());
    }

    // enqueue empty buffers to be filled by the recorder
    for i in 0..NUM_BUFFERS {
        let result = sl!(
            g.recorder_buffer_queue,
            Enqueue(
                audiodata.pmixbuff(i) as *const c_void,
                st.buffer_size as SLuint32
            )
        );
        if SL_RESULT_SUCCESS != result {
            logx!("Record enqueue buffers failed: {}", result);
            return Err(failed());
        }
    }

    // start recording
    let result = sl!(g.recorder_record, SetRecordState(SL_RECORDSTATE_RECORDING));
    if SL_RESULT_SUCCESS != result {
        logx!("Record set state failed: {}", result);
        return Err(failed());
    }

    Ok(())
}

/// this callback handler is called every time a buffer finishes playing.
/// Translation of `bqPlayerCallback()`.
unsafe extern "C" fn bq_player_callback(_bq: SLAndroidSimpleBufferQueueItf, context: *mut c_void) {
    // SAFETY: the device's `hidden`, which outlives the player.
    let audiodata = unsafe { &*(context as *const OpenslesDevice) };

    logx!("SLES: Playback Callback");
    audiodata.playsem.signal();
}

/// Translation of `OPENSLES_DestroyPCMPlayer()` (the semaphore and the
/// buffers go with the device's data).
fn opensles_destroy_pcm_player(g: &mut Globals) {
    // set the player's state to 'stopped'
    if !g.bq_player_play.is_null() {
        let result = sl!(g.bq_player_play, SetPlayState(SL_PLAYSTATE_STOPPED));
        if SL_RESULT_SUCCESS != result {
            logx!("SetPlayState stopped failed: {}", result);
        }
    }

    // destroy buffer queue audio player object, and invalidate all associated interfaces
    if !g.bq_player_object.is_null() {
        sl!(g.bq_player_object, Destroy());

        g.bq_player_object = ptr::null();
        g.bq_player_play = ptr::null();
        g.bq_player_buffer_queue = ptr::null();
    }
}

/// Translation of `OPENSLES_CreatePCMPlayer()`: `false` when it failed
/// (without an error of its own); the device's data in `hidden` once it's
/// made (as the recorder's).
fn opensles_create_pcm_player(
    st: &mut PhysState,
    hidden: &mut Option<Arc<OpenslesDevice>>,
) -> bool {
    /* according to https://developer.android.com/ndk/guides/audio/opensl/opensl-for-android,
    Android's OpenSL ES only supports Uint8 and _littleendian_ Sint16.
    (and float32, with an extension we use, below.) */
    if crate::core::android::get_android_sdk_version() >= 21 {
        let test_format =
            st.spec.format.closest_formats().iter().copied().find(|&f| {
                f == AudioFormat::U8 || f == AudioFormat::S16LE || f == AudioFormat::F32
            });

        // (none: Didn't find a compatible format, using signed 16-bit LE audio)
        st.spec.format = test_format.unwrap_or(AudioFormat::S16LE);
    } else {
        // Just go with signed 16-bit audio as it's the most compatible
        st.spec.format = AudioFormat::S16LE;
    }

    // Update the fragment size as size in bytes
    updated_audio_device_format(st);

    logx!(
        "Try to open {} hz {} {} bit {} channels {} samples {}",
        st.spec.freq,
        if st.spec.format.is_float() {
            "float"
        } else {
            "pcm"
        },
        st.spec.format.bitsize(),
        st.spec.channels,
        if st.spec.format.0 & 0x1000 != 0 {
            "BE"
        } else {
            "LE"
        },
        st.sample_frames
    );

    let mut g = globals();

    // configure audio source
    let mut loc_bufq = SLDataLocator_AndroidSimpleBufferQueue {
        locatorType: SL_DATALOCATOR_ANDROIDSIMPLEBUFFERQUEUE,
        numBuffers: NUM_BUFFERS as SLuint32,
    };

    let bits = st.spec.format.bitsize();
    let mut format_pcm = SLDataFormat_PCM {
        formatType: SL_DATAFORMAT_PCM,
        numChannels: st.spec.channels as SLuint32,
        samplesPerSec: (st.spec.freq as SLuint32).wrapping_mul(1000), // / kilo Hz to milli Hz
        bitsPerSample: bits,
        containerSize: bits,
        ..SLDataFormat_PCM::default()
    };

    format_pcm.endianness = if st.spec.format.is_big_endian() {
        SL_BYTEORDER_BIGENDIAN
    } else {
        SL_BYTEORDER_LITTLEENDIAN
    };

    format_pcm.channelMask = match st.spec.channels {
        1 => SL_SPEAKER_FRONT_LEFT,
        2 => SL_ANDROID_SPEAKER_STEREO,
        3 => SL_ANDROID_SPEAKER_STEREO | SL_SPEAKER_FRONT_CENTER,
        4 => SL_ANDROID_SPEAKER_QUAD,
        5 => SL_ANDROID_SPEAKER_QUAD | SL_SPEAKER_FRONT_CENTER,
        6 => SL_ANDROID_SPEAKER_5DOT1,
        7 => SL_ANDROID_SPEAKER_5DOT1 | SL_SPEAKER_BACK_CENTER,
        8 => SL_ANDROID_SPEAKER_7DOT1,
        _ => {
            // Unknown number of channels, fall back to stereo
            // Note (upstream): after the format's channel count (and the
            // device's buffer size) were taken; SDL has no more than 8
            // channels, so this isn't reached.
            st.spec.channels = 2;
            SL_SPEAKER_FRONT_LEFT | SL_SPEAKER_FRONT_RIGHT
        }
    };

    let mut format_pcm_ex;
    let mut audio_src = SLDataSource {
        pLocator: (&mut loc_bufq as *mut SLDataLocator_AndroidSimpleBufferQueue).cast(),
        pFormat: (&mut format_pcm as *mut SLDataFormat_PCM).cast(),
    };

    if st.spec.format.is_float() {
        // Copy all setup into PCM EX structure
        format_pcm_ex = SLAndroidDataFormat_PCM_EX {
            formatType: SL_ANDROID_DATAFORMAT_PCM_EX,
            endianness: format_pcm.endianness,
            channelMask: format_pcm.channelMask,
            numChannels: format_pcm.numChannels,
            sampleRate: format_pcm.samplesPerSec,
            bitsPerSample: format_pcm.bitsPerSample,
            containerSize: format_pcm.containerSize,
            representation: SL_ANDROID_PCM_REPRESENTATION_FLOAT,
        };
        audio_src.pFormat = (&mut format_pcm_ex as *mut SLAndroidDataFormat_PCM_EX).cast();
    }

    // configure audio sink
    let mut loc_outmix = SLDataLocator_OutputMix {
        locatorType: SL_DATALOCATOR_OUTPUTMIX,
        outputMix: g.output_mix_object,
    };
    let mut audio_snk = SLDataSink {
        pLocator: (&mut loc_outmix as *mut SLDataLocator_OutputMix).cast(),
        pFormat: ptr::null_mut(),
    };

    // create audio player
    // SAFETY: libOpenSLES's interface IDs.
    let ids: [SLInterfaceID; 2] = unsafe { [SL_IID_ANDROIDSIMPLEBUFFERQUEUE, SL_IID_VOLUME] };
    let req: [SLboolean; 2] = [SL_BOOLEAN_TRUE, SL_BOOLEAN_FALSE];
    let result = sl!(
        g.engine_engine,
        CreateAudioPlayer(
            &mut g.bq_player_object,
            &mut audio_src,
            &mut audio_snk,
            2,
            ids.as_ptr(),
            req.as_ptr()
        )
    );
    if SL_RESULT_SUCCESS != result {
        logx!("CreateAudioPlayer failed: {}", result);
        return false;
    }

    // realize the player
    let result = sl!(g.bq_player_object, Realize(SL_BOOLEAN_FALSE));
    if SL_RESULT_SUCCESS != result {
        logx!("RealizeAudioPlayer failed: {}", result);
        return false;
    }

    // get the play interface
    // SAFETY: libOpenSLES's interface ID.
    let iid = unsafe { SL_IID_PLAY };
    let out = (&mut g.bq_player_play as *mut SLPlayItf).cast::<c_void>();
    let result = sl!(g.bq_player_object, GetInterface(iid, out));
    if SL_RESULT_SUCCESS != result {
        logx!("SL_IID_PLAY interface get failed: {}", result);
        return false;
    }

    // get the buffer queue interface
    let out =
        (&mut g.bq_player_buffer_queue as *mut SLAndroidSimpleBufferQueueItf).cast::<c_void>();
    let result = sl!(g.bq_player_object, GetInterface(ids[0], out));
    if SL_RESULT_SUCCESS != result {
        logx!("SL_IID_BUFFERQUEUE interface get failed: {}", result);
        return false;
    }

    // Create the audio buffer semaphore, and the sound buffers
    let audiodata = hidden
        .insert(OpenslesDevice::new(st.buffer_size, NUM_BUFFERS as u32 - 1))
        .clone();

    // register callback on the buffer queue
    // context is '(SDL_PrivateAudioData *)device->hidden'
    let result = sl!(
        g.bq_player_buffer_queue,
        RegisterCallback(bq_player_callback, audiodata.context())
    );
    if SL_RESULT_SUCCESS != result {
        logx!("RegisterCallback failed: {}", result);
        return false;
    }

    // set the player's state to playing
    let result = sl!(g.bq_player_play, SetPlayState(SL_PLAYSTATE_PLAYING));
    if SL_RESULT_SUCCESS != result {
        logx!("Play set state failed: {}", result);
        return false;
    }

    true
}

/// The OpenSL ES driver (`OPENSLES_Init()`).
struct Opensles;

/// An opened device's interface.
struct OpenslesBackend {
    audiodata: Arc<OpenslesDevice>,
}

impl DeviceBackend for OpenslesBackend {
    /// `impl->ThreadInit = Android_AudioThreadInit`.
    fn thread_init(&self, device: &PhysicalDevice) {
        crate::core::android::audio_thread_init(device);
    }

    /// Translation of `OPENSLES_WaitDevice()`.
    fn wait_device(&self, device: &PhysicalDevice) -> bool {
        logx!("OPENSLES_WaitDevice()");

        while !device.shutting_down() {
            // this semaphore won't fire when the app is in the background (OPENSLES_PauseDevices was called).
            if self
                .audiodata
                .playsem
                .wait_timeout(Some(Duration::from_millis(100)))
            {
                return true; // semaphore was signaled, let's go!
            }
            // Still waiting on the semaphore (or the system), check other things then wait again.
        }
        true
    }

    /// Translation of `OPENSLES_GetDeviceBuf()` (the core's buffer, copied
    /// into `pmixbuff[next_buffer]` by `play_device`).
    fn get_device_buf(&self, _device: &PhysicalDevice, buffer_size: usize) -> Option<usize> {
        logx!("OPENSLES_GetDeviceBuf()");
        Some(buffer_size)
    }

    /// Translation of `OPENSLES_PlayDevice()`.
    fn play_device(&self, _device: &PhysicalDevice, buffer: &[u8]) -> bool {
        let audiodata = &self.audiodata;

        logx!("======OPENSLES_PlayDevice()======");

        // (the buffer GetDeviceBuf() gave)
        let next = audiodata.next_buffer.load(Ordering::Relaxed);
        let len = buffer.len().min(audiodata.buffer_size);
        let pbuf = audiodata.pmixbuff(next);
        // SAFETY: the buffer the semaphore gave this thread (not queued).
        unsafe { ptr::copy_nonoverlapping(buffer.as_ptr(), pbuf, len) };

        // Queue it up
        let g = globals();
        let result = if g.bq_player_buffer_queue.is_null() {
            !SL_RESULT_SUCCESS
        } else {
            sl!(
                g.bq_player_buffer_queue,
                Enqueue(pbuf as *const c_void, buffer.len() as SLuint32)
            )
        };
        drop(g);

        audiodata.advance();

        // If Enqueue fails, callback won't be called.
        // Post the semaphore, not to run out of buffer
        if SL_RESULT_SUCCESS != result {
            audiodata.playsem.signal();
        }

        true
    }

    //           n   playn sem
    // getbuf     0   -     1
    // fill buff  0   -     1
    // play       0 - 0     1
    // wait       1   0     0
    // getbuf     1   0     0
    // fill buff  1   0     0
    // play       0   0     0
    // wait
    //
    // okay..

    /// `impl->WaitRecordingDevice = OPENSLES_WaitDevice`.
    fn wait_recording_device(&self, device: &PhysicalDevice) -> bool {
        self.wait_device(device)
    }

    /// Translation of `OPENSLES_RecordDevice()`.
    fn record_device(&self, _device: &PhysicalDevice, buffer: &mut [u8]) -> Result<usize> {
        let audiodata = &self.audiodata;

        // Copy it to the output buffer
        crate::sdl_assert!(buffer.len() == audiodata.buffer_size);
        let next = audiodata.next_buffer.load(Ordering::Relaxed);
        let pbuf = audiodata.pmixbuff(next);
        let len = buffer.len().min(audiodata.buffer_size);
        // SAFETY: the filled buffer the semaphore gave this thread.
        unsafe { ptr::copy_nonoverlapping(pbuf, buffer.as_mut_ptr(), len) };

        // Re-enqueue the buffer
        let g = globals();
        let result = if g.recorder_buffer_queue.is_null() {
            !SL_RESULT_SUCCESS
        } else {
            sl!(
                g.recorder_buffer_queue,
                Enqueue(pbuf as *const c_void, audiodata.buffer_size as SLuint32)
            )
        };
        drop(g);
        if SL_RESULT_SUCCESS != result {
            logx!("Record enqueue buffers failed: {}", result);
            return Err(Error::new("Record enqueue buffers failed"));
        }

        audiodata.advance();

        Ok(audiodata.buffer_size)
    }

    /// Translation of `OPENSLES_CloseDevice()`.
    fn close_device(&self, device: &PhysicalDevice) {
        // (device->hidden is this device's data, freed with it)
        let mut g = globals();
        if device.recording {
            logx!("OPENSLES_CloseDevice() for recording");
            opensles_destroy_pcm_recorder(&mut g);
        } else {
            logx!("OPENSLES_CloseDevice() for playing");
            opensles_destroy_pcm_player(&mut g);
        }
    }
}

impl AudioDriverImpl for Opensles {
    fn flags(&self) -> DriverFlags {
        // and the capabilities
        DriverFlags {
            has_recording_support: true,
            only_has_default_playback_device: true,
            only_has_default_recording_device: true,
            ..DriverFlags::default()
        }
    }

    /// Translation of `OPENSLES_OpenDevice()`: on a failure, what was made
    /// is destroyed here (upstream's `CloseDevice`, which the core calls
    /// with the device's `hidden`).
    fn open_device(
        &self,
        device: &PhysicalDevice,
        state: &mut PhysState,
    ) -> Result<Arc<dyn DeviceBackend>> {
        let mut hidden = None;
        if device.recording {
            logx!("OPENSLES_OpenDevice() for recording");
            if let Err(e) = opensles_create_pcm_recorder(state, &mut hidden) {
                opensles_destroy_pcm_recorder(&mut globals());
                return Err(e);
            }
        } else {
            logx!("OPENSLES_OpenDevice() for playback");
            let mut ret = opensles_create_pcm_player(state, &mut hidden);
            if !ret {
                // Another attempt to open the device with a lower frequency
                if state.spec.freq > 48000 {
                    opensles_destroy_pcm_player(&mut globals());
                    state.spec.freq = 48000;
                    ret = opensles_create_pcm_player(state, &mut hidden);
                }
            }

            if !ret {
                opensles_destroy_pcm_player(&mut globals());
                return Err(Error::new("Open device failed!"));
            }
        }

        let audiodata = hidden.expect("the device's data, once it opened");
        Ok(Arc::new(OpenslesBackend { audiodata }))
    }

    /// `impl->Deinitialize = OPENSLES_DestroyEngine`.
    fn deinitialize(&self) {
        opensles_destroy_engine(&mut globals());
    }
}

/// Translation of `OPENSLES_Init()`.
fn opensles_init() -> Option<Arc<dyn AudioDriverImpl>> {
    logx!("OPENSLES_Init() called");

    if !opensles_create_engine(&mut globals()) {
        return None;
    }

    logx!("OPENSLES_Init() - set pointers");

    // Set the function pointers
    // impl->DetectDevices = OPENSLES_DetectDevices;
    // (the rest: Opensles's and OpenslesBackend's methods)

    logx!("OPENSLES_Init() - success");

    // this audio target is available.
    Some(Arc::new(Opensles))
}

/// Translation of `OPENSLES_bootstrap`.
pub(crate) static OPENSLES_BOOTSTRAP: AudioBootStrap = AudioBootStrap {
    name: "openslES",
    desc: "OpenSL ES audio driver",
    init: opensles_init,
    demand_only: false,
    is_preferred: false,
};

/// Translation of `OPENSLES_ResumeDevices()`.
pub(crate) fn resume_devices() {
    let g = globals();
    if !g.bq_player_play.is_null() {
        // set the player's state to 'playing'
        let result = sl!(g.bq_player_play, SetPlayState(SL_PLAYSTATE_PLAYING));
        if SL_RESULT_SUCCESS != result {
            logx!("OPENSLES_ResumeDevices failed: {}", result);
        }
    }
}

/// Translation of `OPENSLES_PauseDevices()`.
pub(crate) fn pause_devices() {
    let g = globals();
    if !g.bq_player_play.is_null() {
        // set the player's state to 'paused'
        let result = sl!(g.bq_player_play, SetPlayState(SL_PLAYSTATE_PAUSED));
        if SL_RESULT_SUCCESS != result {
            logx!("OPENSLES_PauseDevices failed: {}", result);
        }
    }
}
