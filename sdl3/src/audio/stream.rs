// Rust translation of the audio stream of src/audio/SDL_audiocvt.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use std::cell::{RefCell, RefMut};
use std::sync::Arc;

use super::convert::{calculate_max_frame_size, convert_audio, ConvertSrc};
use super::device::{self, LogicalDevice};
use super::format::{
    audio_specs_equal, channel_map_is_bogus, channel_map_is_default, is_supported_channel_count,
    AudioFormat, AudioSpec,
};
use super::queue::{chmap_ref, store_chmap, AudioQueue, ChannelMap};
use super::resample;
use crate::error::{Error, Result};
use crate::events::AudioDeviceID;
use crate::properties::Properties;
use crate::thread::{ReentrantMutex, ReentrantMutexGuard};

/// Translation of `SDL_PROP_AUDIOSTREAM_AUTO_CLEANUP_BOOLEAN`: if `false`,
/// the stream is not destroyed automatically by
/// [`init::quit`](crate::init::quit) (it is by default).
pub const PROP_AUDIOSTREAM_AUTO_CLEANUP_BOOLEAN: &str = "SDL.audiostream.auto_cleanup";

/// A callback that fires around an audio stream's data being put or got.
/// Translation of `SDL_AudioStreamCallback`.
///
/// It receives the stream, the number of bytes of additional data needed
/// (or just added), and the total amount (see
/// [`AudioStream::set_get_callback`] and [`AudioStream::set_put_callback`]).
/// It runs with the stream locked, possibly on the audio device's thread,
/// and may call back into the stream (to put data, for example).
pub type AudioStreamCallback = Arc<dyn Fn(&AudioStream, i32, i32) + Send + Sync>;

/// The fields of `struct SDL_AudioStream` behind its `lock`.
pub(crate) struct StreamState {
    pub(crate) props: Option<Properties>,

    pub(crate) get_callback: Option<AudioStreamCallback>,
    pub(crate) put_callback: Option<AudioStreamCallback>,

    pub(crate) src_spec: AudioSpec,
    pub(crate) dst_spec: AudioSpec,
    pub(crate) src_chmap: ChannelMap,
    pub(crate) dst_chmap: ChannelMap,
    freq_ratio: f32,
    gain: f32,

    queue: AudioQueue,

    /// The spec of input data currently being processed
    input_spec: AudioSpec,
    input_chmap: ChannelMap,
    resample_offset: i64,

    /// used for scratch space during data conversion/resampling.
    work_buffer: Vec<u8>,

    /// true if created via `open_audio_device_stream`
    pub(crate) simplified: bool,

    pub(crate) bound_device: Option<Arc<LogicalDevice>>,
}

/// Translation of `struct SDL_AudioStream`: the state, behind SDL's
/// recursive stream lock.
pub(crate) struct StreamInner {
    lock: ReentrantMutex<RefCell<StreamState>>,
    /// Set once the stream has been destroyed (by its owner or by
    /// `SDL_QuitAudio`).
    destroyed: std::sync::atomic::AtomicBool,
}

impl StreamInner {
    /// Lock the stream (`SDL_LockMutex(stream->lock)`).
    pub(crate) fn lock(&self) -> ReentrantMutexGuard<'_, RefCell<StreamState>> {
        self.lock.lock()
    }
}

/// The audio stream object, for format conversion and resampling.
/// Translation of `SDL_AudioStream`.
///
/// An audio stream accepts data in one format (the *source* or *input*
/// side) and hands it back converted and resampled to another (the
/// *destination* or *output* side). Data is pushed with
/// [`put_data`](Self::put_data) and pulled with [`get_data`](Self::get_data),
/// from any thread. Bound to an audio device (see
/// [`AudioDevice::bind`](super::AudioDevice::bind)), the device's end of
/// the stream is fed or drained automatically.
///
/// Dropping the stream destroys it (`SDL_DestroyAudioStream()`): it is
/// unbound from its device, and a stream from
/// [`open_audio_device_stream`](super::open_audio_device_stream) closes its
/// device too.
pub struct AudioStream {
    inner: Arc<StreamInner>,
    /// Only the owning handle destroys the stream; callbacks receive a
    /// borrowed, non-owning handle.
    owner: bool,
}

impl std::fmt::Debug for AudioStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let guard = self.inner.lock();
        let result = match guard.try_borrow() {
            Ok(s) => f
                .debug_struct("AudioStream")
                .field("src_spec", &s.src_spec)
                .field("dst_spec", &s.dst_spec)
                .field("queued", &s.queue.queued())
                .finish_non_exhaustive(),
            Err(_) => f.write_str("AudioStream { <in use> }"),
        };
        result
    }
}

/// A lock on an [`AudioStream`]; see [`AudioStream::lock`].
#[must_use]
pub struct AudioStreamLock<'a> {
    _guard: ReentrantMutexGuard<'a, RefCell<StreamState>>,
}

impl std::fmt::Debug for AudioStreamLock<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AudioStreamLock")
    }
}

/// The audio stream frequency ratio limits. Picked mostly arbitrarily.
const MIN_FREQ_RATIO: f32 = 0.01;
const MAX_FREQ_RATIO: f32 = 100.0;

/// When copying in large amounts of data, try and do as much work as
/// possible outside of the stream lock, otherwise the output device is
/// likely to be starved.
const LARGE_INPUT_THRESH: usize = 64 * 1024;

/// Process the data in chunks to avoid allocating too much memory (and
/// potential integer overflows).
const CHUNK_SIZE: i64 = 4096;

/// Is the data small enough to just interleave it on the stack and put it
/// through the normal interface? Translation of `INTERLEAVE_STACK_SIZE`.
const INTERLEAVE_STACK_SIZE: usize = 1024;

impl StreamState {
    /// Translation of `GetAudioStreamResampleRate()`.
    fn resample_rate(&self, src_freq: i32, resample_offset: i64) -> i64 {
        let src_freq = (src_freq as f32 * self.freq_ratio) as i32;

        let mut resample_rate = resample::get_resample_rate(src_freq, self.dst_spec.freq);

        // If src_freq == dst_freq, and we aren't between frames, don't resample
        if resample_rate == 0x1_0000_0000 && resample_offset == 0 {
            resample_rate = 0;
        }

        resample_rate
    }

    /// Translation of `UpdateAudioStreamInputSpec()`.
    fn update_input_spec(&mut self, spec: &AudioSpec, chmap: &ChannelMap) -> bool {
        if audio_specs_equal(
            &self.input_spec,
            spec,
            chmap_ref(&self.input_chmap),
            chmap_ref(chmap),
        ) {
            return true;
        }

        if !self
            .queue
            .reset_history(resample::get_resampler_history_frames())
        {
            return false;
        }

        self.input_chmap = *chmap;
        self.input_spec = *spec;

        true
    }

    /// Translation of `CheckAudioStreamIsFullySetup()`.
    fn check_fully_setup(&self) -> Result<()> {
        if self.src_spec.format == AudioFormat::UNKNOWN {
            Err(Error::new("Stream has no source format"))
        } else if self.dst_spec.format == AudioFormat::UNKNOWN {
            Err(Error::new("Stream has no destination format"))
        } else {
            Ok(())
        }
    }

    /// Translation of `NextAudioStreamIter()`.
    fn next_iter(
        &self,
        iter: &mut super::queue::QueueIter,
        inout_resample_offset: &mut i64,
    ) -> (i64, AudioSpec, ChannelMap, bool) {
        let (queued_bytes, spec, chmap, flushed) = self.queue.next_iter(iter);

        // There is infinite audio available, whether or not we are resampling
        if queued_bytes == usize::MAX {
            *inout_resample_offset = 0;
            return (i32::MAX as i64, spec, chmap, false);
        }

        let mut resample_offset = *inout_resample_offset;
        let resample_rate = self.resample_rate(spec.freq, resample_offset);
        let mut output_frames = (queued_bytes / spec.frame_size()) as i64;

        if resample_rate != 0 {
            // Resampling requires padding frames to the left and right of the current position.
            // Past the end of the track, the right padding is filled with silence.
            // But we only want to do that if the track is actually finished (flushed).
            if !flushed {
                output_frames -= resample::get_resampler_padding_frames(resample_rate) as i64;
            }

            output_frames = resample::get_resampler_output_frames(
                output_frames,
                resample_rate,
                &mut resample_offset,
            );
        }

        if flushed {
            resample_offset = 0;
        }

        *inout_resample_offset = resample_offset;

        (output_frames, spec, chmap, flushed)
    }

    /// Translation of `GetAudioStreamAvailableFrames()`.
    fn available_frames(&self) -> (i64, i64) {
        let mut iter = self.queue.begin_iter();

        let mut resample_offset = self.resample_offset;
        let mut output_frames: i64 = 0;

        while iter.is_some() {
            output_frames += self.next_iter(&mut iter, &mut resample_offset).0;

            // Already got loads of frames. Just clamp it to something reasonable
            if output_frames >= i32::MAX as i64 {
                output_frames = i32::MAX as i64;
                break;
            }
        }

        (output_frames, resample_offset)
    }

    /// Translation of `GetAudioStreamHead()`.
    fn head(&self) -> (i64, AudioSpec, ChannelMap, bool) {
        let mut iter = self.queue.begin_iter();

        if iter.is_none() {
            return (0, AudioSpec::default(), None, false);
        }

        let mut resample_offset = self.resample_offset;
        self.next_iter(&mut iter, &mut resample_offset)
    }

    /// Translation of `SDL_GetAudioStreamAvailable()`'s body (stream locked).
    fn available_bytes(&self) -> i32 {
        if self.check_fully_setup().is_err() {
            return 0;
        }

        let mut count = self.available_frames().0;

        // convert from sample frames to bytes in destination format.
        count *= self.dst_spec.frame_size() as i64;

        // if this overflows an int, just clamp it to a maximum.
        count.min(i32::MAX as i64) as i32
    }

    /// this does not save the previous contents of the work buffer. It's a
    /// work buffer!! Translation of `EnsureAudioStreamWorkBufferSize()`.
    fn ensure_work_buffer(&mut self, newlen: usize) {
        if self.work_buffer.len() < newlen {
            self.work_buffer = vec![0u8; newlen];
        }
    }

    /// Translation of `GetAudioStreamDataInternal()`.
    ///
    /// You must hold the stream lock and validate your parameters before
    /// calling this! Enough input data MUST be available!
    fn get_data_internal(&mut self, buf: &mut [u8], output_frames: usize, gain: f32) -> Result<()> {
        let src_spec = self.input_spec;
        let dst_spec = self.dst_spec;

        let src_format = src_spec.format;
        let src_channels = src_spec.channels;

        let dst_format = dst_spec.format;
        let dst_channels = dst_spec.channels;
        let dst_map_storage = self.dst_chmap;
        let dst_map = chmap_ref(&dst_map_storage);

        let max_frame_size =
            calculate_max_frame_size(src_format, src_channels, dst_format, dst_channels);
        let resample_rate = self.resample_rate(src_spec.freq, self.resample_offset);

        crate::sdl_assert!(output_frames > 0);

        // Not resampling? It's an easy conversion (and maybe not even that!)
        if resample_rate == 0 {
            let mut need_work = false;

            // Ensure we have enough scratch space for any conversions
            if (src_format != dst_format) || (src_channels != dst_channels) || (gain != 1.0) {
                self.ensure_work_buffer(output_frames * max_frame_size);
                need_work = true;
            }

            let StreamState {
                queue, work_buffer, ..
            } = self;
            let scratch = need_work.then_some(&mut work_buffer[..]);
            if queue
                .read(
                    Some(buf),
                    dst_format,
                    dst_channels,
                    dst_map,
                    0,
                    output_frames,
                    0,
                    scratch,
                    gain,
                )
                .is_err()
            {
                return Err(Error::new("Not enough data in queue"));
            }

            return Ok(());
        }

        // Time to do some resampling!
        // Calculate the number of input frames necessary for this request.
        // Because resampling happens "between" frames, The same number of output_frames
        // can require a different number of input_frames, depending on the resample_offset.
        // In fact, input_frames can sometimes even be zero when upsampling.
        let input_frames = resample::get_resampler_input_frames(
            output_frames as i64,
            resample_rate,
            self.resample_offset,
        ) as i32;

        let padding_frames = resample::get_resampler_padding_frames(resample_rate);

        let resample_format = AudioFormat::F32;

        // If increasing channels, do it after resampling, since we'd just
        // do more work to resample duplicate channels. If we're decreasing, do
        // it first so we resample the interpolated data instead of interpolating
        // the resampled data.
        let resample_channels = src_channels.min(dst_channels);

        // The size of the frame used when resampling
        let resample_frame_size = resample_format.bytesize() as usize * resample_channels as usize;

        // The main portion of the work_buffer can be used to store 3 things:
        // src_sample_frame_size * (left_padding+input_buffer+right_padding)
        //   resample_frame_size * (left_padding+input_buffer+right_padding)
        // dst_sample_frame_size * output_frames
        //
        // ResampleAudio also requires an additional buffer if it can't write straight to the output:
        //   resample_frame_size * output_frames
        //
        // Note, ConvertAudio requires (num_frames * max_sample_frame_size) of scratch space
        let work_buffer_frames = input_frames as usize + (padding_frames as usize * 2);
        let mut work_buffer_capacity = work_buffer_frames * max_frame_size;
        let mut resample_buffer_offset = None;

        // Check if we can resample directly into the output buffer.
        // Note, this is just to avoid extra copies.
        // Some other formats may fit directly into the output buffer, but i'd rather process data in a SIMD-aligned buffer.
        if (dst_format != resample_format) || (dst_channels != resample_channels) {
            // Allocate space for converting the resampled output to the destination format
            let resample_convert_bytes = output_frames * max_frame_size;
            work_buffer_capacity = work_buffer_capacity.max(resample_convert_bytes);

            // SIMD-align the buffer
            let simd_alignment = crate::cpuinfo::simd_alignment();
            work_buffer_capacity += simd_alignment - 1;
            work_buffer_capacity -= work_buffer_capacity % simd_alignment;

            // Allocate space for the resampled output
            let resample_bytes = output_frames * resample_frame_size;
            resample_buffer_offset = Some(work_buffer_capacity);
            work_buffer_capacity += resample_bytes;
        }

        self.ensure_work_buffer(work_buffer_capacity);

        // adjust gain either before resampling or after, depending on which point has less
        // samples to process.
        let preresample_gain = if input_frames as usize > output_frames {
            1.0
        } else {
            gain
        };
        let postresample_gain = if input_frames as usize > output_frames {
            gain
        } else {
            1.0
        };

        let StreamState {
            queue,
            work_buffer,
            resample_offset,
            ..
        } = self;

        // (dst channel map is NULL because we'll do the final swizzle on ConvertAudio after resample.)
        if queue
            .read(
                None,
                resample_format,
                resample_channels,
                None,
                padding_frames as usize,
                input_frames as usize,
                padding_frames as usize,
                Some(&mut work_buffer[..]),
                preresample_gain,
            )
            .is_err()
        {
            return Err(Error::new("Not enough data in queue (resample)"));
        }

        let input_offset = padding_frames as usize * resample_frame_size;

        // Decide where the resampled output goes
        match resample_buffer_offset {
            Some(off) => {
                let (head, tail) = work_buffer.split_at_mut(off);
                resample::resample_audio(
                    resample_channels as usize,
                    head,
                    input_offset,
                    input_frames,
                    tail,
                    output_frames,
                    resample_rate,
                    resample_offset,
                );

                // Convert to the final format, if necessary (src channel map is NULL because SDL_ReadFromAudioQueue already handled this).
                convert_audio(
                    output_frames,
                    ConvertSrc::Slice(&tail[..output_frames * resample_frame_size]),
                    resample_format,
                    resample_channels,
                    None,
                    buf,
                    dst_format,
                    dst_channels,
                    dst_map,
                    Some(head),
                    postresample_gain,
                );
            }
            None => {
                resample::resample_audio(
                    resample_channels as usize,
                    work_buffer,
                    input_offset,
                    input_frames,
                    buf,
                    output_frames,
                    resample_rate,
                    resample_offset,
                );

                convert_audio(
                    output_frames,
                    ConvertSrc::InDst,
                    resample_format,
                    resample_channels,
                    None,
                    buf,
                    dst_format,
                    dst_channels,
                    dst_map,
                    Some(&mut work_buffer[..]),
                    postresample_gain,
                );
            }
        }

        Ok(())
    }
}

/// Translation of `InterleaveAudioChannels()` (the `Generic` and
/// `WithNullsGeneric` variants: a missing channel is silence).
fn interleave_audio_channels(
    output: &mut [u8],
    channel_buffers: &[Option<&[u8]>],
    num_samples: usize,
    spec: &AudioSpec,
) {
    let bytes = spec.format.bytesize() as usize;
    let channels = spec.channels as usize; // it's either < 0, needs to be clamped to spec->channels, or we just padded it out to spec->channels with channels_full.
    let silence = spec.format.silence_value();

    // !!! FIXME: it would be possible to do this really well in SIMD for stereo data, using unpack (intel) or zip (arm) instructions, etc.
    let mut o = 0;
    for frame in 0..num_samples {
        for channel in 0..channels {
            let out = &mut output[o..o + bytes];
            match channel_buffers.get(channel).copied().flatten() {
                Some(src) => out.copy_from_slice(&src[frame * bytes..(frame + 1) * bytes]),
                None => out.fill(silence),
            }
            o += bytes;
        }
    }
}

impl AudioStream {
    /// Wrap shared stream state in a non-owning handle (for callbacks and
    /// the audio device threads).
    pub(crate) fn borrowed(inner: &Arc<StreamInner>) -> AudioStream {
        AudioStream {
            inner: inner.clone(),
            owner: false,
        }
    }

    pub(crate) fn inner(&self) -> &Arc<StreamInner> {
        &self.inner
    }

    /// Lock and borrow the state. Never call out (callbacks, devices) while
    /// the returned borrow is alive.
    fn with_state<R>(&self, f: impl FnOnce(&mut StreamState) -> R) -> R {
        let guard = self.inner.lock();
        let mut state: RefMut<'_, StreamState> = guard.borrow_mut();
        f(&mut state)
    }

    /// Create a new audio stream. Translation of `SDL_CreateAudioStream()`.
    ///
    /// Either side may be left unset (`None`) and set later with
    /// [`set_format`](Self::set_format), or by binding the stream to a
    /// device, which sets the device's side.
    pub fn new(src_spec: Option<&AudioSpec>, dst_spec: Option<&AudioSpec>) -> Result<AudioStream> {
        resample::setup_audio_resampler();

        let inner = Arc::new(StreamInner {
            lock: ReentrantMutex::new(RefCell::new(StreamState {
                props: None,
                get_callback: None,
                put_callback: None,
                src_spec: AudioSpec::default(),
                dst_spec: AudioSpec::default(),
                src_chmap: None,
                dst_chmap: None,
                freq_ratio: 1.0,
                gain: 1.0,
                queue: AudioQueue::new(8192),
                input_spec: AudioSpec::default(),
                input_chmap: None,
                resample_offset: 0,
                work_buffer: Vec::new(),
                simplified: false,
                bound_device: None,
            })),
            destroyed: std::sync::atomic::AtomicBool::new(false),
        });

        device::on_audio_stream_created(&inner);

        let stream = AudioStream { inner, owner: true };
        stream.set_format(src_spec, dst_spec)?; // (on failure, dropping `stream` destroys it)
        Ok(stream)
    }

    /// Get the properties associated with an audio stream (created on first
    /// use). Translation of `SDL_GetAudioStreamProperties()`.
    ///
    /// The following properties are understood by SDL:
    ///
    /// - [`PROP_AUDIOSTREAM_AUTO_CLEANUP_BOOLEAN`]: if `false`, the stream
    ///   is not automatically destroyed during [`init::quit`](crate::init::quit).
    ///   Defaults to `true`.
    pub fn properties(&self) -> Properties {
        self.with_state(|s| s.props.get_or_insert_with(Properties::new).clone())
    }

    /// Set a callback that runs when data is requested from an audio stream.
    /// Translation of `SDL_SetAudioStreamGetCallback()`.
    ///
    /// This callback is called _before_ data is obtained from the stream,
    /// giving the callback the chance to add more on-demand. It receives
    /// the number of bytes of additional (input-format) data needed to
    /// satisfy the request, and the total amount being requested (which
    /// the stream may already partly hold).
    ///
    /// The callback is not required to supply exact amounts; it is allowed
    /// to supply too much or too little or none at all. The caller will get
    /// some number of converted/resampled bytes up to the amount they
    /// requested, regardless of this callback's outcome.
    ///
    /// Clearing or flushing an audio stream does not call this callback.
    ///
    /// This function obtains the stream's lock, which means any existing
    /// callback (get or put) in progress will finish running before setting
    /// the new callback. `None` removes the callback.
    pub fn set_get_callback(
        &self,
        callback: Option<impl Fn(&AudioStream, i32, i32) + Send + Sync + 'static>,
    ) {
        let cb: Option<AudioStreamCallback> = callback.map(|c| Arc::new(c) as AudioStreamCallback);
        self.with_state(|s| s.get_callback = cb);
    }

    /// Set a callback that runs when data is added to an audio stream.
    /// Translation of `SDL_SetAudioStreamPutCallback()`.
    ///
    /// This callback is called _after_ the data is added to the stream,
    /// giving the callback the chance to obtain it immediately. It receives
    /// the number of bytes of newly-available (output-format) data, twice.
    ///
    /// The callback can (optionally) call [`get_data`](Self::get_data) to
    /// obtain audio from the stream during this call.
    ///
    /// The callback's `additional_amount` is in bytes of _converted_ data.
    /// `None` removes the callback.
    pub fn set_put_callback(
        &self,
        callback: Option<impl Fn(&AudioStream, i32, i32) + Send + Sync + 'static>,
    ) {
        let cb: Option<AudioStreamCallback> = callback.map(|c| Arc::new(c) as AudioStreamCallback);
        self.with_state(|s| s.put_callback = cb);
    }

    /// Set the stream's callbacks from already-shared closures (used by
    /// `open_audio_device_stream`).
    pub(crate) fn set_callback_arc(&self, get: bool, cb: Option<AudioStreamCallback>) {
        self.with_state(|s| {
            if get {
                s.get_callback = cb;
            } else {
                s.put_callback = cb;
            }
        });
    }

    /// Lock an audio stream for serialized access.
    /// Translation of `SDL_LockAudioStream()`; dropping the lock is
    /// `SDL_UnlockAudioStream()`.
    ///
    /// Each stream has an internal (recursive) lock: while you hold it, no
    /// other thread can use the stream, so a group of calls happens
    /// atomically (and no callback runs in between).
    pub fn lock(&self) -> AudioStreamLock<'_> {
        AudioStreamLock {
            _guard: self.inner.lock(),
        }
    }

    /// Query the current format of an audio stream: `(source, destination)`.
    /// Translation of `SDL_GetAudioStreamFormat()`.
    ///
    /// Fails if either side has no format yet.
    pub fn format(&self) -> Result<(AudioSpec, AudioSpec)> {
        let (src, dst) = self.with_state(|s| (s.src_spec, s.dst_spec));

        if src.format.0 == 0 {
            return Err(Error::new("Stream has no source format"));
        } else if dst.format.0 == 0 {
            return Err(Error::new("Stream has no destination format"));
        }

        Ok((src, dst))
    }

    /// Change the input and output formats of an audio stream; `None`
    /// leaves that side alone. Translation of `SDL_SetAudioStreamFormat()`.
    ///
    /// Future calls to and [`available`](Self::available) and
    /// [`get_data`](Self::get_data) will reflect the new format, and future
    /// calls to [`put_data`](Self::put_data) must provide data in the new
    /// input formats.
    ///
    /// Data that was previously queued in the stream will still be operated
    /// on in the format that was current when it was added, which is to say
    /// you can put the end of a sound file in one format to a stream,
    /// change formats for the next sound file, and start putting that new
    /// data while the previous sound file is still queued, and everything
    /// will still play back correctly.
    ///
    /// If a stream is bound to a device, then the format of the side of the
    /// stream bound to a device cannot be changed (src_spec for recording
    /// devices, dst_spec for playback devices). Attempts to make a change to
    /// this side will be ignored, but this will not report an error. The
    /// other side's format can be changed.
    pub fn set_format(
        &self,
        src_spec: Option<&AudioSpec>,
        dst_spec: Option<&AudioSpec>,
    ) -> Result<()> {
        // note that while we've removed the maximum frequency checks, SDL _will_
        // fail to resample to extremely high sample rates correctly. Really high,
        // like 196608000Hz. File a bug.  :P

        if let Some(spec) = src_spec {
            if !spec.format.is_supported() {
                return Err(Error::invalid_param("src_spec->format"));
            } else if !is_supported_channel_count(spec.channels) {
                return Err(Error::invalid_param("src_spec->channels"));
            } else if spec.freq <= 0 {
                return Err(Error::invalid_param("src_spec->freq"));
            }
        }

        if let Some(spec) = dst_spec {
            if !spec.format.is_supported() {
                return Err(Error::invalid_param("dst_spec->format"));
            } else if !is_supported_channel_count(spec.channels) {
                return Err(Error::invalid_param("dst_spec->channels"));
            } else if spec.freq <= 0 {
                return Err(Error::invalid_param("dst_spec->freq"));
            }
        }

        self.with_state(|s| {
            let (mut src_spec, mut dst_spec) = (src_spec, dst_spec);

            // quietly refuse to change the format of the end currently bound to a device.
            if let Some(dev) = &s.bound_device {
                if dev.recording() {
                    src_spec = None;
                } else {
                    dst_spec = None;
                }
            }

            if let Some(spec) = src_spec {
                if spec.channels != s.src_spec.channels {
                    s.src_chmap = None;
                }
                s.src_spec = *spec;
            }

            if let Some(spec) = dst_spec {
                if spec.channels != s.dst_spec.channels {
                    s.dst_chmap = None;
                }
                s.dst_spec = *spec;
            }
        });

        Ok(())
    }

    /// Get the current input channel map of an audio stream (`None` for the
    /// default layout). Translation of `SDL_GetAudioStreamInputChannelMap()`.
    pub fn input_channel_map(&self) -> Option<Vec<i32>> {
        self.with_state(|s| {
            s.src_chmap
                .map(|m| m[..s.src_spec.channels as usize].to_vec())
        })
    }

    /// Get the current output channel map of an audio stream (`None` for the
    /// default layout). Translation of `SDL_GetAudioStreamOutputChannelMap()`.
    pub fn output_channel_map(&self) -> Option<Vec<i32>> {
        self.with_state(|s| {
            s.dst_chmap
                .map(|m| m[..s.dst_spec.channels as usize].to_vec())
        })
    }

    /// Set the current input channel map of an audio stream.
    /// Translation of `SDL_SetAudioStreamInputChannelMap()`.
    ///
    /// Channel maps are optional; most things do not need them, instead
    /// passing data in the [order that SDL expects](https://wiki.libsdl.org/SDL3/CategoryAudio#channel-layouts).
    ///
    /// The input channel map reorders data that is added to a stream via
    /// [`put_data`](Self::put_data). Future calls to `put_data` must provide
    /// data in the new channel order.
    ///
    /// Each item in the slice represents an input channel, and its value is
    /// the channel that it should be remapped to. To reverse a stereo
    /// signal's left and right values, you'd have `[1, 0]`. It is legal to
    /// remap multiple channels to the same thing, so `[1, 1]` would
    /// duplicate the right channel to both channels of a stereo signal. An
    /// element in the channel map set to -1 instead of a valid channel will
    /// mute that channel, setting it to a silence value.
    ///
    /// You cannot change the number of channels through a channel map, just
    /// reorder/mute them: the map's length must match the input spec's
    /// channel count (a safety measure against a race changing the format
    /// meanwhile). `None` turns remapping off. A map that refers to
    /// channels that don't exist is rejected.
    ///
    /// Data that was previously queued in the stream will still be operated
    /// on in the order that was current when it was added.
    ///
    /// Unlike attempting to change the stream's format, the input channel
    /// map on a stream bound to a recording device is permitted to change
    /// at any time; any data added to the stream from the device after this
    /// call will have the new mapping, but previously-added data will still
    /// have the prior mapping.
    pub fn set_input_channel_map(&self, chmap: Option<&[i32]>) -> Result<()> {
        self.set_channel_map(true, chmap)
    }

    /// Set the current output channel map of an audio stream.
    /// Translation of `SDL_SetAudioStreamOutputChannelMap()`.
    ///
    /// The output channel map reorders data that is leaving a stream via
    /// [`get_data`](Self::get_data). See
    /// [`set_input_channel_map`](Self::set_input_channel_map) for the
    /// format; the map's length must match the output spec's channel count.
    ///
    /// The output channel map on a stream bound to a playback device is
    /// permitted to change at any time; data the device gets from the
    /// stream after this call will have the new mapping.
    pub fn set_output_channel_map(&self, chmap: Option<&[i32]>) -> Result<()> {
        self.set_channel_map(false, chmap)
    }

    fn set_channel_map(&self, input: bool, chmap: Option<&[i32]>) -> Result<()> {
        self.with_state(|s| {
            let channels = chmap.map_or(
                if input {
                    s.src_spec.channels
                } else {
                    s.dst_spec.channels
                },
                |m| m.len() as i32,
            );
            set_audio_stream_channel_map(s, input, chmap, channels)
        })
    }

    /// Get the frequency ratio of an audio stream.
    /// Translation of `SDL_GetAudioStreamFrequencyRatio()`.
    pub fn frequency_ratio(&self) -> f32 {
        self.with_state(|s| s.freq_ratio)
    }

    /// Change the frequency ratio of an audio stream.
    /// Translation of `SDL_SetAudioStreamFrequencyRatio()`.
    ///
    /// The frequency ratio is used to adjust the rate at which input data is
    /// consumed. Changing this effectively modifies the speed and pitch of
    /// the audio. A value greater than 1.0 will play the audio faster, and
    /// at a higher pitch. A value less than 1.0 will play the audio slower,
    /// and at a lower pitch. 1.0 means play at normal speed.
    ///
    /// This is applied during [`get_data`](Self::get_data), and can be
    /// continuously changed to create various effects. The ratio must be
    /// between 0.01 and 100.
    pub fn set_frequency_ratio(&self, freq_ratio: f32) -> Result<()> {
        if freq_ratio < MIN_FREQ_RATIO {
            return Err(Error::new("Frequency ratio is too low"));
        } else if freq_ratio > MAX_FREQ_RATIO {
            return Err(Error::new("Frequency ratio is too high"));
        }

        self.with_state(|s| s.freq_ratio = freq_ratio);
        Ok(())
    }

    /// Get the gain of an audio stream. Translation of `SDL_GetAudioStreamGain()`.
    pub fn gain(&self) -> f32 {
        self.with_state(|s| s.gain)
    }

    /// Change the gain of an audio stream. Translation of `SDL_SetAudioStreamGain()`.
    ///
    /// The gain of a stream is its volume; a larger gain means a louder
    /// output, with a gain of zero being silence. Audio streams default to a
    /// gain of 1.0 (no change in output). This is applied during
    /// [`get_data`](Self::get_data), and can be continuously changed to
    /// create various effects.
    pub fn set_gain(&self, gain: f32) -> Result<()> {
        // (`!(gain >= 0.0)` would also reject NaN; upstream only checks `< 0`.)
        if gain < 0.0 {
            return Err(Error::invalid_param("gain"));
        }
        self.with_state(|s| s.gain = gain);
        Ok(())
    }

    /// Translation of `PutAudioStreamBufferInternal()`.
    ///
    /// You MUST hold the stream lock when calling this, and validate your
    /// parameters! `data` is `Ok(bytes)` to copy, or `Err(track data)` to
    /// queue as its own track.
    fn put_buffer_internal(
        &self,
        spec: &AudioSpec,
        chmap: &ChannelMap,
        data: std::result::Result<&[u8], Box<dyn AsRef<[u8]> + Send>>,
    ) -> Result<()> {
        let put_callback = {
            let guard = self.inner.lock();
            let mut s = guard.borrow_mut();
            let prev_available = if s.put_callback.is_some() {
                s.available_bytes()
            } else {
                0
            };

            match data {
                Err(track_data) => {
                    let track =
                        AudioQueue::create_external_track(spec, chmap_ref(chmap), track_data);
                    s.queue.add_track(track);
                }
                Ok(bytes) => s.queue.write(spec, chmap_ref(chmap), bytes),
            }

            s.put_callback
                .clone()
                .map(|cb| (cb, s.available_bytes() - prev_available))
        };

        if let Some((cb, newavail)) = put_callback {
            cb(&AudioStream::borrowed(&self.inner), newavail, newavail);
        }

        Ok(())
    }

    /// Translation of `PutAudioStreamBuffer()`.
    fn put_buffer(
        &self,
        data: std::result::Result<&[u8], Box<dyn AsRef<[u8]> + Send>>,
    ) -> Result<()> {
        let len = match &data {
            Ok(b) => b.len(),
            Err(b) => (**b).as_ref().len(),
        };

        let _guard = self.inner.lock();

        let (spec, chmap) = {
            let s = _guard.borrow();
            s.check_fully_setup()?;

            if len % s.src_spec.frame_size() != 0 {
                return Err(Error::new("Can't add partial sample frames"));
            }
            (s.src_spec, s.src_chmap)
        };

        self.put_buffer_internal(&spec, &chmap, data)
    }

    /// Add data to the stream. Translation of `SDL_PutAudioStreamData()`.
    ///
    /// This data must match the format/channels/samplerate specified in the
    /// latest call to [`set_format`](Self::set_format), or the format
    /// specified when creating the stream if it hasn't been changed, and
    /// must be whole sample frames.
    ///
    /// Note that this call simply copies the unconverted data for later.
    /// This is different than SDL2, where data was converted during the Put
    /// call and the Get call would just dequeue the previously-converted
    /// data.
    pub fn put_data(&self, buf: &[u8]) -> Result<()> {
        if buf.is_empty() {
            return Ok(()); // nothing to do.
        }

        // When copying in large amounts of data, try and do as much work as possible
        // outside of the stream lock, otherwise the output device is likely to be starved.
        if buf.len() >= LARGE_INPUT_THRESH {
            return self.put_buffer(Err(Box::new(buf.to_vec())));
        }

        self.put_buffer(Ok(buf))
    }

    /// Add data to the stream without copying it.
    /// Translation of `SDL_PutAudioStreamDataNoCopy()`.
    ///
    /// The stream takes ownership of `buf` (a `Vec<u8>`, an `Arc<[u8]>`, a
    /// `&'static [u8]`...) and reads from it until it has consumed the
    /// data; then (or when the stream is cleared or destroyed) it drops it.
    /// Dropping is the C version's completion callback: wrap the buffer in a
    /// type with a `Drop` impl to be notified.
    ///
    /// The data must match the source format and be whole sample frames.
    pub fn put_data_no_copy(&self, buf: impl AsRef<[u8]> + Send + 'static) -> Result<()> {
        if buf.as_ref().is_empty() {
            return Ok(()); // nothing to do. (dropping `buf` is the callback)
        }

        self.put_buffer(Err(Box::new(buf)))
    }

    /// Add data to the stream with each channel in a separate array.
    /// Translation of `SDL_PutAudioStreamPlanarData()`.
    ///
    /// Each entry of `channel_buffers` holds `num_samples` samples of one
    /// channel, in the source format; they are interleaved into the stream.
    /// A `None` entry, or a missing one (fewer entries than the source
    /// spec's channels), is silence. Extra entries are ignored.
    pub fn put_planar_data(
        &self,
        channel_buffers: &[Option<&[u8]>],
        num_samples: usize,
    ) -> Result<()> {
        if num_samples == 0 {
            return Ok(()); // nothing to do.
        }

        // we do the interleaving up front without the lock held, so the audio device doesn't starve while we work.
        //  but we _do_ need to know the current input spec.
        let (spec, chmap) =
            self.with_state(|s| s.check_fully_setup().map(|_| (s.src_spec, s.src_chmap)))?;

        if spec.channels == 1 {
            // nothing to interleave, just use the usual function.
            let Some(Some(buf)) = channel_buffers.first() else {
                return Err(Error::invalid_param("buf"));
            };
            return self.put_data(&buf[..spec.frame_size() * num_samples]);
        }

        let len = spec.frame_size() * num_samples;

        // Is the data small enough to just interleave it on the stack and put it through the normal interface?
        if len <= INTERLEAVE_STACK_SIZE {
            let mut stackbuf = [0u8; INTERLEAVE_STACK_SIZE];
            interleave_audio_channels(&mut stackbuf, channel_buffers, num_samples, &spec);
            // it's okay if the stream format changed on another thread while we didn't hold the lock; PutAudioStreamBufferInternal will notice
            //  and set up a new track with the right format, and the next SDL_PutAudioStreamData will notice that stream->src_spec doesn't
            //  match the new track and set up a new one again. It's a bad idea to change the format on another thread while putting here,
            //  but everything _will_ work out with the format that was (presumably) expected.
            let _guard = self.inner.lock();
            return self.put_buffer_internal(&spec, &chmap, Ok(&stackbuf[..len]));
        }

        // too big for the stack? Just allocate a block and interleave into that. To avoid the extra copy, we'll just set it as a
        //  new track in the queue.
        let mut data = vec![0u8; len];
        interleave_audio_channels(&mut data, channel_buffers, num_samples, &spec);
        let _guard = self.inner.lock();
        self.put_buffer_internal(&spec, &chmap, Err(Box::new(data)))
    }

    /// Tell the stream that you're done sending data, and anything being
    /// buffered should be converted/resampled and made available
    /// immediately. Translation of `SDL_FlushAudioStream()`.
    ///
    /// It is legal to add more data to a stream after flushing, but there
    /// may be audio gaps in the output. Generally this is intended to signal
    /// the end of input, so the complete output becomes available.
    pub fn flush(&self) {
        self.with_state(|s| s.queue.flush());
    }

    /// Get converted/resampled data from the stream, applying an extra gain
    /// on top of the stream's. Translation of `SDL_GetAudioStreamDataAdjustGain()`.
    pub(crate) fn get_data_adjust_gain(&self, buf: &mut [u8], extra_gain: f32) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0); // nothing to do.
        }

        let guard = self.inner.lock();

        let (gain, dst_frame_size, len, get_callback) = {
            let s = guard.borrow();
            s.check_fully_setup()?;

            let gain = s.gain * extra_gain;
            let dst_frame_size = s.dst_spec.frame_size();

            let len = buf.len() - buf.len() % dst_frame_size; // chop off any fractional sample frame.

            // give the callback a chance to fill in more stream data if it wants.
            let get_callback = s.get_callback.clone().map(|cb| {
                let mut total_request = (len / dst_frame_size) as i64; // start with sample frames desired
                let mut additional_request = total_request;

                let (available_frames, resample_offset) = s.available_frames();

                additional_request -= additional_request.min(available_frames);

                let resample_rate = s.resample_rate(s.src_spec.freq, resample_offset);

                if resample_rate != 0 {
                    total_request = resample::get_resampler_input_frames(
                        total_request,
                        resample_rate,
                        resample_offset,
                    );
                    additional_request = resample::get_resampler_input_frames(
                        additional_request,
                        resample_rate,
                        resample_offset,
                    );
                }

                total_request *= s.src_spec.frame_size() as i64; // convert sample frames to bytes.
                additional_request *= s.src_spec.frame_size() as i64; // convert sample frames to bytes.
                (
                    cb,
                    additional_request.min(i32::MAX as i64) as i32,
                    total_request.min(i32::MAX as i64) as i32,
                )
            });
            (gain, dst_frame_size, len, get_callback)
        };

        if let Some((cb, additional, total)) = get_callback {
            cb(&AudioStream::borrowed(&self.inner), additional, total);
        }

        let mut s = guard.borrow_mut();
        let mut total: usize = 0;
        let mut failed = None;

        while total < len {
            // Audio is processed a track at a time.
            let (available_frames, input_spec, input_chmap, flushed) = s.head();

            if available_frames == 0 {
                if flushed {
                    s.queue.pop_head();
                    s.input_spec = AudioSpec::default();
                    s.resample_offset = 0;
                    s.input_chmap = None;
                    continue;
                }
                // There are no frames available, but the track hasn't been flushed, so more might be added later.
                break;
            }

            if !s.update_input_spec(&input_spec, &input_chmap) {
                failed = Some(Error::out_of_memory());
                break;
            }

            // Clamp the output length to the maximum currently available.
            // GetAudioStreamDataInternal requires enough input data is available.
            let mut output_frames = ((len - total) / dst_frame_size) as i64;
            output_frames = output_frames.min(CHUNK_SIZE);
            output_frames = output_frames.min(available_frames);
            let output_frames = output_frames as usize;

            if let Err(e) = s.get_data_internal(
                &mut buf[total..total + output_frames * dst_frame_size],
                output_frames,
                gain,
            ) {
                failed = Some(e);
                break;
            }

            total += output_frames * dst_frame_size;
        }

        match failed {
            Some(e) if total == 0 => Err(e),
            _ => Ok(total),
        }
    }

    /// Get converted/resampled data from the stream.
    /// Translation of `SDL_GetAudioStreamData()`.
    ///
    /// The input/output data format/channels/samplerate is specified when
    /// creating the stream, and can be changed after creation with
    /// [`set_format`](Self::set_format).
    ///
    /// Note that any conversion and resampling necessary is done during this
    /// call, and [`put_data`](Self::put_data) simply queues unconverted data
    /// for later. This is different than SDL2, where that work was done
    /// while inputting new data to the stream and requesting the output
    /// just copied the converted data.
    ///
    /// Returns the number of bytes written into `buf` (whole sample frames,
    /// possibly fewer than requested, possibly zero).
    pub fn get_data(&self, buf: &mut [u8]) -> Result<usize> {
        self.get_data_adjust_gain(buf, 1.0)
    }

    /// Get the number of converted/resampled bytes available.
    /// Translation of `SDL_GetAudioStreamAvailable()`.
    ///
    /// The stream may be buffering data behind the scenes until it has
    /// enough to resample correctly, so this number might be lower than
    /// what you expect, or even be zero. Add more data or flush the stream
    /// if you need the data now.
    ///
    /// If the stream has so much data that it would overflow an `i32`, the
    /// return value is clamped to a maximum value, but no queued data is
    /// lost.
    pub fn available(&self) -> i32 {
        self.with_state(|s| s.available_bytes())
    }

    /// Get the number of bytes currently queued: the amount of unconverted
    /// data put into the stream (clamped to `i32::MAX`).
    /// Translation of `SDL_GetAudioStreamQueued()`.
    ///
    /// This is the amount of data put into the stream that hasn't been
    /// consumed yet; data in the queue may still need to be converted or
    /// resampled, and some may not be retrievable until more is added or
    /// the stream is flushed.
    pub fn queued(&self) -> i32 {
        self.with_state(|s| s.queue.queued().min(i32::MAX as usize) as i32)
    }

    /// Clear any pending data in the stream. Translation of `SDL_ClearAudioStream()`.
    ///
    /// This drops any queued data, so there will be nothing to read from
    /// the stream until more is added.
    pub fn clear(&self) {
        self.with_state(|s| {
            s.queue.clear();
            s.input_spec = AudioSpec::default();
            s.input_chmap = None;
            s.resample_offset = 0;
        });
    }

    /// The logical device this stream is bound to.
    /// Translation of `SDL_GetAudioStreamDevice()`.
    pub fn device(&self) -> Result<AudioDeviceID> {
        self.with_state(|s| match &s.bound_device {
            Some(dev) => Ok(dev.instance_id),
            None => Err(Error::new("Audio stream not bound to an audio device")),
        })
    }

    /// Pause audio playback on the audio device associated with this stream.
    /// Translation of `SDL_PauseAudioStreamDevice()`.
    ///
    /// This pauses the whole logical device the stream is bound to, so any
    /// other streams bound to it pause too.
    pub fn pause_device(&self) -> Result<()> {
        device::pause_audio_device(self.device()?)
    }

    /// Unpause audio playback on the audio device associated with this
    /// stream. Translation of `SDL_ResumeAudioStreamDevice()`.
    ///
    /// This unpauses the whole logical device the stream is bound to.
    pub fn resume_device(&self) -> Result<()> {
        device::resume_audio_device(self.device()?)
    }

    /// Whether the device associated with this stream is paused (`false`
    /// if the stream is unbound). Translation of `SDL_AudioStreamDevicePaused()`.
    pub fn device_paused(&self) -> bool {
        self.device().is_ok_and(device::audio_device_paused)
    }

    /// Destroy the stream: unbind it, or close its device if it came from
    /// `open_audio_device_stream`. Translation of `SDL_DestroyAudioStream()`.
    fn destroy(&self) {
        if self
            .inner
            .destroyed
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return;
        }

        device::on_audio_stream_destroy(&self.inner);

        let (simplified, bound) = self.with_state(|s| {
            s.props = None;
            (s.simplified, s.bound_device.clone())
        });
        if simplified {
            if let Some(dev) = bound {
                crate::sdl_assert!(dev.simplified());
                device::close_audio_device(dev.instance_id); // this will unbind the stream.
            }
        } else {
            device::unbind_audio_streams(&[self]);
        }

        // (the queue's data, including no-copy buffers, is released here.)
        self.with_state(|s| {
            s.queue.clear();
            s.work_buffer = Vec::new();
        });
    }

    /// Destroy the stream if SDL is shutting down and it asked to be
    /// cleaned up (used by `SDL_QuitAudio`).
    pub(crate) fn destroy_for_quit(inner: &Arc<StreamInner>) {
        AudioStream::borrowed(inner).destroy();
    }

    /// Whether the stream wants to be destroyed by `SDL_QuitAudio`.
    pub(crate) fn wants_auto_cleanup(inner: &Arc<StreamInner>) -> bool {
        let guard = inner.lock();
        let s = guard.borrow();
        s.simplified
            || s.props.as_ref().is_none_or(|p| {
                p.get_bool(PROP_AUDIOSTREAM_AUTO_CLEANUP_BOOLEAN)
                    .unwrap_or(true)
            })
    }
}

impl Drop for AudioStream {
    fn drop(&mut self) {
        if self.owner {
            self.destroy();
        }
    }
}

/// Translation of `SetAudioStreamChannelMap()`: the bulk of
/// `SDL_SetAudioStream*putChannelMap`'s work. (Upstream's `isinput`
/// parameter is unused; channel maps may change on bound streams.)
///
/// The stream must be locked (`s` is its state).
pub(crate) fn set_audio_stream_channel_map(
    s: &mut StreamState,
    input: bool,
    chmap: Option<&[i32]>,
    channels: i32,
) -> Result<()> {
    let spec_channels = if input {
        s.src_spec.channels
    } else {
        s.dst_spec.channels
    };
    let stream_chmap = if input {
        &mut s.src_chmap
    } else {
        &mut s.dst_chmap
    };
    let n = channels.max(0) as usize;

    if channels != spec_channels {
        Err(Error::new("Wrong number of channels"))
    } else if stream_chmap.is_none() && chmap.is_none() {
        // already at default, we're good.
        Ok(())
    } else if matches!((stream_chmap.as_ref(), chmap), (Some(cur), Some(new)) if cur[..n] == new[..n])
    {
        // already have this map, don't allocate/copy it again.
        Ok(())
    } else if channel_map_is_bogus(chmap, channels) {
        Err(Error::new("Invalid channel mapping"))
    } else {
        let chmap = if channel_map_is_default(chmap, channels) {
            None // just apply a default mapping.
        } else {
            chmap
        };
        *stream_chmap = store_chmap(chmap, channels);
        Ok(())
    }
}

/// Convert some audio data of one format to another format.
/// Translation of `SDL_ConvertAudioSamples()`.
///
/// Please note that this function is for convenience, but should not be
/// used to resample audio in blocks, as it will introduce audio artifacts
/// on the boundaries. You should only use this function if you are
/// converting audio data in its entirety in one call. If you want to
/// convert audio in smaller chunks, use an [`AudioStream`], which is
/// designed for this situation.
pub fn convert_audio_samples(
    src_spec: &AudioSpec,
    src_data: &[u8],
    dst_spec: &AudioSpec,
) -> Result<Vec<u8>> {
    let stream = AudioStream::new(Some(src_spec), Some(dst_spec))?;
    stream.put_data(src_data)?;
    stream.flush();
    let dstlen = stream.available();
    if dstlen < 0 {
        return Err(Error::new("Couldn't convert audio"));
    }
    let mut dst = vec![0u8; dstlen as usize];
    let got = stream.get_data(&mut dst)?;
    if got != dstlen as usize {
        return Err(Error::new("Couldn't convert audio"));
    }
    Ok(dst)
}
