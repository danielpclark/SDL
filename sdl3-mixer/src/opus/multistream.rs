// Rust translation of src/opus_multistream_decoder.c and
// src/opus_multistream.c from libopus 1.4 (https://opus-codec.org, at the
// revision SDL_mixer's external/opus pins).
// Copyright (c) 2011 Xiph.Org Foundation
// Written by Jean-Marc Valin
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The multistream decoder, which opusfile decodes every stream with. Only
//! the float output (`opus_copy_channel_out_float()`) is translated.

use super::opus_decoder::{opus_decode_native, opus_packet_get_nb_samples, OpusDecoder};
use super::packet::opus_packet_parse_impl;
use super::{
    OPUS_ALLOC_FAIL, OPUS_BAD_ARG, OPUS_BUFFER_TOO_SMALL, OPUS_INTERNAL_ERROR, OPUS_INVALID_PACKET,
    OPUS_OK,
};

/// Translation of `ChannelLayout`.
struct ChannelLayout {
    nb_channels: i32,
    nb_streams: i32,
    nb_coupled_streams: i32,
    mapping: [u8; 256],
}

/// Translation of `validate_layout()`.
fn validate_layout(layout: &ChannelLayout) -> bool {
    let max_channel = layout.nb_streams + layout.nb_coupled_streams;
    if max_channel > 255 {
        return false;
    }
    for &m in layout.mapping[..layout.nb_channels as usize].iter() {
        if m as i32 >= max_channel && m != 255 {
            return false;
        }
    }
    true
}

/// Translation of `get_left_channel()`.
fn get_left_channel(layout: &ChannelLayout, stream_id: i32, prev: i32) -> i32 {
    let start = if prev < 0 { 0 } else { prev + 1 };
    for i in start..layout.nb_channels {
        if layout.mapping[i as usize] as i32 == stream_id * 2 {
            return i;
        }
    }
    -1
}

/// Translation of `get_right_channel()`.
fn get_right_channel(layout: &ChannelLayout, stream_id: i32, prev: i32) -> i32 {
    let start = if prev < 0 { 0 } else { prev + 1 };
    for i in start..layout.nb_channels {
        if layout.mapping[i as usize] as i32 == stream_id * 2 + 1 {
            return i;
        }
    }
    -1
}

/// Translation of `get_mono_channel()`.
fn get_mono_channel(layout: &ChannelLayout, stream_id: i32, prev: i32) -> i32 {
    let start = if prev < 0 { 0 } else { prev + 1 };
    for i in start..layout.nb_channels {
        if layout.mapping[i as usize] as i32 == stream_id + layout.nb_coupled_streams {
            return i;
        }
    }
    -1
}

/// Translation of `struct OpusMSDecoder`; the stream decoders, which C
/// places after it in the same block, are in `decoders` (the coupled
/// streams first).
pub(crate) struct OpusMSDecoder {
    layout: ChannelLayout,
    decoders: Vec<OpusDecoder>,
}

impl OpusMSDecoder {
    /// Translation of `opus_multistream_decoder_create()` /
    /// `opus_multistream_decoder_init()`.
    pub(crate) fn create(
        fs: i32,
        channels: i32,
        streams: i32,
        coupled_streams: i32,
        mapping: &[u8],
    ) -> Result<OpusMSDecoder, i32> {
        if (channels > 255)
            || (channels < 1)
            || (coupled_streams > streams)
            || (streams < 1)
            || (coupled_streams < 0)
            || (streams > 255 - coupled_streams)
        {
            return Err(OPUS_BAD_ARG);
        }

        let mut layout = ChannelLayout {
            nb_channels: channels,
            nb_streams: streams,
            nb_coupled_streams: coupled_streams,
            mapping: [0; 256],
        };

        layout.mapping[..channels as usize].copy_from_slice(&mapping[..channels as usize]);
        if !validate_layout(&layout) {
            return Err(OPUS_BAD_ARG);
        }

        let mut decoders = Vec::new();
        decoders
            .try_reserve_exact(streams as usize)
            .map_err(|_| OPUS_ALLOC_FAIL)?;
        for i in 0..layout.nb_streams {
            decoders.push(OpusDecoder::opus_decoder_init(
                fs,
                if i < layout.nb_coupled_streams { 2 } else { 1 },
            )?);
        }
        Ok(OpusMSDecoder { layout, decoders })
    }

    /// `opus_multistream_decoder_ctl(st, OPUS_GET_SAMPLE_RATE(&value))`: the
    /// first stream's.
    fn sample_rate(&self) -> i32 {
        self.decoders[0].sample_rate()
    }

    /// `opus_multistream_decoder_ctl(st, OPUS_RESET_STATE)`.
    pub(crate) fn reset_state(&mut self) -> i32 {
        for dec in self.decoders.iter_mut() {
            dec.reset_state();
        }
        OPUS_OK
    }

    /// `opus_multistream_decoder_ctl(st, OPUS_SET_GAIN(value))`.
    pub(crate) fn set_gain(&mut self, value: i32) -> i32 {
        let mut ret = OPUS_OK;
        for dec in self.decoders.iter_mut() {
            ret = dec.set_gain(value);
            if ret != OPUS_OK {
                break;
            }
        }
        ret
    }

    /// `opus_multistream_decoder_ctl(st, OPUS_GET_FINAL_RANGE(&value))`.
    #[allow(dead_code)]
    pub(crate) fn final_range(&self) -> u32 {
        let mut value = 0;
        for dec in self.decoders.iter() {
            value ^= dec.final_range();
        }
        value
    }
}

fn opus_multistream_packet_validate(
    mut data: &[u8],
    mut len: i32,
    nb_streams: i32,
    fs: i32,
) -> i32 {
    let mut size = [0i16; 48];
    let mut samples = 0;
    let mut packet_offset: i32 = 0;
    let mut toc: u8 = 0;

    for s in 0..nb_streams {
        if len <= 0 {
            return OPUS_INVALID_PACKET;
        }
        let count = opus_packet_parse_impl(
            data,
            len,
            s != nb_streams - 1,
            Some(&mut toc),
            &mut size,
            None,
            Some(&mut packet_offset),
        );
        if count < 0 {
            return count;
        }
        let tmp_samples = opus_packet_get_nb_samples(data, packet_offset, fs);
        if s != 0 && samples != tmp_samples {
            return OPUS_INVALID_PACKET;
        }
        samples = tmp_samples;
        data = &data[packet_offset as usize..];
        len -= packet_offset;
    }
    samples
}

/// Translation of `opus_copy_channel_out_float()`; `None` is C's NULL `src`.
fn opus_copy_channel_out_float(
    dst: &mut [f32],
    dst_stride: i32,
    dst_channel: i32,
    src: Option<&[f32]>,
    src_stride: i32,
    frame_size: i32,
) {
    let (ds, dc, ss) = (
        dst_stride as usize,
        dst_channel as usize,
        src_stride as usize,
    );
    if let Some(src) = src {
        for i in 0..frame_size as usize {
            dst[i * ds + dc] = src[i * ss];
        }
    } else {
        for i in 0..frame_size as usize {
            dst[i * ds + dc] = 0.0;
        }
    }
}

/// Translation of `opus_multistream_decode_native()` with
/// `opus_copy_channel_out_float()`. `data` holds (at least) `len` bytes.
fn opus_multistream_decode_native(
    st: &mut OpusMSDecoder,
    mut data: &[u8],
    mut len: i32,
    pcm: &mut [f32],
    mut frame_size: i32,
    decode_fec: i32,
    soft_clip: i32,
) -> i32 {
    let mut do_plc = false;

    if frame_size <= 0 {
        return OPUS_BAD_ARG;
    }
    /* Limit frame_size to avoid excessive stack allocations. */
    let fs = st.sample_rate();
    frame_size = frame_size.min(fs / 25 * 3);
    let mut buf = Vec::new();
    if buf.try_reserve_exact(2 * frame_size as usize).is_err() {
        return OPUS_ALLOC_FAIL;
    }
    buf.resize(2 * frame_size as usize, 0.0f32);

    if len == 0 {
        do_plc = true;
    }
    if len < 0 {
        return OPUS_BAD_ARG;
    }
    if !do_plc && len < 2 * st.layout.nb_streams - 1 {
        return OPUS_INVALID_PACKET;
    }
    if !do_plc {
        let ret = opus_multistream_packet_validate(data, len, st.layout.nb_streams, fs);
        if ret < 0 {
            return ret;
        } else if ret > frame_size {
            return OPUS_BUFFER_TOO_SMALL;
        }
    }
    let nb_streams = st.layout.nb_streams;
    for s in 0..nb_streams {
        let dec = &mut st.decoders[s as usize];

        if !do_plc && len <= 0 {
            return OPUS_INTERNAL_ERROR;
        }
        let mut packet_offset: i32 = 0;
        let ret = opus_decode_native(
            dec,
            if do_plc { None } else { Some(data) },
            len,
            &mut buf,
            frame_size,
            decode_fec,
            s != nb_streams - 1,
            Some(&mut packet_offset),
            soft_clip,
        );
        if !do_plc {
            data = &data[packet_offset as usize..];
            len -= packet_offset;
        }
        if ret <= 0 {
            return ret;
        }
        frame_size = ret;
        let layout = &st.layout;
        if s < layout.nb_coupled_streams {
            let mut prev = -1;
            /* Copy "left" audio to the channel(s) where it belongs */
            loop {
                let chan = get_left_channel(layout, s, prev);
                if chan == -1 {
                    break;
                }
                opus_copy_channel_out_float(
                    pcm,
                    layout.nb_channels,
                    chan,
                    Some(&buf),
                    2,
                    frame_size,
                );
                prev = chan;
            }
            prev = -1;
            /* Copy "right" audio to the channel(s) where it belongs */
            loop {
                let chan = get_right_channel(layout, s, prev);
                if chan == -1 {
                    break;
                }
                opus_copy_channel_out_float(
                    pcm,
                    layout.nb_channels,
                    chan,
                    Some(&buf[1..]),
                    2,
                    frame_size,
                );
                prev = chan;
            }
        } else {
            let mut prev = -1;
            /* Copy audio to the channel(s) where it belongs */
            loop {
                let chan = get_mono_channel(layout, s, prev);
                if chan == -1 {
                    break;
                }
                opus_copy_channel_out_float(
                    pcm,
                    layout.nb_channels,
                    chan,
                    Some(&buf),
                    1,
                    frame_size,
                );
                prev = chan;
            }
        }
    }
    /* Handle muted channels */
    for c in 0..st.layout.nb_channels {
        if st.layout.mapping[c as usize] == 255 {
            opus_copy_channel_out_float(pcm, st.layout.nb_channels, c, None, 0, frame_size);
        }
    }
    frame_size
}

/// Translation of `opus_multistream_decode_float()` (float build).
pub(crate) fn opus_multistream_decode_float(
    st: &mut OpusMSDecoder,
    data: &[u8],
    len: i32,
    pcm: &mut [f32],
    frame_size: i32,
    decode_fec: i32,
) -> i32 {
    opus_multistream_decode_native(st, data, len, pcm, frame_size, decode_fec, 0)
}
