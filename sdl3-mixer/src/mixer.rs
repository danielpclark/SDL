// Rust translation of src/SDL_mixer.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The mixer: mixers, tracks, audio, groups and audio decoders.
//!
//! Upstream's objects are pointers in doubly-linked lists, with the
//! mixer's and tracks' state guarded by the locks of their audio streams.
//! Here the lists are `Vec`s (kept in upstream's order: the newest item
//! first) of `Arc`s, the state lives in a `RefCell` behind a reentrant
//! mutex that is always taken after the audio stream's lock (`LockMixer`,
//! `LockTrack`), and no `RefCell` borrow is held across a call to a
//! callback or into a stream whose callbacks might call back here.
//!
//! The public handles ([`Mixer`], [`Track`], [`Group`], [`AudioDecoder`])
//! destroy their object when the owning handle is dropped; callbacks and
//! getters hand out non-owning handles to the same object. [`Audio`] is
//! reference counted (as `MIX_Audio` is): the last clone frees it. An
//! object destroyed behind a handle's back (by [`quit`], or a track whose
//! mixer was destroyed) fails every call with an invalid parameter error,
//! where upstream would use freed memory.

// !!! FIXME: figure out `int` vs Sint64/Uint64 metrics in all of this.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, Weak};

use sdl3::audio::{
    AudioDeviceID, AudioFormat, AudioSpec, AudioStream, AudioStreamLock,
    PROP_AUDIOSTREAM_AUTO_CLEANUP_BOOLEAN,
};
use sdl3::error::{Error, Result};
use sdl3::events::{Event, EventType, EventWatch};
use sdl3::init::InitFlags;
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;
use sdl3::stdlib::string::strcasecmp;
use sdl3::thread::{ReentrantMutex, ReentrantMutexGuard};

use crate::internal::{
    borrow_io, try_alloc, AudioData, Decoder, IoClamp, Precache, PrecacheIo, TrackData,
    PROP_AUDIO_LOAD_ONDEMAND_BOOLEAN, PROP_AUDIO_LOAD_PATH_STRING, PROP_DECODER_CHANNELS_NUMBER,
    PROP_DECODER_FORMAT_NUMBER, PROP_DECODER_FREQ_NUMBER, PROP_DECODER_SINEWAVE_AMPLITUDE_FLOAT,
    PROP_DECODER_SINEWAVE_HZ_NUMBER, PROP_DECODER_SINEWAVE_MS_NUMBER,
};
use crate::metadata_tags::read_metadata_tags;
use crate::spatialization::{spatialize, Vbap2d};
use crate::{
    Point3D, StereoGains, DURATION_INFINITE, DURATION_UNKNOWN, PROP_AUDIO_DECODER_STRING,
    PROP_AUDIO_LOAD_PREDECODE_BOOLEAN, PROP_AUDIO_LOAD_SKIP_METADATA_TAGS_BOOLEAN,
    PROP_METADATA_DURATION_FRAMES_NUMBER, PROP_METADATA_DURATION_INFINITE_BOOLEAN,
    PROP_MIXER_DEVICE_NUMBER, PROP_PLAY_APPEND_SILENCE_FRAMES_NUMBER,
    PROP_PLAY_APPEND_SILENCE_MILLISECONDS_NUMBER, PROP_PLAY_FADE_IN_FRAMES_NUMBER,
    PROP_PLAY_FADE_IN_MILLISECONDS_NUMBER, PROP_PLAY_FADE_IN_START_GAIN_FLOAT,
    PROP_PLAY_HALT_WHEN_EXHAUSTED_BOOLEAN, PROP_PLAY_LOOPS_NUMBER,
    PROP_PLAY_LOOP_START_FRAME_NUMBER, PROP_PLAY_LOOP_START_MILLISECOND_NUMBER,
    PROP_PLAY_MAX_FRAME_NUMBER, PROP_PLAY_MAX_MILLISECONDS_NUMBER, PROP_PLAY_START_FRAME_NUMBER,
    PROP_PLAY_START_MILLISECOND_NUMBER, PROP_PLAY_START_ORDER_NUMBER,
};

// !!! FIXME: should RAW go first (only needs to check if it was explicitly
// !!! FIXME: requested), and SINEWAVE last (must be requested, likely rare).
//
// (the decoders upstream builds against an external library, and the
// bundled Timidity, are left out, as in an upstream build without them.)
static DECODERS: [&Decoder; 9] = [
    &crate::decoder_wav::DECODER,
    &crate::decoder_stb_vorbis::DECODER,
    &crate::decoder_drflac::DECODER,
    &crate::decoder_voc::DECODER,
    &crate::decoder_aiff::DECODER,
    &crate::decoder_au::DECODER,
    &crate::decoder_drmp3::DECODER,
    // these are always available.
    &crate::decoder_sinewave::DECODER,
    &crate::decoder_raw::DECODER,
];

/// The library-wide state upstream keeps in file statics.
struct GlobalState {
    available_decoders: Vec<&'static Decoder>,
    mixer_initialized: i32,
    all_mixers: Vec<Arc<MixerInner>>,
    all_audiodecoders: Vec<Arc<AudioDecoderInner>>,
}

static GLOBAL: Mutex<GlobalState> = Mutex::new(GlobalState {
    available_decoders: Vec::new(),
    mixer_initialized: 0,
    all_mixers: Vec::new(),
    all_audiodecoders: Vec::new(),
});

/// Translation of `LockGlobal()`.
fn lock_global() -> MutexGuard<'static, GlobalState> {
    GLOBAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A track's input to an audio stream as float samples.
fn bytes_to_f32(src: &[u8]) -> Vec<f32> {
    src.chunks_exact(4)
        .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn f32_to_bytes(src: &[f32], dst: &mut [u8]) {
    for (d, s) in dst.chunks_exact_mut(4).zip(src) {
        d.copy_from_slice(&s.to_ne_bytes());
    }
}

/// `SDL_max()` for floats (NaN passes through, unlike `f32::max`).
fn sdl_max_f(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

/// A callback that runs when a track stops. Translation of
/// `MIX_TrackStoppedCallback`.
///
/// It runs with the track locked, possibly on the audio device's thread,
/// and may restart the track (or do anything else with it).
pub type TrackStoppedCallback = Arc<dyn Fn(&Track) + Send + Sync>;

/// A callback that sees (and may change) a track's audio as it is mixed.
/// Translation of `MIX_TrackMixCallback`.
///
/// It receives the track, the format of the data, and the interleaved
/// float samples.
pub type TrackMixCallback = Arc<dyn Fn(&Track, &AudioSpec, &mut [f32]) + Send + Sync>;

/// A callback that sees (and may change) a group's mixed audio.
/// Translation of `MIX_GroupMixCallback`.
pub type GroupMixCallback = Arc<dyn Fn(&Group, &AudioSpec, &mut [f32]) + Send + Sync>;

/// A callback that sees (and may change) the mixer's final mix.
/// Translation of `MIX_PostMixCallback`.
pub type PostMixCallback = Arc<dyn Fn(&Mixer, &AudioSpec, &mut [f32]) + Send + Sync>;

/// Translation of `MIX_TrackState`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TrackStateKind {
    Stopped,
    Paused,
    Playing,
}

/// Translation of `MIX_SpatializationMode`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SpatializationMode {
    None,
    Stereo,
    ThreeD,
}

/// The fields of `struct MIX_Mixer` guarded by `LockMixer`.
struct MixerState {
    default_group: Option<Arc<GroupInner>>,
    all_tracks: Vec<Arc<TrackInner>>,
    fire_and_forget_pool: Vec<Arc<TrackInner>>, // these are also listed in all_tracks.
    all_groups: Vec<Arc<GroupInner>>,
    postmix_callback: Option<PostMixCallback>,
    mix_buffer: Vec<f32>,
    mix_bytes: Vec<u8>,
    actual_mixed_bytes: i32, // on each iteration of the mixer, number of bytes of real mixed audio, ignoring silence at end if no audio was available to mix there.
    gain: f32,
}

/// `mixer->spec` and `mixer->vbap2d`, which tracks read while they hold
/// their own lock (so this is a leaf lock, taken last and never held over
/// a call).
#[derive(Clone)]
struct MixerFormat {
    spec: AudioSpec,
    vbap2d: Vbap2d,
}

/// Translation of `struct MIX_Mixer`.
pub(crate) struct MixerInner {
    output_stream: Option<AudioStream>, // (only `None` while it's being dropped)
    device_id: AudioDeviceID, // can be zero if created from MIX_CreateMixer instead of MIX_CreateMixerDevice.
    props: Properties,
    track_tags: Mutex<BTreeMap<String, Arc<TagList>>>,
    format: Mutex<MixerFormat>,
    state: ReentrantMutex<RefCell<MixerState>>,
    event_watch: Mutex<Option<EventWatch>>,
    destroyed: AtomicBool,
}

/// `LockMixer()` (the output stream's lock, then the state's).
struct MixerGuard<'a> {
    state: ReentrantMutexGuard<'a, RefCell<MixerState>>,
    _stream: AudioStreamLock<'a>,
}

impl std::ops::Deref for MixerGuard<'_> {
    type Target = RefCell<MixerState>;
    fn deref(&self) -> &RefCell<MixerState> {
        &self.state
    }
}

impl MixerInner {
    fn stream(&self) -> &AudioStream {
        self.output_stream
            .as_ref()
            .expect("the output stream lives as long as the mixer")
    }

    /// Translation of `LockMixer()`; dropping the guard is `UnlockMixer()`.
    fn lock(&self) -> MixerGuard<'_> {
        let stream = self.stream().lock();
        MixerGuard {
            state: self.state.lock(),
            _stream: stream,
        }
    }

    fn spec(&self) -> AudioSpec {
        lock(&self.format).spec
    }

    /// Translation of `CheckMixerParam()`.
    fn check(&self) -> Result<()> {
        check_initialized()?;
        if self.destroyed.load(Ordering::Acquire) {
            return Err(Error::invalid_param("mixer"));
        }
        Ok(())
    }
}

impl Drop for MixerInner {
    fn drop(&mut self) {
        drop(self.output_stream.take()); // SDL_DestroyAudioStream(mixer->output_stream)
        if self.device_id != 0 {
            sdl3::init::quit_subsystem(InitFlags::AUDIO);
        }
    }
}

/// `MIX_TagList`: the tracks with a tag.
struct TagList {
    tracks: RwLock<Vec<Arc<TrackInner>>>,
}

/// The fields of `struct MIX_Track` guarded by `LockTrack`.
struct TrackState {
    position3d: [f32; 4], // we only need the X, Y, and Z coords, but the 4th element makes this SIMD-friendly.
    spatialization_mode: SpatializationMode,
    spatialization_panning: [f32; 2],
    spatialization_speakers: [i32; 2],
    input_buffer: Vec<u8>, // a place to process audio as it progresses through the callback.
    input_audio: Option<Audio>, // set if used with MIX_SetTrackAudio. Holds a reference.
    decoder: Option<Box<dyn TrackData>>, // MIX_Decoder-specific data for this run, if any (this owns the track's IOStream, and its I/O clamp).
    halt_when_exhausted: bool,           // true if we should stop the track when input runs out.
    input_stream: Option<Arc<AudioStream>>, // used for both MIX_SetTrackAudio and MIX_SetTrackAudioStream. Maybe not owned by SDL_mixer!
    internal_stream: Option<Arc<AudioStream>>, // used with MIX_SetTrackAudio, where it is also assigned to input_stream. Owned by SDL_mixer!
    output_spec: AudioSpec,                    // processed data we send to SDL is in this format.
    state: TrackStateKind,                     // playing, paused, stopped.
    position: u64,                             // sample frames played from start of file.
    silence_frames: i64, // number of frames of silence to mix at the end of the track.
    max_frame: i64,      // consider audio at EOF at this many sample frame position.
    fire_and_forget: bool, // true if this is a MIX_Track managed internally for fire-and-forget playback.
    total_fade_frames: i64, // fade in or out for this many sample frames.
    fade_frames: i64,      // remaining frames to fade.
    fade_direction: i32,   // -1: fade out  0: don't fade  1: fade in
    fade_start_gain: f32, // between 0.0f and 1.0f. Fade with this volume as the starting point (fade-in only).
    loops_remaining: i32, // seek to loop_start and continue this many more times at end of input. Negative to loop forever.
    loop_start: i32, // sample frame position for loops to begin, so you can play an intro once and then loop from an internal point thereafter.
    raw_callback: Option<TrackMixCallback>,
    cooked_callback: Option<TrackMixCallback>,
    stopped_callback: Option<TrackStoppedCallback>,
}

/// Translation of `struct MIX_Track`.
pub(crate) struct TrackInner {
    mixer: Weak<MixerInner>,
    output_stream: AudioStream, // the stream that is bound to the audio device.
    props: Properties,
    tags: Mutex<BTreeMap<String, bool>>, // lookup tags to see if they are currently applied to this track (true or false).
    group: Mutex<Option<Arc<GroupInner>>>, // might be default_group, which should not be returned to the app (the app sees that as a NULL group).
    state: ReentrantMutex<RefCell<TrackState>>,
    currently_inuse: AtomicBool, // true when we want to delay an app's MIX_DestroyTrack request (they destroyed it from a mixer callback).
    destroy_requested: AtomicBool, // true if MIX_DestroyTrack called while the track is actively mixing.
    destroyed: AtomicBool,
}

/// `LockTrack()` (the output stream's lock, then the state's).
struct TrackGuard<'a> {
    state: ReentrantMutexGuard<'a, RefCell<TrackState>>,
    _stream: AudioStreamLock<'a>,
}

impl std::ops::Deref for TrackGuard<'_> {
    type Target = RefCell<TrackState>;
    fn deref(&self) -> &RefCell<TrackState> {
        &self.state
    }
}

impl TrackInner {
    /// Translation of `LockTrack()`; dropping the guard is `UnlockTrack()`.
    fn lock(&self) -> TrackGuard<'_> {
        let stream = self.output_stream.lock();
        TrackGuard {
            state: self.state.lock(),
            _stream: stream,
        }
    }

    /// Translation of `CheckTrackParam()`.
    fn check(&self) -> Result<()> {
        check_initialized()?;
        if self.destroyed.load(Ordering::Acquire) {
            return Err(Error::invalid_param("track"));
        }
        Ok(())
    }

    fn mixer(&self) -> Result<Arc<MixerInner>> {
        self.mixer
            .upgrade()
            .ok_or_else(|| Error::invalid_param("track"))
    }
}

/// Translation of `struct MIX_Group`.
pub(crate) struct GroupInner {
    mixer: Weak<MixerInner>,
    tracks: Mutex<Vec<Arc<TrackInner>>>,
    props: Properties,
    postmix_callback: Mutex<Option<GroupMixCallback>>,
    destroyed: AtomicBool,
}

impl GroupInner {
    /// Translation of `CheckGroupParam()`.
    fn check(&self) -> Result<()> {
        check_initialized()?;
        if self.destroyed.load(Ordering::Acquire) {
            return Err(Error::invalid_param("group"));
        }
        Ok(())
    }
}

/// Translation of `struct MIX_Audio`.
pub(crate) struct AudioInner {
    props: Properties,
    spec: AudioSpec,
    decoder: &'static Decoder,
    decoder_userdata: Arc<dyn AudioData>,
    precache: Option<Precache>, // set if this cached the audio data (might be unset if we're feeding from an external SDL_IOStream).
    duration_frames: i64,
    clamp_offset: i64,
    clamp_length: i64,
}

/// Translation of `struct MIX_AudioDecoder`.
pub(crate) struct AudioDecoderInner {
    audio: Audio,
    track_userdata: Mutex<Option<Box<dyn TrackData>>>, // (this owns the decoder's IOStream)
    stream: AudioStream,
    destroyed: AtomicBool,
}

/// Translation of `CheckInitialized()`.
fn check_initialized() -> Result<()> {
    if lock_global().mixer_initialized == 0 {
        return Err(Error::new("Mixer not initialized (call MIX_Init first)"));
    }
    Ok(())
}

/// Translation of `CheckMixerTagParam()` and `CheckTrackTagParam()` (the
/// tag can't be missing here).
fn check_tag(tag: &str) -> Result<()> {
    let _ = tag;
    Ok(())
}

/// Translation of `SetTrackOutputStreamFormat()`.
fn set_track_output_stream_format(
    track: &TrackInner,
    st: &mut TrackState,
    mixer_spec: &AudioSpec,
    spec: Option<&AudioSpec>,
) -> bool {
    st.output_spec = *mixer_spec;
    if st.spatialization_mode == SpatializationMode::ThreeD {
        st.output_spec.channels = 1;
    } else if st.spatialization_mode == SpatializationMode::Stereo {
        st.output_spec.channels = 2;
    }

    let retval = track
        .output_stream
        .set_format(spec, Some(&st.output_spec))
        .is_ok(); // input is `spec`, output is to mixer->output_stream (or, if spatializing, to mixer->output_stream but mono...if force_stereo, output_stream but stereo).
    debug_assert!(retval);
    retval
}

// catch events to see if output device format has changed. This can let us move to/from surround sound support on the fly, not to mention spend less time doing unnecessary conversions.
/// Translation of `AudioDeviceChangeEventWatcher()`.
fn audio_device_change_event_watcher(mixer: &Arc<MixerInner>, event: &Event) {
    let Event::AudioDevice(adevice) = event else {
        return; // don't care about this event.
    };
    if adevice.event_type != EventType::AUDIO_DEVICE_FORMAT_CHANGED {
        return; // don't care about this event.
    } else if mixer.device_id != adevice.which {
        return; // don't care about this device.
    } else if mixer.device_id == 0 {
        return; // don't care about this mixer.
    }

    let guard = mixer.lock();

    // adjust all our output streams to the new format.
    if let Ok(mut spec) = mixer.stream().dst_format() {
        spec.format = AudioFormat::F32;
        if mixer.stream().set_format(Some(&spec), None).is_ok() {
            let format = {
                let mut f = lock(&mixer.format);
                f.spec = spec;
                f.vbap2d.init(spec.channels); // deal with channel count changing.
                f.clone()
            };
            let tracks = guard.borrow().all_tracks.clone();
            for track in tracks {
                let tguard = track.lock();
                let mut st = tguard.borrow_mut();
                set_track_output_stream_format(&track, &mut st, &format.spec, None); // input is from internal_stream, output is to mixer->output_stream (or, if spatializing, to mixer->output_stream but mono).
                if st.spatialization_mode == SpatializationMode::ThreeD {
                    // deal with channel count changing.
                    let position = st.position3d;
                    let TrackState {
                        spatialization_panning,
                        spatialization_speakers,
                        ..
                    } = &mut *st;
                    spatialize(
                        &format.vbap2d,
                        &position,
                        spatialization_panning,
                        spatialization_speakers,
                    );
                }
            }
        }
    }
}

/// Translation of `TrackStopped()`.
// this assumes LockTrack(track) was called before this.
fn track_stopped(track: &Arc<TrackInner>) {
    let (stopped_callback, fire_and_forget) = {
        let guard = track.state.lock();
        let mut st = guard.borrow_mut();
        debug_assert!(st.state != TrackStateKind::Stopped); // shouldn't be already stopped at this point.
        st.state = TrackStateKind::Stopped;
        (st.stopped_callback.clone(), st.fire_and_forget)
    };
    if let Some(cb) = stopped_callback {
        cb(&Track::borrowed(track));
    }
    if fire_and_forget {
        let _ = set_track_audio(track, None);
        let Ok(mixer) = track.mixer() else {
            return;
        };
        let guard = mixer.lock(); // !!! FIXME: this locks the mixer after the track; everything else locks in the other order! But StopTrack() is the only place outside the mixer thread (which holds both locks already) that calls this, and it shouldn't be able to call it for fire-and-forget tracks. Clean this up or at least document this better.
        guard.borrow_mut().fire_and_forget_pool.push(track.clone());
    }
}

/// Translation of `ApplyFade()`.
fn apply_fade(st: &mut TrackState, channels: i32, pcm: &mut [f32], frames: i32) {
    debug_assert!(frames >= 0);

    // !!! FIXME: this is probably pretty naive.

    if st.fade_direction == 0 {
        return; // no fade is happening, early exit.
    }

    let to_be_faded = st.fade_frames.min(frames as i64) as i32;
    let total_fade_frames = st.total_fade_frames as i32;
    let mut fade_frame_position = total_fade_frames - st.fade_frames as i32;

    // some hacks to avoid a branch on each sample frame. Might not be a good idea in practice.
    let fade_start_gain = st.fade_start_gain;
    let pctmult = (1.0 - fade_start_gain) * if st.fade_direction < 0 { 1.0 } else { -1.0 };
    let pctsub = if st.fade_direction < 0 { 1.0f32 } else { 0.0 };
    let ftotal_fade_frames = total_fade_frames as f32;

    debug_assert!((fade_start_gain == 0.0) || (st.fade_direction > 0)); // we only allow fade _in_ from arbitrary levels. Fade out always operates on the full signal down to zero.

    let channels = channels.max(0) as usize;
    for frame in pcm
        .chunks_exact_mut(channels.max(1))
        .take(to_be_faded.max(0) as usize)
    {
        let pct = ((pctsub - ((fade_frame_position as f32) / ftotal_fade_frames)) * pctmult)
            + fade_start_gain;
        debug_assert!(pct >= 0.0);
        debug_assert!(pct <= 1.0);
        fade_frame_position += 1;

        // use this fade percentage for the entire sample frame.
        // (upstream unrolls this for up to 8 channels.) // !!! FIXME: profile this and see if this is a dumb idea.
        for sample in frame.iter_mut().take(channels) {
            *sample *= pct;
        }
    }

    st.fade_frames -= to_be_faded as i64;
    debug_assert!(st.fade_frames >= 0);
    if st.fade_frames == 0 {
        // fade is done.
        if st.fade_direction < 0 {
            st.loops_remaining = 0; // we were fading out, don't loop anymore.
        }
        st.fade_direction = 0;
    }
}

/// Translation of `DecodeMore()`.
fn decode_more(st: &mut TrackState, bytes_needed: i32) -> bool {
    debug_assert!(st.input_audio.is_some());

    let (Some(input_stream), Some(decoder)) = (st.input_stream.clone(), st.decoder.as_mut()) else {
        return false;
    };
    let mut retval = true;
    while input_stream.available() < bytes_needed {
        if !decoder.decode(&input_stream) {
            input_stream.flush(); // make sure we read _everything_ now.
            retval = false;
            break;
        }
    }

    retval
}

/// Translation of `FillSilenceFrames()`.
fn fill_silence_frames(st: &mut TrackState, buffer: &mut [u8], channels: i32, buflen: i32) -> i32 {
    debug_assert!(st.silence_frames > 0);
    debug_assert!(buflen > 0);
    let max_silence_bytes = (st.silence_frames * channels as i64 * 4) as i32;
    let br = buflen.min(max_silence_bytes);
    if br != 0 {
        buffer[..br.max(0) as usize].fill(0);
        st.silence_frames -= (br / (channels * 4)) as i64;
    }
    br
}

// This is called every time we try to pull more from a track's output_stream.
// We generate more audio here on-demand, either from a decoder, or pulling
// from another audio stream.
// track->output_stream is locked when calling this.
/// Translation of `TrackGetCallback()`.
fn track_get_callback(track: &Arc<TrackInner>, stream: &AudioStream, additional_amount: i32) {
    let guard = track.state.lock();

    let (input_stream, output_framesize) = {
        let st = guard.borrow();
        if additional_amount == 0 {
            return; // don't need to generate more audio yet.
        } else if st.state != TrackStateKind::Playing {
            return; // paused or stopped, don't make progress.
        }

        debug_assert!(st.output_spec.format == AudioFormat::F32);

        (st.input_stream.clone(), st.output_spec.frame_size() as i32)
    };

    let raw_spec = input_stream
        .as_ref()
        .and_then(|s| s.dst_format().ok())
        .unwrap_or_default();

    // do we need to grow our buffer?
    let mut pcm = std::mem::take(&mut guard.borrow_mut().input_buffer); // we always work in float32 format.
    if additional_amount as usize > pcm.len() {
        if pcm
            .try_reserve(additional_amount as usize - pcm.len())
            .is_err()
        {
            // uhoh.
            drop(guard);
            track_stopped(track);
            return; // not much to be done, we're out of memory!
        }
        pcm.resize(additional_amount as usize, 0);
    }

    let mut bytes_remaining = additional_amount;

    // Calling TrackStopped() might have a stopped_callback that restarts the track, so don't break the loop
    //  for simply being stopped, so we can generate audio without gaps. If not restarted, track->state will no longer be PLAYING.
    while (guard.borrow().state == TrackStateKind::Playing) && (bytes_remaining > 0) {
        let mut end_of_audio = false;
        let mut br: i32 = 0; // bytes read.

        // make sure we're not trying to read half a sample frame.
        bytes_remaining = bytes_remaining.max(output_framesize);
        // (that can ask for more than the buffer holds; upstream would
        // overrun it.)
        if bytes_remaining as usize > pcm.len() {
            pcm.resize(bytes_remaining as usize, 0);
        }

        let input_stream = guard.borrow().input_stream.clone();
        let silence_frames = guard.borrow().silence_frames;
        if silence_frames > 0 {
            debug_assert!(input_stream.is_some()); // should have data bound if you landed here (we need raw_spec to be initialized).
            br = fill_silence_frames(
                &mut guard.borrow_mut(),
                &mut pcm,
                raw_spec.channels,
                bytes_remaining,
            );
        } else if let Some(input_stream) = &input_stream {
            if guard.borrow().input_audio.is_some() {
                decode_more(&mut guard.borrow_mut(), bytes_remaining);
            }
            br = match input_stream.get_data(&mut pcm[..bytes_remaining as usize]) {
                Ok(n) => n as i32,
                Err(_) => -1,
            };
        }

        // if input_audio and input_stream are both NULL, there's nothing to play (maybe they changed out the input on us?), br will be zero and we'll go to end_of_audio=true.

        if br <= 0 {
            // if 0: EOF. if < 0: decoding/input failure, we're done by default. But maybe it'll loop and play the start again...!
            end_of_audio = true;
        } else {
            debug_assert!(input_stream.is_some()); // should have data bound if you landed here.

            // this (probably?) shouldn't be a partial read here. It's either we completely filled the buffer or exhausted the data.
            //  as such, this does raw_callback() as likely the entire buffer, or all we're getting before a finish callback would have to fire,
            //  even if the finish callback would restart the track. As such, the outer loop is mostly here to deal with looping tracks
            //  and finish callbacks that restart the track.

            // if this would put us past the end of maxframes, or a fadeout, clamp br and set end_of_audio=true so we can do looping, etc.
            let raw_channels = raw_spec.channels.max(1);
            let mut frames_read = br / (4 * raw_channels);
            {
                let st = guard.borrow();
                let mut maxpos: i64 = -1;
                if st.max_frame >= 0 {
                    maxpos = st.max_frame;
                }
                if st.fade_direction < 0 {
                    let maxfadepos = (st.position as i64).wrapping_add(st.fade_frames);
                    if (maxpos < 0) || (maxfadepos < maxpos) {
                        maxpos = maxfadepos;
                    }
                }

                if maxpos >= 0 {
                    let newpos = (st.position as i64).wrapping_add(frames_read as i64);
                    if newpos >= maxpos {
                        // we read past the end of the fade out or maxframes, we need to clamp.
                        br -= ((newpos - maxpos) * raw_channels as i64 * 4) as i32;
                        if br < 0 {
                            br = 0;
                        }
                        frames_read = br / (4 * raw_channels);
                        end_of_audio = true;
                    }
                }
            }

            // give the app a shot at the final buffer before sending it on through transformations.
            let samples = frames_read * raw_channels;
            let put_bytes = samples * 4;
            let mut pcmf = bytes_to_f32(&pcm[..put_bytes as usize]);

            let raw_callback = guard.borrow().raw_callback.clone();
            if let Some(cb) = raw_callback {
                cb(&Track::borrowed(track), &raw_spec, &mut pcmf);
            }

            apply_fade(
                &mut guard.borrow_mut(),
                raw_channels,
                &mut pcmf,
                frames_read,
            );

            f32_to_bytes(&pcmf, &mut pcm[..put_bytes as usize]);
            let _ = stream.put_data(&pcm[..put_bytes as usize]);

            let mut st = guard.borrow_mut();
            st.position = st.position.wrapping_add(frames_read as u64);
            bytes_remaining -= put_bytes;
        }

        // remember that the callback in TrackStopped() might restart this track,
        //  so we'll loop to see if we can fill in more audio without a gap even in that case.
        if end_of_audio {
            let mut track_stopped_now = false;
            let halt_when_exhausted;
            {
                let mut st = guard.borrow_mut();
                if st.input_audio.is_some() {
                    if let Some(s) = &st.input_stream {
                        s.clear(); // make sure that any extra buffered input is removed.
                    }
                }
                if st.loops_remaining == 0 {
                    if st.silence_frames < 0 {
                        st.silence_frames = -st.silence_frames; // time to start appending silence.
                    } else {
                        track_stopped_now = true; // out of data, no loops remain, no appended silence left, we're done.
                    }
                } else {
                    if st.loops_remaining > 0 {
                        // negative means infinite loops, so don't decrement for that.
                        st.loops_remaining -= 1;
                    }
                    if st.input_audio.is_none() {
                        // can't loop on a streaming input, you're done.
                        track_stopped_now = true;
                    } else {
                        let loop_start = st.loop_start;
                        let seeked = st
                            .decoder
                            .as_mut()
                            .is_some_and(|d| d.seek(loop_start as i64 as u64).is_ok());
                        if !seeked {
                            track_stopped_now = true; // uhoh, can't seek! Abandon ship!
                        } else {
                            st.position = loop_start as i64 as u64;
                        }
                    }
                }
                halt_when_exhausted = st.halt_when_exhausted;
            }

            if track_stopped_now {
                if halt_when_exhausted {
                    track_stopped(track);
                } else {
                    break; // done with this track for now, but don't halt the track. We'll try it again later.
                }
            }
        }
    }

    let mut st = guard.borrow_mut();
    if st.input_buffer.len() < pcm.len() {
        st.input_buffer = pcm;
    }
}

/// Translation of `MixSpatializedFloat32Audio()`.
fn mix_spatialized_float32_audio(
    dst: &mut [f32],
    src: &[f32],
    samples: usize,
    output_channels: usize,
    panning: &[f32; 2],
    speakers: &[i32; 2],
    gain: f32,
) {
    let panning0 = panning[0] * gain;
    let panning1 = panning[1] * gain;
    let speaker0 = speakers[0] as usize;
    let speaker1 = speakers[1] as usize;

    // !!! FIXME: a common case (output_channels==2, speaker0=0, speaker1=1) can be easily SIMD'd.
    // !!! FIXME: unroll this loop?
    if (panning0 == 0.0) && (panning1 == 0.0) {
        // don't mix silence.
    } else if (panning0 == 1.0) && (panning1 == 1.0) {
        // no modulation.
        for (d, &sample) in dst.chunks_exact_mut(output_channels).zip(&src[..samples]) {
            d[speaker0] += sample;
            d[speaker1] += sample;
        }
    } else {
        for (d, &sample) in dst.chunks_exact_mut(output_channels).zip(&src[..samples]) {
            d[speaker0] += sample * panning0;
            d[speaker1] += sample * panning1;
        }
    }
}

/// Translation of `MixForcedStereoFloat32Audio()`.
fn mix_forced_stereo_float32_audio(
    dst: &mut [f32],
    src: &[f32],
    sample_frames: usize,
    output_channels: usize,
    panning: &[f32; 2],
    gain: f32,
) {
    let panning0 = panning[0] * gain;
    let panning1 = panning[1] * gain;

    // !!! FIXME: a common case (output_channels==2) can be easily SIMD'd.
    // !!! FIXME: unroll this loop?
    if (panning0 == 0.0) && (panning1 == 0.0) {
        // don't mix silence.
    } else if (panning0 == 1.0) && (panning1 == 1.0) {
        // no modulation.
        for (d, s) in dst
            .chunks_exact_mut(output_channels)
            .zip(src.chunks_exact(2))
            .take(sample_frames)
        {
            d[0] += s[0];
            d[1] += s[1];
        }
    } else {
        for (d, s) in dst
            .chunks_exact_mut(output_channels)
            .zip(src.chunks_exact(2))
            .take(sample_frames)
        {
            d[0] += s[0] * panning0;
            d[1] += s[1] * panning1;
        }
    }
}

/// Translation of `MixFloat32Audio()`, with `SDL_MixAudio()`'s float path:
/// nothing is mixed when the gain rounds to zero in 1/128 steps, and the
/// sums are clamped to -1.0..1.0.
fn mix_float32_audio(dst: &mut [f32], src: &[f32], buffer_size: usize, gain: f32) {
    if gain == 0.0 {
        return; // don't mix silence.
    }
    if sdl3::stdlib::math::roundf(gain * 128.0) as i32 == 0 {
        return;
    }
    let n = buffer_size / 4;
    for (d, &s) in dst.iter_mut().zip(src).take(n) {
        let mut dst_sample = s * gain + *d;
        if dst_sample > 1.0 {
            dst_sample = 1.0;
        } else if dst_sample < -1.0 {
            dst_sample = -1.0;
        }
        *d = dst_sample;
    }
}

// SDL calls this function from the audio device thread as more data is needed the mixer.
/// Translation of `MixerCallback()`.
fn mixer_callback(mixer: &Arc<MixerInner>, stream: &AudioStream, additional_amount: i32) {
    let guard = mixer.state.lock();
    guard.borrow_mut().actual_mixed_bytes = 0;

    if additional_amount <= 0 {
        return; // nothing to actually do yet. This was a courtesy call; the stream still has enough buffered.
    }

    // it should be asking for float data...
    debug_assert!((additional_amount % 4) == 0);

    // !!! FIXME: maybe we should do a consistent buffer size, to make this easier for app callbacks
    // !!! FIXME:  and save some trouble on systems that want to do like 200 samples at a time.

    let (groups, gain, mut mix_buffer, mut getbytes) = {
        let mut st = guard.borrow_mut();
        (
            st.all_groups.clone(),
            st.gain,
            std::mem::take(&mut st.mix_buffer),
            std::mem::take(&mut st.mix_bytes),
        )
    };
    let spec = mixer.spec();

    // do we need to grow our buffer?
    let skip_group_mixing = groups.len() <= 1;
    let alloc_multiplier = if skip_group_mixing { 1 } else { 2 };
    let nfloats = additional_amount as usize / 4;
    let alloc_size = nfloats * alloc_multiplier;
    if alloc_size > mix_buffer.len() {
        if mix_buffer
            .try_reserve(alloc_size - mix_buffer.len())
            .is_err()
        {
            return; // not much to be done, we're out of memory!
        }
        mix_buffer.resize(alloc_size, 0.0);
    }
    if additional_amount as usize > getbytes.len() {
        if getbytes
            .try_reserve(additional_amount as usize - getbytes.len())
            .is_err()
        {
            return;
        }
        getbytes.resize(additional_amount as usize, 0);
    }

    let (final_mixbuf, rest) = mix_buffer.split_at_mut(nfloats);
    final_mixbuf.fill(0.0);
    let mut group_mixbuf_storage = if skip_group_mixing {
        None
    } else {
        Some(&mut rest[..nfloats])
    };

    let mixer_framesize = spec.frame_size() as i32;
    let mut actual_mixed_bytes = 0;

    for group in &groups {
        let group_mixbuf: &mut [f32] = match &mut group_mixbuf_storage {
            Some(g) => {
                g.fill(0.0); // if skip_group_mixing, this is final_mixbuf, which we just zero'd out.
                g
            }
            None => final_mixbuf,
        };

        let mut group_bytes: i32 = 0;
        let tracks = lock(&group.tracks).clone(); // (a snapshot: this won't save you from a callback going totally rogue, but it'll deal with the current track leaving the group.)
        for track in &tracks {
            let tguard = track.lock();

            track.currently_inuse.store(true, Ordering::Release);

            let output_spec = tguard.borrow().output_spec;
            let to_be_read =
                (additional_amount / mixer_framesize.max(1)) * output_spec.frame_size() as i32;
            let to_be_read = (to_be_read.max(0) as usize).min(getbytes.len());
            let br = match track.output_stream.get_data(&mut getbytes[..to_be_read]) {
                Ok(n) => n as i32,
                Err(_) => -1,
            };
            if br > 0 {
                let mut getbuf = bytes_to_f32(&getbytes[..br as usize]);
                let cooked_callback = tguard.borrow().cooked_callback.clone();
                if let Some(cb) = cooked_callback {
                    cb(&Track::borrowed(track), &output_spec, &mut getbuf);
                }

                let st = tguard.borrow();
                let channels = spec.channels.max(1) as usize;
                match st.spatialization_mode {
                    SpatializationMode::None => {
                        debug_assert!(st.output_spec.channels == spec.channels);
                        mix_float32_audio(group_mixbuf, &getbuf, br as usize, gain);
                        group_bytes = group_bytes.max(br);
                    }

                    SpatializationMode::ThreeD => {
                        debug_assert!(st.output_spec.channels == 1);
                        mix_spatialized_float32_audio(
                            group_mixbuf,
                            &getbuf,
                            br as usize / 4,
                            channels,
                            &st.spatialization_panning,
                            &st.spatialization_speakers,
                            gain,
                        );
                        group_bytes = group_bytes.max(br * spec.channels);
                    }

                    SpatializationMode::Stereo => {
                        debug_assert!(st.output_spec.channels == 2);
                        mix_forced_stereo_float32_audio(
                            group_mixbuf,
                            &getbuf,
                            br as usize / 8,
                            channels,
                            &st.spatialization_panning,
                            gain,
                        );
                        group_bytes = group_bytes.max((br / 2) * spec.channels);
                    }
                }
            }

            track.currently_inuse.store(false, Ordering::Release);
            let destroy_requested = track.destroy_requested.load(Ordering::Acquire); // save this off just in case, but if the callback destroyed the track, _nothing_ else should touch it once this unlocks.
            drop(tguard);

            if destroy_requested {
                // callback asked to destroy the track while we were still using it.
                destroy_track(track); // actually kill it now.
            }
        }

        if group_bytes > actual_mixed_bytes {
            actual_mixed_bytes = group_bytes;
        }

        let postmix_callback = lock(&group.postmix_callback).clone();
        if let Some(cb) = postmix_callback {
            cb(&Group::borrowed(group), &spec, group_mixbuf);
        }

        if let Some(g) = &group_mixbuf_storage {
            mix_float32_audio(final_mixbuf, g, group_bytes.max(0) as usize, 1.0);
            // we adjusted for mixer->gain for each track, don't adjust gain here, too.
        }
    }

    let postmix_callback = guard.borrow().postmix_callback.clone();
    if let Some(cb) = postmix_callback {
        cb(&Mixer::borrowed(mixer), &spec, final_mixbuf);
    }

    let mut out = vec![0u8; additional_amount as usize];
    f32_to_bytes(final_mixbuf, &mut out);
    let _ = stream.put_data(&out);

    let mut st = guard.borrow_mut();
    st.actual_mixed_bytes = actual_mixed_bytes;
    if st.mix_buffer.is_empty() {
        st.mix_buffer = mix_buffer;
    }
    if st.mix_bytes.is_empty() {
        st.mix_bytes = getbytes;
    }
}

/// Translation of `InitDecoders()`.
fn init_decoders(g: &mut GlobalState) {
    for decoder in DECODERS {
        if decoder.init.is_none_or(|init| init()) {
            g.available_decoders.push(decoder);
        }
    }
}

/// Translation of `QuitDecoders()`.
fn quit_decoders(g: &mut GlobalState) {
    for decoder in g.available_decoders.drain(..) {
        if let Some(quit) = decoder.quit {
            quit();
        }
    }
}

/// The version of SDL_mixer this crate translates. Translation of
/// `MIX_Version()` (which reports the linked library's version; this is
/// [`VERSION`](crate::VERSION)).
pub fn version() -> sdl3::Version {
    crate::VERSION
}

/// Initialize the SDL_mixer library. Translation of `MIX_Init()`.
///
/// This must be called before anything else here. Calls nest: each needs
/// a matching [`quit`].
///
/// Upstream checks for SSE (or NEON) here, which its SIMD code needs; the
/// translation computes everything in portable code.
pub fn init() -> Result<()> {
    let mut g = lock_global();
    if g.mixer_initialized == 0 {
        init_decoders(&mut g);
    }
    g.mixer_initialized += 1;
    Ok(())
}

/// Deinitialize the SDL_mixer library. Translation of `MIX_Quit()`.
///
/// When the last [`init`] is matched, every mixer and audio decoder is
/// destroyed (their handles fail from then on), and their tracks and
/// groups with them. [`Audio`]s are reference counted here, so the ones
/// the app still holds stay valid (Note (upstream): `MIX_Quit` frees them).
pub fn quit() {
    let (mixers, audiodecoders) = {
        let mut g = lock_global();
        debug_assert!(g.mixer_initialized >= 0);

        if g.mixer_initialized <= 0 {
            return; // not initialized.
        } else if g.mixer_initialized > 1 {
            g.mixer_initialized -= 1;
            return; // more refcounts to go.
        }

        // actually shutting down now.
        (
            std::mem::take(&mut g.all_mixers),
            std::mem::take(&mut g.all_audiodecoders),
        )
    };

    for mixer in &mixers {
        destroy_mixer(mixer);
    }

    for audiodecoder in &audiodecoders {
        destroy_audio_decoder(audiodecoder);
    }

    let mut g = lock_global();
    quit_decoders(&mut g);
    g.mixer_initialized = 0;
}

/// The number of audio decoders available. Translation of
/// `MIX_GetNumAudioDecoders()`.
pub fn num_audio_decoders() -> Result<usize> {
    check_initialized()?;
    Ok(lock_global().available_decoders.len())
}

/// The name of an available audio decoder, such as `"WAV"`, `"DRMP3"` or
/// `"STBVORBIS"`; the names can be given as
/// [`PROP_AUDIO_DECODER_STRING`](crate::PROP_AUDIO_DECODER_STRING).
/// Translation of `MIX_GetAudioDecoder()`.
pub fn audio_decoder(index: usize) -> Result<&'static str> {
    check_initialized()?;
    let g = lock_global();
    match g.available_decoders.get(index) {
        Some(d) => Ok(d.name),
        None => Err(Error::invalid_param("index")),
    }
}

/// Translation of `CreateMixer()`.
fn create_mixer(stream: AudioStream, device: bool) -> Result<Mixer> {
    debug_assert!(check_initialized().is_ok());

    let (mut spec, output_spec) = stream.format()?;

    // on our end, we always work in float32 format.
    if spec.format != AudioFormat::F32 {
        spec.format = AudioFormat::F32;
        stream.set_format(Some(&spec), None)?;
    }

    let device_id = if device {
        stream.device().unwrap_or(0)
    } else {
        0
    };

    let mut vbap2d = Vbap2d::default();
    vbap2d.init(output_spec.channels);

    let inner = Arc::new(MixerInner {
        output_stream: Some(stream),
        device_id,
        props: Properties::new(),
        track_tags: Mutex::new(BTreeMap::new()),
        format: Mutex::new(MixerFormat { spec, vbap2d }),
        state: ReentrantMutex::new(RefCell::new(MixerState {
            default_group: None,
            all_tracks: Vec::new(),
            fire_and_forget_pool: Vec::new(),
            all_groups: Vec::new(),
            postmix_callback: None,
            mix_buffer: Vec::new(),
            mix_bytes: Vec::new(),
            actual_mixed_bytes: 0,
            gain: 1.0,
        })),
        event_watch: Mutex::new(None),
        destroyed: AtomicBool::new(false),
    });

    let default_group = create_group(&inner)?;
    inner.state.lock().borrow_mut().default_group = Some(default_group);

    let devnum = inner.stream().device().unwrap_or(0);
    let _ = inner.props.set(PROP_MIXER_DEVICE_NUMBER, devnum as i64);

    let weak = Arc::downgrade(&inner);
    inner.stream().set_get_callback(Some(
        move |stream: &AudioStream, additional_amount: i32, _total_amount: i32| {
            if let Some(mixer) = weak.upgrade() {
                mixer_callback(&mixer, stream, additional_amount);
            }
        },
    ));

    lock_global().all_mixers.insert(0, inner.clone());

    Ok(Mixer { inner, owner: true })
}

/// Translation of `MIX_DestroyMixer()`.
fn destroy_mixer(mixer: &Arc<MixerInner>) {
    if mixer.destroyed.swap(true, Ordering::AcqRel) {
        return; // harmless no-op.
    }

    {
        let mut g = lock_global();
        g.all_mixers.retain(|m| !Arc::ptr_eq(m, mixer));
    }

    let _ = stop_all_tracks(mixer, 0);

    let tracks = mixer.state.lock().borrow().all_tracks.clone();
    for track in &tracks {
        track.currently_inuse.store(false, Ordering::Release);
        destroy_track(track);
    }

    let groups = mixer.state.lock().borrow().all_groups.clone();
    for group in &groups {
        destroy_group(group);
    }

    *lock(&mixer.event_watch) = None; // SDL_RemoveEventWatch(AudioDeviceChangeEventWatcher, mixer)

    mixer
        .stream()
        .set_get_callback(None::<fn(&AudioStream, i32, i32)>);
    let guard = mixer.state.lock();
    let mut st = guard.borrow_mut();
    st.all_tracks.clear();
    st.fire_and_forget_pool.clear();
    st.all_groups.clear();
    st.default_group = None;
    st.postmix_callback = None;
    st.mix_buffer = Vec::new();
    lock(&mixer.track_tags).clear();
    // (the output stream, which closes the device, and the audio subsystem
    // go when the last handle does.)
}

/// An audio mixer: it plays any number of [`Track`]s at once, on an audio
/// device or into a buffer. Translation of `MIX_Mixer`.
///
/// Dropping the handle (the one from [`new`](Self::new) or
/// [`new_device`](Self::new_device)) destroys the mixer
/// (`MIX_DestroyMixer()`), with its tracks and groups.
pub struct Mixer {
    inner: Arc<MixerInner>,
    owner: bool,
}

impl fmt::Debug for Mixer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mixer")
            .field("device_id", &self.inner.device_id)
            .field("spec", &self.inner.spec())
            .finish_non_exhaustive()
    }
}

impl Drop for Mixer {
    fn drop(&mut self) {
        if self.owner {
            destroy_mixer(&self.inner);
        }
    }
}

/// A lock on a [`Mixer`]; see [`Mixer::lock`].
#[must_use]
pub struct MixerLock<'a> {
    _guard: MixerGuard<'a>,
}

impl fmt::Debug for MixerLock<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MixerLock")
    }
}

impl Mixer {
    fn borrowed(inner: &Arc<MixerInner>) -> Mixer {
        Mixer {
            inner: inner.clone(),
            owner: false,
        }
    }

    /// Create a mixer that plays on an audio device. Translation of
    /// `MIX_CreateMixerDevice()`.
    ///
    /// `devid` is a device to open, such as
    /// [`AUDIO_DEVICE_DEFAULT_PLAYBACK`](sdl3::audio::AUDIO_DEVICE_DEFAULT_PLAYBACK);
    /// `spec` is the format wanted (the device might not use it; `None`
    /// takes the device's). The audio subsystem is initialized for the
    /// mixer's lifetime, and the device starts playing at once. If the
    /// device's format changes, the mixer follows it.
    pub fn new_device(devid: AudioDeviceID, spec: Option<&AudioSpec>) -> Result<Mixer> {
        check_initialized()?;
        sdl3::init::init_subsystem(InitFlags::AUDIO)?;

        let stream = match sdl3::audio::open_audio_device_stream(
            devid,
            spec,
            None::<fn(&AudioStream, i32, i32)>,
        ) {
            Ok(s) => s,
            Err(e) => {
                sdl3::init::quit_subsystem(InitFlags::AUDIO);
                return Err(e);
            }
        };

        let mixer = match create_mixer(stream, true) {
            Ok(m) => m,
            Err(e) => {
                sdl3::init::quit_subsystem(InitFlags::AUDIO);
                return Err(e);
            }
        };

        let weak = Arc::downgrade(&mixer.inner);
        *lock(&mixer.inner.event_watch) = Some(sdl3::events::add_watch(move |event| {
            if let Some(mixer) = weak.upgrade() {
                audio_device_change_event_watcher(&mixer, event);
            }
        }));
        let _ = mixer.inner.stream().resume_device();

        Ok(mixer)
    }

    /// Create a mixer that generates audio into a buffer, with
    /// [`generate`](Self::generate). Translation of `MIX_CreateMixer()`.
    pub fn new(spec: &AudioSpec) -> Result<Mixer> {
        check_initialized()?;

        let stream = AudioStream::new(Some(spec), Some(spec))?;

        // we want this stream to survive SDL_Quit(), since it's not attached to an audio device.
        let _ = stream
            .properties()
            .set(PROP_AUDIOSTREAM_AUTO_CLEANUP_BOOLEAN, false);

        create_mixer(stream, false)
    }

    /// The mixer's properties ([`PROP_MIXER_DEVICE_NUMBER`](crate::PROP_MIXER_DEVICE_NUMBER)
    /// is set). Translation of `MIX_GetMixerProperties()`.
    pub fn properties(&self) -> Result<Properties> {
        self.inner.check()?;
        Ok(self.inner.props.clone())
    }

    /// The format the mixer produces (for a device mixer, the device's).
    /// Translation of `MIX_GetMixerFormat()`.
    pub fn format(&self) -> Result<AudioSpec> {
        self.inner.check()?;
        self.inner.stream().dst_format()
    }

    /// Lock the mixer, so a group of changes happens atomically: the mixer
    /// doesn't mix while it's held. Translation of `MIX_LockMixer()`;
    /// dropping the lock is `MIX_UnlockMixer()`.
    pub fn lock(&self) -> MixerLock<'_> {
        MixerLock {
            _guard: self.inner.lock(),
        }
    }

    /// Generate mixed audio into `buffer` (in the format from
    /// [`new`](Self::new)), for a mixer not on a device. Returns how many
    /// bytes of it are real mixed audio (the rest is silence, when the
    /// tracks ran out). Translation of `MIX_Generate()`.
    pub fn generate(&self, buffer: &mut [u8]) -> Result<usize> {
        self.inner.check()?;
        if self.inner.device_id != 0 {
            return Err(Error::new(
                "Can't use MIX_Generate with a MIX_Mixer from MIX_CreateMixerDevice",
            ));
        }
        let output_spec = self.inner.stream().dst_format()?;
        // FIXME (upstream): this tests `!SDL_GetAudioStreamData()`, so a read
        // of zero bytes fails and a failed read (-1) goes on.
        match self.inner.stream().get_data(buffer) {
            // will fire MixerCallback() to generate audio.
            Ok(0) => return Err(Error::new("No audio was generated")),
            Ok(_) | Err(_) => {}
        }

        let actual = self.inner.state.lock().borrow().actual_mixed_bytes;
        Ok((actual as usize / 4) * output_spec.format.bytesize() as usize)
    }

    /// Play an [`Audio`] once, on an internal track, with no further
    /// control. Translation of `MIX_PlayAudio()`.
    pub fn play_audio(&self, audio: &Audio) -> Result<()> {
        self.inner.check()?;
        audio.check()?;

        // grab an existing fire-and-forget track from the available pool.
        let track = self.inner.lock().borrow_mut().fire_and_forget_pool.pop();

        let track = match track {
            Some(t) => t,
            None => {
                // make a new item if the pool was empty.
                let t = create_track(&self.inner)?;
                t.state.lock().borrow_mut().fire_and_forget = true;
                t
            }
        };

        // !!! FIXME: put the track back in the fire/forget pool.
        set_track_audio(&track, Some(audio))?;

        let retval = play_track(&track, None);

        // !!! FIXME: MIX_PlayTrack should only fail for things we already validated here...but if this assertion fires, we need to put this track back in the fire/forget pool.
        debug_assert!(retval.is_ok());

        retval
    }

    /// Start (or restart) every track with `tag`, at the same time; see
    /// [`Track::play`]. Translation of `MIX_PlayTag()`.
    pub fn play_tag(&self, tag: &str, options: Option<&Properties>) -> Result<()> {
        self.inner.check()?;
        check_tag(tag)?;

        let Some(list) = lock(&self.inner.track_tags).get(tag).cloned() else {
            return Ok(()); // nothing is using this tag, do nothing (but not an error).
        };

        let mut retval = Ok(());
        let tracks = list
            .tracks
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let _guard = self.inner.lock(); // so all tracks start at the same time.

        for track in &tracks {
            let has_input = {
                let g = track.lock();
                let st = g.borrow();
                st.input_audio.is_some() || st.input_stream.is_some()
            };
            if has_input {
                // don't treat it as an error if no audio is available, just don't play it.
                if let Err(e) = play_track(track, options) {
                    retval = Err(e);
                }
            }
        }

        retval
    }

    /// Stop every track, after fading out over `fade_out_ms` milliseconds
    /// (or at once, for zero or less). Translation of `MIX_StopAllTracks()`.
    pub fn stop_all_tracks(&self, fade_out_ms: i64) -> Result<()> {
        stop_all_tracks(&self.inner, fade_out_ms)
    }

    /// Stop every track with `tag`; see [`stop_all_tracks`](Self::stop_all_tracks).
    /// Translation of `MIX_StopTag()`.
    pub fn stop_tag(&self, tag: &str, fade_out_ms: i64) -> Result<()> {
        self.inner.check()?;
        check_tag(tag)?;

        let Some(list) = lock(&self.inner.track_tags).get(tag).cloned() else {
            return Ok(()); // nothing is using this tag, do nothing (but not an error).
        };

        let tracks = list
            .tracks
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for track in &tracks {
            let fade_out_frames = track_ms_to_frames(track, fade_out_ms).max(0);
            stop_track(track, if fade_out_ms > 0 { fade_out_frames } else { -1 });
        }

        Ok(())
    }

    /// Pause every playing track. Translation of `MIX_PauseAllTracks()`.
    pub fn pause_all_tracks(&self) -> Result<()> {
        self.inner.check()?;

        let guard = self.inner.lock(); // lock the mixer so all tracks pause at the same time.
        let tracks = guard.borrow().all_tracks.clone();
        for track in &tracks {
            pause_track(track);
        }

        Ok(())
    }

    /// Pause every playing track with `tag`. Translation of `MIX_PauseTag()`.
    pub fn pause_tag(&self, tag: &str) -> Result<()> {
        self.inner.check()?;
        check_tag(tag)?;

        let Some(list) = lock(&self.inner.track_tags).get(tag).cloned() else {
            return Ok(()); // nothing is using this tag, do nothing (but not an error).
        };

        let _guard = self.inner.lock(); // lock the mixer so all tracks pause at the same time.
        let tracks = list
            .tracks
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for track in &tracks {
            pause_track(track);
        }

        Ok(())
    }

    /// Resume every paused track. Translation of `MIX_ResumeAllTracks()`.
    pub fn resume_all_tracks(&self) -> Result<()> {
        self.inner.check()?;

        let guard = self.inner.lock(); // lock the mixer so all tracks resume at the same time.
        let tracks = guard.borrow().all_tracks.clone();
        for track in &tracks {
            resume_track(track);
        }
        Ok(())
    }

    /// Resume every paused track with `tag`. Translation of `MIX_ResumeTag()`.
    pub fn resume_tag(&self, tag: &str) -> Result<()> {
        self.inner.check()?;
        check_tag(tag)?;

        let Some(list) = lock(&self.inner.track_tags).get(tag).cloned() else {
            return Ok(()); // nothing is using this tag, do nothing (but not an error).
        };

        let _guard = self.inner.lock(); // lock the mixer so all tracks resume at the same time.
        let tracks = list
            .tracks
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for track in &tracks {
            resume_track(track);
        }

        Ok(())
    }

    /// Set the gain applied to everything the mixer mixes (1.0 is
    /// unchanged; must not be negative). Translation of `MIX_SetMixerGain()`.
    pub fn set_gain(&self, gain: f32) -> Result<()> {
        self.inner.check()?;
        if gain < 0.0 {
            return Err(Error::invalid_param("gain"));
        }

        self.inner.lock().borrow_mut().gain = gain;
        Ok(())
    }

    /// The mixer's gain. Translation of `MIX_GetMixerGain()`.
    pub fn gain(&self) -> Result<f32> {
        self.inner.check()?;
        Ok(self.inner.lock().borrow().gain)
    }

    /// Set the gain of every track with `tag` (negative gains are clamped to
    /// zero). Translation of `MIX_SetTagGain()`.
    pub fn set_tag_gain(&self, tag: &str, mut gain: f32) -> Result<()> {
        self.inner.check()?;
        check_tag(tag)?;

        if gain < 0.0 {
            gain = 0.0; // !!! FIXME: this clamps, but should it fail instead?
        }

        let Some(list) = lock(&self.inner.track_tags).get(tag).cloned() else {
            return Ok(()); // nothing is using this tag, do nothing (but not an error).
        };

        let _guard = self.inner.lock(); // lock the mixer so all tracks adust gain at the same time.
        let tracks = list
            .tracks
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for track in &tracks {
            let _ = set_track_gain(track, gain);
        }

        Ok(())
    }

    /// Change the speed (and pitch) of everything the mixer plays; clamped
    /// to 0.01..100.0. Translation of `MIX_SetMixerFrequencyRatio()`.
    pub fn set_frequency_ratio(&self, ratio: f32) -> Result<()> {
        self.inner.check()?;

        let ratio = ratio.clamp(0.01, 100.0); // !!! FIXME: this clamps, but should it fail instead?

        // don't have to LockMixer, as SDL_SetAudioStreamFrequencyRatio will do that.
        self.inner.stream().set_frequency_ratio(ratio)
    }

    /// The mixer's frequency ratio. Translation of `MIX_GetMixerFrequencyRatio()`.
    pub fn frequency_ratio(&self) -> Result<f32> {
        self.inner.check()?;

        // don't have to LockMixer, as SDL_GetAudioStreamFrequencyRatio will do that.
        Ok(self.inner.stream().frequency_ratio())
    }

    /// Set a callback that sees the final mix, after every group, before
    /// it goes to the device (or [`generate`](Self::generate)); `None`
    /// removes it. Translation of `MIX_SetPostMixCallback()`.
    pub fn set_postmix_callback(
        &self,
        cb: Option<impl Fn(&Mixer, &AudioSpec, &mut [f32]) + Send + Sync + 'static>,
    ) -> Result<()> {
        self.inner.check()?;

        let cb = cb.map(|c| Arc::new(c) as PostMixCallback);
        self.inner.lock().borrow_mut().postmix_callback = cb;

        Ok(())
    }

    /// The tracks with `tag`. Translation of `MIX_GetTaggedTracks()`.
    pub fn tagged_tracks(&self, tag: &str) -> Result<Vec<Track>> {
        self.inner.check()?;
        check_tag(tag)?;

        let Some(list) = lock(&self.inner.track_tags).get(tag).cloned() else {
            return Ok(Vec::new()); // nothing is using this tag?
        };

        let tracks = list.tracks.read().unwrap_or_else(|e| e.into_inner());
        Ok(tracks.iter().map(Track::borrowed).collect())
    }
}

/// Translation of `PrepareDecoder()`.
fn prepare_decoder(
    mut io: Option<&mut IoStream<'_>>,
    props: &Properties,
    spec: &mut AudioSpec,
    duration_frames: &mut i64,
) -> Result<(&'static Decoder, Arc<dyn AudioData>)> {
    let decoder_name = props.get_string(PROP_AUDIO_DECODER_STRING);

    let original_spec = *spec;

    let available = lock_global().available_decoders.clone();
    for decoder in available {
        if decoder_name
            .as_deref()
            .is_none_or(|name| strcasecmp(decoder.name, name) == std::cmp::Ordering::Equal)
        {
            if let Ok(userdata) =
                (decoder.init_audio)(io.as_deref_mut(), spec, props, duration_frames)
            {
                return Ok((decoder, userdata));
            } else if io
                .as_deref_mut()
                .is_none_or(|io| io.seek(0, IoWhence::Set).is_err())
            {
                // note this seeks to offset 0, because we're using an IoClamp.
                return Err(Error::new("Can't seek in stream to find proper decoder"));
            }
            *spec = original_spec; // reset this, in case init_audio changed it and then failed.
        }
    }

    Err(Error::new(
        "Audio data is in unknown/unsupported/corrupt format",
    ))
}

/// Translation of `DecodeWholeFile()`.
fn decode_whole_file(
    userdata: &Arc<dyn AudioData>,
    spec: &AudioSpec,
    props: &Properties,
    io: Option<&mut IoStream<'_>>,
) -> Result<Vec<u8>> {
    let stream = AudioStream::new(Some(spec), Some(spec))?; // !!! FIXME: if we're decoding up front, we might as well convert to float here too, right?
    let mut track_userdata = userdata
        .clone()
        .init_track(io.map(borrow_io), spec, props)?;
    if track_userdata.seek(0).is_ok() {
        while track_userdata.decode(&stream) {
            // spin.
        }
    }
    drop(track_userdata);

    stream.flush();
    let available = stream.available().max(0) as usize;
    let mut decoded = try_alloc::<u8>(available)?; // !!! FIXME: SIMD align?
    let rc = stream.get_data(&mut decoded)?;
    debug_assert!(rc == available);
    decoded.truncate(rc);
    Ok(decoded)
}

/// Translation of `MIX_LoadAudioWithProperties()`, with the stream, mixer
/// and no-copy data as arguments.
fn load_audio_with_properties(
    origio: Option<&mut IoStream<'_>>,
    mixer: Option<&Arc<MixerInner>>,
    props: Option<&Properties>,
    nocopy: Option<Precache>,
) -> Result<Audio> {
    check_initialized()?;

    let get_bool = |name: &str| props.and_then(|p| p.get_bool(name)).unwrap_or(false);
    let predecode = get_bool(PROP_AUDIO_LOAD_PREDECODE_BOOLEAN);
    let ondemand = get_bool(PROP_AUDIO_LOAD_ONDEMAND_BOOLEAN);
    let skip_metadata_tags = get_bool(PROP_AUDIO_LOAD_SKIP_METADATA_TAGS_BOOLEAN);
    let mut recommended_spec = AudioSpec::new(AudioFormat::F32, 2, 48000); // a reasonable default if no mixer specified.
    if let Some(mixer) = mixer {
        recommended_spec = mixer.spec();
    }

    let mut duration_frames = DURATION_UNKNOWN;
    let audio_props = Properties::new();

    if let Some(props) = props {
        audio_props.copy_from(props)?;
    }

    // check for ID3/APE/MusicMatch/whatever tags here, in case they were slapped onto the edge of any random file format.
    let mut clamp_offset = -1;
    let mut clamp_length = -1;
    let mut io: Option<IoStream<'_>> = None; // we'll replace this if parsing metadata tags.
    if let Some(origio) = origio {
        if !skip_metadata_tags {
            let mut clamp = IoClamp::open(borrow_io(origio))?;

            let orig_filelen = clamp.length;

            // !!! FIXME: currently we're ignoring return values from this function (see FIXME at the top of its code).
            read_metadata_tags(&mut clamp, &audio_props);
            if clamp.seek_io(0, IoWhence::Set) < 0 {
                return Err(Error::new("Error seeking in datastream"));
            }

            // will we need to apply an IoClamp when reading the real data later, too?
            if (clamp.start != 0) || (clamp.length != orig_filelen) {
                clamp_offset = clamp.start;
                clamp_length = clamp.length;
            }
            io = Some(IoStream::open(clamp));
        } else {
            io = Some(borrow_io(origio));
        }
    }

    // the decoder sets audio->spec to whatever it's actually providing, but we pass the current hardware setting in, in case that's useful for
    // things that generate audio in whatever format (for example, a MIDI decoder is going to generate PCM from "notes", so it can do it at any
    // sample rate, so it might as well do it at device format to avoid an unnecessary resample later).
    let mut spec = recommended_spec;

    let (mut decoder, mut audio_userdata) =
        prepare_decoder(io.as_mut(), &audio_props, &mut spec, &mut duration_frames)?;

    // Go back to start of the SDL_IOStream, since we're either precaching, predecoding, or maybe just getting ready to actually play the thing.
    if let Some(io) = &mut io {
        // note this seeks to offset 0, because we're using an IoClamp.
        io.seek(0, IoWhence::Set)?;
    }

    // set this before predecoding might change `decoder` to the RAW implementation.
    let _ = audio_props.set(PROP_AUDIO_DECODER_STRING, decoder.name);

    let mut precache: Option<Precache> = None;

    // if this is already raw data, predecoding is just going to make a copy of it, so skip it.
    if predecode
        && !std::ptr::eq(decoder, &crate::decoder_raw::DECODER)
        && (duration_frames != DURATION_INFINITE)
    {
        let decoded = decode_whole_file(&audio_userdata, &spec, &audio_props, io.as_mut())?;
        let len = decoded.len();
        precache = Some(Arc::new(decoded));

        decoder = &crate::decoder_raw::DECODER;
        audio_userdata = crate::decoder_raw::raw_audio_data(); // no audio_userdata state in the RAW decoder (so we can cheat here and not do a full init_audio().)
        duration_frames = (len / spec.frame_size().max(1)) as i64;
        clamp_offset = -1; // we're raw data now, any existing clamp is just nonsense now.
        clamp_length = -1;
    } else if !ondemand {
        // precache the audio data, so all decoding happens from a single buffer in RAM shared between tracks.
        let Some(io) = &mut io else {
            return Err(Error::invalid_param("src"));
        };
        precache = Some(Arc::new(io.load_all()?));
        clamp_offset = -1; // precache is already clamped
        clamp_length = -1;
    }

    drop(io); // IoClamp's close doesn't close the original stream, but we still need to free its resources here.

    if duration_frames >= 0 {
        let _ = audio_props.set(PROP_METADATA_DURATION_FRAMES_NUMBER, duration_frames);
    } else if duration_frames == DURATION_INFINITE {
        let _ = audio_props.set(PROP_METADATA_DURATION_INFINITE_BOOLEAN, true);
    }

    if nocopy.is_some() {
        precache = nocopy;
    }

    Ok(Audio(Arc::new(AudioInner {
        props: audio_props,
        spec,
        decoder,
        decoder_userdata: audio_userdata,
        precache,
        duration_frames,
        clamp_offset,
        clamp_length,
    })))
}

/// Audio data, loaded once and played by any number of tracks.
/// Translation of `MIX_Audio`.
///
/// It is reference counted: clones share it, and the last one dropped
/// frees it (`MIX_DestroyAudio()`).
#[derive(Clone)]
pub struct Audio(Arc<AudioInner>);

impl fmt::Debug for Audio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Audio")
            .field("decoder", &self.0.decoder.name)
            .field("spec", &self.0.spec)
            .field("duration_frames", &self.0.duration_frames)
            .finish_non_exhaustive()
    }
}

impl Audio {
    /// Translation of `CheckAudioParam()`.
    fn check(&self) -> Result<()> {
        check_initialized()
    }

    /// Load audio, with properties. Translation of `MIX_LoadAudioWithProperties()`.
    ///
    /// `io` is the data (it's read here, from where it is; `None` for audio
    /// a decoder makes up, such as `"SINEWAVE"`); `mixer` suggests a format
    /// to decoders that can produce any. The properties understood are
    /// [`PROP_AUDIO_LOAD_PREDECODE_BOOLEAN`](crate::PROP_AUDIO_LOAD_PREDECODE_BOOLEAN),
    /// [`PROP_AUDIO_LOAD_SKIP_METADATA_TAGS_BOOLEAN`](crate::PROP_AUDIO_LOAD_SKIP_METADATA_TAGS_BOOLEAN),
    /// [`PROP_AUDIO_LOAD_IGNORE_LOOPS_BOOLEAN`](crate::PROP_AUDIO_LOAD_IGNORE_LOOPS_BOOLEAN)
    /// and [`PROP_AUDIO_DECODER_STRING`](crate::PROP_AUDIO_DECODER_STRING);
    /// they are copied into the audio's properties.
    pub fn load_with_properties(
        io: Option<&mut IoStream<'_>>,
        mixer: Option<&Mixer>,
        props: &Properties,
    ) -> Result<Audio> {
        load_audio_with_properties(io, mixer.map(|m| &m.inner), Some(props), None)
    }

    /// Load audio from a stream, from where it is: its data is read into
    /// memory (and, with `predecode`, decoded up front). Translation of
    /// `MIX_LoadAudio_IO()`.
    pub fn load_io(mixer: Option<&Mixer>, io: &mut IoStream<'_>, predecode: bool) -> Result<Audio> {
        let props = Properties::new();
        let _ = props.set(PROP_AUDIO_LOAD_PREDECODE_BOOLEAN, predecode);
        load_audio_with_properties(Some(io), mixer.map(|m| &m.inner), Some(&props), None)
    }

    /// Load audio from a file. Translation of `MIX_LoadAudio()`.
    pub fn load(
        mixer: Option<&Mixer>,
        path: impl AsRef<std::path::Path>,
        predecode: bool,
    ) -> Result<Audio> {
        let path = path.as_ref();
        let mut io = IoStream::from_file(path, "rb")?;
        let props = Properties::new();
        let _ = props.set(
            PROP_AUDIO_LOAD_PATH_STRING,
            path.to_string_lossy().into_owned(),
        );
        let _ = props.set(PROP_AUDIO_LOAD_PREDECODE_BOOLEAN, predecode);
        load_audio_with_properties(Some(&mut io), mixer.map(|m| &m.inner), Some(&props), None)
    }

    /// Load audio from memory the app keeps (it's shared, not copied).
    /// Translation of `MIX_LoadAudioNoCopy()` (`free_when_done` is the
    /// shared data's `Drop`).
    pub fn load_no_copy(
        mixer: Option<&Mixer>,
        data: Arc<dyn AsRef<[u8]> + Send + Sync>,
    ) -> Result<Audio> {
        let mut io = PrecacheIo::open(data.clone());

        let props = Properties::new();
        let _ = props.set(PROP_AUDIO_LOAD_ONDEMAND_BOOLEAN, true); // so it doesn't make a copy to precache
        load_audio_with_properties(
            Some(&mut io),
            mixer.map(|m| &m.inner),
            Some(&props),
            Some(data),
        )
    }

    /// Load raw PCM data in `spec`'s format from a stream. Translation of
    /// `MIX_LoadRawAudio_IO()`.
    pub fn load_raw_io(
        mixer: Option<&Mixer>,
        io: &mut IoStream<'_>,
        spec: &AudioSpec,
    ) -> Result<Audio> {
        check_initialized()?;

        let props = raw_props(spec);
        load_audio_with_properties(Some(io), mixer.map(|m| &m.inner), Some(&props), None)
    }

    /// Load raw PCM data in `spec`'s format (it's copied). Translation of
    /// `MIX_LoadRawAudio()`.
    pub fn load_raw(mixer: Option<&Mixer>, data: &[u8], spec: &AudioSpec) -> Result<Audio> {
        Audio::load_raw_io(mixer, &mut IoStream::from_const_mem(data), spec)
    }

    /// Load raw PCM data in `spec`'s format from memory the app keeps (it's
    /// shared, not copied). Translation of `MIX_LoadRawAudioNoCopy()`.
    pub fn load_raw_no_copy(
        mixer: Option<&Mixer>,
        data: Arc<dyn AsRef<[u8]> + Send + Sync>,
        spec: &AudioSpec,
    ) -> Result<Audio> {
        check_initialized()?;

        let mut io = PrecacheIo::open(data.clone());

        let props = raw_props(spec);
        let _ = props.set(PROP_AUDIO_LOAD_ONDEMAND_BOOLEAN, true); // so it doesn't make a copy to precache
        load_audio_with_properties(
            Some(&mut io),
            mixer.map(|m| &m.inner),
            Some(&props),
            Some(data),
        )
    }

    /// A sine wave of `hz` at `amplitude` (0.0 to 1.0), for `ms`
    /// milliseconds (negative: forever). Translation of `MIX_CreateSineWaveAudio()`.
    pub fn sine_wave(mixer: Option<&Mixer>, hz: i32, amplitude: f32, ms: i64) -> Result<Audio> {
        check_initialized()?;
        if hz <= 0 {
            return Err(Error::invalid_param("hz"));
        } else if !(0.0..=1.0).contains(&amplitude) {
            return Err(Error::invalid_param("amplitude"));
        }

        let props = Properties::new();
        let _ = props.set(PROP_AUDIO_DECODER_STRING, "SINEWAVE");
        let _ = props.set(PROP_DECODER_SINEWAVE_HZ_NUMBER, hz as i64);
        let _ = props.set(PROP_DECODER_SINEWAVE_AMPLITUDE_FLOAT, amplitude);
        let _ = props.set(PROP_DECODER_SINEWAVE_MS_NUMBER, ms);
        let _ = props.set(PROP_AUDIO_LOAD_ONDEMAND_BOOLEAN, true);
        load_audio_with_properties(None, mixer.map(|m| &m.inner), Some(&props), None)
    }

    /// The audio's properties: its metadata (`PROP_METADATA_*`) and the
    /// properties it was loaded with. Translation of `MIX_GetAudioProperties()`.
    pub fn properties(&self) -> Result<Properties> {
        self.check()?;
        Ok(self.0.props.clone())
    }

    /// The format the audio decodes to. Translation of `MIX_GetAudioFormat()`.
    pub fn format(&self) -> Result<AudioSpec> {
        self.check()?;
        Ok(self.0.spec)
    }

    /// The audio's length in sample frames, or [`DURATION_UNKNOWN`](crate::DURATION_UNKNOWN)
    /// or [`DURATION_INFINITE`](crate::DURATION_INFINITE). Translation of
    /// `MIX_GetAudioDuration()`.
    pub fn duration(&self) -> Result<i64> {
        self.check()?;
        Ok(self.0.duration_frames)
    }

    /// Milliseconds to sample frames at the audio's sample rate.
    /// Translation of `MIX_AudioMSToFrames()`.
    pub fn ms_to_frames(&self, ms: i64) -> Result<i64> {
        self.check()?;
        ms_to_frames(self.0.spec.freq, ms)
    }

    /// Sample frames to milliseconds at the audio's sample rate.
    /// Translation of `MIX_AudioFramesToMS()`.
    pub fn frames_to_ms(&self, frames: i64) -> Result<i64> {
        self.check()?;
        frames_to_ms(self.0.spec.freq, frames)
    }
}

fn raw_props(spec: &AudioSpec) -> Properties {
    let props = Properties::new();
    let _ = props.set(PROP_AUDIO_DECODER_STRING, "RAW");
    let _ = props.set(PROP_DECODER_FORMAT_NUMBER, spec.format.0 as i64);
    let _ = props.set(PROP_DECODER_CHANNELS_NUMBER, spec.channels as i64);
    let _ = props.set(PROP_DECODER_FREQ_NUMBER, spec.freq as i64);
    let _ = props.set(PROP_AUDIO_LOAD_SKIP_METADATA_TAGS_BOOLEAN, true);
    props
}

/// Translation of `MIX_CreateTrack()`.
fn create_track(mixer: &Arc<MixerInner>) -> Result<Arc<TrackInner>> {
    mixer.check()?;

    let spec = mixer.spec();
    let output_stream = AudioStream::new(Some(&spec), Some(&spec))?;

    // we want this stream to survive SDL_Quit(), since it's not attached to an audio device.
    let _ = output_stream
        .properties()
        .set(PROP_AUDIOSTREAM_AUTO_CLEANUP_BOOLEAN, false);

    let track = Arc::new(TrackInner {
        mixer: Arc::downgrade(mixer),
        output_stream,
        props: Properties::new(),
        tags: Mutex::new(BTreeMap::new()),
        group: Mutex::new(None),
        state: ReentrantMutex::new(RefCell::new(TrackState {
            position3d: [0.0; 4],
            spatialization_mode: SpatializationMode::None,
            spatialization_panning: [0.0; 2],
            spatialization_speakers: [0; 2],
            input_buffer: Vec::new(),
            input_audio: None,
            decoder: None,
            halt_when_exhausted: true,
            input_stream: None,
            internal_stream: None,
            output_spec: AudioSpec::default(),
            state: TrackStateKind::Stopped,
            position: 0,
            silence_frames: 0,
            max_frame: 0,
            fire_and_forget: false,
            total_fade_frames: 0,
            fade_frames: 0,
            fade_direction: 0,
            fade_start_gain: 0.0,
            loops_remaining: 0,
            loop_start: 0,
            raw_callback: None,
            cooked_callback: None,
            stopped_callback: None,
        })),
        currently_inuse: AtomicBool::new(false),
        destroy_requested: AtomicBool::new(false),
        destroyed: AtomicBool::new(false),
    });

    let weak = Arc::downgrade(&track);
    track.output_stream.set_get_callback(Some(
        move |stream: &AudioStream, additional_amount: i32, _total_amount: i32| {
            if let Some(track) = weak.upgrade() {
                track_get_callback(&track, stream, additional_amount);
            }
        },
    ));

    mixer
        .lock()
        .borrow_mut()
        .all_tracks
        .insert(0, track.clone());

    set_track_group(&track, None)?; // this sets up state and updates linked lists. Should not fail!

    Ok(track)
}

// Take `track` out of `mixer`'s list of tracks tagged with `tag`.
/// Translation of `RemoveTrackFromMixerTagList()`.
fn remove_track_from_mixer_tag_list(mixer: &MixerInner, track: &Arc<TrackInner>, tag: &str) {
    let list = lock(&mixer.track_tags).get(tag).cloned();
    if let Some(list) = list {
        let mut tracks = list.tracks.write().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = tracks.iter().position(|t| Arc::ptr_eq(t, track)) {
            tracks.remove(i);
        }
    }
}

// this is an enumerator; call it multiple times to _actually_ remove from all tag lists.
/// Translation of `RemoveTrackFromAllMixerTagLists()`.
fn remove_track_from_all_mixer_tag_lists(
    mixer: &MixerInner,
    track: &Arc<TrackInner>,
    tags: &BTreeMap<String, bool>,
) {
    // this only removes the track from the mixer's tag lists, as we're
    //  enumerating track->tags and don't want the hash to change during that.
    // After this is done, we'll destroy the hash outright anyhow.
    for (tag, &on) in tags {
        if on {
            // these still exist in track->tags once untagged, so only bother if set to true.
            remove_track_from_mixer_tag_list(mixer, track, tag);
        }
    }
}

/// Translation of `MIX_DestroyTrack()`.
fn destroy_track(track: &Arc<TrackInner>) {
    if track.destroyed.load(Ordering::Acquire) {
        return;
    }

    let mixer = track.mixer.upgrade();

    if let Some(mixer) = &mixer {
        let guard = mixer.lock();

        // handle the case where someone destroys a track during a mixer callback.  :O
        //  tracks are not currently reference-counted like MIX_Audio objects are, but
        //  we'll catch this specific case for now.
        if track.currently_inuse.load(Ordering::Acquire) {
            track.destroy_requested.store(true, Ordering::Release);
            return;
        }

        let mut st = guard.borrow_mut();
        st.all_tracks.retain(|t| !Arc::ptr_eq(t, track));

        // we don't check the fire-and-forget pool because that is only free'd, with mixer->all_tracks, when closing the mixer.
        // !!! FIXME: maybe we _shouldn't_ keep the fire-and-forget pool in all_tracks, so we can skip processing them everywhere, and just explicitly free the pool in MIX_DestroyMixer.

        if let Some(group) = lock(&track.group).take() {
            lock(&group.tracks).retain(|t| !Arc::ptr_eq(t, track));
        }
    }

    track.destroyed.store(true, Ordering::Release);

    // SDL_DestroyAudioStream(track->output_stream)
    track
        .output_stream
        .set_get_callback(None::<fn(&AudioStream, i32, i32)>);

    {
        let guard = track.lock();
        let mut st = guard.borrow_mut();
        st.decoder = None; // quit_track (and closes the track's IOStream).
        st.internal_stream = None;
        st.input_stream = None;
        st.input_audio = None; // UnrefAudio
        st.raw_callback = None;
        st.cooked_callback = None;
        st.stopped_callback = None;
        st.input_buffer = Vec::new();
    }

    let tags = std::mem::take(&mut *lock(&track.tags));
    if let Some(mixer) = &mixer {
        remove_track_from_all_mixer_tag_lists(mixer, track, &tags);
    }
}

/// Translation of `MIX_SetTrackAudio_internal()`.
fn set_track_audio_internal(
    track: &Arc<TrackInner>,
    audio: Option<&Audio>,
    io: Option<IoStream<'static>>,
) -> Result<()> {
    debug_assert!(track.check().is_ok());
    debug_assert!(audio.is_some() || io.is_none()); // if audio==NULL, io must be NULL, too.

    let mut spec = match audio {
        Some(audio) => audio.0.spec,
        // make this reasonable, but in theory we shouldn't touch it again.
        None => AudioSpec::new(AudioFormat::UNKNOWN, 2, 44100),
    };
    spec.format = AudioFormat::F32; // we always work in float32.

    let mixer_spec = track.mixer()?.spec();

    let guard = track.lock();

    if let Some(audio) = audio {
        if guard.borrow().internal_stream.is_none() {
            let internal_stream = AudioStream::new(Some(&audio.0.spec), Some(&spec))?;

            // we want this stream to survive SDL_Quit(), since it's not attached to an audio device.
            let _ = internal_stream
                .properties()
                .set(PROP_AUDIOSTREAM_AUTO_CLEANUP_BOOLEAN, false);
            guard.borrow_mut().internal_stream = Some(Arc::new(internal_stream));
        }
    }

    {
        let mut st = guard.borrow_mut();
        if st.input_audio.is_some() {
            st.decoder = None; // quit_track (which closes the old IOStream, and its clamp).
            st.input_audio = None; // UnrefAudio
        }

        st.input_audio = None;
        st.input_stream = None;
    }

    if let Some(audio) = audio {
        let mut io = io;
        if audio.0.clamp_offset >= 0 {
            // clamp i/o so decoders don't see ID3 tags, etc.
            let Some(inner_io) = io.take() else {
                return Err(Error::invalid_param("context"));
            };
            let mut clamp = IoClamp::open(inner_io)?;
            clamp.start = audio.0.clamp_offset;
            clamp.length = audio.0.clamp_length;
            io = Some(IoStream::open(clamp));
        }

        let decoder =
            audio
                .0
                .decoder_userdata
                .clone()
                .init_track(io, &audio.0.spec, &audio.0.props)?;

        let mut st = guard.borrow_mut();
        let internal_stream = st.internal_stream.clone();
        if let Some(internal_stream) = &internal_stream {
            let _ = internal_stream.set_format(Some(&audio.0.spec), Some(&spec));
            // input is from decoded audio, output is to output_stream
        }
        set_track_output_stream_format(track, &mut st, &mixer_spec, Some(&spec)); // input is from internal_stream, output is to mixer->output_stream (or, if spatializing, to mixer->output_stream but mono).
        st.input_audio = Some(audio.clone()); // RefAudio
        st.decoder = Some(decoder);
        st.input_stream = internal_stream;
        if let Some(s) = &st.input_stream {
            s.clear(); // make sure that any extra buffered input from before is removed.
        }
        st.position = 0;
    }

    Ok(())
}

/// Translation of `MIX_SetTrackAudio()`.
fn set_track_audio(track: &Arc<TrackInner>, audio: Option<&Audio>) -> Result<()> {
    track.check()?;

    let mut io = None;
    if let Some(audio) = audio {
        // external MIX_Audios shouldn't be able to get into a state where they aren't precached (except SINEWAVE,
        // which is generated on the fly). Make this assert more generic if we add another thing like SINEWAVE later!
        debug_assert!(
            audio.0.precache.is_some()
                || std::ptr::eq(audio.0.decoder, &crate::decoder_sinewave::DECODER)
        );

        if let Some(precache) = &audio.0.precache {
            io = Some(PrecacheIo::open(precache.clone()));
        }
    }

    set_track_audio_internal(track, audio, io)
}

/// Translation of `GetTrackOptionFramesOrTicks()`.
fn get_track_option_frames_or_ticks(
    track: &Arc<TrackInner>,
    options: &Properties,
    framesprop: &str,
    msprop: &str,
    defval: i64,
) -> i64 {
    if options.contains(framesprop) {
        return options.get_number(framesprop).unwrap_or(defval);
    } else if options.contains(msprop) {
        let val = options.get_number(msprop).unwrap_or(defval);
        let val_frames = track_ms_to_frames(track, val).max(0);
        return if val < 0 { val } else { val_frames };
    }
    defval
}

/// Translation of `MIX_TrackMSToFrames()` (-1 when the track has no input).
fn track_ms_to_frames(track: &Arc<TrackInner>, ms: i64) -> i64 {
    let freq = track_input_freq(track);
    if freq != 0 {
        return ms_to_frames(freq, ms).unwrap_or(-1);
    }
    -1
}

/// The sample rate of a track's input (0 without one).
fn track_input_freq(track: &Arc<TrackInner>) -> i32 {
    let input_stream = {
        let guard = track.lock();
        let st = guard.borrow();
        st.input_stream.clone()
    };
    input_stream
        .and_then(|s| s.src_format().ok())
        .map_or(0, |spec| spec.freq)
}

/// Translation of `MIX_PlayTrack()`.
fn play_track(track: &Arc<TrackInner>, options: Option<&Properties>) -> Result<()> {
    track.check()?;

    let guard = track.lock();
    {
        let st = guard.borrow();
        if st.input_audio.is_none() && st.input_stream.is_none() {
            return Err(Error::new("No audio currently assigned to this track"));
        }
    }

    let mut loops: i32 = 0;
    let mut max_frame: i64 = -1;
    let mut start_pos: i64 = 0;
    let mut loop_start: i64 = 0;
    let mut fade_in: i64 = 0;
    let mut append_silence_frames: i64 = 0;
    let mut start_order: i32 = -1;
    let mut fade_start_gain: f32 = 0.0;
    let mut halt_when_exhausted = true;

    if let Some(options) = options {
        loops = options
            .get_number(PROP_PLAY_LOOPS_NUMBER)
            .unwrap_or(loops as i64) as i32;
        max_frame = get_track_option_frames_or_ticks(
            track,
            options,
            PROP_PLAY_MAX_FRAME_NUMBER,
            PROP_PLAY_MAX_MILLISECONDS_NUMBER,
            max_frame,
        );
        start_pos = get_track_option_frames_or_ticks(
            track,
            options,
            PROP_PLAY_START_FRAME_NUMBER,
            PROP_PLAY_START_MILLISECOND_NUMBER,
            start_pos,
        );
        loop_start = get_track_option_frames_or_ticks(
            track,
            options,
            PROP_PLAY_LOOP_START_FRAME_NUMBER,
            PROP_PLAY_LOOP_START_MILLISECOND_NUMBER,
            loop_start,
        );
        fade_in = get_track_option_frames_or_ticks(
            track,
            options,
            PROP_PLAY_FADE_IN_FRAMES_NUMBER,
            PROP_PLAY_FADE_IN_MILLISECONDS_NUMBER,
            fade_in,
        );
        fade_start_gain = options
            .get_float(PROP_PLAY_FADE_IN_START_GAIN_FLOAT)
            .unwrap_or(fade_start_gain);
        append_silence_frames = get_track_option_frames_or_ticks(
            track,
            options,
            PROP_PLAY_APPEND_SILENCE_FRAMES_NUMBER,
            PROP_PLAY_APPEND_SILENCE_MILLISECONDS_NUMBER,
            append_silence_frames,
        );
        halt_when_exhausted = options
            .get_bool(PROP_PLAY_HALT_WHEN_EXHAUSTED_BOOLEAN)
            .unwrap_or(halt_when_exhausted);
        start_order = options
            .get_number(PROP_PLAY_START_ORDER_NUMBER)
            .unwrap_or(start_order as i64) as i32;

        if start_pos < 0 {
            start_pos = 0;
        }

        if loop_start < 0 {
            loop_start = 0;
        }

        if append_silence_frames < 0 {
            append_silence_frames = 0;
        }

        fade_start_gain = fade_start_gain.clamp(0.0, 1.0);
    }

    let mut st = guard.borrow_mut();

    if (start_order >= 0)
        && st
            .input_audio
            .as_ref()
            .is_none_or(|a| !a.0.decoder.has_jump_to_order)
    {
        start_order = -1; // ignore this option, it doesn't mean anything on this decoder.
    }

    if start_order >= 0 {
        if let Some(decoder) = st.decoder.as_mut() {
            decoder.jump_to_order(start_order)?;
        }
    } else if st.input_audio.is_some() {
        match st.decoder.as_mut() {
            Some(decoder) => decoder.seek(start_pos as u64)?,
            None => return Err(Error::new("No audio currently assigned to this track")),
        }
    } else if start_pos != 0 {
        return Err(Error::new(
            "Playing an input stream (not MIX_Audio) with a non-zero start position",
        )); // !!! FIXME: should we just read off this many frames right now instead?
    }

    st.max_frame = max_frame;
    st.loops_remaining = loops;
    st.loop_start = loop_start as i32;
    st.total_fade_frames = if fade_in > 0 { fade_in } else { 0 };
    st.fade_frames = st.total_fade_frames;
    st.fade_direction = if fade_in > 0 { 1 } else { 0 };
    st.fade_start_gain = fade_start_gain;
    st.silence_frames = if append_silence_frames > 0 {
        -append_silence_frames
    } else {
        0
    }; // negative means "there is still actual audio data to play", positive means "we're done with actual data, feed silence now." Zero means no silence (left) to feed.
    st.state = TrackStateKind::Playing;
    st.position = start_pos as u64;
    st.halt_when_exhausted = halt_when_exhausted;

    Ok(())
}

/// Translation of `StopTrack()`.
fn stop_track(track: &Arc<TrackInner>, fade_out_frames: i64) {
    {
        let guard = track.lock();
        let stopped = guard.borrow().state == TrackStateKind::Stopped;
        if !stopped {
            if fade_out_frames <= 0 {
                // stop immediately.
                let internal_stream = guard.borrow().internal_stream.clone();
                if let Some(s) = internal_stream {
                    s.clear(); // make sure we don't leave old data hanging around.
                }
                track.currently_inuse.store(true, Ordering::Release);
                track_stopped(track);
                track.currently_inuse.store(false, Ordering::Release);
            } else {
                let mut st = guard.borrow_mut();
                st.total_fade_frames = fade_out_frames;
                st.fade_frames = fade_out_frames;
                st.fade_direction = -1;
                st.fade_start_gain = 0.0; // only used for fade-ins.
            }
        }
    }

    if track.destroy_requested.load(Ordering::Acquire) {
        // callback asked to destroy the track while we were still touching it.
        destroy_track(track); // actually kill it now.
    }
}

/// Translation of `MIX_StopAllTracks()`.
fn stop_all_tracks(mixer: &Arc<MixerInner>, fade_out_ms: i64) -> Result<()> {
    check_initialized()?;

    let guard = mixer.lock(); // lock the mixer so all tracks stop at the same time.
    let tracks = guard.borrow().all_tracks.clone();

    for track in &tracks {
        let fade_out_frames = track_ms_to_frames(track, fade_out_ms).max(0);
        stop_track(track, if fade_out_ms > 0 { fade_out_frames } else { -1 });
    }

    Ok(())
}

/// Translation of `PauseTrack()`.
fn pause_track(track: &Arc<TrackInner>) {
    let guard = track.lock();
    let mut st = guard.borrow_mut();
    if st.state == TrackStateKind::Playing {
        st.state = TrackStateKind::Paused;
    }
}

/// Translation of `ResumeTrack()`.
fn resume_track(track: &Arc<TrackInner>) {
    let guard = track.lock();
    let mut st = guard.borrow_mut();
    if st.state == TrackStateKind::Paused {
        st.state = TrackStateKind::Playing;
    }
}

/// Translation of `SetTrackGain()`.
fn set_track_gain(track: &Arc<TrackInner>, gain: f32) -> Result<()> {
    // don't have to LockTrack, as SDL_SetAudioStreamGain will do that.
    track.output_stream.set_gain(gain)
}

/// Translation of `MIX_CreateGroup()`.
fn create_group(mixer: &Arc<MixerInner>) -> Result<Arc<GroupInner>> {
    check_initialized()?;

    let group = Arc::new(GroupInner {
        mixer: Arc::downgrade(mixer),
        tracks: Mutex::new(Vec::new()),
        props: Properties::new(),
        postmix_callback: Mutex::new(None),
        destroyed: AtomicBool::new(false),
    });

    mixer
        .lock()
        .borrow_mut()
        .all_groups
        .insert(0, group.clone());

    Ok(group)
}

/// Translation of `MIX_DestroyGroup()`.
fn destroy_group(group: &Arc<GroupInner>) {
    if group.destroyed.swap(true, Ordering::AcqRel) {
        return;
    }

    let Some(mixer) = group.mixer.upgrade() else {
        return;
    };

    let guard = mixer.lock();
    guard
        .borrow_mut()
        .all_groups
        .retain(|g| !Arc::ptr_eq(g, group));

    let tracks = lock(&group.tracks).clone();
    for track in &tracks {
        // track->group_next will change in SetTrackGroup, so save it off.
        let is_default = guard
            .borrow()
            .default_group
            .as_ref()
            .is_some_and(|d| Arc::ptr_eq(d, group));
        if is_default {
            // (the default group goes with its mixer: the tracks go too.)
            *lock(&track.group) = None;
        } else {
            let _ = set_track_group(track, None);
        }
    }
    lock(&group.tracks).clear();
    *lock(&group.postmix_callback) = None;
}

/// Translation of `MIX_SetTrackGroup()`.
fn set_track_group(track: &Arc<TrackInner>, group: Option<&Arc<GroupInner>>) -> Result<()> {
    track.check()?;

    let mixer = track.mixer()?;
    let group = match group {
        None => mixer
            .lock()
            .borrow()
            .default_group
            .clone()
            .ok_or_else(|| Error::invalid_param("group"))?,
        Some(group) => {
            if !Weak::ptr_eq(&track.mixer, &group.mixer) {
                return Err(Error::new(
                    "Track and group are not from the same MIX_Mixer.",
                ));
            }
            group.clone()
        }
    };

    let _mguard = mixer.lock();
    let _tguard = track.lock();
    let mut current = lock(&track.group);
    let oldgroup = current.clone();
    if oldgroup
        .as_ref()
        .is_none_or(|old| !Arc::ptr_eq(old, &group))
    {
        if let Some(oldgroup) = oldgroup {
            // remove from current group, if in one.
            lock(&oldgroup.tracks).retain(|t| !Arc::ptr_eq(t, track));
        }

        lock(&group.tracks).insert(0, track.clone());
        *current = Some(group);
    }

    Ok(())
}

/// A track: one sound playing on a [`Mixer`]. Translation of `MIX_Track`.
///
/// Dropping the handle from [`new`](Self::new) destroys the track
/// (`MIX_DestroyTrack()`); the handles callbacks receive and the ones from
/// [`Mixer::tagged_tracks`] don't.
pub struct Track {
    inner: Arc<TrackInner>,
    owner: bool,
}

impl fmt::Debug for Track {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Track")
            .field("destroyed", &self.inner.destroyed.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Drop for Track {
    fn drop(&mut self) {
        if self.owner {
            destroy_track(&self.inner);
        }
    }
}

impl Track {
    fn borrowed(inner: &Arc<TrackInner>) -> Track {
        Track {
            inner: inner.clone(),
            owner: false,
        }
    }

    /// Create a track on a mixer. Translation of `MIX_CreateTrack()`.
    pub fn new(mixer: &Mixer) -> Result<Track> {
        Ok(Track {
            inner: create_track(&mixer.inner)?,
            owner: true,
        })
    }

    /// The mixer this track plays on. Translation of `MIX_GetTrackMixer()`.
    pub fn mixer(&self) -> Result<Mixer> {
        self.inner.check()?;
        Ok(Mixer::borrowed(&self.inner.mixer()?))
    }

    /// The track's properties. Translation of `MIX_GetTrackProperties()`.
    pub fn properties(&self) -> Result<Properties> {
        self.inner.check()?;
        Ok(self.inner.props.clone())
    }

    /// Play an [`Audio`] on this track (or nothing, with `None`); the track
    /// is stopped until [`play`](Self::play). Translation of `MIX_SetTrackAudio()`.
    pub fn set_audio(&self, audio: Option<&Audio>) -> Result<()> {
        set_track_audio(&self.inner, audio)
    }

    /// Play what the app puts in an audio stream on this track (or nothing,
    /// with `None`). The stream's output becomes float samples. Translation
    /// of `MIX_SetTrackAudioStream()`.
    pub fn set_audio_stream(&self, stream: Option<Arc<AudioStream>>) -> Result<()> {
        self.inner.check()?;
        let mixer_spec = self.inner.mixer()?.spec();

        let guard = self.inner.lock();
        let mut st = guard.borrow_mut();

        if st.input_audio.is_some() {
            st.decoder = None; // quit_track (which closes the old IOStream).
            st.input_audio = None; // UnrefAudio
            if let Some(s) = &st.internal_stream {
                s.clear(); // make sure that any extra buffered input is removed.
            }
        }

        if let Some(stream) = &stream {
            let mut spec = stream.src_format().unwrap_or_default();
            spec.format = AudioFormat::F32; // we always work in float32.
            let _ = stream.set_format(None, Some(&spec)); // input is whatever, output is whatever in float format.
            set_track_output_stream_format(&self.inner, &mut st, &mixer_spec, Some(&spec));
            // input is whatever in float format, output is to mixer->output_stream (or, if spatializing, to mixer->output_stream but mono).
        }

        st.input_stream = stream;
        st.position = 0;

        Ok(())
    }

    /// Decode and play a stream on this track, as it plays (or nothing,
    /// with `None`); the track owns the stream from now on. Translation of
    /// `MIX_SetTrackIOStream()`.
    pub fn set_io_stream(&self, io: Option<IoStream<'static>>) -> Result<()> {
        self.inner.check()?;
        let Some(mut io) = io else {
            return set_track_audio(&self.inner, None); // just drop the current input.
        };

        let props = Properties::new();
        let _ = props.set(PROP_AUDIO_LOAD_ONDEMAND_BOOLEAN, true);
        let mixer = self.inner.mixer()?;
        let audio = load_audio_with_properties(Some(&mut io), Some(&mixer), Some(&props), None)?;

        // Drop our reference to `audio` after the track accepts it, so when the track is
        //  done with it, it'll unref it, and `audio` will be cleaned up. If the track failed
        //  to accept the audio, this will clean it up right now.
        set_track_audio_internal(&self.inner, Some(&audio), Some(io))
    }

    /// Play raw PCM data in `spec`'s format from a stream on this track
    /// (or nothing, with `None`); the track owns the stream from now on.
    /// Translation of `MIX_SetTrackRawIOStream()`.
    pub fn set_raw_io_stream(&self, io: Option<IoStream<'static>>, spec: &AudioSpec) -> Result<()> {
        self.inner.check()?;
        let Some(mut io) = io else {
            return set_track_audio(&self.inner, None); // just drop the current input.
        };

        let props = raw_props(spec);
        let _ = props.set(PROP_AUDIO_LOAD_ONDEMAND_BOOLEAN, true);
        let mixer = self.inner.mixer()?;
        let audio = load_audio_with_properties(Some(&mut io), Some(&mixer), Some(&props), None)?;

        // Drop our reference to `audio` after the track accepts it, so when the track is
        //  done with it, it'll unref it, and `audio` will be cleaned up. If the track failed
        //  to accept the audio, this will clean it up right now.
        set_track_audio_internal(&self.inner, Some(&audio), Some(io))
    }

    /// Tag the track (tags group tracks for [`Mixer::play_tag`] and
    /// friends). Translation of `MIX_TagTrack()`.
    pub fn tag(&self, tag: &str) -> Result<()> {
        self.inner.check()?;
        check_tag(tag)?;
        let mixer = self.inner.mixer()?;

        let mut tags = lock(&self.inner.tags);
        if !tags.get(tag).copied().unwrap_or(false) {
            tags.insert(tag.to_owned(), true);

            // CreateTagList()
            let list = lock(&mixer.track_tags)
                .entry(tag.to_owned())
                .or_insert_with(|| {
                    Arc::new(TagList {
                        tracks: RwLock::new(Vec::with_capacity(4)),
                    })
                })
                .clone();

            list.tracks
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .push(self.inner.clone());
        }

        Ok(())
    }

    /// Remove a tag from the track (`None`: every tag). Translation of
    /// `MIX_UntagTrack()`.
    pub fn untag(&self, tag: Option<&str>) {
        if self.inner.check().is_err() {
            return; // do nothing.
        }
        let Ok(mixer) = self.inner.mixer() else {
            return;
        };

        match tag {
            None => {
                // untag everything on the track.
                let tags = std::mem::take(&mut *lock(&self.inner.tags)); // just nuke all the tags and start over.
                remove_track_from_all_mixer_tag_lists(&mixer, &self.inner, &tags);
            }
            Some(tag) => {
                let mut tags = lock(&self.inner.tags);
                if tags.get(tag).copied().unwrap_or(false) {
                    // if tag isn't there, nothing to do.
                    remove_track_from_mixer_tag_list(&mixer, &self.inner, tag);
                    tags.insert(tag.to_owned(), false);
                }
            }
        }
    }

    /// The track's tags. Translation of `MIX_GetTrackTags()`.
    pub fn tags(&self) -> Result<Vec<String>> {
        self.inner.check()?;
        let tags = lock(&self.inner.tags);
        // if false, tag _was_ here, but has since been untagged. Skip it.
        Ok(tags
            .iter()
            .filter(|(_, &on)| on)
            .map(|(t, _)| t.clone())
            .collect())
    }

    /// Seek to a sample frame (of the track's [`Audio`]). Translation of
    /// `MIX_SetTrackPlaybackPosition()`.
    pub fn set_playback_position(&self, frames: i64) -> Result<()> {
        self.inner.check()?;
        if frames < 0 {
            return Err(Error::invalid_param("frames"));
        }

        // !!! FIXME: should it be legal to seek past the end of an track (so it just stops immediately, or maybe stops on next callback)?
        let guard = self.inner.lock();
        let mut st = guard.borrow_mut();
        if st.input_audio.is_none() {
            if st.input_stream.is_some() {
                // can't seek a stream that was set up with MIX_SetTrackAudioStream.
                return Err(Error::new("Can't seek a streaming track"));
            } else {
                return Err(Error::new("No audio currently assigned to this track"));
            }
        }
        if let Some(decoder) = st.decoder.as_mut() {
            decoder.seek(frames as u64)?;
        }
        if let Some(s) = &st.input_stream {
            s.clear(); // make sure that any extra buffered input from before the seek is removed.
        }
        st.position = frames as u64;
        Ok(())
    }

    /// The sample frame the track is at. Translation of
    /// `MIX_GetTrackPlaybackPosition()`.
    pub fn playback_position(&self) -> Result<i64> {
        self.inner.check()?;
        let guard = self.inner.lock();
        let position = guard.borrow().position as i64;
        Ok(position)
    }

    /// The sample frames left in the fade: positive fading in, negative
    /// fading out, zero not fading. Translation of `MIX_GetTrackFadeFrames()`.
    pub fn fade_frames(&self) -> Result<i64> {
        self.inner.check()?;
        let guard = self.inner.lock();
        let st = guard.borrow();
        if st.state != TrackStateKind::Stopped {
            return Ok(st.fade_frames * st.fade_direction as i64);
        }
        Ok(0)
    }

    /// The loops left (-1: forever). Translation of `MIX_GetTrackLoops()`.
    pub fn loops(&self) -> Result<i32> {
        self.inner.check()?;
        let guard = self.inner.lock();
        let st = guard.borrow();
        if st.state != TrackStateKind::Stopped {
            return Ok(st.loops_remaining);
        }
        Ok(0)
    }

    /// Change the loops left (-1: forever). Translation of `MIX_SetTrackLoops()`.
    pub fn set_loops(&self, mut num_loops: i32) -> Result<()> {
        self.inner.check()?;
        if num_loops < -1 {
            num_loops = -1; // keep this value consistent if we're looping infinitely.
        }
        self.inner.lock().borrow_mut().loops_remaining = num_loops;
        Ok(())
    }

    /// The [`Audio`] the track plays, if any (not the temporary one behind
    /// [`set_io_stream`](Self::set_io_stream)). Translation of `MIX_GetTrackAudio()`.
    pub fn audio(&self) -> Result<Option<Audio>> {
        self.inner.check()?;
        let guard = self.inner.lock();
        let st = guard.borrow();
        // don't allow access to the MIX_Audio if this was a temporary one created for MIX_SetTrackIOStream.
        Ok(st
            .input_audio
            .as_ref()
            .filter(|a| a.0.precache.is_some())
            .cloned())
    }

    /// The audio stream the track plays, if one was given to
    /// [`set_audio_stream`](Self::set_audio_stream). Translation of
    /// `MIX_GetTrackAudioStream()`.
    pub fn audio_stream(&self) -> Result<Option<Arc<AudioStream>>> {
        self.inner.check()?;
        let guard = self.inner.lock();
        let st = guard.borrow();
        let is_internal = match (&st.input_stream, &st.internal_stream) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, _) => true,
            _ => false,
        };
        Ok(if is_internal {
            None
        } else {
            st.input_stream.clone()
        })
    }

    /// The sample frames left to play, if known (zero when stopped).
    /// Translation of `MIX_GetTrackRemaining()`.
    pub fn remaining(&self) -> Result<i64> {
        self.inner.check()?;
        let (position, duration, stopped) = {
            let guard = self.inner.lock();
            let st = guard.borrow();
            (
                st.position as i64,
                st.input_audio.as_ref().map_or(-1, |a| a.0.duration_frames),
                st.state == TrackStateKind::Stopped,
            )
        };
        if stopped {
            Ok(0)
        } else if (duration >= 0) && (position >= 0) && (position <= duration) {
            Ok(duration - position)
        } else {
            Ok(-1)
        }
    }

    /// Milliseconds to sample frames at the track's input sample rate.
    /// Translation of `MIX_TrackMSToFrames()`.
    pub fn ms_to_frames(&self, ms: i64) -> Result<i64> {
        self.inner.check()?;
        let freq = track_input_freq(&self.inner);
        if freq == 0 {
            return Err(Error::new("No audio currently assigned to this track"));
        }
        ms_to_frames(freq, ms)
    }

    /// Sample frames to milliseconds at the track's input sample rate.
    /// Translation of `MIX_TrackFramesToMS()`.
    pub fn frames_to_ms(&self, frames: i64) -> Result<i64> {
        self.inner.check()?;
        let freq = track_input_freq(&self.inner);
        if freq == 0 {
            return Err(Error::new("No audio currently assigned to this track"));
        }
        frames_to_ms(freq, frames)
    }

    /// Start (or restart) the track. Translation of `MIX_PlayTrack()`.
    ///
    /// The options are `PROP_PLAY_*` properties: loops, where to start and
    /// stop, where loops start, a fade in, silence to append, and whether
    /// to stop when the input runs out.
    pub fn play(&self, options: Option<&Properties>) -> Result<()> {
        play_track(&self.inner, options)
    }

    /// Stop the track, after fading out over `fade_out_frames` sample
    /// frames (or at once, for zero or less). Translation of `MIX_StopTrack()`.
    pub fn stop(&self, fade_out_frames: i64) -> Result<()> {
        self.inner.check()?;
        stop_track(&self.inner, fade_out_frames);
        Ok(())
    }

    /// Pause the track. Translation of `MIX_PauseTrack()`.
    pub fn pause(&self) -> Result<()> {
        self.inner.check()?;
        pause_track(&self.inner);
        Ok(())
    }

    /// Resume the paused track. Translation of `MIX_ResumeTrack()`.
    pub fn resume(&self) -> Result<()> {
        self.inner.check()?;
        resume_track(&self.inner);
        Ok(())
    }

    /// Whether the track is playing. Translation of `MIX_TrackPlaying()`.
    pub fn playing(&self) -> bool {
        if self.inner.check().is_err() {
            return false;
        }
        let guard = self.inner.lock();
        let retval = guard.borrow().state == TrackStateKind::Playing;
        retval
    }

    /// Whether the track is paused. Translation of `MIX_TrackPaused()`.
    pub fn paused(&self) -> bool {
        if self.inner.check().is_err() {
            return false;
        }
        let guard = self.inner.lock();
        let retval = guard.borrow().state == TrackStateKind::Paused;
        retval
    }

    /// Set a callback that runs when the track stops; `None` removes it.
    /// Translation of `MIX_SetTrackStoppedCallback()`.
    pub fn set_stopped_callback(
        &self,
        cb: Option<impl Fn(&Track) + Send + Sync + 'static>,
    ) -> Result<()> {
        self.inner.check()?;
        let cb = cb.map(|c| Arc::new(c) as TrackStoppedCallback);
        self.inner.lock().borrow_mut().stopped_callback = cb;
        Ok(())
    }

    /// Set the track's gain (1.0 is unchanged; negative gains are clamped
    /// to zero). Translation of `MIX_SetTrackGain()`.
    pub fn set_gain(&self, mut gain: f32) -> Result<()> {
        self.inner.check()?;

        if gain < 0.0 {
            gain = 0.0; // !!! FIXME: this clamps, but should it fail instead?
        }

        set_track_gain(&self.inner, gain)
    }

    /// The track's gain. Translation of `MIX_GetTrackGain()`.
    pub fn gain(&self) -> Result<f32> {
        self.inner.check()?;

        // don't have to LockTrack, as SDL_GetAudioStreamGain will do that.
        Ok(self.inner.output_stream.gain())
    }

    /// Change the speed (and pitch) of the track; clamped to 0.01..100.0.
    /// Translation of `MIX_SetTrackFrequencyRatio()`.
    pub fn set_frequency_ratio(&self, ratio: f32) -> Result<()> {
        self.inner.check()?;

        let ratio = ratio.clamp(0.01, 100.0); // !!! FIXME: this clamps, but should it fail instead?

        // don't have to LockTrack, as SDL_SetAudioStreamFrequencyRatio will do that.
        self.inner.output_stream.set_frequency_ratio(ratio)
    }

    /// The track's frequency ratio. Translation of `MIX_GetTrackFrequencyRatio()`.
    pub fn frequency_ratio(&self) -> Result<f32> {
        self.inner.check()?;

        // don't have to LockTrack, as SDL_GetAudioStreamFrequencyRatio will do that.
        Ok(self.inner.output_stream.frequency_ratio())
    }

    /// Map the track's output channels (`None`: the default layout).
    /// Translation of `MIX_SetTrackOutputChannelMap()`.
    pub fn set_output_channel_map(&self, chmap: Option<&[i32]>) -> Result<()> {
        self.inner.check()?;

        // don't have to LockTrack, as SDL_SetAudioStreamOutputChannelMap will do that.
        self.inner.output_stream.set_output_channel_map(chmap)
    }

    /// Force the track to stereo, with these gains (`None`: back to
    /// normal). Translation of `MIX_SetTrackStereo()`.
    pub fn set_stereo(&self, gains: Option<&StereoGains>) -> Result<()> {
        self.inner.check()?;
        let mixer_spec = self.inner.mixer()?.spec();

        let guard = self.inner.lock();
        let mut st = guard.borrow_mut();

        let wants_stereo = gains.is_some();
        let new_mode = if wants_stereo {
            SpatializationMode::Stereo
        } else {
            SpatializationMode::None
        };
        if st.spatialization_mode != new_mode {
            st.spatialization_mode = new_mode;
            set_track_output_stream_format(&self.inner, &mut st, &mixer_spec, None);
            // change output format to stereo (or back to normal) if necessary.
        }

        st.position3d[0] = 0.0;
        st.position3d[1] = 0.0;
        st.position3d[2] = 0.0;

        if let Some(gains) = gains {
            let left = sdl_max_f(0.0, gains.left);
            let right = sdl_max_f(0.0, gains.right);
            if mixer_spec.channels == 1 {
                // mono output
                st.spatialization_speakers = [0, 0];
                st.spatialization_panning = [left * 0.5, right * 0.5];
            } else {
                st.spatialization_speakers = [0, 1];
                st.spatialization_panning = [left, right];
            }
        }

        Ok(())
    }

    /// Position the track in 3D space (`None`: back to normal). Translation
    /// of `MIX_SetTrack3DPosition()`.
    pub fn set_3d_position(&self, position: Option<&Point3D>) -> Result<()> {
        self.inner.check()?;
        let format = lock(&self.inner.mixer()?.format).clone();

        let guard = self.inner.lock();
        let mut st = guard.borrow_mut();

        let wants_spatialization = position.is_some();
        let new_mode = if wants_spatialization {
            SpatializationMode::ThreeD
        } else {
            SpatializationMode::None
        };
        let toggling = st.spatialization_mode != new_mode;
        if toggling {
            st.spatialization_mode = new_mode;
            set_track_output_stream_format(&self.inner, &mut st, &format.spec, None);
            // change output format to stereo (or back to normal) if necessary.
        }

        match position {
            None => {
                st.position3d[0] = 0.0;
                st.position3d[1] = 0.0;
                st.position3d[2] = 0.0;
            }
            Some(position) => {
                let p = st.position3d;
                // FIXME (upstream): the Y test compares `tposition3d[2]`,
                // where it means `[1]`, so moving only along Y isn't noticed.
                if toggling || (p[0] != position.x) || (p[2] != position.y) || (p[2] != position.z)
                {
                    st.position3d[0] = position.x;
                    st.position3d[1] = position.y;
                    st.position3d[2] = position.z;
                    let tposition3d = st.position3d;
                    let TrackState {
                        spatialization_panning,
                        spatialization_speakers,
                        ..
                    } = &mut *st;
                    spatialize(
                        &format.vbap2d,
                        &tposition3d,
                        spatialization_panning,
                        spatialization_speakers,
                    );
                }
            }
        }

        Ok(())
    }

    /// The track's 3D position. Translation of `MIX_GetTrack3DPosition()`.
    pub fn position_3d(&self) -> Result<Point3D> {
        self.inner.check()?;

        let guard = self.inner.lock();
        let st = guard.borrow();
        Ok(Point3D {
            x: st.position3d[0],
            y: st.position3d[1],
            z: st.position3d[2],
        })
    }

    /// Set a callback that sees the track's audio as decoded, before fades,
    /// format conversion and positioning; `None` removes it. Translation
    /// of `MIX_SetTrackRawCallback()`.
    pub fn set_raw_callback(
        &self,
        cb: Option<impl Fn(&Track, &AudioSpec, &mut [f32]) + Send + Sync + 'static>,
    ) -> Result<()> {
        self.inner.check()?;
        let cb = cb.map(|c| Arc::new(c) as TrackMixCallback);
        self.inner.lock().borrow_mut().raw_callback = cb;
        Ok(())
    }

    /// Set a callback that sees the track's audio just before it is mixed;
    /// `None` removes it. Translation of `MIX_SetTrackCookedCallback()`.
    pub fn set_cooked_callback(
        &self,
        cb: Option<impl Fn(&Track, &AudioSpec, &mut [f32]) + Send + Sync + 'static>,
    ) -> Result<()> {
        self.inner.check()?;
        let cb = cb.map(|c| Arc::new(c) as TrackMixCallback);
        self.inner.lock().borrow_mut().cooked_callback = cb;
        Ok(())
    }

    /// Move the track to a group (`None`: out of any). Translation of
    /// `MIX_SetTrackGroup()`.
    pub fn set_group(&self, group: Option<&Group>) -> Result<()> {
        set_track_group(&self.inner, group.map(|g| &g.inner))
    }
}

/// A group of tracks, mixed together before the final mix (a
/// [`Group::set_postmix_callback`] sees them on their own). Translation of
/// `MIX_Group`.
///
/// Dropping the handle from [`new`](Self::new) destroys the group
/// (`MIX_DestroyGroup()`); its tracks leave it.
pub struct Group {
    inner: Arc<GroupInner>,
    owner: bool,
}

impl fmt::Debug for Group {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Group")
            .field("tracks", &lock(&self.inner.tracks).len())
            .finish_non_exhaustive()
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        if self.owner {
            destroy_group(&self.inner);
        }
    }
}

impl Group {
    fn borrowed(inner: &Arc<GroupInner>) -> Group {
        Group {
            inner: inner.clone(),
            owner: false,
        }
    }

    /// Create a group on a mixer. Translation of `MIX_CreateGroup()`.
    pub fn new(mixer: &Mixer) -> Result<Group> {
        mixer.inner.check()?;
        Ok(Group {
            inner: create_group(&mixer.inner)?,
            owner: true,
        })
    }

    /// The group's properties. Translation of `MIX_GetGroupProperties()`.
    pub fn properties(&self) -> Result<Properties> {
        self.inner.check()?;
        Ok(self.inner.props.clone())
    }

    /// The mixer the group belongs to. Translation of `MIX_GetGroupMixer()`.
    pub fn mixer(&self) -> Result<Mixer> {
        self.inner.check()?;
        let mixer = self
            .inner
            .mixer
            .upgrade()
            .ok_or_else(|| Error::invalid_param("group"))?;
        Ok(Mixer::borrowed(&mixer))
    }

    /// Set a callback that sees the group's mix; `None` removes it.
    /// Translation of `MIX_SetGroupPostMixCallback()`.
    pub fn set_postmix_callback(
        &self,
        cb: Option<impl Fn(&Group, &AudioSpec, &mut [f32]) + Send + Sync + 'static>,
    ) -> Result<()> {
        self.inner.check()?;
        let mixer = self
            .inner
            .mixer
            .upgrade()
            .ok_or_else(|| Error::invalid_param("group"))?;

        let _guard = mixer.lock();
        *lock(&self.inner.postmix_callback) = cb.map(|c| Arc::new(c) as GroupMixCallback);

        Ok(())
    }
}

/// Translation of `MIX_CreateAudioDecoder_IO()`.
fn create_audio_decoder_io(
    mut io: IoStream<'static>,
    props: Option<&Properties>,
) -> Result<AudioDecoder> {
    check_initialized()?;

    let tmpprops = Properties::new();
    if let Some(props) = props {
        tmpprops.copy_from(props)?;
    }

    let _ = tmpprops.set(PROP_AUDIO_LOAD_ONDEMAND_BOOLEAN, true);
    let audio = load_audio_with_properties(Some(&mut io), None, Some(&tmpprops), None)?;

    let mut track_userdata =
        audio
            .0
            .decoder_userdata
            .clone()
            .init_track(Some(io), &audio.0.spec, &audio.0.props)?;
    track_userdata.seek(0)?;
    let stream = AudioStream::new(Some(&audio.0.spec), Some(&audio.0.spec))?;

    // we want this stream to survive SDL_Quit(), since it's not attached to an audio device.
    let _ = stream
        .properties()
        .set(PROP_AUDIOSTREAM_AUTO_CLEANUP_BOOLEAN, false);

    let inner = Arc::new(AudioDecoderInner {
        audio,
        track_userdata: Mutex::new(Some(track_userdata)),
        stream,
        destroyed: AtomicBool::new(false),
    });

    lock_global().all_audiodecoders.insert(0, inner.clone());

    Ok(AudioDecoder { inner })
}

/// Translation of `MIX_DestroyAudioDecoder()`.
fn destroy_audio_decoder(audiodecoder: &Arc<AudioDecoderInner>) {
    if audiodecoder.destroyed.swap(true, Ordering::AcqRel) {
        return;
    }
    lock_global()
        .all_audiodecoders
        .retain(|d| !Arc::ptr_eq(d, audiodecoder));

    *lock(&audiodecoder.track_userdata) = None; // quit_track (and closes the stream)
}

/// Decodes a file without a mixer. Translation of `MIX_AudioDecoder`.
///
/// Dropping it destroys the decoder (`MIX_DestroyAudioDecoder()`).
pub struct AudioDecoder {
    inner: Arc<AudioDecoderInner>,
}

impl fmt::Debug for AudioDecoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AudioDecoder")
            .field("audio", &self.inner.audio)
            .finish_non_exhaustive()
    }
}

impl Drop for AudioDecoder {
    fn drop(&mut self) {
        destroy_audio_decoder(&self.inner);
    }
}

impl AudioDecoder {
    /// Translation of `CheckAudioDecoderParam()`.
    fn check(&self) -> Result<()> {
        check_initialized()?;
        if self.inner.destroyed.load(Ordering::Acquire) {
            return Err(Error::invalid_param("audiodecoder"));
        }
        Ok(())
    }

    /// Open a file to decode. Translation of `MIX_CreateAudioDecoder()`;
    /// the properties are as for [`Audio::load_with_properties`].
    pub fn new(
        path: impl AsRef<std::path::Path>,
        props: Option<&Properties>,
    ) -> Result<AudioDecoder> {
        check_initialized()?;
        let io = IoStream::from_file(path, "rb")?;
        create_audio_decoder_io(io, props)
    }

    /// Decode a stream, which the decoder owns from now on. Translation of
    /// `MIX_CreateAudioDecoder_IO()`.
    pub fn new_io(io: IoStream<'static>, props: Option<&Properties>) -> Result<AudioDecoder> {
        create_audio_decoder_io(io, props)
    }

    /// The decoded audio's properties (its metadata). Translation of
    /// `MIX_GetAudioDecoderProperties()`.
    pub fn properties(&self) -> Result<Properties> {
        self.check()?;
        self.inner.audio.properties()
    }

    /// The format the data decodes to. Translation of `MIX_GetAudioDecoderFormat()`.
    pub fn format(&self) -> Result<AudioSpec> {
        self.check()?;
        self.inner.audio.format()
    }

    /// Decode into `buffer`, in `spec`'s format; returns the bytes written
    /// (fewer at the end of the data, zero after it). Translation of
    /// `MIX_DecodeAudio()`.
    pub fn decode(&self, buffer: &mut [u8], spec: &AudioSpec) -> Result<usize> {
        self.check()?;

        let _ = self.inner.stream.set_format(None, Some(spec));

        let buflen = buffer.len().min(i32::MAX as usize) as i32;
        let mut track = lock(&self.inner.track_userdata);
        if let Some(track) = track.as_mut() {
            while self.inner.stream.available() < buflen {
                if !track.decode(&self.inner.stream) {
                    self.inner.stream.flush(); // make sure we read _everything_ now.
                    break;
                }
            }
        }

        self.inner.stream.get_data(buffer)
    }
}

/// Milliseconds to sample frames at a sample rate. Translation of
/// `MIX_MSToFrames()`.
pub fn ms_to_frames(sample_rate: i32, ms: i64) -> Result<i64> {
    if sample_rate <= 0 {
        return Err(Error::invalid_param("sample_rate"));
    } else if ms < 0 {
        return Err(Error::invalid_param("ms"));
    }
    Ok((((ms as f64) / 1000.0) * (sample_rate as f64)) as i64)
}

/// Sample frames to milliseconds at a sample rate. Translation of
/// `MIX_FramesToMS()`.
pub fn frames_to_ms(sample_rate: i32, frames: i64) -> Result<i64> {
    if sample_rate <= 0 {
        return Err(Error::invalid_param("sample_rate"));
    } else if frames < 0 {
        return Err(Error::invalid_param("frames"));
    }
    Ok((((frames as f64) / (sample_rate as f64)) * 1000.0) as i64)
}

// table to convert from mu-law encoding to floating point samples,
// generated by a throwaway perl script
const fn s2f(s: i32) -> f32 {
    (s as f32) / 32767.0 // short to float.
}

macro_rules! s2f_table {
    ($($s:expr),* $(,)?) => { [$(s2f($s)),*] };
}

/// Translation of `MIX_ulawToFloat`.
pub(crate) static ULAW_TO_FLOAT: [f32; 256] = s2f_table![
    -32124, -31100, -30076, -29052, -28028, -27004, -25980, -24956, -23932, -22908, -21884, -20860,
    -19836, -18812, -17788, -16764, -15996, -15484, -14972, -14460, -13948, -13436, -12924, -12412,
    -11900, -11388, -10876, -10364, -9852, -9340, -8828, -8316, -7932, -7676, -7420, -7164, -6908,
    -6652, -6396, -6140, -5884, -5628, -5372, -5116, -4860, -4604, -4348, -4092, -3900, -3772,
    -3644, -3516, -3388, -3260, -3132, -3004, -2876, -2748, -2620, -2492, -2364, -2236, -2108,
    -1980, -1884, -1820, -1756, -1692, -1628, -1564, -1500, -1436, -1372, -1308, -1244, -1180,
    -1116, -1052, -988, -924, -876, -844, -812, -780, -748, -716, -684, -652, -620, -588, -556,
    -524, -492, -460, -428, -396, -372, -356, -340, -324, -308, -292, -276, -260, -244, -228, -212,
    -196, -180, -164, -148, -132, -120, -112, -104, -96, -88, -80, -72, -64, -56, -48, -40, -32,
    -24, -16, -8, 0, 32124, 31100, 30076, 29052, 28028, 27004, 25980, 24956, 23932, 22908, 21884,
    20860, 19836, 18812, 17788, 16764, 15996, 15484, 14972, 14460, 13948, 13436, 12924, 12412,
    11900, 11388, 10876, 10364, 9852, 9340, 8828, 8316, 7932, 7676, 7420, 7164, 6908, 6652, 6396,
    6140, 5884, 5628, 5372, 5116, 4860, 4604, 4348, 4092, 3900, 3772, 3644, 3516, 3388, 3260, 3132,
    3004, 2876, 2748, 2620, 2492, 2364, 2236, 2108, 1980, 1884, 1820, 1756, 1692, 1628, 1564, 1500,
    1436, 1372, 1308, 1244, 1180, 1116, 1052, 988, 924, 876, 844, 812, 780, 748, 716, 684, 652,
    620, 588, 556, 524, 492, 460, 428, 396, 372, 356, 340, 324, 308, 292, 276, 260, 244, 228, 212,
    196, 180, 164, 148, 132, 120, 112, 104, 96, 88, 80, 72, 64, 56, 48, 40, 32, 24, 16, 8, 0,
];

/// Translation of `MIX_alawToFloat`.
pub(crate) static ALAW_TO_FLOAT: [f32; 256] = s2f_table![
    -5504, -5248, -6016, -5760, -4480, -4224, -4992, -4736, -7552, -7296, -8064, -7808, -6528,
    -6272, -7040, -6784, -2752, -2624, -3008, -2880, -2240, -2112, -2496, -2368, -3776, -3648,
    -4032, -3904, -3264, -3136, -3520, -3392, -22016, -20992, -24064, -23040, -17920, -16896,
    -19968, -18944, -30208, -29184, -32256, -31232, -26112, -25088, -28160, -27136, -11008, -10496,
    -12032, -11520, -8960, -8448, -9984, -9472, -15104, -14592, -16128, -15616, -13056, -12544,
    -14080, -13568, -344, -328, -376, -360, -280, -264, -312, -296, -472, -456, -504, -488, -408,
    -392, -440, -424, -88, -72, -120, -104, -24, -8, -56, -40, -216, -200, -248, -232, -152, -136,
    -184, -168, -1376, -1312, -1504, -1440, -1120, -1056, -1248, -1184, -1888, -1824, -2016, -1952,
    -1632, -1568, -1760, -1696, -688, -656, -752, -720, -560, -528, -624, -592, -944, -912, -1008,
    -976, -816, -784, -880, -848, 5504, 5248, 6016, 5760, 4480, 4224, 4992, 4736, 7552, 7296, 8064,
    7808, 6528, 6272, 7040, 6784, 2752, 2624, 3008, 2880, 2240, 2112, 2496, 2368, 3776, 3648, 4032,
    3904, 3264, 3136, 3520, 3392, 22016, 20992, 24064, 23040, 17920, 16896, 19968, 18944, 30208,
    29184, 32256, 31232, 26112, 25088, 28160, 27136, 11008, 10496, 12032, 11520, 8960, 8448, 9984,
    9472, 15104, 14592, 16128, 15616, 13056, 12544, 14080, 13568, 344, 328, 376, 360, 280, 264,
    312, 296, 472, 456, 504, 488, 408, 392, 440, 424, 88, 72, 120, 104, 24, 8, 56, 40, 216, 200,
    248, 232, 152, 136, 184, 168, 1376, 1312, 1504, 1440, 1120, 1056, 1248, 1184, 1888, 1824, 2016,
    1952, 1632, 1568, 1760, 1696, 688, 656, 752, 720, 560, 528, 624, 592, 944, 912, 1008, 976, 816,
    784, 880, 848,
];
