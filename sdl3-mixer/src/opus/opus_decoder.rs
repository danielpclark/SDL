// Rust translation of src/opus_decoder.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2010 Xiph.Org Foundation, Skype Limited
// Written by Jean-Marc Valin and Koen Vos
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The single stream Opus decoder: SILK, CELT and hybrid frames, mode
//! transitions, redundancy, FEC and concealment, in the float build. The C
//! decoder lives in one memory block holding the SILK and CELT decoders;
//! here they're fields.

use super::celt::celt_decoder::{celt_decode_with_ec, OpusCustomDecoder};
use super::celt::entcode::EcDec;
use super::celt::mathops::celt_exp2;
use super::packet::{opus_packet_get_samples_per_frame, opus_packet_parse_impl};
use super::silk::dec_api::{silk_decode, silk_init_decoder_all, SilkDecoder};
use super::silk::structs::SilkDecControlStruct;
use super::*;

/// Translation of `struct OpusDecoder`.
pub(crate) struct OpusDecoder {
    celt_dec: OpusCustomDecoder,
    silk_dec: SilkDecoder,
    channels: i32,
    fs: i32, /* Sampling rate (at the API level) */
    dec_control: SilkDecControlStruct,
    decode_gain: i32,

    /* Everything beyond this point gets cleared on a reset */
    stream_channels: i32,

    bandwidth: i32,
    mode: i32,
    prev_mode: i32,
    frame_size: i32,
    prev_redundancy: i32,
    last_packet_duration: i32,
    softclip_mem: [f32; 2],

    range_final: u32,
}

#[cfg(debug_assertions)]
fn validate_opus_decoder(st: &OpusDecoder) {
    debug_assert!(st.channels == 1 || st.channels == 2);
    debug_assert!(
        st.fs == 48000 || st.fs == 24000 || st.fs == 16000 || st.fs == 12000 || st.fs == 8000
    );
    debug_assert!(st.dec_control.api_sample_rate == st.fs);
    debug_assert!(
        st.dec_control.internal_sample_rate == 0
            || st.dec_control.internal_sample_rate == 16000
            || st.dec_control.internal_sample_rate == 12000
            || st.dec_control.internal_sample_rate == 8000
    );
    debug_assert!(st.dec_control.n_channels_api == st.channels);
    debug_assert!(
        st.dec_control.n_channels_internal == 0
            || st.dec_control.n_channels_internal == 1
            || st.dec_control.n_channels_internal == 2
    );
    debug_assert!(
        st.dec_control.payload_size_ms == 0
            || st.dec_control.payload_size_ms == 10
            || st.dec_control.payload_size_ms == 20
            || st.dec_control.payload_size_ms == 40
            || st.dec_control.payload_size_ms == 60
    );
    debug_assert!(st.stream_channels == 1 || st.stream_channels == 2);
}

impl OpusDecoder {
    /// Translation of `opus_decoder_create()` / `opus_decoder_init()`.
    pub(crate) fn opus_decoder_init(fs: i32, channels: i32) -> Result<OpusDecoder, i32> {
        if (fs != 48000 && fs != 24000 && fs != 16000 && fs != 12000 && fs != 8000)
            || (channels != 1 && channels != 2)
        {
            return Err(OPUS_BAD_ARG);
        }

        /* Initialize CELT decoder */
        let mut celt_dec = match OpusCustomDecoder::celt_decoder_init(fs, channels) {
            Ok(d) => d,
            Err(OPUS_ALLOC_FAIL) => return Err(OPUS_ALLOC_FAIL),
            Err(_) => return Err(OPUS_INTERNAL_ERROR),
        };
        celt_dec.set_signalling(0);

        let mut st = OpusDecoder {
            celt_dec,
            silk_dec: SilkDecoder::default(),
            channels,
            fs,
            dec_control: SilkDecControlStruct::default(),
            decode_gain: 0,
            stream_channels: channels,
            bandwidth: 0,
            mode: 0,
            prev_mode: 0,
            frame_size: 0,
            prev_redundancy: 0,
            last_packet_duration: 0,
            softclip_mem: [0.0; 2],
            range_final: 0,
        };
        st.dec_control.api_sample_rate = st.fs;
        st.dec_control.n_channels_api = st.channels;

        /* Reset decoder */
        let ret = silk_init_decoder_all(&mut st.silk_dec);
        if ret != 0 {
            return Err(OPUS_INTERNAL_ERROR);
        }

        st.prev_mode = 0;
        st.frame_size = fs / 400;
        Ok(st)
    }

    /// `opus_decoder_ctl(st, OPUS_RESET_STATE)`.
    pub(crate) fn reset_state(&mut self) {
        self.stream_channels = 0;
        self.bandwidth = 0;
        self.mode = 0;
        self.prev_mode = 0;
        self.frame_size = 0;
        self.prev_redundancy = 0;
        self.last_packet_duration = 0;
        self.softclip_mem = [0.0; 2];
        self.range_final = 0;

        self.celt_dec.reset_state();
        silk_init_decoder_all(&mut self.silk_dec);
        self.stream_channels = self.channels;
        self.frame_size = self.fs / 400;
    }

    /// `opus_decoder_ctl(st, OPUS_SET_GAIN(value))`.
    pub(crate) fn set_gain(&mut self, value: i32) -> i32 {
        if !(-32768..=32767).contains(&value) {
            return OPUS_BAD_ARG;
        }
        self.decode_gain = value;
        OPUS_OK
    }

    /// `opus_decoder_ctl(st, OPUS_GET_SAMPLE_RATE(&value))`.
    pub(crate) fn sample_rate(&self) -> i32 {
        self.fs
    }

    /// `opus_decoder_ctl(st, OPUS_GET_FINAL_RANGE(&value))`.
    #[allow(dead_code)]
    pub(crate) fn final_range(&self) -> u32 {
        self.range_final
    }
}

fn smooth_fade(
    in1: &[f32],
    in2: &[f32],
    out: &mut [f32],
    overlap: i32,
    channels: i32,
    window: &[f32],
    fs: i32,
) {
    let inc = (48000 / fs) as usize;
    let ch = channels as usize;
    for c in 0..ch {
        for i in 0..overlap as usize {
            let w = window[i * inc] * window[i * inc];
            out[i * ch + c] = w * in2[i * ch + c] + (1.0 - w) * in1[i * ch + c];
        }
    }
}

fn opus_packet_get_mode(data: &[u8]) -> i32 {
    if data[0] & 0x80 != 0 {
        MODE_CELT_ONLY
    } else if (data[0] & 0x60) == 0x60 {
        MODE_HYBRID
    } else {
        MODE_SILK_ONLY
    }
}

fn opus_decode_frame(
    st: &mut OpusDecoder,
    mut data: Option<&[u8]>,
    mut len: i32,
    pcm: &mut [f32],
    mut frame_size: i32,
    decode_fec: i32,
) -> i32 {
    let mut silk_ret;
    let mut celt_ret = 0;
    let mut silk_frame_size: i32 = 0;
    let mut pcm_transition: Option<Vec<f32>> = None;

    let audiosize: i32;
    let mode: i32;
    let bandwidth: i32;
    let mut transition = false;
    let mut redundancy = 0;
    let mut redundancy_bytes: i32 = 0;
    let mut celt_to_silk = 0;
    let mut redundant_rng: u32 = 0;
    let ch = st.channels as usize;

    let f20 = st.fs / 50;
    let f10 = f20 >> 1;
    let f5 = f10 >> 1;
    let f2_5 = f5 >> 1;
    if frame_size < f2_5 {
        return OPUS_BUFFER_TOO_SMALL;
    }
    /* Limit frame_size to avoid excessive stack allocations. */
    frame_size = frame_size.min(st.fs / 25 * 3);
    /* Payloads of 1 (2 including ToC) or 0 trigger the PLC/DTX */
    if len <= 1 {
        data = None;
        /* In that case, don't conceal more than what the ToC says */
        frame_size = frame_size.min(st.frame_size);
    }
    let data_full = data.unwrap_or(&[]);
    /* (C leaves dec uninitialized without data; nothing reads it then) */
    let dec_len = if data.is_some() { len as usize } else { 0 };
    let mut dec = EcDec::ec_dec_init(&data_full[..dec_len], dec_len as u32);
    if data.is_some() {
        audiosize = st.frame_size;
        mode = st.mode;
        bandwidth = st.bandwidth;
    } else {
        let mut asz = frame_size;
        /* Run PLC using last used mode (CELT if we ended with CELT redundancy) */
        mode = if st.prev_redundancy != 0 {
            MODE_CELT_ONLY
        } else {
            st.prev_mode
        };
        bandwidth = 0;

        if mode == 0 {
            /* If we haven't got any packet yet, all we can do is return zeros */
            pcm[..(asz as usize) * ch].fill(0.0);
            return asz;
        }

        /* Avoids trying to run the PLC on sizes other than 2.5 (CELT), 5 (CELT),
        10, or 20 (e.g. 12.5 or 30 ms). */
        if asz > f20 {
            let mut off = 0usize;
            loop {
                let ret = opus_decode_frame(st, None, 0, &mut pcm[off..], asz.min(f20), 0);
                if ret < 0 {
                    return ret;
                }
                off += ret as usize * ch;
                asz -= ret;
                if asz <= 0 {
                    break;
                }
            }
            return frame_size;
        } else if asz < f20 {
            if asz > f10 {
                asz = f10;
            } else if mode != MODE_SILK_ONLY && asz > f5 && asz < f10 {
                asz = f5;
            }
        }
        audiosize = asz;
    }

    /* In fixed-point, we can tell CELT to do the accumulation on top of the
    SILK PCM buffer. This saves some stack space. */
    let celt_accum = 0;

    if data.is_some()
        && st.prev_mode > 0
        && ((mode == MODE_CELT_ONLY && st.prev_mode != MODE_CELT_ONLY && st.prev_redundancy == 0)
            || (mode != MODE_CELT_ONLY && st.prev_mode == MODE_CELT_ONLY))
    {
        transition = true;
        /* Decide where to allocate the stack memory for pcm_transition */
        /* (one buffer serves as both pcm_transition_celt and pcm_transition_silk) */
    }
    if transition && mode == MODE_CELT_ONLY {
        let mut buf = vec![0.0f32; (f5 as usize) * ch];
        opus_decode_frame(st, None, 0, &mut buf, f5.min(audiosize), 0);
        pcm_transition = Some(buf);
    }
    if audiosize > frame_size {
        /*fprintf(stderr, "PCM buffer too small: %d vs %d (mode = %d)\n", audiosize, frame_size, mode);*/
        return OPUS_BAD_ARG;
    } else {
        frame_size = audiosize;
    }
    let fsz = frame_size as usize;

    /* Don't allocate any memory when in CELT-only mode */
    let pcm_silk_size = if mode != MODE_CELT_ONLY && celt_accum == 0 {
        f10.max(frame_size) as usize * ch
    } else {
        0
    };
    let mut pcm_silk = vec![0i16; pcm_silk_size];

    /* SILK processing */
    if mode != MODE_CELT_ONLY {
        let mut pcm_ptr = 0usize;

        if st.prev_mode == MODE_CELT_ONLY {
            silk_init_decoder_all(&mut st.silk_dec);
        }

        /* The SILK PLC cannot produce frames of less than 10 ms */
        st.dec_control.payload_size_ms = 10.max(1000 * audiosize / st.fs);

        if data.is_some() {
            st.dec_control.n_channels_internal = st.stream_channels;
            if mode == MODE_SILK_ONLY {
                if bandwidth == OPUS_BANDWIDTH_NARROWBAND {
                    st.dec_control.internal_sample_rate = 8000;
                } else if bandwidth == OPUS_BANDWIDTH_MEDIUMBAND {
                    st.dec_control.internal_sample_rate = 12000;
                } else if bandwidth == OPUS_BANDWIDTH_WIDEBAND {
                    st.dec_control.internal_sample_rate = 16000;
                } else {
                    st.dec_control.internal_sample_rate = 16000;
                    debug_assert!(false);
                }
            } else {
                /* Hybrid mode */
                st.dec_control.internal_sample_rate = 16000;
            }
        }

        let lost_flag = if data.is_none() { 1 } else { 2 * decode_fec };
        let mut decoded_samples = 0;
        loop {
            /* Call SILK decoder */
            let first_frame = (decoded_samples == 0) as i32;
            silk_ret = silk_decode(
                &mut st.silk_dec,
                &mut st.dec_control,
                lost_flag,
                first_frame,
                &mut dec,
                &mut pcm_silk[pcm_ptr..],
                &mut silk_frame_size,
            );
            if silk_ret != 0 {
                if lost_flag != 0 {
                    /* PLC failure should not be fatal */
                    silk_frame_size = frame_size;
                    /* (C clears frame_size*channels samples from pcm_ptr, which can run
                    past pcm_silk after a partial decode; this stops at its end) */
                    let end = (pcm_ptr + fsz * ch).min(pcm_silk.len());
                    pcm_silk[pcm_ptr..end].fill(0);
                } else {
                    return OPUS_INTERNAL_ERROR;
                }
            }
            pcm_ptr += silk_frame_size as usize * ch;
            decoded_samples += silk_frame_size;
            if decoded_samples >= frame_size {
                break;
            }
        }
    }

    let mut start_band = 0;
    if decode_fec == 0
        && mode != MODE_CELT_ONLY
        && data.is_some()
        && dec.ec_tell() + 17 + 20 * (mode == MODE_HYBRID) as i32 <= 8 * len
    {
        /* Check if we have a redundant 0-8 kHz band */
        if mode == MODE_HYBRID {
            redundancy = dec.ec_dec_bit_logp(12);
        } else {
            redundancy = 1;
        }
        if redundancy != 0 {
            celt_to_silk = dec.ec_dec_bit_logp(1);
            /* redundancy_bytes will be at least two, in the non-hybrid
            case due to the ec_tell() check above */
            redundancy_bytes = if mode == MODE_HYBRID {
                dec.ec_dec_uint(256) as i32 + 2
            } else {
                len - ((dec.ec_tell() + 7) >> 3)
            };
            len -= redundancy_bytes;
            /* This is a sanity check. It should never happen for a valid
            packet, so the exact behaviour is not normative. */
            if len * 8 < dec.ec_tell() {
                len = 0;
                redundancy_bytes = 0;
                redundancy = 0;
            }
            /* Shrink decoder because of raw bits */
            dec.storage = dec.storage.wrapping_sub(redundancy_bytes as u32);
        }
    }
    if mode != MODE_CELT_ONLY {
        start_band = 17;
    }

    if redundancy != 0 {
        transition = false;
    }

    if transition && mode != MODE_CELT_ONLY {
        let mut buf = vec![0.0f32; (f5 as usize) * ch];
        opus_decode_frame(st, None, 0, &mut buf, f5.min(audiosize), 0);
        pcm_transition = Some(buf);
    }

    if bandwidth != 0 {
        let mut endband = 21;

        match bandwidth {
            OPUS_BANDWIDTH_NARROWBAND => {
                endband = 13;
            }
            OPUS_BANDWIDTH_MEDIUMBAND | OPUS_BANDWIDTH_WIDEBAND => {
                endband = 17;
            }
            OPUS_BANDWIDTH_SUPERWIDEBAND => {
                endband = 19;
            }
            OPUS_BANDWIDTH_FULLBAND => {
                endband = 21;
            }
            _ => {
                debug_assert!(false);
            }
        }
        st.celt_dec.set_end_band(endband);
    }
    st.celt_dec.set_channels(st.stream_channels);

    /* Only allocation memory for redundancy if/when needed */
    let mut redundant_audio = vec![
        0.0f32;
        if redundancy != 0 {
            (f5 as usize) * ch
        } else {
            0
        }
    ];
    let redundant_data: &[u8] = if redundancy != 0 {
        &data_full[len as usize..]
    } else {
        &[]
    };

    /* 5 ms redundant frame for CELT->SILK*/
    if redundancy != 0 && celt_to_silk != 0 {
        /* If the previous frame did not use CELT (the first redundancy frame in
        a transition from SILK may have been lost) then the CELT decoder is
        stale at this point and the redundancy audio is not useful, however
        the final range is still needed (for testing), so the redundancy is
        always decoded but the decoded audio may not be used */
        st.celt_dec.set_start_band(0);
        celt_decode_with_ec(
            &mut st.celt_dec,
            Some(&redundant_data[..redundancy_bytes as usize]),
            redundancy_bytes,
            &mut redundant_audio,
            f5,
            None,
            0,
        );
        redundant_rng = st.celt_dec.final_range();
    }

    /* MUST be after PLC */
    st.celt_dec.set_start_band(start_band);

    if mode != MODE_SILK_ONLY {
        let celt_frame_size = f20.min(frame_size);
        /* Make sure to discard any previous CELT state */
        if mode != st.prev_mode && st.prev_mode > 0 && st.prev_redundancy == 0 {
            st.celt_dec.reset_state();
        }
        /* Decode CELT */
        let celt_data = if decode_fec != 0 {
            None
        } else {
            data.map(|d| &d[..len.max(0) as usize])
        };
        celt_ret = celt_decode_with_ec(
            &mut st.celt_dec,
            celt_data,
            len,
            pcm,
            celt_frame_size,
            Some(&mut dec),
            celt_accum,
        );
    } else {
        let silence: [u8; 2] = [0xFF, 0xFF];
        if celt_accum == 0 {
            pcm[..fsz * ch].fill(0.0);
        }
        /* For hybrid -> SILK transitions, we let the CELT MDCT
        do a fade-out by decoding a silence frame */
        if st.prev_mode == MODE_HYBRID
            && !(redundancy != 0 && celt_to_silk != 0 && st.prev_redundancy != 0)
        {
            st.celt_dec.set_start_band(0);
            celt_decode_with_ec(
                &mut st.celt_dec,
                Some(&silence),
                2,
                pcm,
                f2_5,
                None,
                celt_accum,
            );
        }
    }

    if mode != MODE_CELT_ONLY && celt_accum == 0 {
        for i in 0..fsz * ch {
            pcm[i] += (1.0f32 / 32768.0f32) * pcm_silk[i] as f32;
        }
    }

    let window: &[f32] = st.celt_dec.mode.window;

    let f2_5u = f2_5 as usize;
    /* 5 ms redundant frame for SILK->CELT */
    if redundancy != 0 && celt_to_silk == 0 {
        st.celt_dec.reset_state();
        st.celt_dec.set_start_band(0);

        celt_decode_with_ec(
            &mut st.celt_dec,
            Some(&redundant_data[..redundancy_bytes as usize]),
            redundancy_bytes,
            &mut redundant_audio,
            f5,
            None,
            0,
        );
        redundant_rng = st.celt_dec.final_range();
        let o = ch * (fsz - f2_5u);
        let in1 = pcm[o..o + ch * f2_5u].to_vec();
        smooth_fade(
            &in1,
            &redundant_audio[ch * f2_5u..],
            &mut pcm[o..],
            f2_5,
            st.channels,
            window,
            st.fs,
        );
    }
    /* 5ms redundant frame for CELT->SILK; ignore if the previous frame did not
    use CELT (the first redundancy frame in a transition from SILK may have
    been lost) */
    if redundancy != 0
        && celt_to_silk != 0
        && (st.prev_mode != MODE_SILK_ONLY || st.prev_redundancy != 0)
    {
        for c in 0..ch {
            for i in 0..f2_5u {
                pcm[ch * i + c] = redundant_audio[ch * i + c];
            }
        }
        let o = ch * f2_5u;
        let in2 = pcm[o..o + ch * f2_5u].to_vec();
        smooth_fade(
            &redundant_audio[o..],
            &in2,
            &mut pcm[o..],
            f2_5,
            st.channels,
            window,
            st.fs,
        );
    }
    if transition {
        let pt = pcm_transition.as_deref().unwrap_or(&[]);
        if audiosize >= f5 {
            pcm[..ch * f2_5u].copy_from_slice(&pt[..ch * f2_5u]);
            let o = ch * f2_5u;
            let in2 = pcm[o..o + ch * f2_5u].to_vec();
            smooth_fade(
                &pt[o..],
                &in2,
                &mut pcm[o..],
                f2_5,
                st.channels,
                window,
                st.fs,
            );
        } else {
            /* Not enough time to do a clean transition, but we do it anyway
            This will not preserve amplitude perfectly and may introduce
            a bit of temporal aliasing, but it shouldn't be too bad and
            that's pretty much the best we can do. In any case, generating this
            transition it pretty silly in the first place */
            let in2 = pcm[..ch * f2_5u].to_vec();
            smooth_fade(pt, &in2, pcm, f2_5, st.channels, window, st.fs);
        }
    }

    if st.decode_gain != 0 {
        let gain = celt_exp2(6.48814081e-4f32 * st.decode_gain as f32);
        for v in pcm[..fsz * ch].iter_mut() {
            let x = *v * gain;
            *v = x;
        }
    }

    if len <= 1 {
        st.range_final = 0;
    } else {
        st.range_final = dec.rng ^ redundant_rng;
    }

    st.prev_mode = mode;
    st.prev_redundancy = (redundancy != 0 && celt_to_silk == 0) as i32;

    if celt_ret < 0 {
        celt_ret
    } else {
        audiosize
    }
}

/// Translation of `opus_decode_native()`. `data` is `None` for a lost
/// packet (C's NULL), otherwise it holds (at least) `len` bytes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn opus_decode_native(
    st: &mut OpusDecoder,
    data: Option<&[u8]>,
    len: i32,
    pcm: &mut [f32],
    frame_size: i32,
    decode_fec: i32,
    self_delimited: bool,
    packet_offset: Option<&mut i32>,
    soft_clip: i32,
) -> i32 {
    /* 48 x 2.5 ms = 120 ms */
    let mut size = [0i16; 48];
    let mut offset: i32 = 0;
    let mut toc: u8 = 0;
    #[cfg(debug_assertions)]
    validate_opus_decoder(st);
    if !(0..=1).contains(&decode_fec) {
        return OPUS_BAD_ARG;
    }
    /* For FEC/PLC, frame_size has to be to have a multiple of 2.5 ms */
    if (decode_fec != 0 || len == 0 || data.is_none()) && frame_size % (st.fs / 400) != 0 {
        return OPUS_BAD_ARG;
    }
    let ch = st.channels as usize;
    let Some(data) = data.filter(|_| len != 0) else {
        let mut pcm_count = 0;
        loop {
            let ret = opus_decode_frame(
                st,
                None,
                0,
                &mut pcm[pcm_count as usize * ch..],
                frame_size - pcm_count,
                0,
            );
            if ret < 0 {
                return ret;
            }
            pcm_count += ret;
            if pcm_count >= frame_size {
                break;
            }
        }
        debug_assert!(pcm_count == frame_size);
        st.last_packet_duration = pcm_count;
        return pcm_count;
    };
    if len < 0 {
        return OPUS_BAD_ARG;
    }

    let packet_mode = opus_packet_get_mode(data);
    let packet_bandwidth = opus_packet_get_bandwidth(data);
    let packet_frame_size = opus_packet_get_samples_per_frame(data, st.fs);
    let packet_stream_channels = opus_packet_get_nb_channels(data);

    let count = opus_packet_parse_impl(
        data,
        len,
        self_delimited,
        Some(&mut toc),
        &mut size,
        Some(&mut offset),
        packet_offset,
    );
    if count < 0 {
        return count;
    }

    let mut data = &data[offset as usize..];

    if decode_fec != 0 {
        /* If no FEC can be present, run the PLC (recursive call) */
        if frame_size < packet_frame_size
            || packet_mode == MODE_CELT_ONLY
            || st.mode == MODE_CELT_ONLY
        {
            return opus_decode_native(st, None, 0, pcm, frame_size, 0, false, None, soft_clip);
        }
        /* Otherwise, run the PLC on everything except the size for which we might have FEC */
        let duration_copy = st.last_packet_duration;
        if frame_size - packet_frame_size != 0 {
            let ret = opus_decode_native(
                st,
                None,
                0,
                pcm,
                frame_size - packet_frame_size,
                0,
                false,
                None,
                soft_clip,
            );
            if ret < 0 {
                st.last_packet_duration = duration_copy;
                return ret;
            }
            debug_assert!(ret == frame_size - packet_frame_size);
        }
        /* Complete with FEC */
        st.mode = packet_mode;
        st.bandwidth = packet_bandwidth;
        st.frame_size = packet_frame_size;
        st.stream_channels = packet_stream_channels;
        let ret = opus_decode_frame(
            st,
            Some(data),
            size[0] as i32,
            &mut pcm[ch * (frame_size - packet_frame_size) as usize..],
            packet_frame_size,
            1,
        );
        if ret < 0 {
            return ret;
        } else {
            st.last_packet_duration = frame_size;
            return frame_size;
        }
    }

    if count * packet_frame_size > frame_size {
        return OPUS_BUFFER_TOO_SMALL;
    }

    /* Update the state as the last step to avoid updating it on an invalid packet */
    st.mode = packet_mode;
    st.bandwidth = packet_bandwidth;
    st.frame_size = packet_frame_size;
    st.stream_channels = packet_stream_channels;

    let mut nb_samples = 0;
    for &sz in size[..count as usize].iter() {
        let ret = opus_decode_frame(
            st,
            Some(data),
            sz as i32,
            &mut pcm[nb_samples as usize * ch..],
            frame_size - nb_samples,
            0,
        );
        if ret < 0 {
            return ret;
        }
        debug_assert!(ret == packet_frame_size);
        data = &data[sz as usize..];
        nb_samples += ret;
    }
    st.last_packet_duration = nb_samples;
    debug_assert!(soft_clip == 0, "opus_pcm_soft_clip() is not translated");
    st.softclip_mem = [0.0; 2];
    nb_samples
}

/// Translation of `opus_packet_get_bandwidth()`.
pub(crate) fn opus_packet_get_bandwidth(data: &[u8]) -> i32 {
    let mut bandwidth;
    if data[0] & 0x80 != 0 {
        bandwidth = OPUS_BANDWIDTH_MEDIUMBAND + ((data[0] >> 5) & 0x3) as i32;
        if bandwidth == OPUS_BANDWIDTH_MEDIUMBAND {
            bandwidth = OPUS_BANDWIDTH_NARROWBAND;
        }
    } else if (data[0] & 0x60) == 0x60 {
        bandwidth = if data[0] & 0x10 != 0 {
            OPUS_BANDWIDTH_FULLBAND
        } else {
            OPUS_BANDWIDTH_SUPERWIDEBAND
        };
    } else {
        bandwidth = OPUS_BANDWIDTH_NARROWBAND + ((data[0] >> 5) & 0x3) as i32;
    }
    bandwidth
}

/// Translation of `opus_packet_get_nb_channels()`.
pub(crate) fn opus_packet_get_nb_channels(data: &[u8]) -> i32 {
    if data[0] & 0x4 != 0 {
        2
    } else {
        1
    }
}

/// Translation of `opus_packet_get_nb_frames()`.
pub(crate) fn opus_packet_get_nb_frames(packet: &[u8], len: i32) -> i32 {
    if len < 1 {
        return OPUS_BAD_ARG;
    }
    let count = packet[0] & 0x3;
    if count == 0 {
        1
    } else if count != 3 {
        2
    } else if len < 2 {
        OPUS_INVALID_PACKET
    } else {
        (packet[1] & 0x3F) as i32
    }
}

/// Translation of `opus_packet_get_nb_samples()`.
pub(crate) fn opus_packet_get_nb_samples(packet: &[u8], len: i32, fs: i32) -> i32 {
    let count = opus_packet_get_nb_frames(packet, len);

    if count < 0 {
        return count;
    }

    let samples = count * opus_packet_get_samples_per_frame(packet, fs);
    /* Can't have more than 120 ms */
    if samples * 25 > fs * 3 {
        OPUS_INVALID_PACKET
    } else {
        samples
    }
}
