// Rust translation of src/audio/SDL_audioqueue.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Internal functions used by the audio stream for queueing audio.

use std::collections::VecDeque;

use super::convert::{convert_audio, ConvertSrc};
use super::format::{audio_specs_equal, AudioFormat, AudioSpec, MAX_CHANNELMAP_CHANNELS};
use crate::error::{Error, Result};

/// A channel map stored inline (`chmap_storage`); `None` is the default layout.
pub(crate) type ChannelMap = Option<[i32; MAX_CHANNELMAP_CHANNELS]>;

/// Store a channel map for `channels` channels.
pub(crate) fn store_chmap(chmap: Option<&[i32]>, channels: i32) -> ChannelMap {
    chmap.map(|m| {
        crate::sdl_assert!(MAX_CHANNELMAP_CHANNELS >= channels as usize);
        let mut storage = [0i32; MAX_CHANNELMAP_CHANNELS];
        let n = channels as usize;
        storage[..n].copy_from_slice(&m[..n]);
        storage
    })
}

/// Borrow a stored channel map.
pub(crate) fn chmap_ref(chmap: &ChannelMap) -> Option<&[i32]> {
    chmap.as_ref().map(|m| &m[..])
}

/// The bytes behind a track.
enum TrackData {
    /// A block from the queue's chunk pool, written by [`AudioQueue::write`].
    Chunk(Vec<u8>),
    /// Data owned by the app (`SDL_PutAudioStreamDataNoCopy`); dropping it
    /// is the `SDL_ReleaseAudioBufferCallback`.
    External(Box<dyn AsRef<[u8]> + Send>),
}

impl TrackData {
    fn bytes(&self) -> &[u8] {
        match self {
            TrackData::Chunk(v) => v,
            TrackData::External(b) => (**b).as_ref(),
        }
    }
}

/// Translation of `SDL_AudioTrack`.
pub(crate) struct AudioTrack {
    spec: AudioSpec,
    chmap: ChannelMap,
    flushed: bool,
    data: TrackData,
    head: usize,
    tail: usize,
    capacity: usize,
}

/// Translation of `SDL_AudioQueue`.
///
/// (The C version also pools `SDL_AudioTrack` structs; here tracks live in
/// a `VecDeque`. The pool of data chunks is kept.)
pub(crate) struct AudioQueue {
    tracks: VecDeque<AudioTrack>,

    history_buffer: Vec<u8>,
    history_length: usize,

    /// `chunk_pool`: free blocks and their size.
    chunk_pool: Vec<Vec<u8>>,
    chunk_size: usize,
}

/// Translation of the `max_free` of `chunk_pool`.
const CHUNK_POOL_MAX_FREE: usize = 4;

impl std::fmt::Debug for AudioQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioQueue")
            .field("tracks", &self.tracks.len())
            .field("queued", &self.queued())
            .finish()
    }
}

/// An iterator position: the index of a track (`void *iter`).
pub(crate) type QueueIter = Option<usize>;

impl AudioQueue {
    /// Create a new audio queue. Translation of `SDL_CreateAudioQueue()`.
    pub(crate) fn new(chunk_size: usize) -> AudioQueue {
        AudioQueue {
            tracks: VecDeque::new(),
            history_buffer: Vec::new(),
            history_length: 0,
            chunk_pool: Vec::new(),
            chunk_size,
        }
    }

    /// Allocate a new block, first checking if there are any in the pool.
    /// Translation of `AllocMemoryPoolBlock()`.
    fn alloc_chunk(&mut self) -> Vec<u8> {
        self.chunk_pool
            .pop()
            .unwrap_or_else(|| vec![0u8; self.chunk_size])
    }

    /// Translation of `DestroyAudioTrack()`: give chunks back to the pool
    /// if there's room (`FreeMemoryPoolBlock()`), drop external data.
    fn destroy_track(&mut self, track: AudioTrack) {
        if let TrackData::Chunk(chunk) = track.data {
            if self.chunk_pool.len() < CHUNK_POOL_MAX_FREE {
                self.chunk_pool.push(chunk);
            }
        }
    }

    /// Completely clear the queue. Translation of `SDL_ClearAudioQueue()`.
    pub(crate) fn clear(&mut self) {
        self.history_length = 0;
        while let Some(track) = self.tracks.pop_front() {
            self.destroy_track(track);
        }
    }

    /// Mark the last track as flushed. Translation of `SDL_FlushAudioQueue()`.
    pub(crate) fn flush(&mut self) {
        if let Some(track) = self.tracks.back_mut() {
            track.flushed = true; // FlushAudioTrack()
        }
    }

    /// Pop the current head track. Translation of `SDL_PopAudioQueueHead()`.
    ///
    /// REQUIRES: The head track must exist, and must have been flushed
    pub(crate) fn pop_head(&mut self) {
        while let Some(track) = self.tracks.pop_front() {
            let flushed = track.flushed;
            self.destroy_track(track);
            if flushed {
                break;
            }
        }
        self.history_length = 0;
    }

    /// Translation of `SDL_CreateAudioTrack()`.
    fn create_track(
        spec: &AudioSpec,
        chmap: Option<&[i32]>,
        data: TrackData,
        len: usize,
        capacity: usize,
    ) -> AudioTrack {
        AudioTrack {
            spec: *spec,
            chmap: store_chmap(chmap, spec.channels),
            flushed: false,
            data,
            head: 0,
            tail: len,
            capacity,
        }
    }

    /// Create a track whose data is owned by the caller (and dropped when
    /// the queue is done with it). Translation of `SDL_CreateAudioTrack()`
    /// with a release callback.
    pub(crate) fn create_external_track(
        spec: &AudioSpec,
        chmap: Option<&[i32]>,
        data: Box<dyn AsRef<[u8]> + Send>,
    ) -> AudioTrack {
        let len = (*data).as_ref().len();
        AudioQueue::create_track(spec, chmap, TrackData::External(data), len, len)
    }

    /// Translation of `CreateChunkedAudioTrack()`.
    fn create_chunked_track(&mut self, spec: &AudioSpec, chmap: Option<&[i32]>) -> AudioTrack {
        let chunk = self.alloc_chunk();
        let mut capacity = self.chunk_size;
        capacity -= capacity % spec.frame_size();
        AudioQueue::create_track(spec, chmap, TrackData::Chunk(chunk), 0, capacity)
    }

    /// Add a track to the end of the queue. Translation of `SDL_AddTrackToAudioQueue()`.
    pub(crate) fn add_track(&mut self, track: AudioTrack) {
        if let Some(tail) = self.tracks.back_mut() {
            // If the spec has changed, make sure to flush the previous track
            if !audio_specs_equal(
                &tail.spec,
                &track.spec,
                chmap_ref(&tail.chmap),
                chmap_ref(&track.chmap),
            ) {
                tail.flushed = true;
            }
        }
        self.tracks.push_back(track);
    }

    /// Translation of `WriteToAudioTrack()`.
    fn write_to_track(track: &mut AudioTrack, data: &[u8]) -> usize {
        if track.flushed || track.tail >= track.capacity {
            return 0;
        }
        let TrackData::Chunk(chunk) = &mut track.data else {
            return 0;
        };

        let len = data.len().min(track.capacity - track.tail);
        chunk[track.tail..track.tail + len].copy_from_slice(&data[..len]);
        track.tail += len;

        len
    }

    /// Write data to the end of queue. Translation of `SDL_WriteToAudioQueue()`.
    ///
    /// REQUIRES: If the spec has changed, the last track must have been flushed
    pub(crate) fn write(&mut self, spec: &AudioSpec, chmap: Option<&[i32]>, mut data: &[u8]) {
        if data.is_empty() {
            return;
        }

        if let Some(track) = self.tracks.back_mut() {
            if !audio_specs_equal(&track.spec, spec, chmap_ref(&track.chmap), chmap) {
                track.flushed = true;
            }
        } else {
            let track = self.create_chunked_track(spec, chmap);
            self.tracks.push_back(track);
        }

        loop {
            let track = self.tracks.back_mut().expect("the queue has a tail track");
            let written = AudioQueue::write_to_track(track, data);
            data = &data[written..];

            if data.is_empty() {
                break;
            }

            let new_track = self.create_chunked_track(spec, chmap);
            self.tracks.push_back(new_track);
        }
    }

    /// Iterate over the tracks in the queue. Translation of `SDL_BeginAudioQueueIter()`.
    pub(crate) fn begin_iter(&self) -> QueueIter {
        (!self.tracks.is_empty()).then_some(0)
    }

    /// Query and update the track iterator: the bytes queued up to (and
    /// including) the next flushed track, that run's spec and channel map,
    /// and whether it ends flushed. Translation of `SDL_NextAudioQueueIter()`.
    ///
    /// REQUIRES: `*iter` is a valid iterator.
    pub(crate) fn next_iter(&self, iter: &mut QueueIter) -> (usize, AudioSpec, ChannelMap, bool) {
        let mut idx = iter.expect("a valid audio queue iterator");
        let first = &self.tracks[idx];
        let (spec, chmap) = (first.spec, first.chmap);

        let mut flushed = false;
        let mut queued_bytes: usize = 0;

        let mut next = Some(idx);
        while let Some(i) = next {
            let track = &self.tracks[i];
            idx = i + 1;
            next = (idx < self.tracks.len()).then_some(idx);

            let avail = track.tail - track.head;

            if avail >= usize::MAX - queued_bytes {
                queued_bytes = usize::MAX;
                flushed = false;
                break;
            }

            queued_bytes += avail;
            flushed = track.flushed;

            if flushed {
                break;
            }
        }

        *iter = next;
        (queued_bytes, spec, chmap, flushed)
    }

    /// Translation of `PeekIntoAudioQueuePast()`, copying into `data`.
    fn peek_past(&self, data: &mut [u8], len: usize) -> Result<()> {
        let track = &self.tracks[0];
        let bytes = track.data.bytes();

        if track.head >= len {
            data[..len].copy_from_slice(&bytes[track.head - len..track.head]);
            return Ok(());
        }

        let past = len - track.head;

        if past > self.history_length {
            return Err(Error::new("Not enough audio history"));
        }

        data[..past]
            .copy_from_slice(&self.history_buffer[self.history_length - past..self.history_length]);
        data[past..past + track.head].copy_from_slice(&bytes[..track.head]);

        Ok(())
    }

    /// Translation of `UpdateAudioQueueHistory()` with the data of the head track.
    fn update_history_from_head(&mut self) {
        let track = &self.tracks[0];
        let data = &track.data.bytes()[..track.tail];
        let len = data.len();
        let history_bytes = self.history_length;

        if len >= history_bytes {
            self.history_buffer[..history_bytes].copy_from_slice(&data[len - history_bytes..]);
        } else {
            let preserve = history_bytes - len;
            self.history_buffer.copy_within(len..len + preserve, 0);
            self.history_buffer[preserve..preserve + len].copy_from_slice(data);
        }
    }

    /// Translation of `ReadFromAudioQueue()`, copying into `data`.
    fn read_raw(&mut self, data: &mut [u8], len: usize) -> Result<()> {
        let mut total = 0;

        loop {
            let track = &mut self.tracks[0];
            let avail = (len - total).min(track.tail - track.head);
            data[total..total + avail]
                .copy_from_slice(&track.data.bytes()[track.head..track.head + avail]);
            track.head += avail;
            total += avail;

            if total == len {
                break;
            }

            if track.flushed {
                return Err(Error::new("Reading past end of flushed track"));
            }

            if self.tracks.len() < 2 {
                return Err(Error::new("Reading past end of incomplete track"));
            }

            self.update_history_from_head();

            let track = self.tracks.pop_front().expect("head track");
            self.destroy_track(track);
        }

        Ok(())
    }

    /// Translation of `PeekIntoAudioQueueFuture()`, copying into `data`.
    fn peek_future(&self, data: &mut [u8], len: usize) -> Result<()> {
        let mut total = 0;
        let mut idx = 0;

        loop {
            let track = &self.tracks[idx];
            let avail = (len - total).min(track.tail - track.head);
            data[total..total + avail]
                .copy_from_slice(&track.data.bytes()[track.head..track.head + avail]);
            total += avail;

            if total == len {
                break;
            }

            if track.flushed {
                // If we have run out of data, fill the rest with silence.
                data[total..len].fill(track.spec.format.silence_value());
                break;
            }

            idx += 1;

            if idx >= self.tracks.len() {
                return Err(Error::new("Peeking past end of incomplete track"));
            }
        }

        Ok(())
    }

    /// Read (and convert) `present_frames` frames from the head of the
    /// queue, with `past_frames` of history before them and `future_frames`
    /// peeked after them. Translation of `SDL_ReadFromAudioQueue()`.
    ///
    /// The data ends up at the start of `dst`, or of `scratch` if `dst` is
    /// `None` (the C version can return a pointer into the queue instead;
    /// here it is always copied). `scratch` must hold
    /// `frames * max_frame_size` bytes when converting.
    ///
    /// Returns an error if the queue is empty (with no message: the caller
    /// sets one) or does not hold enough data.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn read(
        &mut self,
        dst: Option<&mut [u8]>,
        dst_format: AudioFormat,
        dst_channels: i32,
        dst_map: Option<&[i32]>,
        past_frames: usize,
        present_frames: usize,
        future_frames: usize,
        scratch: Option<&mut [u8]>,
        gain: f32,
    ) -> Result<()> {
        let Some(track) = self.tracks.front() else {
            return Err(Error::new(""));
        };

        let src_format = track.spec.format;
        let src_channels = track.spec.channels;
        let src_map_storage = track.chmap;
        let src_map = chmap_ref(&src_map_storage);

        let src_frame_size = src_format.bytesize() as usize * src_channels as usize;
        let dst_frame_size = dst_format.bytesize() as usize * dst_channels as usize;

        let src_past_bytes = past_frames * src_frame_size;
        let src_present_bytes = present_frames * src_frame_size;
        let src_future_bytes = future_frames * src_frame_size;

        let dst_past_bytes = past_frames * dst_frame_size;
        let dst_present_bytes = present_frames * dst_frame_size;

        let convert = (src_format != dst_format) || (src_channels != dst_channels) || (gain != 1.0);

        // Where the output goes, and whether dst and scratch are the same buffer.
        enum Out<'a> {
            /// `dst == scratch` (or no scratch at all).
            Same(&'a mut [u8]),
            Split {
                dst: &'a mut [u8],
                scratch: &'a mut [u8],
            },
        }

        let dst_given = dst.is_some();
        let mut out = match (dst, scratch) {
            // The user didn't ask for the data to be copied, but we need to (convert or not), so store it in the scratch buffer
            (None, Some(s)) => Out::Same(s),
            (None, None) => return Err(Error::new("No buffer to read audio into")),
            (Some(d), None) => Out::Same(d),
            // We are only copying, not converting, so copy straight into the dst buffer
            (Some(d), Some(_)) if !convert => Out::Same(d),
            (Some(d), Some(s)) => Out::Split { dst: d, scratch: s },
        };

        // Can we get all of the data straight from this track?
        let track = &mut self.tracks[0];
        if (track.head >= src_past_bytes)
            && ((track.tail - track.head) >= (src_present_bytes + src_future_bytes))
        {
            let start = track.head - src_past_bytes;
            let total = src_past_bytes + src_present_bytes + src_future_bytes;
            track.head += src_present_bytes;
            let ptr = &track.data.bytes()[start..start + total];

            // Do we still need to copy/convert the data?
            if dst_given || convert {
                let frames = past_frames + present_frames + future_frames;
                match &mut out {
                    Out::Same(buf) => convert_audio(
                        frames,
                        ConvertSrc::Slice(ptr),
                        src_format,
                        src_channels,
                        src_map,
                        buf,
                        dst_format,
                        dst_channels,
                        dst_map,
                        None,
                        gain,
                    ),
                    Out::Split { dst, scratch } => convert_audio(
                        frames,
                        ConvertSrc::Slice(ptr),
                        src_format,
                        src_channels,
                        src_map,
                        dst,
                        dst_format,
                        dst_channels,
                        dst_map,
                        Some(scratch),
                        gain,
                    ),
                }
            } else if let Out::Same(buf) = &mut out {
                // FIXME (upstream): this path hands back the raw track data,
                // so a source channel map is not applied here (the
                // piecewise path below does apply it).
                buf[..total].copy_from_slice(ptr);
            }
            return Ok(());
        }

        let mut off = 0;
        let pieces = [
            (past_frames, src_past_bytes, dst_past_bytes, 0u8),
            (present_frames, src_present_bytes, dst_present_bytes, 1),
            (
                future_frames,
                src_future_bytes,
                future_frames * dst_frame_size,
                2,
            ),
        ];
        for (frames, src_bytes, dst_bytes, which) in pieces {
            if src_bytes == 0 {
                continue;
            }
            // Gather the source bytes into the scratch area, then convert into dst.
            {
                let raw: &mut [u8] = match &mut out {
                    Out::Same(buf) => &mut buf[off..],
                    Out::Split { scratch, .. } => &mut scratch[off..],
                };
                match which {
                    0 => self.peek_past(raw, src_bytes)?,
                    1 => self.read_raw(raw, src_bytes)?,
                    _ => self.peek_future(raw, src_bytes)?,
                }
            }
            match &mut out {
                Out::Same(buf) => convert_audio(
                    frames,
                    ConvertSrc::InDst,
                    src_format,
                    src_channels,
                    src_map,
                    &mut buf[off..],
                    dst_format,
                    dst_channels,
                    dst_map,
                    None,
                    gain,
                ),
                Out::Split { dst, scratch } => convert_audio(
                    frames,
                    ConvertSrc::InScratch,
                    src_format,
                    src_channels,
                    src_map,
                    &mut dst[off..],
                    dst_format,
                    dst_channels,
                    dst_map,
                    Some(&mut scratch[off..]),
                    gain,
                ),
            }
            off += dst_bytes;
        }

        Ok(())
    }

    /// Get the total number of bytes currently queued. Translation of `SDL_GetAudioQueueQueued()`.
    pub(crate) fn queued(&self) -> usize {
        let mut total: usize = 0;
        let mut iter = self.begin_iter();

        while iter.is_some() {
            let (avail, _, _, _) = self.next_iter(&mut iter);

            if avail >= usize::MAX - total {
                total = usize::MAX;
                break;
            }

            total += avail;
        }

        total
    }

    /// Translation of `SDL_ResetAudioQueueHistory()`. Fails if the queue is empty.
    pub(crate) fn reset_history(&mut self, num_frames: i32) -> bool {
        let Some(track) = self.tracks.front() else {
            return false;
        };

        let length = num_frames as usize * track.spec.frame_size();
        let silence = track.spec.format.silence_value();

        if self.history_buffer.len() < length {
            self.history_buffer = vec![0u8; length];
        }

        self.history_length = length;
        self.history_buffer[..length].fill(silence);

        true
    }
}
