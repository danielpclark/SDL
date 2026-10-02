// Rust translation of the conversion functions of src/audio/SDL_audiocvt.c
// (SwizzleAudio, ConvertAudio, CalculateMaxFrameSize) from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use super::channel_converters::CHANNEL_CONVERTERS;
use super::format::{
    audio_channel_maps_equal, is_supported_channel_count, AudioFormat, AUDIO_MASK_BIG_ENDIAN,
};
use super::typecvt::{convert_audio_from_float, convert_audio_swap_endian, convert_audio_to_float};

/// Swizzle audio channels in place. It does not change the buffer size.
/// Translation of `SwizzleAudio()` (whose `src != dst` path is the same
/// thing without the temporary frame).
fn swizzle_audio(
    num_frames: usize,
    buf: &mut [u8],
    channels: usize,
    map: &[i32],
    fmt: AudioFormat,
) {
    let bytes = fmt.bytesize() as usize;

    let has_null_mappings = map[..channels].contains(&-1); // !!! FIXME: calculate this when setting the channel map instead.
    let silence = fmt.silence_value(); // (every byte of a silent sample is this value)

    // treat as UintX; we only care about moving bits and not the type here.
    let mut tmp = [0u8; 4 * super::format::MAX_CHANNELMAP_CHANNELS]; // !!! FIXME: allocate this when setting the channel map instead.
    let frame_size = bytes * channels;
    for frame in buf[..num_frames * frame_size].chunks_exact_mut(frame_size) {
        for (ch, &m) in map[..channels].iter().enumerate() {
            let out = &mut tmp[ch * bytes..(ch + 1) * bytes];
            if has_null_mappings && m == -1 {
                out.fill(silence);
            } else {
                let m = m as usize;
                out.copy_from_slice(&frame[m * bytes..(m + 1) * bytes]);
            }
        }
        frame.copy_from_slice(&tmp[..frame_size]);
    }
}

/// Where the input of [`convert_audio`] is.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ConvertSrc<'a> {
    /// A separate buffer.
    Slice(&'a [u8]),
    /// Already at the start of `dst` (`src == dst` in C).
    InDst,
    /// Already at the start of `scratch` (`src == scratch` in C).
    InScratch,
}

/// Translation of `ConvertAudio()`.
///
/// This does type and channel conversions _but not resampling_ (resampling
/// happens in the audio stream). This does not check parameter validity,
/// (beyond asserts), it expects you did that already!
///
/// All of this functions as if src==dst==scratch (conversion in-place): the
/// work happens in `scratch` if given, otherwise in `dst`, and the result
/// ends up at the start of `dst`.
///
/// The scratch buffer must be able to store
/// `num_frames * calculate_max_frame_size(...)` bytes. If the scratch buffer
/// is `None`, this restriction applies to the output buffer instead.
///
/// Since this is a convenient point that audio goes through even if it
/// doesn't need format conversion, we also handle gain adjustment here, so
/// we don't have to make another pass over the data later. Strictly
/// speaking, this is also a "conversion".  :)
#[allow(clippy::too_many_arguments)]
pub(crate) fn convert_audio(
    num_frames: usize,
    src: ConvertSrc<'_>,
    src_format: AudioFormat,
    src_channels: i32,
    mut src_map: Option<&[i32]>,
    dst: &mut [u8],
    dst_format: AudioFormat,
    dst_channels: i32,
    mut dst_map: Option<&[i32]>,
    scratch: Option<&mut [u8]>,
    gain: f32,
) {
    crate::sdl_assert!(src_format.is_supported());
    crate::sdl_assert!(dst_format.is_supported());
    crate::sdl_assert!(is_supported_channel_count(src_channels));
    crate::sdl_assert!(is_supported_channel_count(dst_channels));

    if num_frames == 0 {
        return; // no data to convert, quit.
    }

    let (src_channels_u, dst_channels_u) = (src_channels as usize, dst_channels as usize);
    let src_bytes = num_frames * src_format.bytesize() as usize * src_channels_u;
    let dst_bitsize = dst_format.bitsize();
    let dst_sample_frame_size = (dst_bitsize as usize / 8) * dst_channels_u;
    let dst_bytes = num_frames * dst_sample_frame_size;

    let chmaps_match =
        src_channels == dst_channels && audio_channel_maps_equal(src_channels, src_map, dst_map);
    if chmaps_match {
        src_map = None; // NULL both these out so we don't do any unnecessary swizzling.
        dst_map = None;
    }

    // How much of the working buffer the in-place pipeline touches: the
    // input, and the float32 stages unless one of the early-out paths applies.
    let skips_float = src_channels == dst_channels
        && gain == 1.0
        && (src_format == dst_format || (src_format.0 ^ dst_format.0) == AUDIO_MASK_BIG_ENDIAN);
    let mut needed = src_bytes.max(dst_bytes);
    if !skips_float {
        needed = needed.max(num_frames * 4 * src_channels_u.max(dst_channels_u));
    }

    // Work in dst when it has room (upstream only uses scratch for the
    // stages that need the extra space), otherwise in scratch.
    let work_in_dst = scratch.is_none() || dst.len() >= needed;
    let mut scratch = scratch;
    match src {
        ConvertSrc::Slice(s) => {
            let w: &mut [u8] = if work_in_dst {
                &mut *dst
            } else {
                scratch.as_deref_mut().expect("scratch")
            };
            w[..src_bytes].copy_from_slice(&s[..src_bytes]);
        }
        ConvertSrc::InDst => {
            if !work_in_dst {
                scratch.as_deref_mut().expect("scratch")[..src_bytes]
                    .copy_from_slice(&dst[..src_bytes]);
            }
        }
        ConvertSrc::InScratch => {
            let w = scratch.as_deref_mut().expect("the input is in scratch");
            if work_in_dst {
                dst[..src_bytes].copy_from_slice(&w[..src_bytes]);
            }
        }
    }

    if work_in_dst {
        convert_audio_in_place(
            num_frames,
            dst,
            src_format,
            src_channels,
            src_map,
            dst_format,
            dst_channels,
            dst_map,
            gain,
        );
    } else {
        let work = scratch.expect("scratch");
        convert_audio_in_place(
            num_frames,
            work,
            src_format,
            src_channels,
            src_map,
            dst_format,
            dst_channels,
            dst_map,
            gain,
        );
        // (the final output belongs in `dst`.)
        dst[..dst_bytes].copy_from_slice(&work[..dst_bytes]);
    }
}

/// The body of [`convert_audio`]: convert `num_frames` frames at the start
/// of `work` in place (`src == dst == scratch`). Channel maps that match
/// must already have been cleared.
#[allow(clippy::too_many_arguments)]
pub(crate) fn convert_audio_in_place(
    num_frames: usize,
    work: &mut [u8],
    src_format: AudioFormat,
    src_channels: i32,
    src_map: Option<&[i32]>,
    dst_format: AudioFormat,
    dst_channels: i32,
    dst_map: Option<&[i32]>,
    gain: f32,
) {
    let (src_channels_u, dst_channels_u) = (src_channels as usize, dst_channels as usize);
    let dst_bitsize = dst_format.bitsize();

    /* Type conversion goes like this now:
     - swizzle through source channel map to "standard" layout.
     - byteswap to CPU native format first if necessary.
     - convert to native Float32 if necessary.
     - change channel count if necessary.
     - convert to final data format.
     - byteswap back to foreign format if necessary.
     - swizzle through dest channel map from "standard" layout.

    The expectation is we can process data faster in float32
    (possibly with SIMD), and making several passes over the same
    buffer is likely to be CPU cache-friendly, avoiding the
    biggest performance hit in modern times. Previously we had
    (script-generated) custom converters for every data type and
    it was a bloat on SDL compile times and final library size. */

    // swizzle input to "standard" format if necessary.
    if let Some(map) = src_map {
        swizzle_audio(num_frames, work, src_channels_u, map, src_format);
    }

    // see if we can skip float conversion entirely.
    let mut done = false;
    if src_channels == dst_channels && gain == 1.0 {
        if src_format == dst_format {
            // nothing to do, we're already in the right format, just copy it over if necessary.
            if let Some(map) = dst_map {
                swizzle_audio(num_frames, work, dst_channels_u, map, dst_format);
            }
            done = true;
        } else if (src_format.0 ^ dst_format.0) == AUDIO_MASK_BIG_ENDIAN {
            // just a byteswap needed?
            if let Some(map) = dst_map {
                // do this first, in case we duplicate channels, we can avoid an extra copy if src != dst.
                swizzle_audio(num_frames, work, dst_channels_u, map, dst_format);
            }
            convert_audio_swap_endian(work, num_frames * dst_channels_u, dst_bitsize);
            done = true; // all done.
        }
    }

    if !done {
        let srcconvert = src_format != AudioFormat::F32;
        let channelconvert = src_channels != dst_channels;
        let dstconvert = dst_format != AudioFormat::F32;

        // get us to float format.
        if srcconvert {
            convert_audio_to_float(work, num_frames * src_channels_u, src_format);
        }

        // Gain adjustment
        if gain != 1.0 {
            let total_samples = num_frames * src_channels_u;
            for s in work[..total_samples * 4].chunks_exact_mut(4) {
                let v = f32::from_ne_bytes([s[0], s[1], s[2], s[3]]) * gain;
                s.copy_from_slice(&v.to_ne_bytes());
            }
        }

        // Channel conversion

        if channelconvert {
            // SDL_IsSupportedChannelCount should have caught these asserts, or we added a new format and forgot to update the table.
            crate::sdl_assert!(src_channels_u <= CHANNEL_CONVERTERS.len());
            crate::sdl_assert!(dst_channels_u <= CHANNEL_CONVERTERS[0].len());

            let channel_converter = CHANNEL_CONVERTERS[src_channels_u - 1][dst_channels_u - 1];
            crate::sdl_assert!(channel_converter.is_some());

            // (upstream swaps in SSE versions of stereo->mono and mono->stereo; their results are identical.)
            if let Some(convert) = channel_converter {
                convert(work, 0, 0, num_frames);
            }
        }

        // Resampling is not done in here. SDL_AudioStream handles that.

        // Move to final data type.
        if dstconvert {
            convert_audio_from_float(work, num_frames * dst_channels_u, dst_format);
        }

        if let Some(map) = dst_map {
            swizzle_audio(num_frames, work, dst_channels_u, map, dst_format);
        }
    }
}

/// Calculate the largest frame size needed to convert between the two
/// formats. Translation of `CalculateMaxFrameSize()`.
pub(crate) fn calculate_max_frame_size(
    src_format: AudioFormat,
    src_channels: i32,
    dst_format: AudioFormat,
    dst_channels: i32,
) -> usize {
    let src_format_size = src_format.bytesize() as usize;
    let dst_format_size = dst_format.bytesize() as usize;
    let max_app_format_size = src_format_size.max(dst_format_size);
    let max_format_size = max_app_format_size.max(std::mem::size_of::<f32>()); // ConvertAudio and ResampleAudio use floats.
    let max_channels = src_channels.max(dst_channels) as usize;
    max_format_size * max_channels
}
