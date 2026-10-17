// Rust translation of celt/celt_decoder.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2007-2008 CSIRO
// Copyright (c) 2007-2010 Xiph.Org Foundation
// Copyright (c) 2008 Gregory Maxwell
// Written by Jean-Marc Valin and Gregory Maxwell
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The CELT decoder, in the float build without `CUSTOM_MODES` (so with
//! `deemphasis_stereo_simple()`, and no signalling byte), as libopus uses
//! it. The decoder state's trailing arrays (`_decode_mem` and the band
//! energy histories) are separate vectors; `opus_custom_decoder_ctl()`'s
//! requests are methods.

use super::arch::{imax, imin, max16, min16, CeltSig, Q15ONE, VERY_SMALL};
use super::bands::{
    anti_collapse, celt_lcg_rand, denormalise_bands, quant_all_bands, SPREAD_NORMAL,
};
use super::celt::{
    comb_filter, init_caps, resampling_factor, COMBFILTER_MINPERIOD, SPREAD_ICDF, TAPSET_ICDF,
    TF_SELECT_TABLE, TRIM_ICDF,
};
use super::celt_lpc::{_celt_autocorr, _celt_lpc, celt_fir, celt_iir, LPC_ORDER};
use super::entcode::{EcDec, BITRES};
use super::mathops::{celt_sqrt, frac_div32};
use super::mdct::clt_mdct_backward;
use super::modes::{opus_custom_mode_create, OpusCustomMode, MAX_PERIOD};
use super::pitch::{pitch_downsample, pitch_search};
use super::quant_bands::{unquant_coarse_energy, unquant_energy_finalise, unquant_fine_energy};
use super::rate::clt_compute_allocation;
use super::vq::renormalise_vector;
use crate::opus::{OPUS_BAD_ARG, OPUS_INTERNAL_ERROR, OPUS_OK};

/* The maximum pitch lag to allow in the pitch-based PLC. It's possible to save
CPU time in the PLC pitch search by making this smaller than MAX_PERIOD. The
current value corresponds to a pitch of 66.67 Hz. */
const PLC_PITCH_LAG_MAX: i32 = 720;
/* The minimum pitch lag to allow in the pitch-based PLC. This corresponds to a
pitch of 480 Hz. */
const PLC_PITCH_LAG_MIN: i32 = 100;

/**********************************************************************/
/*                                                                    */
/*                             DECODER                                */
/*                                                                    */
/**********************************************************************/
const DECODE_BUFFER_SIZE: usize = 2048;

/* Decoder state
@brief Decoder state
*/
/// Translation of `struct OpusCustomDecoder` (`CELTDecoder`).
pub(crate) struct OpusCustomDecoder {
    pub mode: &'static OpusCustomMode,
    #[allow(dead_code)] /* (the decoder reads mode->overlap) */ overlap: i32,
    channels: i32,
    stream_channels: i32,

    downsample: i32,
    start: i32,
    end: i32,
    signalling: i32,
    disable_inv: i32,

    /* Everything beyond this point gets cleared on a reset */
    rng: u32,
    error: i32,
    last_pitch_index: i32,
    loss_duration: i32,
    skip_plc: i32,
    postfilter_period: i32,
    postfilter_period_old: i32,
    postfilter_gain: f32,
    postfilter_gain_old: f32,
    postfilter_tapset: i32,
    postfilter_tapset_old: i32,

    preemph_mem_d: [CeltSig; 2],

    /// `_decode_mem`: Size = channels*(DECODE_BUFFER_SIZE+mode->overlap)
    decode_mem: Vec<CeltSig>,
    /// `lpc`: Size = channels*LPC_ORDER
    lpc: Vec<f32>,
    /// `oldEBands`: Size = 2*mode->nbEBands
    old_ebands: Vec<f32>,
    /// `oldLogE`: Size = 2*mode->nbEBands
    old_log_e: Vec<f32>,
    /// `oldLogE2`: Size = 2*mode->nbEBands
    old_log_e2: Vec<f32>,
    /// `backgroundLogE`: Size = 2*mode->nbEBands
    background_log_e: Vec<f32>,
}

impl OpusCustomDecoder {
    /// Translation of `celt_decoder_init()` (allocating the decoder, which
    /// `opus_decoder_init()` places in its own block).
    pub fn celt_decoder_init(sampling_rate: i32, channels: i32) -> Result<OpusCustomDecoder, i32> {
        let mode = opus_custom_mode_create(48000, 960).ok_or(OPUS_BAD_ARG)?;
        let mut st = Self::opus_custom_decoder_init(mode, channels)?;
        st.downsample = resampling_factor(sampling_rate);
        if st.downsample == 0 {
            Err(OPUS_BAD_ARG)
        } else {
            Ok(st)
        }
    }

    /// Translation of `opus_custom_decoder_init()`.
    fn opus_custom_decoder_init(
        mode: &'static OpusCustomMode,
        channels: i32,
    ) -> Result<OpusCustomDecoder, i32> {
        if !(0..=2).contains(&channels) {
            return Err(OPUS_BAD_ARG);
        }
        let nb = mode.nb_ebands as usize;
        let ch = channels as usize;
        let mut st = OpusCustomDecoder {
            mode,
            overlap: mode.overlap,
            channels,
            stream_channels: channels,

            downsample: 1,
            start: 0,
            end: mode.eff_ebands,
            signalling: 1,
            disable_inv: (channels == 1) as i32,

            rng: 0,
            error: 0,
            last_pitch_index: 0,
            loss_duration: 0,
            skip_plc: 0,
            postfilter_period: 0,
            postfilter_period_old: 0,
            postfilter_gain: 0.0,
            postfilter_gain_old: 0.0,
            postfilter_tapset: 0,
            postfilter_tapset_old: 0,
            preemph_mem_d: [0.0; 2],
            decode_mem: crate::internal::try_alloc(
                ch * (DECODE_BUFFER_SIZE + mode.overlap as usize),
            )
            .map_err(|_| crate::opus::OPUS_ALLOC_FAIL)?,
            lpc: vec![0.0; ch * LPC_ORDER],
            old_ebands: vec![0.0; 2 * nb],
            old_log_e: vec![0.0; 2 * nb],
            old_log_e2: vec![0.0; 2 * nb],
            background_log_e: vec![0.0; 2 * nb],
        };

        st.reset_state();

        Ok(st)
    }

    /// `opus_custom_decoder_ctl(st, OPUS_RESET_STATE)`.
    pub fn reset_state(&mut self) {
        self.rng = 0;
        self.error = 0;
        self.last_pitch_index = 0;
        self.loss_duration = 0;
        self.skip_plc = 0;
        self.postfilter_period = 0;
        self.postfilter_period_old = 0;
        self.postfilter_gain = 0.0;
        self.postfilter_gain_old = 0.0;
        self.postfilter_tapset = 0;
        self.postfilter_tapset_old = 0;
        self.preemph_mem_d = [0.0; 2];
        self.decode_mem.fill(0.0);
        self.lpc.fill(0.0);
        self.old_ebands.fill(0.0);
        self.background_log_e.fill(0.0);
        for i in 0..2 * self.mode.nb_ebands as usize {
            self.old_log_e[i] = -28.0;
            self.old_log_e2[i] = -28.0;
        }
        self.skip_plc = 1;
    }

    /// `CELT_SET_START_BAND`.
    pub fn set_start_band(&mut self, value: i32) -> i32 {
        if value < 0 || value >= self.mode.nb_ebands {
            return OPUS_BAD_ARG;
        }
        self.start = value;
        OPUS_OK
    }

    /// `CELT_SET_END_BAND`.
    pub fn set_end_band(&mut self, value: i32) -> i32 {
        if value < 1 || value > self.mode.nb_ebands {
            return OPUS_BAD_ARG;
        }
        self.end = value;
        OPUS_OK
    }

    /// `CELT_SET_CHANNELS`.
    pub fn set_channels(&mut self, value: i32) -> i32 {
        if !(1..=2).contains(&value) {
            return OPUS_BAD_ARG;
        }
        self.stream_channels = value;
        OPUS_OK
    }

    /// `CELT_SET_SIGNALLING`.
    pub fn set_signalling(&mut self, value: i32) {
        self.signalling = value;
    }

    /// `OPUS_GET_FINAL_RANGE`.
    pub fn final_range(&self) -> u32 {
        self.rng
    }

    /// `OPUS_SET_PHASE_INVERSION_DISABLED`.
    #[allow(dead_code)]
    pub fn set_phase_inversion_disabled(&mut self, value: i32) -> i32 {
        if !(0..=1).contains(&value) {
            return OPUS_BAD_ARG;
        }
        self.disable_inv = value;
        OPUS_OK
    }
}

/* Special case for stereo with no downsampling and no accumulation. This is
quite common and we can make it faster by processing both channels in the
same loop, reducing overhead due to the dependency loop in the IIR filter. */
fn deemphasis_stereo_simple(
    x0: &[f32],
    x1: &[f32],
    pcm: &mut [f32],
    n: usize,
    coef0: f32,
    mem: &mut [f32; 2],
) {
    let mut m0 = mem[0];
    let mut m1 = mem[1];
    for j in 0..n {
        /* Add VERY_SMALL to x[] first to reduce dependency chain. */
        let tmp0 = x0[j] + VERY_SMALL + m0;
        let tmp1 = x1[j] + VERY_SMALL + m1;
        m0 = coef0 * tmp0;
        m1 = coef0 * tmp1;
        pcm[2 * j] = super::arch::scaleout(tmp0);
        pcm[2 * j + 1] = super::arch::scaleout(tmp1);
    }
    mem[0] = m0;
    mem[1] = m1;
}

/// Translation of `deemphasis()`: `decode_mem[out_off[c]..]` is `in[c]`.
#[allow(clippy::too_many_arguments)]
fn deemphasis(
    decode_mem: &[f32],
    out_off: [usize; 2],
    pcm: &mut [f32],
    n: usize,
    c: usize,
    downsample: i32,
    coef: &[f32; 4],
    mem: &mut [f32; 2],
    accum: i32,
) {
    let mut apply_downsampling = false;
    /* Short version for common case. */
    if downsample == 1 && c == 2 && accum == 0 {
        deemphasis_stereo_simple(
            &decode_mem[out_off[0]..],
            &decode_mem[out_off[1]..],
            pcm,
            n,
            coef[0],
            mem,
        );
        return;
    }
    let _ = accum;
    debug_assert!(accum == 0);
    let mut scratch = vec![0.0f32; n];
    let coef0 = coef[0];
    let nd = n / downsample as usize;
    for ci in 0..c {
        let mut m = mem[ci];
        let x = &decode_mem[out_off[ci]..];
        if downsample > 1 {
            /* Shortcut for the standard (non-custom modes) case */
            for j in 0..n {
                let tmp = x[j] + VERY_SMALL + m;
                m = coef0 * tmp;
                scratch[j] = tmp;
            }
            apply_downsampling = true;
        } else {
            /* Shortcut for the standard (non-custom modes) case */
            for j in 0..n {
                let tmp = x[j] + VERY_SMALL + m;
                m = coef0 * tmp;
                pcm[ci + j * c] = super::arch::scaleout(tmp);
            }
        }
        mem[ci] = m;

        if apply_downsampling {
            /* Perform down-sampling */
            for j in 0..nd {
                pcm[ci + j * c] = super::arch::scaleout(scratch[j * downsample as usize]);
            }
        }
    }
}

/// Translation of `celt_synthesis()`: `decode_mem[out_off[c]..]` is
/// `out_syn[c]`, and each channel's buffer is `chan_len` long.
#[allow(clippy::too_many_arguments)]
fn celt_synthesis(
    mode: &OpusCustomMode,
    x: &[f32],
    decode_mem: &mut [f32],
    chan_len: usize,
    out_off: [usize; 2],
    old_band_e: &[f32],
    start: i32,
    eff_end: i32,
    c: i32,
    cc: i32,
    is_transient: i32,
    lm: i32,
    downsample: i32,
    silence: i32,
) {
    let overlap = mode.overlap;
    let nb_ebands = mode.nb_ebands as usize;
    let n = (mode.short_mdct_size << lm) as usize;
    let mut freq = vec![0.0f32; n]; /*< Interleaved signal MDCTs */
    let m = 1 << lm;
    let b: usize;
    let nb: usize;
    let shift: i32;

    if is_transient != 0 {
        b = m as usize;
        nb = mode.short_mdct_size as usize;
        shift = mode.max_lm;
    } else {
        b = 1;
        nb = (mode.short_mdct_size << lm) as usize;
        shift = mode.max_lm - lm;
    }
    // (out_syn[c] is chan_len*c + out_off0: the channels' buffers follow
    // each other.)
    let rel = out_off[0];

    if cc == 2 && c == 1 {
        /* Copying a mono streams to two channels */
        denormalise_bands(
            mode, x, &mut freq, old_band_e, start, eff_end, m, downsample, silence,
        );
        /* Store a temporary copy in the output buffer because the IMDCT destroys its input. */
        let (ch0, ch1) = decode_mem.split_at_mut(chan_len);
        let freq2_off = rel + (overlap / 2) as usize;
        ch1[freq2_off..freq2_off + n].copy_from_slice(&freq);
        for bi in 0..b {
            clt_mdct_backward(
                &mode.mdct,
                ch1,
                freq2_off + bi,
                &mut ch0[rel + nb * bi..],
                mode.window,
                overlap,
                shift,
                b as i32,
            );
        }
        for bi in 0..b {
            clt_mdct_backward(
                &mode.mdct,
                &freq,
                bi,
                &mut ch1[rel + nb * bi..],
                mode.window,
                overlap,
                shift,
                b as i32,
            );
        }
    } else if cc == 1 && c == 2 {
        /* Downmixing a stereo stream to mono */
        let freq2_off = rel + (overlap / 2) as usize;
        denormalise_bands(
            mode, x, &mut freq, old_band_e, start, eff_end, m, downsample, silence,
        );
        /* Use the output buffer as temp array before downmixing. */
        denormalise_bands(
            mode,
            &x[n..],
            &mut decode_mem[freq2_off..freq2_off + n],
            &old_band_e[nb_ebands..],
            start,
            eff_end,
            m,
            downsample,
            silence,
        );
        for i in 0..n {
            freq[i] = 0.5f32 * freq[i] + 0.5f32 * decode_mem[freq2_off + i];
        }
        for bi in 0..b {
            clt_mdct_backward(
                &mode.mdct,
                &freq,
                bi,
                &mut decode_mem[rel + nb * bi..],
                mode.window,
                overlap,
                shift,
                b as i32,
            );
        }
    } else {
        /* Normal case (mono or stereo) */
        for ci in 0..cc as usize {
            denormalise_bands(
                mode,
                &x[ci * n..],
                &mut freq,
                &old_band_e[ci * nb_ebands..],
                start,
                eff_end,
                m,
                downsample,
                silence,
            );
            for bi in 0..b {
                clt_mdct_backward(
                    &mode.mdct,
                    &freq,
                    bi,
                    &mut decode_mem[out_off[ci] + nb * bi..],
                    mode.window,
                    overlap,
                    shift,
                    b as i32,
                );
            }
        }
    }
    /* Saturate IMDCT output so that we can't overflow in the pitch postfilter
    or in the */
}

fn tf_decode(
    start: i32,
    end: i32,
    is_transient: i32,
    tf_res: &mut [i32],
    lm: i32,
    dec: &mut EcDec<'_>,
) {
    let mut budget = dec.storage * 8;
    let mut tell = dec.ec_tell() as u32;
    let mut logp: u32 = if is_transient != 0 { 2 } else { 4 };
    let tf_select_rsv = (lm > 0 && tell + logp + 1 <= budget) as i32;
    budget -= tf_select_rsv as u32;
    let mut tf_changed = 0;
    let mut curr = 0;
    for i in start as usize..end as usize {
        if tell + logp <= budget {
            curr ^= dec.ec_dec_bit_logp(logp);
            tell = dec.ec_tell() as u32;
            tf_changed |= curr;
        }
        tf_res[i] = curr;
        logp = if is_transient != 0 { 4 } else { 5 };
    }
    let mut tf_select = 0;
    let t = &TF_SELECT_TABLE[lm as usize];
    if tf_select_rsv != 0
        && t[(4 * is_transient + tf_changed) as usize]
            != t[(4 * is_transient + 2 + tf_changed) as usize]
    {
        tf_select = dec.ec_dec_bit_logp(1);
    }
    for i in start as usize..end as usize {
        tf_res[i] = t[(4 * is_transient + 2 * tf_select + tf_res[i]) as usize] as i32;
    }
}

fn celt_plc_pitch_search(decode_mem: [&[f32]; 2], c: i32) -> i32 {
    let mut pitch_index = 0;
    let mut lp_pitch_buf = vec![0.0f32; DECODE_BUFFER_SIZE >> 1];
    pitch_downsample(decode_mem, &mut lp_pitch_buf, DECODE_BUFFER_SIZE as i32, c);
    pitch_search(
        &lp_pitch_buf[(PLC_PITCH_LAG_MAX >> 1) as usize..],
        &lp_pitch_buf,
        DECODE_BUFFER_SIZE as i32 - PLC_PITCH_LAG_MAX,
        PLC_PITCH_LAG_MAX - PLC_PITCH_LAG_MIN,
        &mut pitch_index,
    );
    PLC_PITCH_LAG_MAX - pitch_index
}

fn celt_decode_lost(st: &mut OpusCustomDecoder, n: i32, lm: i32) {
    let c = st.channels as usize;
    let mode = st.mode;
    let nb_ebands = mode.nb_ebands as usize;
    let overlap = mode.overlap as usize;
    let ebands = mode.ebands;
    let chan_len = DECODE_BUFFER_SIZE + overlap;
    let nu = n as usize;

    let mut out_off = [0usize; 2];
    for ci in 0..c {
        out_off[ci] = ci * chan_len + DECODE_BUFFER_SIZE - nu;
    }

    let loss_duration = st.loss_duration;
    let start = st.start;
    let noise_based = loss_duration >= 40 || start != 0 || st.skip_plc != 0;
    if noise_based {
        /* Noise-based PLC/CNG */
        let end = st.end;
        let eff_end = imax(start, imin(end, mode.eff_ebands));

        let mut x = vec![0.0f32; c * nu]; /*< Interleaved normalised MDCTs */
        for ci in 0..c {
            let base = ci * chan_len;
            st.decode_mem.copy_within(
                base + nu..base + nu + DECODE_BUFFER_SIZE - nu + (overlap >> 1),
                base,
            );
        }

        /* Energy decay */
        let decay: f32 = if loss_duration == 0 { 1.5 } else { 0.5 };
        for ci in 0..c {
            for i in start as usize..end as usize {
                st.old_ebands[ci * nb_ebands + i] = max16(
                    st.background_log_e[ci * nb_ebands + i],
                    st.old_ebands[ci * nb_ebands + i] - decay,
                );
            }
        }
        let mut seed = st.rng;
        for ci in 0..c {
            for i in start as usize..eff_end as usize {
                let boffs = nu * ci + ((ebands[i] as usize) << lm);
                let blen = ((ebands[i + 1] - ebands[i]) as i32) << lm;
                for j in 0..blen as usize {
                    seed = celt_lcg_rand(seed);
                    x[boffs + j] = (seed as i32 >> 20) as f32;
                }
                renormalise_vector(&mut x[boffs..], blen, Q15ONE);
            }
        }
        st.rng = seed;

        celt_synthesis(
            mode,
            &x,
            &mut st.decode_mem,
            chan_len,
            out_off,
            &st.old_ebands,
            start,
            eff_end,
            c as i32,
            c as i32,
            0,
            lm,
            st.downsample,
            0,
        );
    } else {
        /* Pitch-based PLC */
        let window = mode.window;
        let mut fade = Q15ONE;
        let pitch_index;

        if loss_duration == 0 {
            let dm0 = &st.decode_mem[..chan_len];
            let dm1 = if c == 2 {
                &st.decode_mem[chan_len..2 * chan_len]
            } else {
                dm0
            };
            pitch_index = celt_plc_pitch_search([dm0, dm1], c as i32);
            st.last_pitch_index = pitch_index;
        } else {
            pitch_index = st.last_pitch_index;
            fade = 0.8;
        }

        /* We want the excitation for 2 pitch periods in order to look for a
        decaying signal, but we can't get more than MAX_PERIOD. */
        let exc_length = imin(2 * pitch_index, MAX_PERIOD) as usize;

        let mut etmp = vec![0.0f32; overlap];
        let mut _exc = vec![0.0f32; MAX_PERIOD as usize + LPC_ORDER];
        let mut fir_tmp = vec![0.0f32; exc_length];
        let exc = LPC_ORDER; /* exc = _exc+LPC_ORDER */
        let mp = MAX_PERIOD as usize;
        for ci in 0..c {
            let decay: f32;
            let mut s1: f32 = 0.0;
            let base = ci * chan_len;
            let buf = &mut st.decode_mem[base..base + chan_len];

            for i in 0..mp + LPC_ORDER {
                _exc[exc + i - LPC_ORDER] = buf[DECODE_BUFFER_SIZE - mp - LPC_ORDER + i];
            }

            if loss_duration == 0 {
                let mut ac = [0.0f32; LPC_ORDER + 1];
                /* Compute LPC coefficients for the last MAX_PERIOD samples before
                the first loss so we can work in the excitation-filter domain. */
                _celt_autocorr(
                    &_exc[exc..],
                    &mut ac,
                    Some(window),
                    overlap,
                    LPC_ORDER as i32,
                    MAX_PERIOD,
                );
                /* Add a noise floor of -40 dB. */
                ac[0] *= 1.0001f32;
                /* Use lag windowing to stabilize the Levinson-Durbin recursion. */
                for (i, a) in ac.iter_mut().enumerate().skip(1) {
                    /*ac[i] *= exp(-.5*(2*M_PI*.002*i)*(2*M_PI*.002*i));*/
                    *a -= *a * (0.008f32 * 0.008f32) * i as f32 * i as f32;
                }
                _celt_lpc(&mut st.lpc[ci * LPC_ORDER..], &ac, LPC_ORDER);
            }
            /* Initialize the LPC history with the samples just before the start
            of the region for which we're computing the excitation. */
            {
                /* Compute the excitation for exc_length samples before the loss. We need the copy
                because celt_fir() cannot filter in-place. */
                celt_fir(
                    &_exc,
                    exc + mp - exc_length,
                    &st.lpc[ci * LPC_ORDER..],
                    &mut fir_tmp,
                    exc_length as i32,
                    LPC_ORDER,
                );
                _exc[exc + mp - exc_length..exc + mp].copy_from_slice(&fir_tmp);
            }

            /* Check if the waveform is decaying, and if so how fast.
            We do this to avoid adding energy when concealing in a segment
            with decaying energy. */
            {
                let mut e1: f32 = 1.0;
                let mut e2: f32 = 1.0;
                let decay_length = exc_length >> 1;
                for i in 0..decay_length {
                    let mut e = _exc[exc + mp - decay_length + i];
                    e1 += e * e;
                    e = _exc[exc + mp - 2 * decay_length + i];
                    e2 += e * e;
                }
                e1 = super::arch::min32(e1, e2);
                decay = celt_sqrt(frac_div32(e1, e2) as f64);
            }

            /* Move the decoder memory one frame to the left to give us room to
            add the data for the new frame. We ignore the overlap that extends
            past the end of the buffer, because we aren't going to use it. */
            buf.copy_within(nu..DECODE_BUFFER_SIZE, 0);

            /* Extrapolate from the end of the excitation with a period of
            "pitch_index", scaling down each period by an additional factor of
            "decay". */
            let extrapolation_offset = mp - pitch_index as usize;
            /* We need to extrapolate enough samples to cover a complete MDCT
            window (including overlap/2 samples on both sides). */
            let extrapolation_len = nu + overlap;
            /* We also apply fading if this is not the first loss. */
            let mut attenuation = fade * decay;
            let mut j = 0usize;
            for i in 0..extrapolation_len {
                if j >= pitch_index as usize {
                    j -= pitch_index as usize;
                    attenuation *= decay;
                }
                buf[DECODE_BUFFER_SIZE - nu + i] =
                    attenuation * _exc[exc + extrapolation_offset + j];
                /* Compute the energy of the previously decoded signal whose
                excitation we're copying. */
                let tmp = buf[DECODE_BUFFER_SIZE - mp - nu + extrapolation_offset + j];
                s1 += tmp * tmp;
                j += 1;
            }
            {
                let mut lpc_mem = [0.0f32; LPC_ORDER];
                /* Copy the last decoded samples (prior to the overlap region) to
                synthesis filter memory so we can have a continuous signal. */
                for (i, l) in lpc_mem.iter_mut().enumerate() {
                    *l = buf[DECODE_BUFFER_SIZE - nu - 1 - i];
                }
                /* Apply the synthesis filter to convert the excitation back into
                the signal domain. */
                celt_iir(
                    &mut buf[DECODE_BUFFER_SIZE - nu..],
                    &st.lpc[ci * LPC_ORDER..],
                    extrapolation_len as i32,
                    LPC_ORDER,
                    &mut lpc_mem,
                );
            }

            /* Check if the synthesis energy is higher than expected, which can
            happen with the signal changes during our window. If so,
            attenuate. */
            {
                let mut s2: f32 = 0.0;
                for i in 0..extrapolation_len {
                    let tmp = buf[DECODE_BUFFER_SIZE - nu + i];
                    s2 += tmp * tmp;
                }
                /* This checks for an "explosion" in the synthesis. */
                /* The float test is written this way to catch NaNs in the output
                of the IIR filter at the same time. */
                if !(s1 > 0.2f32 * s2) {
                    buf[DECODE_BUFFER_SIZE - nu..DECODE_BUFFER_SIZE - nu + extrapolation_len]
                        .fill(0.0);
                } else if s1 < s2 {
                    let ratio = celt_sqrt(frac_div32(s1 + 1.0, s2 + 1.0) as f64);
                    for i in 0..overlap {
                        let tmp_g = Q15ONE - window[i] * (Q15ONE - ratio);
                        buf[DECODE_BUFFER_SIZE - nu + i] *= tmp_g;
                    }
                    for i in overlap..extrapolation_len {
                        buf[DECODE_BUFFER_SIZE - nu + i] *= ratio;
                    }
                }
            }

            /* Apply the pre-filter to the MDCT overlap for the next frame because
            the post-filter will be re-applied in the decoder after the MDCT
            overlap. */
            comb_filter(
                Some(&mut etmp),
                buf,
                DECODE_BUFFER_SIZE,
                st.postfilter_period,
                st.postfilter_period,
                overlap as i32,
                -st.postfilter_gain,
                -st.postfilter_gain,
                st.postfilter_tapset,
                st.postfilter_tapset,
                &[],
                0,
            );

            /* Simulate TDAC on the concealed audio so that it blends with the
            MDCT of the next frame. */
            for i in 0..overlap / 2 {
                buf[DECODE_BUFFER_SIZE + i] =
                    window[i] * etmp[overlap - 1 - i] + window[overlap - i - 1] * etmp[i];
            }
        }
    }

    /* Saturate to soemthing large to avoid wrap-around. */
    st.loss_duration = imin(10000, loss_duration + (1 << lm));
}

/// Translation of `celt_decode_with_ec()`: `pcm` gets `frame_size`
/// samples per channel; `dec` is the Opus decoder's range decoder (or
/// `None` to decode `data` alone).
pub(crate) fn celt_decode_with_ec<'d>(
    st: &mut OpusCustomDecoder,
    data: Option<&'d [u8]>,
    len: i32,
    pcm: &mut [f32],
    mut frame_size: i32,
    dec: Option<&mut EcDec<'d>>,
    accum: i32,
) -> i32 {
    let cc = st.channels;
    let c = st.stream_channels;
    let mode = st.mode;
    let nb_ebands = mode.nb_ebands;
    let overlap = mode.overlap;
    let ebands = mode.ebands;
    let start = st.start;
    let end = st.end;
    frame_size *= st.downsample;

    let mut lm = 0;
    while lm <= mode.max_lm {
        if mode.short_mdct_size << lm == frame_size {
            break;
        }
        lm += 1;
    }
    if lm > mode.max_lm {
        return OPUS_BAD_ARG;
    }
    let m = 1 << lm;

    if !(0..=1275).contains(&len) {
        return OPUS_BAD_ARG;
    }

    let n = m * mode.short_mdct_size;
    let nu = n as usize;
    let chan_len = DECODE_BUFFER_SIZE + overlap as usize;
    let mut out_off = [0usize; 2];
    for ci in 0..cc as usize {
        out_off[ci] = ci * chan_len + DECODE_BUFFER_SIZE - nu;
    }

    let mut eff_end = end;
    if eff_end > mode.eff_ebands {
        eff_end = mode.eff_ebands;
    }

    let data = match data {
        Some(d) if len > 1 => d,
        _ => {
            celt_decode_lost(st, n, lm);
            deemphasis(
                &st.decode_mem,
                out_off,
                pcm,
                nu,
                cc as usize,
                st.downsample,
                &mode.preemph,
                &mut st.preemph_mem_d,
                accum,
            );
            return frame_size / st.downsample;
        }
    };

    /* Check if there are at least two packets received consecutively before
     * turning on the pitch-based PLC */
    st.skip_plc = (st.loss_duration != 0) as i32;

    let mut own_dec;
    let dec: &mut EcDec<'d> = match dec {
        Some(d) => d,
        None => {
            own_dec = EcDec::ec_dec_init(data, len as u32);
            &mut own_dec
        }
    };

    let nb = nb_ebands as usize;
    if c == 1 {
        for i in 0..nb {
            st.old_ebands[i] = max16(st.old_ebands[i], st.old_ebands[nb + i]);
        }
    }

    let mut total_bits = len * 8;
    let mut tell = dec.ec_tell();

    let silence = if tell >= total_bits {
        1
    } else if tell == 1 {
        dec.ec_dec_bit_logp(15)
    } else {
        0
    };
    if silence != 0 {
        /* Pretend we've read all the remaining bits */
        tell = len * 8;
        dec.nbits_total += tell - dec.ec_tell();
    }

    let mut postfilter_gain: f32 = 0.0;
    let mut postfilter_pitch = 0;
    let mut postfilter_tapset = 0;
    if start == 0 && tell + 16 <= total_bits {
        if dec.ec_dec_bit_logp(1) != 0 {
            let octave = dec.ec_dec_uint(6) as i32;
            postfilter_pitch = (16 << octave) + dec.ec_dec_bits((4 + octave) as u32) as i32 - 1;
            let qg = dec.ec_dec_bits(3) as i32;
            if dec.ec_tell() + 2 <= total_bits {
                postfilter_tapset = dec.ec_dec_icdf(&TAPSET_ICDF, 2);
            }
            postfilter_gain = 0.09375f32 * (qg + 1) as f32;
        }
        tell = dec.ec_tell();
    }

    let is_transient = if lm > 0 && tell + 3 <= total_bits {
        let t = dec.ec_dec_bit_logp(3);
        tell = dec.ec_tell();
        t
    } else {
        0
    };

    let short_blocks = if is_transient != 0 { m } else { 0 };

    /* Decode the global flags (first symbols in the stream) */
    let intra_ener = if tell + 3 <= total_bits {
        dec.ec_dec_bit_logp(3)
    } else {
        0
    };
    /* Get band energies */
    unquant_coarse_energy(mode, start, end, &mut st.old_ebands, intra_ener, dec, c, lm);

    let mut tf_res = [0i32; 21];
    tf_decode(start, end, is_transient, &mut tf_res, lm, dec);

    tell = dec.ec_tell();
    let mut spread_decision = SPREAD_NORMAL;
    if tell + 4 <= total_bits {
        spread_decision = dec.ec_dec_icdf(&SPREAD_ICDF, 5);
    }

    let mut cap = [0i32; 21];

    init_caps(mode, &mut cap, lm, c);

    let mut offsets = [0i32; 21];

    let mut dynalloc_logp = 6;
    total_bits <<= BITRES;
    let mut tell = dec.ec_tell_frac() as i32;
    for i in start as usize..end as usize {
        let width = c * ((ebands[i + 1] - ebands[i]) as i32) << lm;
        /* quanta is 6 bits, but no more than 1 bit/sample
        and no less than 1/8 bit/sample */
        let quanta = imin(width << BITRES, imax(6 << BITRES, width));
        let mut dynalloc_loop_logp = dynalloc_logp;
        let mut boost = 0;
        while tell + (dynalloc_loop_logp << BITRES) < total_bits && boost < cap[i] {
            let flag = dec.ec_dec_bit_logp(dynalloc_loop_logp as u32);
            tell = dec.ec_tell_frac() as i32;
            if flag == 0 {
                break;
            }
            boost += quanta;
            total_bits -= quanta;
            dynalloc_loop_logp = 1;
        }
        offsets[i] = boost;
        /* Making dynalloc more likely */
        if boost > 0 {
            dynalloc_logp = imax(2, dynalloc_logp - 1);
        }
    }

    let mut fine_quant = [0i32; 21];
    let alloc_trim = if tell + (6 << BITRES) <= total_bits {
        dec.ec_dec_icdf(&TRIM_ICDF, 7)
    } else {
        5
    };

    let mut bits = ((len * 8) << BITRES) - dec.ec_tell_frac() as i32 - 1;
    let anti_collapse_rsv = if is_transient != 0 && lm >= 2 && bits >= (lm + 2) << BITRES {
        1 << BITRES
    } else {
        0
    };
    bits -= anti_collapse_rsv;

    let mut pulses = [0i32; 21];
    let mut fine_priority = [0i32; 21];

    let mut intensity = 0;
    let mut dual_stereo = 0;
    let mut balance = 0;
    let coded_bands = clt_compute_allocation(
        mode,
        start,
        end,
        &offsets,
        &cap,
        alloc_trim,
        &mut intensity,
        &mut dual_stereo,
        bits,
        &mut balance,
        &mut pulses,
        &mut fine_quant,
        &mut fine_priority,
        c,
        lm,
        dec,
    );

    unquant_fine_energy(mode, start, end, &mut st.old_ebands, &fine_quant, dec, c);

    for ci in 0..cc as usize {
        let base = ci * chan_len;
        st.decode_mem.copy_within(
            base + nu..base + nu + DECODE_BUFFER_SIZE - nu + overlap as usize / 2,
            base,
        );
    }

    /* Decode fixed codebook */
    let mut collapse_masks = [0u8; 2 * 21];

    let mut x = vec![0.0f32; c as usize * nu]; /*< Interleaved normalised MDCTs */

    {
        let (xc, yc) = x.split_at_mut(nu);
        quant_all_bands(
            mode,
            start,
            end,
            xc,
            if c == 2 { Some(yc) } else { None },
            &mut collapse_masks,
            &pulses,
            short_blocks,
            spread_decision,
            dual_stereo,
            intensity,
            &tf_res,
            len * (8 << BITRES) - anti_collapse_rsv,
            balance,
            dec,
            lm,
            coded_bands,
            &mut st.rng,
            st.disable_inv,
        );
    }

    let mut anti_collapse_on = 0;
    if anti_collapse_rsv > 0 {
        anti_collapse_on = dec.ec_dec_bits(1) as i32;
    }

    unquant_energy_finalise(
        mode,
        start,
        end,
        &mut st.old_ebands,
        &fine_quant,
        &fine_priority,
        len * 8 - dec.ec_tell(),
        dec,
        c,
    );

    if anti_collapse_on != 0 {
        anti_collapse(
            mode,
            &mut x,
            &collapse_masks,
            lm,
            c,
            n,
            start,
            end,
            &st.old_ebands,
            &st.old_log_e,
            &st.old_log_e2,
            &pulses,
            st.rng,
        );
    }

    if silence != 0 {
        for i in 0..(c * nb_ebands) as usize {
            st.old_ebands[i] = -28.0;
        }
    }

    celt_synthesis(
        mode,
        &x,
        &mut st.decode_mem,
        chan_len,
        out_off,
        &st.old_ebands,
        start,
        eff_end,
        c,
        cc,
        is_transient,
        lm,
        st.downsample,
        silence,
    );

    for ci in 0..cc as usize {
        st.postfilter_period = imax(st.postfilter_period, COMBFILTER_MINPERIOD);
        st.postfilter_period_old = imax(st.postfilter_period_old, COMBFILTER_MINPERIOD);
        comb_filter(
            None,
            &mut st.decode_mem,
            out_off[ci],
            st.postfilter_period_old,
            st.postfilter_period,
            mode.short_mdct_size,
            st.postfilter_gain_old,
            st.postfilter_gain,
            st.postfilter_tapset_old,
            st.postfilter_tapset,
            mode.window,
            overlap,
        );
        if lm != 0 {
            comb_filter(
                None,
                &mut st.decode_mem,
                out_off[ci] + mode.short_mdct_size as usize,
                st.postfilter_period,
                postfilter_pitch,
                n - mode.short_mdct_size,
                st.postfilter_gain,
                postfilter_gain,
                st.postfilter_tapset,
                postfilter_tapset,
                mode.window,
                overlap,
            );
        }
    }
    st.postfilter_period_old = st.postfilter_period;
    st.postfilter_gain_old = st.postfilter_gain;
    st.postfilter_tapset_old = st.postfilter_tapset;
    st.postfilter_period = postfilter_pitch;
    st.postfilter_gain = postfilter_gain;
    st.postfilter_tapset = postfilter_tapset;
    if lm != 0 {
        st.postfilter_period_old = st.postfilter_period;
        st.postfilter_gain_old = st.postfilter_gain;
        st.postfilter_tapset_old = st.postfilter_tapset;
    }

    if c == 1 {
        st.old_ebands.copy_within(0..nb, nb);
    }

    if is_transient == 0 {
        let (a, b) = (&mut st.old_log_e2, &st.old_log_e);
        a.copy_from_slice(b);
        let (a, b) = (&mut st.old_log_e, &st.old_ebands);
        a.copy_from_slice(b);
    } else {
        for i in 0..2 * nb {
            st.old_log_e[i] = min16(st.old_log_e[i], st.old_ebands[i]);
        }
    }
    /* In normal circumstances, we only allow the noise floor to increase by
    up to 2.4 dB/second, but when we're in DTX we give the weight of
    all missing packets to the update packet. */
    let max_background_increase = imin(160, st.loss_duration + m) as f32 * 0.001f32;
    for i in 0..2 * nb {
        st.background_log_e[i] = min16(
            st.background_log_e[i] + max_background_increase,
            st.old_ebands[i],
        );
    }
    /* In case start or end were to change */
    for ci in 0..2usize {
        for i in 0..start as usize {
            st.old_ebands[ci * nb + i] = 0.0;
            st.old_log_e[ci * nb + i] = -28.0;
            st.old_log_e2[ci * nb + i] = -28.0;
        }
        for i in end as usize..nb {
            st.old_ebands[ci * nb + i] = 0.0;
            st.old_log_e[ci * nb + i] = -28.0;
            st.old_log_e2[ci * nb + i] = -28.0;
        }
    }
    st.rng = dec.rng;

    deemphasis(
        &st.decode_mem,
        out_off,
        pcm,
        nu,
        cc as usize,
        st.downsample,
        &mode.preemph,
        &mut st.preemph_mem_d,
        accum,
    );
    st.loss_duration = 0;
    if dec.ec_tell() > 8 * len {
        return OPUS_INTERNAL_ERROR;
    }
    if dec.ec_get_error() != 0 {
        st.error = 1;
    }
    frame_size / st.downsample
}
